//! RFC 005 part 5a T4: the prikk binary is asked its version once at start-up and the floor is applied there. A prikk below
//! `MIN_PRIKK_VERSION` refuses to start with the pin's own sentence (it used to start and answer every read with
//! "temporarily unavailable"); a missing prikk is reported when a repository is first opened, as it always was.
//!
//! Self-skips (printed reason) where `prlimit` cannot run.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("prikk-floor-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn prlimit_runs() -> bool {
    Command::new("prlimit")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A stand-in `prikk` that prints `script_body` for any call.
fn stand_in_bin(dir: &Path, version_line: &str) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("prikk");
    std::fs::write(&script, format!("#!/bin/sh\necho '{version_line}'\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A parallel test thread's fork can hold the file open for writing for a moment (ETXTBSY): wait until it runs.
    for _ in 0..200 {
        match Command::new(&script).arg("--version").output() {
            Ok(_) => break,
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("{e}"),
        }
    }
    bin_dir
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Start the server with `path` as its `PATH`; returns the lines it printed until it exited or said it was listening, and
/// its exit code (`None` when it was still running, in which case it is killed).
fn start(dir: &Path, path: &str) -> (Vec<String>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_planeter"))
        .env_clear()
        .env("PATH", path)
        .env("PLANETER_ADDR", format!("127.0.0.1:{}", free_port()))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    let end = Instant::now() + Duration::from_secs(20);
    let mut code = None;
    loop {
        while let Ok(l) = rx.try_recv() {
            lines.push(l);
        }
        if lines
            .iter()
            .any(|l| l.starts_with("planeter listening on "))
        {
            break;
        }
        if let Some(st) = child.try_wait().unwrap() {
            code = Some(st.code().expect("exit code"));
            while let Ok(l) = rx.recv_timeout(Duration::from_millis(200)) {
                lines.push(l);
            }
            break;
        }
        assert!(
            Instant::now() < end,
            "neither exited nor listening: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    (lines, code)
}

#[test]
fn a_prikk_below_the_floor_refuses_to_start_with_the_pins_sentence() {
    if !prlimit_runs() {
        eprintln!("skipping: needs prlimit");
        return;
    }
    for version in ["0.53.0", "0.52.0", "0.46.0"] {
        let dir = scratch(&format!("below-{version}"));
        let bin = stand_in_bin(&dir, &format!("prikk {version}"));
        let (lines, code) = start(&dir, &format!("{}:/usr/bin:/bin", bin.display()));
        assert_eq!(code, Some(1), "{version}: {lines:?}");
        assert_eq!(
            lines,
            vec![format!(
                "planeter: refusing to start: unsupported prikk version {version}; planeter requires >= 0.54.0"
            )],
            "one plain sentence, nothing else, no 'listening'"
        );
    }
}

#[test]
fn the_floor_itself_and_a_newer_prikk_start() {
    if !prlimit_runs() {
        eprintln!("skipping: needs prlimit");
        return;
    }
    for version in ["0.54.0", "0.54.1", "0.60.3", "1.0.0"] {
        let dir = scratch(&format!("ok-{version}"));
        let bin = stand_in_bin(&dir, &format!("prikk {version}"));
        let (lines, code) = start(&dir, &format!("{}:/usr/bin:/bin", bin.display()));
        assert_eq!(code, None, "{version}: {lines:?}");
        assert!(
            !lines.iter().any(|l| l.contains("refusing to start")),
            "{version}: {lines:?}"
        );
    }
}

#[test]
fn a_missing_prikk_is_not_a_refusal_at_start() {
    if !prlimit_runs() {
        eprintln!("skipping: needs prlimit");
        return;
    }
    let dir = scratch("missing");
    // PATH has `prlimit` (in /usr/bin) but the stand-in directory is empty and no prikk is installed there: a machine
    // that has prikk in /usr/bin would find it, so the test only runs where it does not.
    if Command::new("/usr/bin/prikk")
        .arg("--version")
        .output()
        .is_ok()
    {
        eprintln!("skipping: a prikk is installed in /usr/bin");
        return;
    }
    let empty = dir.join("bin");
    std::fs::create_dir_all(&empty).unwrap();
    let (lines, code) = start(&dir, &format!("{}:/usr/bin:/bin", empty.display()));
    assert_eq!(code, None, "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains("refusing to start")),
        "{lines:?}"
    );
}

#[test]
fn a_prikk_that_answers_something_unreadable_is_said_once_and_left_to_the_pin() {
    if !prlimit_runs() {
        eprintln!("skipping: needs prlimit");
        return;
    }
    let dir = scratch("unreadable");
    let bin = stand_in_bin(&dir, "this is not a version line");
    let (lines, code) = start(&dir, &format!("{}:/usr/bin:/bin", bin.display()));
    assert_eq!(code, None, "{lines:?}");
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("planeter: warning: could not read the version of prikk"))
            .count(),
        1,
        "{lines:?}"
    );
}
