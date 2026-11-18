//! prikk 0.58.0 (PK-85): `verify` reports a publication that is still running under a new id, `PRIKK-VERIFY-REF-PUBLICATION-IN-FLIGHT`, as
//! **information**, and exits 0 (0.57.0 called it "interrupted"). planeter reads `verify` as JSON (`verdict.ok`, the stages) and `doctor` as text
//! (every line `error|warning|info [CODE]: …`); it names no `PRIKK-VERIFY-*` id anywhere, so what must hold is that **neither reader takes the
//! finding for a failure, for an unreadable report, or for something to repair**.
//!
//! The finding could not be produced for real in this re-baseline (about 8,900 `verify` runs beside a `tag create` loop, and a writer whose fsyncs
//! were delayed to widen the window: the report is the same before and after the publication and never says it). The fixtures are therefore **of
//! the shape the letter describes** (an `info` line with that id, and `issue summary` counting it), not real output; the `verify` JSON beside
//! them (`verify-0.58.0-real.json`) is real output of 0.58.0 for a sound repository.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_prikk::PrikkRepo;
use planeter_prikk::Sandbox;
use planeter_prikk::cli::CliPrikkRepo;
use planeter_prikk::maintenance::{RepairPlan, Severity, parse_doctor_output};
use planeter_prikk::model::VerifyReport;

const ID: &str = "PRIKK-VERIFY-REF-PUBLICATION-IN-FLIGHT";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn stand_in(name: &str, body: &str) -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("verify-in-flight-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    let json = dir.join("verify.json");
    std::fs::write(&json, body).unwrap();
    let bin = dir.join("prikk");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.58.0';; verify) cat {}; exit 0;; esac\n",
            json.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..200 {
        match Command::new(&bin).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    (dir.join("repo"), bin)
}

/// The doctor text of a repository while a publication runs: the in-flight line is information, so the report is clean, clean for recovery,
/// asks for no repair, and counts as information in the summary.
#[test]
fn the_doctor_text_with_the_in_flight_finding_is_read_as_information_and_asks_for_nothing() {
    let text = fixture("doctor-0.58.0-publication-in-flight.txt");
    assert!(text.contains(ID));
    let r = parse_doctor_output(&text).unwrap();
    assert_eq!((r.errors, r.warnings, r.info), (0, 0, 5));
    let d = r
        .diagnostics
        .iter()
        .find(|d| d.code == ID)
        .expect("the finding is a diagnostic of its own");
    assert_eq!(d.severity, Severity::Info);
    assert!(r.is_clean() && r.is_clean_for_recovery());
    assert!(r.named_repairs().is_empty());
    assert_eq!(r.repair_plan(), RepairPlan::Nothing);
}

/// It is **not** a ref-completion request: the id that means "interrupted, run `prikk ref complete`" is a different one, and the driver never
/// reads this one as that.
#[test]
fn the_in_flight_id_is_not_the_interrupted_one_and_names_no_repair() {
    let text = fixture("doctor-0.58.0-publication-in-flight.txt");
    let r = parse_doctor_output(&text).unwrap();
    assert!(!text.contains("PRIKK-VERIFY-REF-POINTER-LEADS-LOG"));
    assert!(r.diagnostics.iter().all(|d| {
        !d.recommendation
            .as_deref()
            .unwrap_or("")
            .contains("ref complete")
    }));
}

/// Counted wrongly, the summary would be a reader error (the diagnostics must add up to it): the same text with the summary left at 4 is refused,
/// so the finding is counted, not skipped.
#[test]
fn the_summary_must_count_the_in_flight_finding_like_any_other() {
    let text = fixture("doctor-0.58.0-publication-in-flight.txt")
        .replace("warnings=0, info=5", "warnings=0, info=4");
    assert!(parse_doctor_output(&text).is_err());
}

/// What `verify --format json` prints on 0.58.0, as it is: read, ok.
#[test]
fn the_real_0_58_0_verify_json_reads() {
    let r: VerifyReport = serde_json::from_str(&fixture("verify-0.58.0-real.json")).unwrap();
    assert!(r.verdict.ok && r.verdict.failed_conditions.is_empty());
    assert_eq!(r.stages.len(), 17);
}

/// A `verify` that exits 0 and also carries the finding in a field planeter does not know (its place in the JSON is not known: the document is
/// read by the fields it needs, and an unknown one is ignored) is an ok report, never a failure or an unreadable one.
#[test]
fn a_verify_that_says_the_publication_is_in_flight_is_an_ok_report_through_the_driver() {
    let mut doc: serde_json::Value =
        serde_json::from_str(&fixture("verify-0.58.0-real.json")).unwrap();
    doc["info_findings"] = serde_json::json!([{
        "id": ID, "severity": "info",
        "message": "another command is publishing heads/main (pid 4242, from its lock); run this command again when it finishes"
    }]);
    let (repo, bin) = stand_in("driver", &doc.to_string());
    let report = CliPrikkRepo::with_binary(repo, bin)
        .with_sandbox(Sandbox::Unconfined)
        .verify()
        .unwrap();
    assert!(report.verdict.ok);
    assert_eq!(report.unreached_count(), 0);
}
