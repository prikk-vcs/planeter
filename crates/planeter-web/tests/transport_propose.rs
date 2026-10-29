//! RFC 004 part 3b: propose, the write side of the transport, over HTTP with **real prikk** (the tests self-skip,
//! printing why, without it). The router is driven directly; the accept runs prikk for real (unconfined, except the
//! tests that need the sandbox's deadline and incoming bounds, which need bubblewrap and say so).
//!
//! The keyed "client" repository builds artifacts with `sync build` against the forge's `sync have`, exactly as a
//! client does; the forge is a keyless repository that adopted the client's key by an archive import.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{
    HostingService, LeaseClass, LeaseManager, NullAuditSink, ReadService, RepoScope, Scope,
    ScopeAccess, new_boot_id,
};
use planeter_prikk::PrikkRepo;
use planeter_store::{
    ChangeState, ChangeStore, InMemoryChangeStore, InMemoryLeaseRecords, InMemoryMembershipStore,
    InMemoryRecoveries, InMemoryRepositoryStore, InMemoryRevisions, MembershipStore, Owner,
    ProposalPolicy, RecoveryStore, RepoId, RepositoryRecord, RepositoryStore, RevisionStore, Role,
    Subject, UserId, Visibility,
};
use planeter_transport::{
    ArtifactStore, ExportConfig, ExportService, Metrics, ProposalConfig, ProposalService,
    ProposeConfig, ProposeService, RecoveryConfig, RecoveryService, Spool,
};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};

static NOW: AtomicU64 = AtomicU64::new(1_000_000);
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}

fn runs(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn skip() -> bool {
    if !runs("prikk") {
        eprintln!("skipping: needs prikk on PATH");
        return true;
    }
    false
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
        && runs("prlimit")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("propose-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn prikk_in(dir: &Path, keys: &Path, args: &[&str]) -> std::process::Output {
    Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .unwrap()
}

fn prikk(dir: &Path, keys: &Path, args: &[&str]) -> String {
    let out = prikk_in(dir, keys, args);
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ------------------------------------------------------------------------------------------------ the fixtures

/// The keyed client: a repository with one sealed commit, its key directory, and its archive.
struct Client {
    repo: PathBuf,
    keys: PathBuf,
    archive: PathBuf,
    counter: u32,
}

impl Client {
    fn new(base: &Path) -> Client {
        let repo = base.join("client");
        let keys = base.join("client-keys");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        prikk(&repo, &keys, &["setup"]);
        std::fs::write(repo.join("README.md"), b"# Hello\n\nA readme.\n").unwrap();
        prikk(&repo, &keys, &["commit", "--from-worktree", "-m", "init"]);
        prikk(&repo, &keys, &["seal", "--allow-no-audit"]);
        let archive = base.join("client.archive");
        prikk(
            &repo,
            &keys,
            &["archive", "export", archive.to_str().unwrap()],
        );
        Client {
            repo,
            keys,
            archive,
            counter: 0,
        }
    }

    /// Commit and seal `n` new files in the client.
    fn commit(&mut self, n: u32) {
        for _ in 0..n {
            self.counter += 1;
            let c = self.counter;
            std::fs::write(
                self.repo.join(format!("file-{c}.txt")),
                format!("content {c}\n").repeat(20),
            )
            .unwrap();
            prikk(
                &self.repo,
                &self.keys,
                &["commit", "--from-worktree", "-m", &format!("change {c}")],
            );
            prikk(&self.repo, &self.keys, &["seal", "--allow-no-audit"]);
        }
    }

    /// Commit and seal one file of `bytes` bytes of incompressible content (a xorshift stream: the content is not what
    /// is being tested, its size is). One patch, one blob.
    fn commit_blob(&mut self, bytes: usize) {
        self.counter += 1;
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15 ^ u64::from(self.counter);
        let mut data = vec![0u8; bytes];
        for chunk in data.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let b = x.to_le_bytes();
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
        std::fs::write(self.repo.join(format!("blob-{}.bin", self.counter)), &data).unwrap();
        prikk(
            &self.repo,
            &self.keys,
            &["commit", "--from-worktree", "-m", "one big blob"],
        );
        prikk(&self.repo, &self.keys, &["seal", "--allow-no-audit"]);
    }

    /// The artifact that closes the gap between this client and `forge` (what `sync build` makes against the
    /// forge's `sync have`).
    fn artifact(&self, forge: &Path, name: &str) -> Vec<u8> {
        let have = self.repo.join(format!("{name}.have"));
        let art = self.repo.join(format!("{name}.art"));
        let _ = std::fs::remove_file(&have);
        let _ = std::fs::remove_file(&art);
        prikk(
            forge,
            &self.keys,
            &[
                "sync",
                "have",
                "heads/main",
                "--output",
                have.to_str().unwrap(),
            ],
        );
        prikk(
            &self.repo,
            &self.keys,
            &[
                "sync",
                "build",
                "heads/main",
                "--have",
                have.to_str().unwrap(),
                "--output",
                art.to_str().unwrap(),
            ],
        );
        std::fs::read(&art).unwrap()
    }

    /// The patch ids of the client's blocks since `n` blocks ago (from its own `log`), as a declaration.
    fn patch_ids(&self, last: usize) -> Vec<String> {
        let out = prikk(
            &self.repo,
            &self.keys,
            &["log", "--format", "json", "--limit", &last.to_string()],
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|b| b["patch_messages"].as_array().unwrap().iter())
            .map(|p| p["patch_id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[allow(dead_code)]
    fn tip(&self) -> String {
        let out = prikk(
            &self.repo,
            &self.keys,
            &["log", "--format", "json", "--limit", "1"],
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["blocks"][0]["block_id"].as_str().unwrap().to_owned()
    }
}

/// A keyless forge repository that adopted the client's maintainer key and imported its archive.
fn forge(base: &Path, name: &str, client: &Client) -> PathBuf {
    let dir = base.join(name);
    let keys = base.join(format!("{name}-keys"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    prikk(&dir, &keys, &["init", "."]);
    let verify = prikk(
        &dir,
        &keys,
        &[
            "archive",
            "verify",
            client.archive.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&verify).unwrap();
    let key = &v["maintainer_keys"][0];
    prikk(
        &dir,
        &keys,
        &[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            key["key_id"].as_str().unwrap(),
            "--public-key",
            key["public_key"].as_str().unwrap(),
        ],
    );
    prikk(
        &dir,
        &keys,
        &[
            "archive",
            "import",
            client.archive.to_str().unwrap(),
            "--adopt",
        ],
    );
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    let st = Command::new("cp")
        .arg("-a")
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(st.success());
}

struct Opts {
    config: ProposeConfig,
    sandbox: planeter_prikk::Sandbox,
    binary: Option<PathBuf>,
    /// The forge repositories are real `prikk` repositories; a test that replaces the binary with a stand-in may
    /// still need the real one for its own reads.
    ip_limit: u32,
    /// The operator configured the export spool but not the artifact store: fetches work, proposals say so.
    without_propose: bool,
    /// Recovery tries by itself right after an accept that did not finish (as the server's does).
    auto_recovery: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            config: ProposeConfig {
                lease_wait: Duration::from_secs(2),
                accept_deadline: Duration::from_secs(60),
                ..ProposeConfig::default()
            },
            sandbox: planeter_prikk::Sandbox::Unconfined,
            binary: None,
            ip_limit: 10_000,
            without_propose: false,
            auto_recovery: false,
        }
    }
}

struct World {
    router: Router,
    base: PathBuf,
    client: Client,
    /// `alice/forge` (public): the repository most tests propose to.
    forge: PathBuf,
    changes: Arc<InMemoryChangeStore>,
    export: Arc<ExportService>,
    leases: Arc<LeaseManager>,
    artifacts_dir: PathBuf,
    metrics: Arc<Metrics>,
    revisions: Arc<InMemoryRevisions>,
    hosting: Arc<HostingService>,
    store: Arc<InMemoryRepositoryStore>,
    sessions: Arc<planeter_auth::InMemorySessionStore>,
    recovery: Arc<RecoveryService>,
    recoveries: Arc<InMemoryRecoveries>,
    artifacts: Arc<ArtifactStore>,
}

fn world(name: &str, o: Opts) -> World {
    let base = scratch(name);
    let mut client = Client::new(&base);
    let forge_path = forge(&base, "forge", &client);
    let private_path = base.join("private");
    copy_dir(&forge_path, &private_path);
    let anyauth_path = base.join("anyauth");
    copy_dir(&forge_path, &anyauth_path);
    client.commit(0);

    let store = Arc::new(InMemoryRepositoryStore::new());
    for (id, name, vis, proposals, path) in [
        (
            "r-forge",
            "forge",
            Visibility::Public,
            ProposalPolicy::Members,
            &forge_path,
        ),
        (
            "r-private",
            "private",
            Visibility::Private,
            ProposalPolicy::Members,
            &private_path,
        ),
        (
            "r-anyauth",
            "anyauth",
            Visibility::Public,
            ProposalPolicy::AnyAuthenticated,
            &anyauth_path,
        ),
    ] {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(id),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility: vis,
                created_at: 0,
                prikk_format_version: None,
                proposals,
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
    let membership = Arc::new(InMemoryMembershipStore::new());
    // bob may read (not propose to) the forge; dave may write to it; carol and eve hold no role.
    membership
        .grant_repo_role(
            Subject::User(UserId::new("bob")),
            RepoId::new("r-forge"),
            Role::Read,
        )
        .unwrap();
    membership
        .grant_repo_role(
            Subject::User(UserId::new("dave")),
            RepoId::new("r-forge"),
            Role::Write,
        )
        .unwrap();
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        membership.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let accounts = Arc::new(InMemoryAccountStore::new());
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user, access) in [
        ("tok-alice", "alice", ScopeAccess::Write),
        ("tok-alice-ro", "alice", ScopeAccess::ReadOnly),
        ("tok-bob", "bob", ScopeAccess::Write),
        ("tok-carol", "carol", ScopeAccess::Write),
        ("tok-carol-ro", "carol", ScopeAccess::ReadOnly),
        ("tok-dave", "dave", ScopeAccess::Write),
    ] {
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&*hasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let auth = Arc::new(Authenticator::new(accounts, creds, hasher.clone(), hasher));

    let spool = Spool::open(base.join("spool"), 512 << 20).unwrap();
    let artifacts_dir = base.join("artifacts");
    let artifacts = Arc::new(ArtifactStore::open(&artifacts_dir).unwrap());
    let revisions = Arc::new(InMemoryRevisions::new());
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        now,
        Duration::from_secs(3),
    ));
    let metrics = Arc::new(Metrics::new());
    let export = Arc::new(ExportService::new(
        hosting.clone(),
        spool,
        leases.clone(),
        revisions.clone() as Arc<dyn RevisionStore>,
        ExportConfig::default(),
        metrics.clone(),
    ));
    let changes = Arc::new(InMemoryChangeStore::new());
    let recoveries = Arc::new(InMemoryRecoveries::new());
    let recovery = Arc::new(
        RecoveryService::new(
            hosting.clone(),
            leases.clone(),
            changes.clone() as Arc<dyn ChangeStore>,
            recoveries.clone() as Arc<dyn RecoveryStore>,
            export.clone(),
            RecoveryConfig {
                lease_wait: Duration::from_secs(20),
                ..RecoveryConfig::default()
            },
            metrics.clone(),
        )
        .with_clock(now),
    );
    let mut propose = ProposeService::new(
        hosting.clone(),
        artifacts.clone(),
        leases.clone(),
        changes.clone() as Arc<dyn ChangeStore>,
        export.clone(),
        o.config,
        metrics.clone(),
    )
    .with_clock(now);
    if o.auto_recovery {
        propose = propose.with_recovery(recovery.clone());
    }
    let propose = Arc::new(propose);
    let proposals = Arc::new(
        ProposalService::new(
            hosting.clone(),
            changes.clone() as Arc<dyn ChangeStore>,
            export.clone(),
            artifacts.clone(),
            metrics.clone(),
            ProposalConfig::default(),
        )
        .with_clock(now),
    );
    let reviews = Arc::new(
        planeter_transport::ReviewService::new(
            changes.clone() as Arc<dyn ChangeStore>,
            Arc::new(planeter_store::InMemoryReviewStore::new()),
            planeter_transport::ReviewConfig::default(),
        )
        .with_clock(now),
    );
    let transport = Some(Arc::new(TransportRuntime {
        export: export.clone(),
        propose: (!o.without_propose).then(|| propose.clone()),
        recovery: Some(recovery.clone()),
        proposals: Some(proposals.clone()),
        reviews: Some(reviews.clone()),
        ip_limit: RateLimiter::new(o.ip_limit, Duration::from_secs(60)),
        principal_limit: RateLimiter::new(o.ip_limit, Duration::from_secs(60)),
    }));
    let sessions = Arc::new(planeter_auth::InMemorySessionStore::new());
    let router = router(AppState {
        read,
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: sessions.clone(),
        now_unix: now,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport,
        creation: None,
    });
    World {
        router,
        base,
        client,
        forge: forge_path,
        changes,
        export,
        leases,
        artifacts_dir,
        metrics,
        revisions,
        hosting,
        store,
        sessions,
        recovery,
        recoveries,
        artifacts,
    }
}

// ------------------------------------------------------------------------------------------------ the requests

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
    fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or("").to_owned()
    }
    fn h(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

/// The body of a proposal: the magic, the metadata length, the metadata, the artifact.
fn frame(meta: &serde_json::Value, artifact: &[u8]) -> Vec<u8> {
    let m = meta.to_string().into_bytes();
    let mut out = b"PLANETER-PROPOSE-1\n".to_vec();
    out.extend_from_slice(&(m.len() as u64).to_be_bytes());
    out.extend_from_slice(&m);
    out.extend_from_slice(artifact);
    out
}

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let asked = std::env::var("PROPOSALS_TRANSCRIPT")
        .is_ok()
        .then(|| describe_request(&req));
    let resp = r.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = axum::body::to_bytes(resp.into_body(), 1 << 30)
        .await
        .unwrap()
        .to_vec();
    if let Some(asked) = asked {
        record_transcript(&asked, status, &headers, &body);
    }
    Reply {
        status,
        headers,
        body,
    }
}

/// `PROPOSALS_TRANSCRIPT=<file>` appends every request of the proposals tests and its answer (the review request quotes it).
fn describe_request(req: &Request<Body>) -> String {
    let mut line = format!("{} {}", req.method(), req.uri());
    for name in [
        "authorization",
        "range",
        "if-range",
        "if-none-match",
        "cookie",
    ] {
        if let Some(v) = req.headers().get(name).and_then(|v| v.to_str().ok()) {
            let shown = match name {
                "authorization" => v.to_owned(),
                "cookie" => "(a signed-in session)".to_owned(),
                _ => v.to_owned(),
            };
            line += &format!("\n    {name}: {shown}");
        }
    }
    line
}

fn record_transcript(asked: &str, status: StatusCode, headers: &HeaderMap, body: &[u8]) {
    use std::io::Write as _;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Ok(path) = std::env::var("PROPOSALS_TRANSCRIPT") else {
        return;
    };
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    let test = std::thread::current().name().unwrap_or("?").to_owned();
    let mut text = format!("[{test}]\n> {asked}\n< {status}\n");
    for name in [
        "content-type",
        "content-length",
        "content-range",
        "content-disposition",
        "etag",
        "accept-ranges",
        "retry-after",
    ] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            text += &format!("    {name}: {v}\n");
        }
    }
    let printable = std::str::from_utf8(body).ok().filter(|b| {
        headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("application/json") || c.starts_with("text/"))
            || b.is_empty()
    });
    match printable {
        Some(b) if b.starts_with('{') => {
            let shown: String = b.chars().take(1500).collect();
            text += &format!(
                "    {shown}{}\n",
                if shown.len() < b.len() { " …" } else { "" }
            );
        }
        Some("") => {}
        Some(b) => text += &format!("    (a page of {} bytes)\n", b.len()),
        None => text += &format!("    ({} bytes of the artifact)\n", body.len()),
    }
    let _ = writeln!(out, "{text}");
}

fn post_req(
    repo: &str,
    token: Option<&str>,
    body: Vec<u8>,
    extra: &[(&str, &str)],
) -> Request<Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/repos/alice/{repo}/transport/propose"))
        .header("content-type", "application/octet-stream")
        .header("content-length", body.len().to_string());
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    req.body(Body::from(body)).unwrap()
}

async fn propose_as(
    w: &World,
    repo: &str,
    token: &str,
    meta: serde_json::Value,
    art: &[u8],
) -> Reply {
    send(
        &w.router,
        post_req(repo, Some(token), frame(&meta, art), &[]),
    )
    .await
}

fn pending(forge: &Path) -> Vec<String> {
    let driver = planeter_prikk::CliPrikkRepo::new(forge);
    driver.sync_pending_patches().unwrap()
}

fn titled(title: &str) -> serde_json::Value {
    serde_json::json!({"title": title, "client": "tests/1"})
}

// ------------------------------------------------------------------------------------------------ the happy path

#[tokio::test]
async fn a_keyed_clients_artifact_becomes_a_proposal_with_one_revision_and_nothing_is_published() {
    if skip() {
        return;
    }
    let mut w = world("happy", Opts::default());
    let fp_before = w
        .export
        .state(&w.store.get(&RepoId::new("r-forge")).unwrap().unwrap())
        .unwrap();
    w.client.commit(2);
    let art = w.client.artifact(&w.forge, "a1");
    let declared = w.client.patch_ids(2);
    let reply = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({
            "title": "Two small changes",
            "description": "Adds two files.",
            "target_ref": "heads/main",
            "base": w.client.tip_before(2),
            "declared_patches": declared,
            "client": "tests/1"
        }),
        &art,
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let v = reply.json();
    assert_eq!(v["created_proposal"], true);
    assert_eq!(v["already_recorded"], false);
    assert_eq!(v["revision"]["seq"], 1);
    assert_eq!(v["added_count"], 2);
    assert_eq!(v["declared"], true);
    assert_eq!(v["assurance"], "forge-approved-unsealed");
    assert_eq!(v["claims"].as_array().unwrap().len(), 2);
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("no branch has changed")
    );
    let patches = v["patches"].as_array().unwrap();
    assert_eq!(patches.len(), 2);
    assert!(
        patches
            .iter()
            .all(|p| p["assurance"] == "forge-approved-unsealed")
    );
    assert!(reply.h("vary").contains("Accept-Language"));

    // The forge: the patches are pending (held), no branch moved, and the state fingerprint moved.
    let pend = pending(&w.forge);
    assert_eq!(pend.len(), 2);
    for p in patches {
        assert!(pend.contains(&p["patch_id"].as_str().unwrap().to_owned()));
    }
    let driver = planeter_prikk::CliPrikkRepo::new(&w.forge);
    let tip_after = driver.log(Some("heads/main"), Some(1)).unwrap().blocks[0]
        .block_id
        .clone();
    assert_eq!(tip_after, w.client.tip_before(2), "heads/main did not move");
    let fp_after = w
        .export
        .state(&w.store.get(&RepoId::new("r-forge")).unwrap().unwrap())
        .unwrap();
    assert_ne!(
        fp_before, fp_after,
        "the state fingerprint (and so the advertisement's ETag) changed"
    );

    // The records: one proposal, one revision, the intent complete, the artifact retained by digest.
    let change_id = v["proposal_id"].as_str().unwrap();
    let change = w.changes.get_change(change_id).unwrap().unwrap();
    assert_eq!(
        (
            change.state,
            change.opened_by.as_str(),
            change.title.as_str()
        ),
        (ChangeState::Open, "alice", "Two small changes")
    );
    let revs = w.changes.revisions_of(change_id).unwrap();
    assert_eq!(revs.len(), 1);
    assert_eq!(revs[0].added_patch_ids.len(), 2);
    assert_eq!(revs[0].declared_patch_ids.as_ref().unwrap().len(), 2);
    assert_eq!(
        revs[0].declared_base.as_deref(),
        Some(w.client.tip_before(2).as_str())
    );
    assert_eq!(revs[0].author_key_ids.len(), 1);
    assert!(
        revs[0]
            .claim_outcomes
            .iter()
            .all(|c| c.outcome.starts_with("Sound"))
    );
    let digest = revs[0]
        .artifact_digest
        .strip_prefix("sha256:")
        .unwrap()
        .to_owned();
    assert_eq!(
        digest,
        planeter_auth::sha256_hex(&art),
        "the digest is of the artifact bytes"
    );
    assert!(
        w.artifacts_dir
            .join("objects")
            .join(&digest[..2])
            .join(&digest)
            .is_file(),
        "the artifact is retained"
    );
    assert!(
        w.changes
            .open_intents(&RepoId::new("r-forge"))
            .unwrap()
            .is_empty()
    );
    // The working area is gone.
    assert_eq!(
        std::fs::read_dir(w.artifacts_dir.join("work"))
            .unwrap()
            .count(),
        0
    );
    let snap = w.metrics.snapshot();
    assert_eq!(snap.proposals_accepted, 1);
    assert_eq!(snap.propose_attempts_max, 1);
    let _ = (&w.revisions, &w.leases, &w.hosting);
}

impl Client {
    /// The id of the block that was the tip `n` blocks ago (the base an artifact of the last `n` commits was built on).
    fn tip_before(&self, n: usize) -> String {
        let out = prikk(
            &self.repo,
            &self.keys,
            &["log", "--format", "json", "--limit", &(n + 1).to_string()],
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["blocks"][n]["block_id"].as_str().unwrap().to_owned()
    }
}

// ------------------------------------------------------------------------------------------------ revisions and repeats

#[tokio::test]
async fn a_second_revision_to_the_same_proposal_and_a_resend_of_an_identical_artifact() {
    if skip() {
        return;
    }
    let mut w = world("second", Opts::default());
    w.client.commit(1);
    let a1 = w.client.artifact(&w.forge, "a1");
    let first = propose_as(&w, "forge", "tok-alice", titled("One"), &a1).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let change_id = first.json()["proposal_id"].as_str().unwrap().to_owned();
    let rev1 = first.json()["revision"]["id"].as_str().unwrap().to_owned();

    // A second revision: more commits, an artifact built against the forge as it now is (its pending patches are
    // in the artifact again; only the new ones are `added`).
    w.client.commit(2);
    let a2 = w.client.artifact(&w.forge, "a2");
    let declared = w.client.patch_ids(3);
    let second = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"proposal": change_id, "declared_patches": declared}),
        &a2,
    )
    .await;
    assert_eq!(
        second.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&second.body)
    );
    let v = second.json();
    assert_eq!(
        (v["created_proposal"].clone(), v["revision"]["seq"].clone()),
        (false.into(), 2.into())
    );
    assert_eq!(v["added_count"], 2, "only what this push added");
    assert_eq!(v["declared"], true);
    assert_eq!(v["proposal_id"], change_id.as_str());
    let revs = w.changes.revisions_of(&change_id).unwrap();
    assert_eq!(revs.len(), 2);
    assert_eq!(
        revs[1].declared_patch_ids.as_ref().unwrap().len(),
        3,
        "the declaration carries the earlier patch again"
    );
    assert_eq!(pending(&w.forge).len(), 3);

    // A re-send of an identical artifact: the same revision, nothing recorded, nothing accepted again.
    let again = propose_as(&w, "forge", "tok-alice", titled("One"), &a1).await;
    assert_eq!(again.status, StatusCode::OK);
    let v = again.json();
    assert_eq!(v["already_recorded"], true);
    assert_eq!(v["revision"]["id"], rev1.as_str());
    assert_eq!(w.changes.revisions_of(&change_id).unwrap().len(), 2);
    assert_eq!(
        w.changes
            .list_changes(&RepoId::new("r-forge"), None)
            .unwrap()
            .len(),
        1
    );
    assert!(v["message"].as_str().unwrap().contains("already recorded"));
    assert_eq!(w.metrics.snapshot().proposals_already_recorded, 1);
}

