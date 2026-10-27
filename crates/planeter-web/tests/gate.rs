//! IS-4 / RFC 003 D-8: read-path prikk work runs off the async workers, under a bounded number of
//! concurrent invocations, and past a bounded wait a request is refused with `503` + `Retry-After`.
//!
//! A stand-in `prikk` that sleeps plays the slow subprocess; it needs no real prikk, so these tests
//! never self-skip. The router is driven with `ServiceExt::oneshot` on a multi-thread runtime of **two**
//! workers, so a handler that blocked a worker would visibly stall the "cheap route" test.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};
use planeter_web::{AppState, ContentOrigin, PrikkGate, router};

/// `--version` answers at once; `log` records start/end, sleeps for `$PWD/sleep` seconds, and answers
/// with an empty, valid `log-report-v1`. The per-test data lives in the working directory, which the
/// driver sets to the repository root, so one script serves every test.
const STAND_IN: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "prikk 0.54.1"; exit 0 ;;
  log)
    echo "start $$ $(date +%s%N)" >> "$PWD/events"
    sleep "$(cat "$PWD/sleep")"
    echo "end $$ $(date +%s%N)" >> "$PWD/events"
    echo '{"schema_version":"log-report-v1","repository":"/r/.prikk","ref":"heads/main","current_branch":null,"blocks":[]}'
    exit 0 ;;
  *) echo "unexpected: $*" >&2; exit 99 ;;
esac
"#;

/// The shared script, written **once** before any test spawns a process (avoids the write-then-exec
/// `ETXTBSY` race between parallel tests).
fn stand_in() -> PathBuf {
    static ONCE: Once = Once::new();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gate-stand-in");
    let script = dir.join("prikk");
    ONCE.call_once(|| {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&script, STAND_IN).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    });
    script
}

struct Harness {
    router: Router,
    gate: Arc<PrikkGate>,
    repo: PathBuf,
}

/// A public repository `alice/app` whose prikk is the stand-in, sleeping `sleep_secs` per `log`.
fn harness(name: &str, concurrency: usize, wait: Duration, sleep_secs: &str) -> Harness {
    let binary = stand_in();
    let repo = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("gate-{name}"));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("sleep"), sleep_secs).unwrap();
    // Admission measures `.prikk` before any prikk call; the stand-in repository carries a store directory.
    std::fs::create_dir_all(repo.join(".prikk")).unwrap();

    let store = Arc::new(InMemoryRepositoryStore::new());
    store
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "app".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: repo.clone(),
        })
        .expect("seed record");
    let hosting = Arc::new(
        HostingService::new(repo.clone(), store as Arc<dyn RepositoryStore>)
            .with_binary(binary)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Arc::new(Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    ));
    let gate = Arc::new(PrikkGate::new(concurrency, wait));
    let state = AppState {
        read,
        gate: Arc::clone(&gate),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(planeter_auth::InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::parse("10.0.0.0/8").unwrap()),
        oidc: None,
        transport: None,
        creation: None,
    };
    Harness {
        router: router(state),
        gate,
        repo,
    }
}

const HISTORY: &str = "/api/v1/repos/alice/app/history";

async fn get(router: Router, path: &str) -> (StatusCode, axum::http::HeaderMap, usize) {
    let resp = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    (parts.status, parts.headers, bytes.len())
}

/// The largest number of stand-in `log` invocations that overlapped in time.
fn max_overlap(repo: &Path) -> usize {
    let text = std::fs::read_to_string(repo.join("events")).unwrap_or_default();
    let mut events: Vec<(u128, i32)> = text
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let kind = w.next()?;
            let _pid = w.next()?;
            let t: u128 = w.next()?.parse().ok()?;
            Some((t, if kind == "start" { 1 } else { -1 }))
        })
        .collect();
    // At equal times process ends first, so back-to-back jobs do not count as overlapping.
    events.sort_by_key(|&(t, d)| (t, d));
    let (mut cur, mut max) = (0i32, 0i32);
    for (_, d) in events {
        cur += d;
        max = max.max(cur);
    }
    max as usize
}

