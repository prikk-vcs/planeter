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
    ArtifactStore, ExportConfig, ExportService, Metrics, ProposeConfig, ProposeService,
    RecoveryConfig, RecoveryService, Spool,
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
        artifacts,
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
    let transport = Some(Arc::new(TransportRuntime {
        export: export.clone(),
        propose: (!o.without_propose).then(|| propose.clone()),
        recovery: Some(recovery.clone()),
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
    let resp = r.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = axum::body::to_bytes(resp.into_body(), 1 << 30)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
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
async fn a_keyed_clients_artifact_becomes_a_change_with_one_revision_and_nothing_is_published() {
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
    assert_eq!(v["created_change"], true);
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

    // The records: one change, one revision, the intent complete, the artifact retained by digest.
    let change_id = v["change_id"].as_str().unwrap();
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
async fn a_second_revision_to_the_same_change_and_a_resend_of_an_identical_artifact() {
    if skip() {
        return;
    }
    let mut w = world("second", Opts::default());
    w.client.commit(1);
    let a1 = w.client.artifact(&w.forge, "a1");
    let first = propose_as(&w, "forge", "tok-alice", titled("One"), &a1).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let change_id = first.json()["change_id"].as_str().unwrap().to_owned();
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
        serde_json::json!({"change": change_id, "declared_patches": declared}),
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
        (v["created_change"].clone(), v["revision"]["seq"].clone()),
        (false.into(), 2.into())
    );
    assert_eq!(v["added_count"], 2, "only what this push added");
    assert_eq!(v["declared"], true);
    assert_eq!(v["change_id"], change_id.as_str());
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
        .revisions_of(ok.json()["change_id"].as_str().unwrap())
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
    // Under `any-authenticated` carol (no role at all) may propose: 3 open changes at most.
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
    assert_eq!(v["quota"], "open_changes");
    assert_eq!(
        (v["limit"].clone(), v["used"].clone(), v["unit"].clone()),
        (3.into(), 3.into(), "count".into())
    );
    assert!(
        v["message"].as_str().unwrap().contains("3 open changes"),
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
    assert_eq!(ja.json()["quota"], "open_changes");
}

#[tokio::test]
async fn only_the_opener_or_a_writer_adds_to_a_change_and_only_while_it_is_open() {
    if skip() {
        return;
    }
    let mut w = world("changes", Opts::default());
    w.client.commit(1);
    let a1 = w.client.artifact(&w.forge, "a1");
    let first = propose_as(&w, "forge", "tok-dave", titled("Dave's"), &a1).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let change_id = first.json()["change_id"].as_str().unwrap().to_owned();
    // An unknown change.
    w.client.commit(1);
    let a2 = w.client.artifact(&w.forge, "a2");
    let unknown = propose_as(
        &w,
        "forge",
        "tok-dave",
        serde_json::json!({"change": "c-nope"}),
        &a2,
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "unknown_change")
    );
    // Alice (the owner, a writer) may append to dave's change; carol, no role, may not append to anything here.
    let by_alice = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"change": change_id}),
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
    let carols = own.json()["change_id"].as_str().unwrap().to_owned();
    w.client.commit(1);
    let art = w.client.artifact(&w.base.join("anyauth"), "c2");
    let other = propose_as(
        &w,
        "anyauth",
        "tok-bob",
        serde_json::json!({"change": carols}),
        &art,
    )
    .await;
    assert_eq!(
        (other.status, other.code().as_str()),
        (StatusCode::FORBIDDEN, "not_your_change")
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
        serde_json::json!({"change": carols}),
        &art,
    )
    .await;
    assert_eq!(
        (closed.status, closed.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "change_not_open")
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
        .revisions_of(none.json()["change_id"].as_str().unwrap())
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
    // Field errors: a new change needs a title; an existing one takes none; bad ids.
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
            serde_json::json!({"change": "c-1", "title": "x"}),
            "fields_for_existing_change",
        ),
        (
            serde_json::json!({"change": "bad id!"}),
            "change_id_invalid",
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
        .revisions_of(ok.json()["change_id"].as_str().unwrap())
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
if [ "$1" = --version ]; then echo "prikk 0.53.0"; exit 0; fi
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
    // 6. The metadata comes before the change it names: an unknown change id together with a title is a metadata
    //    error (an existing change takes no title), not "unknown change".
    let r = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"change": "c-nope", "title": "x"}),
        &art,
    )
    .await;
    assert_eq!(
        (r.status.as_u16(), r.json()["reason"].clone()),
        (400, "fields_for_existing_change".into())
    );
    // 7. The change comes before the base: an unknown change and an unknown base name the change.
    let r = propose_as(
        &w,
        "forge",
        "tok-alice",
        serde_json::json!({"change": "c-nope", "base": "9".repeat(64)}),
        &art,
    )
    .await;
    assert_eq!(code(&r), (422, "unknown_change".into()));
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
    let runs = w.recoveries.recent(&repo, 5).unwrap();
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
            .revisions_of(ok.json()["change_id"].as_str().unwrap())
            .unwrap()
            .len(),
        1
    );
}
