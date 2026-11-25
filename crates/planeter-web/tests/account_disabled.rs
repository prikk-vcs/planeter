//! RFC 014 D-6 through the real router and **SQLite stores** (as the server runs): a disabled account's live session and its token are
//! refused on the very next request; `enable` restores a token (until its own expiry) and not an ended session; a password attempt on a
//! disabled account is indistinguishable from a wrong password (status, body, and the one verification of IS-31); the 404-for-hidden rule
//! and everyone else's access are untouched.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, PasswordHash, PasswordHasher, SessionStore,
    Sha256TokenHasher, SqliteAccountStore, SqliteCredentialStore, SqliteSessionStore, TokenHasher,
    TokenRecord,
};
use planeter_core::{HostingService, NullAuditSink, ReadService, RepoScope, Scope, ScopeAccess};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore, SqliteDb,
    UserId,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, router};

const NOW: u64 = 1_000_000;

fn prikk_available() -> bool {
    std::process::Command::new("prikk")
        .arg("--version")
        .output()
        .is_ok()
}
const CSRF: &str = "fixed-csrf-token-for-the-tests";

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

struct World {
    router: Router,
    accounts: Arc<SqliteAccountStore>,
    sessions: Arc<SqliteSessionStore>,
    hasher: Arc<Counting>,
}