#[tokio::test]
async fn an_artifact_that_adds_nothing_records_nothing_and_says_so() {
    if skip() {
        return;
    }
    let mut w = world("nothing", Opts::default());
    // Two artifacts: the first holds one patch, the second (built after one more commit) holds both.
    w.client.commit(1);
    let small = w.client.artifact(&w.forge, "small");
    w.client.commit(1);
    let big = w.client.artifact(&w.forge, "big");
    assert_eq!(
        propose_as(&w, "forge", "tok-alice", titled("Both"), &big)
            .await
            .status,
        StatusCode::CREATED
    );
    // The smaller artifact (a different digest) carries only a patch the forge already holds: nothing is added.
    let r = propose_as(&w, "forge", "tok-alice", titled("One"), &small).await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.code(), "no_new_content");
    assert_eq!(
        w.changes
            .list_changes(&RepoId::new("r-forge"), None)
            .unwrap()
            .len(),
        1
    );
    assert!(
        w.changes
            .open_intents(&RepoId::new("r-forge"))
            .unwrap()
            .is_empty(),
        "the intent is closed"
    );
    assert_eq!(pending(&w.forge).len(), 2);
}

// ------------------------------------------------------------------------------------------------ declarations

#[tokio::test]
async fn a_declaration_that_misses_an_added_patch_or_names_one_not_held_is_refused_and_a_corrected_resend_records_once()
 {
    if skip() {
        return;
    }
    let mut w = world("declare", Opts::default());
    w.client.commit(2);
    let art = w.client.artifact(&w.forge, "a1");
    let all = w.client.patch_ids(2);

    // Misses one added patch.
    let missing = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "x", "declared_patches": &all[..1]}),
        &art,
    )
    .await;
    assert_eq!(missing.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing.code(), "declaration_misses_added");
    assert_eq!(missing.json()["count"], 1);
    assert_eq!(missing.json()["patch_ids"][0], all[1].as_str());
    // The accepted patches stay (an accepted patch cannot be removed); the intent stays open; no revision, no change.
    assert_eq!(pending(&w.forge).len(), 2);
    assert!(
        w.changes
            .list_changes(&RepoId::new("r-forge"), None)
            .unwrap()
            .is_empty()
    );
    let open = w.changes.open_intents(&RepoId::new("r-forge")).unwrap();
    assert_eq!(open.len(), 1);
    assert!(
        open[0].pending_before.is_empty(),
        "the first attempt's list"
    );
    assert!(
        !w.changes
            .repo_flags(&RepoId::new("r-forge"))
            .unwrap()
            .needs_recovery,
        "a refused declaration is not a crash"
    );

    // Names a patch the forge does not hold.
    let mut with_stranger = all.clone();
    with_stranger.push("f".repeat(64));
    let not_held = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "x", "declared_patches": with_stranger}),
        &art,
    )
    .await;
    assert_eq!(not_held.code(), "declared_not_held");
    assert_eq!(not_held.json()["count"], 1);

    // The corrected re-send resumes the open intent and records exactly one revision, from the first attempt's list.
    let ok = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "x", "declared_patches": all}),
        &art,
    )
    .await;
    assert_eq!(
        ok.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&ok.body)
    );
    assert_eq!(
        ok.json()["added_count"],
        2,
        "derived from the first attempt's list, not the second's"
    );
    let revs = w
        .changes
        .revisions_of(ok.json()["proposal_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(revs.len(), 1);
    assert_eq!(
        w.metrics.snapshot().propose_attempts_max,
        3,
        "three attempts, one revision"
    );
    assert!(
        w.changes
            .open_intents(&RepoId::new("r-forge"))
            .unwrap()
            .is_empty()
    );
}

// ------------------------------------------------------------------------------------------------ who may, and how

#[tokio::test]
async fn a_stranger_a_reader_a_read_only_token_a_cookie_and_the_anonymous_are_refused_as_documented()
 {
    if skip() {
        return;
    }
    let mut w = world("authz", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let body = frame(&titled("x"), &art);

    // A stranger asking for a PRIVATE repository, and anybody asking for a repository that does not exist: the same.
    let hidden = send(
        &w.router,
        post_req("private", Some("tok-carol"), body.clone(), &[]),
    )
    .await;
    let absent = send(
        &w.router,
        post_req("no-such-repo", Some("tok-carol"), body.clone(), &[]),
    )
    .await;
    assert_eq!(
        (hidden.status, hidden.body.clone()),
        (StatusCode::NOT_FOUND, absent.body.clone())
    );
    assert_eq!(absent.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.code(), "not_found");
    // Anonymous, on a private repository: also 404.
    let anon_private = send(&w.router, post_req("private", None, body.clone(), &[])).await;
    assert_eq!(anon_private.status, StatusCode::NOT_FOUND);
    // A reader of a public repository (and the anonymous) learns only that they may not propose: 403.
    for token in [Some("tok-bob"), Some("tok-carol"), None] {
        let r = send(&w.router, post_req("forge", token, body.clone(), &[])).await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{token:?}"
        );
    }
    // The owner's token limited to reading may not propose either.
    let ro = send(
        &w.router,
        post_req("forge", Some("tok-alice-ro"), body.clone(), &[]),
    )
    .await;
    assert_eq!(
        (ro.status, ro.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    // A session cookie is not looked at (another site could make that request): without a bearer token, anonymous.
    // The control: the same cookie is a perfectly good session for a read.
    use planeter_auth::SessionStore;
    let session = w
        .sessions
        .create(UserId::new("alice"), now(), now() + 3600)
        .unwrap();
    let cookie = format!("planeter_session={}", session.id.0);
    let read = send(
        &w.router,
        Request::builder()
            .uri("/api/v1/repos/alice/private/transport/advertise")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(read.status, StatusCode::OK, "the cookie is a valid session");
    let by_cookie = send(
        &w.router,
        post_req("forge", None, body.clone(), &[("cookie", &cookie)]),
    )
    .await;
    assert_eq!(
        (by_cookie.status, by_cookie.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    // Nothing was accepted by any of them.
    assert!(pending(&w.forge).is_empty());
    assert!(
        w.changes
            .list_changes(&RepoId::new("r-forge"), None)
            .unwrap()
            .is_empty()
    );
    // A writer (dave) may; and under `any-authenticated`, carol (no role) may propose to the `anyauth` repository.
    let dave = send(
        &w.router,
        post_req("forge", Some("tok-dave"), body.clone(), &[]),
    )
    .await;
    assert_eq!(
        dave.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&dave.body)
    );
}

#[tokio::test]
async fn any_authenticated_opens_the_door_to_a_reader_and_the_quota_is_named_when_it_is_used_up() {
    if skip() {
        return;
    }
    let mut w = world("quota", Opts::default());
    let anyauth = w.base.join("anyauth");
    // Under `any-authenticated` carol (no role at all) may propose: 3 open proposals at most.
    let mut first_artifact = Vec::new();
    for i in 0..3 {
        w.client.commit(1);
        let art = w.client.artifact(&anyauth, &format!("q{i}"));
        if i == 0 {
            first_artifact = art.clone();
        }
        let r = propose_as(
            &w,
            "anyauth",
            "tok-carol",
            titled(&format!("change {i}")),
            &art,
        )
        .await;
        assert_eq!(
            r.status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&r.body)
        );
    }
    w.client.commit(1);
    let art = w.client.artifact(&anyauth, "q3");
    let r = propose_as(&w, "anyauth", "tok-carol", titled("one too many"), &art).await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let v = r.json();
    assert_eq!(v["code"], "quota_exceeded");
    assert_eq!(v["quota"], "open_proposals");
    assert_eq!(
        (v["limit"].clone(), v["used"].clone(), v["unit"].clone()),
        (3.into(), 3.into(), "count".into())
    );
    assert!(
        v["message"].as_str().unwrap().contains("3 open proposals"),
        "{v}"
    );
    assert!(v["hint"].as_str().unwrap().contains("Close one"), "{v}");
    assert_eq!(
        w.changes
            .list_changes(&RepoId::new("r-anyauth"), None)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        pending(&anyauth).len(),
        3,
        "the refused artifact was not accepted"
    );

    // A repeat of an artifact that is already recorded is answered with its revision, whatever the quotas now say.
    let again = propose_as(
        &w,
        "anyauth",
        "tok-carol",
        titled("change 0"),
        &first_artifact,
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&again.body)
    );
    assert_eq!(again.json()["already_recorded"], true);

    // The same refusal in the Japanese draft: the figures are filled in, in the language negotiated.
    let ja = send(&w.router, {
        let mut req = post_req(
            "anyauth",
            Some("tok-carol"),
            frame(&titled("ja"), &art),
            &[],
        );
        req.headers_mut()
            .insert("accept-language", "ja".parse().unwrap());
        req
    })
    .await;
    assert_eq!(ja.code(), "quota_exceeded");
    // (Japanese is a draft and is not shipped yet, so the reply is English until its reviewers sign it off.)
    assert_eq!(ja.json()["quota"], "open_proposals");
}

#[tokio::test]
async fn only_the_opener_or_a_writer_adds_to_a_proposal_and_only_while_it_is_open() {
    if skip() {
        return;
    }
    let mut w = world("changes", Opts::default());
    w.client.commit(1);
    let a1 = w.client.artifact(&w.forge, "a1");
    let first = propose_as(&w, "forge", "tok-dave", titled("Dave's"), &a1).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let change_id = first.json()["proposal_id"].as_str().unwrap().to_owned();
    // An unknown proposal.
    w.client.commit(1);
    let a2 = w.client.artifact(&w.forge, "a2");
    let unknown = propose_as(
        &w,
        "forge",
        "tok-dave",
        serde_json::json!({"proposal": "c-nope"}),
        &a2,
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "unknown_proposal")
    );
    // Alice (the owner, a writer) may append to dave's proposal; carol, no role, may not append to anything here.
    let by_alice = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"proposal": change_id}),
        &a2,
    )
    .await;
    assert_eq!(
        by_alice.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&by_alice.body)
    );
    // A third party without write cannot append (under any-authenticated, carol proposes to anyauth).
    let own = {
        w.client.commit(1);
        let art = w.client.artifact(&w.base.join("anyauth"), "c1");
        propose_as(&w, "anyauth", "tok-carol", titled("Carol's"), &art).await
    };
    assert_eq!(own.status, StatusCode::CREATED);
    let carols = own.json()["proposal_id"].as_str().unwrap().to_owned();
    w.client.commit(1);
    let art = w.client.artifact(&w.base.join("anyauth"), "c2");
    let other = propose_as(
        &w,
        "anyauth",
        "tok-bob",
        serde_json::json!({"proposal": carols}),
        &art,
    )
    .await;
    assert_eq!(
        (other.status, other.code().as_str()),
        (StatusCode::FORBIDDEN, "not_your_proposal")
    );
    // Closed: no more revisions.
    w.changes
        .leave_open(
            &carols,
            ChangeState::Closed,
            Some("withdrawn"),
            now(),
            Some(now() + 100),
        )
        .unwrap();
    let closed = propose_as(
        &w,
        "anyauth",
        "tok-carol",
        serde_json::json!({"proposal": carols}),
        &art,
    )
    .await;
    assert_eq!(
        (closed.status, closed.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "proposal_not_open")
    );
    assert_eq!(closed.json()["state"], "closed");
}

