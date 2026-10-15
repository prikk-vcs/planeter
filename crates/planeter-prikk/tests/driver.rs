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
    let repo = CliPrikkRepo::open(&dir).expect("open + version pin (MIN_PRIKK_VERSION)");

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
    // open() runs the version pin (RFC 001 D-3); success means prikk >= MIN_PRIKK_VERSION.
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

// ---------------------------------------------------------------------------------------------
// prikk 0.52.0 re-baseline (interim handoff v2): T1 schema probe, T2 PK-30, T4 IS-3, T5 IS-21.
// ---------------------------------------------------------------------------------------------

/// `(major, minor, patch)` of the installed prikk, or `None` when it is absent/unparsable.
fn prikk_version() -> Option<(u64, u64, u64)> {
    let out = Command::new("prikk").arg("--version").output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.trim().strip_prefix("prikk ")?.split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.split(['-', '+']).next()?.parse().ok()?,
    ))
}

/// T1 (PK-18, PK-43): every verb the driver reads, on a sealed fixture, through the sandbox. The
/// driver refuses an unexpected `schema_version`, so a drifted id fails here by name.
#[test]
fn every_schema_the_driver_reads_parses_on_the_installed_prikk() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = sealed_repo("driver-schema-probe");
    let repo = CliPrikkRepo::open(&dir).expect("open + version pin");

    let log = repo.log(None, None).expect("log");
    assert_eq!(log.schema_version, "log-report-v1");
    let block = log
        .blocks
        .first()
        .expect("one sealed block")
        .block_id
        .clone();

    assert_eq!(repo.show(&block).unwrap().schema_version, "show-report-v1");
    assert_eq!(repo.verify().unwrap().schema_version, "verify-report-v1");
    assert_eq!(
        repo.branches(true).unwrap().schema_version,
        "branch-list-v1"
    );
    assert_eq!(repo.tags().unwrap().schema_version, "tag-list-v1");
    assert_eq!(repo.status().unwrap().schema_version, "status-report-v1");
    assert_eq!(
        repo.worktree_status().unwrap().schema_version,
        "worktree-status-report-v1"
    );
    assert_eq!(
        repo.patch_plan_content(None, &["README.md"])
            .unwrap()
            .schema_version,
        "patch-plan-content-v1"
    );
    assert_eq!(
        repo.tree(None, None).unwrap().schema_version,
        "tree-listing-v1"
    );
    assert_eq!(
        repo.path_content_meta(None, "README.md")
            .unwrap()
            .schema_version,
        "path-content-v1"
    );
    // `key status` needs the operator's key directory, which the keyless sandbox deliberately does not
    // have (INV-2), so through the driver it is a typed failure, not a parse. Its `key-status-v1`
    // document was checked from a raw, unsandboxed dump instead (see the T1 review-request table).
    assert!(matches!(
        repo.key_status(),
        Err(planeter_prikk::PrikkError::Command { code: Some(1), .. })
    ));

    let trust = repo.trust_list().unwrap();
    assert_eq!(trust.schema_version, "trust-list-v1");
    let adopted = &trust
        .keys
        .first()
        .expect("setup adopted a maintainer key")
        .key_id;
    let check = repo.trust_check(adopted).unwrap();
    assert_eq!(check.schema_version, "trust-check-v1");
    assert!(check.trusted);
}

/// T2 (PK-30): on a fresh, unsealed repository the explicit current branch answers exactly like the
/// bare form. prikk 0.45.0/0.46.0 refused it; fixed in 0.47.0. Skips **visibly** below 0.47.0.
#[test]
fn explicit_current_branch_on_an_unsealed_repo_matches_the_bare_form() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    match prikk_version() {
        Some(v) if v >= (0, 47, 0) => {}
        other => {
            eprintln!("SKIPPED (PK-30 is fixed in prikk 0.47.0; installed: {other:?})");
            return;
        }
    }
    let dir = init_repo("driver-pk30");
    let repo = CliPrikkRepo::open(&dir).expect("open");

    let bare = repo.tree(None, None).expect("tree (bare)");
    let explicit = repo
        .tree(Some("heads/main"), None)
        .expect("tree --ref heads/main");
    assert_eq!(
        bare, explicit,
        "tree: explicit ref must equal the bare form"
    );
    assert!(bare.entries.is_empty() && bare.target_block_id.is_none());

    let bare = repo.log(None, None).expect("log (bare)");
    let explicit = repo
        .log(Some("heads/main"), None)
        .expect("log --ref heads/main");
    assert_eq!(bare, explicit, "log: explicit ref must equal the bare form");
    assert!(bare.blocks.is_empty());
}

