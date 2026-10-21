//! RFC 004 part 2: advertise and fetch over HTTP, with **real prikk** (the tests self-skip, printing why, without
//! it). The router is driven directly; the export runs prikk for real (unconfined, except the one test that
//! needs bubblewrap and the real sandbox with the spool-out mount).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util_shim::collect;
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{
    HostingService, LeaseClass, LeaseManager, NullAuditSink, ReadService, RepoScope, Scope,
    ScopeAccess, new_boot_id,
};
use planeter_store::{
    InMemoryLeaseRecords, InMemoryMembershipStore, InMemoryRepositoryStore, InMemoryRevisions,
    MembershipStore, Owner, RepoId, RepositoryRecord, RepositoryStore, RevisionStore, UserId,
    Visibility,
};
use planeter_transport::{ExportConfig, ExportService, Metrics, Spool};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};

/// Collect a response body (the axum body is a stream; no extra crate is needed).
mod http_body_util_shim {
    use axum::body::Body;
    pub async fn collect(body: Body) -> Vec<u8> {
        axum::body::to_bytes(body, 1 << 30).await.unwrap().to_vec()
    }
}

static NOW: AtomicU64 = AtomicU64::new(1_000_000);
/// The tests that move the clock (the rate-limit windows) take turns.
static CLOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn bwrap_works() -> bool {
    Command::new("bwrap")
        .args([
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/lib",
            "/lib64",
            "--symlink",
            "usr/bin",
            "/bin",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--unshare-all",
            "/usr/bin/true",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("transport-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run prikk in `dir` with the key directory `keys`.
fn prikk(dir: &Path, keys: &Path, args: &[&str]) {
    let st = Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

/// A sealed repository with a branch, a tag and a few files, built with a key directory of its own.
fn sealed_repo(base: &Path, name: &str) -> PathBuf {
    let repo = base.join(name);
    let keys = base.join(format!("{name}-keys"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    prikk(&repo, &keys, &["setup"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("README.md"), b"# Hello\n\nA readme.\n").unwrap();
    std::fs::write(repo.join("src/main.rs"), b"fn main() {}\n").unwrap();
    // Some bulk, so a Range has something to slice.
    std::fs::write(
        repo.join("data.bin"),
        (0..200_000u32)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    prikk(&repo, &keys, &["commit", "--from-worktree", "-m", "init"]);
    prikk(&repo, &keys, &["seal", "--allow-no-audit"]);
    prikk(&repo, &keys, &["branch", "create", "heads/dev"]);
    prikk(
        &repo,
        &keys,
        &["tag", "create", "tags/v1", "--target", "heads/main"],
    );
    repo
}

struct Fx {
    router: Router,
    export: Arc<ExportService>,
    revisions: Arc<InMemoryRevisions>,
    leases: Arc<LeaseManager>,
    spool_dir: PathBuf,
    open_path: PathBuf,
    metrics: Arc<Metrics>,
}

struct Opts {
    binary: Option<PathBuf>,
    config: ExportConfig,
    ip_limit: u32,
    principal_limit: u32,
    sandbox: planeter_prikk::Sandbox,
    transport: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            binary: None,
            config: ExportConfig {
                lease_wait: Duration::from_secs(3),
                ..ExportConfig::default()
            },
            ip_limit: 10_000,
            principal_limit: 10_000,
            sandbox: planeter_prikk::Sandbox::Unconfined,
            transport: true,
        }
    }
}

/// `alice/open` (public), `alice/app` (private), `alice/recv` (a repository that only *received* a ref).
fn fixture(name: &str, o: Opts) -> Fx {
    let base = scratch(name);
    let src = sealed_repo(&base, "src");
    let copy = |to: &str| -> PathBuf {
        let dest = base.join(to);
        let st = Command::new("cp")
            .arg("-a")
            .arg(&src)
            .arg(&dest)
            .status()
            .unwrap();
        assert!(st.success());
        dest
    };
    let open_path = copy("open");
    let app_path = copy("app");
    // `recv`: a keyless repository that imported an archive without adopting it: a received ref only.
    let recv_path = base.join("recv");
    std::fs::create_dir_all(&recv_path).unwrap();
    let keys = base.join("recv-keys");
    std::fs::create_dir_all(&keys).unwrap();
    let arc = base.join("src.archive");
    prikk(
        &src,
        &base.join("src-keys"),
        &["archive", "export", arc.to_str().unwrap()],
    );
    prikk(&recv_path, &keys, &["init", "."]);
    prikk(
        &recv_path,
        &keys,
        &["archive", "import", arc.to_str().unwrap()],
    );

    let store = Arc::new(InMemoryRepositoryStore::new());
    for (id, name, vis, path) in [
        ("r-open", "open", Visibility::Public, &open_path),
        ("r-app", "app", Visibility::Private, &app_path),
        ("r-recv", "recv", Visibility::Public, &recv_path),
    ] {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(id),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility: vis,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path: path.clone(),
            })
            .unwrap();
    }
    let mut hosting =
        HostingService::new(base.join("root"), store.clone() as Arc<dyn RepositoryStore>)
            .with_sandbox(o.sandbox);
    if let Some(b) = &o.binary {
        hosting = hosting.with_binary(b);
    }
    let hosting = Arc::new(hosting);
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        membership,
        Arc::new(NullAuditSink),
    ));
    let accounts = Arc::new(InMemoryAccountStore::new());
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    creds
        .add_token(TokenRecord {
            hash: TokenHasher::hash(&*hasher, "tok-alice"),
            user: UserId::new("alice"),
            scope: Scope {
                access: ScopeAccess::ReadOnly,
                repos: RepoScope::All,
                expires_at: None,
            },
        })
        .unwrap();
    let auth = Arc::new(Authenticator::new(accounts, creds, hasher.clone(), hasher));

    let spool_dir = base.join("spool");
    let spool = Spool::open(&spool_dir, 512 << 20).unwrap();
    let revisions = Arc::new(InMemoryRevisions::new());
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        now,
        Duration::from_secs(3),
    ));
    let metrics = Arc::new(Metrics::new());
    let export = Arc::new(ExportService::new(
        hosting,
        spool,
        leases.clone(),
        revisions.clone() as Arc<dyn RevisionStore>,
        o.config,
        metrics.clone(),
    ));
    let transport = o.transport.then(|| {
        Arc::new(TransportRuntime {
            export: export.clone(),
            ip_limit: RateLimiter::new(o.ip_limit, Duration::from_secs(60)),
            principal_limit: RateLimiter::new(o.principal_limit, Duration::from_secs(60)),
        })
    });
    let router = router(AppState {
        read,
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(planeter_auth::InMemorySessionStore::new()),
        now_unix: now,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport,
    });
    Fx {
        router,
        export,
        revisions,
        leases,
        spool_dir,
        open_path,
        metrics,
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn h(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
    fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or("").to_owned()
    }
}

async fn call(r: &Router, uri: &str, extra: &[(&str, &str)]) -> Reply {
    let mut req = Request::builder().uri(uri);
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let resp = r
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    Reply {
        status,
        headers,
        body: collect(resp.into_body()).await,
    }
}

const FETCH_ARCHIVE: &str = "/api/v1/repos/alice/open/transport/fetch?kind=archive";
const FETCH_BUNDLE: &str = "/api/v1/repos/alice/open/transport/fetch?kind=bundle&ref=heads/main";
const ADVERTISE: &str = "/api/v1/repos/alice/open/transport/advertise";
const TOKEN: (&str, &str) = ("authorization", "Bearer tok-alice");

fn skip() -> bool {
    if !have("prikk") {
        eprintln!("skipping: needs prikk on PATH");
        return true;
    }
    false
}

fn prikk_verify_file(kind: &str, path: &Path) -> bool {
    let args: Vec<&str> = if kind == "archive" {
        vec!["archive", "verify", path.to_str().unwrap()]
    } else {
        vec!["bundle", "verify", "--input", path.to_str().unwrap()]
    };
    Command::new("prikk")
        .args(args)
        .output()
        .unwrap()
        .status
        .success()
}

fn write_tmp(name: &str, bytes: &[u8]) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("transport-served-{name}"));
    std::fs::write(&p, bytes).unwrap();
    p
}

