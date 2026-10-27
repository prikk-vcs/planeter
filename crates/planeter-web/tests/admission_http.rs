//! RFC 011 D-1, D-3, D-4 through the router: `too_large` (422), `busy`, existence-neutrality, the bounded
//! content, the UI page, and the page whose optional `verify` does not fit.
//!
//! Stand-in repositories (a `.prikk` directory of a chosen size, no prikk) drive the admission tests;
//! the content and page tests build a sealed repository with real prikk and self-skip without it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};
use planeter_web::budget::MIB;
use planeter_web::catalog::EN;
use planeter_web::errors::ErrorCode;
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("admission-http-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A stand-in repository whose store holds `store_mib` MiB (one file under `.prikk`).
fn stand_in(root: &Path, name: &str, store_mib: u64) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join(".prikk")).unwrap();
    std::fs::write(
        dir.join(".prikk/blob"),
        vec![0u8; (store_mib * MIB) as usize],
    )
    .unwrap();
    dir
}

struct Repo {
    name: &'static str,
    visibility: Visibility,
    path: PathBuf,
}

/// Which prikk the hosted repositories use.
enum Prikk {
    Real,
    Missing,
    Script(PathBuf),
}

fn app(repos: Vec<Repo>, gate: Arc<PrikkGate>, prikk: Prikk) -> (Router, AppState) {
    let root = scratch("hosting-root");
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (i, r) in repos.into_iter().enumerate() {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(format!("r{i}")),
                owner: Owner::User("alice".into()),
                name: r.name.into(),
                visibility: r.visibility,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path: r.path,
            })
            .unwrap();
    }
    let mut hosting = HostingService::new(root, store as Arc<dyn RepositoryStore>)
        .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    match prikk {
        Prikk::Real => {}
        Prikk::Missing => hosting = hosting.with_binary("/nonexistent/prikk"),
        Prikk::Script(path) => hosting = hosting.with_binary(path),
    }
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        Arc::new(hosting),
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    let state = AppState {
        read,
        gate,
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
    };
    (router(state.clone()), state)
}

type Reply = (StatusCode, HeaderMap, String);

async fn get(r: &Router, uri: &str, host: &str) -> Reply {
    let resp = r
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::HOST, host)
                .header(header::COOKIE, "planeter_csrf=t")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 8 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

const APP: &str = "app.planeter.example";
const RAW: &str = "raw.planeter.example";

