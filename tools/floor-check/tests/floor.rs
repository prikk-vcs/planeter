//! The floor gate: it passes on the real repository and fails on planted violations (RFC 010 D-2).
//! Planted fixtures are temporary directories, never files in the repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A scratch repository holding the four files with the given floors (the CI workflow declares the release workflow's).
fn fixture(name: &str, cli: &str, release: &str, docker: &str) -> PathBuf {
    fixture_with_ci(name, cli, release, release, docker)
}

fn fixture_with_ci(name: &str, cli: &str, release: &str, ci: &str, docker: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("floor-{name}"));
    let _ = fs::remove_dir_all(&root);
    for (rel, body) in [
        ("crates/planeter-prikk/src/cli.rs", cli),
        (".github/workflows/release.yml", release),
        (".github/workflows/ci.yml", ci),
        ("Dockerfile", docker),
    ] {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    root
}

fn cli(v: &str) -> String {
    format!(
        "/// 0.99.0 is mentioned here and is not a floor\npub const MIN_PRIKK_VERSION: (u64, u64, u64) = ({v});\n"
    )
}
fn release(v: &str) -> String {
    format!("env:\n  # PRIKK_VERSION: \"9.9.9\" in a comment\n  PRIKK_VERSION: \"{v}\"\n")
}
fn docker(v: &str) -> String {
    format!("FROM debian\nARG PRIKK_VERSION={v}\n")
}

fn run(root: &Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_planeter-floor-check"))
        .arg(root)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap(), text)
}

#[test]
fn the_real_repository_passes() {
    let (code, text) = run(&repo_root());
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("OK"), "{text}");
}

#[test]
fn agreeing_floors_pass_and_comments_are_not_floors() {
    let root = fixture(
        "ok",
        &cli("0, 52, 0"),
        &release("0.52.0"),
        &docker("0.52.0"),
    );
    let (code, text) = run(&root);
    assert_eq!(code, 0, "{text}");
}

#[test]
fn a_disagreeing_floor_fails_and_names_every_file_and_value() {
    let root = fixture(
        "drift",
        &cli("0, 52, 0"),
        &release("0.46.0"),
        &docker("0.52.0"),
    );
    let (code, text) = run(&root);
    assert_eq!(code, 1, "{text}");
    for needle in [
        "crates/planeter-prikk/src/cli.rs",
        ".github/workflows/release.yml",
        "Dockerfile",
        "0.46.0",
        "0.52.0",
        "FAILED",
    ] {
        assert!(text.contains(needle), "{needle:?} missing from:\n{text}");
    }
}

#[test]
fn a_missing_declaration_or_a_non_semver_value_fails() {
    let root = fixture(
        "bad",
        &cli("0, 52, 0"),
        "env:\n  OTHER: x\n",
        &docker("latest"),
    );
    let (code, text) = run(&root);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("no floor declaration found"), "{text}");
    assert!(text.contains("is not X.Y.Z"), "{text}");
}

#[test]
fn a_missing_file_is_reported_and_fails() {
    let root = fixture(
        "missing",
        &cli("0, 52, 0"),
        &release("0.52.0"),
        &docker("0.52.0"),
    );
    fs::remove_file(root.join("Dockerfile")).unwrap();
    let (code, text) = run(&root);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("cannot read"), "{text}");
}

#[test]
fn a_bad_root_is_a_usage_error() {
    let (code, _) = run(Path::new("/definitely/not/a/directory"));
    assert_eq!(code, 2);
}

#[test]
fn the_prikk_the_ci_installs_is_a_floor_too() {
    // ci.yml drifting alone fails and is named (RFC 004 part 10: the client's tests run against the floor).
    let root = fixture_with_ci(
        "ci-drift",
        &cli("0, 52, 0"),
        &release("0.52.0"),
        &release("0.53.0"),
        &docker("0.52.0"),
    );
    let (code, text) = run(&root);
    assert_eq!(code, 1, "{text}");
    for needle in [".github/workflows/ci.yml", "0.53.0", "FAILED"] {
        assert!(text.contains(needle), "{needle:?} missing from:\n{text}");
    }
    // ...and a ci.yml with no declaration is a missing floor, not a pass.
    let root = fixture_with_ci(
        "ci-none",
        &cli("0, 52, 0"),
        &release("0.52.0"),
        "env:\n  OTHER: x\n",
        &docker("0.52.0"),
    );
    let (code, text) = run(&root);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("no floor declaration found"), "{text}");
}