// ================================================================================================ advertise

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn advertise_names_the_refs_the_kinds_the_fingerprint_and_no_path() {
    if skip() {
        return;
    }
    let fx = fixture("advertise", Opts::default());
    let r = call(&fx.router, ADVERTISE, &[]).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert!(!r.h("etag").is_empty());
    let j = r.json();
    let fp = j["fingerprint"].as_str().unwrap();
    assert_eq!(fp.len(), 32);
    assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    let refs = j["refs"].as_array().unwrap();
    let names: BTreeSet<&str> = refs.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["heads/main", "heads/dev", "tags/v1"])
    );
    let main = refs.iter().find(|x| x["name"] == "heads/main").unwrap();
    assert_eq!(main["state"], "published");
    assert_eq!(main["kind"], "branch");
    assert_eq!(main["tip_block"].as_str().unwrap().len(), 64);
    assert_eq!(main["fetch"][0]["kind"], "bundle");
    assert_eq!(main["fetch"][0]["ref"], "heads/main");
    assert!(
        main["fetch"][0]["size"].is_null(),
        "no size before the first export"
    );
    assert!(main["fetch"][0]["etag"].as_str().unwrap().starts_with('"'));
    assert_eq!(j["archive"]["kind"], "archive");
    assert!(j["archive"]["ref"].is_null() && j["archive"]["size"].is_null());
    // No filesystem path anywhere in the body.
    let body = String::from_utf8_lossy(&r.body);
    assert!(
        !body.contains(env!("CARGO_TARGET_TMPDIR"))
            && !body.contains("/open")
            && !body.contains(".prikk"),
        "{body}"
    );
    // The ETag it advertises is the one a fetch returns; after the fetch the size is known.
    let advertised = j["archive"]["etag"].as_str().unwrap().to_owned();
    let f = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(f.status, StatusCode::OK);
    assert_eq!(f.h("etag"), advertised);
    let again = call(&fx.router, ADVERTISE, &[]).await.json();
    assert_eq!(
        again["archive"]["size"].as_u64().unwrap(),
        f.body.len() as u64
    );
    assert_eq!(
        again["fingerprint"], j["fingerprint"],
        "an export does not change the state"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_received_ref_is_advertised_as_received_with_no_bundle_and_the_archive_carries_it() {
    if skip() {
        return;
    }
    let fx = fixture("received", Opts::default());
    let j = call(
        &fx.router,
        "/api/v1/repos/alice/recv/transport/advertise",
        &[],
    )
    .await
    .json();
    let refs = j["refs"].as_array().unwrap();
    let recv = refs
        .iter()
        .find(|x| x["name"] == "remotes/heads/main")
        .expect("a received ref");
    assert_eq!(recv["state"], "received");
    assert_eq!(recv["kind"], "received");
    assert_eq!(
        recv["fetch"].as_array().unwrap().len(),
        0,
        "no bundle is offered for a received ref"
    );
    // A bundle of it is refused with a named reason that points to the archive (PK-60)…
    let b = call(
        &fx.router,
        "/api/v1/repos/alice/recv/transport/fetch?kind=bundle&ref=remotes/heads/main",
        &[],
    )
    .await;
    assert_eq!(b.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(b.code(), "bundle_of_received_ref");
    assert!(b.json()["hint"].as_str().unwrap().contains("kind=archive"));
    // …and the archive of that repository verifies and carries it.
    let a = call(
        &fx.router,
        "/api/v1/repos/alice/recv/transport/fetch?kind=archive",
        &[],
    )
    .await;
    assert_eq!(a.status, StatusCode::OK);
    assert!(prikk_verify_file(
        "archive",
        &write_tmp("recv.archive", &a.body)
    ));
}

// ================================================================================================ authorization

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_private_repository_needs_its_reader_and_hidden_and_absent_answer_alike() {
    if skip() {
        return;
    }
    let fx = fixture("authorization", Opts::default());
    for route in [
        "advertise",
        "fetch?kind=archive",
        "fetch?kind=bundle&ref=heads/main",
        "fetch",
    ] {
        let hidden = call(
            &fx.router,
            &format!("/api/v1/repos/alice/app/transport/{route}"),
            &[],
        )
        .await;
        let absent = call(
            &fx.router,
            &format!("/api/v1/repos/alice/ghost/transport/{route}"),
            &[],
        )
        .await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{route}");
        assert_eq!(absent.status, hidden.status, "{route}");
        assert_eq!(
            absent.body, hidden.body,
            "{route}: a hidden and an absent repository read the same"
        );
        assert_eq!(hidden.code(), "not_found");
        let h: Vec<(&str, &str)> = ["content-type", "cache-control", "vary"]
            .iter()
            .map(|n| (*n, hidden.h(n)))
            .collect();
        let a: Vec<(&str, &str)> = ["content-type", "cache-control", "vary"]
            .iter()
            .map(|n| (*n, absent.h(n)))
            .collect();
        assert_eq!(h, a, "{route}");
    }
    // The reader (the owner, by token) gets both.
    let ok = call(
        &fx.router,
        "/api/v1/repos/alice/app/transport/advertise",
        &[TOKEN],
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);
    let f = call(
        &fx.router,
        "/api/v1/repos/alice/app/transport/fetch?kind=archive",
        &[TOKEN],
    )
    .await;
    assert_eq!(f.status, StatusCode::OK);
    assert!(prikk_verify_file(
        "archive",
        &write_tmp("app.archive", &f.body)
    ));
    // A wrong token is a stranger.
    let bad = call(
        &fx.router,
        "/api/v1/repos/alice/app/transport/advertise",
        &[("authorization", "Bearer nope")],
    )
    .await;
    assert_eq!(bad.status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_spool_a_reader_is_told_so_and_a_stranger_still_gets_404() {
    if skip() {
        return;
    }
    let fx = fixture(
        "off",
        Opts {
            transport: false,
            ..Opts::default()
        },
    );
    let off = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(off.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(off.code(), "transport_off");
    let off = call(&fx.router, ADVERTISE, &[]).await;
    assert_eq!(off.code(), "transport_off");
    let hidden = call(
        &fx.router,
        "/api/v1/repos/alice/app/transport/fetch?kind=archive",
        &[],
    )
    .await;
    assert_eq!(
        hidden.status,
        StatusCode::NOT_FOUND,
        "a stranger learns nothing about the configuration"
    );
}

// ================================================================================================ the fetch matrix

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_fetch_matrix_200_304_206_416_and_the_named_refusals() {
    if skip() {
        return;
    }
    let fx = fixture("matrix", Opts::default());
    // 200 for an archive and a bundle, each verifying with prikk itself.
    let a = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.h("content-type"), "application/octet-stream");
    assert!(
        a.h("content-disposition")
            .starts_with("attachment; filename=\"alice-open.archive\""),
        "{}",
        a.h("content-disposition")
    );
    assert_eq!(a.h("accept-ranges"), "bytes");
    assert_eq!(a.h("content-length"), a.body.len().to_string());
    let etag = a.h("etag").to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'));
    assert!(prikk_verify_file(
        "archive",
        &write_tmp("matrix.archive", &a.body)
    ));
    let b = call(&fx.router, FETCH_BUNDLE, &[]).await;
    assert_eq!(b.status, StatusCode::OK);
    assert!(
        b.h("content-disposition")
            .contains("alice-open-heads_main.bundle"),
        "{}",
        b.h("content-disposition")
    );
    assert!(prikk_verify_file(
        "bundle",
        &write_tmp("matrix.bundle", &b.body)
    ));
    assert_ne!(b.h("etag"), etag);
    // A tag can be bundled too.
    let t = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=bundle&ref=tags/v1",
        &[],
    )
    .await;
    assert_eq!(t.status, StatusCode::OK);

    // 304: the same state, and no export is run for it.
    let exports_before = fx.metrics.snapshot().per_repo["r-open"].exports;
    for inm in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"x\", {etag}"),
        "*".to_owned(),
    ] {
        let r = call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", &inm)]).await;
        assert_eq!(r.status, StatusCode::NOT_MODIFIED, "{inm}");
        assert!(r.body.is_empty());
        assert_eq!(r.h("etag"), etag);
    }
    assert_eq!(
        fx.metrics.snapshot().per_repo["r-open"].exports,
        exports_before,
        "a 304 runs no export"
    );
    // A different validator is a 200.
    let r = call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", "\"other\"")]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, a.body, "the same state serves the same bytes");

    // 206 and 416 within the one current artifact.
    let size = a.body.len();
    let part = call(&fx.router, FETCH_ARCHIVE, &[("range", "bytes=10-99")]).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.h("content-range"), format!("bytes 10-99/{size}"));
    assert_eq!(part.body, a.body[10..100]);
    let open_ended = call(&fx.router, FETCH_ARCHIVE, &[("range", "bytes=100-")]).await;
    assert_eq!(open_ended.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(open_ended.body, a.body[100..]);
    let suffix = call(&fx.router, FETCH_ARCHIVE, &[("range", "bytes=-50")]).await;
    assert_eq!(suffix.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(suffix.body, a.body[size - 50..]);
    let clamped = call(
        &fx.router,
        FETCH_ARCHIVE,
        &[("range", &format!("bytes={}-{}", size - 5, size + 1000))],
    )
    .await;
    assert_eq!(clamped.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(clamped.body, a.body[size - 5..]);
    assert_eq!(
        clamped.h("content-range"),
        format!("bytes {}-{}/{size}", size - 5, size - 1)
    );
    assert_eq!(clamped.h("content-length"), "5");
    assert_eq!(open_ended.h("content-length"), (size - 100).to_string());
    assert_eq!(part.h("content-length"), "90");
    let beyond = call(
        &fx.router,
        FETCH_ARCHIVE,
        &[("range", &format!("bytes={size}-"))],
    )
    .await;
    assert_eq!(beyond.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(beyond.h("content-range"), format!("bytes */{size}"));
    assert_eq!(beyond.code(), "range_not_satisfiable");
    // Several ranges, another unit, a malformed range, a stale If-Range: the whole artifact.
    for hdr in [
        vec![("range", "bytes=0-1,5-6")],
        vec![("range", "items=0-1")],
        vec![("range", "bytes=abc")],
        vec![("range", "bytes=0-9"), ("if-range", "\"stale\"")],
    ] {
        let r = call(&fx.router, FETCH_ARCHIVE, &hdr).await;
        assert_eq!(r.status, StatusCode::OK, "{hdr:?}");
        assert_eq!(r.body, a.body);
    }
    let r = call(
        &fx.router,
        FETCH_ARCHIVE,
        &[("range", "bytes=0-9"), ("if-range", &etag)],
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::PARTIAL_CONTENT,
        "a current If-Range keeps the range"
    );

    // The named refusals.
    let r = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=bundle",
        &[],
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::BAD_REQUEST, "invalid_request")
    );
    assert!(r.json()["hint"].as_str().unwrap().contains("ref=NAME"));
    let r = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=bundle&ref=heads/nope",
        &[],
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "unknown_ref")
    );
    let r = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=zip",
        &[],
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "unknown_kind")
    );
    let r = call(&fx.router, "/api/v1/repos/alice/open/transport/fetch", &[]).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::BAD_REQUEST, "invalid_request")
    );
    // The error shape: one JSON shape, no path.
    let shape = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=zip",
        &[],
    )
    .await;
    assert_eq!(shape.json().as_object().unwrap().len(), 3);
    assert!(!shape.h("content-type").is_empty());
    // The refusals are counted by reason.
    let snap = fx.metrics.snapshot();
    assert!(snap.refusals["unknown_ref"] >= 1 && snap.refusals["unknown_kind"] >= 1);
    assert!(snap.refusals["range_not_satisfiable"] >= 1);
    // A fetch is a GET.
    let post = fx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(FETCH_ARCHIVE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_error_texts_follow_accept_language() {
    if skip() {
        return;
    }
    let fx = fixture("language", Opts::default());
    let en = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=zip",
        &[],
    )
    .await
    .json();
    assert_eq!(en["message"], "That fetch kind is not supported.");
    // Japanese is a draft: not negotiated here (the preview is not on in this process).
    let ja = call(
        &fx.router,
        "/api/v1/repos/alice/open/transport/fetch?kind=zip",
        &[("accept-language", "ja")],
    )
    .await;
    assert_eq!(ja.json()["code"], "unknown_kind");
}

