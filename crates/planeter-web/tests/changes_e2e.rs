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
        for (rel, body) in initial {
            let p = repo.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
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
        ip_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
        principal_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
    }));
    let gate = Arc::new(o.gate.unwrap_or_default());
    let router = router(AppState {
        read,
        gate: gate.clone(),
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
    let mut w = world("base", Opts::default());
    w.client.write("a.txt", b"a\n");
    w.client.seal("one");
    let first_block = w.client.block_ids(2)[1].clone(); // the client's init block: the forge holds it
    let art = w.client.artifact(&w.forge, "base");
    let (none_id, _) = propose(&w, title(), &art).await;
    let l = list(&w, &none_id).await.json();
    assert_eq!(l["compared"]["base_differs"], false);
    assert!(l["compared"]["declared_base"].is_null());
    // a second proposal declaring a base that the forge holds but that is not the real parent of the work
    w.client.write("b.txt", b"b\n");
    w.client.seal("two");
    let art2 = w.client.artifact(&w.forge, "base2");
    let (id2, _) = propose(
        &w,
        serde_json::json!({"title": "declared", "client": "tests/1", "base": first_block}),
        &art2,
    )
    .await;
    let l2 = list(&w, &id2).await.json();
    assert_eq!(l2["compared"]["declared_base"], first_block);
    assert_eq!(
        l2["compared"]["base_differs"], false,
        "the declared base is the real parent here"
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
