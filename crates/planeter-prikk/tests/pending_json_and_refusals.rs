//! Two typed readers that nothing in planeter uses yet (re-baseline v2, B4 and B5):
//!
//! * `sync-pending-v1` (`sync pending --format json`, prikk 0.56.0): the base of the file-by-file review view;
//! * `refusal-v1` for a delivery (`archive import`, `bundle import`): rollback, diverged, does-not-connect (0.56.0) and
//!   repository-format-too-old, outside-version-rule, forged-publication-signature (0.57.0). The driver never passes
//!   `--allow-non-fast-forward`.
//!
//! Fixtures are the real output of prikk 0.57.0; the real-prikk tests skip (printing why) without prikk 0.57.0 or later.
//! Convention of the crate for JSON readers: `schema_version` exact, every named field required, unknown fields tolerated.

use std::path::Path;
use std::process::Command;

use planeter_prikk::PrikkError;
use planeter_prikk::delivery::{DeliveryRefusal, parse_import_refusal};
use planeter_prikk::exchange::{SignatureOutcome, parse_pending_json};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

// ------------------------------------------------------------------------------------------------ sync-pending-v1

#[test]
fn the_real_sync_pending_document_is_read_with_its_claims_and_signatures() {
    let r = parse_pending_json(&fixture("sync-pending-0.57.0-two-claims-sound.json")).unwrap();
    assert_eq!(r.schema_version, "sync-pending-v1");
    assert_eq!(r.patches.len(), 2);
    for p in &r.patches {
        assert_eq!(p.patch_id.len(), 64);
        assert_eq!(p.claims.len(), 1, "one claim names each patch");
        let c = &p.claims[0];
        assert_eq!(c.claim_id.len(), 64);
        assert_eq!(c.target_block_id.len(), 64);
        assert_eq!(c.parent_block_ids.len(), 1);
        assert_eq!(c.signature.outcome, SignatureOutcome::Sound);
        assert!(
            c.signature
                .key_id
                .as_deref()
                .unwrap()
                .starts_with("ed25519-")
        );
    }
    // A chain: one claim's parent is the other's target block.
    let blocks: Vec<&str> = r
        .patches
        .iter()
        .map(|p| p.claims[0].target_block_id.as_str())
        .collect();
    assert!(
        r.patches
            .iter()
            .any(|p| blocks.contains(&p.claims[0].parent_block_ids[0].as_str())),
        "the two claims form a chain"
    );
}

#[test]
fn a_claim_signed_by_a_key_the_forge_has_not_adopted_reads_as_unverifiable() {
    let r =
        parse_pending_json(&fixture("sync-pending-0.57.0-two-claims-unverifiable.json")).unwrap();
    assert!(
        r.patches
            .iter()
            .all(|p| p.claims[0].signature.outcome == SignatureOutcome::Unverifiable)
    );
}