// ================================================================================================ the fingerprint

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_changes_the_etag_reads_do_not_and_an_out_of_band_write_is_seen() {
    if skip() {
        return;
    }
    let fx = fixture("fingerprint", Opts::default());
    let first = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    let etag = first.h("etag").to_owned();
    // Reads of the repository through the API leave the fingerprint alone.
    for uri in [
        "/api/v1/repos/alice/open/tree",
        "/api/v1/repos/alice/open/history",
        "/api/v1/repos/alice/open/verify",
        "/api/v1/repos/alice/open/refs",
        ADVERTISE,
    ] {
        assert_eq!(
            call(&fx.router, uri, &[]).await.status,
            StatusCode::OK,
            "{uri}"
        );
    }
    let same = call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", &etag)]).await;
    assert_eq!(
        same.status,
        StatusCode::NOT_MODIFIED,
        "reads do not change the state"
    );

    // (Simulated) write: the counter is bumped under the W lease.
    let w = fx
        .leases
        .acquire(
            &RepoId::new("r-open"),
            LeaseClass::Write,
            "test write",
            Duration::ZERO,
        )
        .unwrap();
    let revision = fx.export.write_completed(&w).unwrap();
    drop(w);
    assert_eq!(revision, 1);
    assert_eq!(fx.revisions.get(&RepoId::new("r-open")).unwrap(), 1);
    let stale = call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", &etag)]).await;
    assert_eq!(
        stale.status,
        StatusCode::OK,
        "the old validator no longer matches"
    );
    let second_etag = stale.h("etag").to_owned();
    assert_ne!(second_etag, etag);
    assert_eq!(
        fx.metrics.snapshot().per_repo["r-open"].exports,
        2,
        "the next fetch exported again"
    );
    assert_eq!(
        stale.body, first.body,
        "the same bytes (nothing really changed), a different validator"
    );
    assert_eq!(
        call(
            &fx.router,
            FETCH_ARCHIVE,
            &[("if-none-match", &second_etag)]
        )
        .await
        .status,
        StatusCode::NOT_MODIFIED
    );

    // An out-of-band write: a file under `.prikk` changes without planeter knowing.
    std::fs::write(
        fx.open_path.join(".prikk/out-of-band-marker"),
        b"someone else wrote here",
    )
    .unwrap();
    let third = call(
        &fx.router,
        FETCH_ARCHIVE,
        &[("if-none-match", &second_etag)],
    )
    .await;
    assert_eq!(
        third.status,
        StatusCode::OK,
        "an out-of-band write invalidates the cache"
    );
    assert_ne!(third.h("etag"), second_etag);
    // A cache directory is not state.
    let third_etag = third.h("etag").to_owned();
    std::fs::create_dir_all(fx.open_path.join(".prikk/cache")).unwrap();
    std::fs::write(fx.open_path.join(".prikk/cache/scratch.v1"), b"x").unwrap();
    assert_eq!(
        call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", &third_etag)])
            .await
            .status,
        StatusCode::NOT_MODIFIED
    );
}

