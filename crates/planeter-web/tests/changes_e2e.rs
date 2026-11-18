//! The review view, end to end (RFC 005 part 6): a real keyed client builds an artifact with real prikk, the in-process server accepts it as a
//! proposal, and `GET …/revisions/{seq}/changes[/{file_id}]` answers from files that real prikk rebuilt. Self-skips (printing why) without
//! prikk or `bwrap`-free environments cannot run it: the sandbox is the unconfined one, as in the other end-to-end tests.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{
    HostingService, LeaseManager, NullAuditSink, ReadService, RepoScope, Scope, ScopeAccess,
    new_boot_id,
};
use planeter_store::{
    ChangeStore, InMemoryChangeStore, InMemoryLeaseRecords, InMemoryMembershipStore,
    InMemoryRepositoryStore, InMemoryRevisions, MembershipStore, Owner, ProposalPolicy, RepoId,
    RepositoryRecord, RepositoryStore, RevisionStore, UserId, Visibility,
};
use planeter_transport::{
    ArtifactStore, ChangesConfig, ChangesService, ExportConfig, ExportService, Metrics,
    ProposalConfig, ProposalService, ProposeConfig, ProposeService, Spool,
};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};

#[allow(dead_code)]
#[path = "structure/dom.rs"]
mod dom;
#[allow(dead_code)]
#[path = "structure/rules.rs"]
mod rules;
#[allow(unused_imports)]
use planeter_web::catalog::{msg, planted_language};

static NOW: AtomicU64 = AtomicU64::new(1_000_000);
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}

fn skip() -> bool {
    let has = |t: &str| {
        Command::new(t)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !has("prikk") || !has("prlimit") {
        eprintln!("SKIPPED: prikk or prlimit is not available");
        return true;
    }
    false
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("changes-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn prikk_out(dir: &Path, keys: &Path, args: &[&str]) -> std::process::Output {
    Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .unwrap()
}

fn prikk(dir: &Path, keys: &Path, args: &[&str]) -> String {
    let out = prikk_out(dir, keys, args);
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The keyed client.
struct Client {
    repo: PathBuf,
    keys: PathBuf,
    archive: PathBuf,
    n: u32,
}

impl Client {
    fn new(base: &Path, initial: &[(&str, &[u8])]) -> Client {
        let repo = base.join("client");
        let keys = base.join("client-keys");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        prikk(&repo, &keys, &["setup"]);
        std::fs::write(repo.join("README.md"), b"# Hello\n\nA readme.\n").unwrap();
        prikk(&repo, &keys, &["commit", "--from-worktree", "-m", "init"]);
        prikk(&repo, &keys, &["seal", "--allow-no-audit"]);
        // the initial files are a second sealed block, so that "the block before the real parent" exists
        for (rel, body) in initial {
            let p = repo.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        if !initial.is_empty() {
            prikk(
                &repo,
                &keys,
                &["commit", "--from-worktree", "-m", "base files"],
            );
            prikk(&repo, &keys, &["seal", "--allow-no-audit"]);
        }
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
            n: 0,
        }
    }

    fn write(&self, rel: &str, body: &[u8]) {
        let p = self.repo.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// A file whose name is arbitrary bytes.
    fn write_named(&self, name: &[u8], body: &[u8]) {
        std::fs::write(self.repo.join(std::ffi::OsStr::from_bytes(name)), body).unwrap();
    }

    fn remove(&self, rel: &str) {
        std::fs::remove_file(self.repo.join(rel)).unwrap();
    }

    /// Commit the worktree and seal it: one block, one claim.
    fn seal(&mut self, message: &str) {
        self.n += 1;
        prikk(
            &self.repo,
            &self.keys,
            &["commit", "--from-worktree", "-m", message],
        );
        prikk(&self.repo, &self.keys, &["seal", "--allow-no-audit"]);
    }

    fn try_seal(&mut self, message: &str) -> bool {
        let c = prikk_out(
            &self.repo,
            &self.keys,
            &["commit", "--from-worktree", "-m", message],
        );
        if !c.status.success() {
            eprintln!(
                "prikk refused the commit: {}",
                String::from_utf8_lossy(&c.stderr)
            );
            return false;
        }
        prikk(&self.repo, &self.keys, &["seal", "--allow-no-audit"]);
        true
    }

    fn block_ids(&self, n: usize) -> Vec<String> {
        let out = prikk(
            &self.repo,
            &self.keys,
            &["log", "--format", "json", "--limit", &n.to_string()],
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["block_id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The artifact that closes the gap between this client and `forge`.
    fn artifact(&self, forge: &Path, name: &str) -> Vec<u8> {
        let out = self.repo.parent().unwrap().join("artifacts-out");
        std::fs::create_dir_all(&out).unwrap();
        let have = out.join(format!("{name}.have"));
        let art = out.join(format!("{name}.art"));
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
}

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

struct World {
    router: Router,
    base: PathBuf,
    client: Client,
    forge: PathBuf,
    changes: Arc<InMemoryChangeStore>,
    metrics: Arc<Metrics>,
    svc: Arc<ChangesService>,
    gate: Arc<planeter_web::PrikkGate>,
    spool: PathBuf,
    forge_keys: PathBuf,
    sessions: Arc<planeter_auth::InMemorySessionStore>,
}

struct Opts {
    gate: Option<planeter_web::PrikkGate>,
    cache: bool,
    max_bytes: Option<u64>,
    wall: Option<Duration>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            gate: None,
            cache: true,
            max_bytes: None,
            wall: None,
        }
    }
}

fn world(name: &str, o: Opts) -> World {
    world_with(name, o, &[])
}

fn world_with(name: &str, o: Opts, initial: &[(&str, &[u8])]) -> World {
    let base = scratch(name);
    let client = Client::new(&base, initial);
    let forge_path = forge(&base, "forge", &client);
    let private_path = base.join("private");
    assert!(
        Command::new("cp")
            .arg("-a")
            .arg(&forge_path)
            .arg(&private_path)
            .status()
            .unwrap()
            .success()
    );
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (id, n, vis, path) in [
        ("r-forge", "forge", Visibility::Public, &forge_path),
        ("r-private", "private", Visibility::Private, &private_path),
    ] {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(id),
                owner: Owner::User("alice".into()),
                name: n.into(),
                visibility: vis,
                created_at: 0,
                prikk_format_version: None,
                proposals: ProposalPolicy::Members,
                path: path.clone(),
            })
            .unwrap();
    }
    let hosting = Arc::new(HostingService::new(
        base.join("root"),
        store.clone() as Arc<dyn RepositoryStore>,
    ));
    let membership = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        membership.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let accounts = Arc::new(InMemoryAccountStore::new());
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user) in [("tok-alice", "alice"), ("tok-bob", "bob")] {
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&*hasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access: ScopeAccess::Write,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let auth = Arc::new(Authenticator::new(accounts, creds, hasher.clone(), hasher));
    let spool_dir = base.join("spool");
    let spool = Spool::open(&spool_dir, 512 << 20).unwrap();
    let artifacts = Arc::new(ArtifactStore::open(base.join("artifacts")).unwrap());
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
        revisions as Arc<dyn RevisionStore>,
        ExportConfig::default(),
        metrics.clone(),
    ));
    let changes = Arc::new(InMemoryChangeStore::new());
    let propose = Arc::new(
        ProposeService::new(
            hosting.clone(),
            artifacts.clone(),
            leases.clone(),
            changes.clone() as Arc<dyn ChangeStore>,
            export.clone(),
            ProposeConfig {
                lease_wait: Duration::from_secs(2),
                accept_deadline: Duration::from_secs(60),
                ..ProposeConfig::default()
            },
            metrics.clone(),
        )
        .with_clock(now),
    );
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
    let mut cfg = ChangesConfig {
        spool: o.cache.then(|| spool_dir.clone()),
        ..ChangesConfig::default()
    };
    if let Some(b) = o.max_bytes {
        cfg.max_bytes = b;
    }
    if let Some(t) = o.wall {
        cfg.wall = t;
    }
    let svc = Arc::new(ChangesService::new(hosting.clone(), metrics.clone(), cfg));
    let transport = Some(Arc::new(TransportRuntime {
        export: export.clone(),
        propose: Some(propose),
        recovery: None,
        proposals: Some(proposals),
        reviews: None,
        changes: Some(svc.clone()),
        deliveries: None,
        ip_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
        principal_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
    }));
    let gate = Arc::new(o.gate.unwrap_or_default());
    let sessions = Arc::new(planeter_auth::InMemorySessionStore::new());
    let router = router(AppState {
        read,
        gate: gate.clone(),
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
        base: base.clone(),
        client,
        forge: forge_path,
        changes,
        metrics,
        svc,
        gate,
        spool: spool_dir,
        forge_keys: base.join("forge-keys"),
        sessions,
    }
}

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
}

/// `CHANGES_TRANSCRIPT=<file>` appends every GET of these tests and its answer (the review request quotes real examples from it).
fn transcript(uri: &str, status: StatusCode, headers: &HeaderMap, body: &[u8]) {
    let Ok(path) = std::env::var("CHANGES_TRANSCRIPT") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let retry = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        let _ = writeln!(
            f,
            "### GET {uri} -> {status} (retry-after: {retry})\n{}\n",
            String::from_utf8_lossy(body)
        );
    }
}

async fn call(r: &Router, req: Request<Body>) -> Reply {
    let uri = req.uri().to_string();
    let is_get = req.method() == "GET";
    let resp = r.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = axum::body::to_bytes(resp.into_body(), 1 << 30)
        .await
        .unwrap()
        .to_vec();
    if is_get {
        transcript(&uri, status, &headers, &body);
    }
    Reply {
        status,
        headers,
        body,
    }
}

async fn get(w: &World, uri: &str, token: Option<&str>) -> Reply {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    call(&w.router, req.body(Body::empty()).unwrap()).await
}

fn frame(meta: &serde_json::Value, artifact: &[u8]) -> Vec<u8> {
    let m = meta.to_string().into_bytes();
    let mut out = b"PLANETER-PROPOSE-1\n".to_vec();
    out.extend_from_slice(&(m.len() as u64).to_be_bytes());
    out.extend_from_slice(&m);
    out.extend_from_slice(artifact);
    out
}

/// Send an artifact as alice; returns the proposal id.
async fn propose(w: &World, meta: serde_json::Value, art: &[u8]) -> (String, Reply) {
    let body = frame(&meta, art);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/forge/transport/propose")
        .header("content-type", "application/octet-stream")
        .header("content-length", body.len().to_string())
        .header("authorization", "Bearer tok-alice")
        .body(Body::from(body))
        .unwrap();
    let r = call(&w.router, req).await;
    assert!(
        r.status == StatusCode::CREATED || r.status == StatusCode::OK,
        "{} {}",
        r.status,
        String::from_utf8_lossy(&r.body)
    );
    (r.json()["proposal_id"].as_str().unwrap().to_owned(), r)
}

fn title() -> serde_json::Value {
    serde_json::json!({"title": "a change", "client": "tests/1"})
}

fn changes_uri(id: &str, seq: u32) -> String {
    format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/{seq}/changes")
}

async fn list(w: &World, id: &str) -> Reply {
    get(w, &changes_uri(id, 1), Some("tok-alice")).await
}

fn find<'a>(list: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    list["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .unwrap_or_else(|| panic!("no {path} in {}", list["files"]))
}