// ------------------------------------------------------------------------------------------------ the base

#[tokio::test]
async fn a_base_must_be_a_block_the_repository_holds() {
    if skip() {
        return;
    }
    let mut w = world("base", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let unknown = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "x", "base": "9".repeat(64)}),
        &art,
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "base_unknown")
    );
    assert!(pending(&w.forge).is_empty(), "refused before any accept");
    // The block the artifact was built on is held.
    let ok = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "x", "base": w.client.tip_before(1)}),
        &art,
    )
    .await;
    assert_eq!(ok.status, StatusCode::CREATED);
    // No base: recorded as none.
    w.client.commit(1);
    let art2 = w.client.artifact(&w.forge, "a2");
    let none = propose_as(&w, "forge", "tok-alice", titled("no base"), &art2).await;
    let revs = w
        .changes
        .revisions_of(none.json()["proposal_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(revs[0].declared_base, None);
    assert_eq!(
        revs[0].declared_patch_ids, None,
        "a push by plain prikk has no declaration"
    );
}

// ------------------------------------------------------------------------------------------------ the framing

#[tokio::test]
async fn a_malformed_body_is_refused_with_a_named_reason_and_nothing_is_accepted() {
    if skip() {
        return;
    }
    let mut w = world("framing", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let good = frame(&titled("x"), &art);
    let reason = |r: &Reply| r.json()["reason"].as_str().unwrap_or("").to_owned();

    // Wrong magic.
    let mut bad = good.clone();
    bad[0] = b'X';
    let r = send(&w.router, post_req("forge", Some("tok-alice"), bad, &[])).await;
    assert_eq!(
        (r.status, r.code().as_str(), reason(&r).as_str()),
        (StatusCode::BAD_REQUEST, "invalid_request", "magic")
    );
    // A body shorter than it says (the stream ends early).
    let mut req = post_req(
        "forge",
        Some("tok-alice"),
        good[..good.len() - 10].to_vec(),
        &[],
    );
    req.headers_mut()
        .insert("content-length", good.len().to_string().parse().unwrap());
    let r = send(&w.router, req).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // No Content-Length.
    let mut req = post_req("forge", Some("tok-alice"), good.clone(), &[]);
    req.headers_mut().remove("content-length");
    let r = send(&w.router, req).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::LENGTH_REQUIRED, "length_required")
    );
    // A content type that is not octet-stream.
    let mut req = post_req("forge", Some("tok-alice"), good.clone(), &[]);
    req.headers_mut()
        .insert("content-type", "text/plain".parse().unwrap());
    let r = send(&w.router, req).await;
    assert_eq!(
        (r.status, reason(&r).as_str()),
        (StatusCode::BAD_REQUEST, "content_type")
    );
    // Metadata that is not JSON, and an unknown field.
    let m = b"not json".to_vec();
    let mut raw = b"PLANETER-PROPOSE-1\n".to_vec();
    raw.extend_from_slice(&(m.len() as u64).to_be_bytes());
    raw.extend_from_slice(&m);
    raw.extend_from_slice(&art);
    let r = send(&w.router, post_req("forge", Some("tok-alice"), raw, &[])).await;
    assert_eq!(reason(&r), "not_json");
    let r = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "x", "surprise": 1}),
        &art,
    )
    .await;
    assert_eq!(
        (r.status, reason(&r).as_str()),
        (StatusCode::BAD_REQUEST, "unknown_field")
    );
    // Field errors: a new proposal needs a title; an existing one takes none; bad ids.
    for (meta, why) in [
        (serde_json::json!({}), "title_required"),
        (
            serde_json::json!({"title": "x".repeat(201)}),
            "title_too_long",
        ),
        (
            serde_json::json!({"title": "x", "base": "short"}),
            "base_invalid",
        ),
        (
            serde_json::json!({"title": "x", "declared_patches": ["short"]}),
            "patch_id_invalid",
        ),
        (
            serde_json::json!({"title": "x", "target_ref": "has space"}),
            "target_ref_invalid",
        ),
        (
            serde_json::json!({"proposal": "c-1", "title": "x"}),
            "fields_for_existing_proposal",
        ),
        (
            serde_json::json!({"proposal": "bad id!"}),
            "proposal_id_invalid",
        ),
    ] {
        let r = propose_as(&w, "forge", "tok-alice", meta, &art).await;
        assert_eq!(
            (r.status, reason(&r).as_str()),
            (StatusCode::BAD_REQUEST, why),
            "{why}"
        );
    }
    assert!(pending(&w.forge).is_empty());
    assert!(
        w.changes
            .list_changes(&RepoId::new("r-forge"), None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_dir(w.artifacts_dir.join("work"))
            .unwrap()
            .count(),
        0,
        "no working area is left behind"
    );
}

#[tokio::test]
async fn a_proposal_over_the_bound_is_refused_before_its_body_is_read() {
    if skip() {
        return;
    }
    let w = world(
        "oversize",
        Opts {
            config: ProposeConfig {
                max_artifact_bytes: 1000,
                ..Opts::default().config
            },
            ..Opts::default()
        },
    );
    // The declared length alone decides, before a byte of the body is read: this body never produces one.
    struct Never;
    impl futures_core::Stream for Never {
        type Item = Result<axum::body::Bytes, std::io::Error>;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Pending
        }
    }
    let mut hang = post_req("forge", Some("tok-alice"), Vec::new(), &[]);
    *hang.body_mut() = Body::from_stream(Never);
    hang.headers_mut()
        .insert("content-length", "5000000000".parse().unwrap());
    let r = tokio::time::timeout(Duration::from_secs(5), send(&w.router, hang))
        .await
        .expect("refused without waiting for the body");
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
    );
    // And with a short body behind the same declaration.
    let mut req = post_req(
        "forge",
        Some("tok-alice"),
        b"PLANETER-PROPOSE-1\n".to_vec(),
        &[],
    );
    req.headers_mut()
        .insert("content-length", "5000000000".parse().unwrap());
    let r = send(&w.router, req).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
    );
    // An artifact over the bound with the right length is refused too, as is a metadata length over its cap.
    let big = vec![7u8; 1001];
    let r = send(
        &w.router,
        post_req("forge", Some("tok-alice"), frame(&titled("x"), &big), &[]),
    )
    .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    let mut raw = b"PLANETER-PROPOSE-1\n".to_vec();
    raw.extend_from_slice(&(5u64 << 20).to_be_bytes());
    raw.extend_from_slice(&[0u8; 100]);
    let r = send(&w.router, post_req("forge", Some("tok-alice"), raw, &[])).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    // The refusals are counted by reason.
    assert!(
        w.metrics
            .snapshot()
            .refusals
            .get("payload_too_large")
            .copied()
            .unwrap_or(0)
            >= 3
    );
}

// ------------------------------------------------------------------------------------------------ recovery and busy

#[tokio::test]
async fn a_repository_that_needs_recovery_refuses_writes_and_still_answers_reads() {
    if skip() {
        return;
    }
    let mut w = world("flag", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    w.changes
        .set_needs_recovery(&RepoId::new("r-forge"), "a write did not finish", now())
        .unwrap();
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "needs_recovery")
    );
    assert!(pending(&w.forge).is_empty());
    // Reads are unaffected.
    let adv = send(
        &w.router,
        Request::builder()
            .uri("/api/v1/repos/alice/forge/transport/advertise")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(adv.status, StatusCode::OK);
    // The hook that recovery (part 4) will replace: clear the flag, and the same artifact goes through.
    w.changes
        .clear_needs_recovery(&RepoId::new("r-forge"))
        .unwrap();
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(r.status, StatusCode::CREATED);
}

#[tokio::test]
async fn a_busy_repository_is_a_queue_not_an_error() {
    if skip() {
        return;
    }
    let mut w = world(
        "busy",
        Opts {
            config: ProposeConfig {
                lease_wait: Duration::from_millis(300),
                ..Opts::default().config
            },
            ..Opts::default()
        },
    );
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    // An export holds the repository: the proposal waits its bound, then is told to retry, and prikk is never run.
    let held = w
        .leases
        .acquire(
            &RepoId::new("r-forge"),
            LeaseClass::Export,
            "archive export",
            Duration::from_secs(1),
        )
        .unwrap();
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "busy")
    );
    assert!(r.h("retry-after").parse::<u64>().unwrap() >= 1);
    assert!(pending(&w.forge).is_empty());
    assert!(
        w.changes
            .open_intents(&RepoId::new("r-forge"))
            .unwrap()
            .is_empty(),
        "no intent before the lease"
    );
    drop(held);
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(r.status, StatusCode::CREATED);
}

// ------------------------------------------------------------------------------------------------ the sealed rule

#[tokio::test]
async fn a_pending_patch_is_never_called_sealed_whatever_show_says_of_queued() {
    if skip() {
        return;
    }
    let mut w = world("assurance", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let patch = r.json()["patches"][0]["patch_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        r.json()["patches"][0]["assurance"],
        "forge-approved-unsealed"
    );
    // What `show` says of that pending patch: queued is false (U-10) ...
    let driver = planeter_prikk::CliPrikkRepo::new(&w.forge);
    let shown = driver.show(&patch).unwrap();
    assert!(
        shown.patches.iter().all(|p| !p.queued),
        "a pending patch reports queued: false"
    );
    // ... which `ChangeView::from_show` would read as sealed: that function is for the patches of a SEALED block
    // only, and the propose path never uses it.
    let by_show = planeter_core::ChangeView::from_show(&shown);
    assert_eq!(
        by_show.patches[0].assurance,
        planeter_core::Assurance::Sealed
    );
    let facts = planeter_core::change::PatchFacts {
        in_block_from_log: false,
        listed_pending: pending(&w.forge).contains(&patch),
        reported_queued: shown.patches[0].queued,
    };
    assert_eq!(
        planeter_core::change::patch_assurance(&facts),
        Some(planeter_core::Assurance::ForgeApprovedUnsealed)
    );
}

// ------------------------------------------------------------------------------------------------ a run that does not finish

/// A stand-in `prikk` that passes everything to the real one, except `sync accept`, which does what the control
/// file says. The control file is read at each call, so a test can change the behaviour between requests.
fn standin(base: &Path) -> (PathBuf, PathBuf) {
    let real = Command::new("sh")
        .args(["-c", "command -v prikk"])
        .output()
        .unwrap()
        .stdout;
    let real = String::from_utf8(real).unwrap().trim().to_owned();
    let ctrl = base.join("standin.mode");
    std::fs::write(&ctrl, "pass").unwrap();
    let script = base.join("prikk-standin");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
REAL={real}
if [ "$1" = sync ] && [ "$2" = accept ]; then
  case "$(cat {ctrl})" in
    die-after-real) "$REAL" "$@"; kill -9 $$ ;;
    unreadable) echo "this is not what accept prints"; exit 0 ;;
    flood) head -c 40000000 /dev/zero | tr '\0' ' '; exit 0 ;;
    refuse) echo "error: the artifact is damaged" >&2; exit 1 ;;
    fail-after-real) "$REAL" "$@" >/dev/null; echo "error: something broke midway" >&2; exit 1 ;;
    ceiling) echo "memory allocation of 4194304 bytes failed" >&2; exit 134 ;;
    lock) echo "error: lock conflict: container:object-store lock already exists: /x" >&2; exit 1 ;;
    *) exec "$REAL" "$@" ;;
  esac
