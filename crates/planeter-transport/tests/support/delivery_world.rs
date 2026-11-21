//! The world of the delivery tests (RFC 008 handoff 1): **real prikk**, real archives of the measurement's cases, a hosted repository
//! made as creation makes one, proposals made through the propose service, and the delivery engine over on-disk SQLite stores (so a
//! second process, or a restart, sees what the first one did). Included by path. Nothing here is part of the product.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use planeter_core::{HostingService, LeaseManager, new_boot_id};
use planeter_prikk::{PrikkRepo, Sandbox};
use planeter_store::{
    ChangeStore, DeliveryStore, InMemoryRepositoryStore, Owner, ProposalPolicy, RepoId,
    RepositoryRecord, RepositoryStore, RevisionStore, SqliteChangeStore, SqliteDb,
    SqliteDeliveryStore, SqliteLeaseRecords, SqliteRevisions, UserId, Visibility,
};
use planeter_transport::delivery::{DeliveryConfig, DeliveryService};
use planeter_transport::{
    ArtifactStore, ExportConfig, ExportService, Metrics, ProposeConfig, ProposeMeta,
    ProposeService, RecoveryConfig, RecoveryService, Spool, Submitter,
};

use crate::archives::{KeyInfo, Maintainer, run};

/// A script that was just written can meet ETXTBSY ("text file busy") when another test thread's fork still holds the write handle for a moment:
/// run it until it runs (the project's pattern: `proposals_unit.rs`, `extract.rs`). Any other error is a real failure.
pub fn wait_until_runs(path: &std::path::Path) {
    for _ in 0..200 {
        match std::process::Command::new(path).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(e) => panic!("{}: {e}", path.display()),
            Ok(_) => break,
        }
    }
}

