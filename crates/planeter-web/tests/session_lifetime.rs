//! RFC 014 D-13: **`PLANETER_SESSION_HOURS`**, the lifetime of a sign-in, as one value from one place (the session store). It sets the
//! session's server-side expiry and the cookie's `Max-Age` together, at **both** sign-in paths (a password and the provider), through the
//! real router; a session expires at exactly its lifetime; a session that exists keeps the lifetime it was created with; and the default
//! is the old seven days. (The start-up refusal of a bad value is tested in the server binary's `provider_env` and `provider_settings`.)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, OidcConfig, OidcProvider, PasswordHash, PasswordHasher,
    SESSION_TTL_SECS, SessionId, SessionStore, Sha256TokenHasher, SqliteAccountStore,
    SqliteCredentialStore, SqliteSessionStore,
};
use planeter_core::{
    FetchError, FetchRequest, HostingService, HttpsFetcher, NullAuditSink, ReadService,
};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore, SqliteDb,
    UserId,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, router};

const NOW: u64 = 1_000_000;
const ISSUER: &str = "https://idp.example";
const CLIENT: &str = "planeter-client";
const CSRF: &str = "fixed-csrf-token-for-the-tests";
const KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgopZeFiRGZXWvz9h4
G2WRp6WFSymfcUW7Kavsub8xSMehRANCAATQ+QQEj43jJBGpA4RYiR3l6Pnckdxw
BXaTdqRaPA7mOrNmL/VGO0eeJS9V/25SRXPdRAdaGWs75OxYcjsHqSZV
-----END PRIVATE KEY-----";
const JWK_X: &str = "0PkEBI-N4yQRqQOEWIkd5ej53JHccAV2k3akWjwO5jo";
const JWK_Y: &str = "s2Yv9UY7R54lL1X_blJFc91EB1oZazvk7FhyOwepJlU";

struct Stub;
impl PasswordHasher for Stub {
    fn hash(&self, password: &str) -> PasswordHash {
        PasswordHash(format!("h:{password}"))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        hash.0 == format!("h:{password}")
    }
}

#[derive(Default)]
struct Mock(Mutex<HashMap<String, Vec<u8>>>);
impl HttpsFetcher for Mock {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        self.0
            .lock()
            .unwrap()
            .get(&req.url)
            .cloned()
            .ok_or_else(|| FetchError::Transport(format!("no route for {}", req.url)))
    }
}

struct World {
    router: Router,
    sessions: Arc<SqliteSessionStore>,
    fetcher: Arc<Mock>,
}

/// A router over `db` with sessions that last `ttl_secs` (`None`: the store's default), the clock at `now`.
fn world(db: &SqliteDb, name: &str, ttl_secs: Option<u64>, now: fn() -> u64) -> World {
    let accounts = Arc::new(SqliteAccountStore::new(db.clone()).unwrap());
    if accounts.get(&UserId::new("alice")).unwrap().is_none() {
        accounts
            .upsert(Account::local("alice", "Alice", Some(Stub.hash("pw"))))
            .unwrap();
        accounts
            .link_oidc(ISSUER, "user-42", UserId::new("alice"))
            .unwrap();
    }
    let creds = Arc::new(SqliteCredentialStore::new(db.clone()).unwrap());
    let sessions = SqliteSessionStore::new(db.clone()).unwrap();
    let sessions = Arc::new(match ttl_secs {
        Some(t) => sessions.with_ttl_secs(t),
        None => sessions,
    });
    let auth = Arc::new(
        Authenticator::new(accounts, creds, Arc::new(Stub), Arc::new(Sha256TokenHasher))
            .with_clock(now),
    );
    let fetcher = Arc::new(Mock::default());
    fetcher.0.lock().unwrap().insert(
        format!("{ISSUER}/.well-known/openid-configuration"),
        format!(r#"{{"issuer":"{ISSUER}","authorization_endpoint":"{ISSUER}/auth","token_endpoint":"{ISSUER}/token","jwks_uri":"{ISSUER}/jwks"}}"#).into_bytes(),
    );
    fetcher.0.lock().unwrap().insert(
        format!("{ISSUER}/jwks"),
        format!(r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"{JWK_X}","y":"{JWK_Y}","kid":"k1","alg":"ES256","use":"sig"}}]}}"#).into_bytes(),
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
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(root, store));
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));
    let state = AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: sessions.clone(),
        now_unix: now,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: Some(oidc),
        transport: None,
        creation: None,
    };
    World {
        router: router(state),
        sessions,
        fetcher,
    }
}

async fn send(w: &World, req: Request<Body>) -> axum::response::Response {
    w.router.clone().oneshot(req).await.unwrap()
}

fn set_cookie_of(resp: &axum::response::Response, name: &str) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(str::to_owned)
}

fn value_and_max_age(set_cookie: &str) -> (String, u64) {
    let value = set_cookie
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_owned();
    let max = set_cookie
        .split(';')
        .find_map(|p| p.trim().strip_prefix("Max-Age="))
        .unwrap()
        .parse()
        .unwrap();
    (value, max)
}

