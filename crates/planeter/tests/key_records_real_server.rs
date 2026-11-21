//! RFC 013 D-4 (the foundation), T5 on the **real server process**: the one-time backfill of the record of where each trusted key came from.
//!
//! A database with 30 repositories that existed before the record (one of them already has a row, one is flagged for recovery) and a real
//! prikk behind the real sandbox; the server is started, answers while the pass runs, is **killed in the middle** of it, started again, and
//! ends where a straight run ends; a third start does nothing; the stop line has the counters. Tests self-skip, printing why, without prikk,
//! bubblewrap and prlimit.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_store::{
    ChangeStore, KeyEventStore, Owner, RepoId, RepositoryRecord, RepositoryStore,
    SqliteChangeStore, SqliteDb, SqliteKeyEventStore, SqliteRepositoryStore, Visibility,
};

const REPOS: usize = 30;

fn sandbox_works() -> bool {
    ["bwrap", "prlimit"].iter().all(|t| {
        Command::new(t)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

struct Fix {
    dir: PathBuf,
    db: SqliteDb,
}

/// 30 repositories, each trusting the same two real keys (`prikk init` + `trust maintainer add`); `r00` has one row already; `r01` is flagged.
fn world(name: &str) -> (Fix, Vec<archives::KeyInfo>) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("keyrec-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut keys = Vec::new();
    for i in 0..2 {
        let m = archives::Maintainer::new(&dir, &format!("maker{i}"));
        let out = archives::run(
            &m.work,
            &m.keys,
            &["trust", "maintainer", "list", "--format", "json"],
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        keys.push(archives::KeyInfo {
            key_id: v["keys"][0]["key_id"].as_str().unwrap().to_owned(),
            public_key: v["keys"][0]["public_key"].as_str().unwrap().to_owned(),
        });
    }
    let db = SqliteDb::open(dir.join("state.db")).unwrap();
    let repos = SqliteRepositoryStore::new(db.clone());
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    for i in 0..REPOS {
        let id = format!("r{i:02}");
        let path = dir.join("root").join("rr").join(&id);
        std::fs::create_dir_all(&path).unwrap();
        archives::run(&path, &home, &["init", "."]);
        for k in &keys {
            archives::run(
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
        repos
            .create(RepositoryRecord {
                repo_id: RepoId::new(&id),
                owner: Owner::User("alice".into()),
                name: id.clone(),
                visibility: Visibility::Public,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path,
            })
            .unwrap();
    }
    let events = SqliteKeyEventStore::new(db.clone()).unwrap();
    events
        .record(&planeter_store::KeyEvent {
            repo_id: RepoId::new("r00"),
            key_id: keys[0].key_id.clone(),
            public_key: keys[0].public_key.clone(),
            source: planeter_store::KeySource::Operator,
            detail: "added later".to_owned(),
            delivery_id: None,
            recorded_at: 5,
            backfilled: false,
        })
        .unwrap();
    SqliteChangeStore::new(db.clone())
        .unwrap()
        .set_needs_recovery(&RepoId::new("r01"), "the server stopped during a write", 1)
        .unwrap();
    (Fix { dir, db }, keys)
}

struct Server {
    child: Child,
    port: u16,
    lines: std::sync::mpsc::Receiver<String>,
}

fn start(dir: &Path) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_planeter"))
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("root"))
        .env("PLANETER_DB", dir.join("state.db"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        // the flag on r01 must stay: the unattended recovery would try it
        .env("PLANETER_AUTOMATIC_RECOVERY", "off")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let reader = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let log = dir.join("server.log");
    std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        for line in reader.lines().map_while(Result::ok) {
            let _ = writeln!(file, "{line}");
            let _ = tx.send(line);
        }
    });
    let s = Server {
        child,
        port,
        lines: rx,
    };
    s.wait_for("planeter listening on ", Duration::from_secs(60));
    s
}

impl Server {
    /// Wait for a log line that starts with `prefix`; return it.
    fn wait_for(&self, prefix: &str, within: Duration) -> String {
        let end = Instant::now() + within;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) if l.starts_with(prefix) => return l,
                Ok(_) => {}
                Err(_) => panic!("the server never said {prefix:?}"),
            }
        }
    }
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    fn get(&self, path: &str) -> u16 {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).ok();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        String::from_utf8_lossy(&raw)
            .lines()
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }
}