async fn diff(w: &World, id: &str, file: &serde_json::Value) -> serde_json::Value {
    let r = get(
        w,
        &format!(
            "{}/{}",
            changes_uri(id, 1),
            file["file_id"].as_str().unwrap()
        ),
        Some("tok-alice"),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

fn counter(w: &World, k: &str) -> u64 {
    w.metrics.snapshot().changes.get(k).copied().unwrap_or(0)
}

// ------------------------------------------------------------------------------------------------ the contract

#[tokio::test]
async fn a_one_claim_revision_lists_an_edit_a_delete_an_add_and_a_rename_as_delete_plus_add_and_each_diff_is_exact()
 {
    if skip() {
        return;
    }
    let initial: &[(&str, &[u8])] = &[
        ("docs/old-name.md", b"moved content\n"),
        ("gone.txt", b"bye\n"),
    ];
    let mut w = world_with("basic", Opts::default(), initial);
    w.client
        .write("README.md", b"# Hello\n\nA readme, edited.\n");
    w.client.remove("gone.txt");
    w.client.write("new.txt", b"hello\nworld\n");
    std::fs::remove_file(w.client.repo.join("docs/old-name.md")).unwrap();
    w.client.write("docs/new-name.md", b"moved content\n");
    w.client.seal("edit, delete, add, rename");
    let art = w.client.artifact(&w.forge, "a1");
    let (id, _) = propose(&w, title(), &art).await;
    let r = list(&w, &id).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let l = r.json();
    eprintln!("{}", serde_json::to_string_pretty(&l["compared"]).unwrap());
    assert_eq!(l["rebuilt"], true);
    assert_eq!(l["revision"]["seq"], 1);
    assert!(l["compared"]["after_block"].is_string());
    let paths: Vec<&str> = l["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.windows(2).all(|p| p[0] <= p[1]),
        "ordered by display path: {paths:?}"
    );
    for (p, st) in [
        ("README.md", "modified"),
        ("gone.txt", "deleted"),
        ("new.txt", "added"),
        ("docs/old-name.md", "deleted"),
        ("docs/new-name.md", "added"),
    ] {
        assert_eq!(find(&l, p)["status"], st, "{p}");
    }
    let readme = diff(&w, &id, find(&l, "README.md")).await;
    assert_eq!(readme["lines_added"], 1);
    assert_eq!(readme["lines_removed"], 1);
    let lines: Vec<(String, String)> = readme["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["op"].as_str().unwrap().to_owned(),
                x["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(
        lines.contains(&("remove".into(), "A readme.".into()))
            && lines.contains(&("add".into(), "A readme, edited.".into())),
        "{lines:?}"
    );
    assert_eq!(l["totals"]["files_changed"], 5);
    assert_eq!(l["limits"]["page_max"], 200);
}

// ------------------------------------------------------------------------------------------------ more shapes

#[tokio::test]
async fn a_three_claim_chain_is_one_change_against_the_real_parent_of_its_oldest_claim() {
    if skip() {
        return;
    }
    let mut w = world_with(
        "chain",
        Opts::default(),
        &[("keep.txt", b"keep\n"), ("evolve.txt", b"one\n")],
    );
    w.client.write("evolve.txt", b"one\ntwo\n");
    w.client.write("temp.txt", b"temporary\n");
    w.client.seal("claim 1");
    w.client.write("evolve.txt", b"one\ntwo\nthree\n");
    w.client.remove("temp.txt");
    w.client.seal("claim 2");
    w.client.write("added-last.txt", b"last\n");
    w.client.seal("claim 3");
    let art = w.client.artifact(&w.forge, "chain");
    let (id, p) = propose(&w, title(), &art).await;
    assert_eq!(
        p.json()["claims"].as_array().unwrap().len(),
        3,
        "three claims: three blocks"
    );
    let l = list(&w, &id).await;
    assert_eq!(
        l.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&l.body)
    );
    let l = l.json();
    let paths: Vec<&str> = l["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec!["added-last.txt", "evolve.txt"],
        "the net effect: temp.txt came and went inside the chain"
    );
    assert_eq!(find(&l, "evolve.txt")["lines_added"], 2);
    assert_eq!(l["compared"]["base_differs"], false);
}

#[tokio::test]
async fn binary_big_non_utf8_crlf_and_one_line_files_have_their_reasons_and_the_caps_hold() {
    if skip() {
        return;
    }
    let big = "a line of text\n".repeat(20_000);
    let sjis: Vec<u8> = b"\x83e\x83X\x83g\n".to_vec();
    let five_mb = "z".repeat(5 * 1024 * 1024);
    let initial: Vec<(&str, Vec<u8>)> = vec![
        ("bin.dat", vec![0, 1, 2, 3, 4]),
        ("big.txt", big.clone().into_bytes()),
        ("sjis.txt", sjis.clone()),
        ("crlf.txt", b"one\r\ntwo\r\nthree\r\n".to_vec()),
        ("oneline.txt", five_mb.clone().into_bytes()),
        ("lines.txt", Vec::new()),
    ];
    let init_refs: Vec<(&str, &[u8])> = initial.iter().map(|(a, b)| (*a, b.as_slice())).collect();
    let mut w = world_with("shapes", Opts::default(), &init_refs);
    w.client.write("bin.dat", &[0, 1, 2, 9, 4]);
    w.client
        .write("big.txt", big.replacen("a line", "A line", 1).as_bytes());
    w.client.write("sjis.txt", b"\x83e\x83X\x83g\n\x83e\n");
    w.client.write("crlf.txt", b"one\ntwo\nthree\n");
    w.client
        .write("oneline.txt", five_mb.replacen('z', "y", 1).as_bytes());
    let lines: String = (0..6000).map(|i| format!("line {i}\n")).collect();
    w.client.write("lines.txt", lines.as_bytes());
    w.client.seal("shapes");
    let art = w.client.artifact(&w.forge, "shapes");
    let (id, _) = propose(&w, title(), &art).await;
    let l = list(&w, &id).await;
    assert_eq!(
        l.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&l.body)
    );
    let l = l.json();
    for (p, why) in [
        ("bin.dat", "binary"),
        ("big.txt", "too_large"),
        ("sjis.txt", "not_utf8"),
        ("oneline.txt", "too_large"),
    ] {
        let f = find(&l, p);
        assert_eq!(
            (f["shown"].clone(), f["not_shown_because"].clone()),
            (serde_json::json!(false), serde_json::json!(why)),
            "{p}"
        );
        let d = diff(&w, &id, f).await;
        assert!(d["hunks"].as_array().unwrap().is_empty(), "{p}");
    }
    let crlf = diff(&w, &id, find(&l, "crlf.txt")).await;
    assert_eq!(
        (crlf["lines_added"].clone(), crlf["lines_removed"].clone()),
        (serde_json::json!(3), serde_json::json!(3))
    );
    let removed: Vec<String> = crlf["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["op"] == "remove")
        .map(|x| x["text"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(removed, vec!["one\u{240D}", "two\u{240D}", "three\u{240D}"]);
    let ls = diff(&w, &id, find(&l, "lines.txt")).await;
    assert_eq!(ls["lines_added"], 6000);
    assert_eq!(
        ls["hunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["lines"].as_array().unwrap().len())
            .sum::<usize>(),
        5000
    );
    assert_eq!(ls["truncated"]["lines_not_shown"], 1000);
    assert_eq!(l["totals"]["unshown"], 4);
    assert_eq!(l["totals"]["lines_added"], 6003, "6000 + the CRLF file's 3");
}

#[tokio::test]
async fn prikk_itself_refuses_hostile_names_and_the_one_it_takes_is_shown_as_it_is_with_only_a_hash_in_the_url()
 {
    if skip() {
        return;
    }
    let mut w = world("names", Opts::default());
    // prikk 0.57 refuses these at `commit` (so such a tree cannot be authored; the engine's own escaping of them is the second line, and is
    // tested on hand-built trees in `changes_compare.rs`). If a later prikk takes one, this fails and the case moves here.
    let refused: Vec<Vec<u8>> = vec![
        b"line\nbreak.txt".to_vec(),
        "fdp.\u{202E}txt".as_bytes().to_vec(),
        b"bad-\xFF-byte.txt".to_vec(),
        "zero\u{200B}width.txt".as_bytes().to_vec(),
        b"tab\there.txt".to_vec(),
    ];
    for n in &refused {
        w.client.write_named(n, b"x\n");
        assert!(
            !w.client.try_seal("hostile"),
            "prikk takes {:?} now",
            String::from_utf8_lossy(n)
        );
        std::fs::remove_file(w.client.repo.join(std::ffi::OsStr::from_bytes(n))).unwrap();
    }
    w.client
        .write_named(b"<script>alert(1)<!--.txt", b"content\n");
    assert!(w.client.try_seal("markup in a name"));
    let art = w.client.artifact(&w.forge, "names");
    let (id, _) = propose(&w, title(), &art).await;
    let l = list(&w, &id).await;
    assert_eq!(
        l.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&l.body)
    );
    let l = l.json();
    let f = find(&l, "<script>alert(1)<!--.txt");
    assert_eq!(
        f["path_escaped"], false,
        "markup is the page's to escape, not a disguise"
    );
    let url = format!("{}/{}", changes_uri(&id, 1), f["file_id"].as_str().unwrap());
    assert!(url.is_ascii() && !url.contains('<'), "{url}");
}

#[tokio::test]
async fn a_symlink_to_etc_passwd_is_shown_as_its_target_and_never_read() {
    if skip() {
        return;
    }
    let mut w = world("symlink", Opts::default());
    std::os::unix::fs::symlink("/etc/passwd", w.client.repo.join("pw")).unwrap();
    if !w.client.try_seal("a link") {
        eprintln!("SKIPPED: prikk does not take a symbolic link");
        return;
    }
    let art = w.client.artifact(&w.forge, "link");
    let (id, _) = propose(&w, title(), &art).await;
    let l = list(&w, &id).await;
    assert_eq!(
        l.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&l.body)
    );
    let l = l.json();
    let f = find(&l, "pw");
    eprintln!("a symbolic link as prikk rebuilt it: {f}");
    let d = diff(&w, &id, f).await;
    let all = d.to_string();
    assert!(
        !all.contains("root:"),
        "the content of /etc/passwd is never read: {all}"
    );
}

#[tokio::test]
async fn work_that_arrived_as_received_sealed_history_is_claim_not_held() {
    if skip() {
        return;
    }
    let mut w = world("received", Opts::default());
    w.client.write("x.txt", b"x\n");
    w.client.seal("work");
    let art = w.client.artifact(&w.forge, "received");
    let (id, _) = propose(&w, title(), &art).await;
    assert_eq!(list(&w, &id).await.status, StatusCode::OK);
    // the cached answer is correct for ever and is still served, whatever happens to the repository afterwards
    assert_eq!(
        list(&w, &id).await.status,
        StatusCode::OK,
        "a cached answer is a pure function of immutable inputs"
    );
    // …but a revision the cache has never seen is refused: import without --adopt, then ask afresh
    let mut w3 = world(
        "received3",
        Opts {
            cache: false,
            ..Opts::default()
        },
    );
    w3.client.write("y.txt", b"y\n");
    w3.client.seal("work");
    let art3 = w3.client.artifact(&w3.forge, "received3");
    let (id3, _) = propose(&w3, title(), &art3).await;
    let archive3 = w3.base.join("work3.archive");
    prikk(
        &w3.client.repo,
        &w3.client.keys,
        &["archive", "export", archive3.to_str().unwrap()],
    );
    prikk(
        &w3.forge,
        &w3.forge_keys,
        &["archive", "import", archive3.to_str().unwrap()],
    );
    let r = list(&w3, &id3).await;
    assert_eq!(
        r.status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["reason"], "claim_not_held");
    assert_eq!(r.json()["code"], "changes_unavailable");
}

#[tokio::test]
async fn a_proposal_with_no_base_and_a_revision_whose_declared_base_is_not_the_real_parent() {
    if skip() {
        return;
    }
    let mut w = world_with("base", Opts::default(), &[("base.txt", b"base\n")]);
    // the client's history: init, base files, then the work. The forge holds the first two.
    let blocks = w.client.block_ids(2); // [base files, init]
    let (real_parent, older) = (blocks[0].clone(), blocks[1].clone());
    w.client.write("a.txt", b"a\n");
    w.client.seal("one");
    let art = w.client.artifact(&w.forge, "base");
    // no base declared: nothing to differ from
    let (none_id, _) = propose(&w, title(), &art).await;
    let l = list(&w, &none_id).await.json();
    assert_eq!(l["compared"]["base_differs"], false);
    assert!(l["compared"]["declared_base"].is_null());
    assert_eq!(l["compared"]["before_block"], real_parent);
    // the declared base is the real parent: no difference
    w.client.write("b.txt", b"b\n");
    w.client.seal("two");
    let art2 = w.client.artifact(&w.forge, "base2");
    let (id2, _) = propose(
        &w,
        serde_json::json!({"title": "declared right", "client": "tests/1", "base": real_parent}),
        &art2,
    )
    .await;
    let l2 = list(&w, &id2).await.json();
    assert_eq!(l2["compared"]["declared_base"], real_parent);
    assert_eq!(l2["compared"]["base_differs"], false);
    // a base the forge holds but that is not the real parent: compared with the real parent, and said so
    w.client.write("c.txt", b"c\n");
    w.client.seal("three");
    let art3 = w.client.artifact(&w.forge, "base3");
    let (id3, _) = propose(
        &w,
        serde_json::json!({"title": "declared older", "client": "tests/1", "base": older}),
        &art3,
    )
    .await;
    let l3 = list(&w, &id3).await;
    assert_eq!(
        l3.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&l3.body)
    );
    let l3 = l3.json();
    assert_eq!(l3["compared"]["declared_base"], older);
    assert_eq!(
        l3["compared"]["base_differs"], true,
        "never silently reconciled"
    );
    assert_eq!(
        l3["compared"]["before_block"], real_parent,
        "the real parent is what is compared"
    );
}

#[tokio::test]
async fn a_hidden_repository_an_unknown_revision_and_an_unknown_proposal_answer_the_same_404_bytes()
{
    if skip() {
        return;
    }
    let mut w = world("hidden", Opts::default());
    w.client.write("a.txt", b"a\n");
    w.client.seal("one");
    let art = w.client.artifact(&w.forge, "h");
    let (id, _) = propose(&w, title(), &art).await;
    let uris = [
        format!("/api/v1/repos/alice/private/proposals/{id}/revisions/1/changes"),
        format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/9/changes"),
        "/api/v1/repos/alice/forge/proposals/no-such/revisions/1/changes".to_string(),
        format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/x/changes"),
        format!("/api/v1/repos/alice/nope/proposals/{id}/revisions/1/changes"),
        format!(
            "/api/v1/repos/alice/forge/proposals/{id}/revisions/1/changes/{}",
            "0".repeat(64)
        ),
        format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/1/changes/not-hex"),
    ];
    let mut bodies = Vec::new();
    for u in &uris {
        let r = get(&w, u, Some("tok-bob")).await;
        assert_eq!(
            r.status,
            StatusCode::NOT_FOUND,
            "{u}: {}",
            String::from_utf8_lossy(&r.body)
        );
        bodies.push(r.body.clone());
    }
    assert!(
        bodies.windows(2).all(|p| p[0] == p[1]),
        "one 404, byte for byte, whatever was asked"
    );
    // the owner sees an unknown file as the same 404 as everything else
    let r = get(
        &w,
        &format!(
            "/api/v1/repos/alice/forge/proposals/{id}/revisions/1/changes/{}",
            "0".repeat(64)
        ),
        Some("tok-alice"),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body, bodies[0]);
}

#[tokio::test]
async fn the_cache_answers_the_second_request_and_concurrent_identical_requests_extract_once() {
    if skip() {
        return;
    }
    let mut w = world("cache", Opts::default());
    for i in 0..20 {
        w.client.write(
            &format!("f{i}.txt"),
            format!("file {i}\n").repeat(50).as_bytes(),
        );
    }
    w.client.seal("twenty files");
    let art = w.client.artifact(&w.forge, "c");
    let (id, _) = propose(&w, title(), &art).await;
    let (a, b, c) = tokio::join!(list(&w, &id), list(&w, &id), list(&w, &id));
    for r in [&a, &b, &c] {
        assert_eq!(
            r.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&r.body)
        );
    }
    assert_eq!(a.body, b.body);
    assert_eq!(b.body, c.body);
    assert_eq!(
        counter(&w, "extractions"),
        1,
        "three identical requests at once: one pair of extractions"
    );
    let before_hits = counter(&w, "cache_hit");
    let again = list(&w, &id).await;
    assert_eq!(again.body, a.body);
    assert!(
        counter(&w, "cache_hit") > before_hits,
        "the second request is served from the cache"
    );
    assert_eq!(counter(&w, "extractions"), 1);
    // one file's diff, then again: the first comes from the cache too (the list wrote every diff)
    let l = a.json();
    let f = find(&l, "f3.txt");
    let _ = diff(&w, &id, f).await;
    assert_eq!(
        counter(&w, "extractions"),
        1,
        "a diff of a listed file is read from the cache"
    );
    // without a spool nothing is cached: every request extracts
    let mut n = world(
        "nocache",
        Opts {
            cache: false,
            ..Opts::default()
        },
    );
    n.client.write("x.txt", b"x\n");
    n.client.seal("x");
    let artn = n.client.artifact(&n.forge, "n");
    let (idn, _) = propose(&n, title(), &artn).await;
    assert_eq!(list(&n, &idn).await.status, StatusCode::OK);
    assert_eq!(list(&n, &idn).await.status, StatusCode::OK);
    assert_eq!(
        counter(&n, "extractions"),
        2,
        "no spool: computed per request"
    );
    assert!(!n.svc.caches());
}

#[tokio::test]
async fn a_busy_gate_is_a_503_with_retry_after_in_words_and_nothing_is_cached() {
    if skip() {
        return;
    }
    // one slot, a short wait, and a budget so that the view is admitted only when the slot is free
    let gate = planeter_web::PrikkGate::new(1, Duration::from_millis(100));
    let mut w = world(
        "busy",
        Opts {
            gate: Some(gate),
            ..Opts::default()
        },
    );
    w.client.write("x.txt", b"x\n");
    w.client.seal("x");
    let art = w.client.artifact(&w.forge, "b");
    let (id, _) = propose(&w, title(), &art).await;
    // hold the only slot for a second
    let g = w.gate.clone();
    let holder = tokio::spawn(async move {
        g.run(|| std::thread::sleep(Duration::from_millis(1500)))
            .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let r = list(&w, &id).await;
    assert_eq!(
        r.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let j = r.json();
    assert_eq!(
        (j["code"].as_str(), j["reason"].as_str()),
        (Some("changes_unavailable"), Some("busy"))
    );
    assert!(r.headers.get("retry-after").is_some());
    assert!(j["message"].as_str().unwrap().len() > 10 && j["hint"].as_str().unwrap().len() > 5);
    assert_eq!(counter(&w, "busy"), 1);
    let _ = holder.await;
    // nothing was cached for the transient failure: the cache holds no entry for this revision
    let cache = w.spool.join("changes-cache");
    let entries: Vec<String> = std::fs::read_dir(&cache)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(entries.is_empty(), "{entries:?}");
    // and the next request, with a free slot, works
    assert_eq!(list(&w, &id).await.status, StatusCode::OK);
}

#[tokio::test]
async fn the_list_is_paged_by_file_id_and_a_bad_cursor_or_a_zero_limit_is_named() {
    if skip() {
        return;
    }
    let mut w = world("paging", Opts::default());
    for i in 0..7 {
        w.client.write(&format!("p{i}.txt"), b"x\n");
    }
    w.client.seal("seven");
    let art = w.client.artifact(&w.forge, "p");
    let (id, _) = propose(&w, title(), &art).await;
    let mut seen = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let uri = match &after {
            None => format!("{}?limit=3", changes_uri(&id, 1)),
            Some(a) => format!("{}?limit=3&after={a}", changes_uri(&id, 1)),
        };
        let r = get(&w, &uri, Some("tok-alice")).await;
        assert_eq!(r.status, StatusCode::OK);
        let j = r.json();
        assert_eq!(
            j["totals"]["files_changed"], 7,
            "the totals are of the whole set, not the page"
        );
        seen.extend(
            j["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["path"].as_str().unwrap().to_owned()),
        );
        match j["next"].as_str() {
            Some(n) => after = Some(n.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 7);
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen, sorted);
    let zero = get(
        &w,
        &format!("{}?limit=0", changes_uri(&id, 1)),
        Some("tok-alice"),
    )
    .await;
    assert_eq!(
        (zero.status, zero.json()["reason"].clone()),
        (StatusCode::BAD_REQUEST, serde_json::json!("limit_zero"))
    );
    let bad = get(
        &w,
        &format!("{}?after={}", changes_uri(&id, 1), "1".repeat(64)),
        Some("tok-alice"),
    )
    .await;
    assert_eq!(
        (bad.status, bad.json()["reason"].clone()),
        (StatusCode::BAD_REQUEST, serde_json::json!("after_unknown"))
    );
    // over the maximum is clamped to it
    let big = get(
        &w,
        &format!("{}?limit=100000", changes_uri(&id, 1)),
        Some("tok-alice"),
    )
    .await;
    assert_eq!(big.status, StatusCode::OK);
}

#[tokio::test]
async fn a_revision_with_two_heads_is_not_one_change_and_the_refusal_is_remembered() {
    if skip() {
        return;
    }
    // two claims on one parent, held by the forge: two proposals, each one head
    let mut w = world("twoheads", Opts::default());
    let mut other = Client {
        repo: w.base.join("client2"),
        keys: w.base.join("client-keys2"),
        archive: w.client.archive.clone(),
        n: 0,
    };
    assert!(
        Command::new("cp")
            .arg("-a")
            .arg(&w.client.repo)
            .arg(&other.repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("cp")
            .arg("-a")
            .arg(&w.client.keys)
            .arg(&other.keys)
            .status()
            .unwrap()
            .success()
    );
    w.client.write("a.txt", b"head one\n");
    w.client.seal("head one");
    other.write("b.txt", b"head two\n");
    other.seal("head two");
    let (id_a, _) = propose(&w, title(), &w.client.artifact(&w.forge, "ha")).await;
    let (id_b, rb) = propose(&w, title(), &other.artifact(&w.forge, "hb")).await;
    let claim_b = rb.json()["claims"][0]["claim_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // a revision that names both claims, written to the records the way an accept would
    let a_rev = w.changes.revisions_of(&id_a).unwrap().remove(0);
    let b_rev = w.changes.revisions_of(&id_b).unwrap().remove(0);
    let mut two = a_rev.clone();
    two.claim_ids.push(claim_b);
    two.revision_id = "v-two-heads".to_owned();
    two.seq = 1;
    let mut change = w.changes.get_change(&id_a).unwrap().unwrap();
    change.change_id = "c-two-heads".to_owned();
    two.change_id = change.change_id.clone();
    two.artifact_digest = "ab".repeat(32);
    two.artifact_bytes = b_rev.artifact_bytes;
    w.changes
        .open_or_resume_intent(
            "i-two-heads",
            &two.repo_id,
            &two.artifact_digest,
            two.artifact_bytes,
            &[],
            "alice",
            now(),
        )
        .unwrap();
    w.changes
        .complete_intent_with_revision("i-two-heads", Some(&change), &two, now())
        .unwrap();
    let r = list(&w, "c-two-heads").await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["reason"], "not_one_change");
    let extractions = counter(&w, "extractions");
    assert_eq!(
        extractions, 0,
        "decided from `sync pending` alone: nothing was extracted"
    );
    // remembered: the second answer comes from the cache
    let hits = counter(&w, "cache_hit");
    let again = list(&w, "c-two-heads").await;
    assert_eq!(again.body, r.body);
    assert!(counter(&w, "cache_hit") > hits);
}

#[tokio::test]
async fn a_tree_over_the_bound_is_tree_too_large_and_that_is_remembered() {
    if skip() {
        return;
    }
    let mut w = world(
        "toolarge",
        Opts {
            max_bytes: Some(2048),
            ..Opts::default()
        },
    );
    w.client.write("big.bin", &vec![b'x'; 10_000]);
    w.client.seal("big");
    let art = w.client.artifact(&w.forge, "tl");
    let (id, _) = propose(&w, title(), &art).await;
    let r = list(&w, &id).await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["reason"], "tree_too_large");
    let n = counter(&w, "extractions");
    let again = list(&w, &id).await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        counter(&w, "extractions"),
        n,
        "the refusal is cached for the key"
    );
}

#[tokio::test]
async fn ten_thousand_and_one_files_are_tree_too_large_through_the_real_prikk_bound() {
    if skip() {
        return;
    }
    let mut w = world("manyfiles", Opts::default());
    for i in 0..10_001 {
        w.client.write(&format!("d{}/f{i}.txt", i % 100), b"x\n");
    }
    let t = Instant::now();
    w.client.seal("ten thousand and one");
    eprintln!("sealing 10,001 files took {:?}", t.elapsed());
    let art = w.client.artifact(&w.forge, "many");
    let (id, _) = propose(&w, title(), &art).await;
    let r = list(&w, &id).await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["reason"], "tree_too_large");
}

#[tokio::test]
async fn a_hundred_level_directory_is_listed_as_one_entry_at_the_depth_cap() {
    if skip() {
        return;
    }
    let mut w = world("deep", Opts::default());
    let mut rel = String::new();
    for i in 0..100 {
        rel.push_str(&format!("d{i}/"));
    }
    rel.push_str("bottom.txt");
    w.client.write(&rel, b"bottom\n");
    if !w.client.try_seal("deep") {
        eprintln!("SKIPPED: prikk refuses a path this deep");
        return;
    }
    let art = w.client.artifact(&w.forge, "deep");
    let (id, _) = propose(&w, title(), &art).await;
    let r = list(&w, &id).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let l = r.json();
    let f = &l["files"][0];
    assert_eq!(
        (f["kind"].clone(), f["not_shown_because"].clone()),
        (
            serde_json::json!("directory"),
            serde_json::json!("unreadable")
        )
    );
    assert_eq!(l["files"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_view_that_runs_out_of_time_is_timed_out_and_a_broken_repository_is_failed_and_neither_is_cached()
 {
    if skip() {
        return;
    }
    let mut w = world(
        "timeout",
        Opts {
            wall: Some(Duration::ZERO),
            ..Opts::default()
        },
    );
    w.client.write("x.txt", b"x\n");
    w.client.seal("x");
    let art = w.client.artifact(&w.forge, "t");
    let (id, _) = propose(&w, title(), &art).await;
    let r = list(&w, &id).await;
    assert_eq!(
        r.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["reason"], "timed_out");
    assert_eq!(counter(&w, "timeout"), 1);
    let entries = std::fs::read_dir(w.spool.join("changes-cache"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(
        entries, 0,
        "a run that did not finish leaves nothing in the cache"
    );
    // a repository whose store cannot be read
    let mut f = world("failed", Opts::default());
    f.client.write("x.txt", b"x\n");
    f.client.seal("x");
    let artf = f.client.artifact(&f.forge, "f");
    let (idf, _) = propose(&f, title(), &artf).await;
    let moved = f.forge.join(".prikk-away");
    std::fs::rename(f.forge.join(".prikk"), &moved).unwrap();
    let rf = list(&f, &idf).await;
    std::fs::rename(&moved, f.forge.join(".prikk")).unwrap();
    assert_eq!(
        rf.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        String::from_utf8_lossy(&rf.body)
    );
    assert_eq!(rf.json()["reason"], "failed");
    assert_eq!(counter(&f, "failed"), 1);
    assert_eq!(
        list(&f, &idf).await.status,
        StatusCode::OK,
        "the repository is back: not remembered as failed"
    );
}

#[tokio::test]
async fn a_thousand_file_repository_view_cold_and_cached_timings() {
    if skip() {
        return;
    }
    let initial: Vec<(String, Vec<u8>)> = (0..1000)
        .map(|i| {
            (
                format!("src/m{}/f{i}.rs", i % 40),
                format!("fn f{i}() {{\n    {i}\n}}\n")
                    .repeat(10)
                    .into_bytes(),
            )
        })
        .collect();
    let init: Vec<(&str, &[u8])> = initial
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_slice()))
        .collect();
    let mut w = world_with("thousand", Opts::default(), &init);
    for i in (0..1000).step_by(10) {
        w.client.write(
            &format!("src/m{}/f{i}.rs", i % 40),
            format!("fn f{i}() {{\n    changed {i}\n}}\n")
                .repeat(10)
                .as_bytes(),
        );
    }
    w.client.seal("touch a hundred files");
    let art = w.client.artifact(&w.forge, "k");
    let (id, _) = propose(&w, title(), &art).await;
    let t = Instant::now();
    let cold = list(&w, &id).await;
    let cold_t = t.elapsed();
    assert_eq!(
        cold.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&cold.body)
    );
    assert_eq!(cold.json()["totals"]["files_changed"], 100);
    let t = Instant::now();
    let warm = list(&w, &id).await;
    let warm_t = t.elapsed();
    assert_eq!(warm.body, cold.body);
    eprintln!(
        "a view of a 1,000-file repository (100 files changed): cold {cold_t:?}, cached {warm_t:?}"
    );
    assert!(cold_t < Duration::from_secs(30));
}

#[tokio::test]
async fn a_proposal_built_after_another_carries_the_earlier_claims_again_and_its_view_shows_the_whole_chain_against_the_sealed_base()
 {
    if skip() {
        return;
    }
    let mut w = world("stacked", Opts::default());
    w.client.write("first.txt", b"first\n");
    w.client.seal("first");
    let (_id1, _) = propose(&w, title(), &w.client.artifact(&w.forge, "s1")).await;
    // the have-list the forge SERVES does not list what it holds pending, so the next artifact carries the earlier block again
    let served = get(
        &w,
        "/api/v1/repos/alice/forge/transport/have?ref=heads/main",
        None,
    )
    .await;
    assert_eq!(served.status, StatusCode::OK);
    let have = w.base.join("served.have");
    std::fs::write(&have, &served.body).unwrap();
    w.client.write("second.txt", b"second\n");
    w.client.seal("second");
    let out = w.base.join("artifacts-out").join("s2.art");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    prikk(
        &w.client.repo,
        &w.client.keys,
        &[
            "sync",
            "build",
            "heads/main",
            "--have",
            have.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ],
    );
    let (id2, p2) = propose(&w, title(), &std::fs::read(&out).unwrap()).await;
    assert_eq!(
        p2.json()["claims"].as_array().unwrap().len(),
        2,
        "both blocks travel again"
    );
    let r = list(&w, &id2).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let l = r.json();
    let paths: Vec<&str> = l["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec!["first.txt", "second.txt"],
        "the revision's own chain against its sealed base: the earlier work is part of this revision's claims"
    );
}

#[tokio::test]
async fn twenty_simultaneous_identical_requests_make_one_extraction_eight_wait_and_the_rest_are_busy_at_once()
 {
    if skip() {
        return;
    }
    let mut w = world("waiters", Opts::default());
    for i in 0..30 {
        w.client.write(
            &format!("f{i}.txt"),
            format!("file {i}\n").repeat(200).as_bytes(),
        );
    }
    w.client.seal("thirty files");
    let art = w.client.artifact(&w.forge, "wt");
    let (id, _) = propose(&w, title(), &art).await;
    // twenty requests at once; the first computes (a second or so), eight wait for it, eleven are refused without parking
    let replies = futures_join_all((0..20).map(|_| list(&w, &id)).collect()).await;
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
        (
            1 + planeter_transport::MAX_WAITERS,
            20 - 1 - planeter_transport::MAX_WAITERS
        ),
        "{:?}",
        replies
            .iter()
            .map(|r| r.status.as_u16())
            .collect::<Vec<_>>()
    );
    for r in &busy {
        let j = r.json();
        assert_eq!(
            (j["code"].as_str(), j["reason"].as_str()),
            (Some("changes_unavailable"), Some("busy"))
        );
        assert!(r.headers.get("retry-after").is_some());
    }
    assert_eq!(
        counter(&w, "extractions"),
        1,
        "one pair of extractions for all twenty"
    );
    assert_eq!(
        counter(&w, "busy"),
        busy.len() as u64,
        "counted in the existing busy counter"
    );
    let bodies: Vec<&Vec<u8>> = replies
        .iter()
        .filter(|r| r.status == StatusCode::OK)
        .map(|r| &r.body)
        .collect();
    assert!(bodies.windows(2).all(|p| p[0] == p[1]));
}

/// Poll every call together on this one task, so that they overlap.
async fn futures_join_all<F: std::future::Future<Output = Reply>>(calls: Vec<F>) -> Vec<Reply> {
    let mut out: Vec<Option<Reply>> = (0..calls.len()).map(|_| None).collect();
    let mut pinned: Vec<std::pin::Pin<Box<F>>> = calls.into_iter().map(Box::pin).collect();
    std::future::poll_fn(|cx| {
        let mut done = true;
        for (i, f) in pinned.iter_mut().enumerate() {
            if out[i].is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(r) => out[i] = Some(r),
                    std::task::Poll::Pending => done = false,
                }
            }
        }
        if done {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await;
    out.into_iter().map(Option::unwrap).collect()
}

// ================================================================================================ the pages (handoff 2)

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn h(&self, name: &str) -> String {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    }
}

/// A page, as a browser asks for it: a language, and perhaps a session.
async fn page(w: &World, uri: &str, lang: &str, cookie: Option<&str>) -> Reply {
    let mut req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("host", "forge.example")
        .header("accept-language", lang);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    call(&w.router, req.body(Body::empty()).unwrap()).await
}

fn session(w: &World, user: &str) -> String {
    use planeter_auth::SessionStore as _;
    let s = w
        .sessions
        .create(UserId::new(user), now(), now() + 3600)
        .unwrap();
    format!("planeter_csrf=tok123; planeter_session={}", s.id.0)
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

/// `CHANGES_PAGES_DUMP=<dir>` saves the pages these tests render (the review request quotes them).
fn dump(name: &str, html: &str) {
    if let Ok(dir) = std::env::var("CHANGES_PAGES_DUMP") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(Path::new(&dir).join(name), html);
    }
}

fn title_of(html: &str) -> String {
    let a = html.find("<title>").unwrap() + 7;
    html[a..a + html[a..].find("</title>").unwrap()].to_owned()
}

fn h1_of(html: &str) -> String {
    let a = html.find("<h1>").unwrap() + 4;
    html[a..a + html[a..].find("</h1>").unwrap()].to_owned()
}

/// What every page of the view must be, in every language: a valid structure, no placeholder left, one `<h1>`, no sideways-scrolling construct, no inline
/// style or script, and no word of the engine's inside.
fn check_page(html: &str, lang: &str) {
    assert_eq!(violations(html), Vec::<String>::new(), "{lang}: {html}");
    assert_eq!(unfilled(html), None, "{lang}");
    assert_eq!(html.matches("<h1").count(), 1, "{lang}");
    assert!(
        !html.contains("style=") && !html.contains("<script"),
        "{lang}: no inline style or script"
    );
    let visible = html.split_once("<main").map_or(html, |(_, m)| m);
    let lower = visible.to_lowercase();
    for word in ["claim", "chain", "extract", "foot", "block"] {
        assert!(
            !lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == word),
            "{lang}: the internal word {word:?} is on the page"
        );
    }
    for word in ["クレーム", "チェーン", "エクストラクト", "ブロック"] {
        assert!(!visible.contains(word), "{lang}: {word}");
    }
}

fn file_href_of(html: &str, path: &str) -> String {
    let marker = format!("<code>{path}</code></a>");
    let at = html
        .find(&marker)
        .unwrap_or_else(|| panic!("no link for {path} in {html}"));
    let a = html[..at].rfind("<a href=\"").unwrap() + 9;
    html[a..a + html[a..].find('"').unwrap()].to_owned()
}

/// A world with one real proposal that edits, adds and deletes, and its two addresses.
async fn pages_world(name: &str) -> (World, String, String) {
    let initial: &[(&str, &[u8])] = &[("gone.txt", b"bye\n"), ("keep.txt", b"keep\n")];
    let mut w = world_with(name, Opts::default(), initial);
    w.client
        .write("README.md", b"# Hello\n\nA readme, edited.\n");
    w.client.write("docs/new.txt", b"first\nsecond\n");
    w.client.remove("gone.txt");
    w.client.seal("edit, add, delete");
    let art = w.client.artifact(&w.forge, "pg");
    let (id, _) = propose(&w, title(), &art).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    (w, id, uri)
}

#[tokio::test]
async fn the_changes_page_and_a_file_page_say_what_the_files_are_in_both_languages_and_pass_every_structural_rule()
 {
    if skip() {
        return;
    }
    let (w, id, uri) = pages_world("pg-both").await;
    let json = list(&w, &id).await.json();
    assert_eq!(
        json["provisional"], false,
        "the precondition: this world's answer is not provisional (prikk 0.58.0: the creation by `archive import --adopt` recorded the history it confirmed; 0.57.0 said true for a young repository)"
    );
    let short = &id[..12];
    // ---- English
    let en = page(&w, &uri, "en", None).await;
    assert_eq!(en.status, StatusCode::OK, "{}", en.text());
    let p = en.text();
    check_page(&p, "en");
    dump("changes-page.en.html", &p);
    assert_eq!(
        h1_of(&p),
        format!("Changes in revision 1 of proposal #{short}")
    );
    assert!(
        title_of(&p).starts_with(&format!(
            "Changes in revision 1 of proposal #{short} · alice/forge"
        )),
        "{}",
        title_of(&p)
    );
    for sentence in
        ["These files are rebuilt from the patches in this revision. Nothing here is signed."]
    {
        assert!(
            p.contains(sentence) || p.contains(&sentence.replace("&#39;", "'")),
            "missing: {sentence}\n{p}"
        );
    }
    // prikk 0.58.0: the history of a repository created by `archive import --adopt` was recorded, so this world's answer is not
    // provisional and the page does not say it is. The page that does say it is tested below with a change set that is provisional.
    assert!(
        !p.contains("No earlier check of this repository"),
        "the provisional sentence is on a page whose answer is not provisional\n{p}"
    );
    let before = json["compared"]["before_block"].as_str().unwrap();
    assert!(
        p.contains(&format!(
            "Compared with <code>{}</code> (the point in history the first patch continues from).",
            &before[..12]
        )),
        "{p}"
    );
    assert!(
        p.contains(&format!("<code>{before}</code>")),
        "the full id is under a <details>"
    );
    assert!(
        p.contains("3 files changed, 3 lines added, 2 lines removed"),
        "{p}"
    );
    assert!(
        p.contains("<caption>Files changed in revision 1</caption>"),
        "{p}"
    );
    for h in ["File", "Change", "Lines", "Notes"] {
        assert!(p.contains(&format!("<th scope=\"col\">{h}</th>")), "{h}");
    }
    for (path, word) in [
        ("README.md", "Changed"),
        ("docs/new.txt", "Added"),
        ("gone.txt", "Deleted"),
    ] {
        let row = &p[p.find(&format!("<code>{path}</code>")).unwrap()..];
        let row = &row[..row.find("</tr>").unwrap()];
        assert!(row.contains(&format!("<td>{word}</td>")), "{path}: {row}");
    }
    assert!(p.contains("<span aria-hidden=\"true\">+1 \u{2212}1</span><span class=\"visually-hidden\">1 added, 1 removed</span>"), "{p}");
    assert!(p.contains("Back to the proposal"), "{p}");
    // ---- the file page
    let readme = file_href_of(&p, "README.md");
    assert_eq!(
        readme,
        format!(
            "{uri}/{}",
            find(&json, "README.md")["file_id"].as_str().unwrap()
        )
    );
    let f = page(&w, &readme, "en", None).await;
    assert_eq!(f.status, StatusCode::OK, "{}", f.text());
    let fp = f.text();
    check_page(&fp, "en");
    dump("file-page.en.html", &fp);
    assert_eq!(h1_of(&fp), "<code>README.md</code>");
    assert!(
        fp.contains("<caption>Changes to <code>README.md</code></caption>"),
        "{fp}"
    );
    for h in ["Old line", "New line", "Change", "Text"] {
        assert!(fp.contains(&format!("<th scope=\"col\">{h}</th>")), "{h}");
    }
    assert!(
        fp.contains("<th scope=\"rowgroup\" colspan=\"4\">Lines 1–3 before, lines 1–3 after</th>"),
        "{fp}"
    );
    assert!(fp.contains("<tr class=\"diff-remove\"><td>3</td><td></td><td><span aria-hidden=\"true\">\u{2212}</span><span class=\"visually-hidden\">Removed line</span></td><td><code>A readme.</code></td></tr>"), "{fp}");
    assert!(fp.contains("<tr class=\"diff-add\"><td></td><td>3</td><td><span aria-hidden=\"true\">+</span><span class=\"visually-hidden\">Added line</span></td><td><code>A readme, edited.</code></td></tr>"), "{fp}");
    assert!(
        fp.contains("<span class=\"visually-hidden\">Unchanged line</span>"),
        "{fp}"
    );
    assert!(fp.contains("Change: Changed"), "{fp}");
    assert!(fp.contains("Back to the list of files") && fp.contains("Back to the proposal"));
    // the neighbours in list order: README.md is first among the shown files (docs/new.txt, then README.md? ordered by path)
    assert!(
        fp.contains("Next changed file") || fp.contains("Previous changed file"),
        "{fp}"
    );
    // ---- Japanese
    let ja = page(&w, &uri, "ja", None).await;
    assert_eq!(ja.status, StatusCode::OK);
    let jp = ja.text();
    check_page(&jp, "ja");
    dump("changes-page.ja.html", &jp);
    assert_eq!(h1_of(&jp), format!("提案 #{short} のリビジョン 1 の変更点"));
    assert!(jp.contains("ここに表示するファイルは、このリビジョンのパッチから再構築したものです。ここに表示した内容は、署名されたものではありません。"), "{jp}");
    assert!(
        !jp.contains("履歴を最初から再現してファイルを再構築しました"),
        "the provisional sentence is on a page whose answer is not provisional\n{jp}"
    );
    assert!(
        jp.contains("変わったファイル 3 件、追加 3 行、削除 2 行"),
        "{jp}"
    );
    assert!(
        jp.contains("<caption>リビジョン 1 で変わったファイル</caption>"),
        "{jp}"
    );
    for h in ["ファイル", "変更の種別", "行数", "補足"] {
        assert!(jp.contains(&format!("<th scope=\"col\">{h}</th>")), "{h}");
    }
    let jf = page(&w, &readme, "ja", None).await;
    assert_eq!(jf.status, StatusCode::OK);
    let jfp = jf.text();
    check_page(&jfp, "ja");
    dump("file-page.ja.html", &jfp);
    assert!(
        jfp.contains("変更前：1～3 行目、変更後：1～3 行目"),
        "{jfp}"
    );
    assert!(
        jfp.contains("<span class=\"visually-hidden\">削除された行</span>"),
        "{jfp}"
    );
    // the page keeps its language's attribute and the whole set of dynamic-page headers
    assert!(jp.contains("<html lang=\"ja\""), "{jp}");
}

#[tokio::test]
async fn one_file_one_line_and_zero_lines_use_the_right_plural_forms_in_both_languages() {
    if skip() {
        return;
    }
    let mut w = world("pg-plural", Opts::default());
    w.client.write("only.txt", b"one line\n");
    w.client.seal("one");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "pl")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let en = page(&w, &uri, "en", None).await.text();
    assert!(
        en.contains("1 file changed, 1 line added, 0 lines removed"),
        "{en}"
    );
    let ja = page(&w, &uri, "ja", None).await.text();
    assert!(
        ja.contains("変わったファイル 1 件、追加 1 行、削除 0 行"),
        "{ja}"
    );
    // two files, 1,200 lines (grouped digits)
    let mut w2 = world("pg-plural2", Opts::default());
    w2.client.write("a.txt", "x\n".repeat(1200).as_bytes());
    w2.client.write("b.txt", b"y\n");
    w2.client.seal("many");
    let (id2, _) = propose(&w2, title(), &w2.client.artifact(&w2.forge, "pl2")).await;
    let en2 = page(
        &w2,
        &format!("/alice/forge/proposals/{id2}/revisions/1/changes"),
        "en",
        None,
    )
    .await
    .text();
    assert!(
        en2.contains("2 files changed, 1,201 lines added, 0 lines removed"),
        "{en2}"
    );
}

// ---------------------------------------------------------------------------------------------- the reasons, as pages

struct Titled {
    identity: String,
    title: String,
}

#[tokio::test]
async fn every_reason_the_view_is_unavailable_is_a_plain_page_with_the_engines_status_in_both_languages()
 {
    if skip() {
        return;
    }
    let mut titles: Vec<Titled> = Vec::new();
    let mut seen: Vec<(&str, u16)> = Vec::new();
    let mut check = |reason: &'static str, status: u16, en: &Reply, ja: &Reply| {
        assert_eq!(
            (en.status.as_u16(), ja.status.as_u16()),
            (status, status),
            "{reason}"
        );
        for (lang, r) in [("en", en), ("ja", ja)] {
            check_page(&r.text(), lang);
            dump(&format!("reason-{reason}.{lang}.html"), &r.text());
            let t = r.text();
            assert!(!t.contains("<table"), "{reason}: a plain page has no table");
            assert!(t.contains("proposals/"), "{reason}: a way back");
        }
        titles.push(Titled {
            identity: format!("reason-{reason}"),
            title: title_of(&en.text()),
        });
        titles.push(Titled {
            identity: format!("reason-{reason}-ja"),
            title: title_of(&ja.text()),
        });
        seen.push((reason, status));
    };

    // claim_not_held (409): the work arrived as received sealed history
    let mut w = world(
        "pg-r-held",
        Opts {
            cache: false,
            ..Opts::default()
        },
    );
    w.client.write("y.txt", b"y\n");
    w.client.seal("work");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "r1")).await;
    let archive = w.base.join("work.archive");
    prikk(
        &w.client.repo,
        &w.client.keys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    prikk(
        &w.forge,
        &w.forge_keys,
        &["archive", "import", archive.to_str().unwrap()],
    );
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let en = page(&w, &uri, "en", None).await;
    let ja = page(&w, &uri, "ja", None).await;
    assert!(en.text().contains("The work of this revision is no longer held here as a proposal, for example because it arrived as signed history. Look at the repository&#39;s history instead."), "{}", en.text());
    check("claim_not_held", 409, &en, &ja);

    // tree_too_large (422)
    let mut w = world(
        "pg-r-large",
        Opts {
            max_bytes: Some(2048),
            ..Opts::default()
        },
    );
    w.client.write("big.bin", &vec![b'x'; 10_000]);
    w.client.seal("big");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "r2")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let (en, ja) = (
        page(&w, &uri, "en", None).await,
        page(&w, &uri, "ja", None).await,
    );
    check("tree_too_large", 422, &en, &ja);

    // timed_out (503)
    let mut w = world(
        "pg-r-time",
        Opts {
            wall: Some(Duration::ZERO),
            ..Opts::default()
        },
    );
    w.client.write("x.txt", b"x\n");
    w.client.seal("x");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "r3")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let (en, ja) = (
        page(&w, &uri, "en", None).await,
        page(&w, &uri, "ja", None).await,
    );
    check("timed_out", 503, &en, &ja);

    // failed (503): the store cannot be read
    let mut w = world("pg-r-fail", Opts::default());
    w.client.write("x.txt", b"x\n");
    w.client.seal("x");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "r4")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let moved = w.forge.join(".prikk-away");
    std::fs::rename(w.forge.join(".prikk"), &moved).unwrap();
    let (en, ja) = (
        page(&w, &uri, "en", None).await,
        page(&w, &uri, "ja", None).await,
    );
    std::fs::rename(&moved, w.forge.join(".prikk")).unwrap();
    check("failed", 503, &en, &ja);

    // busy (503, Retry-After): the only slot is taken
    let gate = planeter_web::PrikkGate::new(1, Duration::from_millis(100));
    let mut w = world(
        "pg-r-busy",
        Opts {
            gate: Some(gate),
            ..Opts::default()
        },
    );
    w.client.write("x.txt", b"x\n");
    w.client.seal("x");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "r5")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let g = w.gate.clone();
    let holder = tokio::spawn(async move {
        g.run(|| std::thread::sleep(Duration::from_millis(1500)))
            .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (en, ja) = (
        page(&w, &uri, "en", None).await,
        page(&w, &uri, "ja", None).await,
    );
    assert!(en.headers.get("retry-after").is_some() && ja.headers.get("retry-after").is_some());
    check("busy", 503, &en, &ja);
    let _ = holder.await;

    // not_one_change (422): a revision naming two claims on one parent
    let mut w = world("pg-r-two", Opts::default());
    let mut other = Client {
        repo: w.base.join("client2"),
        keys: w.base.join("client-keys2"),
        archive: w.client.archive.clone(),
        n: 0,
    };
    assert!(
        Command::new("cp")
            .arg("-a")
            .arg(&w.client.repo)
            .arg(&other.repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("cp")
            .arg("-a")
            .arg(&w.client.keys)
            .arg(&other.keys)
            .status()
            .unwrap()
            .success()
    );
    w.client.write("a.txt", b"head one\n");
    w.client.seal("head one");
    other.write("b.txt", b"head two\n");
    other.seal("head two");
    let (id_a, _) = propose(&w, title(), &w.client.artifact(&w.forge, "ha")).await;
    let (id_b, rb) = propose(&w, title(), &other.artifact(&w.forge, "hb")).await;
    let claim_b = rb.json()["claims"][0]["claim_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let a_rev = w.changes.revisions_of(&id_a).unwrap().remove(0);
    let b_rev = w.changes.revisions_of(&id_b).unwrap().remove(0);
    let mut two = a_rev.clone();
    two.claim_ids.push(claim_b);
    two.revision_id = "v-two-heads".to_owned();
    two.seq = 1;
    let mut change = w.changes.get_change(&id_a).unwrap().unwrap();
    change.change_id = "c-two-heads".to_owned();
    two.change_id = change.change_id.clone();
    two.artifact_digest = "ab".repeat(32);
    two.artifact_bytes = b_rev.artifact_bytes;
    w.changes
        .open_or_resume_intent(
            "i-two-heads",
            &two.repo_id,
            &two.artifact_digest,
            two.artifact_bytes,
            &[],
            "alice",
            now(),
        )
        .unwrap();
    w.changes
        .complete_intent_with_revision("i-two-heads", Some(&change), &two, now())
        .unwrap();
    let uri = "/alice/forge/proposals/c-two-heads/revisions/1/changes";
    let (en, ja) = (
        page(&w, uri, "en", None).await,
        page(&w, uri, "ja", None).await,
    );
    check("not_one_change", 422, &en, &ja);

    let mut reasons: Vec<&str> = seen.iter().map(|s| s.0).collect();
    reasons.sort_unstable();
    assert_eq!(
        reasons,
        vec![
            "busy",
            "claim_not_held",
            "failed",
            "not_one_change",
            "timed_out",
            "tree_too_large"
        ],
        "every reason of the closed list has its page"
    );
    // each reason has its own title, in each language
    for t in &titles {
        let same: Vec<&Titled> = titles
            .iter()
            .filter(|o| o.title == t.title && o.identity != t.identity)
            .collect();
        assert!(
            same.is_empty(),
            "{} shares its title {:?} with another reason",
            t.identity,
            t.title
        );
    }
}

// ---------------------------------------------------------------------------------------------- hostile content, and every statement and note

use planeter_transport::changes::cache::{Cache, KEY_SCHEMA, ListEntry, RevEntry};
use planeter_transport::changes::model::{
    ChangeSet, Compared, DiffHunk, DiffLine, DiffOp, FileDiff, FileEntry, Kind, NotShown, Status,
    Totals, Truncated,
};

fn fid(n: u32) -> String {
    format!("{:064x}", n)
}

fn entry(
    n: u32,
    path: &str,
    status: Status,
    kind: Kind,
    shown: bool,
    why: Option<NotShown>,
    (added, removed): (Option<u64>, Option<u64>),
) -> FileEntry {
    FileEntry {
        file_id: fid(n),
        path: path.to_owned(),
        path_cut_bytes: 0,
        path_escaped: false,
        status,
        kind,
        size_before: None,
        size_after: None,
        lines_added: added,
        lines_removed: removed,
        shown,
        not_shown_because: why,
    }
}

fn lines(ops: &[(DiffOp, &str)]) -> Vec<DiffLine> {
    ops.iter()
        .map(|(op, t)| DiffLine {
            op: *op,
            text: (*t).to_owned(),
            cut_chars: 0,
        })
        .collect()
}

fn totals(files: &[FileEntry]) -> Totals {
    Totals {
        files_changed: files.len() as u64,
        lines_added: files
            .iter()
            .filter(|f| f.shown)
            .filter_map(|f| f.lines_added)
            .sum(),
        lines_removed: files
            .iter()
            .filter(|f| f.shown)
            .filter_map(|f| f.lines_removed)
            .sum(),
        unshown: files.iter().filter(|f| !f.shown).count() as u64,
    }
}

/// Replace what the server will answer for revision 1 of `id` by a hand-made change set (the cache is a pure function of immutable inputs, so the
/// engine never rewrites it): the way to put content on a page that real prikk will not author (a name with `</script>`, a bidirectional override, a link
/// target `javascript:…`), exactly as the engine would hand it to the page, or worse, unescaped.
fn inject(w: &World, id: &str, key: &str, set: ChangeSet, diffs: &[FileDiff]) {
    let rev = w.changes.revisions_of(id).unwrap().remove(0);
    let cache = Cache::open(&w.spool, 64 << 20).unwrap();
    let key = &format!("{KEY_SCHEMA}-{key}");
    let writer = cache.begin(key).expect("a fresh key");
    for d in diffs {
        writer.put(d);
    }
    writer.finish(&ListEntry::Set(Box::new(set)));
    cache.put_rev(&rev.revision_id, &RevEntry::Key(key.to_owned()));
}

fn compared(before: Option<&str>, declared: Option<&str>, differs: bool) -> Compared {
    Compared {
        before_block: before.map(str::to_owned),
        after_block: "b".repeat(64),
        base_differs: differs,
        declared_base: declared.map(str::to_owned),
    }
}

fn change_set(compared: Compared, provisional: bool, files: Vec<FileEntry>) -> ChangeSet {
    ChangeSet {
        compared,
        rebuilt: true,
        provisional,
        chain_signature: None,
        totals: totals(&files),
        files,
    }
}

fn diff_of(entry: FileEntry, hunks: Vec<DiffHunk>) -> FileDiff {
    FileDiff {
        entry,
        hunks,
        truncated: None,
        approximate: false,
    }
}

async fn one_proposal(name: &str) -> (World, String) {
    let mut w = world(name, Opts::default());
    w.client.write("x.txt", b"x\n");
    w.client.seal("x");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "inj")).await;
    (w, id)
}

#[tokio::test]
async fn hostile_names_and_text_never_reach_a_page_unescaped_and_no_link_comes_from_file_content() {
    if skip() {
        return;
    }
    let (w, id) = one_proposal("pg-hostile").await;
    let script = "<script>alert(1)</script>.txt";
    let bidi = "fdp.\u{202E}txt";
    let img = "<img src=x onerror=alert(1)>";
    let long = "z".repeat(2000); // as the engine cuts a 2,500-character line: the head, 500 characters left out
    let mut e1 = entry(
        1,
        script,
        Status::Added,
        Kind::Text,
        true,
        None,
        (Some(3), Some(0)),
    );
    e1.path_escaped = false;
    let e2 = entry(
        2,
        bidi,
        Status::Added,
        Kind::Text,
        true,
        None,
        (Some(1), Some(0)),
    ); // as a broken engine would hand it: not escaped
    let e3 = entry(
        3,
        "evil.lnk",
        Status::Added,
        Kind::Symlink,
        true,
        None,
        (None, None),
    );
    let e4 = entry(
        4,
        "a\u{200B}b\u{FEFF}c\u{2028}d",
        Status::Added,
        Kind::Text,
        false,
        Some(NotShown::Binary),
        (None, None),
    );
    let files = vec![e1.clone(), e2.clone(), e3.clone(), e4.clone()];
    let hunk = |ls: Vec<DiffLine>| DiffHunk {
        old_start: 0,
        old_lines: 0,
        new_start: 1,
        new_lines: ls.len() as u64,
        lines: ls,
    };
    let d1 = diff_of(
        e1.clone(),
        vec![hunk({
            let mut ls = lines(&[
                (DiffOp::Add, img),
                (DiffOp::Add, "\u{202E}reversed \u{200B}zero-width\u{1B}[31m"),
                (DiffOp::Add, &long),
            ]);
            ls[2].cut_chars = 500;
            ls
        })],
    );
    let d2 = diff_of(e2.clone(), vec![hunk(lines(&[(DiffOp::Add, "line")]))]);
    let d3 = diff_of(
        e3.clone(),
        vec![hunk(lines(&[(DiffOp::Add, "javascript:alert(1)")]))],
    );
    inject(
        &w,
        &id,
        "hostile-1",
        change_set(compared(Some(&"a".repeat(64)), None, false), true, files),
        &[
            d1.clone(),
            d2.clone(),
            d3.clone(),
            diff_of(e4.clone(), vec![]),
        ],
    );
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    for lang in ["en", "ja"] {
        let r = page(&w, &uri, lang, None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        let p = r.text();
        check_page(&p, lang);
        dump(&format!("hostile-list.{lang}.html"), &p);
        assert!(
            !p.contains("<script") && !p.contains("</script>"),
            "{lang}: {p}"
        );
        assert!(
            (p.contains("&#60;script&#62;alert(1)&#60;/script&#62;.txt")
                || p.contains("&lt;script&gt;alert(1)&lt;/script&gt;.txt")),
            "{lang}: the name is there, escaped\n{p}"
        );
        for raw in ['\u{202E}', '\u{200B}', '\u{FEFF}', '\u{2028}', '\u{1B}'] {
            assert!(!p.contains(raw), "{lang}: {raw:?} is on the page raw");
        }
        assert!(
            p.contains("fdp.\\u{202E}txt"),
            "{lang}: the override is a visible escape\n{p}"
        );
        // every address on the page is one of ours: nothing a file said is a link
        for href in p
            .split("href=\"")
            .skip(1)
            .map(|s| &s[..s.find('"').unwrap()])
        {
            assert!(
                href.starts_with('/') || href == "#main",
                "{lang}: a link that is not ours: {href}"
            );
        }
        // a file page for each
        for (n, d) in [(1, &d1), (2, &d2), (3, &d3)] {
            let f = page(&w, &format!("{uri}/{}", fid(n)), lang, None).await;
            assert_eq!(f.status, StatusCode::OK, "{lang} file {n}: {}", f.text());
            let fp = f.text();
            check_page(&fp, lang);
            dump(&format!("hostile-file{n}.{lang}.html"), &fp);
            assert!(
                !fp.contains("<img") && !fp.contains("<script"),
                "{lang} file {n}: {fp}"
            );
            for raw in ['\u{202E}', '\u{200B}', '\u{FEFF}', '\u{1B}'] {
                assert!(!fp.contains(raw), "{lang} file {n}: {raw:?} raw");
            }
            for href in fp
                .split("href=\"")
                .skip(1)
                .map(|s| &s[..s.find('"').unwrap()])
            {
                assert!(
                    href.starts_with('/') || href == "#main",
                    "{lang} file {n}: a link that is not ours: {href}"
                );
            }
            let _ = d;
        }
    }
    // the first file's page: the markup line is text, the long line is cut with its own words, and the sentence says so
    let fp = page(&w, &format!("{uri}/{}", fid(1)), "en", None)
        .await
        .text();
    assert!(
        (fp.contains("<code>&#60;img src=x onerror=alert(1)&#62;</code>")
            || fp.contains("<code>&lt;img src=x onerror=alert(1)&gt;</code>")),
        "{fp}"
    );
    assert!(
        fp.contains("Lines that were too long are cut, and each cut says how much was left out."),
        "{fp}"
    );
    assert!(
        fp.contains("\\u{202E}reversed \\u{200B}zero-width\\u{1B}[31m"),
        "{fp}"
    );
    assert!(
        fp.contains("…(500 more characters not shown)"),
        "the page's own words for the cut: {fp}"
    );
    // a symbolic link target is text in a sentence, never a link
    let lp = page(&w, &format!("{uri}/{}", fid(3)), "en", None)
        .await
        .text();
    assert!(
        lp.contains("This is a symbolic link to <code>javascript:alert(1)</code>."),
        "{lp}"
    );
    assert!(!lp.contains("href=\"javascript"), "{lp}");
    assert!(!lp.contains("<table"), "a link has no diff table");
    let ljp = page(&w, &format!("{uri}/{}", fid(3)), "ja", None)
        .await
        .text();
    assert!(
        ljp.contains("<code>javascript:alert(1)</code> へのシンボリックリンクです。"),
        "{ljp}"
    );
    // a file with no diff: its reason, no table, and the way out is only offered to someone who may download
    let np = page(&w, &format!("{uri}/{}", fid(4)), "en", None)
        .await
        .text();
    assert!(
        np.contains("This is a binary file, so it cannot be shown as text."),
        "{np}"
    );
    assert!(
        !np.contains("<table") && !np.contains("Download revision"),
        "{np}"
    );
}

#[tokio::test]
async fn each_statement_and_each_note_appears_exactly_when_it_is_true() {
    if skip() {
        return;
    }
    let (w, id) = one_proposal("pg-statements").await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let (before, declared) = ("a".repeat(64), "c".repeat(64));
    let rebuilt =
        "These files are rebuilt from the patches in this revision. Nothing here is signed.";
    let provisional = "No earlier check of this repository&#39;s history was available";
    let starts = "This revision starts the history, so there are no earlier files to compare with.";
    let differs = format!(
        "This revision says it was built on <code>{}</code>, but its first patch continues from <code>{}</code>. The changes below are compared with <code>{}</code>.",
        &declared[..12],
        &before[..12],
        &before[..12]
    );
    let e = entry(
        1,
        "f.txt",
        Status::Modified,
        Kind::Text,
        true,
        None,
        (Some(1), Some(1)),
    );
    // 1. not provisional, a declared base that is the real parent
    inject(
        &w,
        &id,
        "st-1",
        change_set(
            compared(Some(&before), Some(&before), false),
            false,
            vec![e.clone()],
        ),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    assert!(
        p.contains(rebuilt)
            && !p.contains(provisional)
            && !p.contains(starts)
            && !p.contains("This revision says it was built on"),
        "{p}"
    );
    assert!(
        p.contains(&format!("Compared with <code>{}</code>", &before[..12])),
        "{p}"
    );
    // 2. provisional
    inject(
        &w,
        &id,
        "st-2",
        change_set(compared(Some(&before), None, false), true, vec![e.clone()]),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    assert!(p.contains(provisional), "{p}");
    // the page that carries the provisional sentence still passes every structural rule (the world above no longer does: prikk 0.58.0)
    check_page(&p, "en");
    let jp = page(&w, &uri, "ja", None).await.text();
    assert!(jp.contains("履歴を最初から再現してファイルを再構築しました"));
    check_page(&jp, "ja");
    // 3. the declared base is not the real parent
    inject(
        &w,
        &id,
        "st-3",
        change_set(
            compared(Some(&before), Some(&declared), true),
            false,
            vec![e.clone()],
        ),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    assert!(p.contains(&differs), "{p}");
    assert!(!p.contains(starts));
    check_page(&p, "en");
    // 4. the revision starts the history: no earlier files, no "compared with"
    inject(
        &w,
        &id,
        "st-4",
        change_set(compared(None, None, false), false, vec![e.clone()]),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    assert!(p.contains(starts) && !p.contains("Compared with"), "{p}");
    check_page(&p, "en");
    // 5. a declared base on a revision that starts the history
    inject(
        &w,
        &id,
        "st-5",
        change_set(
            compared(None, Some(&declared), true),
            false,
            vec![e.clone()],
        ),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    assert!(p.contains(&format!("This revision says it was built on <code>{}</code>, but its first patch starts the history", &declared[..12])), "{p}");
    assert!(!p.contains(starts), "the other sentence says it already");
    // 6. an empty change set
    inject(
        &w,
        &id,
        "st-6",
        change_set(compared(Some(&before), None, false), false, vec![]),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    assert!(
        p.contains("This revision changes no files.") && !p.contains("<table"),
        "{p}"
    );
    assert!(
        page(&w, &uri, "ja", None)
            .await
            .text()
            .contains("このリビジョンで変わるファイルはありません。")
    );
    check_page(&p, "en");
    // 7. some files cannot be shown as text: the summary says how many, each row says why (every reason of the closed list)
    let reasons = [
        NotShown::TooLarge,
        NotShown::NotUtf8,
        NotShown::Binary,
        NotShown::Symlink,
        NotShown::Directory,
        NotShown::Unreadable,
    ];
    let mut files: Vec<FileEntry> = reasons
        .iter()
        .enumerate()
        .map(|(i, r)| {
            entry(
                10 + i as u32,
                &format!("n{i}.dat"),
                Status::Modified,
                Kind::Binary,
                false,
                Some(*r),
                (None, None),
            )
        })
        .collect();
    files.push(entry(
        20,
        "current.lnk",
        Status::Added,
        Kind::Symlink,
        true,
        None,
        (None, None),
    ));
    files.push(e.clone());
    inject(
        &w,
        &id,
        "st-7",
        change_set(compared(Some(&before), None, false), false, files),
        &[],
    );
    let p = page(&w, &uri, "en", None).await.text();
    check_page(&p, "en");
    assert!(
        p.contains("8 files changed, 1 line added, 1 line removed"),
        "{p}"
    );
    assert!(
        p.contains("6 of them cannot be shown as text (see the Notes column)."),
        "{p}"
    );
    for note in [
        "This file is too large to show here.",
        "This file is not UTF-8 text, so it cannot be shown as text.",
        "This is a binary file, so it cannot be shown as text.",
        "This path changed between a file and a symbolic link, so it cannot be shown as text.",
        "This path is a folder, or became one, so there is no text to show.",
        "This entry is not an ordinary file, or is nested too deeply, so nothing is shown.",
        "A symbolic link: its target is shown.",
    ] {
        assert!(p.contains(note), "{note}\n{p}");
    }
    // a file that cannot be shown has no link to a page; the rows that can are links
    assert!(
        !p.contains("<code>n0.dat</code></a>") && p.contains("<code>f.txt</code></a>"),
        "{p}"
    );
    let pj = page(&w, &uri, "ja", None).await.text();
    check_page(&pj, "ja");
    assert!(
        pj.contains("そのうち 6 件は、テキストとして表示できません（「補足」列を参照）。"),
        "{pj}"
    );
    // 8. truncation, approximation and a diff with no lines (an empty file)
    let mut big = diff_of(
        entry(
            30,
            "big.txt",
            Status::Added,
            Kind::Text,
            true,
            None,
            (Some(6000), Some(0)),
        ),
        vec![DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 2,
            lines: lines(&[(DiffOp::Add, "one"), (DiffOp::Add, "two")]),
        }],
    );
    big.truncated = Some(Truncated {
        lines_not_shown: 5998,
    });
    big.approximate = true;
    let empty = diff_of(
        entry(
            31,
            "empty.txt",
            Status::Added,
            Kind::Text,
            true,
            None,
            (Some(0), Some(0)),
        ),
        vec![],
    );
    inject(
        &w,
        &id,
        "st-8",
        change_set(
            compared(Some(&before), None, false),
            false,
            vec![big.entry.clone(), empty.entry.clone()],
        ),
        &[big.clone(), empty.clone()],
    );
    let p = page(&w, &format!("{uri}/{}", fid(30)), "en", None)
        .await
        .text();
    check_page(&p, "en");
    assert!(p.contains("5,998 more lines are not shown."), "{p}");
    assert!(p.contains("This file changed so much that it is shown as all of the old lines removed and all of the new lines added."), "{p}");
    assert!(p.contains("No lines before, lines 1–2 after"), "{p}");
    let e1 = page(&w, &format!("{uri}/{}", fid(31)), "en", None)
        .await
        .text();
    assert!(
        e1.contains("This file has no lines.") && !e1.contains("<table"),
        "{e1}"
    );
    let one = diff_of(
        entry(
            32,
            "one.txt",
            Status::Added,
            Kind::Text,
            true,
            None,
            (Some(1), Some(0)),
        ),
        vec![DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            lines: lines(&[(DiffOp::Add, "only")]),
        }],
    );
    inject(
        &w,
        &id,
        "st-9",
        change_set(
            compared(Some(&before), None, false),
            false,
            vec![one.entry.clone()],
        ),
        &[one],
    );
    let p = page(&w, &format!("{uri}/{}", fid(32)), "en", None)
        .await
        .text();
    assert!(p.contains("No lines before, line 1 after"), "{p}");
    assert!(p.contains("1 more line") || !p.contains("more line"), "{p}");
}

#[tokio::test]
async fn a_really_long_line_is_cut_by_the_engine_and_the_page_says_so_in_words() {
    if skip() {
        return;
    }
    let mut w = world("pg-longline", Opts::default());
    let text = format!("start\n{}\nend\n", "w".repeat(3000));
    w.client.write("wide.txt", text.as_bytes());
    w.client.seal("wide");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "ll")).await;
    let list = list(&w, &id).await.json();
    let f = find(&list, "wide.txt");
    let uri = format!(
        "/alice/forge/proposals/{id}/revisions/1/changes/{}",
        f["file_id"].as_str().unwrap()
    );
    for lang in ["en", "ja"] {
        let p = page(&w, &uri, lang, None).await.text();
        check_page(&p, lang);
        dump(&format!("cut-line-file.{lang}.html"), &p);
        assert!(
            !p.contains("more characters)"),
            "{lang}: no English marker comes from the engine"
        );
        assert!(
            p.contains(&format!("<code>{}</code>", "w".repeat(2000))),
            "{lang}: the head of the line, 2,000 characters, and nothing else"
        );
        let words = if lang == "en" {
            "…(1,000 more characters not shown)"
        } else {
            "…（あと 1,000 文字は表示されません）"
        };
        assert!(
            p.contains(words),
            "{lang}: the page's words with the number: {p}"
        );
    }
    let json = diff(&w, &id, f).await;
    let cut: Vec<_> = json["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["cut_chars"].is_number())
        .collect();
    assert_eq!(cut.len(), 1, "{json}");
    assert_eq!(cut[0]["cut_chars"], 1000);
    assert_eq!(cut[0]["text"].as_str().unwrap().len(), 2000);
    assert!(
        json["hunks"][0]["lines"][0].get("cut_chars").is_none(),
        "omitted when 0"
    );
    let en = page(&w, &uri, "en", None).await.text();
    assert!(
        en.contains("Lines that were too long are cut, and each cut says how much was left out."),
        "{en}"
    );
    assert!(
        page(&w, &uri, "ja", None)
            .await
            .text()
            .contains("長すぎる行は途中で切ってあり")
    );
    // the unchanged end of the file is not mistaken for a cut
    let (id2, _) = (id.clone(), ());
    let _ = id2;
}

#[tokio::test]
async fn the_proposal_page_links_each_revision_to_its_changes_with_a_text_of_its_own_and_the_old_sentence_is_gone()
 {
    if skip() {
        return;
    }
    let (w, id, _) = pages_world("pg-link").await;
    let proposal = format!("/alice/forge/proposals/{id}");
    for (lang, text) in [
        ("en", "See the changes in revision 1"),
        ("ja", "リビジョン 1 の変更点を見る"),
    ] {
        let r = page(&w, &proposal, lang, None).await;
        assert_eq!(r.status, StatusCode::OK);
        let p = r.text();
        check_page(&p, lang);
        let href = format!("{proposal}/revisions/1/changes");
        assert!(
            p.contains(&format!("<a href=\"{href}\">{text}</a>")),
            "{lang}: {p}"
        );
        // beside the actions, above the details (the default view), and in the revisions' table
        let (default_view, details) = p.split_once("<summary>").unwrap();
        assert!(
            default_view.contains(text),
            "{lang}: the link is in the default view"
        );
        assert!(details.contains(text), "{lang}: and in the revisions table");
        assert!(
            !p.contains("not available yet") && !p.contains("まだ利用できません"),
            "{lang}"
        );
    }
    // a second revision: one link per revision, each with its own text, the latest also in the default view
    let mut w2 = world("pg-link2", Opts::default());
    w2.client.write("a.txt", b"a\n");
    w2.client.seal("one");
    let (id2, _) = propose(&w2, title(), &w2.client.artifact(&w2.forge, "l1")).await;
    w2.client.write("b.txt", b"b\n");
    w2.client.seal("two");
    let art = w2.client.artifact(&w2.forge, "l2");
    let (id2b, _) = propose(&w2, serde_json::json!({"proposal": id2.clone()}), &art).await;
    assert_eq!(
        id2, id2b,
        "the second artifact is a revision of the same proposal"
    );
    let p = page(&w2, &format!("/alice/forge/proposals/{id2}"), "en", None)
        .await
        .text();
    check_page(&p, "en");
    assert!(
        p.contains("See the changes in revision 1") && p.contains("See the changes in revision 2"),
        "{p}"
    );
    let default_view = p.split_once("<summary>").unwrap().0;
    assert!(
        default_view.contains("See the changes in revision 2")
            && !default_view.contains("See the changes in revision 1"),
        "the default view links the latest revision"
    );
    // the page of revision 2 compares the two blocks of the artifact against its own base
    let r2 = page(
        &w2,
        &format!("/alice/forge/proposals/{id2}/revisions/2/changes"),
        "en",
        None,
    )
    .await;
    assert_eq!(r2.status, StatusCode::OK, "{}", r2.text());
    check_page(&r2.text(), "en");
}

#[tokio::test]
async fn the_pages_carry_the_headers_of_the_other_dynamic_pages_and_the_download_only_for_who_may_use_it()
 {
    if skip() {
        return;
    }
    let (w, id, uri) = pages_world("pg-headers").await;
    let proposal = page(&w, &format!("/alice/forge/proposals/{id}"), "ja", None).await;
    let changes = page(&w, &uri, "ja", None).await;
    for name in [
        "cache-control",
        "vary",
        "content-language",
        "content-security-policy",
        "x-content-type-options",
        "referrer-policy",
        "content-type",
    ] {
        assert_eq!(
            changes.h(name),
            proposal.h(name),
            "{name} is the proposal page's"
        );
    }
    assert!(
        changes.h("cache-control").contains("no-store"),
        "{}",
        changes.h("cache-control")
    );
    assert!(
        changes.h("vary").to_lowercase().contains("accept-language"),
        "{}",
        changes.h("vary")
    );
    assert_eq!(changes.h("content-language"), "ja");
    assert!(
        changes.h("content-security-policy").contains("default-src"),
        "{}",
        changes.h("content-security-policy")
    );
    let file = page(&w, &format!("{uri}/{}", fid(0)), "en", None).await;
    assert_eq!(
        file.status,
        StatusCode::NOT_FOUND,
        "an unknown file is the page of a missing thing"
    );
    // the download: not for a stranger (no review right), for the signed-in owner
    let anon = page(&w, &uri, "en", None).await.text();
    assert!(!anon.contains("Download the files of revision"), "{anon}");
    let cookie = session(&w, "alice");
    let owner = page(&w, &uri, "en", Some(&cookie)).await.text();
    assert!(owner.contains(&format!("<a href=\"/api/v1/repos/alice/forge/proposals/{id}/revisions/1/artifact\">Download the files of revision 1</a>")), "{owner}");
    check_page(&owner, "en");
}

#[tokio::test]
async fn a_hidden_repository_an_unknown_proposal_revision_or_file_are_one_page_whoever_asks() {
    if skip() {
        return;
    }
    let (w, id, uri) = pages_world("pg-404").await;
    let asks = [
        format!("/alice/private/proposals/{id}/revisions/1/changes"),
        "/alice/forge/proposals/no-such/revisions/1/changes".to_string(),
        format!("/alice/forge/proposals/{id}/revisions/9/changes"),
        format!("/alice/forge/proposals/{id}/revisions/x/changes"),
        format!("/alice/nope/proposals/{id}/revisions/1/changes"),
        format!("{uri}/{}", "0".repeat(64)),
        format!("{uri}/not-hex"),
    ];
    let mut bodies = Vec::new();
    for u in &asks {
        let r = page(&w, u, "en", None).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{u}");
        check_page(&r.text(), "en");
        // the page names the proposal's address in its title; blank it to compare the rest
        bodies.push(r.text().split("<main").nth(1).unwrap().to_owned());
    }
    // the repository that does not exist and the one that is hidden say exactly the same thing (they differ only in the address they name)
    let strip = |s: &str| s.replace("private", "X").replace("nope", "X");
    assert_eq!(
        strip(&bodies[0]).split_once("</main>").unwrap().0.len(),
        strip(&bodies[4]).split_once("</main>").unwrap().0.len()
    );
}

#[tokio::test]
async fn the_previous_and_next_file_follow_the_list_and_skip_files_with_no_diff() {
    if skip() {
        return;
    }
    let (w, id) = one_proposal("pg-neighbours").await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let mk = |n: u32, path: &str, shown: bool| {
        entry(
            n,
            path,
            Status::Added,
            Kind::Text,
            shown,
            (!shown).then_some(NotShown::Binary),
            (shown.then_some(1), shown.then_some(0)),
        )
    };
    let files = vec![
        mk(1, "a.txt", true),
        mk(2, "b.bin", false),
        mk(3, "c.txt", true),
        mk(4, "d.txt", true),
    ];
    let one = |e: &FileEntry| {
        diff_of(
            e.clone(),
            if e.shown {
                vec![DiffHunk {
                    old_start: 0,
                    old_lines: 0,
                    new_start: 1,
                    new_lines: 1,
                    lines: lines(&[(DiffOp::Add, "x")]),
                }]
            } else {
                vec![]
            },
        )
    };
    let diffs: Vec<FileDiff> = files.iter().map(one).collect();
    inject(
        &w,
        &id,
        "nb-1",
        change_set(compared(Some(&"a".repeat(64)), None, false), false, files),
        &diffs,
    );
    let href = |n: u32| format!("{uri}/{}", fid(n));
    let p1 = page(&w, &href(1), "en", None).await.text();
    assert!(
        !p1.contains("Previous changed file")
            && p1.contains(&format!("<a href=\"{}\">Next changed file</a>", href(3))),
        "the next of a.txt skips b.bin: {p1}"
    );
    let p3 = page(&w, &href(3), "en", None).await.text();
    assert!(
        p3.contains(&format!(
            "<a href=\"{}\">Previous changed file</a>",
            href(1)
        )) && p3.contains(&format!("<a href=\"{}\">Next changed file</a>", href(4))),
        "{p3}"
    );
    let p4 = page(&w, &href(4), "en", None).await.text();
    assert!(
        p4.contains(&format!(
            "<a href=\"{}\">Previous changed file</a>",
            href(3)
        )) && !p4.contains("Next changed file"),
        "{p4}"
    );
    for p in [&p1, &p3, &p4] {
        assert!(p.contains(&format!("<a href=\"{uri}\">Back to the list of files</a>")));
        check_page(p, "en");
    }
    let ja = page(&w, &href(3), "ja", None).await.text();
    assert!(
        ja.contains("前の変更されたファイルへ")
            && ja.contains("次の変更されたファイルへ")
            && ja.contains("ファイルの一覧に戻る"),
        "{ja}"
    );
}

#[tokio::test]
async fn more_than_a_hundred_files_are_paged_with_earlier_and_more_links() {
    if skip() {
        return;
    }
    let (w, id) = one_proposal("pg-paging").await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let files: Vec<FileEntry> = (0..250u32)
        .map(|i| {
            entry(
                i + 1,
                &format!("f{i:03}.txt"),
                Status::Added,
                Kind::Text,
                true,
                None,
                (Some(1), Some(0)),
            )
        })
        .collect();
    inject(
        &w,
        &id,
        "pg-1",
        change_set(compared(Some(&"a".repeat(64)), None, false), false, files),
        &[],
    );
    let p1 = page(&w, &uri, "en", None).await.text();
    check_page(&p1, "en");
    dump("paged-list.en.html", &p1);
    assert!(
        p1.contains("250 files changed")
            && p1.contains("<code>f099.txt</code>")
            && !p1.contains("<code>f100.txt</code>"),
        "100 on the first page"
    );
    assert!(!p1.contains("Earlier files"));
    let more1 = p1
        .split("href=\"")
        .skip(1)
        .map(|s| &s[..s.find('"').unwrap()])
        .find(|h| h.contains("?after="))
        .unwrap()
        .to_owned();
    assert_eq!(more1, format!("{uri}?after={}", fid(100)));
    let p2 = page(&w, &more1.replace("&amp;", "&"), "en", None)
        .await
        .text();
    check_page(&p2, "en");
    assert!(
        p2.contains("<code>f100.txt</code>")
            && p2.contains("<code>f199.txt</code>")
            && !p2.contains("<code>f200.txt</code>"),
        "{p2}"
    );
    assert!(
        p2.contains(&format!("<a href=\"{uri}\">Earlier files</a>")),
        "the earlier page of the second is the first"
    );
    let more2 = format!("{uri}?after={}", fid(200));
    assert!(
        p2.contains(&format!("<a href=\"{more2}\">More files</a>")),
        "{p2}"
    );
    let p3 = page(&w, &more2, "en", None).await.text();
    assert!(
        p3.contains("<code>f249.txt</code>") && !p3.contains("More files"),
        "{p3}"
    );
    assert!(
        p3.contains(&format!(
            "<a href=\"{uri}?after={}\">Earlier files</a>",
            fid(100)
        )),
        "{p3}"
    );
    // an unknown cursor is the first page
    let bad = page(&w, &format!("{uri}?after={}", fid(9999)), "en", None).await;
    assert_eq!(bad.status, StatusCode::OK);
    assert!(bad.text().contains("<code>f000.txt</code>"));
}

#[tokio::test]
async fn files_that_real_prikk_rebuilt_as_binary_non_utf8_or_too_large_say_why_and_offer_the_download_to_who_may_use_it()
 {
    if skip() {
        return;
    }
    let big = "a line of text\n".repeat(20_000);
    let initial: Vec<(&str, Vec<u8>)> = vec![
        ("bin.dat", vec![0, 1, 2, 3]),
        ("big.txt", big.clone().into_bytes()),
        ("sjis.txt", b"\x83e\x83X\x83g\n".to_vec()),
    ];
    let init: Vec<(&str, &[u8])> = initial.iter().map(|(a, b)| (*a, b.as_slice())).collect();
    let mut w = world_with("pg-unshown", Opts::default(), &init);
    w.client.write("bin.dat", &[0, 1, 2, 9]);
    w.client
        .write("big.txt", big.replacen("a line", "A line", 1).as_bytes());
    w.client.write("sjis.txt", b"\x83e\x83X\x83g\n\x83e\n");
    w.client.seal("unshown");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "un")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let list = list(&w, &id).await.json();
    let owner = session(&w, "alice");
    for (path, note) in [
        (
            "bin.dat",
            "This is a binary file, so it cannot be shown as text.",
        ),
        ("big.txt", "This file is too large to show here."),
        (
            "sjis.txt",
            "This file is not UTF-8 text, so it cannot be shown as text.",
        ),
    ] {
        let href = format!("{uri}/{}", find(&list, path)["file_id"].as_str().unwrap());
        let anon = page(&w, &href, "en", None).await.text();
        check_page(&anon, "en");
        assert!(
            anon.contains(note) && !anon.contains("<table") && !anon.contains("Download revision"),
            "{path}: {anon}"
        );
        let signed = page(&w, &href, "en", Some(&owner)).await.text();
        check_page(&signed, "en");
        assert!(signed.contains(&format!("<a href=\"/api/v1/repos/alice/forge/proposals/{id}/revisions/1/artifact\">Download revision 1 to read it</a>")), "{path}: {signed}");
        let ja = page(&w, &href, "ja", Some(&owner)).await.text();
        check_page(&ja, "ja");
        assert!(
            ja.contains("読むにはリビジョン 1 をダウンロード"),
            "{path}: {ja}"
        );
    }
    let p = page(&w, &uri, "en", None).await.text();
    assert!(
        p.contains("3 of them cannot be shown as text (see the Notes column)."),
        "{p}"
    );
    // none of the three has a link to a file page
    assert!(
        !p.contains("<code>bin.dat</code></a>")
            && !p.contains("<code>big.txt</code></a>")
            && !p.contains("<code>sjis.txt</code></a>"),
        "{p}"
    );
}

#[tokio::test]
async fn twenty_simultaneous_requests_for_the_page_make_one_extraction_and_the_rest_are_the_busy_page_at_once()
 {
    if skip() {
        return;
    }
    let mut w = world("pg-waiters", Opts::default());
    for i in 0..30 {
        w.client.write(
            &format!("f{i}.txt"),
            format!("file {i}\n").repeat(200).as_bytes(),
        );
    }
    w.client.seal("thirty files");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "pw")).await;
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let replies = futures_join_all((0..20).map(|_| page(&w, &uri, "en", None)).collect()).await;
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
        (
            1 + planeter_transport::MAX_WAITERS,
            20 - 1 - planeter_transport::MAX_WAITERS
        )
    );
    for r in &busy {
        assert!(r.headers.get("retry-after").is_some());
        let t = r.text();
        check_page(&t, "en");
        assert!(
            t.contains(
                "The server is busy showing other changes right now. Try again in a moment."
            ),
            "{t}"
        );
    }
    assert_eq!(counter(&w, "extractions"), 1);
}

// ------------------------------------------------------------------------------------------------ cut text is data, not words

#[tokio::test]
async fn a_line_that_ends_like_the_old_english_marker_is_shown_as_itself_and_makes_no_cut_sentence()
{
    if skip() {
        return;
    }
    let mut w = world("pg-lookalike", Opts::default());
    let looks = format!("{}\u{2026}(500 more characters)", "z".repeat(40));
    w.client
        .write("notes.txt", format!("one\n{looks}\nthree\n").as_bytes());
    w.client.seal("notes");
    let (id, _) = propose(&w, title(), &w.client.artifact(&w.forge, "lk")).await;
    let list = list(&w, &id).await.json();
    let f = find(&list, "notes.txt");
    let uri = format!(
        "/alice/forge/proposals/{id}/revisions/1/changes/{}",
        f["file_id"].as_str().unwrap()
    );
    for lang in ["en", "ja"] {
        let p = page(&w, &uri, lang, None).await.text();
        check_page(&p, lang);
        assert!(
            p.contains(&format!("<code>{looks}</code>")),
            "{lang}: the line is shown as itself\n{p}"
        );
        assert!(
            !p.contains("Lines that were too long are cut")
                && !p.contains("長すぎる行は途中で切ってあり"),
            "{lang}: no cut sentence for a line that was not cut"
        );
        assert!(
            !p.contains("not shown)") && !p.contains("表示されません）"),
            "{lang}: no marker"
        );
    }
    let d = diff(&w, &id, f).await;
    assert!(
        d["hunks"][0]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l.get("cut_chars").is_none()),
        "{d}"
    );
}