fn world(name: &str) -> World {
    let db = SqliteDb::open_in_memory().unwrap();
    let accounts = Arc::new(SqliteAccountStore::new(db.clone()).unwrap());
    let creds = Arc::new(SqliteCredentialStore::new(db.clone()).unwrap());
    let sessions = Arc::new(SqliteSessionStore::new(db).unwrap());
    let hasher = Arc::new(Counting::default());
    for user in ["alice", "bob"] {
        accounts
            .upsert(Account::local(user, user, Some(hasher.hash("pw"))))
            .unwrap();
    }
    for (secret, user) in [("tok-alice", "alice"), ("tok-bob", "bob")] {
        creds
            .add_token(TokenRecord {
                hash: Sha256TokenHasher.hash(secret),
                user: UserId::new(user),
                scope: Scope {
                    access: ScopeAccess::Full,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let auth = Arc::new(
        Authenticator::new(
            accounts.clone(),
            creds,
            hasher.clone(),
            Arc::new(Sha256TokenHasher),
        )
        .with_clock(|| NOW),
    );
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(
        HostingService::new(root, store as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    // alice's private repository, made by the real prikk when it is there: what a token is *for* is shown by a read that only she may do.
    // Without prikk the token half of these tests is skipped (said), the session and password halves still run.
    if prikk_available() {
        hosting
            .create_repo(
                planeter_store::Owner::User("alice".into()),
                "app",
                planeter_store::Visibility::Private,
            )
            .expect("create repo");
    }
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
        now_unix: || NOW,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport: None,
        creation: None,
    };
    World {
        router: router(state),
        accounts,
        sessions,
        hasher,
    }
}

type Reply = (StatusCode, Vec<(String, String)>, Vec<u8>);

async fn send(w: &World, req: Request<Body>) -> Reply {
    let resp = w.router.clone().oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    let headers = parts
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    (parts.status, headers, bytes.to_vec())
}

fn home(cookie: Option<&str>) -> Request<Body> {
    let mut r = Request::builder().uri("/");
    if let Some(c) = cookie {
        r = r.header(header::COOKIE, format!("planeter_session={c}"));
    }
    r.body(Body::empty()).unwrap()
}

/// A read of alice's **private** repository: 404 for anyone who is not authorized (the same for a stranger, an unknown token and no
/// credential), anything else for alice. Authorization is decided before prikk is asked, so this needs no prikk.
fn api_read(token: &str) -> Request<Body> {
    Request::builder()
        .uri("/api/v1/repos/alice/app/history")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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

fn signed_in(body: &[u8]) -> bool {
    String::from_utf8_lossy(body).contains("action=\"/logout\"")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_session_and_a_token_are_refused_on_the_very_next_request_after_disable() {
    let w = world("account-disabled-live");
    let alice = w
        .sessions
        .create(UserId::new("alice"), NOW, NOW + 3600)
        .unwrap();
    let bob = w
        .sessions
        .create(UserId::new("bob"), NOW, NOW + 3600)
        .unwrap();

    // Before: the session is a sign-in, the token is a principal (the private repository is not a 404 for it).
    assert!(signed_in(&send(&w, home(Some(&alice.id.0))).await.2));
    let tokens = prikk_available();
    if !tokens {
        eprintln!("skipping the token half: prikk not on PATH");
    }
    let unknown = send(&w, api_read("tok-nobody")).await;
    assert_eq!(unknown.0, StatusCode::NOT_FOUND);
    if tokens {
        assert_eq!(send(&w, api_read("tok-alice")).await.0, StatusCode::OK);
    }

    w.accounts
        .disable(&UserId::new("alice"), "left the project", NOW)
        .unwrap();

    // The very next request: no sign-in, and the same 404 as an unknown token, byte for byte.
    assert!(
        !signed_in(&send(&w, home(Some(&alice.id.0))).await.2),
        "the session is no session"
    );
    if tokens {
        let refused = send(&w, api_read("tok-alice")).await;
        assert_eq!(refused.0, StatusCode::NOT_FOUND);
        assert_eq!(refused.2, unknown.2, "the body is the unknown token's");
    }
    // Everyone else is untouched.
    assert!(signed_in(&send(&w, home(Some(&bob.id.0))).await.2));
    assert_eq!(
        send(&w, api_read("tok-bob")).await.0,
        StatusCode::NOT_FOUND,
        "bob never had access to alice's private repository"
    );

    // Enable: the token works again (until its own expiry); the session that ended stays ended; a fresh sign-in works.
    w.accounts.enable(&UserId::new("alice")).unwrap();
    if tokens {
        assert_eq!(send(&w, api_read("tok-alice")).await.0, StatusCode::OK);
    }
    assert!(
        !signed_in(&send(&w, home(Some(&alice.id.0))).await.2),
        "an ended session does not come back"
    );
    assert_eq!(
        send(&w, login("alice", "pw")).await.0,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_that_was_made_after_the_account_was_disabled_is_refused_too() {
    // A session row can exist for a disabled account (a session store that is not the account's own, a race): resolution refuses it
    // regardless of how it got there, which is what "checked at every request" means.
    let w = world("account-disabled-late-session");
    w.accounts.disable(&UserId::new("alice"), "x", NOW).unwrap();
    let late = w
        .sessions
        .create(UserId::new("alice"), NOW, NOW + 3600)
        .unwrap();
    assert!(!signed_in(&send(&w, home(Some(&late.id.0))).await.2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_password_attempt_on_a_disabled_account_is_a_wrong_password_in_status_body_and_work() {
    let w = world("account-disabled-password");
    w.accounts.disable(&UserId::new("alice"), "x", NOW).unwrap();
    let count = || w.hasher.verifies.load(Ordering::SeqCst);

    let before = count();
    let wrong = send(&w, login("bob", "wrong")).await;
    assert_eq!((wrong.0, count() - before), (StatusCode::UNAUTHORIZED, 1));

    // The disabled account with the right password, and with a wrong one, and an unknown one: one verification, the same status, the same
    // body. (The body carries the form's own nonce only through the cookie, which these requests fix.)
    for (user, pw) in [("alice", "pw"), ("alice", "wrong"), ("ghost", "pw")] {
        let before = count();
        let got = send(&w, login(user, pw)).await;
        assert_eq!(got.0, wrong.0, "{user}/{pw}");
        assert_eq!(count() - before, 1, "{user}/{pw}: exactly one verification");
        // No session cookie is set, and the page is the same page.
        assert!(
            !got.1
                .iter()
                .any(|(k, v)| k == "set-cookie" && v.starts_with("planeter_session"))
        );
        let strip = |b: &[u8]| String::from_utf8_lossy(b).replace(user, "USER");
        assert_eq!(
            strip(&got.2),
            strip(&wrong.2).replace("bob", "USER"),
            "{user}/{pw}: the same body"
        );
    }
}