/// T4 (IS-3): `bundle export` from a sealed repository, `bundle import --input` into a fresh one; the
/// ref is visible under `remotes/` and no local ref moved (PK-4). Artifacts live **inside** the
/// repository root because that is the only path the sandbox binds.
#[test]
fn bundle_round_trip_lands_under_remotes() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let source = sealed_repo("driver-bundle-src");
    let src = CliPrikkRepo::open(&source).expect("open source");
    let bundle = source.join("out.bundle");
    src.bundle_export("heads/main", &bundle)
        .expect("bundle export");
    assert!(bundle.metadata().expect("bundle written").len() > 0);

    let dest = init_repo("driver-bundle-dst");
    let landed = dest.join("in.bundle");
    std::fs::copy(&bundle, &landed).unwrap();
    let dst = CliPrikkRepo::open(&dest).expect("open dest");
    dst.bundle_import(&landed).expect("bundle import --input");

    let branches = dst.branches(true).expect("branches --all");
    assert!(
        branches
            .received
            .iter()
            .any(|r| r.ref_name == "remotes/heads/main"),
        "received refs: {:?}",
        branches.received
    );
    assert!(
        !branches.branches.iter().any(|b| b.ref_name == "heads/main"),
        "import must not create a local heads/main"
    );

    // The refusal path is keyed on exit status: a missing artifact is a typed Command error.
    let missing = dest.join("absent.bundle");
    assert!(matches!(
        dst.bundle_import(&missing),
        Err(planeter_prikk::PrikkError::Command { .. })
    ));
}

/// T5 on the live binary: both answers come back as `Ok(TrustCheck)` whatever prikk's exit code for
/// "not trusted" is (0 before 0.52.0, 1 since).
#[test]
fn trust_check_answers_for_adopted_and_unknown_keys() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let dir = sealed_repo("driver-trust-check");
    let repo = CliPrikkRepo::open(&dir).expect("open");
    let adopted = repo.trust_list().unwrap().keys[0].key_id.clone();

    let yes = repo.trust_check(&adopted).expect("adopted key");
    assert!(yes.trusted && yes.public_key.is_some());

    let no = repo
        .trust_check("nobody-here")
        .expect("unknown key is an answer");
    assert!(!no.trusted && no.public_key.is_none());
    eprintln!(
        "trust_check on prikk {:?}: adopted=true unknown=false",
        prikk_version()
    );
}

/// T3 end to end (PK-31): the bound set on the sandbox profile reaches prikk through `--clearenv`
/// and `sync accept` refuses the oversize file by exit status. Without the bound the same file is
/// refused for its content instead — so the size refusal below is the bound's doing.
#[test]
fn sandbox_bound_reaches_prikk_and_refuses_an_oversize_artifact() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    use planeter_prikk::sandbox::{BwrapConfig, IncomingBounds};
    let dir = init_repo("driver-accept-bound");
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, vec![0u8; 4096]).unwrap();

    let bounded =
        CliPrikkRepo::new(&dir).with_sandbox(planeter_prikk::Sandbox::Bwrap(BwrapConfig {
            bounds: IncomingBounds {
                exchange_max_bytes: Some(64),
                ..IncomingBounds::default()
            },
            ..BwrapConfig::default()
        }));
    match bounded.sync_accept(&junk) {
        Err(planeter_prikk::PrikkError::Command { code, stderr }) => {
            assert_eq!(code, Some(1));
            assert!(
                stderr.contains("PRIKK_EXCHANGE_MAX_BYTES"),
                "refused for size, not content: {stderr}"
            );
        }
        other => panic!("expected the size refusal, got {other:?}"),
    }

    let unbounded = CliPrikkRepo::new(&dir);
    match unbounded.sync_accept(&junk) {
        Err(planeter_prikk::PrikkError::Command { code, stderr }) => {
            assert_eq!(code, Some(1));
            assert!(!stderr.contains("PRIKK_EXCHANGE_MAX_BYTES"), "{stderr}");
        }
        other => panic!("expected a content refusal, got {other:?}"),
    }
}