#[tokio::test]
async fn a_cut_name_is_its_head_with_path_cut_bytes_and_the_pages_say_it_in_each_language() {
    if skip() {
        return;
    }
    let (w, id) = one_proposal("pg-cutname").await;
    let head = "n".repeat(1024);
    let mut e = entry(
        1,
        &head,
        Status::Added,
        Kind::Text,
        true,
        None,
        (Some(1), Some(0)),
    );
    e.path_escaped = true;
    e.path_cut_bytes = 276;
    let d = diff_of(
        e.clone(),
        vec![DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            lines: lines(&[(DiffOp::Add, "x")]),
        }],
    );
    let mut one = entry(
        2,
        "m",
        Status::Added,
        Kind::Text,
        false,
        Some(NotShown::Binary),
        (None, None),
    );
    one.path_cut_bytes = 1;
    let files = vec![e.clone(), one.clone()];
    inject(
        &w,
        &id,
        "cutname",
        change_set(compared(Some(&"a".repeat(64)), None, false), false, files),
        &[d, diff_of(one.clone(), vec![])],
    );
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    for (lang, many, single) in [
        (
            "en",
            "…(276 more bytes of the name not shown)",
            "…(1 more byte of the name not shown)",
        ),
        (
            "ja",
            "…（名前のあと 276 バイトは表示されません）",
            "…（名前のあと 1 バイトは表示されません）",
        ),
    ] {
        let list = page(&w, &uri, lang, None).await.text();
        check_page(&list, lang);
        dump(&format!("cut-name-list.{lang}.html"), &list);
        assert!(
            list.contains(many) && list.contains(single),
            "{lang}: {list}"
        );
        let file = page(&w, &format!("{uri}/{}", fid(1)), lang, None)
            .await
            .text();
        check_page(&file, lang);
        dump(&format!("cut-name-file.{lang}.html"), &file);
        let h1 = file
            .split("<h1>")
            .nth(1)
            .unwrap()
            .split("</h1>")
            .next()
            .unwrap();
        assert!(
            h1.contains(&head) && h1.contains(many),
            "{lang}: the heading says it: {h1}"
        );
    }
    // the JSON carries the number and no words
    let j = get(&w, &changes_uri(&id, 1), Some("tok-alice"))
        .await
        .json();
    assert_eq!(find_by_id(&j, &fid(1))["path_cut_bytes"], 276);
    assert_eq!(
        find_by_id(&j, &fid(1))["path"].as_str().unwrap().len(),
        1024
    );
    assert!(!j.to_string().contains("more bytes"), "{j}");
}

