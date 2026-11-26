//! RFC 013 D-2: **`planeter operator key add`**, the built binary against real prikk and the real sandbox. The sequence, every refusal (each
//! leaving the tree and the database as they were), a real server holding the lock, the kill at every point of the sequence (a debug build's
//! failpoints) followed by the real server's start and a re-run, the secret canary and the language. Tests self-skip, printing why, without
//! prikk, bubblewrap and prlimit.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;
#[path = "common/reaped.rs"]
#[allow(dead_code)]
mod reaped;
#[path = "../../planeter-transport/tests/support/delivery_world.rs"]
mod world;

use reaped::{Reaped, server_command};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    ChangeStore, DeliveryStore, KeyEventStore, LeaseRecordStore, Owner, RepoId, RepositoryRecord,
    RepositoryStore, RevisionStore, SqliteChangeStore, SqliteDb, SqliteDeliveryStore,
    SqliteKeyEventStore, SqliteLeaseRecords, SqliteRepositoryStore, SqliteRevisions, UserId,
    Visibility,
};
use world::{fixtures, scratch};

macro_rules! need_all {
    () => {
        if !world::prikk_ok() || !world::sandbox_works() {
            eprintln!("skipping: needs prikk, bubblewrap and prlimit");
            return;
        }
    };
}

const BIN: &str = env!("CARGO_BIN_EXE_planeter");

// ------------------------------------------------------------------------------------------------ the world

struct Fix {
    dir: PathBuf,
    live: PathBuf,
    db: SqliteDb,
}

/// A repository `alice/forge` made from the measurement's H0 archive with **only `keys` adopted**, in a database of its own.
fn fix(name: &str, adopt: &[&archives::KeyInfo]) -> Fix {
    let fx = fixtures();
    let dir = scratch(&format!("opkey-{name}"));
    let live = world::create_live(&dir, &fx.h0, adopt);
    let db = SqliteDb::open(dir.join("state.db")).unwrap();
    SqliteRepositoryStore::new(db.clone())
        .create(RepositoryRecord {
            repo_id: RepoId::new(world::REPO),
            owner: Owner::User("alice".into()),
            name: "forge".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: live.clone(),
        })
        .unwrap();
    let creds = SqliteCredentialStore::new(db.clone()).unwrap();
    creds
        .add_token(TokenRecord {
            id: String::new(),
            name: String::new(),
            created_at: None,
            last_used_at: None,
            hash: TokenHasher::hash(&Sha256TokenHasher, "tok-alice"),
            user: UserId::new("alice"),
            scope: Scope {
                access: ScopeAccess::Full,
                repos: RepoScope::All,
                expires_at: None,
            },
        })
        .unwrap();
    Fix { dir, live, db }
}

fn repo_id() -> RepoId {
    RepoId::new(world::REPO)
}

/// A fresh maintainer key made by prikk (its own directory and key directory).
fn new_key(dir: &Path, name: &str) -> archives::KeyInfo {
    let m = archives::Maintainer::new(dir, name);
    world::key_of(&m)
}

/// The command, as a process: only the settings it needs, and a canary in a secret it must never read.
fn operator(fix: &Fix, args: &[&str]) -> Output {
    operator_env(fix, args, &[])
}

fn operator_env(fix: &Fix, args: &[&str], extra: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.arg("operator")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_DB", fix.dir.join("state.db"))
        .env("PLANETER_REPOS_ROOT", fix.dir.join("root"))
        .env("PLANETER_OIDC_CLIENT_SECRET", CANARY)
        .env("HOME", fix.dir.join("forge-keys"))
        .current_dir(&fix.dir)
        .stdin(Stdio::null());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.output().expect("run planeter operator")
}

const CANARY: &str = "canary-secret-4f8a2c9d1e7b";

