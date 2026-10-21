//! IS-24 / IS-31: password verification runs off the async workers under its own small gate, every
//! verification (real or dummy) takes a permit, and a refusal under load is the same for an existing
//! account and an unknown one.
//!
//! A slow, counting stand-in hasher plays Argon2id. The router is driven through the real `/login`
//! route with `ServiceExt::oneshot` on a multi-thread runtime of **two** workers, so a handler that
//! hashed on a worker would visibly stall the cheap-route test.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, PasswordHash, PasswordHasher, SessionStore, Sha256TokenHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore, UserId,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, router};

/// `verify` sleeps `delay_ms` and counts; `hash` is `h:<password>`.
#[derive(Default)]
struct SlowHasher {
    delay_ms: AtomicU64,
    verifies: AtomicUsize,
}

impl PasswordHasher for SlowHasher {
    fn hash(&self, password: &str) -> PasswordHash {
        PasswordHash(format!("h:{password}"))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        let ms = self.delay_ms.load(Ordering::SeqCst);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        hash.0 == format!("h:{password}")
    }
}

struct Harness {
    router: Router,
    gate: Arc<BlockingGate>,
    hasher: Arc<SlowHasher>,
    sessions: Arc<InMemorySessionStore>,
    csrf: String,
}

fn harness(concurrency: usize, wait: Duration, delay_ms: u64) -> Harness {
    let hasher = Arc::new(SlowHasher::default());
    let accounts = Arc::new(InMemoryAccountStore::new());
    for name in ["alice", "user-1", "user-2", "user-3", "user-4"] {
        accounts
            .upsert(Account {
                user: UserId::new(name),
                display_name: name.into(),
                email: None,
                password: Some(hasher.hash("pw")),
            })
            .unwrap();
    }
    let auth = Arc::new(Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        Arc::new(Sha256TokenHasher),
    ));
    // Only now make verification slow (the dummy hash was made at construction).
    hasher.delay_ms.store(delay_ms, Ordering::SeqCst);

    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("auth-gate");
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(
        HostingService::new(root, store as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));
    let gate = Arc::new(BlockingGate::new(concurrency, wait));
    let sessions = Arc::new(InMemorySessionStore::new());
    let state = AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::clone(&gate),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: sessions.clone(),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::parse("10.0.0.0/8").unwrap()),
        oidc: None,
        transport: None,
    };
    Harness {
        router: router(state),
        gate,
        hasher,
        sessions,
        csrf: "fixed-csrf-token-for-the-tests".into(),
    }
}

type Reply = (StatusCode, HeaderMap, Vec<u8>);

async fn send(router: Router, req: Request<Body>) -> Reply {
    let resp = router.oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    (parts.status, parts.headers, bytes.to_vec())
}

fn login(h: &Harness, user: &str, password: &str, peer: Option<SocketAddr>) -> Request<Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("planeter_csrf={}", h.csrf))
        .body(Body::from(format!(
            "csrf={}&username={user}&password={password}",
            h.csrf
        )))
        .unwrap();
    if let Some(p) = peer {
        req.extensions_mut().insert(ConnectInfo(p));
    }
    req
}

