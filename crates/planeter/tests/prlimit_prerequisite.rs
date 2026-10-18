//! RFC 011 D-2 (T2): `prlimit` (util-linux) is a runtime prerequisite, checked at start. A missing one
//! refuses to start with a message that names `prlimit` and util-linux — and `--version`/`--help` stay
//! independent of it (they never read configuration or start anything).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("prlimit-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run the binary with a cleared environment whose PATH is an empty directory (so `prlimit` cannot be
/// found), under a timeout that fails the test if the process is still running (it started serving).
fn run_without_prlimit(tag: &str, args: &[&str]) -> (i32, String, String) {
    // Per-test directories: the tests run in parallel and `scratch` recreates its directory.
    let empty_path = scratch(&format!("{tag}-empty-path"));
    let cwd = scratch(&format!("{tag}-cwd"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_planeter"))
        .args(args)
        .env_clear()
        .env("PATH", &empty_path)
        .env("PLANETER_ADDR", "127.0.0.1:0")
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("planeter {args:?} did not exit: it started a server without prlimit");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().expect("exit code"),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn a_missing_prlimit_refuses_to_start_and_names_it_and_util_linux() {
    let (code, _, stderr) = run_without_prlimit("refuse", &[]);
    assert_eq!(code, 1, "{stderr}");
    let line = stderr.trim_end();
    assert!(
        line.starts_with("planeter: refusing to start: `prlimit` (part of util-linux) is required to bound the memory of prikk calls, but it could not be run ("),
        "{line}"
    );
    assert!(line.contains("Install util-linux"), "{line}");
    assert_eq!(line.lines().count(), 1, "one plain message: {line}");
    assert!(!stderr.contains("listening"), "{stderr}");
}

#[test]
fn version_and_help_do_not_need_prlimit() {
    for flag in ["--version", "--help"] {
        let (code, stdout, stderr) = run_without_prlimit(&format!("flag{flag}"), &[flag]);
        assert_eq!(code, 0, "{flag}: {stderr}");
        assert!(
            !stdout.is_empty() && stderr.is_empty(),
            "{flag}: {stdout:?} {stderr:?}"
        );
    }
}
