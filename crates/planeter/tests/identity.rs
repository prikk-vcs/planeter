//! RFC 010 D-6 / IS-1: the binary identifies itself and never starts a server by accident.
//!
//! Every invocation runs the built binary with a **cleared environment** in a scratch working
//! directory, under a timeout that fails the test if the process does not exit (a server would not).
//! The one invocation that is allowed to serve (no argument) gets loopback port 0 and scratch paths,
//! and is killed once it announces itself.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("identity-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn command(cwd: &PathBuf, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_planeter"));
    c.args(args)
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Run to completion; kill and fail if it is still running at the timeout (a server was started).
fn run(name: &str, args: &[&str]) -> (i32, String, String, PathBuf) {
    let cwd = scratch(name);
    let mut child: Child = command(&cwd, args).spawn().expect("spawn planeter");
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            panic!("planeter {args:?} did not exit: it started a server");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().expect("exit code"),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        cwd,
    )
}

fn assert_no_side_effects(cwd: &PathBuf) {
    let entries: Vec<_> = std::fs::read_dir(cwd).unwrap().collect();
    assert!(entries.is_empty(), "the process created files: {entries:?}");
}

fn assert_identity(stdout: &str) {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout:?}");
    assert_eq!(lines[0], format!("planeter {}", env!("CARGO_PKG_VERSION")));
    // The commit is `unknown` unless PLANETER_GIT_COMMIT was set when this test binary was built.
    let commit = lines[1].strip_prefix("commit ").expect("commit line");
    assert!(
        !commit.is_empty() && !commit.contains(char::is_whitespace),
        "{commit:?}"
    );
    let floor = lines[2].strip_prefix("prikk-floor ").expect("floor line");
    let (maj, min, pat) = planeter_prikk::cli::MIN_PRIKK_VERSION;
    assert_eq!(
        floor,
        format!("{maj}.{min}.{pat}"),
        "the floor is read from the constant"
    );
}

#[test]
fn long_version_flag_prints_identity_and_exits_zero() {
    let (code, out, err, cwd) = run("version-long", &["--version"]);
    assert_eq!(code, 0, "{err}");
    assert_identity(&out);
    assert_eq!(err, "");
    assert_no_side_effects(&cwd);
}

#[test]
fn short_version_flag_prints_the_same() {
    let (code, out, _, cwd) = run("version-short", &["-V"]);
    assert_eq!(code, 0);
    assert_identity(&out);
    assert_no_side_effects(&cwd);
    let (_, long, _, _) = run("version-long2", &["--version"]);
    assert_eq!(out, long);
}

#[test]
fn help_flags_print_usage_naming_the_environment_and_exit_zero() {
    for flag in ["--help", "-h"] {
        let (code, out, err, cwd) = run(&format!("help{flag}"), &[flag]);
        assert_eq!(code, 0, "{err}");
        assert!(out.starts_with("usage: planeter"), "{out}");
        for var in [
            "PLANETER_ADDR",
            "PLANETER_REPOS_ROOT",
            "PLANETER_CONTENT_ORIGIN",
            "PLANETER_DB",
            "PLANETER_TRUSTED_PROXIES",
            "PLANETER_OIDC_ISSUER",
            "PLANETER_OIDC_CLIENT_SECRET",
        ] {
            assert!(out.contains(var), "{var} missing from usage");
        }
        assert_no_side_effects(&cwd);
    }
}

#[test]
fn an_unknown_argument_is_refused_with_usage_and_exit_two() {
    for args in [
        &["--serve"][..],
        &["serve"],
        &["--version", "--help"],
        &["-x"],
    ] {
        let (code, out, err, cwd) = run(&format!("unknown-{}", args.join("_")), args);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert_eq!(out, "", "nothing on stdout for a refusal");
        // One error line that does not echo the argument, then the usage.
        let mut lines = err.lines();
        assert_eq!(lines.next(), Some("planeter: unrecognized argument"));
        assert!(err.contains("usage: planeter"), "{err}");
        assert_no_side_effects(&cwd);
    }
}

#[test]
fn no_argument_serves_as_before() {
    let cwd = scratch("serve");
    let mut child = command(&cwd, &[])
        .env("PLANETER_ADDR", "127.0.0.1:0")
        .env("PLANETER_DB", cwd.join("p.db"))
        .env("PLANETER_REPOS_ROOT", cwd.join("repos"))
        .spawn()
        .expect("spawn planeter");
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // Start-up lines up to and including the announcement.
        let mut seen = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok).take(8) {
            let done = line.starts_with("planeter listening on ");
            seen.push_str(&line);
            seen.push('\n');
            if done {
                break;
            }
        }
        let _ = tx.send(seen);
    });
    let announced = rx.recv_timeout(TIMEOUT);
    let _ = child.kill();
    let _ = child.wait();
    let seen = announced.expect("planeter announced itself within the timeout");
    assert!(
        seen.contains("planeter listening on http://127.0.0.1:"),
        "{seen}"
    );
    // The prikk concurrency cap and wait are logged once at start (IS-4).
    assert!(seen.contains("concurrent prikk invocations"), "{seen}");
}
