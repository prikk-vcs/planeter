//! RFC 012 D-3 through the router, with a **test-only second language** (`qaa`: right-to-left, no grammatical
//! plural, its own number format, placeholders in another order): negotiation, the language cookie,
//! `lang`/`dir`, `Vary`/`Content-Language`, plural and placeholder handling, the API's `message`/`hint`,
//! the pages that used to be bare strings, and the escaping of typed slots (T2). No prikk needed.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore,
};
use planeter_web::catalog::{
    Dir, EN, Entry, Key, Lang, LangDef, Plural, T, audit, register_test_language, shipped,
};
use planeter_web::errors::ErrorCode;
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

// ---------------------------------------------------------------- the test language

fn other_only(_: u64) -> Plural {
    Plural::Other
}

/// English wrapped in guillemets, with two deliberate differences from English: `repo.location_branch` puts its
/// placeholders in the other order, and the numbers use `.` for groups and `,` for decimals.
fn qaa() -> &'static LangDef {
    static DEF: std::sync::OnceLock<&'static LangDef> = std::sync::OnceLock::new();
    DEF.get_or_init(|| {
        let entries: Vec<Entry> = shipped()[0]
            .entries
            .iter()
            .map(|e| {
                let text = match e.key {
                    "repo.location_branch" => "{branch} / {point} ‹branch›".to_owned(),
                    // a counted message has one form in a language with no plural
                    "history.patch_count" => "{count} ‹patch›".to_owned(),
                    _ => format!("‹{}›", e.other),
                };
                let text: &'static str = Box::leak(text.into_boxed_str());
                Entry {
                    key: e.key,
                    one: text,
                    other: text,
                }
            })
            .collect();
        Box::leak(Box::new(LangDef {
            tag: "qaa",
            endonym: "Test language",
            dir: Dir::Rtl,
            plural: other_only,
            group_separator: '.',
            decimal_separator: ',',
            entries: Box::leak(entries.into_boxed_slice()),
        }))
    })
}

fn qaa_lang() -> Lang {
    register_test_language(qaa());
    Lang::from_def(qaa())
}

// ---------------------------------------------------------------- the router

fn app() -> Router {
    qaa_lang();
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("i18n-root"),
        store,
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        Arc::new(hosting),
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    router(AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth: Arc::new(Authenticator::new(
            Arc::new(InMemoryAccountStore::new()),
            Arc::new(InMemoryCredentialStore::new()),
            hasher.clone(),
            hasher,
        )),
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::default()),
        oidc: None,
        transport: None,
    })
}

type Reply = (StatusCode, HeaderMap, String);

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let resp = r.clone().oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get(uri: &str, accept: Option<&str>, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri);
    if let Some(a) = accept {
        b = b.header(header::ACCEPT_LANGUAGE, a);
    }
    let cookie = match cookie {
        Some(c) => format!("planeter_csrf=t; {c}"),
        None => "planeter_csrf=t".to_owned(),
    };
    b.header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn h<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

// ---------------------------------------------------------------- negotiation, cookie, lang/dir, Vary

