//! RFC 012 D-3b, amended 2026-11-04 (handoff `language-registry-handoff-v1` T4, T5, T9): every shipped language other than the source is
//! negotiable by `Accept-Language` and by the `planeter_lang` cookie, with no operator setting; the page says its `lang` and `dir`;
//! English stays the default for a caller who does not ask; and the switcher in the footer of every page — the sign-in page included —
//! chooses between the languages with a CSRF-protected POST that sets the cookie and returns to the same page, with no script. With
//! four languages or more the switcher is one `<details>` menu (a fourth language is registered by the test to prove it).
//! Nothing here names a language: the "other" language is the first shipped one that is not the source.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore,
};
use planeter_web::catalog::{Key, Lang, LangDef, T, shipped};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn app() -> Router {
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("language-switching-root"),
        store as Arc<dyn RepositoryStore>,
    )
    .with_sandbox(planeter_prikk::Sandbox::Unconfined);
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
        creation: None,
    })
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let resp = r.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn get(uri: &str, accept: Option<&str>, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri);
    if let Some(a) = accept {
        b = b.header(header::ACCEPT_LANGUAGE, a);
    }
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

fn post_lang(form: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/lang")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, cookie)
        .body(Body::from(form.to_owned()))
        .unwrap()
}

fn header_str<'a>(r: &'a Reply, name: &str) -> &'a str {
    r.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

fn set_cookies(r: &Reply) -> Vec<String> {
    r.headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect()
}

/// The language that is not the source: the first shipped one.
fn other() -> &'static LangDef {
    shipped()
        .iter()
        .copied()
        .find(|d| !d.is_source())
        .expect("a language besides the source ships")
}

fn words(key: Key) -> String {
    T::new(Lang::from_def(other())).get(key)
}

fn source_words(key: Key) -> String {
    T::new(planeter_web::catalog::EN).get(key)
}

fn body_lang(tag: &str, dir: &str) -> String {
    format!("<html lang=\"{tag}\" dir=\"{dir}\">")
}

#[tokio::test]
async fn the_language_is_negotiated_by_accept_language_and_by_the_cookie_with_no_operator_setting()
{
    let r = app();
    let (tag, src) = (other().tag, planeter_web::catalog::EN.tag());
    let dir = Lang::from_def(other()).dir();
    for accept in [
        tag.to_owned(),
        format!("{tag}-XX"),
        format!("{tag}-XX,{src};q=0.5"),
        format!("{src};q=0.4,{tag};q=0.9"),
    ] {
        let page = send(&r, get("/", Some(&accept), None)).await;
        assert_eq!(page.status, StatusCode::OK, "{accept}");
        assert!(
            page.body.contains(&body_lang(tag, dir)),
            "{accept}: {}",
            page.body
        );
        assert_eq!(header_str(&page, "content-language"), tag, "{accept}");
        assert!(
            header_str(&page, "vary").contains("Accept-Language"),
            "{accept}: vary {}",
            header_str(&page, "vary")
        );
        let title = T::new(Lang::from_def(other()))
            .msg(Key::TitleHome)
            .product()
            .text();
        assert!(
            page.body.contains(&format!("<title>{title}</title>")),
            "{accept}: {title}"
        );
    }
    // English first, or nothing: the source language is the default.
    for accept in [
        src.to_owned(),
        format!("{src}-GB,{tag};q=0.3"),
        String::new(),
        "fr".to_owned(),
    ] {
        let page = send(&r, get("/", Some(&accept), None)).await;
        assert!(
            page.body.contains(&body_lang(src, "ltr")),
            "{accept:?}: {}",
            page.body
        );
    }
    // The cookie beats the header, both ways.
    let page = send(
        &r,
        get("/", Some(src), Some(&format!("planeter_lang={tag}"))),
    )
    .await;
    assert!(
        page.body.contains(&format!("<html lang=\"{tag}\"")),
        "{}",
        page.body
    );
    let page = send(
        &r,
        get("/", Some(tag), Some(&format!("planeter_lang={src}"))),
    )
    .await;
    assert!(
        page.body.contains(&format!("<html lang=\"{src}\"")),
        "{}",
        page.body
    );
    // A cookie that names nothing we have does not break the page.
    let page = send(&r, get("/", Some(tag), Some("planeter_lang=xx"))).await;
    assert!(page.body.contains(&format!("<html lang=\"{tag}\"")));
}

