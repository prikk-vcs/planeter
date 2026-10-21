//! RFC 012 D-3f, handoff T6/T7: **without the operator's preview, Japanese is not offered**: not negotiated
//! from `Accept-Language`, not from the cookie, not selectable by the switcher (which shows English only), and
//! `shipped()` does not list it. (This binary never turns the preview on; `tests/ja_preview.rs` does.)

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
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ja-not-offered-root"),
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
fn japanese_is_not_shipped_and_not_negotiable_without_the_preview() {
    assert!(
        !planeter_web::catalog::shipped()
            .iter()
            .any(|d| d.tag == "ja"),
        "Japanese is not signed off"
    );
    assert!(planeter_web::catalog::Lang::from_tag("ja").is_none());
    assert!(planeter_web::catalog::Lang::from_tag("ja-JP").is_none());
    assert_eq!(
        planeter_web::catalog::Lang::negotiate(Some("ja,ja-JP;q=0.9"), None).tag(),
        "en"
    );
    assert_eq!(
        planeter_web::catalog::Lang::negotiate(Some("en"), Some("ja")).tag(),
        "en"
    );
}

#[tokio::test]
async fn the_pages_stay_english_and_the_switcher_shows_english_only() {
    let r = app();
    for (accept, cookie) in [
        ("ja", None),
        ("ja-JP", None),
        ("en", Some("planeter_lang=ja")),
    ] {
        let (status, headers, body) = send(&r, get("/", accept, cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("<html lang=\"en\" dir=\"ltr\">"),
            "{accept}: {body}"
        );
        assert_eq!(headers.get("content-language").unwrap(), "en");
        let footer = &body[body.find("<footer>").unwrap()..];
        assert!(
            footer.contains(
                "<button type=\"submit\" lang=\"en\" aria-current=\"true\">English</button>"
            ),
            "{footer}"
        );
        assert!(!footer.contains("日本語"), "{footer}");
        assert_eq!(footer.matches("action=\"/lang\"").count(), 1);
    }
}

#[tokio::test]
async fn choosing_japanese_is_refused_and_sets_nothing() {
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
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        !headers
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap_or("").starts_with("planeter_lang="))
    );
}