#[test]
fn an_empty_document_is_read_and_a_document_of_another_shape_is_not() {
    let empty = r#"{"schema_version": "sync-pending-v1", "patches": []}"#;
    assert!(parse_pending_json(empty).unwrap().patches.is_empty());
    let id = "a".repeat(64);
    let claim = format!(
        r#"{{"claim_id": "{id}", "target_block_id": "{id}", "parent_block_ids": ["{id}"], "signature": {{"outcome": "sound", "key_id": "ed25519-1"}}}}"#
    );
    let ok = format!(
        r#"{{"schema_version": "sync-pending-v1", "patches": [{{"patch_id": "{id}", "claims": [{claim}]}}]}}"#
    );
    assert!(parse_pending_json(&ok).is_ok());
    // An unknown field is tolerated (a later prikk may add some); an unknown outcome is kept as it came.
    let extra = ok
        .replace(r#""patches": ["#, r#""later": 1, "patches": ["#)
        .replace("\"sound\"", "\"odd\"");
    let r = parse_pending_json(&extra).unwrap();
    assert_eq!(
        r.patches[0].claims[0].signature.outcome,
        SignatureOutcome::Other("odd".to_owned())
    );
    for bad in [
        ok.replace("sync-pending-v1", "sync-pending-v2"),
        ok.replace(r#""schema_version": "sync-pending-v1", "#, ""),
        ok.replace(r#""patches": ["#, r#""patchez": ["#),
        ok.replace(&format!(r#""patch_id": "{id}""#), r#""patch_id": "short""#),
        ok.replace(r#""claim_id""#, r#""claim""#),
        ok.replace(
            &format!(r#""parent_block_ids": ["{id}"]"#),
            r#""parent_block_ids": ["x"]"#,
        ),
        ok.replace(
            r#""signature": {"outcome": "sound", "key_id": "ed25519-1"}"#,
            r#""signature": {"key_id": "ed25519-1"}"#,
        ),
        "not json".to_owned(),
        String::new(),
    ] {
        assert!(
            matches!(parse_pending_json(&bad), Err(PrikkError::Parse(_))),
            "{bad}"
        );
    }
}

// ------------------------------------------------------------------------------------------------ refusal-v1 of a delivery

#[test]
fn the_real_refusals_of_0_56_0_and_0_57_0_are_read_by_id() {
    for (file, command, id) in [
        (
            "refusal-0.57.0-archive-import-rollback.json",
            "archive-import",
            DeliveryRefusal::Rollback,
        ),
        (
            "refusal-0.57.0-bundle-import-diverged.json",
            "bundle-import",
            DeliveryRefusal::Diverged,
        ),
        (
            "refusal-0.57.0-archive-import-does-not-connect.json",
            "archive-import",
            DeliveryRefusal::DoesNotConnect,
        ),
        (
            "refusal-0.57.0-archive-import-repository-format-too-old.json",
            "archive-import",
            DeliveryRefusal::RepositoryFormatTooOld,
        ),
    ] {
        let r = parse_import_refusal(&fixture(file)).unwrap();
        assert_eq!((r.command.as_str(), &r.refusal), (command, &id), "{file}");
        assert_eq!(r.reason, None);
        assert!(
            r.message.starts_with("precondition not met: "),
            "{}",
            r.message
        );
    }
}

#[test]
fn every_id_reads_to_its_own_value_and_never_to_another() {
    assert_eq!(DeliveryRefusal::KNOWN.len(), 6);
    let mut seen = Vec::new();
    for id in DeliveryRefusal::KNOWN {
        let r = DeliveryRefusal::from(id.to_owned());
        assert!(!matches!(r, DeliveryRefusal::Other(_)), "{id}");
        assert_eq!(r.id(), id);
        assert!(!seen.contains(&r), "{id} read as an earlier one");
        seen.push(r);
    }
    let other = DeliveryRefusal::from("a-later-id".to_owned());
    assert_eq!(other, DeliveryRefusal::Other("a-later-id".to_owned()));
    assert_eq!(other.id(), "a-later-id");
    // The two ids of 0.57.0 that need a hand-built document (outside-version-rule, forged-publication-signature) are read from the
    // document shape prikk's letter describes: the same four fields, `reason: null`.
    for id in ["outside-version-rule", "forged-publication-signature"] {
        let doc = format!(
            r#"{{"schema_version": "refusal-v1", "command": "bundle-import", "refusal": "{id}", "reason": null, "message": "precondition not met: x"}}"#
        );
        assert_eq!(parse_import_refusal(&doc).unwrap().refusal.id(), id);
    }
}

#[test]
fn a_refusal_of_another_shape_or_command_is_not_read() {
    let ok = r#"{"schema_version": "refusal-v1", "command": "archive-import", "refusal": "rollback", "reason": null, "message": "m"}"#;
    assert!(parse_import_refusal(ok).is_ok());
    for bad in [
        ok.replace("archive-import", "extract"),
        ok.replace("refusal-v1", "refusal-v2"),
        ok.replace(r#""reason": null, "#, ""),
        ok.replace(r#""message": "m""#, r#""message": 5"#),
        ok.replace(r#""refusal": "rollback", "#, ""),
        "{}".to_owned(),
    ] {
        assert!(parse_import_refusal(&bad).is_err(), "{bad}");
    }
}

/// A real rollback, from prikk 0.57.0 itself (skips without it): the document on stdout of a refused `archive import`, exit 1, nothing
/// written. The command is run directly: the driver's `archive_import` does not carry stdout (`creation.rs` shows prikk's `error:` line).
#[test]
fn a_real_prikk_refusal_of_a_rollback_is_read() {
    let ver = Command::new("prikk").arg("--version").output();
    let ok = ver
        .as_ref()
        .map(|o| {
            let t = String::from_utf8_lossy(&o.stdout);
            let mut p = t
                .trim()
                .trim_start_matches("prikk ")
                .split('.')
                .map(|x| x.parse::<u32>().unwrap_or(0));
            (p.next().unwrap_or(0), p.next().unwrap_or(0)) >= (0, 57)
        })
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping: needs prikk >= 0.57.0 on PATH");
        return;
    }
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("delivery-refusal");
    let _ = std::fs::remove_dir_all(&base);
    let run = |dir: &str, keys: &str, args: &[&str]| -> (i32, String) {
        let d = base.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        let k = base.join(keys);
        std::fs::create_dir_all(&k).unwrap();
        let o = Command::new("prikk")
            .args(args)
            .current_dir(&d)
            .env("HOME", &k)
            .env("XDG_CONFIG_HOME", &k)
            .output()
            .unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let step = |dir: &str, keys: &str, args: &[&str]| {
        let (code, out) = run(dir, keys, args);
        assert_eq!(code, 0, "prikk {args:?}: {out}");
    };
    step("c", "ck", &["setup"]);
    std::fs::write(base.join("c/a.txt"), "one\n").unwrap();
    step("c", "ck", &["commit", "--from-worktree", "-m", "one"]);
    step("c", "ck", &["seal", "--allow-no-audit"]);
    let older = base.join("older.archive");
    step("c", "ck", &["archive", "export", older.to_str().unwrap()]);
    std::fs::write(base.join("c/b.txt"), "two\n").unwrap();
    step("c", "ck", &["commit", "--from-worktree", "-m", "two"]);
    step("c", "ck", &["seal", "--allow-no-audit"]);
    let newer = base.join("newer.archive");
    step("c", "ck", &["archive", "export", newer.to_str().unwrap()]);
    step("f", "fk", &["init", "."]);
    step("f", "fk", &["archive", "import", newer.to_str().unwrap()]);
    let (code, out) = run(
        "f",
        "fk",
        &[
            "archive",
            "import",
            older.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert_eq!(code, 1, "{out}");
    let r = parse_import_refusal(&out).unwrap();
    assert_eq!(r.command, "archive-import");
    assert_eq!(r.refusal, DeliveryRefusal::Rollback);
    assert!(
        r.message.contains("--allow-non-fast-forward"),
        "{}",
        r.message
    );
}
