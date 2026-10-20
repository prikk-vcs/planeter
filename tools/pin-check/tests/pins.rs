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

// ---------------------------------------------------------------- the rehearsal cannot reach the real package (RFC 010 D-10)

/// A release-shaped workflow: the real publish steps are push-only (one by its job's `if`), the rehearsal
/// steps are rehearsal-only and name only the rehearsal package.
fn release_shaped(extra_rehearsal_step: &str) -> String {
    format!(
        r#"name: r
on:
  push:
    tags: ["[0-9]+.[0-9]+.[0-9]+"]
  workflow_dispatch:
    inputs:
      mode:
        type: choice
        options: [dry-run, rehearsal]
jobs:
  container:
    runs-on: ubuntu-24.04
    steps:
      - uses: docker/login-action@{SHA} # v3
        if: github.event_name == 'push'
      - name: Build image (dry run)
        if: inputs.mode == 'dry-run'
        uses: docker/build-push-action@{SHA}
        with:
          push: false
      - name: Build and push (rehearsal)
        id: rehearsal_push
        if: inputs.mode == 'rehearsal'
        uses: docker/build-push-action@{SHA}
        with:
          push: true
          tags: ghcr.io/prikk-vcs/planeter-rehearsal:rehearsal
      - name: Attest rehearsal image
        if: inputs.mode == 'rehearsal'
        uses: actions/attest-build-provenance@{SHA}
        with:
          subject-name: ghcr.io/prikk-vcs/planeter-rehearsal
          push-to-registry: true
      - name: Build and push
        if: github.event_name == 'push'
        uses: docker/build-push-action@{SHA}
        with:
          push: ${{{{ github.event_name == 'push' }}}}
          tags: |
            ghcr.io/prikk-vcs/planeter:${{{{ github.ref_name }}}}
            ghcr.io/prikk-vcs/planeter:latest
{extra_rehearsal_step}  publish:
    if: github.event_name == 'push'
    runs-on: ubuntu-24.04
    steps:
      - name: Notes
        run: echo "use ghcr.io/prikk-vcs/planeter:$TAG"
      - name: Publish
        run: gh release create "$TAG" --repo prikk-vcs/planeter
"#
    )
}

#[test]
fn a_release_shaped_workflow_with_a_confined_rehearsal_passes() {
    let (code, text) = run(&fixture(
        "rehearsal-ok",
        &[(".github/workflows/release.yml", &release_shaped(""))],
    ));
    assert_eq!(code, 0, "{text}");
}

#[test]
fn a_step_naming_the_real_image_without_the_push_condition_fails() {
    // The real image named by a step that also runs in a dispatch…
    let wf = release_shaped("").replace(
        "      - name: Build and push\n        if: github.event_name == 'push'\n",
        "      - name: Build and push\n",
    );
    expect_fail(
        "real-no-push",
        &[(".github/workflows/release.yml", &wf)],
        &[
            "a step naming the real image",
            "github.event_name == 'push'",
        ],
    );
    // …or by a step whose condition names a dispatch mode instead.
    let wf = release_shaped("").replace(
        "      - name: Build and push\n        if: github.event_name == 'push'\n",
        "      - name: Build and push\n        if: inputs.mode == 'dry-run'\n",
    );
    expect_fail(
        "real-in-dry-run",
        &[(".github/workflows/release.yml", &wf)],
        &["a step naming the real image"],
    );
    // The job-level `if` counts: removing the publish job's makes its note step fail.
    let wf = release_shaped("").replace(
        "  publish:\n    if: github.event_name == 'push'\n",
        "  publish:\n",
    );
    expect_fail(
        "job-no-push",
        &[(".github/workflows/release.yml", &wf)],
        &["a step naming the real image"],
    );
}

#[test]
fn a_step_naming_both_the_real_image_and_the_rehearsal_package_fails() {
    let wf = release_shaped("").replace(
        "          tags: ghcr.io/prikk-vcs/planeter-rehearsal:rehearsal\n",
        "          tags: |\n            ghcr.io/prikk-vcs/planeter-rehearsal:rehearsal\n            ghcr.io/prikk-vcs/planeter:latest\n",
    );
    expect_fail(
        "both",
        &[(".github/workflows/release.yml", &wf)],
        &["names both the real image"],
    );
}

#[test]
fn a_rehearsal_step_that_can_also_run_on_a_tag_push_fails() {
    for (name, cond) in [
        (
            "or-push",
            "inputs.mode == 'rehearsal' || github.event_name == 'push'",
        ),
        (
            "and-push",
            "inputs.mode == 'rehearsal' && github.event_name == 'push'",
        ),
    ] {
        let wf = release_shaped("").replace(
            "        id: rehearsal_push\n        if: inputs.mode == 'rehearsal'\n",
            &format!("        id: rehearsal_push\n        if: {cond}\n"),
        );
        expect_fail(
            name,
            &[(".github/workflows/release.yml", &wf)],
            &["a rehearsal step can also run on a tag push"],
        );
    }
}

#[test]
fn a_step_naming_the_rehearsal_package_without_the_rehearsal_condition_fails() {
    let wf = release_shaped("").replace(
        "        id: rehearsal_push\n        if: inputs.mode == 'rehearsal'\n",
        "        id: rehearsal_push\n",
    );
    expect_fail(
        "rehearsal-unconditioned",
        &[(".github/workflows/release.yml", &wf)],
        &["a step in the rehearsal path must have inputs.mode == 'rehearsal'"],
    );
}

#[test]
fn a_rehearsal_step_that_pushes_somewhere_else_fails() {
    let step = "      - name: Push elsewhere\n        if: inputs.mode == 'rehearsal'\n        uses: docker/build-push-action@3d3c42e5aac5ba805825da76410c181273ba90b1\n        with:\n          push: true\n          tags: ghcr.io/someone/else:x\n";
    expect_fail(
        "push-elsewhere",
        &[(".github/workflows/release.yml", &release_shaped(step))],
        &["pushes to something other than ghcr.io/prikk-vcs/planeter-rehearsal"],
    );
    let step = "      - name: Push by hand\n        if: inputs.mode == 'rehearsal'\n        run: docker push ghcr.io/someone/else:x\n";
    expect_fail(
        "docker-push",
        &[(".github/workflows/release.yml", &release_shaped(step))],
        &["pushes to something other than"],
    );
}

#[test]
fn a_rehearsal_step_that_names_a_release_a_tag_or_crates_io_fails() {
    for (name, cmd) in [
        ("gh-release", "gh release create x"),
        ("git-tag", "git tag -s x"),
        ("git-push", "git push origin x"),
        ("cargo-publish", "cargo publish -p planeter"),
        ("crates", "curl https://crates.io/api/v1/crates"),
    ] {
        let step = format!(
            "      - name: Oops\n        if: inputs.mode == 'rehearsal'\n        run: {cmd}\n"
        );
        expect_fail(
            name,
            &[(".github/workflows/release.yml", &release_shaped(&step))],
            &["a rehearsal step names a release, tag or crates.io"],
        );
    }
}