#[test]
fn the_bump_api_needs_the_w_lease() {
    let base = scratch("bump");
    let spool = Spool::open(base.join("spool"), 1 << 20).unwrap();
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(
        base.join("root"),
        store as Arc<dyn RepositoryStore>,
    ));
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        now,
        Duration::from_secs(1),
    ));
    let export = ExportService::new(
        hosting,
        spool,
        leases.clone(),
        Arc::new(InMemoryRevisions::new()),
        ExportConfig::default(),
        Arc::new(Metrics::new()),
    );
    let repo = RepoId::new("r");
    let x = leases
        .acquire(&repo, LeaseClass::Export, "x", Duration::ZERO)
        .unwrap();
    let attempt =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export.write_completed(&x)));
    assert!(
        attempt.is_err(),
        "bumping under an export lease is a programming error"
    );
    drop(x);
    let w = leases
        .acquire(&repo, LeaseClass::Write, "w", Duration::ZERO)
        .unwrap();
    assert_eq!(export.write_completed(&w).unwrap(), 1);
    assert_eq!(export.write_completed(&w).unwrap(), 2);
}

// ================================================================================================ the serve gate

/// A stand-in prikk: `--version` answers the pin; `archive export` writes `junk`; `archive verify` says it is not
/// ok. (The repository is never opened by it.)
fn stand_in(dir: &Path, body: &str) -> PathBuf {
    let p = dir.join("prikk-stand-in");
    std::fs::write(
        &p,
        format!("#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.52.0';; *) {body};; esac\n"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_that_does_not_verify_is_never_served_and_never_kept() {
    if skip() {
        return;
    }
    let dir = scratch("gate-bin");
    let bin = stand_in(
        &dir,
        r#"if [ "$2" = "export" ]; then echo junk > "$3"; elif [ "$2" = "verify" ]; then echo '{"schema_version":"prepo-verify-report-v1","ok":false,"structure":{"ok":false},"closure":{"ok":true},"damaged":["x"],"interrupted_appends":[]}'; exit 1; fi"#,
    );
    let fx = fixture(
        "gate",
        Opts {
            binary: Some(bin),
            ..Opts::default()
        },
    );
    let r = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.code(), "not_verified");
    assert!(!String::from_utf8_lossy(&r.body).contains("junk"));
    // Nothing is kept: no object, no partial.
    let objects: Vec<_> = walk(&fx.spool_dir.join("objects"));
    assert!(objects.is_empty(), "{objects:?}");
    assert!(walk(&fx.spool_dir.join("incoming")).is_empty());
    assert_eq!(fx.metrics.snapshot().refusals["not_verified"], 1);
    // A second request exports again (a failure is not cached).
    let again = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(again.code(), "not_verified");
    assert_eq!(fx.metrics.snapshot().refusals["not_verified"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_whose_report_says_ok_but_whose_parts_do_not_is_not_served() {
    if skip() {
        return;
    }
    let dir = scratch("gate2-bin");
    let bin = stand_in(
        &dir,
        r#"if [ "$2" = "export" ]; then echo junk > "$3"; elif [ "$2" = "verify" ]; then echo '{"schema_version":"prepo-verify-report-v1","ok":true,"structure":{"ok":true},"closure":{"ok":false},"damaged":[],"interrupted_appends":[]}'; fi"#,
    );
    let fx = fixture(
        "gate2",
        Opts {
            binary: Some(bin),
            ..Opts::default()
        },
    );
    let r = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(
        r.code(),
        "not_verified",
        "a report is trusted only if every part is ok"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_export_and_a_memory_ceiling_are_named_refusals_and_keep_nothing() {
    if skip() {
        return;
    }
    let dir = scratch("fail-bin");
    let failing = stand_in(&dir, r#"echo "no space left" >&2; exit 1"#);
    let fx = fixture(
        "fail",
        Opts {
            binary: Some(failing),
            ..Opts::default()
        },
    );
    let r = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "export_failed")
    );
    assert!(
        !String::from_utf8_lossy(&r.body).contains("no space left"),
        "tool output never reaches the client"
    );
    assert!(walk(&fx.spool_dir.join("incoming")).is_empty());

    let dir2 = scratch("ceiling-bin");
    let ceiling = stand_in(
        &dir2,
        r#"echo "memory allocation of 99 bytes failed" >&2; exit 134"#,
    );
    let fx2 = fixture(
        "ceiling",
        Opts {
            binary: Some(ceiling),
            ..Opts::default()
        },
    );
    if have("prlimit") {
        let r = call(&fx2.router, FETCH_ARCHIVE, &[]).await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, "export_failed")
        );
        assert!(
            r.json()["hint"].as_str().unwrap().contains("memory"),
            "{}",
            r.json()
        );
        assert_eq!(fx2.metrics.snapshot().refusals["export_ceiling"], 1);
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_corrupted_spool_file_is_never_served() {
    if skip() {
        return;
    }
    let fx = fixture("corrupt", Opts::default());
    let first = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(fx.metrics.snapshot().per_repo["r-open"].exports, 1);
    // Flip a byte in the spooled object.
    let objects = walk(&fx.spool_dir.join("objects"));
    assert_eq!(objects.len(), 1);
    let mut bytes = std::fs::read(&objects[0]).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&objects[0], &bytes).unwrap();
    let second = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        second.body, first.body,
        "the served bytes are the exported ones, not the damaged file"
    );
    assert_eq!(
        fx.metrics.snapshot().per_repo["r-open"].exports,
        2,
        "the damaged object was discarded and exported again"
    );
    assert!(prikk_verify_file(
        "archive",
        &write_tmp("corrupt.archive", &second.body)
    ));
    // A truncated object is the same.
    let objects = walk(&fx.spool_dir.join("objects"));
    let mut bytes = std::fs::read(&objects[0]).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&objects[0], &bytes).unwrap();
    let third = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(third.body, first.body);
    assert_eq!(fx.metrics.snapshot().per_repo["r-open"].exports, 3);
}

// ================================================================================================ single flight etc.

/// A wrapper around the real prikk: counts `archive export` calls in `counter` and sleeps `secs` first.
fn slow_prikk(dir: &Path, secs: u32) -> (PathBuf, PathBuf) {
    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v prikk"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let counter = dir.join("exports.count");
    let p = dir.join("prikk-slow");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nif [ \"$1\" = archive ] && [ \"$2\" = export ]; then echo x >> {}; sleep {secs}; fi\nexec {} \"$@\"\n",
            counter.display(),
            real.trim()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    (p, counter)
}

fn count_lines(p: &Path) -> usize {
    std::fs::read_to_string(p)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn n_concurrent_requests_for_one_state_run_one_export() {
    if skip() {
        return;
    }
    let dir = scratch("flight-bin");
    let (bin, counter) = slow_prikk(&dir, 1);
    // One export slot and a lease wait far shorter than the export: only requests that share the leader's export
    // (rather than queue for a slot of their own) can all succeed.
    let cfg = ExportConfig {
        max_concurrent: 1,
        lease_wait: Duration::from_millis(200),
        ..ExportConfig::default()
    };
    let fx = fixture(
        "flight",
        Opts {
            binary: Some(bin),
            config: cfg,
            ..Opts::default()
        },
    );
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let r = fx.router.clone();
            tokio::spawn(async move { call(&r, FETCH_ARCHIVE, &[]).await })
        })
        .collect();
    let replies: Vec<Reply> = futures_join(handles).await;
    for r in &replies {
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body, replies[0].body);
        assert_eq!(r.h("etag"), replies[0].h("etag"));
    }
    assert_eq!(count_lines(&counter), 1, "eight requests, one prikk export");
    let snap = fx.metrics.snapshot();
    assert_eq!(snap.per_repo["r-open"].exports, 1);
    assert_eq!(
        snap.per_repo["r-open"].cache_misses, 8,
        "the leader and the seven that shared its export"
    );
    assert_eq!(snap.per_repo["r-open"].cache_hits, 0);
    // The ninth is a hit.
    let hit = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(hit.status, StatusCode::OK);
    assert_eq!(count_lines(&counter), 1);
    assert_eq!(fx.metrics.snapshot().per_repo["r-open"].cache_hits, 1);
    assert!(fx.metrics.snapshot().per_repo["r-open"].hit_rate() > 0.0);
}

async fn futures_join<T: Send + 'static>(handles: Vec<tokio::task::JoinHandle<T>>) -> Vec<T> {
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await.unwrap());
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_exports_are_bounded_and_the_overflow_is_busy_with_a_retry_hint() {
    if skip() {
        return;
    }
    let dir = scratch("slots-bin");
    let (bin, _) = slow_prikk(&dir, 3);
    let cfg = ExportConfig {
        max_concurrent: 2,
        lease_wait: Duration::from_secs(1),
        ..ExportConfig::default()
    };
    let fx = fixture(
        "slots",
        Opts {
            binary: Some(bin),
            config: cfg,
            ..Opts::default()
        },
    );
    // Three different repositories at once: two exports run, the third waits a second for a slot and is refused.
    let uris = [
        "/api/v1/repos/alice/open/transport/fetch?kind=archive",
        "/api/v1/repos/alice/recv/transport/fetch?kind=archive",
        "/api/v1/repos/alice/app/transport/fetch?kind=archive",
    ];
    let handles: Vec<_> = uris
        .iter()
        .map(|u| {
            let (r, u) = (fx.router.clone(), u.to_string());
            tokio::spawn(async move { call(&r, &u, &[TOKEN]).await })
        })
        .collect();
    let replies = futures_join(handles).await;
    let ok = replies
        .iter()
        .filter(|r| r.status == StatusCode::OK)
        .count();
    let busy: Vec<&Reply> = replies
        .iter()
        .filter(|r| r.status == StatusCode::SERVICE_UNAVAILABLE)
        .collect();
    assert_eq!(
        (ok, busy.len()),
        (2, 1),
        "{:?}",
        replies.iter().map(|r| r.status).collect::<Vec<_>>()
    );
    assert_eq!(busy[0].code(), "busy");
    assert!(
        busy[0].h("retry-after").parse::<u64>().unwrap() >= 1,
        "a retry hint"
    );
    assert_eq!(fx.metrics.snapshot().refusals["busy"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_writer_holding_the_repository_makes_a_fetch_busy_and_readers_do_not() {
    if skip() {
        return;
    }
    let cfg = ExportConfig {
        lease_wait: Duration::from_millis(600),
        ..ExportConfig::default()
    };
    let fx = fixture(
        "w-lease",
        Opts {
            config: cfg,
            ..Opts::default()
        },
    );
    let w = fx
        .leases
        .acquire(
            &RepoId::new("r-open"),
            LeaseClass::Write,
            "sync accept",
            Duration::ZERO,
        )
        .unwrap();
    let t = Instant::now();
    let r = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.code(), "busy");
    assert!(r.h("retry-after").parse::<u64>().is_ok());
    assert!(
        t.elapsed() >= Duration::from_millis(500),
        "the wait is bounded, not skipped: {:?}",
        t.elapsed()
    );
    // A 304 and the advertisement need no export and do not wait for the writer.
    let adv = call(&fx.router, ADVERTISE, &[]).await;
    assert_eq!(adv.status, StatusCode::OK);
    let etag = adv.json()["archive"]["etag"].as_str().unwrap().to_owned();
    let t = Instant::now();
    let not_modified = call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", &etag)]).await;
    assert_eq!(not_modified.status, StatusCode::NOT_MODIFIED);
    assert!(t.elapsed() < Duration::from_millis(500));
    drop(w);
    // With the writer gone the fetch works; a reader lease never blocks an export.
    let _reader = fx
        .leases
        .acquire(
            &RepoId::new("r-open"),
            LeaseClass::Read,
            "read",
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        call(&fx.router, FETCH_ARCHIVE, &[]).await.status,
        StatusCode::OK
    );
}

// ================================================================================================ T4 reads stay unblocked

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn reads_succeed_while_an_export_runs_and_a_slow_client_holds_no_lease() {
    if skip() {
        return;
    }
    let dir = scratch("reads-bin");
    let (bin, counter) = slow_prikk(&dir, 2);
    let fx = fixture(
        "reads",
        Opts {
            binary: Some(bin),
            ..Opts::default()
        },
    );
    let export = {
        let r = fx.router.clone();
        tokio::spawn(async move { call(&r, FETCH_ARCHIVE, &[]).await })
    };
    // Wait until the export is really running.
    let end = Instant::now() + Duration::from_secs(10);
    while count_lines(&counter) == 0 && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(count_lines(&counter), 1);
    assert_eq!(
        fx.leases.holder_of(&RepoId::new("r-open")),
        Some(LeaseClass::Export),
        "the export holds X while it runs"
    );
    // The A10-style loop: reads (tree, log, verify, refs) keep succeeding for as long as the export runs.
    let mut calls = 0;
    let mut failed = Vec::new();
    while !export.is_finished() {
        for uri in [
            "/api/v1/repos/alice/open/tree",
            "/api/v1/repos/alice/open/history",
            "/api/v1/repos/alice/open/verify",
            "/api/v1/repos/alice/open/refs",
        ] {
            let r = call(&fx.router, uri, &[]).await;
            calls += 1;
            if r.status != StatusCode::OK {
                failed.push((uri, r.status));
            }
        }
    }
    assert!(calls >= 8, "the loop ran {calls} calls");
    assert!(
        failed.is_empty(),
        "reads failed during an export: {failed:?}"
    );
    let served = export.await.unwrap();
    assert_eq!(served.status, StatusCode::OK);

    // A client throttled to 1 KiB/s mid-download does not hold the X lease: it is free while the bytes drain.
    let resp = fx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(FETCH_ARCHIVE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body().into_data_stream();
    use futures_stream::StreamExt as _;
    let started = Instant::now();
    let mut got = 0usize;
    while started.elapsed() < Duration::from_secs(3) {
        let Some(chunk) = stream.next().await else {
            break;
        };
        got += chunk.unwrap().len().min(1024);
        // Mid-download: no lease is held by this transfer, and another writer can take W at once.
        assert_eq!(
            fx.leases.holder_of(&RepoId::new("r-open")),
            None,
            "a slow client holds no lock"
        );
        let w = fx
            .leases
            .acquire(
                &RepoId::new("r-open"),
                LeaseClass::Write,
                "w",
                Duration::ZERO,
            )
            .expect("W is free while a client drains");
        drop(w);
        tokio::time::sleep(Duration::from_secs(1)).await; // ~1 KiB/s
    }
    assert!(got > 0);
    drop(stream);
}

/// `StreamExt::next` without a new dependency: a tiny poll_fn over `Stream`.
mod futures_stream {
    use std::pin::Pin;
    use std::task::{Context, Poll};
    pub trait StreamExt: Sized {
        fn next(&mut self) -> Next<'_, Self>
        where
            Self: Unpin;
    }
    impl<S: futures_core::Stream + Unpin> StreamExt for S {
        fn next(&mut self) -> Next<'_, Self> {
            Next(self)
        }
    }
    pub struct Next<'a, S>(&'a mut S);
    impl<S: futures_core::Stream + Unpin> std::future::Future for Next<'_, S> {
        type Output = Option<S::Item>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut *self.0).poll_next(cx)
        }
    }
    #[allow(dead_code)]
    pub fn unused(_: Poll<()>) {}
}

