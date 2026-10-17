//! The pin gate: it passes on the real repository and fails on each planted violation (RFC 010 D-5).
//! Planted fixtures are temporary directories, never files in the repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SHA: &str = "3d3c42e5aac5ba805825da76410c181273ba90b1";
const DIGEST: &str = "sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A scratch repository with the given `(relative path, body)` files.
fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pins-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    for (rel, body) in files {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    root
}

fn run(root: &Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_planeter-pin-check"))
        .arg(root)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap(), text)
}

fn workflow(body: &str) -> String {
    format!("name: t\non: push\njobs:\n  j:\n    runs-on: ubuntu-24.04\n    steps:\n{body}")
}

fn expect_fail(name: &str, files: &[(&str, &str)], needles: &[&str]) {
    let (code, text) = run(&fixture(name, files));
    assert_eq!(code, 1, "{name}: {text}");
    for n in needles {
        assert!(text.contains(n), "{name}: {n:?} missing from:\n{text}");
    }
}

#[test]
fn the_real_repository_passes() {
    let (code, text) = run(&repo_root());
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("OK"), "{text}");
}

#[test]
fn a_fully_pinned_chain_passes() {
    let wf = workflow(&format!(
        "      - uses: actions/checkout@{SHA} # v7.0.1\n      - uses: ./local-action\n      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: 1.99.0\n      - run: rustup component add clippy\n"
    ));
    let df = format!("FROM debian:bookworm-slim@{DIGEST} AS base\nFROM base\nFROM scratch\n");
    let (code, text) = run(&fixture(
        "ok",
        &[
            (".github/workflows/ci.yml", &wf),
            ("Dockerfile", &df),
            ("rust-toolchain.toml", "[toolchain]\nchannel = \"1.99.0\"\n"),
            (
                "README.md",
                "uses: actions/checkout@v7 (prose is not scanned)\n",
            ),
        ],
    ));
    assert_eq!(code, 0, "{text}");
}

#[test]
fn a_mutable_tag_is_named_with_file_and_line() {
    let wf = workflow("      - uses: actions/checkout@v7\n");
    expect_fail(
        "tag",
        &[(".github/workflows/ci.yml", &wf)],
        &[".github/workflows/ci.yml:7:", "\"v7\"", "40-hex"],
    );
}

#[test]
fn a_branch_ref_and_a_missing_ref_and_a_short_sha_fail() {
    let wf = workflow(
        "      - uses: dtolnay/rust-toolchain@stable\n      - uses: actions/checkout\n      - uses: actions/checkout@3d3c42e\n",
    );
    expect_fail(
        "refs",
        &[(".github/workflows/r.yml", &wf)],
        &["\"stable\"", "has no @<commit SHA>", "\"3d3c42e\""],
    );
}

#[test]
fn a_channel_toolchain_fails_in_every_form() {
    let wf = workflow(&format!(
        "      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: stable\n      - run: rustup default nightly\n"
    ));
    expect_fail(
        "channel",
        &[
            (".github/workflows/c.yml", &wf),
            ("rust-toolchain.toml", "[toolchain]\nchannel = \"stable\"\n"),
        ],
        &[
            "toolchain \"stable\" is not an explicit X.Y.Z",
            "rustup selects a toolchain by channel",
            "channel \"stable\" is not an explicit X.Y.Z",
        ],
    );
}

#[test]
fn a_from_without_a_digest_fails() {
    expect_fail(
        "from",
        &[(
            "Dockerfile",
            "FROM debian:bookworm-slim\nFROM --platform=linux/amd64 rust@sha256:abc\n",
        )],
        &[
            "Dockerfile:1:",
            "no @sha256: digest",
            "malformed sha256 digest",
        ],
    );
}

#[test]
fn a_from_built_from_a_variable_fails() {
    expect_fail(
        "arg",
        &[("Dockerfile", "ARG BASE=debian\nFROM ${BASE}\n")],
        &["built from a variable"],
    );
}

#[test]
fn a_service_image_and_a_docker_action_without_a_digest_fail() {
    let wf = workflow(
        "      - uses: docker://alpine:3.20\n      - name: x\n        image: postgres:16\n",
    );
    expect_fail(
        "images",
        &[(".github/workflows/i.yml", &wf)],
        &["docker action", "image \"postgres:16\""],
    );
}

#[test]
fn a_second_toolchain_version_fails_and_names_the_files() {
    let ci = workflow(&format!(
        "      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: 1.99.0\n"
    ));
    let release = workflow(&format!(
        "      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: 1.98.1\n"
    ));
    expect_fail(
        "two-toolchains",
        &[
            (".github/workflows/ci.yml", &ci),
            (".github/workflows/release.yml", &release),
        ],
        &[
            ".github/workflows/ci.yml:9:",
            ".github/workflows/release.yml:9:",
            "disagrees with the other workflows",
            "1.99.0 in .github/workflows/ci.yml",
            "1.98.1 in .github/workflows/release.yml",
        ],
    );
}