fi
exec "$REAL" "$@"
"#,
            ctrl = ctrl.display()
        ),
    )
    .unwrap();
    let st = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(st.success());
    (script, ctrl)
}

fn standin_world(name: &str) -> (World, PathBuf) {
    let base = std::env::temp_dir().join(format!("planeter-standin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let (script, ctrl) = standin(&base);
    let w = world(
        name,
        Opts {
            binary: Some(script),
            ..Opts::default()
        },
    );
    (w, ctrl)
}

fn object_exists(w: &World, art: &[u8]) -> bool {
    let d = planeter_auth::sha256_hex(art);
    w.artifacts_dir
        .join("objects")
        .join(&d[..2])
        .join(&d)
        .is_file()
}

#[tokio::test]
async fn a_run_that_dies_after_changing_the_repository_is_incomplete_and_a_resend_after_recovery_records_exactly_one()
 {
    if skip() {
        return;
    }
    let (mut w, ctrl) = standin_world("died");
    std::fs::write(&ctrl, "die-after-real").unwrap();
    w.client.commit(2);
    let art = w.client.artifact(&w.forge, "a1");
    let repo = RepoId::new("r-forge");
    let fp_before = w
        .export
        .state(&w.store.get(&repo).unwrap().unwrap())
        .unwrap();

    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "incomplete"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let v = r.json();
    assert_eq!(v["cause"], "accept_failed");
    let intent_id = v["intent"].as_str().unwrap().to_owned();
    assert!(v["hint"].as_str().unwrap().contains("same artifact again"));

    // What the next start reads: an open intent with the first attempt's list, the flag, the artifact kept.
    let intent = w.changes.get_intent(&intent_id).unwrap().unwrap();
    assert_eq!(
        (intent.state, intent.attempts),
        (planeter_store::IntentState::Open, 1)
    );
    assert!(intent.pending_before.is_empty());
    assert!(w.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert!(object_exists(&w, &art), "the uploaded artifact is kept");
    assert!(
        w.changes.list_changes(&repo, None).unwrap().is_empty(),
        "a killed accept records nothing"
    );
    assert_eq!(
        pending(&w.forge).len(),
        2,
        "the repository did change (here, all of it)"
    );
    assert_ne!(
        fp_before,
        w.export
            .state(&w.store.get(&repo).unwrap().unwrap())
            .unwrap(),
        "so its fingerprint moved"
    );
    assert_eq!(w.metrics.snapshot().proposals_incomplete, 1);

    // Reads still work; the repository verifies and its doctor is clean (U-13); a new proposal is refused.
    let adv = send(
        &w.router,
        Request::builder()
            .uri("/api/v1/repos/alice/forge/transport/advertise")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(adv.status, StatusCode::OK);
    let driver = planeter_prikk::CliPrikkRepo::new(&w.forge);
    assert!(
        driver.verify().unwrap().verdict.ok,
        "verify is clean after a kill"
    );
    let doctor = Command::new("prikk")
        .arg("doctor")
        .current_dir(&w.forge)
        .output()
        .unwrap();
    assert!(
        doctor.status.success(),
        "doctor is clean: {}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    w.client.commit(1);
    let other = w.client.artifact(&w.forge, "a2");
    let blocked = propose_as(&w, "forge", "tok-alice", titled("y"), &other).await;
    assert_eq!(blocked.code(), "needs_recovery");

    // The real procedure (RFC 004 part 4) clears the flag. Re-sending the SAME artifact resumes the intent,
    // completes, and records exactly one revision, derived from the first attempt's list.
    let record = w.store.get(&repo).unwrap().unwrap();
    let recovered = w.recovery.recover(&record, "alice", &[]).unwrap();
    assert_eq!(
        recovered.outcome,
        planeter_transport::RecoveryOutcome::Recovered,
        "{recovered:?}"
    );
    assert!(!w.changes.repo_flags(&repo).unwrap().needs_recovery);
    std::fs::write(&ctrl, "pass").unwrap();
    let ok = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        ok.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&ok.body)
    );
    assert_eq!(
        ok.json()["added_count"],
        2,
        "both patches count as this revision's, though they were pending by then"
    );
    let revs = w
        .changes
        .revisions_of(ok.json()["proposal_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(revs.len(), 1, "exactly one revision");
    let done = w.changes.get_intent(&intent_id).unwrap().unwrap();
    assert_eq!(
        (done.state, done.attempts),
        (planeter_store::IntentState::Complete, 2)
    );
    assert_eq!(w.metrics.snapshot().propose_attempts_max, 2);
    assert!(w.changes.open_intents(&repo).unwrap().is_empty());
}

#[tokio::test]
async fn a_refusal_that_leaves_nothing_closes_the_intent_and_does_not_flag_the_repository() {
    if skip() {
        return;
    }
    let (mut w, ctrl) = standin_world("refused");
    std::fs::write(&ctrl, "refuse").unwrap();
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let repo = RepoId::new("r-forge");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "accept_refused")
    );
    let v = r.json();
    assert!(
        v["detail"]
            .as_str()
            .unwrap()
            .contains("the artifact is damaged"),
        "prikk's text is kept verbatim: {v}"
    );
    assert!(
        !v["message"].as_str().unwrap().contains("damaged"),
        "behind a plain sentence of ours"
    );
    assert!(!w.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert!(
        w.changes.open_intents(&repo).unwrap().is_empty(),
        "the intent is abandoned"
    );
    assert!(!object_exists(&w, &art), "a refused upload is not kept");
    assert!(pending(&w.forge).is_empty());
    // And the repository takes the next proposal.
    std::fs::write(&ctrl, "pass").unwrap();
    let ok = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(ok.status, StatusCode::CREATED);
}

#[tokio::test]
async fn output_that_cannot_be_read_a_failure_after_a_change_and_a_lock_conflict_are_all_incomplete()
 {
    if skip() {
        return;
    }
    let repo = RepoId::new("r-forge");
    for (mode, cause) in [
        ("unreadable", "accept_unreadable"),
        // IS-55: an accept that prints 40 MB (the limit is 16 MiB) is killed and is incomplete, like any output that cannot be read.
        ("flood", "accept_unreadable"),
        ("fail-after-real", "accept_failed"),
        ("lock", "accept_failed"),
    ] {
        let (mut w, ctrl) = standin_world(&format!("incomplete-{mode}"));
        std::fs::write(&ctrl, mode).unwrap();
        w.client.commit(1);
        let art = w.client.artifact(&w.forge, "a1");
        let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, "incomplete"),
            "{mode}"
        );
        assert_eq!(r.json()["cause"], cause, "{mode}");
        assert!(
            w.changes.repo_flags(&repo).unwrap().needs_recovery,
            "{mode}"
        );
        assert_eq!(w.changes.open_intents(&repo).unwrap().len(), 1, "{mode}");
        assert!(
            w.changes.list_changes(&repo, None).unwrap().is_empty(),
            "{mode}: nothing is derived from an unfinished run"
        );
    }
}

#[tokio::test]
async fn a_server_killed_during_a_write_comes_back_with_the_repository_flagged() {
    if skip() {
        return;
    }
    let w = world("reconcile", Opts::default());
    let repo = RepoId::new("r-forge");
    // The evidence a killed process leaves: a durable W holder record that this process did not grant.
    let records = Arc::new(InMemoryLeaseRecords::new());
    use planeter_store::{HolderRecord, LeaseRecordStore};
    records
        .record(&HolderRecord {
            repo_id: repo.clone(),
            class: "W".into(),
            operation: "sync accept".into(),
            holder_id: "old-boot-1".into(),
            boot_id: "old-boot".into(),
            since_unix: 1,
        })
        .unwrap();
    let leases = Arc::new(LeaseManager::new(
        records,
        new_boot_id(),
        now,
        Duration::from_secs(3),
    ));
    let artifacts = Arc::new(ArtifactStore::open(w.base.join("artifacts2")).unwrap());
    let svc = ProposeService::new(
        w.hosting.clone(),
        artifacts,
        leases,
        w.changes.clone() as Arc<dyn ChangeStore>,
        w.export.clone(),
        ProposeConfig::default(),
        w.metrics.clone(),
    );
    assert!(!w.changes.repo_flags(&repo).unwrap().needs_recovery);
    let flagged = svc.reconcile_at_start();
    assert_eq!(flagged, vec![repo.clone()]);
    let f = w.changes.repo_flags(&repo).unwrap();
    assert!(f.needs_recovery);
    assert!(f.reason.unwrap().contains("stopped during a write"));
    // Idempotent: a second start flags nothing new.
    assert!(svc.reconcile_at_start().is_empty());
    // A read-only (R) record, or an export's (X), is not a write.
    let _ = w.base;
}

// ------------------------------------------------------------------------------------------------ the real sandbox

/// A stand-in `prikk` that needs nothing outside the sandbox (bubblewrap binds only this script): it answers the
/// version pin and `sync pending` (nothing pending) itself, and does `accept_body` for `sync accept`.
fn sandbox_standin(base: &Path, accept_body: &str) -> PathBuf {
    let script = base.join("prikk-sandboxed");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo "prikk 0.54.1"; exit 0; fi
if [ "$1" = sync ] && [ "$2" = pending ]; then echo "pending (accepted, unsealed) patches: 0"; exit 0; fi
if [ "$1" = sync ] && [ "$2" = accept ]; then
{accept_body}
fi
echo "unexpected: $*" >&2; exit 2
"#
        ),
    )
    .unwrap();
    let st = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(st.success());
    script
}

fn sandboxed(name: &str, standin_body: Option<&str>, config: ProposeConfig) -> World {
    let base =
        std::env::temp_dir().join(format!("planeter-sandboxed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let binary = standin_body.map(|b| sandbox_standin(&base, b));
    world(
        name,
        Opts {
            config,
            sandbox: planeter_prikk::Sandbox::default(),
            binary,
            ..Opts::default()
        },
    )
}

#[tokio::test]
async fn under_the_real_sandbox_the_accept_sees_its_own_two_directories_and_nothing_else_of_the_store()
 {
    if skip() || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap and prlimit");
        return;
    }
    // The probe records, into the repository (the one place it may write), what it can see and do, then refuses.
    let w = sandboxed(
        "probe",
        Some(
            r#"  A="$3"; IN=$(dirname "$A"); WORK=$(dirname "$IN"); ROOT=$(dirname $(dirname "$WORK"))
  {
    echo "objects-listing: $(ls "$ROOT/objects" 2>/dev/null | wc -l)"
    echo "objects-write: $(touch "$ROOT/objects/evil" >/dev/null 2>&1 && echo ok || echo fail)"
    echo "incoming-write: $(touch "$ROOT/incoming/evil" >/dev/null 2>&1 && echo ok || echo fail)"
    echo "in-visible: $(ls "$IN" | tr '\n' ' ')"
    echo "in-write: $(touch "$IN/evil" >/dev/null 2>&1 && echo ok || echo fail)"
    echo "out-write: $(touch "$WORK/out/mine" >/dev/null 2>&1 && echo ok || echo fail)"
    echo "siblings: $(ls "$ROOT/work" | wc -l)"
    echo "artifact-bytes: $(wc -c < "$A")"
    echo "bound-bytes: ${PRIKK_EXCHANGE_MAX_BYTES:-unset} objects: ${PRIKK_EXCHANGE_MAX_OBJECTS:-unset}"
  } > .probe
  echo "error: probe done" >&2; exit 1"#,
        ),
        ProposeConfig {
            max_artifact_bytes: 7_000_000,
            max_objects: 1234,
            ..Opts::default().config
        },
    );
    let mut w = w;
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.code(), "accept_refused");
    let probe =
        std::fs::read_to_string(w.forge.join(".probe")).expect("the probe ran inside the sandbox");
    let line = |k: &str| {
        probe
            .lines()
            .find(|l| l.starts_with(k))
            .unwrap_or("")
            .to_owned()
    };
    assert_eq!(
        line("objects-listing"),
        "objects-listing: 0",
        "no stored artifact is visible\n{probe}"
    );
    assert_eq!(line("in-visible"), "in-visible: artifact ", "{probe}");
    assert_eq!(
        line("in-write"),
        "in-write: fail",
        "the input is read-only\n{probe}"
    );
    assert_eq!(
        line("out-write"),
        "out-write: ok",
        "the output directory is writable\n{probe}"
    );
    assert_eq!(
        line("siblings"),
        "siblings: 1",
        "only this proposal's own working area\n{probe}"
    );
    assert_eq!(
        line("artifact-bytes"),
        format!("artifact-bytes: {}", art.len()),
        "{probe}"
    );
    assert_eq!(
        line("bound-bytes"),
        "bound-bytes: 7000000 objects: 1234",
        "the incoming bounds reach prikk's environment\n{probe}"
    );
    // Whatever the probe wrote outside its two directories left nothing in the store.
    assert!(!w.artifacts_dir.join("objects/evil").exists());
    assert!(!w.artifacts_dir.join("incoming/evil").exists());
    assert_eq!(
        std::fs::read_dir(w.artifacts_dir.join("objects"))
            .unwrap()
            .count(),
        0,
        "a refused upload is not kept"
    );
}

#[tokio::test]
async fn an_accept_past_its_deadline_is_killed_and_is_incomplete() {
    if skip() || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap and prlimit");
        return;
    }
    let mut w = sandboxed(
        "deadline",
        Some("  sleep 60; exit 0"),
        ProposeConfig {
            accept_deadline: Duration::from_secs(2),
            ..Opts::default().config
        },
    );
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let started = std::time::Instant::now();
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline killed it: {:?}",
        started.elapsed()
    );
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "incomplete"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["cause"], "accept_deadline");
    let repo = RepoId::new("r-forge");
    assert!(w.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert_eq!(w.changes.open_intents(&repo).unwrap().len(), 1);
    assert!(object_exists(&w, &art));
}

#[tokio::test]
async fn under_the_real_sandbox_with_real_prikk_a_proposal_is_taken_in_and_an_over_bound_artifact_is_refused_by_prikk()
 {
    if skip() || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap and prlimit");
        return;
    }
    // prikk's own object bound (the edge checks bytes; the object count is prikk's to enforce): exit 1, nothing accepted.
    let mut w = sandboxed(
        "bound",
        None,
        ProposeConfig {
            max_objects: 1,
            ..Opts::default().config
        },
    );
    w.client.commit(2);
    let art = w.client.artifact(&w.forge, "a1");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "accept_refused"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert!(
        r.json()["detail"]
            .as_str()
            .unwrap()
            .contains("over the configured limit of 1"),
        "{}",
        r.json()
    );
    assert!(pending(&w.forge).is_empty(), "nothing was accepted");
    let repo = RepoId::new("r-forge");
    assert!(!w.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert!(w.changes.open_intents(&repo).unwrap().is_empty());

    // With the default bounds the same artifact goes through the whole real path: both mounts, the claims file,
    // the bounds, the ceiling, the deadline.
    let mut ok = sandboxed("sandboxed-ok", None, Opts::default().config);
    ok.client.commit(2);
    let art = ok.client.artifact(&ok.forge, "a1");
    let r = propose_as(&ok, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["added_count"], 2);
    assert_eq!(pending(&ok.forge).len(), 2);
}

