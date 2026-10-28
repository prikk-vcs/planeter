//! IS-55 against the real prikk (self-skips without it): an archive with many branches prints an import report as large as the
//! archive's refs (171 B a branch, measured), and the driver reads it under the table's cap and refuses it under a smaller one, with
//! the child killed. (The 20,000-branch measurement is in the part's T1; an import is about 50 ms a branch, too slow for a test.)

use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_prikk::sandbox::CallLimits;
use planeter_prikk::{CliPrikkRepo, OutputLimit, PrikkError, PrikkRepo, Sandbox};

const BRANCHES: usize = 120;

fn have_prikk() -> bool {
    Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn prikk(args: &[&str], cwd: &Path, home: &Path) -> Vec<u8> {
    let out = Command::new("prikk")
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .output()
        .expect("spawn prikk");
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// An archive of `BRANCHES` branches, and a fresh repository that trusts its keys.
fn fixture(name: &str) -> (PathBuf, PathBuf) {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&base);
    let (src, keys, forge, forge_home) = (
        base.join("src"),
        base.join("keys"),
        base.join("forge"),
        base.join("forge-home"),
    );
    for d in [&src, &keys, &forge, &forge_home] {
        std::fs::create_dir_all(d).unwrap();
    }
    prikk(&["setup", "."], &src, &keys);
    std::fs::write(src.join("a.txt"), b"x\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"], &src, &keys);
    prikk(&["seal", "--allow-no-audit"], &src, &keys);
    for i in 1..BRANCHES {
        prikk(
            &[
                "branch",
                "create",
                &format!("heads/b{i}"),
                "--from",
                "heads/main",
            ],
            &src,
            &keys,
        );
    }
    let archive = base.join("many.archive");
    prikk(
        &["archive", "export", archive.to_str().unwrap()],
        &src,
        &keys,
    );
    prikk(&["init", "."], &forge, &forge_home);
    let report = prikk(
        &[
            "archive",
            "verify",
            archive.to_str().unwrap(),
            "--format",
            "json",
        ],
        &forge,
        &forge_home,
    );
    let report: serde_json::Value = serde_json::from_slice(&report).unwrap();
    for key in report["maintainer_keys"].as_array().unwrap() {
        prikk(
            &[
                "trust",
                "maintainer",
                "add",
                "--key-id",
                key["key_id"].as_str().unwrap(),
                "--public-key",
                key["public_key"].as_str().unwrap(),
            ],
            &forge,
            &forge_home,
        );
    }
    (archive, forge)
}

#[test]
fn an_archive_with_many_branches_is_imported_under_the_table_cap_and_refused_under_a_smaller_one() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let (archive, forge) = fixture("output-cap-real");

    // Under a limit smaller than the report (120 branches print about 20 KB): the call is killed and is a named error.
    let tight = CliPrikkRepo::new(&forge)
        .with_sandbox(Sandbox::Unconfined)
        .with_stdout_limit(4096);
    match tight.archive_import(&archive, true) {
        Err(PrikkError::OutputTooLarge { verb, limit, .. }) => {
            assert_eq!(verb, "archive import");
            assert_eq!(limit, 4096);
        }
        other => panic!("expected OutputTooLarge, got {other:?}"),
    }
    // The same call through the sandbox function: stdout dropped, child killed, tail of stderr only.
    let run = Sandbox::Unconfined
        .run_capped(
            std::ffi::OsStr::new("prikk"),
            &forge,
            &[
                "archive",
                "import",
                archive.to_str().unwrap(),
                "--adopt",
                "--format",
                "json",
            ],
            CallLimits::NONE,
            OutputLimit::new(4096, 1 << 20),
        )
        .unwrap();
    assert!(run.exceeded.is_some() && run.stdout.is_empty());

    // Under the table's own cap (64 MiB for an import): the whole report, every branch.
    CliPrikkRepo::new(&forge)
        .with_sandbox(Sandbox::Unconfined)
        .archive_import(&archive, true)
        .expect("under the table cap");
    let branches = CliPrikkRepo::new(&forge)
        .with_sandbox(Sandbox::Unconfined)
        .branches(true)
        .unwrap();
    assert!(
        branches.branches.len() >= BRANCHES,
        "{}",
        branches.branches.len()
    );
}