#[test]
fn one_toolchain_version_across_workflows_passes() {
    let wf = workflow(&format!(
        "      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: 1.99.0\n"
    ));
    let (code, text) = run(&fixture(
        "one-toolchain",
        &[
            (".github/workflows/ci.yml", &wf),
            (".github/workflows/supply-chain.yml", &wf),
        ],
    ));
    assert_eq!(code, 0, "{text}");
}

#[test]
fn a_moving_runner_label_fails_in_runs_on_and_in_a_matrix() {
    let wf = "name: t\non: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: []\n  b:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        include:\n          - target: x\n            os: macos-latest\n          - target: y\n            os: ubuntu-24.04-arm\n    steps: []\n  c:\n    runs-on: [self-hosted, windows-latest]\n    steps: []\n";
    expect_fail(
        "latest",
        &[(".github/workflows/l.yml", wf)],
        &[
            ".github/workflows/l.yml:5:",
            "\"ubuntu-latest\"",
            ".github/workflows/l.yml:13:",
            "\"macos-latest\"",
            "\"windows-latest\"",
            "3 mutable reference(s)",
        ],
    );
}

#[test]
fn a_bad_root_is_a_usage_error() {
    let (code, _) = run(Path::new("/definitely/not/a/directory"));
    assert_eq!(code, 2);
}

// ---- IS-32: the declared minimum compiler is the `msrv` job's, and nothing else's ----

/// A CI workflow with a build job on `build` and a job `msrv_id` on `msrv`.
fn two_job_ci(msrv_id: &str, build: &str, msrv: &str) -> String {
    format!(
        "name: t\non: push\njobs:\n  build:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: {build}\n  {msrv_id}:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: {msrv}\n"
    )
}

fn manifest(rust_version: &str) -> String {
    format!("[workspace.package]\nedition = \"2024\"\nrust-version = \"{rust_version}\"\n")
}

#[test]
fn the_msrv_job_may_differ_from_the_build_compiler_when_it_equals_rust_version() {
    let ci = two_job_ci("msrv", "1.99.0", "1.88.0");
    for declared in ["1.88.0", "1.88"] {
        let (code, text) = run(&fixture(
            "msrv-ok",
            &[
                (".github/workflows/ci.yml", &ci),
                ("Cargo.toml", &manifest(declared)),
            ],
        ));
        assert_eq!(code, 0, "{declared}: {text}");
    }
}

#[test]
fn a_planted_mismatch_between_the_msrv_job_and_rust_version_fails_and_names_both_files() {
    let ci = two_job_ci("msrv", "1.99.0", "1.87.0");
    expect_fail(
        "msrv-mismatch",
        &[
            (".github/workflows/ci.yml", &ci),
            ("Cargo.toml", &manifest("1.88.0")),
        ],
        &[
            ".github/workflows/ci.yml:15:",
            "job `msrv` builds with \"1.87.0\"",
            "Cargo.toml:3 declares rust-version 1.88.0",
        ],
    );
}

#[test]
fn a_declared_minimum_with_no_msrv_job_fails_and_names_cargo_toml() {
    let ci = workflow(&format!(
        "      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: 1.99.0\n"
    ));
    expect_fail(
        "msrv-missing",
        &[
            (".github/workflows/ci.yml", &ci),
            ("Cargo.toml", &manifest("1.88.0")),
        ],
        &[
            "Cargo.toml:3:",
            "rust-version 1.88.0 is declared but no job `msrv`",
        ],
    );
}

#[test]
fn only_a_job_named_msrv_is_exempt_from_the_one_compiler_rule() {
    // Same two versions, but the second job is not called `msrv`: it is a second compiler.
    let ci = two_job_ci("lint", "1.99.0", "1.88.0");
    expect_fail(
        "msrv-wrong-name",
        &[
            (".github/workflows/ci.yml", &ci),
            ("Cargo.toml", &manifest("1.88.0")),
        ],
        &["disagrees with the other workflows", "no job `msrv`"],
    );
}

#[test]
fn the_msrv_job_does_not_count_towards_the_build_compilers_agreement() {
    // Build jobs in two files agree; the msrv job is a third version and is checked on its own.
    let a = two_job_ci("msrv", "1.99.0", "1.88.0");
    let b = workflow(&format!(
        "      - uses: dtolnay/rust-toolchain@{SHA}\n        with:\n          toolchain: 1.99.0\n"
    ));
    let (code, text) = run(&fixture(
        "msrv-third-version",
        &[
            (".github/workflows/ci.yml", &a),
            (".github/workflows/release.yml", &b),
            ("Cargo.toml", &manifest("1.88.0")),
        ],
    ));
    assert_eq!(code, 0, "{text}");
}

#[test]
fn a_channel_in_the_msrv_job_is_still_refused() {
    let ci = two_job_ci("msrv", "1.99.0", "stable");
    expect_fail(
        "msrv-channel",
        &[
            (".github/workflows/ci.yml", &ci),
            ("Cargo.toml", &manifest("1.88.0")),
        ],
        &["toolchain \"stable\" is not an explicit X.Y.Z version"],
    );
}