// ---------------------------------------------------------------- too_large / busy through the API

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repository_above_the_budget_is_422_too_large_for_a_reader_and_404_for_everyone_else() {
    let root = scratch("repos");
    let repos = vec![
        Repo {
            name: "open",
            visibility: Visibility::Public,
            path: stand_in(&root, "open", 40),
        },
        Repo {
            name: "secret",
            visibility: Visibility::Private,
            path: stand_in(&root, "secret", 40),
        },
    ];
    // 32 MiB budget (a 16 MiB light and a 16 MiB heavy pool); a `tree` of a 40 MiB store costs 84 MiB.
    let gate = Arc::new(PrikkGate::new(8, Duration::from_secs(5)).with_budget(32 * MIB));
    let (r, state) = app(repos, gate, Prikk::Missing);

    let (status, headers, body) = get(&r, "/api/v1/repos/alice/open/tree", APP).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(body, ErrorCode::TooLarge.body());
    assert!(body.contains("\"code\":\"too_large\""));
    assert!(body.contains("This is too large to serve here."));
    assert!(body.contains("raise the read memory budget"), "{body}");

    // A raw file of the same repository above the budget is `too_large` for the *budget's* reason, so it
    // keeps the budget's hint (the raw route's own hint is for a file above the raw cap).
    let (status, _, body) = get(&r, "/api/v1/repos/alice/open/raw?path=x", RAW).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body, ErrorCode::TooLarge.body());
    assert!(!body.contains("does not serve files that large"), "{body}");

    // The same request for a repository the caller may not read — and for one that does not exist — is the
    // one `not_found`, byte for byte: `too_large` cannot reveal a hidden repository.
    let hidden = get(&r, "/api/v1/repos/alice/secret/tree", APP).await;
    let absent = get(&r, "/api/v1/repos/alice/nope/tree", APP).await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert_eq!(
        (hidden.0, &hidden.1, &hidden.2),
        (absent.0, &absent.1, &absent.2)
    );
    assert_eq!(hidden.2, ErrorCode::NotFound.body());

    // A cheap verb on the same repository fits the budget and gets as far as prikk (a stand-in that is
    // not there): `unavailable`, not `too_large`.
    let (status, _, body) = get(&r, "/api/v1/repos/alice/open/history", APP).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.contains("\"unavailable\""));

    let s = state.gate.stats();
    assert_eq!(s.refused_too_large, 2, "{s:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_budget_is_503_with_retry_after_and_never_for_a_hidden_repository() {
    let root = scratch("busy");
    let repos = vec![
        Repo {
            name: "open",
            visibility: Visibility::Public,
            path: stand_in(&root, "open", 1),
        },
        Repo {
            name: "secret",
            visibility: Visibility::Private,
            path: stand_in(&root, "secret", 1),
        },
    ];
    // 64 MiB: a 32 MiB light pool and a 32 MiB heavy pool. `history` is a floor-class (light) read.
    let gate = Arc::new(PrikkGate::new(8, Duration::from_millis(250)).with_budget(64 * MIB));
    let (r, state) = app(repos, Arc::clone(&gate), Prikk::Missing);

    // Other reads hold the whole light pool for a while (two floor reads of 16 MiB).
    let hold: Vec<_> = (0..2)
        .map(|_| {
            let g = Arc::clone(&gate);
            tokio::spawn(async move {
                g.run_weighted(16 * MIB, "log", || {
                    std::thread::sleep(Duration::from_millis(900))
                })
                .await
            })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (status, headers, body) = get(&r, "/api/v1/repos/alice/open/history", APP).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("\"busy\""), "{body}");
    assert!(headers.get(header::RETRY_AFTER).is_some());

    // A repository the caller may not read is `not_found` straight away — it never reaches the queue, so
    // load cannot be used to tell it from an absent one.
    let hidden = get(&r, "/api/v1/repos/alice/secret/history", APP).await;
    let absent = get(&r, "/api/v1/repos/alice/nope/history", APP).await;
    assert_eq!((hidden.0, &hidden.2), (StatusCode::NOT_FOUND, &absent.2));

    for h in hold {
        h.await.unwrap().unwrap();
    }
    assert_eq!(state.gate.stats().refused_busy, 1);
}

// ---------------------------------------------------------------- the UI

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_repository_page_renders_a_plain_too_large_page() {
    let root = scratch("ui");
    let repos = vec![Repo {
        name: "open",
        visibility: Visibility::Public,
        path: stand_in(&root, "open", 40),
    }];
    let gate = Arc::new(PrikkGate::new(8, Duration::from_secs(5)).with_budget(32 * MIB));
    let (r, _) = app(repos, gate, Prikk::Missing);
    let (status, _, html) = get(&r, "/alice/open", APP).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        html.contains("This is too large for this server to show."),
        "{html}"
    );
    assert!(
        html.contains("ask the person who runs this server to raise the limit"),
        "{html}"
    );
    let (status, _, html) = get(&r, "/alice/open/blob/anything.bin", APP).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{html}");
}

// ---------------------------------------------------------------- with real prikk