async fn password_sign_in(w: &World) -> (String, u64) {
    let resp = send(
        w,
        Request::builder()
            .method("POST")
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, format!("planeter_csrf={CSRF}"))
            .body(Body::from(format!(
                "csrf={CSRF}&username=alice&password=pw"
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER, "{:?}", resp.headers());
    value_and_max_age(&set_cookie_of(&resp, "planeter_session").expect("a session cookie"))
}

async fn provider_sign_in(w: &World) -> (String, u64) {
    let begin = send(
        w,
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
    let query = |name: &str| {
        location
            .split_once('?')
            .unwrap()
            .1
            .split('&')
            .find_map(|kv| {
                kv.split_once('=')
                    .filter(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_owned())
            })
            .unwrap()
    };
    let state_cookie = value_and_max_age(&set_cookie_of(&begin, "planeter_oidc_state").unwrap()).0;
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some("k1".into());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = encode(
        &h,
        &serde_json::json!({"iss": ISSUER, "sub": "user-42", "aud": CLIENT, "exp": now + 300, "iat": now, "nonce": query("nonce")}),
        &EncodingKey::from_ec_pem(KEY_PEM.as_bytes()).unwrap(),
    )
    .unwrap();
    w.fetcher.0.lock().unwrap().insert(
        format!("{ISSUER}/token"),
        format!(r#"{{"id_token":"{token}"}}"#).into_bytes(),
    );
    let cb = send(
        w,
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
    value_and_max_age(&set_cookie_of(&cb, "planeter_session").expect("a session cookie"))
}

fn home(session: &str) -> Request<Body> {
    Request::builder()
        .uri("/")
        .header(header::COOKIE, format!("planeter_session={session}"))
        .body(Body::empty())
        .unwrap()
}

async fn signed_in(w: &World, session: &str) -> bool {
    let resp = send(w, home(session)).await;
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8_lossy(&body).contains("action=\"/logout\"")
}

#[tokio::test]
async fn the_setting_reaches_the_expiry_and_the_cookie_at_both_sign_in_paths() {
    for hours in [1u64, 24, 168, 720] {
        let ttl = hours * 3600;
        let db = SqliteDb::open_in_memory().unwrap();
        let w = world(&db, "session-both", Some(ttl), || NOW);
        assert_eq!(w.sessions.ttl_secs(), ttl);
        for (path, (session, max_age)) in [
            ("password", password_sign_in(&w).await),
            ("provider", provider_sign_in(&w).await),
        ] {
            assert_eq!(max_age, ttl, "{hours} h, {path}: the cookie's Max-Age");
            let stored = w
                .sessions
                .get(&SessionId(session.clone()), NOW)
                .unwrap()
                .unwrap();
            assert_eq!(
                stored.expires_at,
                NOW + ttl,
                "{hours} h, {path}: the stored expiry"
            );
            assert_eq!(stored.created_at, NOW);
        }
    }
}

#[tokio::test]
async fn a_session_expires_at_exactly_its_lifetime_and_not_a_second_before() {
    let ttl = 24 * 3600;
    let db = SqliteDb::open_in_memory().unwrap();
    let w = world(&db, "session-exact", Some(ttl), || NOW);
    let (session, _) = password_sign_in(&w).await;
    let id = SessionId(session.clone());
    assert!(
        w.sessions.get(&id, NOW + ttl - 1).unwrap().is_some(),
        "one second before the end"
    );
    assert!(
        w.sessions.get(&id, NOW + ttl).unwrap().is_none(),
        "at the end"
    );
    // Through the real router with the clock moved (the clock is a plain function: one router per instant).
    let before = world(&db, "session-exact-before", Some(ttl), || {
        NOW + 24 * 3600 - 1
    });
    assert!(
        signed_in(&before, &session).await,
        "the router still knows the person one second before the end"
    );
    let at_end = world(&db, "session-exact-end", Some(ttl), || NOW + 24 * 3600);
    assert!(!signed_in(&at_end, &session).await, "and not at the end");
}

#[tokio::test]
async fn a_session_that_exists_keeps_the_lifetime_it_was_created_with() {
    let db = SqliteDb::open_in_memory().unwrap();
    // Made when the lifetime was seven days.
    let old = world(&db, "session-keeps-old", Some(168 * 3600), || NOW);
    let (session, max_age) = password_sign_in(&old).await;
    assert_eq!(max_age, 168 * 3600);
    // The operator then sets one hour and restarts: the same database, a new store.
    let new = world(&db, "session-keeps-new", Some(3600), || NOW + 100 * 3600);
    let id = SessionId(session.clone());
    let kept = new
        .sessions
        .get(&id, NOW + 100 * 3600)
        .unwrap()
        .expect("the old session is still valid at 100 hours");
    assert_eq!(
        kept.expires_at,
        NOW + 168 * 3600,
        "its own expiry, as it was stored"
    );
    assert!(signed_in(&new, &session).await);
    // A new sign-in gets the new lifetime.
    let (fresh, fresh_age) = password_sign_in(&new).await;
    assert_eq!(fresh_age, 3600);
    assert_eq!(
        new.sessions
            .get(&SessionId(fresh), NOW + 100 * 3600)
            .unwrap()
            .unwrap()
            .expires_at,
        NOW + 100 * 3600 + 3600
    );
}

#[tokio::test]
async fn the_default_is_the_old_seven_days_at_both_paths() {
    let db = SqliteDb::open_in_memory().unwrap();
    let w = world(&db, "session-default", None, || NOW);
    assert_eq!(w.sessions.ttl_secs(), 7 * 24 * 3600);
    assert_eq!(SESSION_TTL_SECS, 7 * 24 * 3600);
    assert_eq!(password_sign_in(&w).await.1, 7 * 24 * 3600);
    assert_eq!(provider_sign_in(&w).await.1, 7 * 24 * 3600);
}
