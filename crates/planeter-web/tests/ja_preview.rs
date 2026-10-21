//! RFC 012 D-3b / D-3f, handoff T4 and T7: with the operator's preview on, Japanese is negotiable by
//! `Accept-Language` (`ja`, `ja-JP`) and by the `planeter_lang` cookie, the page says `lang="ja" dir="ltr"`,
//! and the switcher in the footer of every page — the sign-in page included — chooses between English and
//! 日本語 with a CSRF-protected POST that sets the cookie and returns to the same page, with no script.
//! (`tests/ja_not_offered.rs` is the other half: the same without the preview.)

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
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn app() -> Router {
    planeter_web::catalog::enable_preview_languages("ja").expect("ja is previewable");
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ja-preview-root"),
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

#[tokio::test]
async fn japanese_is_negotiated_by_accept_language_and_by_the_cookie() {
    let r = app();
    for accept in ["ja", "ja-JP", "ja-JP,en;q=0.5", "en;q=0.4,ja;q=0.9"] {
        let page = send(&r, get("/", Some(accept), None)).await;
        assert_eq!(page.status, StatusCode::OK, "{accept}");
        assert!(
            page.body.contains("<html lang=\"ja\" dir=\"ltr\">"),
            "{accept}: {}",
            page.body
        );
        assert_eq!(header_str(&page, "content-language"), "ja", "{accept}");
        assert!(
            header_str(&page, "vary").contains("Accept-Language"),
            "{accept}: vary {}",
            header_str(&page, "vary")
        );
        assert!(
            page.body
                .contains("<title>リポジトリを探す · planeter</title>")
        );
    }
    // English first, or nothing: English.
    for accept in ["en", "en-GB,ja;q=0.3", ""] {
        let page = send(&r, get("/", Some(accept), None)).await;
        assert!(
            page.body.contains("<html lang=\"en\" dir=\"ltr\">"),
            "{accept}: {}",
            page.body
        );
    }
    // The cookie beats the header, both ways.
    let page = send(&r, get("/", Some("en"), Some("planeter_lang=ja"))).await;
    assert!(page.body.contains("<html lang=\"ja\""), "{}", page.body);
    let page = send(&r, get("/", Some("ja"), Some("planeter_lang=en"))).await;
    assert!(page.body.contains("<html lang=\"en\""), "{}", page.body);
    // A cookie that names nothing we have does not break the page.
    let page = send(&r, get("/", Some("ja"), Some("planeter_lang=xx"))).await;
    assert!(page.body.contains("<html lang=\"ja\""));
}

#[tokio::test]
async fn the_sign_in_page_and_a_not_found_page_are_in_japanese_with_the_catalog_words() {
    let r = app();
    let login = send(&r, get("/login", Some("ja"), None)).await;
    assert!(login.body.contains("<h1>ログイン</h1>"), "{}", login.body);
    assert!(login.body.contains("ユーザー名"), "{}", login.body);
    let missing = send(&r, get("/alice/none", Some("ja"), None)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(
        missing.body.contains("<h1>見つかりません</h1>"),
        "{}",
        missing.body
    );
    assert_eq!(header_str(&missing, "content-language"), "ja");
}

/// Every form in the footer: its language code, endonym, `lang`, the current one marked, a CSRF token and the
/// address to return to.
#[tokio::test]
async fn the_switcher_is_in_the_footer_of_every_page_and_marks_the_current_language() {
    let r = app();
    for (uri, accept, current, aria_label) in [
        ("/", "en", "en", "Language"),
        ("/login", "en", "en", "Language"),
        ("/", "ja", "ja", "言語"),
        ("/login", "ja", "ja", "言語"),
        ("/alice/none", "ja", "ja", "言語"),
    ] {
        let page = send(&r, get(uri, Some(accept), None)).await;
        let footer = &page.body[page.body.find("<footer>").expect("a footer")..];
        assert!(
            footer.contains(&format!("<nav class=\"lang\" aria-label=\"{aria_label}\">")),
            "{uri} {accept}: {footer}"
        );
        // One real form per language, POSTing to /lang, with its endonym as the button text and `lang` on it.
        for (tag, endonym) in [("en", "English"), ("ja", "日本語")] {
            assert!(
                footer.contains(&format!(
                    "<input type=\"hidden\" name=\"lang\" value=\"{tag}\">"
                )),
                "{uri} {accept}: no form for {tag}"
            );
            let marked = tag == current;
            let button = format!(
                "<button type=\"submit\" lang=\"{tag}\"{}>{endonym}</button>",
                if marked { " aria-current=\"true\"" } else { "" }
            );
            assert!(
                footer.contains(&button),
                "{uri} {accept}: {button} in {footer}"
            );
        }
        assert_eq!(footer.matches("action=\"/lang\"").count(), 2);
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
        get("/alice/none/tree/src?ref=heads/dev", Some("ja"), None),
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
        get("/login/oidc/callback?code=SECRET&state=s", Some("ja"), None),
    )
    .await;
    assert!(
        !callback.body.contains("SECRET"),
        "the one-time code must not be copied into the page: {}",
        callback.body
    );
}

#[tokio::test]
async fn choosing_a_language_sets_the_cookie_and_returns_to_the_same_page() {
    let r = app();
    // Get a CSRF token the way a browser does: from the cookie the first page sets.
    let first = send(&r, get("/login", Some("en"), None)).await;
    let csrf = set_cookies(&first)
        .into_iter()
        .find_map(|c| {
            c.strip_prefix("planeter_csrf=")
                .map(|v| v.split(';').next().unwrap().to_owned())
        })
        .expect("a CSRF cookie");
    let cookie = format!("planeter_csrf={csrf}");

    let reply = send(
        &r,
        post_lang(
            &format!("csrf={csrf}&lang=ja&next=/alice/none/history"),
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
        lang_cookie.starts_with("planeter_lang=ja;"),
        "{lang_cookie}"
    );
    for attr in ["Path=/", "Secure", "SameSite=Lax"] {
        assert!(lang_cookie.contains(attr), "{lang_cookie}");
    }
    assert!(
        !lang_cookie.contains("HttpOnly"),
        "nothing secret in it: {lang_cookie}"
    );
    // The next page, with that cookie and a browser that asks for English, is Japanese.
    let page = send(
        &r,
        get(
            "/alice/none/history",
            Some("en"),
            Some(&format!("{cookie}; planeter_lang=ja")),
        ),
    )
    .await;
    assert!(page.body.contains("<html lang=\"ja\""), "{}", page.body);
    // And back to English.
    let reply = send(
        &r,
        post_lang(&format!("csrf={csrf}&lang=en&next=/"), &cookie),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert!(
        set_cookies(&reply)
            .iter()
            .any(|c| c.starts_with("planeter_lang=en;"))
    );
    // ja-JP is normalised to the code we serve.
    let reply = send(
        &r,
        post_lang(&format!("csrf={csrf}&lang=ja-JP&next=/"), &cookie),
    )
    .await;
    assert!(
        set_cookies(&reply)
            .iter()
            .any(|c| c.starts_with("planeter_lang=ja;"))
    );
}

#[tokio::test]
async fn the_switcher_refuses_a_forged_form_an_unknown_language_and_an_open_redirect() {
    let r = app();
    let first = send(&r, get("/login", Some("en"), None)).await;
    let csrf = set_cookies(&first)
        .into_iter()
        .find_map(|c| {
            c.strip_prefix("planeter_csrf=")
                .map(|v| v.split(';').next().unwrap().to_owned())
        })
        .unwrap();
    let cookie = format!("planeter_csrf={csrf}");
    // No token, a wrong token, no cookie: refused, nothing set.
    for (form, cookie) in [
        ("lang=ja&next=/".to_owned(), cookie.clone()),
        ("csrf=wrong&lang=ja&next=/".to_owned(), cookie.clone()),
        (
            format!("csrf={csrf}&lang=ja&next=/"),
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
    for lang in ["xx", "", "ja;evil=1", "en\r\nSet-Cookie: x=1"] {
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
            post_lang(&format!("csrf={csrf}&lang=ja&next={encoded}"), &cookie),
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
