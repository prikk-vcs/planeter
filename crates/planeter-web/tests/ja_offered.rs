//! RFC 012 D-3f, after the sign-off (japanese-final-handoff T3): **Japanese is offered without any operator setting**: it is in
//! `shipped()`, negotiated from `Accept-Language` and from the cookie, selectable by the switcher (which shows both choices, English
//! first), and English stays the default for a caller who does not ask. (This binary never names `PLANETER_PREVIEW_LANGUAGES`; the
//! other half, the setting still being accepted, is in `tests/ja_catalog.rs`.)

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
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ja-offered-root"),
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

async fn send(r: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let resp = r.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get(uri: &str, accept: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .uri(uri)
        .header(header::ACCEPT_LANGUAGE, accept);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

#[test]
fn japanese_is_shipped_and_negotiable_without_any_setting_and_english_stays_the_default() {
    assert!(
        planeter_web::catalog::shipped()
            .iter()
            .any(|d| d.tag == "ja")
    );
    assert_eq!(
        planeter_web::catalog::shipped()[0].tag,
        "en",
        "English first"
    );
    assert_eq!(
        planeter_web::catalog::Lang::from_tag("ja").unwrap().tag(),
        "ja"
    );
    assert_eq!(
        planeter_web::catalog::Lang::from_tag("ja-JP")
            .unwrap()
            .tag(),
        "ja"
    );
    assert_eq!(
        planeter_web::catalog::Lang::negotiate(Some("ja,ja-JP;q=0.9"), None).tag(),
        "ja"
    );
    assert_eq!(
        planeter_web::catalog::Lang::negotiate(Some("en"), Some("ja")).tag(),
        "ja",
        "the cookie beats Accept-Language"
    );
    // A caller who does not ask for Japanese is English: no header, an unrelated language, an unknown cookie.
    for (accept, cookie) in [
        (None, None),
        (Some("fr,de;q=0.8"), None),
        (Some("ja"), Some("zz")),
    ] {
        let expected = if cookie == Some("zz") { "ja" } else { "en" };
        assert_eq!(
            planeter_web::catalog::Lang::negotiate(accept, cookie).tag(),
            expected,
            "{accept:?} {cookie:?}"
        );
    }
}

#[tokio::test]
async fn the_pages_are_japanese_for_a_japanese_caller_and_the_switcher_shows_both_choices() {
    let r = app();
    for (accept, cookie) in [
        ("ja", None),
        ("ja-JP", None),
        ("en", Some("planeter_lang=ja")),
    ] {
        let (status, headers, body) = send(&r, get("/", accept, cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("<html lang=\"ja\" dir=\"ltr\">"),
            "{accept}: {body}"
        );
        assert_eq!(headers.get("content-language").unwrap(), "ja");
        let footer = &body[body.find("<footer>").unwrap()..];
        assert_eq!(
            footer.matches("action=\"/lang\"").count(),
            2,
            "both languages, never one: {footer}"
        );
        assert!(
            footer.contains(">English</button>") && footer.contains(">日本語</button>"),
            "{footer}"
        );
        assert!(
            footer.contains(
                "<button type=\"submit\" lang=\"ja\" aria-current=\"true\">日本語</button>"
            ),
            "{footer}"
        );
    }
    // An English caller sees English, with both buttons too.
    let (_, headers, body) = send(&r, get("/", "en", None)).await;
    assert!(body.contains("<html lang=\"en\" dir=\"ltr\">"));
    assert_eq!(headers.get("content-language").unwrap(), "en");
    let footer = &body[body.find("<footer>").unwrap()..];
    assert_eq!(footer.matches("action=\"/lang\"").count(), 2);
}

#[tokio::test]
async fn choosing_japanese_sets_the_cookie() {
    let r = app();
    let (_, headers, _) = send(&r, get("/login", "en", None)).await;
    let csrf = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| {
            c.strip_prefix("planeter_csrf=")
                .map(|v| v.split(';').next().unwrap().to_owned())
        })
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/lang")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("planeter_csrf={csrf}"))
        .body(Body::from(format!("csrf={csrf}&lang=ja&next=/")))
        .unwrap();
    let (status, headers, _) = send(&r, req).await;
    assert!(status.is_redirection(), "{status}");
    assert!(
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap_or("").starts_with("planeter_lang=ja"))
    );
}