fn add_args<'a>(k: &'a archives::KeyInfo, reason: &'a str) -> Vec<&'a str> {
    vec![
        "key",
        "add",
        "alice/forge",
        "--key-id",
        &k.key_id,
        "--public-key",
        &k.public_key,
        "--reason",
        reason,
    ]
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

fn text(o: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// Everything a refusal must leave as it was: the key events, the revision counter, the flag, the lease records, the deliveries, the trust
/// list and the files of the `.prikk` tree (names and sizes; prikk's own caches are rewritten by reads and do not count).
fn snapshot(fix: &Fix) -> String {
    let mut s = String::new();
    s += &format!(
        "events {:?}\n",
        SqliteKeyEventStore::new(fix.db.clone())
            .unwrap()
            .events(&repo_id())
            .unwrap()
    );
    s += &format!(
        "revision {}\n",
        SqliteRevisions::new(fix.db.clone())
            .unwrap()
            .get(&repo_id())
            .unwrap()
    );
    s += &format!(
        "flags {:?}\n",
        SqliteChangeStore::new(fix.db.clone())
            .unwrap()
            .repo_flags(&repo_id())
            .unwrap()
    );
    s += &format!(
        "leases {:?}\n",
        SqliteLeaseRecords::new(fix.db.clone())
            .unwrap()
            .list()
            .unwrap()
    );
    s += &format!(
        "deliveries {:?}\n",
        SqliteDeliveryStore::new(fix.db.clone())
            .unwrap()
            .all()
            .unwrap()
    );
    s += &format!("trust {}\n", trust_list(fix));
    fn walk(d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.ends_with("cache") || p.ends_with("tmp") {
                continue;
            }
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&p, out);
            } else {
                out.push(format!(
                    "{} {}",
                    p.display(),
                    std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
                ));
            }
        }
    }
    let mut files = Vec::new();
    walk(&fix.live.join(".prikk"), &mut files);
    files.sort();
    s += &files.join("\n");
    s
}

fn trust_list(fix: &Fix) -> String {
    archives::run(
        &fix.live,
        &fix.dir.join("forge-keys"),
        &["trust", "maintainer", "list", "--format", "json"],
    )
}

fn trusted_ids(fix: &Fix) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(&trust_list(fix)).unwrap();
    let mut ids: Vec<String> = v["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["key_id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn verify_exit(fix: &Fix) -> i32 {
    Command::new("prikk")
        .args(["verify", "--format", "json"])
        .current_dir(&fix.live)
        .env("HOME", fix.dir.join("forge-keys"))
        .output()
        .unwrap()
        .status
        .code()
        .unwrap_or(-1)
}

/// The operator's events (the rows the command writes for the key it adds); the older keys of a repository with no record get a
/// `creation`, `backfilled` row each, before the add (step 7b).
fn operator_events(fix: &Fix) -> Vec<planeter_store::KeyEvent> {
    events(fix)
        .into_iter()
        .filter(|e| e.source == planeter_store::KeySource::Operator)
        .collect()
}

fn events(fix: &Fix) -> Vec<planeter_store::KeyEvent> {
    SqliteKeyEventStore::new(fix.db.clone())
        .unwrap()
        .events(&repo_id())
        .unwrap()
}

fn revision(fix: &Fix) -> u64 {
    SqliteRevisions::new(fix.db.clone())
        .unwrap()
        .get(&repo_id())
        .unwrap()
}

fn flagged(fix: &Fix) -> bool {
    SqliteChangeStore::new(fix.db.clone())
        .unwrap()
        .repo_flags(&repo_id())
        .unwrap()
        .needs_recovery
}

// ------------------------------------------------------------------------------------------------ a real server

struct Server {
    child: Reaped,
    port: u16,
}

fn start_server(dir: &Path) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Reaped::spawn(
        server_command(BIN)
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
            .env("PLANETER_REPOS_ROOT", dir.join("root"))
            .env("PLANETER_DB", dir.join("state.db"))
            .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
            .env("PLANETER_ARTIFACT_DIR", dir.join("artifacts"))
            .env("PLANETER_SPOOL_DIR", dir.join("spool"))
            .env("PLANETER_AUTOMATIC_RECOVERY", "on")
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    );
    let lines = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let log = dir.join("server.log");
    std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            let _ = writeln!(file, "{line}");
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(60)).expect("listening");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    Server { child, port }
}

