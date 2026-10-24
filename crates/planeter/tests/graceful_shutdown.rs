//! RFC 004 part 2, T0(b): the server shuts down **gracefully on SIGTERM and SIGINT** — it stops accepting, lets the
//! requests in flight finish within a bounded drain (`PLANETER_SHUTDOWN_DRAIN_SECS`), removes its lock file, and
//! exits 0; a second signal, or the end of the drain, stops it at once (exit 1). Real binary, real signals.
//! The in-flight cases need bubblewrap (the slow request is a read in the production sandbox) and self-skip
//! with a printed reason without it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, Visibility,
};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("graceful-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

/// A stand-in `prikk`: answers the version pin, and for any other verb sleeps `secs` seconds then prints nothing.
fn stand_in(dir: &Path, secs: u32) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("prikk");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.54.1';; *) sleep {secs};; esac\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin_dir
}

/// A public repository record pointing at an empty store directory (admission measures `.prikk`).
fn seed(dir: &Path) -> PathBuf {
    let repo_dir = dir.join("repo");
    std::fs::create_dir_all(repo_dir.join(".prikk")).unwrap();
    std::fs::write(repo_dir.join(".prikk/FORMAT"), b"7\n").unwrap();
    let db = SqliteDb::open(dir.join("db.sqlite")).expect("db");
    SqliteRepositoryStore::new(db)
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "open".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: repo_dir.clone(),
        })
        .expect("seed record");
    repo_dir
}

fn start(dir: &Path, bin_dir: &Path, drain: Option<&str>) -> (Child, u16) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut c = Command::new(env!("CARGO_BIN_EXE_planeter"));
    c.env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(d) = drain {
        c.env("PLANETER_SHUTDOWN_DRAIN_SECS", d);
    }
    let mut server = c.spawn().expect("spawn planeter");
    let lines = BufReader::new(server.stderr.take().unwrap());
    // Keep the log: the reader thread hands it back through a file next to the database.
    let log = dir.join("server.log");
    let log2 = log.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut f = std::fs::File::create(&log2).unwrap();
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            let _ = writeln!(f, "{line}");
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("the server never listened");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    (server, port)
}

fn signal(child: &Child, name: &str) {
    let ok = Command::new("kill")
        .args([&format!("-{name}"), &child.id().to_string()])
        .status()
        .unwrap();
    assert!(ok.success());
}

fn wait_exit(child: &mut Child, limit: Duration) -> Option<i32> {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if let Some(st) = child.try_wait().unwrap() {
            return Some(st.code().unwrap_or(-1));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

fn log_of(dir: &Path) -> String {
    std::thread::sleep(Duration::from_millis(100));
    std::fs::read_to_string(dir.join("server.log")).unwrap_or_default()
}

fn lock(dir: &Path) -> PathBuf {
    dir.join("db.sqlite.lock")
}

/// Processes of the slow call against `root` (the sandbox's command line names it).
fn procs_of(root: &Path) -> Vec<u32> {
    let root_s = root.to_string_lossy().into_owned();
    let me = std::process::id();
    let mut found = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == me {
            continue;
        }
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        if !matches!(comm.trim(), "bwrap" | "sleep") {
            continue;
        }
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        if String::from_utf8_lossy(&cmd)
            .replace('\0', " ")
            .contains(&root_s)
        {
            found.push(pid);
        }
    }
    found
}

fn wait_for(limit: Duration, mut ok: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    ok()
}

fn slow_request(port: u16) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = s.write_all(
            b"GET /api/v1/repos/alice/open/history HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

#[test]
fn an_idle_server_stops_at_once_on_sigterm_and_on_sigint_and_removes_its_lock() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    for sig in ["TERM", "INT"] {
        let dir = scratch(&format!("idle-{sig}"));
        let bin = stand_in(&dir, 1);
        let (mut server, _) = start(&dir, &bin, None);
        assert!(lock(&dir).exists(), "the lock is held while it runs");
        let t = Instant::now();
        signal(&server, sig);
        let code = wait_exit(&mut server, Duration::from_secs(5));
        assert_eq!(code, Some(0), "SIG{sig}: {}", log_of(&dir));
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "idle must stop at once: {:?}",
            t.elapsed()
        );
        assert!(!lock(&dir).exists(), "SIG{sig}: the lock file is removed");
        let log = log_of(&dir);
        assert!(
            log.contains(&format!("SIG{sig} received")) && log.contains("planeter: stopped"),
            "{log}"
        );
    }
}

