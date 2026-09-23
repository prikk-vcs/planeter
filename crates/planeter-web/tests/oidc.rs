//! OIDC sign-in through the web layer (WEB-05 / RFC 002 T2): redirect → callback → session, driven via
//! `ServiceExt::oneshot` against a mock fetcher — no network, no prikk. The ID token is signed with a
//! throwaway P-256 key whose JWK the mock JWKS serves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, InsecureStubHasher, OidcConfig, OidcProvider,
};
use planeter_core::{
    FetchError, FetchRequest, HostingService, HttpsFetcher, NullAuditSink, ReadService,
};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore, UserId,
};
use planeter_web::{AppState, ContentOrigin, router};

const ISSUER: &str = "https://idp.example";
const CLIENT: &str = "planeter-client";
const TEST_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgopZeFiRGZXWvz9h4
G2WRp6WFSymfcUW7Kavsub8xSMehRANCAATQ+QQEj43jJBGpA4RYiR3l6Pnckdxw
BXaTdqRaPA7mOrNmL/VGO0eeJS9V/25SRXPdRAdaGWs75OxYcjsHqSZV
-----END PRIVATE KEY-----";
const JWK_X: &str = "0PkEBI-N4yQRqQOEWIkd5ej53JHccAV2k3akWjwO5jo";
const JWK_Y: &str = "s2Yv9UY7R54lL1X_blJFc91EB1oZazvk7FhyOwepJlU";

#[derive(Default)]
struct MockFetcher {
    routes: Mutex<HashMap<String, Vec<u8>>>,
}
impl MockFetcher {
    fn set(&self, url: &str, body: String) {
        self.routes
            .lock()
            .unwrap()
            .insert(url.to_owned(), body.into_bytes());
    }
}
impl HttpsFetcher for MockFetcher {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        self.routes
            .lock()
            .unwrap()
            .get(&req.url)
            .cloned()
            .ok_or_else(|| FetchError::Transport(format!("no route for {}", req.url)))
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn signed_id_token(nonce: &str, sub: &str) -> String {
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some("k1".into());
    encode(
        &h,
        &serde_json::json!({"iss": ISSUER, "sub": sub, "aud": CLIENT, "exp": now() + 300, "iat": now(), "nonce": nonce}),
        &EncodingKey::from_ec_pem(TEST_KEY_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

/// An app with OIDC configured against the mock provider; `alice` is linked to subject `user-42`.
fn app(subdir: &str) -> (AppState, Arc<MockFetcher>) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(subdir);
    let _ = std::fs::remove_dir_all(&root);
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(root, store));
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account {
            user: UserId::new("alice"),
            display_name: "Alice".into(),
            email: None,
            password: None,
        })
        .unwrap();
    accounts
        .link_oidc(ISSUER, "user-42", UserId::new("alice"))
        .unwrap();
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Arc::new(Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    ));
    let fetcher = Arc::new(MockFetcher::default());
    fetcher.set(
        "https://idp.example/.well-known/openid-configuration",
        format!(r#"{{"issuer":"{ISSUER}","authorization_endpoint":"https://idp.example/auth","token_endpoint":"https://idp.example/token","jwks_uri":"https://idp.example/jwks"}}"#),
    );
    fetcher.set(
        "https://idp.example/jwks",
        format!(r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"{JWK_X}","y":"{JWK_Y}","kid":"k1","alg":"ES256","use":"sig"}}]}}"#),
    );
    let oidc = Arc::new(OidcProvider::new(
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: CLIENT.into(),
            client_secret: None,
            redirect_uri: "https://forge.example/login/oidc/callback".into(),
        },
        fetcher.clone(),
    ));
    let state = AppState {
        read,
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: Some(oidc),
    };
    (state, fetcher)
}

async fn send(r: &axum::Router, req: Request<Body>) -> Response {
    r.clone().oneshot(req).await.unwrap()
}

fn cookie_value(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| {
            c.split(';')
                .next()
                .unwrap()
                .split_once('=')
                .unwrap()
                .1
                .to_owned()
        })
}

fn query_param(url: &str, name: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v.to_owned())
    })
}

#[tokio::test]
async fn oidc_redirect_callback_and_session() {
    let (state, fetcher) = app("web-oidc");
    let r = router(state);

    // The login page advertises SSO.
    let page = send(
        &r,
        Request::builder()
            .uri("/login")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let body = axum::body::to_bytes(page.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("/login/oidc"));

    // Begin: 303 to the provider with PKCE, and the opaque state in an HttpOnly cookie.
    let begin = send(
        &r,
        Request::builder()
            .uri("/login/oidc")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(begin.status(), StatusCode::SEE_OTHER);
    let location = begin
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(location.starts_with("https://idp.example/auth?"));
    assert!(location.contains("code_challenge_method=S256"));
    let state_cookie = cookie_value(&begin, "planeter_oidc_state").expect("state cookie");
    let state_param = query_param(&location, "state").unwrap();
    assert_eq!(state_cookie, state_param);
    let nonce = query_param(&location, "nonce").unwrap();

    // The provider will answer the token exchange with an ID token bound to that nonce.
    fetcher.set(
        "https://idp.example/token",
        format!(r#"{{"id_token":"{}"}}"#, signed_id_token(&nonce, "user-42")),
    );

    // A callback with a mismatched state is refused.
    let bad = send(
        &r,
        Request::builder()
            .uri("/login/oidc/callback?code=abc&state=forged")
            .header(
                header::COOKIE,
                format!("planeter_oidc_state={state_cookie}"),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // The real callback signs alice in.
    let cb = send(
        &r,
        Request::builder()
            .uri(format!(
                "/login/oidc/callback?code=abc&state={state_cookie}"
            ))
            .header(
                header::COOKIE,
                format!("planeter_oidc_state={state_cookie}"),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(cb.status(), StatusCode::SEE_OTHER, "{:?}", cb.headers());
    let session = cookie_value(&cb, "planeter_session").expect("session cookie");
    assert!(!session.is_empty());

    // The state was one-shot: replaying the callback is refused.
    let replay = send(
        &r,
        Request::builder()
            .uri(format!(
                "/login/oidc/callback?code=abc&state={state_cookie}"
            ))
            .header(
                header::COOKIE,
                format!("planeter_oidc_state={state_cookie}"),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unlinked_subject_is_refused() {
    let (state, fetcher) = app("web-oidc-unlinked");
    let r = router(state);
    let begin = send(
        &r,
        Request::builder()
            .uri("/login/oidc")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let location = begin
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let st = cookie_value(&begin, "planeter_oidc_state").unwrap();
    let nonce = query_param(&location, "nonce").unwrap();
    fetcher.set(
        "https://idp.example/token",
        format!(
            r#"{{"id_token":"{}"}}"#,
            signed_id_token(&nonce, "nobody-7")
        ),
    );
    let cb = send(
        &r,
        Request::builder()
            .uri(format!("/login/oidc/callback?code=abc&state={st}"))
            .header(header::COOKIE, format!("planeter_oidc_state={st}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(cb.status(), StatusCode::UNAUTHORIZED);
    assert!(cookie_value(&cb, "planeter_session").is_none());
}