// ------------------------------------------------------------------------------------------------ the order of the checks

fn open_change(id: &str, repo: &str, by: &str) -> planeter_store::ChangeRecord {
    planeter_store::ChangeRecord {
        change_id: id.to_owned(),
        repo_id: RepoId::new(repo),
        title: "t".to_owned(),
        description: String::new(),
        target_ref: None,
        state: ChangeState::Open,
        closed_reason: None,
        opened_by: by.to_owned(),
        opened_at: 1,
        updated_at: 1,
        retain_until: None,
    }
}

/// Each row: a request that breaks two rules at once, and the one named. The earlier check wins, and a refusal
/// never says more than the caller is entitled to know.
#[tokio::test]
async fn the_checks_run_in_the_documented_order_and_each_refusal_is_the_earlier_rule() {
    if skip() {
        return;
    }
    let mut w = world("order", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let garbage = b"this is not a proposal at all".to_vec();
    let good = frame(&titled("x"), &art);
    let code = |r: &Reply| (r.status.as_u16(), r.code());

    // 1. The read gate comes before everything about the body: a stranger and a garbage body for a private repository.
    let r = send(
        &w.router,
        post_req("private", Some("tok-carol"), garbage.clone(), &[]),
    )
    .await;
    assert_eq!(code(&r), (404, "not_found".into()));
    // 2. Authorization comes before the recovery flag and the body: a reader, a flagged repository, garbage.
    w.changes
        .set_needs_recovery(&RepoId::new("r-forge"), "x", now())
        .unwrap();
    let r = send(
        &w.router,
        post_req("forge", Some("tok-bob"), garbage.clone(), &[]),
    )
    .await;
    assert_eq!(code(&r), (403, "forbidden".into()));
    // 3. The recovery flag comes before the framing: a writer, a flagged repository, garbage.
    let r = send(
        &w.router,
        post_req("forge", Some("tok-alice"), garbage.clone(), &[]),
    )
    .await;
    assert_eq!(code(&r), (503, "needs_recovery".into()));
    w.changes
        .clear_needs_recovery(&RepoId::new("r-forge"))
        .unwrap();
    // 4. The framing comes before the metadata: garbage is a framing error, not a metadata one.
    let r = send(
        &w.router,
        post_req("forge", Some("tok-alice"), garbage.clone(), &[]),
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.json()["reason"].clone()),
        (400, "magic".into())
    );
    // 5. Size comes before the body is read: a declared length over the bound, with a body that is garbage.
    let mut req = post_req("forge", Some("tok-alice"), garbage.clone(), &[]);
    req.headers_mut()
        .insert("content-length", "99999999999".parse().unwrap());
    assert_eq!(
        code(&send(&w.router, req).await),
        (413, "payload_too_large".into())
    );
    // 6. The metadata comes before the change it names: an unknown proposal id together with a title is a metadata
    //    error (an existing proposal takes no title), not "unknown proposal".
    let r = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"proposal": "c-nope", "title": "x"}),
        &art,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.json()["reason"].clone()),
        (400, "fields_for_existing_proposal".into())
    );
    // 7. The proposal comes before the base: an unknown proposal and an unknown base name the proposal.
    let r = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"proposal": "c-nope", "base": "9".repeat(64)}),
        &art,
    )
    .await;
    assert_eq!(code(&r), (422, "unknown_proposal".into()));
    // 8. The base comes before the quota: with the quota used up, an unknown base is still named.
    for i in 0..3 {
        w.changes
            .create_change(&open_change(&format!("c-q{i}"), "r-anyauth", "carol"))
            .unwrap();
    }
    let anyauth_art = {
        let a = w.base.join("anyauth");
        w.client.artifact(&a, "qa")
    };
    let r = propose_as(
        &w,
        "anyauth",
        "tok-carol",
        serde_json::json!({"title": "x", "base": "9".repeat(64)}),
        &anyauth_art,
    )
    .await;
    assert_eq!(code(&r), (422, "base_unknown".into()));
    // 9. The metadata comes before the quota (the quota is held back until the artifact's digest is known):
    //    with the quota used up, a missing title is the metadata error.
    let r = propose_as(
        &w,
        "anyauth",
        "tok-carol",
        serde_json::json!({}),
        &anyauth_art,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.json()["reason"].clone()),
        (400, "title_required".into())
    );
    // 10. With nothing else wrong, the quota is named.
    let r = propose_as(&w, "anyauth", "tok-carol", titled("x"), &anyauth_art).await;
    assert_eq!(code(&r), (422, "quota_exceeded".into()));
    // Nothing above accepted anything.
    assert!(pending(&w.forge).is_empty());
    let _ = good;
}

#[tokio::test]
async fn the_rate_limit_comes_before_the_read_gate_and_says_nothing_about_any_repository() {
    if skip() {
        return;
    }
    let w = world(
        "ratelimit",
        Opts {
            ip_limit: 2,
            ..Opts::default()
        },
    );
    let garbage = b"x".to_vec();
    let a = send(
        &w.router,
        post_req("no-such-repo", Some("tok-carol"), garbage.clone(), &[]),
    )
    .await;
    let b = send(
        &w.router,
        post_req("private", Some("tok-carol"), garbage.clone(), &[]),
    )
    .await;
    assert_eq!(
        (a.status, b.status),
        (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
    );
    // The third, for a repository that exists and one that does not, is limited the same way.
    let c = send(
        &w.router,
        post_req("no-such-repo", Some("tok-carol"), garbage.clone(), &[]),
    )
    .await;
    let d = send(
        &w.router,
        post_req("private", Some("tok-carol"), garbage, &[]),
    )
    .await;
    assert_eq!(
        (c.status, c.code()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited".into())
    );
    assert_eq!(c.body, d.body);
    assert!(c.h("retry-after").parse::<u64>().unwrap() >= 1);
}

// ------------------------------------------------------------------------------------------------ IS-50: the debt

/// Each artifact whose declaration is refused is already accepted by prikk and cannot be taken back: its bytes are
/// owed. Different artifacts with bad declarations therefore reach the cumulative limit; a corrected re-send of one of
/// them records its revision and its bytes are counted once.
#[tokio::test]
async fn artifacts_whose_declarations_are_refused_are_counted_and_the_limit_is_reached() {
    if skip() {
        return;
    }
    let mut w = world(
        "owed",
        Opts {
            config: ProposeConfig {
                proposer_bytes_limit: Some(25_000),
                ..Opts::default().config
            },
            ..Opts::default()
        },
    );
    let repo = RepoId::new("r-forge");
    let mut sizes: Vec<u64> = Vec::new();
    let mut first_artifact = Vec::new();
    let mut refused_at = None;
    for i in 0..30 {
        w.client.commit(1);
        let art = w.client.artifact(&w.forge, &format!("o{i}"));
        let r = propose_as(
            &w,
            "forge",
            "tok-alice",
            serde_json::json!({"title": format!("bad declaration {i}"), "declared_patches": []}),
            &art,
        )
        .await;
        if r.code() == "quota_exceeded" {
            let v = r.json();
            assert_eq!(v["quota"], "accepted_bytes", "{v}");
            assert_eq!(v["unit"], "mebibytes");
            refused_at = Some(i);
            // The refusal left nothing: no new intent, nothing accepted by this push.
            assert_eq!(w.changes.open_intents(&repo).unwrap().len(), sizes.len());
            break;
        }
        assert_eq!(
            r.code(),
            "declaration_misses_added",
            "push {i}: {}",
            String::from_utf8_lossy(&r.body)
        );
        if i == 0 {
            first_artifact = art.clone();
        }
        sizes.push(art.len() as u64);
        assert!(
            object_exists_in(&w.artifacts_dir, &art),
            "the artifact of a refused declaration is kept"
        );
    }
    let at = refused_at.expect("the limit was reached");
    assert!(
        at >= 3,
        "several different artifacts were refused for their declarations first ({at})"
    );
    let owed: u64 = sizes.iter().sum();
    let usage = w.changes.usage(&repo, "alice", now()).unwrap();
    assert_eq!(usage.accepted_bytes, owed, "every open intent is a debt");
    assert_eq!(usage.repo_retained_bytes, owed);
    assert!(
        w.changes.list_changes(&repo, None).unwrap().is_empty(),
        "no revision exists"
    );

    // A corrected re-send of the first records its revision; its bytes are counted once.
    let all_pending = pending(&w.forge);
    let ok = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "corrected", "declared_patches": all_pending}),
        &first_artifact,
    )
    .await;
    assert_eq!(
        ok.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&ok.body)
    );
    let usage = w.changes.usage(&repo, "alice", now()).unwrap();
    assert_eq!(
        usage.accepted_bytes, owed,
        "counted once: the revision replaced the debt"
    );
    assert_eq!(
        w.changes.open_intents(&repo).unwrap().len(),
        sizes.len() - 1
    );
}

fn object_exists_in(artifacts_dir: &Path, art: &[u8]) -> bool {
    let d = planeter_auth::sha256_hex(art);
    artifacts_dir
        .join("objects")
        .join(&d[..2])
        .join(&d)
        .is_file()
}

// ------------------------------------------------------------------------------------------------ RFC 004 part 4, T0(e)

#[tokio::test]
async fn without_the_artifact_store_a_proposal_says_so_in_its_own_words() {
    if skip() {
        return;
    }
    let mut w = world(
        "propose-off",
        Opts {
            without_propose: true,
            ..Opts::default()
        },
    );
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "transport_off")
    );
    let v = r.json();
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("Proposals are not set up"),
        "{v}"
    );
    assert!(
        v["hint"].as_str().unwrap().contains("artifact store"),
        "{v}"
    );
    // The fetch side still works, and says its own thing when it is off.
    let adv = send(
        &w.router,
        Request::builder()
            .uri("/api/v1/repos/alice/forge/transport/advertise")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(adv.status, StatusCode::OK);
    // A stranger still learns nothing: the read gate comes first.
    let hidden = send(
        &w.router,
        post_req("private", Some("tok-carol"), frame(&titled("x"), &art), &[]),
    )
    .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
}

/// The memory-ceiling case is a test double of the driver: a stand-in `prikk` that fails the way prikk fails when it
/// hits `RLIMIT_DATA` (exit 134, "memory allocation of … failed"), because making the real prikk hit the ceiling needs
/// an artifact of gigabytes. The classification (`CeilingExceeded` → incomplete) is what is under test.
#[tokio::test]
async fn an_accept_that_hits_its_memory_ceiling_is_incomplete_not_refused() {
    if skip() {
        return;
    }
    let (mut w, ctrl) = standin_world("ceiling");
    std::fs::write(&ctrl, "ceiling").unwrap();
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "a1");
    let repo = RepoId::new("r-forge");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "incomplete"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["cause"], "accept_ceiling");
    assert!(w.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert_eq!(w.changes.open_intents(&repo).unwrap().len(), 1);
    assert!(object_exists(&w, &art));
}

/// After an accept that did not finish the server tries recovery by itself (on its own thread, once the child is
/// gone); when every witness agrees the flag is cleared without anyone being asked, and the same artifact then
/// records exactly one revision.
#[tokio::test]
async fn after_an_unfinished_accept_recovery_runs_by_itself_and_the_same_artifact_then_records_one_revision()
 {
    if skip() {
        return;
    }
    let base = std::env::temp_dir().join(format!("planeter-standin-auto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let (script, ctrl) = standin(&base);
    let mut w = world(
        "auto-recovery",
        Opts {
            binary: Some(script),
            auto_recovery: true,
            ..Opts::default()
        },
    );
    std::fs::write(&ctrl, "die-after-real").unwrap();
    w.client.commit(2);
    let art = w.client.artifact(&w.forge, "a1");
    let repo = RepoId::new("r-forge");
    let r = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, "incomplete")
    );
    // The flag was set before the answer; recovery clears it on its own thread.
    let end = std::time::Instant::now() + Duration::from_secs(30);
    while w.changes.repo_flags(&repo).unwrap().needs_recovery {
        assert!(
            std::time::Instant::now() < end,
            "recovery did not clear the flag by itself"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // The flag is cleared a step before the run's own record gets its outcome: wait for the record, not only the flag.
    let end = std::time::Instant::now() + Duration::from_secs(30);
    let runs = loop {
        let runs = w.recoveries.recent(&repo, 5).unwrap();
        if runs.iter().all(|r| r.outcome.is_some()) || std::time::Instant::now() >= end {
            break runs;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (runs[0].actor.as_str(), runs[0].outcome.as_deref()),
        ("automatic", Some("recovered"))
    );
    assert_eq!(
        w.changes.open_intents(&repo).unwrap().len(),
        1,
        "it never abandons an intent"
    );
    std::fs::write(&ctrl, "pass").unwrap();
    let ok = propose_as(&w, "forge", "tok-alice", titled("x"), &art).await;
    assert_eq!(
        ok.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&ok.body)
    );
    assert_eq!(ok.json()["added_count"], 2);
    assert_eq!(
        w.changes
            .revisions_of(ok.json()["proposal_id"].as_str().unwrap())
            .unwrap()
            .len(),
        1
    );
}

// ------------------------------------------------------------------------------------------------ the accept's cost model (part 4b)

/// IS-52 and part 4c: one large blob needs 7.0 × its size. The artifact of the **default** bound's largest size (a blob within a few KiB
/// of 100 MiB) goes through the whole real path (bubblewrap, `prlimit` with the default ceiling, real prikk, the claims file) and is accepted.
#[tokio::test]
async fn a_single_blob_of_the_default_bound_is_accepted_end_to_end_under_the_default_ceiling() {
    largest_blob_end_to_end("largest-blob", ProposeConfig::default()).await;
}

/// The operator can raise the bound by configuration: 256 MiB costs 1.8 GiB, and the accept of a blob of that size still goes through
/// the real sandbox under its (larger) ceiling; the start-up sentence agrees for a host that has the memory (`accept_admission.rs`).
#[tokio::test]
async fn a_single_blob_of_a_bound_raised_to_256_mib_by_configuration_is_accepted_end_to_end() {
    let config = ProposeConfig {
        max_artifact_bytes: 256 << 20,
        ..Opts::default().config
    };
    largest_blob_end_to_end("largest-blob-raised", config).await;
}

async fn largest_blob_end_to_end(name: &str, config: ProposeConfig) {
    if skip() || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap and prlimit");
        return;
    }
    let mut w = sandboxed(name, None, config.clone());
    let bound = config.max_artifact_bytes;
    // The blob plus prikk's own framing stays just under the bound (the measured overhead is a few hundred bytes).
    w.client.commit_blob((bound - 4096) as usize);
    let art = w.client.artifact(&w.forge, "big");
    assert!(
        (art.len() as u64) <= bound && (art.len() as u64) > bound - 8192,
        "the artifact is the largest the bound allows: {} of {bound}",
        art.len()
    );
    let ceiling = config.accept_ceiling(art.len() as u64);
    let r = propose_as(&w, "forge", "tok-alice", titled("the largest blob"), &art).await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{} (ceiling {} MiB)",
        String::from_utf8_lossy(&r.body),
        ceiling >> 20
    );
    assert_eq!(r.json()["added_count"], 1);
    assert_eq!(pending(&w.forge).len(), 1);
    let repo = RepoId::new("r-forge");
    assert!(!w.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert!(w.changes.open_intents(&repo).unwrap().is_empty());
}

/// The object bound is prikk's to enforce: an artifact one patch over it is refused (`accept_refused`), nothing is accepted and
/// nothing is flagged; one exactly at the bound goes through.
#[tokio::test]
async fn an_artifact_one_patch_over_the_object_bound_is_refused_by_prikk_and_one_at_the_bound_is_taken_in()
 {
    if skip() || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap and prlimit");
        return;
    }
    let config = ProposeConfig {
        max_objects: 3,
        ..Opts::default().config
    };
    let mut over = sandboxed("one-over", None, config.clone());
    over.client.commit(4);
    let art = over.client.artifact(&over.forge, "four");
    let r = propose_as(&over, "forge", "tok-alice", titled("four"), &art).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "accept_refused"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert!(
        r.json()["detail"]
            .as_str()
            .unwrap()
            .contains("declares 4 patches, over the configured limit of 3"),
        "{}",
        r.json()
    );
    assert!(pending(&over.forge).is_empty(), "nothing was accepted");
    let repo = RepoId::new("r-forge");
    assert!(!over.changes.repo_flags(&repo).unwrap().needs_recovery);
    assert!(over.changes.open_intents(&repo).unwrap().is_empty());

    let mut at = sandboxed("at-bound", None, config);
    at.client.commit(3);
    let art = at.client.artifact(&at.forge, "three");
    let r = propose_as(&at, "forge", "tok-alice", titled("three"), &art).await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(pending(&at.forge).len(), 3);
}

