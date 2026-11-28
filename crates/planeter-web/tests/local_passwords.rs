//! RFC 014 D-5: `PLANETER_LOCAL_PASSWORDS=off`, through the real router and the SQLite stores. With it off the sign-in page offers the sign-in
//! provider only and says why; `POST /login` answers every account — one with a password, one without, one that does not exist, a right
//! password and a wrong one — **as a wrong password is answered, with the same one verification** (no oracle that says whether password
//! sign-in is off for an account: IS-31); no stored hash is touched, so turning it back on restores every password; the provider's sign-in,
//! sessions that exist and tokens are unaffected; and `on` is today's behaviour. (The start-up refusals are tested in the server binary's
//! `provider_env`; the operator commands' sentence in `operator_account`.)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, OidcConfig, OidcProvider, PasswordHash, PasswordHasher,
    Sha256TokenHasher, SqliteAccountStore, SqliteCredentialStore, SqliteSessionStore,
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

/// `h:<password>` hashes; counts verifications.
#[derive(Default)]
struct Counting {
    verifies: AtomicUsize,
}
impl PasswordHasher for Counting {
    fn hash(&self, password: &str) -> PasswordHash {
        PasswordHash(format!("h:{password}"))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        self.verifies.fetch_add(1, Ordering::SeqCst);
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
    hasher: Arc<Counting>,
    fetcher: Arc<Mock>,
}

/// A router over `db`; `passwords` is `PLANETER_LOCAL_PASSWORDS` (`true`: on). Alice has a password and a linked identity; Bob has an account
/// and no password.
fn world(db: &SqliteDb, name: &str, passwords: bool) -> World {
    let hasher = Arc::new(Counting::default());
    let accounts = Arc::new(SqliteAccountStore::new(db.clone()).unwrap());
    if accounts.get(&UserId::new("alice")).unwrap().is_none() {
        accounts
            .upsert(Account::local("alice", "Alice", Some(hasher.hash("pw"))))
            .unwrap();
        accounts
            .link_oidc(ISSUER, "user-42", UserId::new("alice"))
            .unwrap();
        accounts.upsert(Account::local("bob", "Bob", None)).unwrap();
    }
    let creds = Arc::new(SqliteCredentialStore::new(db.clone()).unwrap());
    let sessions = Arc::new(SqliteSessionStore::new(db.clone()).unwrap());
    let auth = Arc::new(
        Authenticator::new(accounts, creds, hasher.clone(), Arc::new(Sha256TokenHasher))
            .with_clock(|| NOW)
            .with_local_passwords(passwords),
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
        sessions,
        now_unix: || NOW,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: Some(oidc),
        transport: None,
        creation: None,
    };
    World {
        router: router(state),
        hasher,
        fetcher,
    }
}

struct Reply {
    status: StatusCode,
    set_cookies: Vec<String>,
    location: Option<String>,
    body: String,
}

async fn send(w: &World, req: Request<Body>) -> Reply {
    let resp = w.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let set_cookies = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_owned)
        .collect();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        set_cookies,
        location,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

fn session_of(r: &Reply) -> Option<String> {
    r.set_cookies
        .iter()
        .find(|c| c.starts_with("planeter_session="))
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

fn login_page(lang: &str) -> Request<Body> {
    Request::builder()
        .uri("/login")
        .header(header::COOKIE, format!("planeter_lang={lang}"))
        .body(Body::empty())
        .unwrap()
}

fn login(user: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("planeter_csrf={CSRF}"))
        .body(Body::from(format!(
            "csrf={CSRF}&username={user}&password={password}"
        )))
        .unwrap()
}

fn home(session: &str) -> Request<Body> {
    Request::builder()
        .uri("/")
        .header(header::COOKIE, format!("planeter_session={session}"))
        .body(Body::empty())
        .unwrap()
}

async fn signed_in(w: &World, session: &str) -> bool {
    send(w, home(session))
        .await
        .body
        .contains("action=\"/logout\"")
}

#[tokio::test]
async fn with_passwords_off_the_page_offers_the_provider_only_and_says_why_in_both_languages() {
    let db = SqliteDb::open_in_memory().unwrap();
    let off = world(&db, "lp-page-off", false);
    for (lang, sentence) in [
        (
            "en",
            "Password sign-in is turned off on this server. Sign in through the sign-in provider.",
        ),
        (
            "ja",
            "このサーバーでは、パスワードでのログインはオフになっています。ログインのプロバイダー経由でログインしてください。",
        ),
    ] {
        let page = send(&off, login_page(lang)).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains(sentence), "{lang}: the reason is said");
        assert!(
            page.body.contains("href=\"/login/oidc\""),
            "{lang}: the provider's link"
        );
        for gone in [
            "name=\"password\"",
            "name=\"username\"",
            "action=\"/login\"",
        ] {
            assert!(
                !page.body.contains(gone),
                "{lang}: no {gone} with passwords off"
            );
        }
    }
    // `on` is today's page: the fields, the provider's link, and not the sentence.
    let on = world(&db, "lp-page-on", true);
    let page = send(&on, login_page("en")).await;
    for there in [
        "name=\"password\"",
        "name=\"username\"",
        "action=\"/login\"",
        "href=\"/login/oidc\"",
    ] {
        assert!(page.body.contains(there), "{there}");
    }
    assert!(!page.body.contains("Password sign-in is turned off"));
}