async fn eventually(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < limit, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cap_bounds_running_prikk_and_the_extra_request_waits_then_succeeds() {
    let h = harness("cap", 2, Duration::from_secs(30), "1");
    let start = Instant::now();
    let tasks: Vec<_> = (0..3)
        .map(|_| tokio::spawn(get(h.router.clone(), HISTORY)))
        .collect();
    for t in tasks {
        let (status, _, _) = t.await.unwrap();
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(max_overlap(&h.repo), 2, "at most N prikk run at once");
    assert!(
        start.elapsed() >= Duration::from_millis(1900),
        "the third request had to wait for a slot: {:?}",
        start.elapsed()
    );
    assert_eq!(h.gate.rejected(), 0);
    assert_eq!(h.gate.available(), 2, "every permit came back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn past_the_bounded_wait_the_request_is_503_with_retry_after_and_counted() {
    let h = harness("busy", 1, Duration::from_secs(1), "3");
    let first = tokio::spawn(get(h.router.clone(), HISTORY));
    eventually(
        "the first request to hold the slot",
        Duration::from_secs(5),
        || h.gate.available() == 0,
    )
    .await;

    let started = Instant::now();
    let (status, headers, body_len) = get(h.router.clone(), HISTORY).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers.get(header::RETRY_AFTER).unwrap(), "1");
    assert_eq!(
        body_len,
        planeter_web::errors::ErrorCode::Busy.body().len(),
        "existence-neutral: the fixed `busy` body, no prikk output"
    );
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(h.gate.rejected(), 1);

    // The request that held the slot is unaffected.
    assert_eq!(first.await.unwrap().0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cheap_route_answers_in_under_a_second_while_every_slot_is_busy() {
    let h = harness("cheap", 2, Duration::from_secs(30), "3");
    // Two requests fill both slots; three more queue behind them.
    let busy: Vec<_> = (0..5)
        .map(|_| tokio::spawn(get(h.router.clone(), HISTORY)))
        .collect();
    eventually("both slots to fill", Duration::from_secs(5), || {
        h.gate.available() == 0
    })
    .await;

    let started = Instant::now();
    let (status, _, body_len) = get(h.router.clone(), "/api/v1/openapi.json").await;
    let took = started.elapsed();
    assert_eq!(status, StatusCode::OK);
    assert!(body_len > 0);
    assert!(
        took < Duration::from_secs(1),
        "the cheap route took {took:?} while prikk was saturated"
    );
    for t in busy {
        t.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_request_never_leaks_a_permit() {
    let h = harness("cancel", 1, Duration::from_secs(30), "1.5");
    let running = tokio::spawn(get(h.router.clone(), HISTORY));
    eventually("the slot to be taken", Duration::from_secs(5), || {
        h.gate.available() == 0
    })
    .await;

    // Cancelled while *waiting*: it never held a slot, so nothing changes.
    let waited =
        tokio::time::timeout(Duration::from_millis(300), get(h.router.clone(), HISTORY)).await;
    assert!(waited.is_err(), "the waiter must have been cancelled");
    assert_eq!(
        h.gate.available(),
        0,
        "the running job still holds its slot"
    );

    // Cancelled while *running*: its prikk is still running, so the slot is still taken (the cap
    // counts running processes) and comes back when that process ends.
    running.abort();
    assert_eq!(h.gate.available(), 0);
    eventually("the permit to return", Duration::from_secs(10), || {
        h.gate.available() == 1
    })
    .await;
    assert_eq!(h.gate.rejected(), 0);

    // And the gate is fully usable again.
    let (status, _, _) = get(h.router.clone(), HISTORY).await;
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn configuration_defaults_and_bounds() {
    let d = PrikkGate::from_vars(None, None).unwrap();
    assert!((1..=8).contains(&d.concurrency()));
    assert_eq!(d.wait(), Duration::from_secs(10));

    let g = PrikkGate::from_vars(Some("3"), Some("7")).unwrap();
    assert_eq!((g.concurrency(), g.wait()), (3, Duration::from_secs(7)));

    // Empty means unset (an exported-but-blank variable is not an error).
    assert!(PrikkGate::from_vars(Some(""), Some("")).is_ok());

    for (c, w, var) in [
        (Some("0"), None, "PLANETER_PRIKK_CONCURRENCY"),
        (Some("-1"), None, "PLANETER_PRIKK_CONCURRENCY"),
        (Some("many"), None, "PLANETER_PRIKK_CONCURRENCY"),
        (None, Some("0"), "PLANETER_PRIKK_QUEUE_WAIT_SECS"),
        (None, Some("1.5"), "PLANETER_PRIKK_QUEUE_WAIT_SECS"),
    ] {
        let e = PrikkGate::from_vars(c, w).unwrap_err();
        assert!(e.contains(var), "{e}");
    }
}

/// The grep-gate: no handler reaches the read service except through `read_blocking`. The only
/// permitted mention of `state.read` is the one inside that helper.
#[test]
fn no_handler_calls_the_read_service_directly() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut direct = Vec::new();
    for file in ["api.rs", "ui.rs"] {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        for (i, line) in text.lines().enumerate() {
            let t = line.trim();
            let mentions = t.contains("state.read") || t == ".read" || t.starts_with(".read.");
            let is_comment = t.starts_with("//");
            if mentions && !is_comment {
                direct.push(format!("{file}:{}: {t}", i + 1));
            }
        }
    }
    assert_eq!(
        direct.len(),
        1,
        "only read_blocking may touch state.read: {direct:?}"
    );
    assert!(
        direct[0].starts_with("api.rs:") && direct[0].contains("Arc::clone(&state.read)"),
        "{direct:?}"
    );
}