#[test]
fn a_request_in_flight_finishes_when_sigterm_arrives_and_then_the_server_exits_cleanly() {
    if !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap (with user namespaces) and prlimit");
        return;
    }
    let dir = scratch("inflight");
    let bin = stand_in(&dir, 3);
    let repo = seed(&dir);
    let (mut server, port) = start(&dir, &bin, None);
    let client = slow_request(port);
    assert!(
        wait_for(Duration::from_secs(15), || !procs_of(&repo).is_empty()),
        "the read never reached the sandbox"
    );
    let t = Instant::now();
    signal(&server, "TERM");
    // No new connection is accepted once it is draining.
    assert!(
        wait_for(Duration::from_secs(2), || TcpStream::connect((
            "127.0.0.1",
            port
        ))
        .is_err()),
        "a draining server must stop accepting"
    );
    assert!(
        server.try_wait().unwrap().is_none(),
        "it must wait for the request in flight"
    );
    let response = client.join().unwrap();
    assert!(
        response.starts_with("HTTP/1.1 "),
        "the request in flight got its answer: {response:?}"
    );
    let code = wait_exit(&mut server, Duration::from_secs(10));
    assert_eq!(code, Some(0), "{}", log_of(&dir));
    assert!(
        t.elapsed() >= Duration::from_secs(2),
        "it waited for the slow request: {:?}",
        t.elapsed()
    );
    assert!(!lock(&dir).exists(), "the lock file is gone");
}

#[test]
fn a_second_signal_stops_at_once_and_so_does_the_end_of_the_drain() {
    if !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap (with user namespaces) and prlimit");
        return;
    }
    // Second signal.
    let dir = scratch("second");
    let bin = stand_in(&dir, 60);
    let repo = seed(&dir);
    let (mut server, port) = start(&dir, &bin, None);
    let client = slow_request(port);
    assert!(wait_for(Duration::from_secs(15), || !procs_of(&repo).is_empty()));
    signal(&server, "TERM");
    std::thread::sleep(Duration::from_millis(300));
    assert!(server.try_wait().unwrap().is_none());
    signal(&server, "TERM");
    assert_eq!(
        wait_exit(&mut server, Duration::from_secs(5)),
        Some(1),
        "{}",
        log_of(&dir)
    );
    assert!(!lock(&dir).exists());
    assert!(
        wait_for(Duration::from_secs(5), || procs_of(&repo).is_empty()),
        "no sandbox process survives"
    );
    let _ = client.join();
    assert!(log_of(&dir).contains("received again: stopping at once"));

    // The end of the drain.
    let dir = scratch("deadline");
    let bin = stand_in(&dir, 60);
    let repo = seed(&dir);
    let (mut server, port) = start(&dir, &bin, Some("1"));
    let client = slow_request(port);
    assert!(wait_for(Duration::from_secs(15), || !procs_of(&repo).is_empty()));
    let t = Instant::now();
    signal(&server, "INT");
    assert_eq!(
        wait_exit(&mut server, Duration::from_secs(6)),
        Some(1),
        "{}",
        log_of(&dir)
    );
    assert!(
        t.elapsed() < Duration::from_secs(4),
        "the drain is bounded by the setting: {:?}",
        t.elapsed()
    );
    assert!(!lock(&dir).exists());
    assert!(wait_for(Duration::from_secs(5), || procs_of(&repo).is_empty()));
    let _ = client.join();
    assert!(log_of(&dir).contains("the drain of 1 s ended"));
}

#[test]
fn a_bad_drain_setting_is_refused_at_start_and_leaves_no_lock() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("bad");
    let bin = stand_in(&dir, 1);
    for bad in ["0", "-1", "soon", "1.5"] {
        let out = Command::new(env!("CARGO_BIN_EXE_planeter"))
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("PLANETER_ADDR", "127.0.0.1:0")
            .env("PLANETER_REPOS_ROOT", dir.join("repos"))
            .env("PLANETER_DB", dir.join("db.sqlite"))
            .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
            .env("PLANETER_SHUTDOWN_DRAIN_SECS", bad)
            .current_dir(&dir)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{bad}: {stderr}");
        assert!(stderr.contains("PLANETER_SHUTDOWN_DRAIN_SECS"), "{stderr}");
        assert!(!lock(&dir).exists());
    }
}