pub fn real_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The test binary's own name: the directories of a run carry it (not a process id), so that the next run replaces them
/// instead of leaving them behind.
fn binary_tag() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("delivery-{name}-{}", binary_tag()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn prikk_ok() -> bool {
    crate::archives::prikk_available()
}

// ------------------------------------------------------------------------------------------------ keyed workers

pub fn copy_dir(from: &Path, to: &Path) {
    let st = Command::new("cp")
        .arg("-a")
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(st.success());
}

/// The maintainer key of a keyed worker (its trust list's first key: `setup` trusts its own).
pub fn key_of(m: &Maintainer) -> KeyInfo {
    let out = run(
        &m.work,
        &m.keys,
        &["trust", "maintainer", "list", "--format", "json"],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let k = &v["keys"][0];
    KeyInfo {
        key_id: k["key_id"].as_str().unwrap().to_owned(),
        public_key: k["public_key"].as_str().unwrap().to_owned(),
    }
}

/// A copy of a worker (its repository and its keys): the same key, the same history so far.
pub fn clone_of(src: &Maintainer, base: &Path, name: &str) -> Maintainer {
    let work = base.join(name);
    let keys = base.join(format!("{name}-keys"));
    copy_dir(&src.work, &work);
    copy_dir(&src.keys, &keys);
    Maintainer { work, keys }
}

/// A worker with **the keys of `like`** and a repository of its own, empty (no block).
pub fn empty_with_keys_of(like: &Maintainer, base: &Path, name: &str) -> Maintainer {
    let work = base.join(name);
    let keys = base.join(format!("{name}-keys"));
    std::fs::create_dir_all(&work).unwrap();
    copy_dir(&like.keys, &keys);
    run(&work, &keys, &["setup", "."]);
    Maintainer { work, keys }
}

pub fn adopt(m: &Maintainer, k: &KeyInfo) {
    run(
        &m.work,
        &m.keys,
        &[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            &k.key_id,
            "--public-key",
            &k.public_key,
        ],
    );
}

pub fn import(m: &Maintainer, archive: &Path) {
    run(
        &m.work,
        &m.keys,
        &["archive", "import", &archive.to_string_lossy(), "--adopt"],
    );
}

/// One sealed block: a file, a commit, a seal.
pub fn step(m: &Maintainer, tag: &str) {
    m.write(
        &format!("src/{tag}.txt"),
        &format!("content of {tag}\n").repeat(8),
    );
    m.commit(&format!("change {tag}"));
    m.seal(None);
}

pub fn blocks_of(m: &Maintainer) -> Vec<String> {
    let out = run(
        &m.work,
        &m.keys,
        &["log", "--format", "json", "--limit", "100000"],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    v["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["block_id"].as_str().unwrap().to_owned())
        .collect()
}

pub fn patch_ids_of_last(m: &Maintainer, blocks: usize) -> Vec<String> {
    let out = run(
        &m.work,
        &m.keys,
        &["log", "--format", "json", "--limit", &blocks.to_string()],
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

// ------------------------------------------------------------------------------------------------ the archives of the measurement

/// Every archive of the measurement, built once per test binary.
pub struct Fixtures {
    pub dir: PathBuf,
    pub m1: KeyInfo,
    pub m2: KeyInfo,
    pub x: KeyInfo,
    /// The worker whose history the live repository is created from, and whose key it adopted.
    pub h0: PathBuf,
    pub d1: PathBuf,
    pub d2: PathBuf,
    pub div: PathBuf,
    pub m2_archive: PathBuf,
    pub m2div: PathBuf,
    pub x_archive: PathBuf,
    pub mix: PathBuf,
    pub xcarry: PathBuf,
    pub dev: PathBuf,
    pub unrelated: PathBuf,
    pub truncated: PathBuf,
    pub flips: Vec<(String, PathBuf)>,
    /// The M1 worker at H0 (keys and repository) for the scenario's clones.
    pub m1_at_h0: PathBuf,
    pub m1_at_h0_keys: PathBuf,
}

static FIXTURES: OnceLock<Fixtures> = OnceLock::new();

pub fn fixtures() -> &'static Fixtures {
    FIXTURES.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("delivery-fixtures-{}", binary_tag()));
        build_fixtures(&dir)
    })
}

pub fn build_fixtures(dir: &Path) -> Fixtures {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let arch = |name: &str| dir.join(format!("{name}.archive"));
    let m1 = Maintainer::new(dir, "m1");
    let m2 = Maintainer::new(dir, "m2");
    let x = Maintainer::new(dir, "x");
    let (k1, k2, kx) = (key_of(&m1), key_of(&m2), key_of(&x));
    // H0: five blocks by M1, and M2's key adopted (so the archive carries both: the forge adopts both at creation).
    adopt(&m1, &k2);
    for t in ["h2", "h3", "h4", "h5"] {
        step(&m1, t);
    }
    m1.archive(&arch("h0"));
    let m1_at_h0 = clone_of(&m1, dir, "m1-at-h0");
    // D1: five more.
    for t in ["d6", "d7", "d8", "d9", "d10"] {
        step(&m1, t);
    }
    m1.archive(&arch("d1"));
    let m1_at_d1 = clone_of(&m1, dir, "m1-at-d1");
    // D2: one more.
    step(&m1, "d11");
    m1.archive(&arch("d2"));
    // div: a rewrite from H0.
    let div = clone_of(&m1_at_h0, dir, "div");
    for t in ["v6", "v7", "v8"] {
        step(&div, t);
    }
    div.archive(&arch("div"));
    // xcarry: D1 + 1, M1's blocks only, but it carries X's key.
    let xc = clone_of(&m1_at_d1, dir, "xcarry");
    adopt(&xc, &kx);
    step(&xc, "c11");
    xc.archive(&arch("xcarry"));
    // dev: D1 and a second published branch.
    let dev = clone_of(&m1_at_d1, dir, "dev");
    run(
        &dev.work,
        &dev.keys,
        &["branch", "create", "heads/dev", "--from", "heads/main"],
    );
    run(&dev.work, &dev.keys, &["branch", "switch", "heads/dev"]);
    dev.write("src/dev.txt", "on dev\n");
    dev.commit("work on dev");
    dev.seal(Some("heads/dev"));
    dev.archive(&arch("dev"));
    // m2: D1 + 2 blocks by M2.
    let w = empty_with_keys_of(&m2, dir, "m2b");
    adopt(&w, &k1);
    import(&w, &arch("d1"));
    step(&w, "m11");
    step(&w, "m12");
    w.archive(&arch("m2"));
    // m2div: 3 blocks by M2 from H0.
    let w = empty_with_keys_of(&m2, dir, "m2c");
    adopt(&w, &k1);
    import(&w, &arch("h0"));
    for t in ["w6", "w7", "w8"] {
        step(&w, t);
    }
    w.archive(&arch("m2div"));
    // x: D1 + 2 blocks by the unadopted X.
    let w = empty_with_keys_of(&x, dir, "xb");
    adopt(&w, &k1);
    import(&w, &arch("d1"));
    step(&w, "x11");
    step(&w, "x12");
    w.archive(&arch("x"));
    // mix: an X block in the middle, an M2 block on top.
    let w = empty_with_keys_of(&x, dir, "xc");
    adopt(&w, &k1);
    import(&w, &arch("d1"));
    step(&w, "y11");
    w.archive(&arch("mix-mid"));
    let w2 = empty_with_keys_of(&m2, dir, "m2d");
    adopt(&w2, &k1);
    adopt(&w2, &kx);
    import(&w2, &arch("mix-mid"));
    step(&w2, "y12");
    w2.archive(&arch("mix"));
    // unrelated: M1's key, no common block.
    let u = empty_with_keys_of(&m1, dir, "u");
    for t in ["u1", "u2", "u3", "u4", "u5"] {
        step(&u, t);
    }
    u.archive(&arch("unrelated"));
    // truncated and five flips of D1.
    let bytes = std::fs::read(arch("d1")).unwrap();
    let truncated = dir.join("truncated.archive");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let needle = b"ed25519-";
    let hits: Vec<usize> = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect();
    let first = hits.first().map_or(bytes.len() / 3, |i| i + 9);
    let last = hits.last().map_or(bytes.len() / 3, |i| i + 9);
    let mut flips = Vec::new();
    for (name, pos) in [
        ("early", bytes.len() / 10),
        ("middle", bytes.len() / 2),
        ("after-first-id", first),
        ("after-last-id", last),
        ("tail", bytes.len() - 8),
    ] {
        let mut b = bytes.clone();
        b[pos] ^= 1;
        let p = dir.join(format!("flip-{name}.archive"));
        std::fs::write(&p, b).unwrap();
        flips.push((name.to_owned(), p));
    }
    Fixtures {
        dir: dir.to_path_buf(),
        m1: k1,
        m2: k2,
        x: kx,
        h0: arch("h0"),
        d1: arch("d1"),
        d2: arch("d2"),
        div: arch("div"),
        m2_archive: arch("m2"),
        m2div: arch("m2div"),
        x_archive: arch("x"),
        mix: arch("mix"),
        xcarry: arch("xcarry"),
        dev: arch("dev"),
        unrelated: arch("unrelated"),
        truncated,
        flips,
        m1_at_h0: m1_at_h0.work.clone(),
        m1_at_h0_keys: m1_at_h0.keys.clone(),
    }
}

// ------------------------------------------------------------------------------------------------ the services

pub type Hook = Arc<dyn Fn(&str) + Send + Sync>;

/// Everything the engine runs over, opened on a directory: a second `open` of the same directory is a restart.
pub struct Services {
    pub base: PathBuf,
    pub hosting: Arc<HostingService>,
    pub repos: Arc<InMemoryRepositoryStore>,
    pub deliveries: Arc<SqliteDeliveryStore>,
    pub changes: Arc<SqliteChangeStore>,
    pub artifacts: Arc<ArtifactStore>,
    pub export: Arc<ExportService>,
    pub leases: Arc<LeaseManager>,
    pub metrics: Arc<Metrics>,
    pub propose: Arc<ProposeService>,
    pub recovery: Arc<RecoveryService>,
    pub delivery: Arc<DeliveryService>,
    pub record: RepositoryRecord,
}

pub const REPO: &str = "fg01";

pub fn alice() -> UserId {
    UserId::new("alice")
}

pub fn repo_path(base: &Path) -> PathBuf {
    base.join("root").join("fg").join(REPO)
}

pub fn open(base: &Path, hook: Option<Hook>) -> Services {
    open_with(base, hook, |_| {})
}

pub fn open_with(
    base: &Path,
    hook: Option<Hook>,
    tweak: impl FnOnce(&mut DeliveryConfig),
) -> Services {
    open_full(base, hook, Sandbox::Unconfined, None, tweak)
}

/// Whether bubblewrap and `prlimit` work here (the tests of the real sandbox say why they skip).
pub fn sandbox_works() -> bool {
    let bwrap = Command::new("bwrap")
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
        .unwrap_or(false);
    bwrap
        && Command::new("prlimit")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

pub fn open_full(
    base: &Path,
    hook: Option<Hook>,
    sandbox: Sandbox,
    binary: Option<PathBuf>,
    tweak: impl FnOnce(&mut DeliveryConfig),
) -> Services {
    let db = SqliteDb::open(base.join("state.db")).unwrap();
    let record = RepositoryRecord {
        repo_id: RepoId::new(REPO),
        owner: Owner::User("alice".into()),
        name: "forge".into(),
        visibility: Visibility::Public,
        created_at: 0,
        prikk_format_version: None,
        proposals: ProposalPolicy::Members,
        path: repo_path(base),
    };
    let repos = Arc::new(InMemoryRepositoryStore::new());
    repos.create(record.clone()).unwrap();
    let mut hosting =
        HostingService::new(base.join("root"), repos.clone() as Arc<dyn RepositoryStore>)
            .with_sandbox(sandbox);
    if let Some(b) = binary {
        hosting = hosting.with_binary(b);
    }
    let hosting = Arc::new(hosting);
    let deliveries = Arc::new(SqliteDeliveryStore::new(db.clone()).unwrap());
    let changes = Arc::new(SqliteChangeStore::new(db.clone()).unwrap());
    let revisions = Arc::new(SqliteRevisions::new(db.clone()).unwrap());
    let leases = Arc::new(LeaseManager::new(
        Arc::new(SqliteLeaseRecords::new(db.clone()).unwrap()),
        new_boot_id(),
        real_clock,
        Duration::from_secs(1),
    ));
    let artifacts = Arc::new(ArtifactStore::open(base.join("artifacts")).unwrap());
    let spool = Spool::open(base.join("spool"), 512 << 20).unwrap();
    let metrics = Arc::new(Metrics::new());
    let export = Arc::new(ExportService::new(
        hosting.clone(),
        spool,
        leases.clone(),
        revisions as Arc<dyn RevisionStore>,
        ExportConfig::default(),
        metrics.clone(),
    ));
    let propose_config = ProposeConfig {
        lease_wait: Duration::from_secs(3),
        accept_deadline: Duration::from_secs(120),
        ..ProposeConfig::default()
    };
    let propose = Arc::new(
        ProposeService::new(
            hosting.clone(),
            artifacts.clone(),
            leases.clone(),
            changes.clone() as Arc<dyn ChangeStore>,
            export.clone(),
            propose_config.clone(),
            metrics.clone(),
        )
        .with_clock(real_clock),
    );
    let recoveries = Arc::new(planeter_store::SqliteRecoveries::new(db.clone()).unwrap());
    let recovery = Arc::new(
        RecoveryService::new(
            hosting.clone(),
            leases.clone(),
            changes.clone() as Arc<dyn ChangeStore>,
            recoveries as Arc<dyn planeter_store::RecoveryStore>,
            export.clone(),
            RecoveryConfig {
                lease_wait: Duration::from_secs(10),
                ..RecoveryConfig::default()
            },
            metrics.clone(),
        )
        .with_clock(real_clock),
    );
    let mut config = DeliveryConfig {
        deadline: Duration::from_secs(300),
        lease_wait: Duration::from_secs(3),
        propose: propose_config,
        ..DeliveryConfig::default()
    };
    tweak(&mut config);
    let mut delivery = DeliveryService::new(
        hosting.clone(),
        deliveries.clone() as Arc<dyn DeliveryStore>,
        changes.clone() as Arc<dyn ChangeStore>,
        artifacts.clone(),
        export.clone(),
        leases.clone(),
        metrics.clone(),
        config,
    )
    .unwrap()
    .with_clock(real_clock)
    .with_recovery(recovery.clone());
    if let Some(h) = hook {
        delivery = delivery.with_failpoint(h);
    }
    let delivery = Arc::new(delivery);
    // a recovery run that clears a `delivery_unfinished` flag completes the delivery (what the server's start-up wires)
    let weak = Arc::downgrade(&delivery);
    recovery.set_delivery_completion(Arc::new(move |record| {
        if let Some(d) = weak.upgrade() {
            let _ = d.resume(record);
        }
    }));
    Services {
        base: base.to_path_buf(),
        hosting,
        repos,
        deliveries,
        changes,
        artifacts,
        export,
        leases,
        metrics,
        propose,
        recovery,
        delivery,
        record,
    }
}

impl Services {
    /// A reader of the live repository.
    pub fn live(&self) -> planeter_prikk::CliPrikkRepo {
        self.hosting.open_record(&self.record).unwrap()
    }

    pub fn live_tip(&self) -> String {
        self.live().log(Some("heads/main"), Some(1)).unwrap().blocks[0]
            .block_id
            .clone()
    }

    pub fn pending(&self) -> Vec<String> {
        let mut p = self.live().sync_pending_patches().unwrap();
        p.sort();
        p
    }

    pub fn trust_ids(&self) -> Vec<String> {
        self.live()
            .trust_list()
            .unwrap()
            .keys
            .into_iter()
            .map(|k| k.key_id)
            .collect()
    }

    /// Propose `artifact` as `who` (a new proposal, or a revision of `proposal`).
    pub fn propose_artifact(
        &self,
        title: &str,
        artifact: &[u8],
        proposal: Option<&str>,
    ) -> planeter_transport::Proposed {
        let who = Submitter {
            id: "alice".into(),
            can_write: true,
        };
        let meta = ProposeMeta {
            title: Some(title.to_owned()),
            proposal: proposal.map(str::to_owned),
            ..ProposeMeta::default()
        };
        let plan = self
            .propose
            .check(&self.record, &who, &meta, artifact.len() as u64)
            .unwrap();
        let mut up = self.propose.start_upload(artifact.len() as u64).unwrap();
        up.write(artifact).unwrap();
        let up = up.finish().unwrap();
        self.propose
            .accept(&self.record, &who, &meta, &plan, up)
            .unwrap()
    }
}

/// The live repository as creation makes it from `archive`: `init`, every key of `keys` adopted, `archive import --adopt`.
pub fn create_live(base: &Path, archive: &Path, keys: &[&KeyInfo]) -> PathBuf {
    let path = repo_path(base);
    std::fs::create_dir_all(&path).unwrap();
    let home = base.join("forge-keys");
    std::fs::create_dir_all(&home).unwrap();
    run(&path, &home, &["init", "."]);
    for k in keys {
        run(
            &path,
            &home,
            &[
                "trust",
                "maintainer",
                "add",
                "--key-id",
                &k.key_id,
                "--public-key",
                &k.public_key,
            ],
        );
    }
    run(
        &path,
        &home,
        &["archive", "import", &archive.to_string_lossy(), "--adopt"],
    );
    path
}

/// The artifact `sync build` makes of `from` against the live repository's `sync have` (what a client does).
pub fn artifact_of(
    from: &Maintainer,
    live: &Path,
    live_keys: &Path,
    reference: &str,
    name: &str,
) -> Vec<u8> {
    let have = from.work.join(format!("{name}.have"));
    let art = from.work.join(format!("{name}.art"));
    let _ = std::fs::remove_file(&have);
    let _ = std::fs::remove_file(&art);
    run(
        live,
        live_keys,
        &[
            "sync",
            "have",
            reference,
            "--output",
            &have.to_string_lossy(),
        ],
    );
    run(
        &from.work,
        &from.keys,
        &[
            "sync",
            "build",
            reference,
            "--have",
            &have.to_string_lossy(),
            "--output",
            &art.to_string_lossy(),
        ],
    );
    std::fs::read(&art).unwrap()
}

/// The hash of every file under `dir`'s `.prikk` (name, size, bytes), as one digest: **byte-identical** is this being equal.
pub fn tree_hash(dir: &Path) -> String {
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    walk(&dir.join(".prikk"), &mut files);
    files.sort();
    let mut all = Vec::new();
    for f in files {
        all.extend_from_slice(f.strip_prefix(dir).unwrap().to_string_lossy().as_bytes());
        all.push(0);
        all.extend_from_slice(&std::fs::read(&f).unwrap_or_default());
        all.push(0);
    }
    planeter_auth::sha256_hex(&all)
}

/// The whole delivery as a client does it: start, upload, stage; the delivery's id and the report.
pub fn deliver(
    s: &Services,
    archive: &Path,
) -> Result<(String, planeter_transport::DeliveryReport), planeter_transport::DeliveryError> {
    let bytes = std::fs::read(archive).unwrap();
    let mut up = s.delivery.start(&alice(), &s.record, bytes.len() as u64)?;
    let id = up.delivery_id().to_owned();
    for chunk in bytes.chunks(64 * 1024) {
        up.write(chunk)?;
    }
    let uploaded = up.finish(&s.delivery)?;
    let report = s.delivery.finish(&alice(), &s.record, uploaded)?;
    Ok((id, report))
}

// ------------------------------------------------------------------------------------------------ the scenario

/// The measurement's scenario on the live repository: P1 (one patch) lands, P2 (one patch) stays, P3 (two patches) lands in part;
/// `d` is the delivery that carries P1's patch and P3's first patch and two more blocks, `d2` one block more.
pub struct Scenario {
    pub p1: String,
    pub p2: String,
    pub p3: String,
    /// The patches of P2 and of P3's second patch (what stays pending after `d`).
    pub stays: Vec<String>,
    pub p2_patches: Vec<String>,
    pub d: PathBuf,
    pub d2: PathBuf,
    pub d_tip: String,
    pub work: PathBuf,
}

pub fn scenario(s: &Services, fx: &Fixtures) -> Scenario {
    scenario_with(&s.base, &s.record.path, fx, &|title, artifact| {
        s.propose_artifact(title, artifact, None).proposal_id
    })
}

/// The scenario with the proposals made by `propose(title, artifact) -> proposal id` (in process, or over HTTP to a server).
pub fn scenario_with(
    base: &Path,
    live: &Path,
    fx: &Fixtures,
    propose: &dyn Fn(&str, &[u8]) -> String,
) -> Scenario {
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let live_keys = base.join("forge-keys");
    let live = live.to_path_buf();
    let at_h0 = Maintainer {
        work: fx.m1_at_h0.clone(),
        keys: fx.m1_at_h0_keys.clone(),
    };
    let m1 = clone_of(&at_h0, &work, "m1");
    // P1: one patch.
    step(&m1, "p1");
    let a1 = artifact_of(&m1, &live, &live_keys, "heads/main", "a1");
    let p1 = propose("P1", &a1);
    // P3: two patches; the delivery will carry the first only.
    let cc = clone_of(&m1, &work, "cc");
    step(&cc, "p3a");
    let cc2 = clone_of(&cc, &work, "cc2");
    step(&cc, "p3b");
    let p3b = patch_ids_of_last(&cc, 1);
    let a3 = artifact_of(&cc, &live, &live_keys, "heads/main", "a3");
    let p3 = propose("P3", &a3);
    // P2: one patch no delivery carries.
    let pc = clone_of(&m1, &work, "pc");
    step(&pc, "p2");
    let p2ids = patch_ids_of_last(&pc, 1);
    let a2 = artifact_of(&pc, &live, &live_keys, "heads/main", "a2");
    let p2 = propose("P2", &a2);
    // The delivery: H0 + p1 + p3a + e1 + e2; then one more.
    step(&cc2, "e1");
    step(&cc2, "e2");
    let d = work.join("delivery.archive");
    cc2.archive(&d);
    let d_tip = blocks_of(&cc2)[0].clone();
    step(&cc2, "e3");
    let d2 = work.join("delivery2.archive");
    cc2.archive(&d2);
    let p2_patches = p2ids.clone();
    let mut stays = p2ids;
    stays.extend(p3b);
    stays.sort();
    Scenario {
        p1,
        p2,
        p3,
        stays,
        p2_patches,
        d,
        d2,
        d_tip,
        work,
    }
}

pub fn open_ids(s: &Services) -> Vec<String> {
    let mut v: Vec<String> = s
        .changes
        .list_changes(&s.record.repo_id, Some(planeter_store::ChangeState::Open))
        .unwrap()
        .into_iter()
        .map(|c| c.change_id)
        .collect();
    v.sort();
    v
}

pub fn deliveries_dir_is_empty(s: &Services) -> bool {
    std::fs::read_dir(s.base.join("root/.deliveries"))
        .map(|d| d.count() == 0)
        .unwrap_or(true)
}

/// Per-file digests under `.prikk` (for a diff when two trees should be equal).
pub fn tree_files(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    walk(&dir.join(".prikk"), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|f| {
            (
                f.strip_prefix(dir).unwrap().to_string_lossy().into_owned(),
                planeter_auth::sha256_hex(&std::fs::read(&f).unwrap_or_default()),
            )
        })
        .collect()
}