fn filled(db: &SqliteDb) -> usize {
    let store = SqliteKeyEventStore::new(db.clone()).unwrap();
    (0..REPOS)
        .filter(|i| store.has_any(&RepoId::new(format!("r{i:02}"))).unwrap())
        .count()
}

#[test]
fn the_backfill_answers_while_it_runs_survives_a_kill_in_the_middle_and_a_second_run_does_nothing()
{
    if !archives::prikk_available() || !sandbox_works() {
        eprintln!("skipping: needs prikk, bubblewrap and prlimit");
        return;
    }
    let (fix, keys) = world("main");
    let store = SqliteKeyEventStore::new(fix.db.clone()).unwrap();
    assert_eq!(filled(&fix.db), 1, "before: only r00 has a row");

    // ---- the first start: the server answers while the pass runs; it is killed in the middle of it
    let server = start(&fix.dir);
    let status = server.get("/alice/r05");
    assert_eq!(status, 200, "the server answers while the pass is running");
    let end = Instant::now() + Duration::from_secs(120);
    while filled(&fix.db) < 4 {
        assert!(Instant::now() < end, "the pass never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    let in_the_middle = filled(&fix.db);
    server.kill();
    // (the pass may have been at its very end: then the kill changes nothing and the test says so)
    let after_kill = filled(&fix.db);
    eprintln!(
        "killed with {in_the_middle} of {REPOS} repositories recorded ({after_kill} after the kill)"
    );
    // every repository is all or none: a repository has both of its keys or none (r00 has its one row, r01 none)
    for i in 2..REPOS {
        let n = store
            .events(&RepoId::new(format!("r{i:02}")))
            .unwrap()
            .len();
        assert!(
            n == 0 || n == 2,
            "r{i:02} has {n} rows: a fill is all or none"
        );
    }

    // ---- the second start finishes the work; the counts are for the rest only
    let server = start(&fix.dir);
    let line = server.wait_for("planeter: key records: ", Duration::from_secs(300));
    let after_restart = filled(&fix.db);
    // r00 (a row of its own) and r01 (flagged) are not filled; every other repository is
    assert_eq!(after_restart, REPOS - 1, "r01 stays unrecorded");
    assert!(
        line.contains("skipped") && line.contains(", 1 skipped"),
        "{line}"
    );
    for i in 2..REPOS {
        let rows = store.events(&RepoId::new(format!("r{i:02}"))).unwrap();
        assert_eq!(rows.len(), 2, "r{i:02}");
        assert!(
            rows.iter()
                .all(|e| e.backfilled && e.source == planeter_store::KeySource::Creation)
        );
        let mut got: Vec<&str> = rows.iter().map(|e| e.public_key.as_str()).collect();
        got.sort_unstable();
        let mut want: Vec<&str> = keys.iter().map(|k| k.public_key.as_str()).collect();
        want.sort_unstable();
        assert_eq!(got, want, "r{i:02}: the keys prikk lists");
    }
    assert!(store.events(&RepoId::new("r01")).unwrap().is_empty());
    let r00 = store.events(&RepoId::new("r00")).unwrap();
    assert_eq!(r00.len(), 1, "a repository with a row is not touched");
    assert_eq!(r00[0].detail, "added later");
    server.stop();

    // ---- the third start: nothing to fill, r01 still skipped; the stop line has the counters
    let server = start(&fix.dir);
    let line = server.wait_for("planeter: key records: ", Duration::from_secs(120));
    assert!(line.contains("0 repositories filled, 1 skipped"), "{line}");
    assert_eq!(filled(&fix.db), REPOS - 1);
    let log_before = std::fs::read_to_string(fix.dir.join("server.log")).unwrap();
    server.stop();
    let log = std::fs::read_to_string(fix.dir.join("server.log")).unwrap();
    assert!(
        log[log_before.len()..].contains("key_events_skipped=1"),
        "the stop counters: {}",
        &log[log_before.len()..]
    );
    // the log names no repository and no key
    for k in &keys {
        assert!(
            !log.contains(&k.public_key) && !log.contains(&k.key_id),
            "a key is named in the log"
        );
    }
    assert!(
        !log.contains("r05") && !log.contains("r00"),
        "a repository is named in the log"
    );
}