// ================================================================================================ T6 edge limits

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_rate_limits_answer_429_with_retry_after_and_say_nothing_about_any_repository() {
    if skip() {
        return;
    }
    let _turn = CLOCK.lock().await;
    NOW.store(2_000_000, Ordering::Relaxed);
    let fx = fixture(
        "rate",
        Opts {
            ip_limit: 4,
            ..Opts::default()
        },
    );
    // Four requests are within the limit, whichever repository they name (a hidden one included).
    for uri in [
        "/api/v1/repos/alice/app/transport/advertise",
        "/api/v1/repos/alice/ghost/transport/advertise",
        ADVERTISE,
        FETCH_ARCHIVE,
    ] {
        let r = call(&fx.router, uri, &[]).await;
        assert_ne!(r.status, StatusCode::TOO_MANY_REQUESTS, "{uri}");
    }
    // The fifth is refused: the same answer for a hidden, an absent and a public repository.
    let mut answers = Vec::new();
    for uri in [
        "/api/v1/repos/alice/app/transport/advertise",
        "/api/v1/repos/alice/ghost/transport/fetch?kind=archive",
        FETCH_ARCHIVE,
        ADVERTISE,
    ] {
        let r = call(&fx.router, uri, &[]).await;
        assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{uri}");
        assert_eq!(r.code(), "rate_limited");
        let after: u64 = r.h("retry-after").parse().unwrap();
        assert!((1..=60).contains(&after), "{after}");
        answers.push((r.body.clone(), r.h("retry-after").to_owned()));
    }
    assert!(
        answers.windows(2).all(|w| w[0] == w[1]),
        "the answers are identical whatever the repository"
    );
    assert!(fx.metrics.snapshot().refusals["rate_limited"] >= 4);
    // A minute later the window has passed.
    NOW.store(2_000_061, Ordering::Relaxed);
    assert_ne!(
        call(&fx.router, ADVERTISE, &[]).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    NOW.store(1_000_000, Ordering::Relaxed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_per_user_limit_counts_a_signed_in_caller_wherever_they_come_from() {
    if skip() {
        return;
    }
    let _turn = CLOCK.lock().await;
    NOW.store(3_000_000, Ordering::Relaxed);
    let fx = fixture(
        "rate-user",
        Opts {
            principal_limit: 2,
            ..Opts::default()
        },
    );
    for _ in 0..2 {
        assert_eq!(
            call(&fx.router, ADVERTISE, &[TOKEN]).await.status,
            StatusCode::OK
        );
    }
    let r = call(&fx.router, ADVERTISE, &[TOKEN]).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // An anonymous request is not counted against the user.
    assert_eq!(
        call(&fx.router, ADVERTISE, &[]).await.status,
        StatusCode::OK
    );
    NOW.store(1_000_000, Ordering::Relaxed);
}

#[test]
fn the_limiter_is_a_sliding_window_and_bounded() {
    let l = RateLimiter::new(3, Duration::from_secs(60));
    for t in [100, 101, 102] {
        assert_eq!(l.check("k", t), Ok(()));
    }
    assert_eq!(
        l.check("k", 103),
        Err(57),
        "retry when the oldest request leaves the window"
    );
    assert_eq!(
        l.check("other", 103),
        Ok(()),
        "another key has its own window"
    );
    assert_eq!(l.check("k", 160), Ok(()), "the first has expired");
    assert_eq!(l.check("k", 161), Ok(()));
    assert_eq!(l.check("k", 162), Ok(()));
    assert_eq!(l.check("k", 163), Err(57), "160 + 60 - 163");
    // A refused request is not counted: it does not push the window out.
    let l = RateLimiter::new(1, Duration::from_secs(10));
    assert_eq!(l.check("k", 0), Ok(()));
    for t in 1..9 {
        assert!(l.check("k", t).is_err());
    }
    assert_eq!(l.check("k", 10), Ok(()));
}

// ================================================================================================ T5 instrumentation

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_counters_say_changed_unchanged_kind_bytes_cache_and_wait() {
    if skip() {
        return;
    }
    let fx = fixture("metrics", Opts::default());
    let a = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    let etag = a.h("etag").to_owned();
    let _ = call(&fx.router, FETCH_ARCHIVE, &[]).await; // a hit
    let _ = call(&fx.router, FETCH_ARCHIVE, &[("if-none-match", &etag)]).await; // 304
    let b = call(&fx.router, FETCH_BUNDLE, &[]).await;
    let s = fx.metrics.snapshot();
    assert_eq!(s.fetches_changed, 3);
    assert_eq!(s.fetches_unchanged, 1);
    assert_eq!(s.by_kind["archive"], 3);
    assert_eq!(s.by_kind["bundle"], 1);
    assert_eq!(s.bytes_served, (2 * a.body.len() + b.body.len()) as u64);
    let repo = &s.per_repo["r-open"];
    assert_eq!(repo.exports, 2);
    assert_eq!((repo.cache_hits, repo.cache_misses), (1, 2));
    assert!((repo.hit_rate() - 1.0 / 3.0).abs() < 1e-9);
    assert!(s.wall_total > Duration::ZERO);
}

// ================================================================================================ the real sandbox

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_the_real_sandbox_the_export_writes_into_the_spool_mount_and_nowhere_else() {
    if skip() {
        return;
    }
    if !bwrap_works() || !have("prlimit") {
        eprintln!("skipping: needs bubblewrap (with user namespaces) and prlimit");
        return;
    }
    // The spool and the repositories live under one scratch base; the sandbox mounts the spool read-write and
    // each repository read-write for its own calls only.
    let fx = fixture(
        "bwrap",
        Opts {
            sandbox: planeter_prikk::Sandbox::default(),
            ..Opts::default()
        },
    );
    let a = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&a.body)
    );
    assert!(prikk_verify_file(
        "archive",
        &write_tmp("bwrap.archive", &a.body)
    ));
    let b = call(&fx.router, FETCH_BUNDLE, &[]).await;
    assert_eq!(b.status, StatusCode::OK);
    assert!(walk(&fx.spool_dir.join("incoming")).is_empty());
    // A deadline is a named refusal and keeps nothing: a stand-in that sleeps past a one-second deadline.
    let dir = scratch("bwrap-slow-bin");
    // (A shell script and `sleep` are all the sandbox's read-only /usr offers: no wrapper around the real prikk.)
    let slow = stand_in(&dir, r#"if [ "$2" = "export" ]; then sleep 5; fi"#);
    let cfg = ExportConfig {
        deadline: Duration::from_secs(1),
        ..ExportConfig::default()
    };
    let fx2 = fixture(
        "bwrap-deadline",
        Opts {
            sandbox: planeter_prikk::Sandbox::default(),
            binary: Some(slow),
            config: cfg,
            ..Opts::default()
        },
    );
    let t = Instant::now();
    let r = call(&fx2.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "export_failed"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert!(
        r.json()["hint"].as_str().unwrap().contains("deadline"),
        "{}",
        r.json()
    );
    assert!(
        t.elapsed() < Duration::from_secs(4),
        "the deadline bounded the call: {:?}",
        t.elapsed()
    );
    assert_eq!(fx2.metrics.snapshot().refusals["export_deadline"], 1);
    assert!(walk(&fx2.spool_dir.join("incoming")).is_empty());
    assert!(walk(&fx2.spool_dir.join("objects")).is_empty());
}

// ================================================================================================ the OpenAPI drift guard

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_advertisement_serializes_to_exactly_the_fields_the_document_declares() {
    if skip() {
        return;
    }
    let fx = fixture("openapi", Opts::default());
    let doc = call(&fx.router, "/api/v1/openapi.json", &[]).await.json();
    let props = |schema: &str| -> BTreeSet<String> {
        doc["components"]["schemas"][schema]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{schema} is not in the document"))
            .keys()
            .cloned()
            .collect()
    };
    let keys = |v: &serde_json::Value| -> BTreeSet<String> {
        v.as_object().unwrap().keys().cloned().collect()
    };
    let j = call(&fx.router, ADVERTISE, &[]).await.json();
    assert_eq!(keys(&j), props("Advertisement"));
    assert_eq!(keys(&j["archive"]), props("Offer"));
    let main = j["refs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "heads/main")
        .unwrap();
    assert_eq!(keys(main), props("AdvertisedRef"));
    assert_eq!(keys(&main["fetch"][0]), props("Offer"));
    // The two operations and their error codes are documented.
    let fetch = &doc["paths"]["/api/v1/repos/{owner}/{name}/transport/fetch"]["get"];
    for status in ["200", "206", "304", "416", "422", "429", "503"] {
        assert!(
            fetch["responses"][status].is_object(),
            "fetch documents {status}"
        );
    }
    let codes = doc["components"]["schemas"]["Error"]["properties"]["code"]["enum"]
        .as_array()
        .unwrap();
    for c in [
        "transport_off",
        "unknown_ref",
        "unknown_kind",
        "bundle_of_received_ref",
        "not_verified",
        "export_failed",
        "rate_limited",
        "range_not_satisfiable",
    ] {
        assert!(
            codes.iter().any(|x| x == c),
            "{c} is an error code of the document"
        );
    }
    assert!(
        doc["paths"]["/api/v1/repos/{owner}/{name}/transport/advertise"]["get"]["responses"]["429"]
            .is_object()
    );
}

// ================================================================================================ T0(b): a narrower mount

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_the_real_sandbox_an_export_sees_its_own_working_directory_and_no_other_spool_object()
{
    if skip() {
        return;
    }
    if !bwrap_works() || !have("prlimit") {
        eprintln!("skipping: needs bubblewrap (with user namespaces) and prlimit");
        return;
    }
    // A stand-in prikk that, as `archive export`, writes into the file it was given a report of what it can do
    // to the rest of the spool; and whose `archive verify` accepts anything (the report is the "archive").
    let dir = scratch("isolation-bin");
    let bin = stand_in(
        &dir,
        r#"if [ "$2" = "export" ]; then
  out="$3"; work=$(dirname "$out"); spool=$(dirname "$(dirname "$work")")
  {
    echo "own-dir-write: $(echo ok > "$work/note" 2>/dev/null && echo ok || echo fail)"
    echo "objects-listing: $(ls "$spool/objects" 2>/dev/null | wc -l)"
    echo "objects-read: $(cat "$spool"/objects/*/* 2>/dev/null | wc -c)"
    echo "objects-write: $(echo x > "$spool/objects/planted" 2>/dev/null && echo ok || echo fail)"
    echo "spool-write: $(echo x > "$spool/planted" 2>/dev/null && echo ok || echo fail)"
    echo "incoming-write: $(echo x > "$spool/incoming/planted.partial" 2>/dev/null && echo ok || echo fail)"
    echo "sibling-work-dirs: $(ls "$spool/work" 2>/dev/null | wc -l)"
    echo "sibling-read: $(cat "$spool"/work/*/secret 2>/dev/null | wc -c)"
    echo "sibling-write: $(echo x > "$spool"/work/other-export/planted 2>/dev/null && echo ok || echo fail)"
  } > "$out"
elif [ "$2" = "verify" ]; then
  echo '{"schema_version":"prepo-verify-report-v1","ok":true,"structure":{"ok":true},"closure":{"ok":true},"damaged":[],"interrupted_appends":[]}'
fi"#,
    );
    let fx = fixture(
        "isolation",
        Opts {
            binary: Some(bin),
            sandbox: planeter_prikk::Sandbox::default(),
            ..Opts::default()
        },
    );
    // A stored object and another export's working directory, both of which the stand-in must not reach.
    let victim = fx
        .export
        .spool()
        .put(&mut std::io::Cursor::new(
            b"another artifact, not for this export".to_vec(),
        ))
        .unwrap();
    let other = fx.spool_dir.join("work/other-export");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("secret"), b"another export's file").unwrap();

    let r = call(&fx.router, FETCH_ARCHIVE, &[]).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let report = String::from_utf8_lossy(&r.body).into_owned();
    let line = |k: &str| {
        report
            .lines()
            .find(|l| l.starts_with(k))
            .unwrap_or_else(|| panic!("{k}: {report}"))
            .to_owned()
    };
    assert_eq!(
        line("own-dir-write"),
        "own-dir-write: ok",
        "it can write its own working directory\n{report}"
    );
    assert_eq!(
        line("objects-listing"),
        "objects-listing: 0",
        "stored objects are not even listed\n{report}"
    );
    assert_eq!(line("objects-read"), "objects-read: 0", "{report}");
    assert_eq!(line("objects-write"), "objects-write: fail", "{report}");
    // (A write under a directory that bubblewrap only made to hold the mount lands in the sandbox's private
    // in-memory root and vanishes with it: only the host check below says anything about it.)
    assert_eq!(
        line("sibling-work-dirs"),
        "sibling-work-dirs: 1",
        "it sees only its own working directory\n{report}"
    );
    assert_eq!(line("incoming-write"), "incoming-write: fail", "{report}");
    assert_eq!(line("sibling-read"), "sibling-read: 0", "{report}");
    assert_eq!(line("sibling-write"), "sibling-write: fail", "{report}");
    // And on the host nothing was planted or disturbed.
    assert!(!fx.spool_dir.join("planted").exists());
    assert!(!fx.spool_dir.join("objects/planted").exists());
    assert!(!fx.spool_dir.join("incoming/planted.partial").exists());
    assert!(!other.join("planted").exists());
    assert_eq!(
        std::fs::read(other.join("secret")).unwrap(),
        b"another export's file"
    );
    assert!(
        fx.export.spool().verify_object(&victim),
        "the stored object is intact"
    );
    // The export's own directory is gone once it is done; the other export's directory is still there.
    let left: Vec<_> = std::fs::read_dir(fx.spool_dir.join("work"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        left,
        vec![std::ffi::OsString::from("other-export")],
        "only the directory that was not ours remains: {left:?}"
    );
}
