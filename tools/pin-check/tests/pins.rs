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
    format!("name: t\non: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n{body}")
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
fn a_bad_root_is_a_usage_error() {
    let (code, _) = run(Path::new("/definitely/not/a/directory"));
    assert_eq!(code, 2);
}