// ================================================================================================ proposals you can see
// RFC 005 part 5: the list, the detail, closing, the download for reviewers, the retention sweep, and the pages. Real
// prikk (self-skipping). Landing is observed from facts: a patch counts as landed only when the repository no longer
// holds it as pending **and** a block the repository lists contains it.

#[allow(dead_code)]
#[path = "structure/dom.rs"]
mod dom;
#[allow(dead_code)]
#[path = "structure/rules.rs"]
mod rules;

static PROPOSAL_NAME: AtomicU64 = AtomicU64::new(0);

async fn get_as(w: &World, uri: &str, token: Option<&str>, extra: &[(&str, &str)]) -> Reply {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    send(&w.router, req.body(Body::empty()).unwrap()).await
}

async fn close_as(w: &World, repo: &str, id: &str, token: Option<&str>, body: &str) -> Reply {
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/repos/alice/{repo}/proposals/{id}/close"))
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    send(&w.router, req.body(Body::from(body.to_owned())).unwrap()).await
}

fn repo_dir(w: &World, repo: &str) -> PathBuf {
    match repo {
        "forge" => w.forge.clone(),
        other => w.base.join(other),
    }
}

/// Commit `n` patches in the client and propose them to `repo` as `token`; the id of the proposal.
async fn make_proposal(w: &mut World, repo: &str, token: &str, title: &str, n: u32) -> String {
    make_proposal_on(w, repo, token, title, n, true).await
}

/// As `make_proposal`; `with_base: false` leaves the base out (the repository may not hold the client's previous tip).
async fn make_proposal_on(
    w: &mut World,
    repo: &str,
    token: &str,
    title: &str,
    n: u32,
    with_base: bool,
) -> String {
    w.client.commit(n);
    let name = format!("p{}", PROPOSAL_NAME.fetch_add(1, Ordering::Relaxed));
    let art = w.client.artifact(&repo_dir(w, repo), &name);
    let declared = w.client.patch_ids(n as usize);
    let reply = propose_as(
        w,
        repo,
        token,
        serde_json::json!({
            "title": title,
            "description": "Some *words* about it.\n\n<script>alert(1)</script>",
            "target_ref": "heads/main",
            "base": with_base.then(|| w.client.tip_before(n as usize)),
            "declared_patches": with_base.then_some(declared),
            "client": "tests/1"
        }),
        &art,
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json()["proposal_id"].as_str().unwrap().to_owned()
}

fn session_for(w: &World, user: &str) -> String {
    use planeter_auth::SessionStore as _;
    let s = w
        .sessions
        .create(UserId::new(user), now(), now() + 3600)
        .unwrap();
    format!("planeter_csrf=tok123; planeter_session={}", s.id.0)
}

async fn page_as(w: &World, method: &str, uri: &str, cookie: Option<&str>, lang: &str) -> Reply {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "forge.example")
        .header("accept-language", lang);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    send(&w.router, req.body(Body::empty()).unwrap()).await
}

async fn form_as(w: &World, uri: &str, cookie: Option<&str>, form: &str) -> Reply {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("host", "forge.example")
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    send(&w.router, req.body(Body::from(form.to_owned())).unwrap()).await
}

fn text(r: &Reply) -> String {
    String::from_utf8_lossy(&r.body).into_owned()
}

fn claims_of(w: &World, id: &str) -> Vec<String> {
    w.changes
        .revisions_of(id)
        .unwrap()
        .iter()
        .flat_map(|r| r.claim_ids.clone())
        .collect()
}

/// Seal one accepted claim into `heads/main` of the forge, with the client's key. This models **"a key-holding action published
/// it"**: the one thing that makes a patch part of a *published* ref's history here (D-40), whatever D-8 turns out to define. (The
/// real maintainer seals on their own computer, and history that is merely received here does not count; see
/// `history_received_by_import_is_a_sighting_and_never_lands_the_proposal`.)
fn seal_claim(w: &World, repo: &str, claim: &str) {
    prikk(
        &repo_dir(w, repo),
        &w.client.keys,
        &["sync", "seal", "heads/main", "--claim", claim],
    );
}

