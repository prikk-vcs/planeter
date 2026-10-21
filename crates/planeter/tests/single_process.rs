//! RFC 004 D-7, end to end: **one server process per database.** The lease is in-process, so a second server on
//! the same database is refused at start with a message that says what to do; a killed server's leftover lock
//! file is recognised as stale; a start that is refused for another reason does not leave its lock behind.
//!
//! Self-skips (printed reason) where `prlimit` cannot run.

use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("single-process-{name}"));
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

/// A stand-in `prikk` that answers the version pin (the server refuses to start without one).
fn stand_in_bin(dir: &Path) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("prikk");
    std::fs::write(&script, "#!/bin/sh\necho 'prikk 0.52.0'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin_dir
}

fn command(dir: &Path, bin_dir: &Path, port: u16) -> Command {
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
    c
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Start a server that is expected to come up; returns it once it prints that it is listening.
fn start_serving(dir: &Path, bin_dir: &Path) -> Child {
    let port = free_port();
    let mut server = command(dir, bin_dir, port).spawn().expect("spawn planeter");
    let stderr = server.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let mut seen = Vec::new();
    let mut listening = false;
    for line in lines.by_ref().map_while(Result::ok) {
        let up = line.starts_with("planeter listening on ");
        seen.push(line);
        if up {
            listening = true;
            break;
        }
    }
    std::thread::spawn(move || for _ in lines {});
    assert!(listening, "the server did not come up: {seen:?}");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(25));
    }
    server
}

/// Run a server that is expected to refuse; returns (exit code, stderr).
fn start_refused(dir: &Path, bin_dir: &Path, extra_env: &[(&str, &str)]) -> (Option<i32>, String) {
    let mut c = command(dir, bin_dir, free_port());
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let out = c.output().expect("run planeter");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn lock_file(dir: &Path) -> PathBuf {
    dir.join("db.sqlite.lock")
}

#[test]
fn a_second_server_on_one_database_is_refused_and_a_killed_ones_lock_is_stale() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("second");
    let bin_dir = stand_in_bin(&dir);

    let mut first = start_serving(&dir, &bin_dir);
    let pid = first.id();
    assert_eq!(
        std::fs::read_to_string(lock_file(&dir)).unwrap().trim(),
        pid.to_string(),
        "the lock names the running server"
    );

    // A second server against the same database: refused, exit 1, and the message says why and what to do.
    let (code, stderr) = start_refused(&dir, &bin_dir, &[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("refusing to start"), "{stderr}");
    assert!(stderr.contains(&format!("process {pid}")), "{stderr}");
    assert!(
        stderr.contains("only one server process per database"),
        "{stderr}"
    );
    assert!(stderr.contains("db.sqlite.lock"), "{stderr}");
    // The refused one did not touch the first's lock.
    assert_eq!(
        std::fs::read_to_string(lock_file(&dir)).unwrap().trim(),
        pid.to_string()
    );

    // SIGKILL leaves the file behind; the next start sees that its process is gone and carries on.
    first.kill().expect("SIGKILL");
    let _ = first.wait();
    assert!(lock_file(&dir).exists(), "a killed server cannot clean up");
    let mut third = start_serving(&dir, &bin_dir);
    assert_eq!(
        std::fs::read_to_string(lock_file(&dir)).unwrap().trim(),
        third.id().to_string()
    );
    let _ = third.kill();
    let _ = third.wait();
}

#[test]
fn a_start_refused_for_another_reason_does_not_leave_its_lock_behind() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("other-refusal");
    let bin_dir = stand_in_bin(&dir);
    // An incomplete OIDC setting is refused after the lock was taken (it is read after the database is opened).
    let (code, stderr) = start_refused(
        &dir,
        &bin_dir,
        &[("PLANETER_OIDC_ISSUER", "https://issuer.invalid")],
    );
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("PLANETER_OIDC_CLIENT_ID"), "{stderr}");
    assert!(
        !lock_file(&dir).exists(),
        "a refused start must not leave the lock file"
    );
}