#[tokio::test]
async fn the_sign_in_page_and_a_not_found_page_are_in_the_language_with_the_catalog_words() {
    let r = app();
    let tag = other().tag;
    let login = send(&r, get("/login", Some(tag), None)).await;
    assert!(
        login
            .body
            .contains(&format!("<h1>{}</h1>", words(Key::LoginHeading))),
        "{}",
        login.body
    );
    assert!(
        login.body.contains(&words(Key::LoginUser)),
        "{}",
        login.body
    );
    let missing = send(&r, get("/alice/none", Some(tag), None)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(
        missing
            .body
            .contains(&format!("<h1>{}</h1>", words(Key::NotFoundHeading))),
        "{}",
        missing.body
    );
    assert_eq!(header_str(&missing, "content-language"), tag);
    // The words are not the source language's.
    assert_ne!(words(Key::LoginHeading), source_words(Key::LoginHeading));
}

/// Every form in the footer: its language code, endonym, `lang`, the current one marked, a CSRF token and the address to return to.
/// With up to three languages the buttons are inline.
#[tokio::test]
async fn the_switcher_is_in_the_footer_of_every_page_and_marks_the_current_language() {
    let r = app();
    let (o, s) = (other(), shipped()[0]);
    assert!(
        shipped().len() <= 3,
        "this binary tests the inline rendering; tests/language_menu.rs the menu"
    );
    for (uri, accept, current, aria_label) in [
        ("/", s.tag, s.tag, source_words(Key::LayoutLanguage)),
        ("/login", s.tag, s.tag, source_words(Key::LayoutLanguage)),
        ("/", o.tag, o.tag, words(Key::LayoutLanguage)),
        ("/login", o.tag, o.tag, words(Key::LayoutLanguage)),
        ("/alice/none", o.tag, o.tag, words(Key::LayoutLanguage)),
    ] {
        let page = send(&r, get(uri, Some(accept), None)).await;
        let footer = &page.body[page.body.find("<footer>").expect("a footer")..];
        assert!(
            footer.contains(&format!("<nav class=\"lang\" aria-label=\"{aria_label}\">")),
            "{uri} {accept}: {footer}"
        );
        assert!(
            !footer.contains("<details"),
            "up to three languages are inline buttons: {footer}"
        );
        // One real form per language, POSTing to /lang, with its endonym as the button text and `lang` on it.
        for d in shipped() {
            let (tag, endonym) = (d.tag, d.endonym);
            assert!(
                footer.contains(&format!(
                    "<input type=\"hidden\" name=\"lang\" value=\"{tag}\">"
                )),
                "{uri} {accept}: no form for {tag}"
            );
            let button = format!(
                "<button type=\"submit\" lang=\"{tag}\"{}>{endonym}</button>",
                if tag == current {
                    " aria-current=\"true\""
                } else {
                    ""
                }
            );
            assert!(
                footer.contains(&button),
                "{uri} {accept}: {button} in {footer}"
            );
        }
        assert_eq!(footer.matches("action=\"/lang\"").count(), shipped().len());
        assert!(footer.contains("name=\"csrf\""));
        assert!(
            footer.contains(&format!("name=\"next\" value=\"{uri}\"")),
            "{uri} {accept}: the return address is the page: {footer}"
        );
        // No script anywhere on the page.
        assert!(!page.body.to_lowercase().contains("<script"));
    }
    // The return address carries a query, and never a sign-in callback's one-time code.
    let page = send(
        &r,
        get("/alice/none/tree/src?ref=heads/dev", Some(o.tag), None),
    )
    .await;
    assert!(
        page.body
            .contains("name=\"next\" value=\"/alice/none/tree/src?ref=heads/dev\""),
        "{}",
        page.body
    );
    let callback = send(
        &r,
        get(
            "/login/oidc/callback?code=SECRET&state=s",
            Some(o.tag),
            None,
        ),
    )
    .await;
    assert!(
        !callback.body.contains("SECRET"),
        "the one-time code must not be copied into the page: {}",
        callback.body
    );
}

async fn csrf_and_cookie(r: &Router) -> (String, String) {
    let first = send(r, get("/login", Some("en"), None)).await;
    let csrf = set_cookies(&first)
        .into_iter()
        .find_map(|c| {
            c.strip_prefix("planeter_csrf=")
                .map(|v| v.split(';').next().unwrap().to_owned())
        })
        .expect("a CSRF cookie");
    let cookie = format!("planeter_csrf={csrf}");
    (csrf, cookie)
}

#[tokio::test]
async fn choosing_a_language_sets_the_cookie_and_returns_to_the_same_page() {
    let r = app();
    let (tag, src) = (other().tag, planeter_web::catalog::EN.tag());
    let (csrf, cookie) = csrf_and_cookie(&r).await;
    let reply = send(
        &r,
        post_lang(
            &format!("csrf={csrf}&lang={tag}&next=/alice/none/history"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(header_str(&reply, "location"), "/alice/none/history");
    let lang_cookie = set_cookies(&reply)
        .into_iter()
        .find(|c| c.starts_with("planeter_lang="))
        .expect("the language cookie");
    assert!(
        lang_cookie.starts_with(&format!("planeter_lang={tag};")),
        "{lang_cookie}"
    );
    for attr in ["Path=/", "Secure", "SameSite=Lax"] {
        assert!(lang_cookie.contains(attr), "{lang_cookie}");
    }
    assert!(
        !lang_cookie.contains("HttpOnly"),
        "nothing secret in it: {lang_cookie}"
    );
    // The next page, with that cookie and a browser that asks for the source language, is in the chosen one.
    let page = send(
        &r,
        get(
            "/alice/none/history",
            Some(src),
            Some(&format!("{cookie}; planeter_lang={tag}")),
        ),
    )
    .await;
    assert!(
        page.body.contains(&format!("<html lang=\"{tag}\"")),
        "{}",
        page.body
    );
    // And back.
    let reply = send(
        &r,
        post_lang(&format!("csrf={csrf}&lang={src}&next=/"), &cookie),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert!(
        set_cookies(&reply)
            .iter()
            .any(|c| c.starts_with(&format!("planeter_lang={src};")))
    );
    // A tag with a region is normalised to the code we serve.
    let reply = send(
        &r,
        post_lang(&format!("csrf={csrf}&lang={tag}-XX&next=/"), &cookie),
    )
    .await;
    assert!(
        set_cookies(&reply)
            .iter()
            .any(|c| c.starts_with(&format!("planeter_lang={tag};")))
    );
}

#[tokio::test]
async fn the_switcher_refuses_a_forged_form_an_unknown_language_and_an_open_redirect() {
    let r = app();
    let tag = other().tag;
    let (csrf, cookie) = csrf_and_cookie(&r).await;
    // No token, a wrong token, no cookie: refused, nothing set.
    for (form, cookie) in [
        (format!("lang={tag}&next=/"), cookie.clone()),
        (format!("csrf=wrong&lang={tag}&next=/"), cookie.clone()),
        (
            format!("csrf={csrf}&lang={tag}&next=/"),
            "planeter_other=1".to_owned(),
        ),
    ] {
        let reply = send(&r, post_lang(&form, &cookie)).await;
        assert!(
            matches!(
                reply.status,
                StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY
            ),
            "{form}: {}",
            reply.status
        );
        assert!(
            !set_cookies(&reply)
                .iter()
                .any(|c| c.starts_with("planeter_lang=")),
            "{form}: a refused form must not set the language"
        );
    }
    // An unknown language: refused, not defaulted.
    for lang in [
        "xx".to_owned(),
        String::new(),
        format!("{tag};evil=1"),
        "en\r\nSet-Cookie: x=1".to_owned(),
    ] {
        let encoded: String = lang.bytes().map(|b| format!("%{b:02X}")).collect();
        let reply = send(
            &r,
            post_lang(&format!("csrf={csrf}&lang={encoded}&next=/"), &cookie),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{lang:?}");
        assert!(
            !set_cookies(&reply)
                .iter()
                .any(|c| c.starts_with("planeter_lang="))
        );
    }
    // The return address is a local path or `/`.
    for next in [
        "//evil.example/x",
        "https://evil.example/",
        "/\\evil.example",
        "javascript:alert(1)",
        "",
        "/a b",
    ] {
        let encoded: String = next.bytes().map(|b| format!("%{b:02X}")).collect();
        let reply = send(
            &r,
            post_lang(&format!("csrf={csrf}&lang={tag}&next={encoded}"), &cookie),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{next:?}");
        assert_eq!(header_str(&reply, "location"), "/", "{next:?}");
    }
    // GET is not the way to change it.
    let reply = send(&r, get("/lang", None, None)).await;
    assert!(
        matches!(
            reply.status,
            StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_FOUND
        ),
        "{}",
        reply.status
    );
}

#[test]
fn the_registry_ships_the_source_first_and_a_language_besides_it_without_any_setting() {
    assert!(shipped()[0].is_source());
    assert!(
        shipped().len() >= 2,
        "a language besides the source is shipped"
    );
    assert!(shipped().iter().all(|d| d.is_shipped()));
    assert_eq!(
        Lang::negotiate(None, None).tag(),
        shipped()[0].tag,
        "a caller who does not ask gets the source language"
    );
    let tag = other().tag;
    assert_eq!(Lang::negotiate(Some(tag), None).tag(), tag);
    assert_eq!(
        Lang::negotiate(Some("fr,de;q=0.8"), None).tag(),
        shipped()[0].tag
    );
    assert_eq!(
        Lang::negotiate(Some("en"), Some(tag)).tag(),
        tag,
        "the cookie beats Accept-Language"
    );
}

/// Prints the footer of the sign-in page with the shipped languages, for the review request: `cargo test --test language_switching dump -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "prints the footer"]
async fn dump_the_footer_with_the_shipped_languages() {
    let r = app();
    let page = send(&r, get("/login", Some("en"), None)).await;
    println!("{}", &page.body[page.body.find("<footer>").unwrap()..]);
}