#[tokio::test]
async fn the_list_and_the_detail_show_what_is_held_for_review_and_say_nothing_more() {
    if skip() {
        return;
    }
    let mut w = world("pr-list", Opts::default());
    let id = make_proposal(&mut w, "forge", "tok-alice", "Two small changes", 2).await;

    // A reader, and an anonymous caller of a public repository, see it.
    for token in [Some("tok-bob"), None] {
        let r = get_as(&w, "/api/v1/repos/alice/forge/proposals", token, &[]).await;
        assert_eq!(r.status, StatusCode::OK, "{}", text(&r));
        let v = r.json();
        let list = v["proposals"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        let p = &list[0];
        assert_eq!(p["id"], id.as_str());
        assert_eq!(p["title"], "Two small changes");
        assert_eq!(p["state"], "open");
        assert_eq!(p["opened_by"], "alice");
        assert_eq!(p["revision_count"], 1);
        assert_eq!(p["patch_count"], 2);
        assert_eq!(p["built_on"], true);
        assert!(p["retention"].is_null(), "an open proposal has no end date");
        assert!(v["next"].is_null());
    }

    let d = get_as(
        &w,
        &format!("/api/v1/repos/alice/forge/proposals/{id}"),
        Some("tok-bob"),
        &[],
    )
    .await;
    assert_eq!(d.status, StatusCode::OK, "{}", text(&d));
    let v = d.json();
    assert_eq!(v["state"], "open");
    assert!(v["landed_at"].is_null());
    assert_eq!(v["still_pending"], 2);
    assert_eq!(v["patch_total"], 2);
    assert_eq!(v["truncated"], false);
    let patches = v["patches"].as_array().unwrap();
    assert_eq!(patches.len(), 2);
    assert!(
        patches
            .iter()
            .all(|p| p["assurance"] == "forge-approved-unsealed"),
        "held, not signed in: never more than that"
    );
    // The description is shown as text and as sanitised HTML: the script is gone from the HTML.
    assert!(
        v["description"]["source"]
            .as_str()
            .unwrap()
            .contains("<script>")
    );
    let html = v["description"]["html"].as_str().unwrap();
    assert!(html.contains("<em>words</em>"), "{html}");
    assert!(!html.contains("<script"), "{html}");
    let rev = &v["revisions"][0];
    assert_eq!(rev["seq"], 1);
    assert_eq!(rev["submitter"], "alice");
    assert_eq!(rev["declared"], true);
    assert_eq!(rev["added_count"], 2);
    assert_eq!(rev["artifact_available"], true);
    assert_eq!(rev["claims"].as_array().unwrap().len(), 2);
    assert!(
        v["note"]
            .as_str()
            .unwrap()
            .contains("does not merge anything")
    );

    // Nothing about another repository's proposal, or an unknown one, differs from "no such proposal".
    let unknown = get_as(
        &w,
        "/api/v1/repos/alice/forge/proposals/nope",
        Some("tok-bob"),
        &[],
    )
    .await;
    let other_repo = get_as(
        &w,
        &format!("/api/v1/repos/alice/anyauth/proposals/{id}"),
        Some("tok-bob"),
        &[],
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(other_repo.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body, other_repo.body);

    // A private repository: a stranger is told nothing, for the list and for an id alike.
    for uri in [
        "/api/v1/repos/alice/private/proposals".to_owned(),
        format!("/api/v1/repos/alice/private/proposals/{id}"),
    ] {
        let r = get_as(&w, &uri, Some("tok-carol"), &[]).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{uri}");
    }

    // Bad queries are refused by name.
    for (q, reason) in [("state=weird", "unknown_state"), ("limit=0", "limit_zero")] {
        let r = get_as(
            &w,
            &format!("/api/v1/repos/alice/forge/proposals?{q}"),
            Some("tok-bob"),
            &[],
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
        assert_eq!(r.json()["reason"], reason);
    }
    let r = get_as(
        &w,
        "/api/v1/repos/alice/forge/proposals?state=landed",
        Some("tok-bob"),
        &[],
    )
    .await;
    assert_eq!(r.json()["proposals"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_proposal_lands_only_when_every_patch_is_in_a_sealed_block_and_none_is_pending() {
    if skip() {
        return;
    }
    let mut w = world("pr-landing", Opts::default());
    let id = make_proposal(&mut w, "forge", "tok-alice", "Two patches", 2).await;
    let claims = claims_of(&w, &id);
    assert_eq!(claims.len(), 2);
    assert_eq!(detail_of(&w, &id).await["state"], "open");

    // One of the two claims sealed: one patch is in a block, the other is still pending. Not landed.
    seal_claim(&w, "forge", &claims[0]);
    let v = detail_of(&w, &id).await;
    assert_eq!(v["state"], "open", "half of it is in the history");
    assert_eq!(v["still_pending"], 1);
    let mut kinds: Vec<String> = v["patches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["assurance"].as_str().unwrap_or("none").to_owned())
        .collect();
    kinds.sort();
    assert_eq!(kinds, ["forge-approved-unsealed", "sealed"]);

    // The other sealed too: every patch is in a block and none is pending. Landed, observed from the repository.
    seal_claim(&w, "forge", &claims[1]);
    let v = detail_of(&w, &id).await;
    assert_eq!(v["state"], "landed", "{v}");
    assert!(v["landed_at"].is_number());
    assert_eq!(v["still_pending"], 0);
    assert!(
        v["patches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["assurance"] == "sealed")
    );
    // A landed proposal keeps its artifacts for the grace period.
    assert_eq!(v["retention"]["state"], "kept");
    let open = get_as(
        &w,
        "/api/v1/repos/alice/forge/proposals",
        Some("tok-bob"),
        &[],
    )
    .await
    .json();
    assert_eq!(open["proposals"].as_array().unwrap().len(), 0);
    let landed = get_as(
        &w,
        "/api/v1/repos/alice/forge/proposals?state=landed",
        Some("tok-bob"),
        &[],
    )
    .await
    .json();
    assert_eq!(landed["proposals"][0]["id"], id.as_str());

    // Landed is final: it cannot be closed any more.
    let r = close_as(&w, "forge", &id, Some("tok-alice"), r#"{"reason":"late"}"#).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.code(), "proposal_final");
    assert_eq!(r.json()["state"], "landed");
}

/// Patches that are not pending and are in no block the repository lists have not landed: leaving the pending list
/// is not landing. (The records are seeded: the forge cannot be made to drop a pending patch without sealing it.)
#[tokio::test]
async fn a_patch_that_is_merely_absent_from_pending_is_not_landed() {
    if skip() {
        return;
    }
    let w = world("pr-negative", Opts::default());
    let repo = RepoId::new("r-forge");
    let ids: Vec<String> = (1..=2).map(|n| format!("{n:064x}")).collect();
    let revision = planeter_store::RevisionRecord {
        revision_id: "v-neg".into(),
        change_id: "c-neg".into(),
        repo_id: repo.clone(),
        seq: 1,
        artifact_digest: format!("sha256:{:064x}", 7),
        artifact_bytes: 10,
        declared_base: None,
        claim_ids: vec![format!("{:064x}", 9)],
        added_patch_ids: ids.clone(),
        declared_patch_ids: Some(ids.clone()),
        claim_outcomes: vec![],
        submitter: "alice".into(),
        author_key_ids: vec![],
        created_at: now(),
    };
    let change = planeter_store::ChangeRecord {
        change_id: "c-neg".into(),
        repo_id: repo.clone(),
        title: "Gone from pending, never sealed".into(),
        description: String::new(),
        target_ref: None,
        state: ChangeState::Open,
        closed_reason: None,
        opened_by: "alice".into(),
        opened_at: now(),
        updated_at: now(),
        retain_until: None,
    };
    w.changes
        .open_or_resume_intent(
            "i-neg",
            &repo,
            &revision.artifact_digest,
            0,
            &[],
            "alice",
            now(),
        )
        .unwrap();
    w.changes
        .complete_intent_with_revision("i-neg", Some(&change), &revision, now())
        .unwrap();
    assert!(pending(&w.forge).is_empty());

    let v = get_as(
        &w,
        "/api/v1/repos/alice/forge/proposals/c-neg",
        Some("tok-bob"),
        &[],
    )
    .await
    .json();
    assert_eq!(v["state"], "open", "{v}");
    assert!(
        v["patches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["assurance"].is_null()),
        "nothing is claimed about a patch that is neither held nor in the history"
    );
    assert_eq!(v["still_pending"], 0);
}

#[tokio::test]
async fn closing_needs_a_reason_and_belongs_to_the_opener_or_to_whoever_may_close() {
    if skip() {
        return;
    }
    let mut w = world("pr-close", Opts::default());
    // carol may propose to `anyauth` (any signed-in account) but holds no role there.
    let mine = make_proposal(&mut w, "anyauth", "tok-carol", "Carol's", 1).await;
    let uri = format!("/api/v1/repos/alice/anyauth/proposals/{mine}");

    // A stranger who can read the repository: refused, by name, with the hint.
    for (who, body) in [
        (Some("tok-bob"), r#"{"reason":"no"}"#),
        (None, r#"{"reason":"no"}"#),
    ] {
        let r = close_as(&w, "anyauth", &mine, who, body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{who:?}");
        assert_eq!(r.code(), "forbidden");
    }
    // The opener with a read-only credential: the proposal is theirs, the token is not allowed to close anything.
    let r = close_as(
        &w,
        "anyauth",
        &mine,
        Some("tok-carol-ro"),
        r#"{"reason":"no"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", text(&r));
    assert_eq!(r.code(), "forbidden");
    // The reason is required, and bounded.
    for body in ["{}", r#"{"reason":""}"#, r#"{"reason":"   "}"#] {
        let r = close_as(&w, "anyauth", &mine, Some("tok-carol"), body).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(r.code(), "reason_required");
    }
    let long = format!(r#"{{"reason":"{}"}}"#, "x".repeat(1001));
    let r = close_as(&w, "anyauth", &mine, Some("tok-carol"), &long).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["reason"], "reason_too_long");
    let r = close_as(
        &w,
        "anyauth",
        &mine,
        Some("tok-carol"),
        r#"{"reason":"x","extra":1}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["reason"], "unknown_field");
    let r = close_as(&w, "anyauth", &mine, Some("tok-carol"), "not json").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["reason"], "not_json");

    // Nothing changed so far.
    let v = get_as(&w, &uri, None, &[]).await.json();
    assert_eq!(v["state"], "open");

    // The opener closes it, with a reason; the record and the files stay.
    let r = close_as(
        &w,
        "anyauth",
        &mine,
        Some("tok-carol"),
        r#"{"reason":"Superseded by another way"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", text(&r));
    let v = r.json();
    assert_eq!(v["state"], "closed");
    assert_eq!(v["closed_reason"], "Superseded by another way");
    assert!(v["retain_until"].as_u64().unwrap() > now());
    let d = get_as(&w, &uri, None, &[]).await.json();
    assert_eq!(d["state"], "closed");
    assert_eq!(d["closed_reason"], "Superseded by another way");
    assert_eq!(d["retention"]["state"], "kept");
    assert_eq!(d["revisions"][0]["artifact_available"], true);

    // Twice is refused: closed is final.
    let r = close_as(
        &w,
        "anyauth",
        &mine,
        Some("tok-carol"),
        r#"{"reason":"again"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.code(), "proposal_final");
    assert_eq!(r.json()["state"], "closed");

    // A read-only credential of the opener is not enough; a maintainer's (write) is, for anyone's proposal.
    let theirs = make_proposal_on(&mut w, "forge", "tok-dave", "Dave's", 1, false).await;
    let r = close_as(
        &w,
        "forge",
        &theirs,
        Some("tok-alice-ro"),
        r#"{"reason":"x"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = close_as(&w, "forge", &theirs, Some("tok-bob"), r#"{"reason":"x"}"#).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "bob only reads here");
    let r = close_as(
        &w,
        "forge",
        &theirs,
        Some("tok-alice"),
        r#"{"reason":"Not wanted"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", text(&r));

    // Unknown ids are `404` before anything is said about permission; a private repository is `404` to a stranger.
    let r = close_as(&w, "forge", "nope", Some("tok-bob"), r#"{"reason":"x"}"#).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = close_as(&w, "private", &mine, Some("tok-carol"), r#"{"reason":"x"}"#).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // The cookie of a signed-in browser is not a credential here: the route is for tokens.
    let cookie = session_for(&w, "alice");
    let theirs2 = make_proposal_on(&mut w, "forge", "tok-dave", "Dave's second", 1, false).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/repos/alice/forge/proposals/{theirs2}/close"
        ))
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(r#"{"reason":"x"}"#))
        .unwrap();
    let r = send(&w.router, req).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reviewers_download_the_accepted_files_byte_for_byte_and_the_sweep_removes_them_after_the_grace()
 {
    if skip() {
        return;
    }
    let mut w = world("pr-download", Opts::default());
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "dl");
    let reply = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"title": "To download", "base": w.client.tip_before(1), "client": "tests/1"}),
        &art,
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", text(&reply));
    let id = reply.json()["proposal_id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/1/artifact");

    // dave writes here (he may review); bob only reads; an anonymous caller reads a public repository.
    let r = get_as(&w, &uri, Some("tok-dave"), &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, art, "byte for byte what was accepted");
    assert_eq!(r.h("content-type"), "application/octet-stream");
    assert!(r.h("content-disposition").contains("attachment"));
    assert!(
        r.h("content-disposition")
            .contains(&format!("forge-proposal-{id}-r1.prikk-sync"))
    );
    let digest = planeter_auth::sha256_hex(&art);
    assert_eq!(r.h("etag"), format!("\"{digest}\""));
    for who in [Some("tok-bob"), None] {
        let r = get_as(&w, &uri, who, &[]).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{who:?}");
        assert_eq!(r.code(), "forbidden");
    }
    // A private repository: `404` to a stranger.
    let r = get_as(
        &w,
        &format!("/api/v1/repos/alice/private/proposals/{id}/revisions/1/artifact"),
        Some("tok-carol"),
        &[],
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // No such revision, or not a number.
    for tail in ["2", "0", "x"] {
        let r = get_as(
            &w,
            &format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/{tail}/artifact"),
            Some("tok-dave"),
            &[],
        )
        .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{tail}");
    }

    // Ranges: a part, and the rest from a point; a conditional request; an unsatisfiable one.
    let etag = r_etag(&w, &uri).await;
    let part = get_as(&w, &uri, Some("tok-dave"), &[("range", "bytes=10-99")]).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body, art[10..100]);
    assert_eq!(
        part.h("content-range"),
        format!("bytes 10-99/{}", art.len())
    );
    let rest = get_as(&w, &uri, Some("tok-dave"), &[("range", "bytes=100-")]).await;
    assert_eq!([&art[..100], &rest.body[..]].concat(), art);
    let same = get_as(&w, &uri, Some("tok-dave"), &[("if-none-match", &etag)]).await;
    assert_eq!(same.status, StatusCode::NOT_MODIFIED);
    let stale = get_as(
        &w,
        &uri,
        Some("tok-dave"),
        &[("range", "bytes=10-99"), ("if-range", "\"other\"")],
    )
    .await;
    assert_eq!(
        stale.status,
        StatusCode::OK,
        "a stale validator gets the whole file"
    );
    let big = get_as(
        &w,
        &uri,
        Some("tok-dave"),
        &[("range", &format!("bytes={}-", art.len() + 5))],
    )
    .await;
    assert_eq!(big.status, StatusCode::RANGE_NOT_SATISFIABLE);

    // The sweep: nothing before the grace ends, and nothing while the proposal is open.
    let sweeper = |later: fn() -> u64| {
        ProposalService::new(
            w.hosting.clone(),
            w.changes.clone() as Arc<dyn ChangeStore>,
            w.export.clone(),
            w.artifacts.clone(),
            w.metrics.clone(),
            ProposalConfig::default(),
        )
        .with_clock(later)
    };
    fn after_grace() -> u64 {
        now() + 31 * 86_400
    }
    let report = sweeper(after_grace).sweep();
    assert_eq!(report.removed, 0, "an open proposal's files are kept");
    let r = close_as(
        &w,
        "forge",
        &id,
        Some("tok-alice"),
        r#"{"reason":"Done with"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    fn within_grace() -> u64 {
        now() + 29 * 86_400
    }
    assert_eq!(
        sweeper(within_grace).sweep().removed,
        0,
        "kept until the grace ends"
    );
    assert_eq!(
        get_as(&w, &uri, Some("tok-dave"), &[]).await.status,
        StatusCode::OK
    );
    let report = sweeper(after_grace).sweep();
    assert_eq!(report.removed, 1, "{report:?}");
    let gone = get_as(&w, &uri, Some("tok-dave"), &[]).await;
    assert_eq!(gone.status, StatusCode::GONE);
    assert_eq!(gone.code(), "artifact_removed");
    assert!(gone.json()["kept_until"].as_u64().unwrap() > now());
    assert!(
        gone.json()["message"]
            .as_str()
            .unwrap()
            .contains("kept until 19"),
        "{}",
        text(&gone)
    );
    // The record stays and says what became of the files.
    let d = get_as(
        &w,
        &format!("/api/v1/repos/alice/forge/proposals/{id}"),
        Some("tok-bob"),
        &[],
    )
    .await
    .json();
    assert_eq!(d["state"], "closed");
    assert_eq!(d["retention"]["state"], "removed");
    assert_eq!(d["revisions"][0]["artifact_available"], false);
    // The page says the same in words, and no longer offers the download.
    let owner = session_for(&w, "alice");
    let page = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            Some(&owner),
            "en",
        )
        .await,
    );
    assert!(
        page.contains("The files of this proposal were kept until 19"),
        "{page}"
    );
    assert!(page.contains("have since been removed."), "{page}");
    assert!(page.contains("Removed"), "{page}");
    assert!(!page.contains("/artifact"), "{page}");
    // Running it again finds nothing to do.
    assert_eq!(sweeper(after_grace).sweep().removed, 0);
}

async fn detail_of(w: &World, id: &str) -> serde_json::Value {
    get_as(
        w,
        &format!("/api/v1/repos/alice/forge/proposals/{id}"),
        Some("tok-bob"),
        &[],
    )
    .await
    .json()
}

async fn r_etag(w: &World, uri: &str) -> String {
    get_as(w, uri, Some("tok-dave"), &[])
        .await
        .h("etag")
        .to_owned()
}

fn violations(html: &str) -> Vec<String> {
    rules::check(&dom::parse(html))
        .into_iter()
        .map(|v| format!("{}: {}", v.rule, v.detail))
        .collect()
}

fn unfilled(page: &str) -> Option<String> {
    let b: Vec<char> = page.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '{' {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_lowercase() || b[j] == '_') {
                j += 1;
            }
            if j > i + 1 && j < b.len() && b[j] == '}' {
                return Some(b[i..=j].iter().collect());
            }
        }
        i += 1;
    }
    None
}

#[tokio::test]
async fn the_pages_say_the_default_view_in_plain_words_and_keep_the_rest_under_details() {
    if skip() {
        return;
    }
    let mut w = world("pr-pages", Opts::default());
    // Before any proposal: bob may not propose here (members only) and the repository has none: no link.
    let home = page_as(&w, "GET", "/alice/forge", None, "en").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(
        !text(&home).contains("/alice/forge/proposals"),
        "no link without a proposal and without the right to propose"
    );
    let write = session_for(&w, "dave");
    let home = page_as(&w, "GET", "/alice/forge", Some(&write), "en").await;
    assert!(
        text(&home).contains(">Proposals</a>"),
        "a person who may propose sees the link"
    );

    let id = make_proposal(&mut w, "forge", "tok-alice", "Two small changes", 2).await;
    let anon_home = page_as(&w, "GET", "/alice/forge", None, "en").await;
    let h = text(&anon_home);
    assert!(h.contains(">Proposals</a>"), "{h}");
    assert!(h.contains("1 open proposal"), "{h}");
    assert!(!h.contains("1 open proposals"));

    // The list.
    let list = page_as(&w, "GET", "/alice/forge/proposals", None, "en").await;
    assert_eq!(list.status, StatusCode::OK, "{}", text(&list));
    let l = text(&list);
    assert!(
        l.contains("<title>Proposals · alice/forge · planeter</title>"),
        "{l}"
    );
    assert!(
        l.contains(&format!(
            "<a href=\"/alice/forge/proposals/{id}\">Two small changes</a>"
        )),
        "{l}"
    );
    assert!(l.contains("2 patches in 1 revision"), "{l}");
    assert!(l.contains("<span class=\"badge\">Open</span>"), "{l}");
    assert!(l.contains("aria-current=\"page\""), "{l}");
    assert_eq!(violations(&l), Vec::<String>::new(), "{l}");
    assert_eq!(unfilled(&l), None);

    // The detail: the default view, then everything else under one <details>.
    let d = page_as(
        &w,
        "GET",
        &format!("/alice/forge/proposals/{id}"),
        None,
        "en",
    )
    .await;
    assert_eq!(d.status, StatusCode::OK, "{}", text(&d));
    let p = text(&d);
    assert!(
        p.contains("<title>Two small changes · Proposals · alice/forge · planeter</title>"),
        "{p}"
    );
    assert!(
        p.contains("<h1>Two small changes <span class=\"badge\">Open</span></h1>"),
        "{p}"
    );
    assert!(
        p.contains("Held for review: not yet part of the project&#39;s history.")
            || p.contains("Held for review: not yet part of the project's history."),
        "{p}"
    );
    assert!(p.contains("Proposed by alice"), "{p}");
    assert!(p.contains("2 patches in 1 revision"), "{p}");
    assert!(
        p.contains("The file-by-file view is not available yet."),
        "{p}"
    );
    assert_eq!(
        p.matches("<details").count() - p.matches("<details class=\"explain\"").count(),
        1,
        "one Details"
    );
    let (before, after) = p
        .split_once("<summary>Details</summary>")
        .expect("a Details section");
    for later in ["Revisions", "Description", "Patches", "ed25519"] {
        assert!(
            !before.contains(later) || later == "Patches" && before.contains("2 patches"),
            "{later} is under Details:\n{before}"
        );
    }
    assert!(after.contains("<h2>Revisions</h2>"), "{after}");
    assert!(after.contains("<h2>Patches</h2>"), "{after}");
    assert!(
        after.contains("<em>words</em>"),
        "the description is rendered: {after}"
    );
    assert!(!p.contains("<script"), "the description is sanitised");
    assert!(
        p.contains("badge warn") || after.contains("Queued"),
        "the existing badge for a held patch: {after}"
    );
    assert!(
        !before.contains("Download"),
        "an anonymous reader has no download"
    );
    assert!(
        !p.contains("/artifact"),
        "no download link anywhere on the page, Details included"
    );
    assert!(
        !p.contains("Removed"),
        "files that are kept are not called removed, whoever is looking"
    );
    assert!(
        !p.contains("<details open"),
        "Details is closed until it is opened"
    );
    assert!(
        !p.contains("Close proposal"),
        "an anonymous reader has no close form"
    );
    assert!(
        !p.contains("Review state") && !p.contains("Checks"),
        "nothing about things that do not exist yet"
    );
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");
    assert_eq!(unfilled(&p), None);

    // A reviewer sees the download; the owner sees the form.
    let owner = session_for(&w, "alice");
    let d = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            Some(&owner),
            "en",
        )
        .await,
    );
    assert!(
        d.contains(&format!(
            "/api/v1/repos/alice/forge/proposals/{id}/revisions/1/artifact"
        )),
        "{d}"
    );
    assert!(d.contains("Download the files of revision 1"), "{d}");
    assert!(
        d.contains("Closing keeps the record and the files; nobody can reopen it."),
        "{d}"
    );
    assert!(
        d.contains("<button type=\"submit\">Close proposal</button>"),
        "{d}"
    );
    assert_eq!(violations(&d), Vec::<String>::new(), "{d}");

    // Two languages beside English: the Japanese draft (previewed) passes the same structural rules, and the test language
    // (right to left, every catalog message wrapped in ‹ ›) fills every placeholder and shows no catalog sentence unwrapped.
    planeter_web::catalog::enable_preview_languages("ja").expect("ja is previewable");
    register_qaa();
    let dump = std::env::var("PROPOSALS_HTML_DUMP").ok().map(PathBuf::from);
    if let Some(dir) = &dump {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("list-en.html"), &l).unwrap();
        std::fs::write(dir.join("detail-en.html"), &p).unwrap();
        std::fs::write(dir.join("detail-owner-en.html"), &d).unwrap();
    }
    for (name, uri) in [
        ("list", "/alice/forge/proposals".to_owned()),
        ("detail", format!("/alice/forge/proposals/{id}")),
        ("repo", "/alice/forge".to_owned()),
    ] {
        let ja = page_as(&w, "GET", &uri, Some(&owner), "ja").await;
        assert_eq!(ja.status, StatusCode::OK, "ja {uri}");
        let ja = text(&ja);
        assert!(
            ja.contains("<html lang=\"ja\""),
            "ja {uri}: not served in Japanese"
        );
        assert!(ja.contains("提案"), "ja {uri}");
        assert_eq!(unfilled(&ja), None, "ja {uri}");
        assert_eq!(violations(&ja), Vec::<String>::new(), "ja {uri}: {ja}");
        let qaa = text(&page_as(&w, "GET", &uri, Some(&owner), "qaa").await);
        assert!(qaa.contains("<html lang=\"qaa\" dir=\"rtl\">"), "qaa {uri}");
        assert_eq!(unfilled(&qaa), None, "qaa {uri}");
        assert!(
            qaa.contains("‹Proposals›") || name == "repo",
            "qaa {uri}: the nav is not from the catalog"
        );
        if let Some(dir) = &dump {
            std::fs::write(dir.join(format!("{name}-ja.html")), &ja).unwrap();
            std::fs::write(dir.join(format!("{name}-qaa.html")), &qaa).unwrap();
        }
    }

    // Closing from the page: a wrong token and a stranger are refused; the owner's form works and lands on the page.
    let uri = format!("/alice/forge/proposals/{id}/close");
    let r = form_as(&w, &uri, Some(&owner), "csrf=bogus&reason=x").await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let bob = session_for(&w, "bob");
    let r = form_as(&w, &uri, Some(&bob), "csrf=tok123&reason=because").await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", text(&r));
    assert!(
        text(&r).contains("only the person who made this proposal, or a maintainer, can close it")
    );
    let r = form_as(&w, &uri, Some(&owner), "csrf=tok123&reason=").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(&r).contains("A proposal is closed with a reason"));
    let r = form_as(
        &w,
        &uri,
        Some(&owner),
        "csrf=tok123&reason=Not+needed+any+more",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", text(&r));
    assert_eq!(r.h("location"), format!("/alice/forge/proposals/{id}"));
    let closed = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            Some(&owner),
            "en",
        )
        .await,
    );
    assert!(
        closed.contains("<span class=\"badge\">Closed</span>"),
        "{closed}"
    );
    assert!(
        closed.contains("Closed: it will not become part of the project&#39;s history.")
            || closed.contains("Closed: it will not become part of the project's history."),
        "{closed}"
    );
    assert!(closed.contains("Reason: Not needed any more"), "{closed}");
    assert!(
        !closed.contains("Close proposal"),
        "a closed proposal has no form"
    );
    assert_eq!(violations(&closed), Vec::<String>::new(), "{closed}");
    let r = form_as(&w, &uri, Some(&owner), "csrf=tok123&reason=again").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(&r).contains("already landed or been closed"));
    // The default list no longer shows it; "Closed" does.
    let l = text(&page_as(&w, "GET", "/alice/forge/proposals", None, "en").await);
    assert!(l.contains("There are no proposals to show."), "{l}");
    let l = text(&page_as(&w, "GET", "/alice/forge/proposals?state=closed", None, "en").await);
    assert!(l.contains("Two small changes"), "{l}");

    // A private repository: a stranger gets the same not-found page as for a repository that is not there.
    let stranger = session_for(&w, "carol");
    let hidden = page_as(&w, "GET", "/alice/private/proposals", Some(&stranger), "en").await;
    let absent = page_as(
        &w,
        "GET",
        "/alice/nothing-here/proposals",
        Some(&stranger),
        "en",
    )
    .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.status, absent.status);
}

/// `prikk log` shows the newest 20 blocks unless it is told otherwise; a patch sealed longer ago than that is still in the
/// history, and its proposal still lands.
#[tokio::test]
async fn a_patch_sealed_more_than_twenty_blocks_ago_still_counts_as_in_the_history() {
    if skip() {
        return;
    }
    let mut w = world("pr-old-block", Opts::default());
    let early = make_proposal(&mut w, "forge", "tok-alice", "Early", 1).await;
    let later = make_proposal_on(&mut w, "forge", "tok-alice", "Many after it", 25, false).await;
    let (early_claims, later_claims) = (claims_of(&w, &early), claims_of(&w, &later));
    // The second artifact carries the first one's claim again (the forge held it as pending); a claim is sealed once.
    let mut claims = early_claims.clone();
    claims.extend(
        later_claims
            .iter()
            .filter(|c| !early_claims.contains(c))
            .cloned(),
    );
    assert_eq!(claims.len(), 26);
    for c in &claims {
        seal_claim(&w, "forge", c);
    }
    let v = detail_of(&w, &early).await;
    assert_eq!(v["state"], "landed", "{v}");
    assert_eq!(v["patches"][0]["assurance"], "sealed");
    assert_eq!(detail_of(&w, &later).await["state"], "landed");
}

#[tokio::test]
async fn revisions_appear_in_order_and_a_push_without_a_list_is_labelled_so() {
    if skip() {
        return;
    }
    let mut w = world("pr-revisions", Opts::default());
    let id = make_proposal(&mut w, "forge", "tok-alice", "Grows", 1).await;
    // A second revision with no declaration (what a plain prikk push is): two more patches, and the proposal's set is
    // its first revision's declared patch plus these.
    w.client.commit(2);
    let art = w.client.artifact(&w.forge, "second");
    let second = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"proposal": id}),
        &art,
    )
    .await;
    assert_eq!(second.status, StatusCode::OK, "{}", text(&second));

    let v = detail_of(&w, &id).await;
    let revs = v["revisions"].as_array().unwrap();
    assert_eq!(
        revs.iter()
            .map(|r| r["seq"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        (revs[0]["declared"].clone(), revs[1]["declared"].clone()),
        (true.into(), false.into())
    );
    assert_eq!(
        (
            revs[0]["added_count"].clone(),
            revs[1]["added_count"].clone()
        ),
        (1.into(), 2.into())
    );
    assert_eq!(
        v["patch_total"], 3,
        "the first revision's declared patch and the two later ones"
    );
    let list = get_as(&w, "/api/v1/repos/alice/forge/proposals", None, &[])
        .await
        .json();
    assert_eq!(list["proposals"][0]["revision_count"], 2);
    assert_eq!(list["proposals"][0]["patch_count"], 3);

    // The page: both revisions, in order, and the undeclared one says so; the summary counts both.
    let p = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            None,
            "en",
        )
        .await,
    );
    assert!(p.contains("3 patches in 2 revisions"), "{p}");
    assert!(p.contains("Listed by the sender"), "{p}");
    assert!(
        p.contains("Not listed by the sender, so the patches may be incomplete"),
        "{p}"
    );
    assert!(
        p.find("Listed by the sender").unwrap() < p.find("Not listed by the sender").unwrap(),
        "revision 1 before revision 2"
    );
    assert_eq!(violations(&p), Vec::<String>::new());
    // Each revision has its own download for a reviewer.
    let owner = session_for(&w, "alice");
    let p = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            Some(&owner),
            "en",
        )
        .await,
    );
    for seq in [1, 2] {
        assert!(
            p.contains(&format!("/proposals/{id}/revisions/{seq}/artifact")),
            "{seq}"
        );
    }
}