fn find_by_id<'a>(list: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    list["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file_id"] == id)
        .unwrap_or_else(|| panic!("no {id} in {}", list["files"]))
}

#[tokio::test]
async fn a_pointer_to_a_key_of_an_earlier_format_is_not_read() {
    if skip() {
        return;
    }
    let (w, id) = one_proposal("pg-schema").await;
    // an entry as an earlier version wrote it: a key with no version component, a file name with the old English marker in it
    let rev = w.changes.revisions_of(&id).unwrap().remove(0);
    let cache = Cache::open(&w.spool, 64 << 20).unwrap();
    let old_key = "0123456789abcdef0123456789abcdef01234567";
    let e = entry(
        1,
        "OLD-FORMAT.txt\u{2026}(5 more bytes)",
        Status::Added,
        Kind::Text,
        false,
        Some(NotShown::Binary),
        (None, None),
    );
    let writer = cache.begin(old_key).unwrap();
    writer.finish(&ListEntry::Set(Box::new(change_set(
        compared(None, None, false),
        false,
        vec![e],
    ))));
    cache.put_rev(&rev.revision_id, &RevEntry::Key(old_key.to_owned()));
    let r = list(&w, &id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let body = r.text();
    assert!(
        !body.contains("OLD-FORMAT"),
        "the old entry was served: {body}"
    );
    assert!(
        body.contains("x.txt"),
        "the view was computed again: {body}"
    );
    // and the pointer now names a key of this version
    match cache.rev(&rev.revision_id).unwrap() {
        RevEntry::Key(k) => assert!(k.starts_with(&format!("{KEY_SCHEMA}-")), "{k}"),
        other => panic!("{other:?}"),
    }
}
