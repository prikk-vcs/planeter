//! The readers of `doctor` and `unlock` on prikk 0.57.0 (output captured from the real binary after a killed accept; the repository path is
//! replaced): the one `info` finding for unreached patches and blobs, the held-lock information, and the `recorded start` line of `unlock`.
//! Plus the closed family of unreached codes: a code of it the reader does not know, or the content finding in another shape, is unreadable
//! (recovery stops and keeps the text) and is never counted as something else.

use planeter_prikk::maintenance::{
    KNOWN_UNREACHED_CODES, Severity, UNREACHED_CONTENT_CODE, parse_doctor_output,
    parse_unlock_output, parse_unreached_content,
};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn the_real_0_57_0_doctor_report_with_unreached_content_is_read_as_information() {
    let r = parse_doctor_output(&fixture("doctor-0.57.0-unreached-content.txt")).unwrap();
    assert_eq!((r.errors, r.warnings, r.info), (0, 0, 4));
    let content = r.unreached_content().expect("the content finding");
    assert_eq!(
        (content.objects, content.patches, content.blobs),
        (81, 81, 0)
    );
    let d = r
        .diagnostics
        .iter()
        .find(|d| d.code == UNREACHED_CONTENT_CODE)
        .unwrap();
    assert_eq!(d.severity, Severity::Info);
    assert!(r.is_clean() && r.is_clean_for_recovery());
    // Information names no repair, though its text mentions `--format` of another command.
    assert!(r.named_repairs().is_empty(), "{:?}", r.named_repairs());
}

#[test]
fn the_real_0_57_0_unlock_listing_with_a_recorded_start_is_read() {
    let locks = parse_unlock_output(&fixture("unlock-0.57.0-unknown-holder.txt")).unwrap();
    assert_eq!(locks.len(), 2);
    assert_eq!(locks[0].kind, "active");
    assert_eq!(locks[1].kind, "container:object-store");
    for l in &locks {
        assert!(
            l.recorded_start
                .as_deref()
                .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
        );
        assert!(l.liveness.starts_with("unknown:"), "{}", l.liveness);
        assert!(l.path.ends_with(".lock"));
    }
}

#[test]
fn a_listing_without_a_recorded_start_still_reads_and_a_bad_or_other_detail_line_does_not() {
    let plain =
        "/r/.prikk/active/default/active.lock\n  kind: active\n  recorded pid: 7\n  liveness: x\n";
    assert_eq!(parse_unlock_output(plain).unwrap()[0].recorded_start, None);
    for bad in [
        "/r/.prikk/a.lock\n  kind: active\n  recorded pid: 7\n  recorded start: soon\n  liveness: x\n",
        "/r/.prikk/a.lock\n  kind: active\n  recorded pid: 7\n  recorded start: \n  liveness: x\n",
        "/r/.prikk/a.lock\n  kind: active\n  recorded pid: 7\n  recorded boot: 1\n  liveness: x\n",
        "/r/.prikk/a.lock\n  kind: active\n  recorded start: 1\n  recorded pid: 7\n  liveness: x\n",
    ] {
        assert!(parse_unlock_output(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_unreached_family_is_closed() {
    assert_eq!(KNOWN_UNREACHED_CODES.len(), 2);
    let real = fixture("doctor-0.57.0-unreached-content.txt");
    // A renamed code of the family.
    let renamed = real.replace(
        "PRIKK-DOCTOR-UNREACHED-CONTENT",
        "PRIKK-DOCTOR-UNREACHED-PATCHES",
    );
    assert!(parse_doctor_output(&renamed).is_err());
    // The content finding as a warning (it is information), or without its counts.
    let warning = real
        .replace(
            "info [PRIKK-DOCTOR-UNREACHED-CONTENT]",
            "warning [PRIKK-DOCTOR-UNREACHED-CONTENT]",
        )
        .replace("warnings=0, info=4", "warnings=1, info=3");
    assert!(parse_doctor_output(&warning).is_err());
    let no_counts = real.replace(
        "unreached content: 81 object(s) (81 patch(es), 0 blob(s))",
        "unreached content: lots",
    );
    assert!(parse_doctor_output(&no_counts).is_err());
    // The counts must add up.
    assert_eq!(
        parse_unreached_content(
            "unreached content: 5 object(s) (2 patch(es), 3 blob(s)) that nothing reaches"
        )
        .map(|c| c.objects),
        Some(5)
    );
    assert!(
        parse_unreached_content("unreached content: 6 object(s) (2 patch(es), 3 blob(s)) x")
            .is_none()
    );
}
