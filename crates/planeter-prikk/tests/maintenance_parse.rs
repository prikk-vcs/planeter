//! The readers of `prikk doctor` and `prikk unlock` (RFC 004 part 4): output captured from prikk 0.53.0, and every
//! way the shape can be wrong. Recovery removes locks on what these say, so the readers refuse what they do not know.

use planeter_prikk::maintenance::{Repair, Severity, parse_doctor_output, parse_unlock_output};

const DOCTOR_OK: &str = "\
doctor repository: /srv/r/.prikk
verified repository: /srv/r/.prikk
active sessions: 1 total, 1 covered by sealed-history verification (default only, by construction -- see `prikk doctor` for the rest)
verification stages: 17
stage objects: evaluated
object items: 352 scanned, 0 failed
interrupted appends: 0
trailing partial WAL bytes: 0
info [PRIKK-DOCTOR-VERIFY-OK]: repository structural verification scan completed
  recommendation: review the remaining diagnostics before deciding whether action is required
issue summary: errors=0, warnings=0, info=1
";

const UNLOCK_TWO: &str = "\
/srv/r/.prikk/active/default/active.lock
  kind: active
  recorded pid: 408657
  liveness: does not appear to be running (not proof it is safe to clear)
/srv/r/.prikk/containers/objects.lock
  kind: container:object-store
  recorded pid: 408657
  liveness: does not appear to be running (not proof it is safe to clear)

liveness is advisory only: a positive result is reliable evidence the process is still running, but a negative or unknown result is NOT proof it is safe to clear -- PID reuse and container namespace isolation can both make a genuinely running process appear absent. Clear a lock only if you have independently confirmed the process that created it is gone.
";

#[test]
fn a_clean_doctor_report_is_read() {
    let r = parse_doctor_output(DOCTOR_OK).unwrap();
    assert_eq!((r.errors, r.warnings, r.info), (0, 0, 1));
    assert!(r.is_clean());
    assert_eq!(r.diagnostics.len(), 1);
    assert_eq!(r.diagnostics[0].code, "PRIKK-DOCTOR-VERIFY-OK");
    assert_eq!(r.diagnostics[0].severity, Severity::Info);
    assert!(
        r.diagnostics[0]
            .recommendation
            .as_deref()
            .unwrap()
            .starts_with("review the remaining")
    );
    assert!(r.named_repairs().is_empty());
}

#[test]
fn a_report_with_a_warning_is_not_clean_and_names_the_repair_it_recommends() {
    let text = "\
doctor repository: /srv/r/.prikk
warning [PRIKK-DOCTOR-WAL-TAIL]: trailing partial WAL bytes: 12
  recommendation: run `prikk doctor --repair-wal-tail` after checking the tail
  recommendation: or `prikk doctor --repair-tails --plan-only` to preview
info [PRIKK-DOCTOR-VERIFY-OK]: scan completed
issue summary: errors=0, warnings=1, info=1
";
    let r = parse_doctor_output(text).unwrap();
    assert!(!r.is_clean());
    assert_eq!((r.errors, r.warnings, r.info), (0, 1, 1));
    assert_eq!(
        r.named_repairs(),
        vec!["--repair-wal-tail", "--repair-tails", "--plan-only"]
    );
    // Only the known idempotent repairs have a name here; `--plan-only` is not one.
    assert_eq!(
        Repair::from_flag("--repair-wal-tail"),
        Some(Repair::WalTail)
    );
    assert_eq!(Repair::from_flag("--repair-tails"), Some(Repair::Tails));
    assert_eq!(Repair::from_flag("--plan-only"), None);
    assert_eq!(
        Repair::from_flag("--discard-damaged-commits"),
        None,
        "data-losing repairs are never ours to run"
    );
    assert_eq!(Repair::from_flag("--repair-main-ref"), None);
}

#[test]
fn an_error_report_is_read_and_is_not_clean() {
    let text = "doctor repository: /x\nerror [PRIKK-DOCTOR-X]: damaged\nissue summary: errors=1, warnings=0, info=0\n";
    let r = parse_doctor_output(text).unwrap();
    assert!(!r.is_clean());
    assert_eq!(r.diagnostics[0].severity, Severity::Error);
}

#[test]
fn a_doctor_report_of_an_unknown_shape_is_refused() {
    for bad in [
        "",
        "something else entirely\n",
        // the diagnostics do not add up to the summary
        "doctor repository: /x\ninfo [A]: a\nissue summary: errors=0, warnings=0, info=2\n",
        "doctor repository: /x\nissue summary: errors=0, warnings=0, info=1\n",
        // no summary
        "doctor repository: /x\ninfo [A]: a\n",
        // a summary that names something else, or a count that is not a number
        "doctor repository: /x\nissue summary: errors=0, warnings=0, notes=1\n",
        "doctor repository: /x\nissue summary: errors=x, warnings=0, info=0\n",
        // a recommendation with nothing before it
        "doctor repository: /x\n  recommendation: orphan\nissue summary: errors=0, warnings=0, info=0\n",
    ] {
        assert!(parse_doctor_output(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_unlock_listing_is_read_with_its_locks_and_its_advisory_ignored() {
    let locks = parse_unlock_output(UNLOCK_TWO).unwrap();
    assert_eq!(locks.len(), 2);
    assert_eq!(locks[0].path, "/srv/r/.prikk/active/default/active.lock");
    assert_eq!(locks[0].kind, "active");
    assert_eq!(locks[0].recorded_pid, 408657);
    assert!(
        locks[0]
            .liveness
            .starts_with("does not appear to be running")
    );
    assert_eq!(locks[1].kind, "container:object-store");
    assert!(
        parse_unlock_output("no locks currently held\n")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_unlock_listing_of_an_unknown_shape_is_refused() {
    for bad in [
        "",
        "   \n",
        "garbage\n",
        "/a/b.lock\n",
        "/a/b.lock\n  kind: active\n",
        "/a/b.lock\n  kind: active\n  recorded pid: nope\n  liveness: x\n",
        "/a/b.lock\n  pid: 1\n  kind: active\n  liveness: x\n",
        "/a/b.lock\n  kind: active\n  recorded pid: 1\n  liveness: x\nno locks currently held\n",
        "/path with space.lock\n  kind: active\n  recorded pid: 1\n  liveness: x\n",
        "no locks currently held\nsomething new a later prikk prints\n",
        "/a/b.lock\n  kind: active\n  recorded pid: 1\n  liveness: x\n\nliveness is advisory only: x\nand then more\n",
    ] {
        assert!(parse_unlock_output(bad).is_err(), "{bad:?}");
    }
}