#[tokio::test]
async fn every_password_attempt_is_answered_as_a_wrong_password_and_costs_one_verification() {
    let db = SqliteDb::open_in_memory().unwrap();
    let off = world(&db, "lp-attempts", false);
    // Alice's right password; alice's wrong one; bob, who has none; an account that does not exist.
    let attempts = [
        ("alice", "pw"),
        ("alice", "wrong"),
        ("bob", "pw"),
        ("ghost", "pw"),
    ];
    let mut replies = Vec::new();
    for (user, password) in attempts {
        let before = off.hasher.verifies.load(Ordering::SeqCst);
        let r = send(&off, login(user, password)).await;
        let cost = off.hasher.verifies.load(Ordering::SeqCst) - before;
        assert_eq!(
            cost, 1,
            "{user}/{password}: one verification, as for any password attempt"
        );
        assert_eq!(
            r.status,
            StatusCode::UNAUTHORIZED,
            "{user}/{password}: refused"
        );
        assert!(
            session_of(&r).is_none(),
            "{user}/{password}: no sign-in was made"
        );
        replies.push((user, password, r));
    }
    // The same status and the same page for all of them (the sign-in page again; the fields are absent with the setting off).
    let first = &replies[0].2;
    for (user, password, r) in &replies[1..] {
        assert_eq!(r.status, first.status, "{user}/{password}: status");
        assert_eq!(
            r.body, first.body,
            "{user}/{password}: the page is the same, byte for byte"
        );
        assert_eq!(
            r.set_cookies.len(),
            first.set_cookies.len(),
            "{user}/{password}: the same cookies"
        );
    }
    // And what a wrong password is answered with *with passwords on*: the same status.
    let on = world(&db, "lp-attempts-on", true);
    let wrong_on = send(&on, login("alice", "wrong")).await;
    assert_eq!(wrong_on.status, first.status);
    let right_on = send(&on, login("alice", "pw")).await;
    assert_eq!(right_on.status, StatusCode::SEE_OTHER, "`on` signs in");
    assert!(session_of(&right_on).is_some());
}

#[tokio::test]
async fn turning_it_back_on_restores_every_password_and_nothing_stored_was_touched() {
    let db = SqliteDb::open_in_memory().unwrap();
    let before = {
        let on = world(&db, "lp-back-1", true);
        let accounts = SqliteAccountStore::new(db.clone()).unwrap();
        let hash = accounts
            .get(&UserId::new("alice"))
            .unwrap()
            .unwrap()
            .password;
        assert!(session_of(&send(&on, login("alice", "pw")).await).is_some());
        hash
    };
    let off = world(&db, "lp-back-2", false);
    assert!(session_of(&send(&off, login("alice", "pw")).await).is_none());
    let accounts = SqliteAccountStore::new(db.clone()).unwrap();
    assert_eq!(
        accounts
            .get(&UserId::new("alice"))
            .unwrap()
            .unwrap()
            .password,
        before,
        "the stored hash is exactly as it was"
    );
    let again = world(&db, "lp-back-3", true);
    assert!(
        session_of(&send(&again, login("alice", "pw")).await).is_some(),
        "back on, the same password signs in"
    );
}

#[tokio::test]
async fn the_provider_still_signs_in_and_a_session_that_exists_stays() {
    let db = SqliteDb::open_in_memory().unwrap();
    // A sign-in made by password while it was on.
    let on = world(&db, "lp-keep-1", true);
    let session = session_of(&send(&on, login("alice", "pw")).await).unwrap();
    // Off: the session that exists is still known …
    let off = world(&db, "lp-keep-2", false);
    assert!(signed_in(&off, &session).await, "an existing sign-in stays");
    // … and the provider's sign-in works (the in-process flow of the other tests).
    let begin = send(
        &off,
        Request::builder()
            .uri("/login/oidc")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(begin.status, StatusCode::SEE_OTHER);
    let redirect = begin.location.clone().unwrap();
    let state_cookie = begin
        .set_cookies
        .iter()
        .find(|c| c.starts_with("planeter_oidc_state="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_owned();
    let nonce = redirect
        .split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|kv| {
            kv.split_once('=')
                .filter(|(k, _)| *k == "nonce")
                .map(|(_, v)| v.to_owned())
        })
        .unwrap();
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some("k1".into());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = encode(
        &h,
        &serde_json::json!({"iss": ISSUER, "sub": "user-42", "aud": CLIENT, "exp": now + 300, "iat": now, "nonce": nonce}),
        &EncodingKey::from_ec_pem(KEY_PEM.as_bytes()).unwrap(),
    )
    .unwrap();
    off.fetcher.0.lock().unwrap().insert(
        format!("{ISSUER}/token"),
        format!(r#"{{"id_token":"{token}"}}"#).into_bytes(),
    );
    let cb = send(
        &off,
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
    assert_eq!(cb.status, StatusCode::SEE_OTHER, "{}", cb.body);
    let provider_session = session_of(&cb).expect("the provider's sign-in made a session");
    assert!(signed_in(&off, &provider_session).await);
}