async fn eventually(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < limit, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_excess_signins_get_the_same_503_for_existing_and_unknown_names() {
    let h = harness(2, Duration::from_millis(300), 800);
    // Two sign-ins fill both slots (real accounts, wrong password: they will end as 401).
    let held: Vec<_> = ["user-1", "user-2"]
        .into_iter()
        .map(|u| tokio::spawn(send(h.router.clone(), login(&h, u, "nope", None))))
        .collect();
    eventually("both slots to fill", Duration::from_secs(5), || {
        h.gate.available() == 0
    })
    .await;

    // The excess: one existing account, one unknown name. Both wait the bounded time, then 503.
    let existing = send(h.router.clone(), login(&h, "user-3", "nope", None)).await;
    let unknown = send(h.router.clone(), login(&h, "no-such-user", "nope", None)).await;

    assert_eq!(existing.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unknown.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(existing.1.get(header::RETRY_AFTER).unwrap(), "1");
    assert_eq!(
        existing.1, unknown.1,
        "headers must not distinguish an existing account from an unknown one"
    );
    assert_eq!(
        existing.2, unknown.2,
        "bodies must not distinguish an existing account from an unknown one"
    );
    assert_eq!(h.gate.rejected(), 2);

    // The two that held slots finish as ordinary wrong-password refusals, and only they hashed.
    for t in held {
        assert_eq!(t.await.unwrap().0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        h.hasher.verifies.load(Ordering::SeqCst),
        2,
        "a refused sign-in never reaches the hasher"
    );
    assert_eq!(h.gate.available(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_account_and_wrong_password_each_cost_one_verify_through_the_handler() {
    let h = harness(2, Duration::from_secs(5), 0);
    let count = |h: &Harness| h.hasher.verifies.load(Ordering::SeqCst);

    let before = count(&h);
    let (s, ..) = send(h.router.clone(), login(&h, "alice", "wrong", None)).await;
    assert_eq!(
        (s, count(&h) - before),
        (StatusCode::UNAUTHORIZED, 1),
        "wrong password"
    );

    let before = count(&h);
    let (s, ..) = send(h.router.clone(), login(&h, "ghost", "wrong", None)).await;
    assert_eq!(
        (s, count(&h) - before),
        (StatusCode::UNAUTHORIZED, 1),
        "unknown user"
    );

    let before = count(&h);
    let (s, ..) = send(h.router.clone(), login(&h, "alice", "pw", None)).await;
    assert_eq!(
        (s, count(&h) - before),
        (StatusCode::SEE_OTHER, 1),
        "right password"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cheap_route_and_a_session_check_answer_quickly_while_the_auth_gate_is_full() {
    let h = harness(2, Duration::from_secs(30), 2500);
    let session = h
        .sessions
        .create(UserId::new("alice"), 1_000_000, 1_000_000 + 3600)
        .unwrap();
    // Two sign-ins fill the gate; three more queue behind them.
    let busy: Vec<_> = (0..5)
        .map(|i| {
            let user = ["user-1", "user-2", "user-3", "user-4", "ghost"][i];
            tokio::spawn(send(h.router.clone(), login(&h, user, "nope", None)))
        })
        .collect();
    eventually("the gate to fill", Duration::from_secs(5), || {
        h.gate.available() == 0
    })
    .await;

    let t = Instant::now();
    let (s, _, _) = send(
        h.router.clone(),
        Request::builder()
            .uri("/api/v1/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "cheap route: {:?}",
        t.elapsed()
    );

    // A session cookie is resolved without a permit: the page greets the user while the gate is full.
    let t = Instant::now();
    let (s, _, body) = send(
        h.router.clone(),
        Request::builder()
            .uri("/")
            .header(header::COOKIE, format!("planeter_session={}", session.id.0))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("alice"));
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "session check: {:?}",
        t.elapsed()
    );
    for b in busy {
        b.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_signin_never_leaks_a_permit() {
    let h = harness(1, Duration::from_secs(30), 1200);
    let running = tokio::spawn(send(h.router.clone(), login(&h, "user-1", "nope", None)));
    eventually("the slot to be taken", Duration::from_secs(5), || {
        h.gate.available() == 0
    })
    .await;

    // Cancelled while waiting: it never held a slot.
    let waited = tokio::time::timeout(
        Duration::from_millis(300),
        send(h.router.clone(), login(&h, "user-2", "nope", None)),
    )
    .await;
    assert!(waited.is_err());
    assert_eq!(h.gate.available(), 0);

    // Cancelled while running: the verification is still running, so the slot stays taken until it
    // ends, and then comes back.
    running.abort();
    assert_eq!(h.gate.available(), 0);
    eventually("the permit to return", Duration::from_secs(10), || {
        h.gate.available() == 1
    })
    .await;
    let (s, ..) = send(h.router.clone(), login(&h, "alice", "pw", None)).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_locked_account_or_address_is_refused_without_any_verify() {
    let h = harness(2, Duration::from_secs(5), 0);
    let peer: SocketAddr = "198.51.100.7:4000".parse().unwrap();

    // Five wrong passwords lock the account (C-2b); the sixth, with the RIGHT password, is refused
    // before any hashing.
    for _ in 0..5 {
        let (s, ..) = send(h.router.clone(), login(&h, "user-4", "nope", Some(peer))).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let before = h.hasher.verifies.load(Ordering::SeqCst);
    let (s, ..) = send(h.router.clone(), login(&h, "user-4", "pw", Some(peer))).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        h.hasher.verifies.load(Ordering::SeqCst),
        before,
        "locked account: no verify"
    );

    // Twenty failures across names lock the address; a fresh name from it is refused unhashed.
    for i in 0..16 {
        let (s, ..) = send(
            h.router.clone(),
            login(&h, &format!("n{i}"), "x", Some(peer)),
        )
        .await;
        assert!(s == StatusCode::UNAUTHORIZED || s == StatusCode::TOO_MANY_REQUESTS);
    }
    let before = h.hasher.verifies.load(Ordering::SeqCst);
    let (s, ..) = send(h.router.clone(), login(&h, "alice", "pw", Some(peer))).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        h.hasher.verifies.load(Ordering::SeqCst),
        before,
        "locked address: no verify"
    );
}

#[test]
fn configuration_defaults_and_bounds() {
    let d = BlockingGate::auth_from_vars(None, None).unwrap();
    assert!((1..=4).contains(&d.concurrency()));
    assert_eq!(d.wait(), Duration::from_secs(5));
    let g = BlockingGate::auth_from_vars(Some("2"), Some("9")).unwrap();
    assert_eq!((g.concurrency(), g.wait()), (2, Duration::from_secs(9)));
    for (c, w, var) in [
        (Some("0"), None, "PLANETER_AUTH_CONCURRENCY"),
        (Some("x"), None, "PLANETER_AUTH_CONCURRENCY"),
        (None, Some("0"), "PLANETER_AUTH_QUEUE_WAIT_SECS"),
    ] {
        let e = BlockingGate::auth_from_vars(c, w).unwrap_err();
        assert!(e.contains(var), "{e}");
    }
}

/// The grep-gate: no handler reaches the authentication layer except through `authn`.
#[test]
fn no_handler_touches_auth_sessions_or_oidc_directly() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut direct = Vec::new();
    for file in ["api.rs", "ui.rs", "principal.rs"] {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        for (i, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("//") {
                continue;
            }
            let bad = t.contains("state.auth.")
                || t.contains("state.sessions.")
                || t.contains("state.auth\n")
                || t == ".auth"
                || (t.contains("state.oidc")
                    && !t.contains("state.oidc.is_some()")
                    && !t.contains("state.oidc.is_none()"));
            if bad {
                direct.push(format!("{file}:{}: {t}", i + 1));
            }
        }
    }
    assert!(direct.is_empty(), "go through crate::authn: {direct:?}");
}