impl Server {
    fn stop(mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn http(&self, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(300))).ok();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nAccept-Language: en\r\nAuthorization: Bearer tok-alice\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        s.write_all(req.as_bytes()).unwrap();
        let _ = s.write_all(body);
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(raw.len().saturating_sub(4));
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (status, raw.get(split + 4..).unwrap_or(&[]).to_vec())
    }

    /// POST the archive as a delivery (the import's framing); the delivery's JSON.
    fn deliver(&self, archive: &Path) -> serde_json::Value {
        let mut body = b"PLANETER-IMPORT-1\n".to_vec();
        body.extend_from_slice(&2u64.to_be_bytes());
        body.extend_from_slice(b"{}");
        body.extend_from_slice(&std::fs::read(archive).unwrap());
        let (status, reply) = self.http("POST", "/api/v1/repos/alice/forge/deliveries", &body);
        assert!(
            status == 201 || status == 202,
            "{status}: {}",
            String::from_utf8_lossy(&reply)
        );
        let v: serde_json::Value = serde_json::from_slice(&reply).unwrap();
        let id = v["id"].as_str().unwrap().to_owned();
        // wait for the check to finish
        let end = Instant::now() + Duration::from_secs(120);
        loop {
            let (_, r) = self.http(
                "GET",
                &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
                b"",
            );
            let v: serde_json::Value = serde_json::from_slice(&r).unwrap();
            if v["state"] != "staged" || Instant::now() > end {
                return v;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn wait_log_line(fix: &Fix, prefix: &str) {
    let end = Instant::now() + Duration::from_secs(90);
    loop {
        let log = std::fs::read_to_string(fix.dir.join("server.log")).unwrap_or_default();
        if log.lines().any(|l| l.starts_with(prefix)) {
            return;
        }
        assert!(Instant::now() < end, "the server never said {prefix:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_flag_clear(fix: &Fix) {
    let end = Instant::now() + Duration::from_secs(90);
    while flagged(fix) {
        assert!(Instant::now() < end, "the repository stayed flagged");
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ------------------------------------------------------------------------------------------------ the add

#[test]
fn the_add_makes_the_key_trusted_verifies_bumps_the_counter_and_records_how_it_came() {
    need_all!();
    let fx = fixtures();
    let f = fix("add", &[&fx.m1]);
    let b = new_key(&f.dir, "keyb");
    let before = revision(&f);
    assert_eq!(trusted_ids(&f), vec![fx.m1.key_id.clone()]);
    let o = operator(&f, &add_args(&b, "the second maintainer's key"));
    let (out, err) = text(&o);
    assert_eq!(code(&o), 0, "{out}{err}");
    // the result sentence, in the words the page will show
    let fp = planeter_core::creation::group_fingerprint(&b.public_key);
    assert!(
        out.contains(&format!(
            "{fp} is now trusted by alice/forge. Added by this server's operator on "
        )),
        "{out}"
    );
    assert!(out.contains(": the second maintainer's key"), "{out}");
    assert!(out.contains("Start the server."), "{out}");
    let mut want = vec![fx.m1.key_id.clone(), b.key_id.clone()];
    want.sort();
    assert_eq!(trusted_ids(&f), want, "prikk lists both");
    assert_eq!(verify_exit(&f), 0, "verify exits 0");
    let rows = operator_events(&f);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].key_id, b.key_id);
    assert_eq!(rows[0].public_key, b.public_key);
    assert_eq!(rows[0].detail, "the second maintainer's key");
    assert!(!rows[0].backfilled);
    // the repository had no record: the key it already trusted was recorded first, as the start-up pass would (creation, backfilled)
    let all = events(&f);
    assert_eq!(all.len(), 2, "{all:?}");
    let older = all
        .iter()
        .find(|e| e.public_key == fx.m1.public_key)
        .unwrap();
    assert_eq!(older.source, planeter_store::KeySource::Creation);
    assert!(older.backfilled);
    assert_eq!(
        revision(&f),
        before + 1,
        "the revision counter moved by one"
    );
    assert!(!flagged(&f));
    // nothing is left held: no lease record, no lock file
    assert!(
        SqliteLeaseRecords::new(f.db.clone())
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
    assert!(
        !f.dir.join("state.db.lock").exists(),
        "the lock is released"
    );
    // the secret canary appears nowhere
    assert!(!out.contains(CANARY) && !err.contains(CANARY));
    // the same key again: already trusted, and nothing changes
    let snap = snapshot(&f);
    let again = operator(&f, &add_args(&b, "again"));
    assert_eq!(code(&again), 1, "{:?}", text(&again));
    assert!(text(&again).1.contains("This key is already trusted."));
    assert_eq!(snapshot(&f), snap);
}

#[test]
fn the_whole_story_a_delivery_signed_by_the_new_key_is_refused_before_the_add_and_accepted_after_it()
 {
    need_all!();
    let fx = fixtures();
    let f = fix("story", &[&fx.m1]);
    // before: the real server refuses the delivery signed by M2 (its key is carried by the archive but not adopted)
    let server = start_server(&f.dir);
    let v = server.deliver(&fx.m2_archive);
    assert_eq!(v["state"], "failed", "{v}");
    assert_eq!(v["report"]["reason"], "unadopted_key", "{v}");
    // the command cannot run beside it
    let beside = operator(&f, &add_args(&fx.m2, "the second maintainer"));
    assert_eq!(code(&beside), 3, "{:?}", text(&beside));
    server.stop();
    // after the server is stopped: the add
    let o = operator(&f, &add_args(&fx.m2, "the second maintainer"));
    assert_eq!(code(&o), 0, "{:?}", text(&o));
    // the same delivery is now accepted by the real server
    let server = start_server(&f.dir);
    let v = server.deliver(&fx.m2_archive);
    assert_eq!(v["state"], "reported", "{v}");
    assert_eq!(v["report"]["verdict"], "acceptable", "{v}");
    let id = v["id"].as_str().unwrap();
    let (status, body) = server.http(
        "POST",
        &format!("/api/v1/repos/alice/forge/deliveries/{id}/confirm"),
        b"",
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let done: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(done["state"], "done", "{done}");
    // RFC 013 D-4, in the real server: the key the operator added is on the repository's page and in the JSON, with the reason and how it came
    let (status, page) = server.http("GET", "/alice/forge", b"");
    assert_eq!(status, 200);
    let page = String::from_utf8_lossy(&page).into_owned();
    assert!(
        page.contains("Added by this server&#39;s operator on ")
            && page.contains(": the second maintainer"),
        "{page}"
    );
    assert!(page.contains("Keys this repository trusts<span class=\"count\">(2)</span>"));
    let (status, refs) = server.http("GET", "/api/v1/repos/alice/forge/refs", b"");
    assert_eq!(status, 200);
    let refs: serde_json::Value = serde_json::from_slice(&refs).unwrap();
    let keys = refs["trusted_keys"].as_array().expect("the keys");
    assert_eq!(keys.len(), 2, "{refs}");
    let added = keys.iter().find(|k| k["key_id"] == fx.m2.key_id).unwrap();
    assert_eq!(added["source"]["kind"], "operator");
    assert_eq!(added["source"]["reason"], "the second maintainer");
    server.stop();
    assert_eq!(verify_exit(&f), 0);
}

// ------------------------------------------------------------------------------------------------ the three states of "trusted, not recorded"

/// RFC 013 D-4: the page never claims what was not recorded. In a repository with **no key record at all**, every trusted key is
/// "not recorded"; a key trusted since creation is **not** an interrupted act of this command (which always leaves a row), so the
/// command refuses it as already trusted and writes no `operator` event for it.
#[test]
fn a_key_trusted_since_creation_in_a_repository_with_no_record_is_refused_and_never_becomes_an_operators_add()
 {
    need_all!();
    let fx = fixtures();
    let f = fix("no-record", &[&fx.m1]);
    assert!(
        events(&f).is_empty(),
        "the precondition: no key record at all"
    );
    let snap = snapshot(&f);
    let o = operator(&f, &add_args(&fx.m1, "x"));
    let (out, err) = text(&o);
    assert_eq!(code(&o), 1, "{out}{err}");
    assert!(err.contains("This key is already trusted."), "{err}");
    assert!(out.is_empty(), "{out}");
    assert!(
        operator_events(&f).is_empty(),
        "no operator event for a key that was there from the start"
    );
    assert_eq!(
        snapshot(&f),
        snap,
        "nothing changed: no event, counter unmoved, no lease record"
    );
}

/// The state an interrupted act leaves (rows exist, the key is trusted and has no row of its own) is completed: the kill-point table
/// (`a_kill_at_every_point_is_recovered…`, rows `operator_after_add`, `_verify`, `_bump`) is that state, and this test pins it in the
/// third form below. A key added out of band beside rows is recorded as the operator's add with the operator's reason: the command cannot
/// tell it from an interrupted act, and an operator who runs it with a reason asserts it.
#[test]
fn a_key_trusted_and_unrecorded_beside_existing_rows_is_completed_and_recorded_with_the_operators_reason()
 {
    need_all!();
    let fx = fixtures();
    let f = fix("rows-then-oob", &[&fx.m1]);
    // the repository has rows: the key it trusts is recorded (creation, backfilled)
    SqliteKeyEventStore::new(f.db.clone())
        .unwrap()
        .record(&planeter_store::KeyEvent {
            repo_id: repo_id(),
            key_id: fx.m1.key_id.clone(),
            public_key: fx.m1.public_key.clone(),
            source: planeter_store::KeySource::Creation,
            detail: String::new(),
            delivery_id: None,
            recorded_at: 1,
            backfilled: true,
        })
        .unwrap();
    // key B added out of band with prikk, never recorded
    let b = new_key(&f.dir, "keyb");
    archives::run(
        &f.live,
        &f.dir.join("forge-keys"),
        &[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            &b.key_id,
            "--public-key",
            &b.public_key,
        ],
    );
    assert!(trusted_ids(&f).contains(&b.key_id));
    assert!(operator_events(&f).is_empty());
    let before = revision(&f);
    let o = operator(&f, &add_args(&b, "added by hand earlier"));
    assert_eq!(code(&o), 0, "{:?}", text(&o));
    assert!(
        text(&o).0.contains(
            "was already trusted by alice/forge; the record of it was missing and is written now."
        ),
        "{:?}",
        text(&o)
    );
    let rows = operator_events(&f);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].key_id, b.key_id);
    assert_eq!(rows[0].detail, "added by hand earlier");
    assert!(!rows[0].backfilled);
    assert_eq!(revision(&f), before + 1);
    assert_eq!(verify_exit(&f), 0);
}

// ------------------------------------------------------------------------------------------------ the refusals

/// Run a refused command and hold that nothing changed.
fn refused(f: &Fix, args: &[&str], want_code: i32, sentence: &str) {
    let snap = snapshot(f);
    let o = operator(f, args);
    let (out, err) = text(&o);
    assert_eq!(code(&o), want_code, "{args:?}: {out}{err}");
    assert!(
        err.contains(sentence),
        "{args:?}: wanted {sentence:?} in {err:?}"
    );
    assert!(out.is_empty(), "a refusal prints nothing on stdout: {out}");
    assert!(!out.contains(CANARY) && !err.contains(CANARY));
    assert_eq!(snapshot(f), snap, "{args:?}: nothing changed");
    assert!(
        !f.dir.join("state.db.lock").exists(),
        "{args:?}: the lock is released"
    );
}

#[test]
fn every_refusal_says_what_is_wrong_and_changes_nothing() {
    need_all!();
    let fx = fixtures();
    let f = fix("refusals", &[&fx.m1]);
    let b = new_key(&f.dir, "keyb");
    let base = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = [
            "key",
            "add",
            "alice/forge",
            "--key-id",
            &b.key_id,
            "--public-key",
            &b.public_key,
        ]
        .map(String::from)
        .to_vec();
        v.extend(extra.iter().map(|s| (*s).to_owned()));
        v
    };
    let r = |extra: &[&str], want: i32, sentence: &str| {
        let owned = base(extra);
        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        refused(&f, &args, want, sentence);
    };
    // the command line (exit 2)
    r(&[], 2, "A reason is required");
    r(&["--reason", ""], 2, "A reason is required");
    r(
        &["--reason", &"x".repeat(201)],
        2,
        "The reason is 201 characters long, and it can be at most 200",
    );
    r(&["--reason", "two\nlines"], 2, "cannot be shown");
    r(&["--reason", "ok", "--extra", "1"], 2, "not understood");
    let upper = b.public_key.to_uppercase();
    refused(
        &f,
        &[
            "key",
            "add",
            "alice/forge",
            "--key-id",
            &b.key_id,
            "--public-key",
            &upper,
            "--reason",
            "ok",
        ],
        2,
        "64 characters, each one 0-9 or a-f in lower case",
    );
    refused(
        &f,
        &[
            "key",
            "add",
            "alice/forge",
            "--key-id",
            "ed25519-XYZ",
            "--public-key",
            &b.public_key,
            "--reason",
            "ok",
        ],
        2,
        "ed25519- followed by 16 characters",
    );
    refused(
        &f,
        &[
            "key",
            "add",
            "alicenoslash",
            "--key-id",
            &b.key_id,
            "--public-key",
            &b.public_key,
            "--reason",
            "ok",
        ],
        2,
        "OWNER/NAME",
    );
    refused(&f, &["key", "remove", "alice/forge"], 2, "not understood");
    refused(&f, &["frobnicate"], 2, "not understood");
    // the key (exit 1): an id that is not the one prikk derives, shown with the derived one
    let wrong_id = "ed25519-0000000000000000";
    refused(
        &f,
        &[
            "key",
            "add",
            "alice/forge",
            "--key-id",
            wrong_id,
            "--public-key",
            &b.public_key,
            "--reason",
            "ok",
        ],
        1,
        &format!(
            "The key id {wrong_id} does not belong to this public key: prikk derives {} from it",
            b.key_id
        ),
    );
    // an unknown repository
    refused(
        &f,
        &[
            "key",
            "add",
            "alice/nope",
            "--key-id",
            &b.key_id,
            "--public-key",
            &b.public_key,
            "--reason",
            "ok",
        ],
        1,
        "There is no repository alice/nope",
    );
    // a flagged repository
    SqliteChangeStore::new(f.db.clone())
        .unwrap()
        .set_needs_recovery(&repo_id(), "a write did not finish (accept_deadline)", 1)
        .unwrap();
    r(
        &["--reason", "ok"],
        1,
        "Start the server and let recovery finish first.",
    );
    SqliteChangeStore::new(f.db.clone())
        .unwrap()
        .clear_needs_recovery(&repo_id())
        .unwrap();
    // an orphan lease record that a dead process left
    SqliteLeaseRecords::new(f.db.clone())
        .unwrap()
        .record(&planeter_store::HolderRecord {
            repo_id: repo_id(),
            class: "W".into(),
            operation: "sync accept".into(),
            holder_id: "h-dead".into(),
            boot_id: "a-boot-that-is-gone".into(),
            since_unix: 1,
        })
        .unwrap();
    r(
        &["--reason", "ok"],
        1,
        "Start the server and let recovery finish first.",
    );
    SqliteLeaseRecords::new(f.db.clone())
        .unwrap()
        .clear("h-dead")
        .unwrap();
    // an unfinished delivery
    SqliteDeliveryStore::new(f.db.clone())
        .unwrap()
        .start(
            planeter_store::DeliveryRecord {
                delivery_id: "dl-aaaaaaaaaaaaaaaaaaaaaaaa".into(),
                repo_id: repo_id(),
                principal: "alice".into(),
                state: planeter_store::DeliveryState::Staged,
                archive_digest: String::new(),
                archive_bytes: 0,
                stage_dir: "dl-aaaaaaaaaaaaaaaaaaaaaaaa".into(),
                old_tip: None,
                new_tip: None,
                report_json: None,
                reason: None,
                created_at: 1,
                updated_at: 1,
                finished_at: None,
            },
            planeter_store::StageLimits {
                per_principal: 2,
                total_bytes: 1 << 30,
            },
        )
        .unwrap()
        .unwrap();
    r(
        &["--reason", "ok"],
        1,
        "Start the server and let recovery finish first.",
    );
    // and a database that is not there
    let mut nodb = Command::new(BIN);
    nodb.args([
        "operator",
        "key",
        "add",
        "alice/forge",
        "--key-id",
        &b.key_id,
        "--public-key",
        &b.public_key,
        "--reason",
        "ok",
    ])
    .env_clear()
    .env("PATH", std::env::var("PATH").unwrap_or_default())
    .env("PLANETER_DB", f.dir.join("nothing-here.db"))
    .env("PLANETER_REPOS_ROOT", f.dir.join("root"));
    let o = nodb.output().unwrap();
    assert_eq!(code(&o), 5, "{:?}", text(&o));
    assert!(text(&o).1.contains("There is no database at"));
    assert!(
        !f.dir.join("nothing-here.db").exists() && !f.dir.join("nothing-here.db.lock").exists(),
        "nothing was made"
    );
}

#[test]
fn a_repository_that_trusts_the_most_keys_refuses_one_more_with_the_bound_sentence() {
    need_all!();
    let fx = fixtures();
    let f = fix("bound", &[&fx.m1]);
    // 63 more keys, made directly with prikk (it takes any well-formed pair), so that the repository trusts 64
    let home = f.dir.join("forge-keys");
    for i in 0..63u8 {
        let pk = format!("{:02x}{:02x}", i, 0xa5).repeat(16);
        let id = planeter_prikk::keys::key_id_of(&pk).unwrap();
        archives::run(
            &f.live,
            &home,
            &[
                "trust",
                "maintainer",
                "add",
                "--key-id",
                &id,
                "--public-key",
                &pk,
            ],
        );
    }
    assert_eq!(trusted_ids(&f).len(), 64);
    let b = new_key(&f.dir, "keyb");
    refused(
        &f,
        &add_args(&b, "one too many"),
        1,
        "This repository already trusts the most keys planeter supports: 64. Keys cannot be removed yet, so no more can be added.",
    );
}

// ------------------------------------------------------------------------------------------------ the lock

#[test]
fn a_stale_lock_of_a_dead_process_is_replaced_and_the_command_runs() {
    need_all!();
    let fx = fixtures();
    let f = fix("stale", &[&fx.m1]);
    let b = new_key(&f.dir, "keyb");
    // a process that has ended: its id is in the lock file
    let mut dead = Command::new("true").spawn().unwrap();
    let pid = dead.id();
    dead.wait().unwrap();
    std::fs::write(f.dir.join("state.db.lock"), format!("{pid}\n")).unwrap();
    let o = operator(&f, &add_args(&b, "after a crash"));
    assert_eq!(code(&o), 0, "{:?}", text(&o));
    assert!(!f.dir.join("state.db.lock").exists());
    // a lock file that names nothing that can be checked is the server's own message, exit 3, nothing changed
    std::fs::write(f.dir.join("state.db.lock"), "not a pid\n").unwrap();
    let c = new_key(&f.dir, "keyc");
    let snap = snapshot(&f);
    let o = operator(&f, &add_args(&c, "unclear"));
    assert_eq!(code(&o), 3, "{:?}", text(&o));
    assert!(text(&o).1.contains("can be checked"), "{:?}", text(&o));
    assert_eq!(snapshot(&f), snap);
}

// ------------------------------------------------------------------------------------------------ the language

#[test]
fn the_same_refusal_is_said_in_english_and_in_japanese_by_the_flag_and_by_the_locale() {
    need_all!();
    let fx = fixtures();
    let f = fix("lang", &[&fx.m1]);
    let b = new_key(&f.dir, "keyb");
    let args = |extra: &[&'static str]| -> Vec<String> {
        let mut v = add_args(&b, "ok")
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        v.truncate(7); // without `--reason`: the refusal is the missing reason
        v.extend(extra.iter().map(|s| (*s).to_owned()));
        v
    };
    let run = |extra: &[&'static str], env: &[(&str, &str)]| {
        let a = args(extra);
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        text(&operator_env(&f, &refs, env)).1
    };
    let en = "A reason is required";
    let ja = "理由が必要です";
    assert!(run(&[], &[]).contains(en), "no locale at all: English");
    assert!(
        run(&["--lang", "en"], &[("LC_ALL", "ja_JP.UTF-8")]).contains(en),
        "the flag beats the locale"
    );
    assert!(run(&["--lang", "ja"], &[]).contains(ja));
    assert!(run(&[], &[("LC_ALL", "ja_JP.UTF-8")]).contains(ja));
    assert!(run(&[], &[("LC_MESSAGES", "ja_JP.UTF-8")]).contains(ja));
    assert!(run(&[], &[("LANG", "ja_JP.UTF-8")]).contains(ja));
    assert!(
        run(&[], &[("LC_ALL", "C"), ("LANG", "ja_JP.UTF-8")]).contains(en),
        "C names no language: English"
    );
    // an unknown --lang is a malformed command line, said in English
    let o = operator(&f, &["--lang", "xx", "--help"]);
    assert_eq!(code(&o), 2);
    // --help in both languages says the sentence of the handoff
    let h = |lang: &str| {
        String::from_utf8_lossy(&operator(&f, &["--help", "--lang", lang]).stdout).into_owned()
    };
    assert!(h("en").contains("For the person who runs this server. It needs the server's data and cannot be run from the web. Stop the server first."));
    assert!(h("ja").contains("このサーバーを運用している人のためのコマンドです。"));
    // `planeter --help` names the family
    let top = Command::new(BIN).arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&top.stdout).contains("operator"));
}

// ------------------------------------------------------------------------------------------------ the kills

/// Run the command with a failpoint, SIGKILL it there.
fn kill_at(f: &Fix, k: &archives::KeyInfo, point: &str) {
    let marker = f.dir.join(format!("marker-{point}"));
    let _ = std::fs::remove_file(&marker);
    let args = add_args(k, "killed on purpose");
    let mut child = Command::new(BIN)
        .arg("operator")
        .args(&args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_DB", f.dir.join("state.db"))
        .env("PLANETER_REPOS_ROOT", f.dir.join("root"))
        .env("HOME", f.dir.join("forge-keys"))
        .env(
            "PLANETER_FAILPOINT",
            format!("{point}={}", marker.display()),
        )
        .current_dir(&f.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(120);
    while !marker.exists() {
        assert!(Instant::now() < end, "the command never reached {point}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_kill_at_every_point_is_recovered_by_the_servers_start_and_the_rerun_ends_in_the_same_state() {
    need_all!();
    let fx = fixtures();
    // (point, the key is trusted after the kill, the revision counter moved, the event is written)
    let table = [
        ("operator_after_lease", false, false, false),
        ("operator_after_add", true, false, false),
        ("operator_after_verify", true, false, false),
        ("operator_after_bump", true, true, false),
        ("operator_after_event", true, true, true),
    ];
    let mut report = String::new();
    for (point, trusted, bumped, recorded) in table {
        let f = fix(&format!("kill-{point}"), &[&fx.m1]);
        let b = new_key(&f.dir, "keyb");
        let before = revision(&f);
        kill_at(&f, &b, point);
        // what the kill left
        assert_eq!(
            trusted_ids(&f).contains(&b.key_id),
            trusted,
            "{point}: trusted"
        );
        assert_eq!(revision(&f) > before, bumped, "{point}: counter");
        assert_eq!(operator_events(&f).len() == 1, recorded, "{point}: event");
        let leases = SqliteLeaseRecords::new(f.db.clone())
            .unwrap()
            .list()
            .unwrap();
        assert!(
            leases.iter().any(|h| h.operation == "operator key add"
                && h.class == "W"
                && h.repo_id == repo_id()),
            "{point}: the lease record is left as the intent: {leases:?}"
        );
        // the real server starts: it flags the repository and recovers it by itself where the lease record says so
        let server = start_server(&f.dir);
        wait_flag_clear(&f);
        // the server's start-up pass over the key records has ended (it skips a flagged repository and never touches one with rows: it must not
        // label a key the operator added "creation")
        wait_log_line(&f, "planeter: key records:");
        // before the re-run a key added and not recorded shows as "not recorded"
        if trusted && !recorded {
            let keys = planeter_transport::repo_keys(
                &planeter_prikk::CliPrikkRepo::new(&f.live),
                &SqliteKeyEventStore::new(f.db.clone()).unwrap(),
                &repo_id(),
            )
            .unwrap();
            let me = keys.iter().find(|k| k.key_id == b.key_id).unwrap();
            assert_eq!(
                me.source,
                planeter_transport::KeySourceView::NotRecorded,
                "{point}"
            );
        }
        server.stop();
        assert_eq!(
            verify_exit(&f),
            0,
            "{point}: verify is clean after the recovery"
        );
        assert!(
            SqliteLeaseRecords::new(f.db.clone())
                .unwrap()
                .list()
                .unwrap()
                .is_empty(),
            "{point}: no lease record"
        );
        // the re-run ends in the same final state
        let o = operator(&f, &add_args(&b, "killed on purpose"));
        // killed after the event, everything was done: the re-run finds the key trusted and recorded and refuses (the state is the same)
        assert_eq!(
            code(&o),
            if recorded { 1 } else { 0 },
            "{point}: {:?}",
            text(&o)
        );
        assert!(trusted_ids(&f).contains(&b.key_id), "{point}");
        let rows = operator_events(&f);
        assert_eq!(rows.len(), 1, "{point}: one event: {rows:?}");
        assert_eq!(rows[0].key_id, b.key_id);
        assert_eq!(verify_exit(&f), 0, "{point}");
        assert!(!flagged(&f), "{point}");
        // a second re-run refuses: it is already trusted and recorded
        assert_eq!(code(&operator(&f, &add_args(&b, "again"))), 1, "{point}");
        report += &format!(
            "{point}: after the kill trusted={trusted} counter_moved={bumped} event={recorded}; the server's start recovered it; the re-run: exit 0, one event, verify 0\n"
        );
    }
    let _ = std::fs::write(
        std::env::temp_dir().join("planeter-operator-kill-table.txt"),
        report,
    );
}

// ------------------------------------------------------------------------------------------------ verify fails after the add

#[test]
fn a_verify_that_fails_after_the_add_flags_the_repository_says_so_and_cannot_undo_the_add() {
    need_all!();
    // A repository that adopted no key: its history is signed by M1, whose ref landed under `remotes/`, so `verify` fails (publication trust).
    // The key the operator adds is not the signer's, so the repository still fails after the add: a real failure of the check, not an injection.
    let f = fix("verify-fails", &[]);
    let b = new_key(&f.dir, "keyb");
    assert_ne!(
        verify_exit(&f),
        0,
        "the precondition: verify fails before the add"
    );
    let o = operator(&f, &add_args(&b, "verify will fail"));
    assert_eq!(code(&o), 4, "{:?}", text(&o));
    assert!(
        text(&o)
            .1
            .contains("The key was added, but the repository no longer passes its checks"),
        "{:?}",
        text(&o)
    );
    assert!(flagged(&f), "the repository is flagged");
    let reason = SqliteChangeStore::new(f.db.clone())
        .unwrap()
        .repo_flags(&repo_id())
        .unwrap()
        .reason;
    assert_eq!(
        reason.as_deref(),
        Some("an operator's key add did not verify")
    );
    assert!(trusted_ids(&f).contains(&b.key_id), "the add is not undone");
    assert!(
        events(&f).is_empty(),
        "no record is written for a repository that did not verify"
    );
    // and the next run refuses until recovery is done
    let again = operator(&f, &add_args(&b, "again"));
    assert_eq!(code(&again), 1);
    assert!(
        text(&again)
            .1
            .contains("Start the server and let recovery finish first.")
    );
}

// ------------------------------------------------------------------------------------------------ the secret canary, everywhere

#[test]
fn a_secret_in_the_environment_is_never_read_and_appears_in_no_output_and_no_file() {
    need_all!();
    let fx = fixtures();
    let f = fix("canary", &[&fx.m1]);
    let b = new_key(&f.dir, "keyb");
    let mut seen = String::new();
    for args in [
        add_args(&b, "canary run"),
        add_args(&b, "canary run again"),
        vec!["--help"],
        vec!["key", "add"],
        vec!["nonsense"],
    ] {
        let o = operator(&f, &args);
        let (out, err) = text(&o);
        seen += &out;
        seen += &err;
    }
    assert!(!seen.contains(CANARY));
    // nor in the database or the repository the command wrote
    let db_bytes = std::fs::read(f.dir.join("state.db")).unwrap();
    assert!(
        !db_bytes
            .windows(CANARY.len())
            .any(|w| w == CANARY.as_bytes()),
        "the canary is in the database"
    );
    let tree = snapshot(&f);
    assert!(!tree.contains(CANARY));
}
