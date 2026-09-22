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

/// Create a repo with keys, write files, commit and seal them to `heads/main`, and return its root.
/// Key material is isolated to a per-test config dir (HOME/XDG override) so it never touches the
/// developer's real key directory. The sealing steps run **unsandboxed** (they need the key dir);
/// planeter's own reads (tree/cat) then run through the sandboxed [`CliPrikkRepo`], which needs no keys.
fn sealed_repo(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let keys = base.join("keys");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::create_dir_all(&keys).expect("create keys dir");

    let prikk = |args: &[&str]| {
        let ok = Command::new("prikk")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &keys)
            .env("XDG_CONFIG_HOME", &keys)
            .status()
            .expect("spawn prikk");
        assert!(ok.success(), "prikk {args:?} failed");
    };

    prikk(&["setup"]);
    std::fs::write(repo.join("README.md"), b"hello world\n").unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/main.rs"), b"fn main() {}\n").unwrap();
    std::fs::write(repo.join("data.bin"), [0x00u8, 0x01, 0x02, b'X', 0xff]).unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

#[test]
fn tree_and_cat_read_a_sealed_repo() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = sealed_repo("driver-tree-cat");
    let repo = CliPrikkRepo::open(&dir).expect("open + version pin (>= 0.46.0)");

    // tree: leaf paths at heads/main, with text/binary encoding.
    let tree = repo.tree(None, None).expect("tree");
    assert_eq!(tree.schema_version, "tree-listing-v1");
    let readme = tree
        .entries
        .iter()
        .find(|e| e.path == "README.md")
        .expect("README.md listed");
    assert_eq!(readme.encoding.as_deref(), Some("text"));
    let bin = tree
        .entries
        .iter()
        .find(|e| e.path == "data.bin")
        .expect("data.bin listed");
    assert_eq!(bin.encoding.as_deref(), Some("binary"));
    assert!(bin.content_id.is_some(), "binary carries a content_id");

    // tree with a prefix filters to the subtree.
    let src = repo.tree(None, Some("src")).expect("tree --prefix src");
    assert!(src.entries.iter().all(|e| e.path.starts_with("src")));

    // cat metadata (no bytes).
    let meta = repo
        .path_content_meta(None, "README.md")
        .expect("cat --format json");
    assert_eq!(meta.schema_version, "path-content-v1");
    assert_eq!(meta.encoding.as_deref(), Some("text"));

    // cat bytes: text is exact; binary comes back with its non-UTF-8 bytes intact.
    let text = repo.cat_bytes(None, "README.md", None).expect("cat text");
    assert_eq!(text, b"hello world\n");
    let raw = repo.cat_bytes(None, "data.bin", None).expect("cat binary");
    assert_eq!(raw, vec![0x00u8, 0x01, 0x02, b'X', 0xff]);

    // --max-bytes below the size refuses with nothing written (a typed Command error).
    assert!(repo.cat_bytes(None, "README.md", Some(3)).is_err());
}

#[test]
fn open_pins_the_version_and_reports_it() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = init_repo("driver-version");
    // open() runs the version pin (RFC 001 D-3); success means prikk >= 0.46.0 (MIN_PRIKK_VERSION).
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

    // Repo-level trust policy reads fine confined. (`key status` is deliberately *not* asserted here: it
    // needs the operator's key directory / HOME, which the confined, keyless forge subprocess does not
    // have — correct for the keyless model, INV-2.)
    let trust = repo.trust_list().expect("trust list");
    assert_eq!(trust.schema_version, "trust-list-v1");
}

#[test]
fn unconfined_dev_mode_also_works() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = init_repo("driver-unconfined");
    let repo = CliPrikkRepo::new(&dir).with_sandbox(planeter_prikk::Sandbox::Unconfined);
    // The dev-only unconfined path drives prikk directly (no bwrap).
    assert_eq!(
        repo.status().expect("status").schema_version,
        "status-report-v1"
    );
}
