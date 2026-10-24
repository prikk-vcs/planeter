//! prikk 0.54.0 (PK-70): `verify` prints `unreached objects: N` on every run and adds `unreached_objects` to the JSON. An
//! unreached object is a **warning**, not a trust failure: the exit stays 0 on it alone and the verdict stays `ok`. The model
//! records the count (not shown anywhere); the verification page's verdict stays prikk's `verdict.ok`.
//!
//! The fixtures are the real output of prikk 0.54.1 on a repository whose received ref was removed (`received remove`):
//! one block and one ref-state that nothing reaches. The text reports are read by planeter only through `doctor` (the strict
//! reader of `maintenance.rs`); `verify` itself is read as JSON.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_prikk::PrikkRepo;
use planeter_prikk::Sandbox;
use planeter_prikk::cli::CliPrikkRepo;
use planeter_prikk::maintenance::{Severity, parse_doctor_output};
use planeter_prikk::model::{UnreachedKind, VerifyReport};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn the_real_0_54_1_report_with_unreached_objects_parses_and_the_verdict_stays_ok() {
    let r: VerifyReport = serde_json::from_str(&fixture("verify-0.54.1-unreached.json")).unwrap();
    assert!(r.verdict.ok && r.verdict.failed_conditions.is_empty());
    assert_eq!(r.unreached_count(), 2);
    assert_eq!(r.unreached_objects[0].object_type, UnreachedKind::Block);
    assert_eq!(r.unreached_objects[1].object_type, UnreachedKind::RefState);
    assert_eq!(r.unreached_objects[0].object_id.len(), 64);
}

#[test]
fn a_0_53_style_report_without_the_array_still_parses_with_a_count_of_nothing() {
    let json = r#"{"schema_version": "verify-report-v1", "verdict": {"ok": true, "failed_conditions": []},
        "active_sessions": {"count": 1, "verified_count": 1}, "stages": [{"stage": "objects", "status": "evaluated"}]}"#;
    let r: VerifyReport = serde_json::from_str(json).unwrap();
    assert!(r.verdict.ok);
    assert_eq!(r.unreached_count(), 0);
}

#[test]
fn an_unreached_object_of_a_type_prikk_does_not_document_is_kept_as_it_came() {
    let json = r#"{"schema_version": "verify-report-v1", "verdict": {"ok": true, "failed_conditions": []},
        "active_sessions": {"count": 1, "verified_count": 1}, "stages": [],
        "unreached_objects": [{"object_id": "ab", "object_type": "tag", "a_field_prikk_adds_later": 1}]}"#;
    let r: VerifyReport = serde_json::from_str(json).unwrap();
    assert_eq!(
        r.unreached_objects[0].object_type,
        UnreachedKind::Other("tag".to_owned())
    );
}

#[test]
fn a_failing_verdict_with_unreached_objects_is_still_a_failure() {
    let json = r#"{"schema_version": "verify-report-v1",
        "verdict": {"ok": false, "failed_conditions": [{"id": "stage-failure", "message": "boom"}]},
        "active_sessions": {"count": 1, "verified_count": 1}, "stages": [],
        "unreached_objects": [{"object_id": "ab", "object_type": "block"}]}"#;
    let r: VerifyReport = serde_json::from_str(json).unwrap();
    assert!(
        !r.verdict.ok,
        "the unreached objects never turn a failure into anything else"
    );
}

#[test]
fn the_driver_reads_a_stand_in_verify_that_carries_the_new_array() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("verify-unreached-standin");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    let new = dir.join("verify-new.json");
    std::fs::write(&new, fixture("verify-0.54.1-unreached.json")).unwrap();
    let bin = dir.join("prikk");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.54.1';; verify) cat {}; exit 0;; esac\n", new.display()),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = CliPrikkRepo::with_binary(dir.join("repo"), &bin).with_sandbox(Sandbox::Unconfined);
    let report = repo.verify().unwrap();
    assert!(report.verdict.ok);
    assert_eq!(report.unreached_count(), 2);
}