/// A sealed repository holding a 1 MiB file, a `big_mib` MiB file and, if `huge_mib > 0`, a `huge_mib` MiB one.
fn sealed(name: &str, big_mib: usize, huge_mib: usize) -> PathBuf {
    let base = scratch(name);
    let repo = base.join("repo");
    let keys = base.join("keys");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    let prikk = |args: &[&str]| {
        let ok = Command::new("prikk")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &keys)
            .env("XDG_CONFIG_HOME", &keys)
            .status()
            .unwrap();
        assert!(ok.success(), "prikk {args:?}");
    };
    prikk(&["setup"]);
    let mut x: u64 = 0xA5A5_5A5A_1234_8765;
    let mut bytes = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    };
    std::fs::write(repo.join("small.bin"), bytes(1 << 20)).unwrap();
    std::fs::write(repo.join("big.bin"), bytes(big_mib << 20)).unwrap();
    if huge_mib > 0 {
        std::fs::write(repo.join("huge.bin"), bytes(huge_mib << 20)).unwrap();
    }
    std::fs::write(repo.join("README.md"), b"# hello\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_is_bounded_at_4_mib_and_raw_keeps_its_25_mib_cap_each_with_its_own_hint() {
    if !have("prikk") || !have("prlimit") {
        eprintln!("skipping: needs prikk and prlimit on PATH");
        return;
    }
    // small.bin 1 MiB, big.bin 5 MiB (above the 4 MiB `file` bound, below the 25 MiB raw cap), huge.bin
    // 26 MiB (above both).
    let repos = vec![Repo {
        name: "open",
        visibility: Visibility::Public,
        path: sealed("content", 5, 26),
    }];
    let (r, _) = app(
        repos,
        Arc::new(PrikkGate::default().with_budget(2048 * MIB)),
        Prikk::Real,
    );
    let raw_hint_body = ErrorCode::TooLarge.body_for(ErrorCode::TOO_LARGE_RAW, EN);

    // Below both bounds: `file` is 200 with a kind-tagged content; raw is 200 on the content host.
    let (status, _, body) = get(&r, "/api/v1/repos/alice/open/file?path=small.bin", APP).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"kind\":\"binary\""), "{body}");
    let (status, headers, _) = get(&r, "/api/v1/repos/alice/open/raw?path=small.bin", RAW).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(header::CONTENT_DISPOSITION).is_some());

    // 5 MiB: `file` refuses (422, the default hint: the file can be downloaded), raw still serves it —
    // the raw route was not reduced to the `file` bound (0.3.0 served up to 25 MiB).
    let (status, _, body) = get(&r, "/api/v1/repos/alice/open/file?path=big.bin", APP).await;
    assert_eq!(
        (status, body.as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::TooLarge.body().as_str()
        )
    );
    let (status, headers, _) = get(&r, "/api/v1/repos/alice/open/raw?path=big.bin", RAW).await;
    assert_eq!(status, StatusCode::OK, "raw must serve a 5 MiB file");
    assert!(headers.get(header::CONTENT_DISPOSITION).is_some());

    // 26 MiB: both refuse; the hint is true for the route. `file` says "download it" (raw is where),
    // raw says that this server does not serve a file that large.
    let (status, _, body) = get(&r, "/api/v1/repos/alice/open/file?path=huge.bin", APP).await;
    assert_eq!(
        (status, body.as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::TooLarge.body().as_str()
        )
    );
    let (status, headers, body) = get(&r, "/api/v1/repos/alice/open/raw?path=huge.bin", RAW).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, raw_hint_body.as_str())
    );
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    // One code, one message, one status: only the hint differs between the two causes.
    let (file_v, raw_v): (serde_json::Value, serde_json::Value) = (
        serde_json::from_str(&ErrorCode::TooLarge.body()).unwrap(),
        serde_json::from_str(&raw_hint_body).unwrap(),
    );
    assert_eq!(file_v["code"], raw_v["code"]);
    assert_eq!(file_v["message"], raw_v["message"]);
    assert_ne!(file_v["hint"], raw_v["hint"]);
    assert_eq!(
        raw_v["hint"],
        "This server does not serve files that large; ask the person who runs this server."
    );

    // A path that is not there is still a plain 404.
    let (status, _, _) = get(&r, "/api/v1/repos/alice/open/file?path=nope", APP).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = get(&r, "/api/v1/repos/alice/open/raw?path=nope", RAW).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The UI file page keeps the 4 MiB bound (as in 0.3.0) and its default hint.
    let (status, _, html) = get(&r, "/alice/open/blob/big.bin", APP).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        html.contains("This is too large for this server to show."),
        "{html}"
    );
    assert!(html.contains("to raise the limit"), "{html}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repository_whose_verify_does_not_fit_still_gets_its_page() {
    if !have("prikk") || !have("prlimit") {
        eprintln!("skipping: needs prikk and prlimit on PATH");
        return;
    }
    // Store ≈ 8.4 MiB: `tree` costs ≈ 17.6 MiB (a light read), `verify` ≈ 34.5 MiB (heavy). A 64 MiB budget
    // (32 MiB light, 32 MiB heavy) admits the first, not the second — and the page must not be refused for the sake of its optional badge.
    let repos = vec![Repo {
        name: "open",
        visibility: Visibility::Public,
        path: sealed("badge", 7, 0),
    }];
    let gate = Arc::new(PrikkGate::new(8, Duration::from_secs(5)).with_budget(64 * MIB));
    let (r, state) = app(repos, gate, Prikk::Real);
    let (status, _, html) = get(&r, "/alice/open", APP).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("Could not be checked now"),
        "the badge degrades: {html}"
    );
    assert!(html.contains("README.md"), "the listing is there: {html}");
    assert_eq!(
        state.gate.stats().refused_too_large,
        1,
        "exactly the verify was refused"
    );
    // The dedicated verify page, whose only call is the one that does not fit, is `too_large`.
    let (status, _, _) = get(&r, "/alice/open/verify", APP).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ---------------------------------------------------------------- the ceiling the server supplies (T5)

