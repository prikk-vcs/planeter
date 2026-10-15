//! IS-16 T2 and T5, at the binary: the content-origin start-up rule, and the single list of
//! environment variables.
//!
//! Every run uses a **cleared environment** and a scratch working directory. A run that must refuse is
//! given a timeout that fails the test if the process does not exit; a run that must serve is killed
//! once it announces itself.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

const REFUSAL: &str = "planeter: refusing to start: raw repository content would be served from \
the application's own host, which breaks the isolation the documentation promises. Set \
PLANETER_CONTENT_ORIGIN to a different host (for example https://raw.example.org).";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("startup-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn command(cwd: &PathBuf, envs: &[(&str, &str)], args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_planeter"));
    c.args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

/// Run to completion; `(exit code, stderr)`. Fails if the process is still running at the timeout.
fn run_to_exit(name: &str, envs: &[(&str, &str)]) -> (i32, String) {
    let cwd = scratch(name);
    let mut child: Child = command(&cwd, envs, &[]).spawn().expect("spawn planeter");
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            panic!("planeter did not exit: it started a server");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().expect("exit code"),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// Start, read stderr up to the "listening" line, kill; return the lines read.
fn serve_and_collect(name: &str, envs: &[(&str, &str)]) -> Vec<String> {
    let cwd = scratch(name);
    let mut child = command(&cwd, envs, &[]).spawn().expect("spawn planeter");
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let done = line.starts_with("planeter listening on");
            let _ = tx.send(line);
            if done {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(l) => {
                let done = l.starts_with("planeter listening on");
                lines.push(l);
                if done {
                    break;
                }
            }
            Err(_) => {
                let _ = child.kill();
                panic!("planeter never announced itself; got {lines:?}");
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    lines
}

fn env_for<'a>(
    cwd_tag: &str,
    addr: &'a str,
    extra: &[(&'a str, &'a str)],
) -> Vec<(String, String)> {
    let dir = scratch(&format!("{cwd_tag}-data"));
    let mut v = vec![
        ("PLANETER_ADDR".to_owned(), addr.to_owned()),
        (
            "PLANETER_REPOS_ROOT".to_owned(),
            dir.join("repos").display().to_string(),
        ),
        (
            "PLANETER_DB".to_owned(),
            dir.join("db.sqlite").display().to_string(),
        ),
    ];
    v.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    v
}

fn as_refs(v: &[(String, String)]) -> Vec<(&str, &str)> {
    v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
}

#[test]
fn a_non_loopback_bind_with_the_application_host_as_content_origin_refuses_to_start() {
    // Default content origin (http://<addr>): the application's own host.
    let e = env_for(
        "refuse-default",
        "0.0.0.0:0",
        &[("PLANETER_TRUSTED_PROXIES", "10.0.0.0/8")],
    );
    let (code, stderr) = run_to_exit("refuse-default", &as_refs(&e));
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(stderr.trim_end(), REFUSAL);
    assert!(!stderr.contains("listening"));
}

#[test]
fn a_port_only_difference_on_a_non_loopback_bind_refuses_to_start() {
    let e = env_for(
        "refuse-port",
        "0.0.0.0:0",
        &[
            ("PLANETER_TRUSTED_PROXIES", "10.0.0.0/8"),
            ("PLANETER_CONTENT_ORIGIN", "https://0.0.0.0:8443"),
        ],
    );
    let (code, stderr) = run_to_exit("refuse-port", &as_refs(&e));
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(stderr.trim_end(), REFUSAL);
}

#[test]
fn a_distinct_content_host_starts_without_a_warning() {
    let e = env_for(
        "distinct",
        "127.0.0.1:0",
        &[("PLANETER_CONTENT_ORIGIN", "https://raw.example.org")],
    );
    let lines = serve_and_collect("distinct", &as_refs(&e));
    assert!(
        !lines.iter().any(|l| l.contains("warning")),
        "unexpected warning: {lines:?}"
    );
}

#[test]
fn the_same_host_on_loopback_starts_with_exactly_one_warning() {
    let e = env_for("loopback", "127.0.0.1:0", &[]);
    let lines = serve_and_collect("loopback", &as_refs(&e));
    let warnings: Vec<_> = lines.iter().filter(|l| l.contains("warning")).collect();
    assert_eq!(warnings.len(), 1, "{lines:?}");
    assert!(warnings[0].contains("PLANETER_CONTENT_ORIGIN"));
    assert!(warnings[0].contains("local development"));
}

// ---------------------------------------------------------------- T5: one list of variables

/// Every `PLANETER_*` name appearing in `text`, where a name is `PLANETER_` + `[A-Z0-9_]+`.
fn names_in(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find("PLANETER_") {
        let start = i + off;
        let mut end = start + "PLANETER_".len();
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        found.insert(text[start..end].trim_end_matches('_').to_owned());
        i = end;
    }
    found
}

#[test]
fn usage_lists_exactly_the_variables_main_reads() {
    let main =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
            .unwrap();
    // Names read by the binary: string literals in main.rs outside comments.
    let code: String = main
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let read = names_in(&code);
    assert!(
        read.contains("PLANETER_ADDR"),
        "the scan found nothing: {read:?}"
    );

    let out = Command::new(env!("CARGO_BIN_EXE_planeter"))
        .arg("--help")
        .env_clear()
        .output()
        .unwrap();
    assert!(out.status.success());
    let usage = String::from_utf8(out.stdout).unwrap();
    let listed = names_in(&usage);

    let missing: Vec<_> = read.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&read).collect();
    assert!(
        missing.is_empty(),
        "read by main.rs but not in USAGE: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "in USAGE but not read by main.rs: {stale:?}"
    );
}

#[test]
fn main_rs_no_longer_carries_its_own_copy_of_the_list() {
    let main =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
            .unwrap();
    let doc_names: Vec<_> = main
        .lines()
        .filter(|l| l.trim_start().starts_with("//!"))
        .flat_map(names_in)
        .collect();
    assert!(
        doc_names.is_empty(),
        "the header comment lists variables again: {doc_names:?}"
    );
    assert!(main.contains("--help"));
}
