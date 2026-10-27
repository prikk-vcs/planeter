//! RFC 004 part 7, T1: the real server, real prikk and the real sandbox; the server is **SIGKILLed in the middle of an archive
//! creation**. Afterwards, at the next start, nothing is visible and nothing is left on disk:
//!
//! * the repository record (state `creating`), its directory with the half-imported store, its grant, the lease record the killed
//!   process left and the stage whose confirmation was running are all removed, and each removal is **logged**;
//! * the name is free again (an empty repository of that name is created at once);
//! * no `needs-recovery` flag is set for the repository that never was.
//!
//! A second test kills the server **while an archive is being uploaded**: the half-written stage directory has no record, and the
//! next start removes it. Both self-skip (printing why) without prikk, prlimit or bubblewrap.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    ChangeStore, Owner, RepoId, RepositoryStore, SqliteChangeStore, SqliteDb, SqliteLeaseRecords,
    SqliteRepositoryStore, SqliteStageStore, StageStore, UserId,
};

fn runs(tool: &str) -> bool {
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

fn can_run() -> bool {
    if !archives::prikk_available() || !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs prikk, prlimit and a working bubblewrap");
        return false;
    }
    true
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("creation-kill-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Server {
    child: Child,
    port: u16,
    /// Everything the server wrote to stderr.
    log: Arc<Mutex<String>>,
}

impl Server {
    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
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
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .env("PLANETER_ARTIFACT_DIR", dir.join("artifacts"))
        .env("PLANETER_SPOOL_DIR", dir.join("spool"))
        .env("PLANETER_SPOOL_BUDGET_MB", "64")
        .env("PLANETER_AUTOMATIC_RECOVERY", "off")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let lines = BufReader::new(child.stderr.take().unwrap());
    let log = Arc::new(Mutex::new(String::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    let sink = Arc::clone(&log);
    std::thread::spawn(move || {
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            sink.lock().unwrap().push_str(&format!("{line}\n"));
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(30)).expect("listening");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    Server { child, port, log }
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
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

fn bearer() -> Vec<(&'static str, String)> {
    vec![("Authorization", "Bearer tok-alice".to_owned())]
}

fn frame(meta: &str, archive: &[u8]) -> Vec<u8> {
    let mut out = b"PLANETER-IMPORT-1\n".to_vec();
    out.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(archive);
    out
}

fn add_token(dir: &Path) {
    let db = SqliteDb::open(dir.join("db.sqlite")).unwrap();
    SqliteCredentialStore::new(db)
        .unwrap()
        .add_token(TokenRecord {
            hash: TokenHasher::hash(&Sha256TokenHasher, "tok-alice"),
            user: UserId::new("alice"),
            scope: Scope {
                access: ScopeAccess::Write,
                repos: RepoScope::All,
                expires_at: None,
            },
        })
        .unwrap();
}

fn text_of(body: &[u8]) -> String {
    String::from_utf8_lossy(body).into_owned()
}

/// The text after `"key":"` in a flat JSON reply.
fn json_str(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\":\""))? + key.len() + 4;
    Some(text[at..at + text[at..].find('"')?].to_owned())
}

fn dirs_in(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for shard in std::fs::read_dir(root).into_iter().flatten().flatten() {
        for child in std::fs::read_dir(shard.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            out.push(child.path());
        }
    }
    out
}

fn dir_bytes(p: &Path) -> u64 {
    let mut total = 0;
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        total += if m.is_dir() {
            dir_bytes(&e.path())
        } else {
            m.len()
        };
    }
    total
}

#[test]
fn a_server_killed_in_the_middle_of_an_archive_creation_leaves_nothing_and_the_name_is_free() {
    if !can_run() {
        return;
    }
    let dir = scratch("confirm");
    let built = Instant::now();
    let (archive, keys) = archives::simple_archive(&dir, "maint", 1200, 0);
    eprintln!("built a 1,200-patch archive in {:?}", built.elapsed());
    let bytes = std::fs::read(&archive).unwrap();
    add_token(&dir);

    let mut first = start(&dir);
    let meta = r#"{"owner":"alice","name":"arch","visibility":"private"}"#;
    let mut headers = bearer();
    headers.push(("Content-Type", "application/octet-stream".to_owned()));
    let (status, body) = http(
        first.port,
        "POST",
        "/api/v1/repos/import",
        &headers,
        &frame(meta, &bytes),
    );
    assert_eq!(status, 201, "{}", text_of(&body));
    let stage = json_str(&text_of(&body), "stage_id").expect("stage_id");
    // Not yet a repository.
    let db = || SqliteDb::open(dir.join("db.sqlite")).unwrap();
    assert!(
        SqliteRepositoryStore::new(db())
            .creating()
            .unwrap()
            .is_empty()
    );

    // Confirm, and kill the server once the half-made repository has bytes of the import in it.
    let port = first.port;
    let confirm_body = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
        keys[0].key_id, keys[0].public_key
    );
    let path = format!("/api/v1/repos/import/{stage}/confirm");
    let confirmer = std::thread::spawn(move || {
        let mut h = bearer();
        h.push(("Content-Type", "application/json".to_owned()));
        http(port, "POST", &path, &h, confirm_body.as_bytes())
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let creating = loop {
        let rows = SqliteRepositoryStore::new(db()).creating().unwrap();
        if let Some(r) = rows.first()
            && dir_bytes(&r.path.join(".prikk")) > 150_000
        {
            break r.clone();
        }
        assert!(Instant::now() < deadline, "the import never began");
        std::thread::sleep(Duration::from_millis(10));
    };
    first.child.kill().expect("SIGKILL");
    first.child.wait().unwrap();
    let (status, _) = confirmer.join().unwrap();
    assert_ne!(
        status, 201,
        "the confirmation must not have finished before the kill"
    );

    // What the kill left: a `creating` record, its directory, a stage being confirmed, and the lease record of the dead holder.
    assert!(
        SqliteRepositoryStore::new(db())
            .resolve(&Owner::User("alice".into()), "arch")
            .unwrap()
            .is_none(),
        "invisible"
    );
    assert!(creating.path.is_dir(), "the half-made directory is on disk");
    assert_eq!(SqliteStageStore::new(db()).unwrap().all().unwrap().len(), 1);
    let leases_before =
        planeter_store::LeaseRecordStore::list(&SqliteLeaseRecords::new(db()).unwrap()).unwrap();
    assert!(
        leases_before
            .iter()
            .any(|l| l.repo_id == creating.repo_id && l.class == "W"),
        "{leases_before:?}"
    );

    // The next start.
    let second = start(&dir);
    let log = second.log();
    assert!(
        log.contains("start-up sweep: removed the unfinished creation of alice/arch"),
        "{log}"
    );
    assert!(log.contains(&format!("removed stage {stage}")), "{log}");
    assert!(
        log.contains("cleared the lease record of the unfinished creation"),
        "{log}"
    );
    assert!(
        SqliteRepositoryStore::new(db())
            .creating()
            .unwrap()
            .is_empty(),
        "no record"
    );
    assert!(
        SqliteRepositoryStore::new(db())
            .known_ids()
            .unwrap()
            .is_empty()
    );
    assert!(!creating.path.exists(), "no directory");
    assert!(
        dirs_in(&dir.join("repos")).is_empty(),
        "nothing under the repository root: {:?}",
        dirs_in(&dir.join("repos"))
    );
    assert!(
        SqliteStageStore::new(db())
            .unwrap()
            .all()
            .unwrap()
            .is_empty(),
        "no stage record"
    );
    assert!(
        std::fs::read_dir(dir.join("artifacts").join("stages"))
            .map(|d| d.flatten().count())
            .unwrap_or(0)
            == 0,
        "no stage directory"
    );
    let leases =
        planeter_store::LeaseRecordStore::list(&SqliteLeaseRecords::new(db()).unwrap()).unwrap();
    assert!(
        leases.iter().all(|l| l.repo_id != creating.repo_id),
        "{leases:?}"
    );
    assert!(
        !SqliteChangeStore::new(db())
            .unwrap()
            .repo_flags(&RepoId::new(creating.repo_id.as_str()))
            .unwrap()
            .needs_recovery,
        "a repository that never was is not waiting for recovery"
    );
    // Not routable, and the name is free: an empty repository of that name is created at once.
    let (status, _) = http(
        second.port,
        "GET",
        "/api/v1/repos/alice/arch/refs",
        &bearer(),
        b"",
    );
    assert_eq!(status, 404);
    let mut h = bearer();
    h.push(("Content-Type", "application/json".to_owned()));
    let (status, body) = http(
        second.port,
        "POST",
        "/api/v1/repos",
        &h,
        br#"{"owner":"alice","name":"arch","visibility":"private"}"#,
    );
    assert_eq!(status, 201, "{}", text_of(&body));
    let (status, _) = http(
        second.port,
        "GET",
        "/api/v1/repos/alice/arch/refs",
        &bearer(),
        b"",
    );
    assert_eq!(status, 200);
    let mut second = second;
    let _ = second.child.kill();
    let _ = second.child.wait();
}

#[test]
fn a_server_killed_while_an_archive_is_uploaded_leaves_a_stage_directory_the_next_start_removes() {
    if !can_run() {
        return;
    }
    let dir = scratch("upload");
    let (archive, _) = archives::simple_archive(&dir, "maint", 300, 0);
    let bytes = std::fs::read(&archive).unwrap();
    add_token(&dir);
    let mut first = start(&dir);

    // Send the headers and half of the body, and stop talking.
    let meta = r#"{"owner":"alice","name":"half","visibility":"private"}"#;
    let body = frame(meta, &bytes);
    let mut s = TcpStream::connect(("127.0.0.1", first.port)).unwrap();
    let head = format!(
        "POST /api/v1/repos/import HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer tok-alice\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(&body[..body.len() / 2]).unwrap();
    s.flush().unwrap();
    let stages = dir.join("artifacts").join("stages");
    let deadline = Instant::now() + Duration::from_secs(30);
    let stage_dir = loop {
        let found = std::fs::read_dir(&stages)
            .map(|d| d.flatten().next().map(|e| e.path()))
            .ok()
            .flatten();
        if let Some(d) = found
            && dir_bytes(&d) > 1000
        {
            break d;
        }
        assert!(Instant::now() < deadline, "the upload never began");
        std::thread::sleep(Duration::from_millis(20));
    };
    first.child.kill().expect("SIGKILL");
    first.child.wait().unwrap();
    drop(s);
    assert!(
        stage_dir.is_dir(),
        "the half-written stage is on disk after the kill"
    );
    let db = || SqliteDb::open(dir.join("db.sqlite")).unwrap();
    assert!(
        SqliteStageStore::new(db())
            .unwrap()
            .all()
            .unwrap()
            .is_empty(),
        "and has no record"
    );

    let mut second = start(&dir);
    let log = second.log();
    let name = stage_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        log.contains(&format!("removed the stage directory {name}")),
        "{log}"
    );
    assert!(!stage_dir.exists());
    // The principal's stage slot is free: a whole upload now works.
    let mut h = bearer();
    h.push(("Content-Type", "application/octet-stream".to_owned()));
    let (status, reply) = http(second.port, "POST", "/api/v1/repos/import", &h, &body);
    assert_eq!(status, 201, "{}", text_of(&reply));
    let _ = second.child.kill();
    let _ = second.child.wait();
}

#[test]
fn an_archive_creation_completes_through_the_real_server_and_the_real_sandbox() {
    if !can_run() {
        return;
    }
    // The whole path with nothing stubbed: upload, stage (verify and the dry-run import in the sandbox, under `prlimit`), the
    // fingerprint typed, the confirmation (init, trust, import, verify), and the repository served by the same server.
    let dir = scratch("complete");
    let (archive, keys) = archives::simple_archive(&dir, "maint", 40, 64);
    let bytes = std::fs::read(&archive).unwrap();
    add_token(&dir);
    let mut server = start(&dir);
    let mut h = bearer();
    h.push(("Content-Type", "application/octet-stream".to_owned()));
    let meta = r#"{"owner":"alice","name":"whole","visibility":"public"}"#;
    let (status, body) = http(
        server.port,
        "POST",
        "/api/v1/repos/import",
        &h,
        &frame(meta, &bytes),
    );
    assert_eq!(status, 201, "{}", text_of(&body));
    let reply = text_of(&body);
    let stage = json_str(&reply, "stage_id").expect("stage_id");
    assert!(reply.contains(&keys[0].public_key), "{reply}");
    assert!(reply.contains("\"verifies_with_all_keys\":true"), "{reply}");
    // The wrong fingerprint first: refused, nothing made.
    let mut j = bearer();
    j.push(("Content-Type", "application/json".to_owned()));
    let path = format!("/api/v1/repos/import/{stage}/confirm");
    let wrong = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
        keys[0].key_id,
        "9".repeat(64)
    );
    let (status, body) = http(server.port, "POST", &path, &j, wrong.as_bytes());
    assert_eq!(status, 422, "{}", text_of(&body));
    assert!(dirs_in(&dir.join("repos")).is_empty());
    let right = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
        keys[0].key_id, keys[0].public_key
    );
    let (status, body) = http(server.port, "POST", &path, &j, right.as_bytes());
    assert_eq!(status, 201, "{}", text_of(&body));
    let done = text_of(&body);
    assert!(
        done.contains("\"verified\":true") && done.contains("\"landed_as\":\"heads/main\""),
        "{done}"
    );
    // Served by the same server: the published branch, the history, the verification — an anonymous reader (it is public).
    let (status, refs) = http(
        server.port,
        "GET",
        "/api/v1/repos/alice/whole/refs",
        &[],
        b"",
    );
    assert_eq!(status, 200);
    assert!(text_of(&refs).contains("heads/main"), "{}", text_of(&refs));
    let (status, hist) = http(
        server.port,
        "GET",
        "/api/v1/repos/alice/whole/history?limit=5",
        &[],
        b"",
    );
    assert_eq!(status, 200, "{}", text_of(&hist));
    assert!(text_of(&hist).contains("\"entries\""), "{}", text_of(&hist));
    let (status, verify) = http(
        server.port,
        "GET",
        "/api/v1/repos/alice/whole/verify",
        &[],
        b"",
    );
    assert_eq!(status, 200, "{}", text_of(&verify));
    assert!(
        text_of(&verify).contains("verified\":true") || text_of(&verify).contains("\"ok\":true"),
        "{}",
        text_of(&verify)
    );
    assert!(
        std::fs::read_dir(dir.join("artifacts").join("stages"))
            .map(|d| d.flatten().count())
            .unwrap_or(0)
            == 0,
        "the stage is gone"
    );
    let _ = server.child.kill();
    let _ = server.child.wait();
}