/// A stand-in prikk that answers the version pin and, for any other verb, records the data limit it was
/// started under (`/proc/self/limits`) in the repository's store directory and exits 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_call_runs_under_the_documented_ceiling() {
    if !have("prlimit") {
        eprintln!("skipping: needs prlimit on PATH");
        return;
    }
    use planeter_core::admission::{ReadVerb, ceiling_for};
    let root = scratch("ceiling");
    let repo = stand_in(&root, "open", 40); // a 40 MiB store
    let script = root.join("prikk");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.54.1';; *) grep 'Max data size' /proc/self/limits | awk '{print $4}' > \"$PWD/.prikk/observed\"; exit 1;; esac\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let budget = 1000 * MIB;
    let repos = vec![Repo {
        name: "open",
        visibility: Visibility::Public,
        path: repo.clone(),
    }];
    let gate = Arc::new(PrikkGate::new(8, Duration::from_secs(5)).with_budget(budget));
    let (r, _) = app(repos, gate, Prikk::Script(script));
    let store = planeter_core::admission::store_size(&repo).unwrap();
    assert_eq!(store, 40 * MIB);

    for (uri, verb) in [
        ("/api/v1/repos/alice/open/history", ReadVerb::Log),
        ("/api/v1/repos/alice/open/tree", ReadVerb::Tree),
        ("/api/v1/repos/alice/open/verify", ReadVerb::Verify),
    ] {
        // S as the server will see it for this request (the stand-in's own `observed` file is part of it).
        let store = planeter_core::admission::store_size(&repo).unwrap();
        let (status, _, _) = get(&r, uri, APP).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}"); // the stand-in exits 1
        let seen: u64 = std::fs::read_to_string(repo.join(".prikk/observed"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let want = ceiling_for(verb.cost(store), Some(budget));
        assert_eq!(
            seen, want,
            "{verb:?}: the call ran under {seen}, the documented ceiling is {want}"
        );
    }
    // Spot values: log is the 16 MiB floor × 1.5; verify is 4.1 × 40 MiB × 1.5.
    assert_eq!(
        ceiling_for(ReadVerb::Log.cost(store), Some(budget)),
        24 * MIB
    );
    assert_eq!(
        ceiling_for(ReadVerb::Verify.cost(store), Some(budget)),
        246 * MIB
    );
}