#[tokio::test]
async fn english_is_the_default_and_the_page_says_so() {
    let r = app();
    let (s, headers, page) = send(&r, get("/", None, None)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(page.contains("<html lang=\"en\" dir=\"ltr\">"), "{page}");
    assert!(page.contains(">Open repository</button>"), "{page}");
    assert_eq!(h(&headers, "content-language"), "en");
    assert_eq!(
        h(&headers, "vary"),
        "Cookie, Authorization, Accept-Language"
    );
}

#[tokio::test]
async fn accept_language_selects_a_language_by_q_value_and_unknown_ones_fall_back_to_english() {
    let r = app();
    let (_, headers, page) = send(&r, get("/", Some("qaa"), None)).await;
    assert!(page.contains("<html lang=\"qaa\" dir=\"rtl\">"), "{page}");
    assert!(page.contains(">‹Open repository›</button>"), "{page}");
    assert!(
        page.contains("<title>‹Find a repository · planeter›</title>"),
        "the product name is not translated: {page}"
    );
    assert_eq!(h(&headers, "content-language"), "qaa");
    assert_eq!(
        h(&headers, "vary"),
        "Cookie, Authorization, Accept-Language"
    );

    for (accept, want) in [
        ("en;q=0.5, qaa;q=0.9", "qaa"),
        ("qaa;q=0.5, en;q=0.9", "en"),
        ("qaa-XX", "qaa"),
        ("qaa;q=0", "en"),
        ("fr, de;q=0.5", "en"),
        ("*", "en"),
        ("", "en"),
    ] {
        let (_, _, page) = send(&r, get("/", Some(accept), None)).await;
        assert!(
            page.contains(&format!("<html lang=\"{want}\"")),
            "{accept:?} → {want}: {page}"
        );
    }
}

#[tokio::test]
async fn the_language_cookie_beats_accept_language_and_an_unknown_value_is_ignored() {
    let r = app();
    // The cookie wins over the header, both ways.
    let (_, _, page) = send(&r, get("/", Some("en"), Some("planeter_lang=qaa"))).await;
    assert!(page.contains("lang=\"qaa\""), "{page}");
    let (_, _, page) = send(&r, get("/", Some("qaa"), Some("planeter_lang=en"))).await;
    assert!(page.contains("lang=\"en\""), "{page}");
    // A code that names no language is ignored: the header decides.
    let (_, _, page) = send(&r, get("/", Some("qaa"), Some("planeter_lang=zz"))).await;
    assert!(page.contains("lang=\"qaa\""), "{page}");
    let (_, _, page) = send(&r, get("/", None, Some("planeter_lang=<script>"))).await;
    assert!(page.contains("lang=\"en\""), "{page}");
    // The cookie holds a language code and nothing else, and is not HttpOnly (nothing secret).
    let c = planeter_web::catalog::lang_cookie("qaa");
    assert!(
        c.starts_with("planeter_lang=qaa;")
            && c.contains("SameSite=Lax")
            && !c.contains("HttpOnly"),
        "{c}"
    );
}

// ---------------------------------------------------------------- plurals, placeholders, numbers

#[test]
fn plural_and_placeholder_rules_follow_the_language() {
    let en = T::new(EN);
    assert_eq!(en.msg(Key::HistoryPatchCount).n(1).text(), "1 patch");
    assert_eq!(en.msg(Key::HistoryPatchCount).n(0).text(), "0 patches");
    assert_eq!(
        en.msg(Key::HistoryPatchCount).n(1200).text(),
        "1,200 patches"
    );

    let q = T::new(qaa_lang());
    // No grammatical plural: one form for every count, in the language's number format.
    assert_eq!(q.msg(Key::HistoryPatchCount).n(1).text(), "1 ‹patch›");
    assert_eq!(
        q.msg(Key::HistoryPatchCount).n(1200).text(),
        "1.200 ‹patch›"
    );
    // Another word order: the placeholders are named, so the catalog may reorder them.
    assert_eq!(
        q.msg(Key::RepoLocationBranch)
            .with("point", "heads/main")
            .with("branch", "heads/dev")
            .text(),
        "heads/dev / heads/main ‹branch›"
    );
}

#[test]
fn sizes_use_the_unit_and_the_number_format_of_the_catalog() {
    let en = T::new(EN);
    assert_eq!(en.size(0), "0 B");
    assert_eq!(en.size(223), "223 B");
    assert_eq!(en.size(1023), "1,023 B");
    assert_eq!(en.size(1024), "1.0 KiB");
    assert_eq!(en.size(2048), "2.0 KiB");
    assert_eq!(en.size(5 * 1024 * 1024), "5.0 MiB");
    assert_eq!(en.size(1536 * 1024), "1.5 MiB");
    assert_eq!(en.size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    let q = T::new(qaa_lang());
    assert_eq!(q.size(5 * 1024 * 1024), "‹5,0 MiB›");
    assert_eq!(q.size(1500), "‹1,5 KiB›");
}

#[test]
fn a_missing_key_in_a_registered_language_is_a_visible_marker_never_a_crash_or_english() {
    let partial: &'static LangDef = Box::leak(Box::new(LangDef {
        tag: "qab",
        endonym: "Partial",
        dir: Dir::Ltr,
        plural: other_only,
        group_separator: ',',
        decimal_separator: '.',
        entries: Box::leak(
            vec![Entry {
                key: "login.heading",
                one: "‹Sign in›",
                other: "‹Sign in›",
            }]
            .into_boxed_slice(),
        ),
    }));
    let t = T::new(Lang::from_def(partial));
    assert_eq!(t.get(Key::LoginHeading), "‹Sign in›");
    assert_eq!(t.get(Key::LoginFailed), "[login.failed]");
    assert!(
        audit(&[shipped()[0], partial]).len() > 100,
        "the audit sees every missing key"
    );
}