/// The test language: right to left, every message of the English catalog wrapped in `‹ ›` (registered once).
fn register_qaa() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        use planeter_web::catalog::{Dir, Entry, LangDef, Plural, register_test_language, shipped};
        fn other_only(_: u64) -> Plural {
            Plural::Other
        }
        let entries: Vec<Entry> = shipped()[0]
            .entries
            .iter()
            .map(|e| {
                let text: &'static str = Box::leak(format!("‹{}›", e.other).into_boxed_str());
                Entry {
                    key: e.key,
                    one: text,
                    other: text,
                }
            })
            .collect();
        register_test_language(Box::leak(Box::new(LangDef {
            tag: "qaa",
            endonym: "Test language",
            dir: Dir::Rtl,
            plural: other_only,
            group_separator: ',',
            decimal_separator: '.',
            entries: Box::leak(entries.into_boxed_slice()),
        })));
    });
}

// ================================================================================================ the landing rule, corrected (D-40)

/// The transcript of part 5's open question 1, now pinned: the maintainer's sealed history is imported into the forge
/// (`archive import`, no `--adopt`): it lands as a **received** ref, `sync pending` still lists the patch, and the proposal
/// stays open, with the received ref shown as a sighting and nothing derived from it.
#[tokio::test]
async fn history_received_by_import_is_a_sighting_and_never_lands_the_proposal() {
    if skip() {
        return;
    }
    let mut w = world("pr-sighting", Opts::default());
    let id = make_proposal(&mut w, "forge", "tok-alice", "Seen elsewhere", 1).await;
    let archive = w.base.join("client-sighting.archive");
    prikk(
        &w.client.repo,
        &w.client.keys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    let forge_keys = w.base.join("forge-keys");
    let out = prikk(
        &w.forge,
        &forge_keys,
        &["archive", "import", archive.to_str().unwrap()],
    );
    assert!(out.contains("landed as remotes/heads/main"), "{out}");
    assert_eq!(
        pending(&w.forge).len(),
        1,
        "the import did not take the patch out of sync pending (prikk 0.54.1)"
    );

    let v = detail_of(&w, &id).await;
    assert_eq!(v["state"], "open", "{v}");
    assert_eq!(
        v["received_sightings"],
        serde_json::json!([{"ref": "remotes/heads/main"}])
    );
    assert_eq!(v["patches"][0]["assurance"], "forge-approved-unsealed");
    assert_eq!(v["still_pending"], 1);
    let landed = get_as(
        &w,
        "/api/v1/repos/alice/forge/proposals?state=landed",
        Some("tok-bob"),
        &[],
    )
    .await
    .json();
    assert!(landed["proposals"].as_array().unwrap().is_empty());

    // The page: the sighting is under Details only, with its explanation, and the default view is unchanged.
    let p = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            None,
            "en",
        )
        .await,
    );
    let (before, after) = p.split_once("<summary>Details</summary>").unwrap();
    assert!(!before.contains("received here"), "{before}");
    assert!(
        after.contains("Also found in history received here (not verified here): <code>remotes/heads/main</code>"),
        "{after}"
    );
    assert!(after.contains("can be replaced"), "{after}");
    assert!(before.contains("Held for review"), "{before}");
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");
    register_qaa();
    planeter_web::catalog::enable_preview_languages("ja").unwrap();
    let ja = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{id}"),
            None,
            "ja",
        )
        .await,
    );
    assert!(ja.contains("ここで受信した履歴にも見つかりました"), "{ja}");
    assert_eq!(violations(&ja), Vec::<String>::new(), "{ja}");
    assert_eq!(unfilled(&ja), None);
    let dump = std::env::var("PROPOSALS_HTML_DUMP").ok().map(PathBuf::from);
    if let Some(dir) = dump {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("detail-sighting-en.html"), &p).unwrap();
        std::fs::write(dir.join("detail-sighting-ja.html"), &ja).unwrap();
    }
    // Sealing on the forge itself (a key-holding action; it models "a key-holding action published it") lands it, and the
    // sighting is gone from the landed proposal.
    let claims = claims_of(&w, &id);
    seal_claim(&w, "forge", &claims[0]);
    let v = detail_of(&w, &id).await;
    assert_eq!(v["state"], "landed", "{v}");
    assert_eq!(v["patches"][0]["assurance"], "sealed");
    assert_eq!(v["received_sightings"], serde_json::json!([]));
}

#[tokio::test]
async fn a_closed_proposal_shows_its_reason_beside_the_state_cut_at_200_characters() {
    if skip() {
        return;
    }
    let mut w = world("pr-closed-line", Opts::default());
    let short = make_proposal(&mut w, "forge", "tok-alice", "Short reason", 1).await;
    let long = make_proposal_on(&mut w, "forge", "tok-alice", "Long reason", 1, false).await;
    let r = close_as(
        &w,
        "forge",
        &short,
        Some("tok-alice"),
        r#"{"reason":"<b>Not</b> needed & done"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", text(&r));
    let reason = format!("{}{}", "é".repeat(199), "TAIL-THAT-IS-CUT");
    let r = close_as(
        &w,
        "forge",
        &long,
        Some("tok-alice"),
        &serde_json::json!({"reason": reason}).to_string(),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", text(&r));

    let p = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{short}"),
            None,
            "en",
        )
        .await,
    );
    let (before, after) = p.split_once("<summary>Details</summary>").unwrap();
    assert!(
        before.contains("<p>Closed: &#60;b&#62;Not&#60;/b&#62; needed &#38; done</p>"),
        "escaped, in the default view: {before}"
    );
    assert!(!before.contains("<b>"), "{before}");
    assert!(
        after.contains("Reason: &#60;b&#62;Not&#60;/b&#62; needed &#38; done"),
        "{after}"
    );
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");

    let p = text(
        &page_as(
            &w,
            "GET",
            &format!("/alice/forge/proposals/{long}"),
            None,
            "en",
        )
        .await,
    );
    let (before, after) = p.split_once("<summary>Details</summary>").unwrap();
    let cut = format!("Closed: {}…", "é".repeat(199) + "T");
    assert!(
        before.contains(&cut),
        "cut at 200 characters with an ellipsis: {before}"
    );
    assert!(!before.contains("TAIL-THAT-IS-CUT"), "{before}");
    assert!(
        after.contains("TAIL-THAT-IS-CUT"),
        "the full text is under Details: {after}"
    );
    assert_eq!(violations(&p), Vec::<String>::new());
    let dump = std::env::var("PROPOSALS_HTML_DUMP").ok().map(PathBuf::from);
    if let Some(dir) = dump {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("detail-closed-en.html"), &p).unwrap();
        planeter_web::catalog::enable_preview_languages("ja").unwrap();
        let ja = text(
            &page_as(
                &w,
                "GET",
                &format!("/alice/forge/proposals/{long}"),
                None,
                "ja",
            )
            .await,
        );
        assert_eq!(violations(&ja), Vec::<String>::new());
        assert!(ja.contains("閉じています："));
        std::fs::write(dir.join("detail-closed-ja.html"), &ja).unwrap();
    }
}

/// D-40 for the propose route, as far as prikk 0.54.1 lets it be driven: `archive import` leaves the imported patches in
/// `sync pending` (it is what keeps a received block from clearing the queue), so a declared patch that the forge holds
/// only through received history is *held* by the queue, and the push is accepted with only the patch it added. (The check
/// still reads the logs of the received refs too, in case a later prikk stops doing that; on 0.54.1 removing them changes no
/// answer, which the part 5a review request reports as an equivalent mutation. And an answer never calls a patch signed in
/// on the strength of a received ref: `patches_in_logs` hands the response only the published refs.)
#[tokio::test]
async fn a_declared_patch_that_an_import_left_pending_is_held() {
    if skip() {
        return;
    }
    let mut w = world("pr-declared-received", Opts::default());
    w.client.commit(1);
    let first = w.client.patch_ids(1);
    // The maintainer's history, with that patch in it, was received here before the push.
    let archive = w.base.join("client-before.archive");
    prikk(
        &w.client.repo,
        &w.client.keys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    prikk(
        &w.forge,
        &w.base.join("forge-keys"),
        &["archive", "import", archive.to_str().unwrap()],
    );
    w.client.commit(1);
    let art = w.client.artifact(&w.forge, "after-received");
    let both = w.client.patch_ids(2);
    assert!(both.contains(&first[0]) && both.len() == 2);
    let r = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({
            "title": "On top of received history",
            "declared_patches": both,
            "client": "tests/1"
        }),
        &art,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", text(&r));
    let v = r.json();
    assert_eq!(v["declared"], true);
    assert_eq!(v["added_count"], 1, "only the patch this push added: {v}");
}
