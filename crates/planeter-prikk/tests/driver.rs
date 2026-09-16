//! Integration tests for the subprocess driver (RFC 001 T3), run against a real prikk binary.
//! They **self-skip** when `prikk` is not on `PATH`, so `cargo test` stays green everywhere; a
//! prikk-provisioned CI job runs them for real. Uses `CARGO_TARGET_TMPDIR` (cargo-provided) for repos.

use std::path::PathBuf;
use std::process::Command;

use planeter_prikk::{CliPrikkRepo, PrikkRepo};

fn prikk_available() -> bool {
    Command::new("prikk").arg("--version").output().is_ok()
}

/// Create a fresh initialized prikk repository under the test tmp dir and return its root.
fn init_repo(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    let status = Command::new("prikk")
        .arg("init")
        .current_dir(&dir)
        .status()
        .expect("spawn prikk init");
    assert!(status.success(), "prikk init failed");
    dir
}

#[test]
fn open_pins_the_version_and_reports_it() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = init_repo("driver-version");
    // open() runs the version pin (RFC 001 D-3); success means prikk >= 0.43.0.
    let repo = CliPrikkRepo::open(&dir).expect("open + version pin");
    let v = repo.version().expect("version");
    assert!(
        v.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "version should look like a semver: {v:?}"
    );
}

#[test]
fn reads_parse_to_typed_models_on_a_fresh_repo() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = init_repo("driver-reads");
    let repo = CliPrikkRepo::new(&dir);

    let status = repo.status().expect("status");
    assert_eq!(status.schema_version, "status-report-v1");

    let verify = repo.verify().expect("verify");
    assert_eq!(verify.schema_version, "verify-report-v1");
    // A fresh repo has the full stage list.
    assert!(!verify.stages.is_empty());

    let branches = repo.branches(true).expect("branches");
    assert_eq!(branches.schema_version, "branch-list-v1");

    let tags = repo.tags().expect("tags");
    assert_eq!(tags.schema_version, "tag-list-v1");

    let keys = repo.key_status().expect("key status");
    assert_eq!(keys.schema_version, "key-status-v1");
}