// ---------------------------------------------------------------- the API's message and hint

#[tokio::test]
async fn api_errors_follow_accept_language_with_the_code_status_and_shape_unchanged() {
    let r = app();
    let (s_en, h_en, b_en) = send(&r, get("/api/v1/nonsense", None, None)).await;
    let (s_q, h_q, b_q) = send(&r, get("/api/v1/nonsense", Some("qaa"), None)).await;
    assert_eq!((s_en, s_q), (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND));
    let en: serde_json::Value = serde_json::from_str(&b_en).unwrap();
    let q: serde_json::Value = serde_json::from_str(&b_q).unwrap();
    assert_eq!(
        b_en,
        ErrorCode::NotFound.body(),
        "English is byte-for-byte what it was"
    );
    assert_eq!(en["code"], "not_found");
    assert_eq!(q["code"], "not_found", "the code never changes");
    assert_eq!(q["message"], "‹No such resource.›");
    assert!(q["hint"].as_str().unwrap().starts_with('‹'), "{q}");
    assert_eq!(q.as_object().unwrap().len(), 3);
    assert_eq!(h(&h_en, "content-language"), "en");
    assert_eq!(h(&h_q, "content-language"), "qaa");
    assert_eq!(h(&h_q, "content-type"), "application/json");
    assert_eq!(h(&h_q, "vary"), "Cookie, Authorization, Accept-Language");

    // A hint with a value the code supplies (the OpenAPI path), and the status line is the same.
    let (s, _, b) = send(
        &r,
        get(
            "/api/v1/repos/alice/open/history?limit=x",
            Some("qaa"),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["code"], "invalid_request");
    assert_eq!(
        v["hint"],
        "‹Check the request against the OpenAPI document at /api/v1/openapi.json.›"
    );

    // A wrong method is answered in the language too, with `Allow: GET` kept.
    let post = Request::builder()
        .method("POST")
        .uri("/api/v1/openapi.json")
        .header(header::ACCEPT_LANGUAGE, "qaa")
        .body(Body::empty())
        .unwrap();
    let (s, headers, b) = send(&r, post).await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(h(&headers, "allow"), "GET");
    assert!(
        b.contains("\"message\":\"‹That method is not supported here.›\""),
        "{b}"
    );

    // A successful document does not vary by language.
    let (_, headers, _) = send(&r, get("/api/v1/openapi.json", Some("qaa"), None)).await;
    assert_eq!(h(&headers, "vary"), "Cookie, Authorization");
    assert_eq!(h(&headers, "content-language"), "");
}

#[test]
fn the_two_hints_of_too_large_are_two_keys_chosen_at_the_call_site() {
    let q = qaa_lang();
    let general = ErrorCode::TooLarge.body_for(ErrorCode::TooLarge.keys(), q);
    let raw = ErrorCode::TooLarge.body_for(ErrorCode::TOO_LARGE_RAW, q);
    let (g, r): (serde_json::Value, serde_json::Value) = (
        serde_json::from_str(&general).unwrap(),
        serde_json::from_str(&raw).unwrap(),
    );
    assert_eq!(g["message"], r["message"], "one code, one message");
    assert_ne!(g["hint"], r["hint"]);
    assert_eq!(
        r["hint"],
        "‹This server does not serve files that large; ask the person who runs this server.›"
    );
    assert_eq!(ErrorCode::TOO_LARGE_RAW.hint, Key::ErrTooLargeRawHint);
}

// ---------------------------------------------------------------- pages that used to be bare strings

#[tokio::test]
async fn the_former_bare_strings_are_pages_in_the_callers_language_with_their_status_codes() {
    let r = app();
    let post = |uri: &str, cookie: Option<&str>, accept: Option<&str>, body: &str| {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        if let Some(a) = accept {
            b = b.header(header::ACCEPT_LANGUAGE, a);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    };
    // CSRF failure: 403 and a page with a heading, one sentence and one link.
    let (s, headers, page) = send(
        &r,
        post(
            "/login",
            Some("planeter_csrf=aaa"),
            None,
            "csrf=bbb&username=a&password=b",
        ),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(
        page.contains("<h1>The request was not accepted</h1>"),
        "{page}"
    );
    assert!(
        page.contains("<title>The request was not accepted · planeter</title>"),
        "{page}"
    );
    assert!(
        page.contains("<a href=\"/login\">Go to the sign-in page</a>"),
        "{page}"
    );
    assert_eq!(h(&headers, "content-type"), "text/html; charset=utf-8");
    // …in the test language, with the same status.
    let (s, _, page) = send(
        &r,
        post(
            "/login",
            Some("planeter_csrf=aaa"),
            Some("qaa"),
            "csrf=bbb&username=a&password=b",
        ),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(
        page.contains("<h1>‹The request was not accepted›</h1>")
            && page.contains("lang=\"qaa\" dir=\"rtl\""),
        "{page}"
    );
    // An incomplete form is 422 (it was axum's text), a missing address is 400 (the home page again, with the error).
    let (s, _, page) = send(
        &r,
        post("/login", Some("planeter_csrf=aaa"), None, "csrf=aaa"),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        page.contains("<h1>The form was incomplete</h1>")
            && !page.contains("Failed to deserialize"),
        "{page}"
    );
    let (s, _, page) = send(&r, get("/go", None, None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // …shown again beside the field, which is marked invalid and tied to the message.
    assert!(
        page.contains("<h1>planeter</h1>")
            && page.contains("aria-invalid=\"true\" aria-describedby=\"repo-error\"")
            && page.contains(
                "<p id=\"repo-error\" class=\"badge bad\">That is not a repository address."
            )
            && !page.contains("missing field"),
        "{page}"
    );
    // SSO is not configured in this app: 404 pages, no internal text.
    let (s, _, page) = send(&r, get("/login/oidc", None, None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(
        page.contains("<h1>Single sign-on is not set up</h1>"),
        "{page}"
    );
    let (s, _, page) = send(&r, get("/login/oidc/callback?code=x&state=y", None, None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(page.contains("Single sign-on is not set up"), "{page}");
}

// ---------------------------------------------------------------- T2: typed slots cannot inject markup

fn tags_of(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        let j = rest[i..].find('>').map(|j| j + i).unwrap_or(rest.len() - 1);
        out.push(rest[i..=j].to_owned());
        rest = &rest[j + 1..];
    }
    out
}

fn only_expected_tags(html: &str) {
    for tag in tags_of(html) {
        let ok = tag == "<code>"
            || tag == "</code>"
            || tag == "<span class=\"muted\">"
            || tag == "</span>"
            || tag == "</a>"
            || (tag.starts_with("<a href=\"")
                && tag.ends_with("\">")
                // the value holds no quote, so it cannot end the attribute and start another
                && tag[9..tag.len() - 2].chars().all(|c| c != '"' && c != '<' && c != '>'));
        assert!(ok, "unexpected markup {tag:?} in {html}");
    }
}

const HOSTILE: [&str; 6] = [
    "<script>alert(1)</script>",
    "\"><img src=x onerror=alert(1)>",
    "' onmouseover='alert(1)",
    "</code><a href=\"javascript:alert(1)\">x</a>",
    "&lt;b&gt; &amp; <b>",
    "{download}{ref}{command}",
];

#[test]
fn a_hostile_value_in_a_text_code_muted_or_link_slot_cannot_inject_markup_or_an_attribute() {
    let t = T::new(EN);
    for v in HOSTILE {
        // Every slot kind, in the sentences that have them.
        for html in [
            t.msg(Key::HomeIntro).code("address", v).html(),
            t.msg(Key::HistoryHeading).muted("ref", v).html(),
            t.msg(Key::RepoLocationBranch)
                .code("point", v)
                .code("branch", v)
                .html(),
            t.msg(Key::FileBinary).link("download", v, "/raw/x").html(),
            t.msg(Key::TitleChange)
                .with("target", v)
                .with("repo", v)
                .html(),
            t.msg(Key::HistoryPatchCount).n(3).html(),
        ] {
            only_expected_tags(html.as_str());
            assert!(
                !html.as_str().contains(v)
                    || !v.contains('<') && !v.contains('"')
                    || v.contains("{download}"),
                "unescaped {v:?} in {html}"
            );
        }
    }
    // A value that looks like a placeholder is shown as it is: values are never scanned for placeholders.
    assert_eq!(
        t.msg(Key::TitleRefs)
            .with("repo", "{repo}")
            .product()
            .text(),
        "Branches and tags · {repo} · planeter"
    );
    // The plain-text rendering is not HTML and is escaped by whoever prints it (askama, JSON).
    assert_eq!(
        t.msg(Key::TitleChange)
            .with("target", "<b>")
            .with("repo", "r")
            .product()
            .text(),
        "Change <b> · r · planeter"
    );
}

#[test]
fn a_link_href_comes_from_code_and_only_a_local_path_or_an_http_url_becomes_a_link() {
    let t = T::new(EN);
    for (href, linked) in [
        ("/x/y?ref=heads/main", true),
        ("https://raw.example/a?b=c", true),
        ("http://raw.example/a", true),
        ("javascript:alert(1)", false),
        ("data:text/html,<script>", false),
        ("//evil.example/x", false),
        ("/ok onmouseover=alert(1)", false),
        ("", false),
        ("relative/path", false),
        ("\u{0}/x", false),
    ] {
        let html = t.msg(Key::FileBinary).link("download", "go", href).html();
        only_expected_tags(html.as_str());
        assert_eq!(
            html.as_str().contains("<a href="),
            linked,
            "{href:?}: {html}"
        );
        assert!(
            html.as_str().contains("go"),
            "the label always shows: {html}"
        );
    }
    // A quote in an allowed-looking href cannot end the attribute.
    let html = t
        .msg(Key::FileBinary)
        .link("download", "go", "/x\"onmouseover=\"y")
        .html();
    only_expected_tags(html.as_str());
    assert!(html.as_str().contains("&quot;"), "{html}");
}

#[test]
fn a_catalog_entry_cannot_introduce_a_tag_an_attribute_or_a_link_by_itself() {
    let evil: &'static LangDef = Box::leak(Box::new(LangDef {
        tag: "qac",
        endonym: "Evil",
        dir: Dir::Ltr,
        plural: other_only,
        group_separator: ',',
        decimal_separator: '.',
        entries: Box::leak(
            vec![
                Entry {
                    key: "file.binary",
                    one: "<b onclick=\"x\">{download}</b> <a href=\"/steal\">go</a>",
                    other: "<b onclick=\"x\">{download}</b> <a href=\"/steal\">go</a>",
                },
                Entry {
                    key: "login.failed",
                    one: "<script>alert(1)</script>",
                    other: "<script>alert(1)</script>",
                },
            ]
            .into_boxed_slice(),
        ),
    }));
    let t = T::new(Lang::from_def(evil));
    let html = t.msg(Key::FileBinary).link("download", "dl", "/raw").html();
    // The catalog's markup is text; only the one typed link slot is an element.
    assert!(
        html.as_str().contains("&lt;b onclick=&quot;x&quot;&gt;"),
        "{html}"
    );
    assert!(
        html.as_str().contains("&lt;a href=&quot;/steal&quot;&gt;"),
        "{html}"
    );
    assert_eq!(html.as_str().matches("<a ").count(), 1, "{html}");
    assert!(
        !t.msg(Key::LoginFailed).html().as_str().contains("<script"),
        "{}",
        t.msg(Key::LoginFailed).html()
    );
    // …and the build refuses such an entry in the first place.
    let p = audit(&[shipped()[0], evil]);
    assert!(
        p.iter().any(|x| matches!(
            x,
            planeter_web::catalog::Problem::Markup {
                lang: "qac",
                key: "file.binary"
            }
        )),
        "{p:?}"
    );
    assert!(
        p.iter().any(|x| matches!(
            x,
            planeter_web::catalog::Problem::Markup {
                lang: "qac",
                key: "login.failed"
            }
        )),
        "{p:?}"
    );
}

#[test]
fn askama_renders_catalog_html_as_is_and_still_escapes_everything_else() {
    #[derive(Template)]
    #[template(
        source = "<p>{{ t.msg(Key::FileBinary).link(\"download\", label, href).html() }}</p><p>{{ t.get(Key::LoginFailed) }}</p><p>{{ other }}</p>",
        ext = "html"
    )]
    struct P {
        t: T,
        label: String,
        href: String,
        other: String,
    }
    let out = P {
        t: T::new(EN),
        label: "<i>go</i>".into(),
        href: "/raw?a=1&b=2".into(),
        other: "<script>".into(),
    }
    .render()
    .unwrap();
    // Not escaped a second time (`&amp;lt;`), and the query ampersand is an entity once.
    assert!(
        out.contains("<a href=\"/raw?a=1&amp;b=2\">&lt;i&gt;go&lt;/i&gt;</a>"),
        "{out}"
    );
    assert!(
        !out.contains("&amp;lt;") && !out.contains("&amp;amp;"),
        "{out}"
    );
    assert!(
        out.contains("<p>The username or password was not correct. Check both and try again.</p>")
            && out.contains("<p>&#60;script&#62;</p>"),
        "{out}"
    );
}