#[test]
fn the_doctor_text_of_0_54_1_with_unreached_objects_is_read_and_is_not_clean() {
    // `doctor` prints the same `unreached objects: N` line and one warning per object, whose recommendation names no repair.
    let r = parse_doctor_output(&fixture("doctor-0.54.1-unreached.txt")).unwrap();
    assert_eq!((r.errors, r.warnings, r.info), (0, 2, 1));
    assert!(!r.is_clean());
    assert_eq!(r.diagnostics.iter().filter(|d| d.code == "PRIKK-DOCTOR-UNREACHED-OBJECT" && d.severity == Severity::Warning).count(), 2);
    assert!(r.named_repairs().is_empty());
}

fn have_prikk() -> bool {
    Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Real prikk: import a keyed repository's archive into a keyless one, adopt the key, remove the received ref. `verify` exits 0
/// and the model sees two unreached objects. (Skipped without prikk, or with a prikk older than 0.54.)
#[test]
fn real_prikk_after_received_remove_reports_unreached_objects_and_verify_still_passes() {
    if !have_prikk() {
        eprintln!("skipping: prikk is not on PATH");
        return;
    }
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("verify-unreached-real");
    let _ = std::fs::remove_dir_all(&base);
    let (client, ckeys, forge, fkeys) = (
        base.join("c"),
        base.join("ck"),
        base.join("f"),
        base.join("fk"),
    );
    for d in [&client, &ckeys, &forge, &fkeys] {
        std::fs::create_dir_all(d).unwrap();
    }
    let run = |dir: &Path, keys: &Path, args: &[&str]| -> String {
        let out = Command::new("prikk")
            .args(args)
            .current_dir(dir)
            .env("HOME", keys)
            .env("XDG_CONFIG_HOME", keys)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "prikk {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    if !run(&client, &ckeys, &["--version"]).contains("0.5")
        || run(&client, &ckeys, &["--version"]).trim() < "prikk 0.54.0"
    {
        eprintln!("skipping: needs prikk >= 0.54.0");
        return;
    }
    run(&client, &ckeys, &["setup"]);
    std::fs::write(client.join("a.txt"), "hello\n").unwrap();
    run(
        &client,
        &ckeys,
        &["commit", "--from-worktree", "-m", "init"],
    );
    run(&client, &ckeys, &["seal", "--allow-no-audit"]);
    let archive = base.join("c.archive");
    run(
        &client,
        &ckeys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    run(&forge, &fkeys, &["init", "."]);
    run(
        &forge,
        &fkeys,
        &["archive", "import", archive.to_str().unwrap()],
    );
    let info: serde_json::Value = serde_json::from_str(&run(
        &forge,
        &fkeys,
        &[
            "archive",
            "verify",
            archive.to_str().unwrap(),
            "--format",
            "json",
        ],
    ))
    .unwrap();
    let key = &info["maintainer_keys"][0];
    run(
        &forge,
        &fkeys,
        &[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            key["key_id"].as_str().unwrap(),
            "--public-key",
            key["public_key"].as_str().unwrap(),
        ],
    );
    let before = CliPrikkRepo::new(&forge).with_sandbox(Sandbox::Unconfined);
    // The driver reads HOME for nothing here: prikk finds the repository by its directory.
    let r = before.verify().unwrap();
    assert!(r.verdict.ok && r.unreached_count() == 0, "{r:?}");
    run(
        &forge,
        &fkeys,
        &["received", "remove", "remotes/heads/main"],
    );
    let after = before.verify().unwrap();
    assert!(after.verdict.ok, "{after:?}");
    assert_eq!(after.unreached_count(), 2);
    assert!(
        after
            .unreached_objects
            .iter()
            .any(|u| u.object_type == UnreachedKind::Block)
    );
    assert!(
        after
            .unreached_objects
            .iter()
            .any(|u| u.object_type == UnreachedKind::RefState)
    );
}
