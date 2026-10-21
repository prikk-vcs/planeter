//! The two text readers of the exchange verbs (`sync accept`, `sync pending`), against output captured from prikk
//! 0.53.0 and against every way the shape can be wrong. Strictness is the point: a record is derived from these.

use planeter_prikk::exchange::{parse_accept_output, parse_claims_file, parse_pending_output};

const C1: &str = "f2a5543e6339a8990fbd223d97452464911e828b0d38181cd4efb96b9f82f05f";
const C2: &str = "1eff6e6db77b4bec54f04987f78b9fb866c17a1a40281e8290b8f93638b2c492";

fn real() -> String {
    format!(
        "accepted sync artifact\n  patches: 2\n  blobs: 2\n  claims: 2\n  tags: 0\n  new objects: 6\n  \
         author key material: 1 recorded (continuity only, not a trust decision)\n    \
         claim {C1}: Sound (ed25519-4e47dc83e8e323ef)\n    \
         claim {C2}: Unverifiable (ed25519-c3911046e6a1c93c not adopted here -- its order is unattributed)\n  \
         wrote claim ids to /somewhere/claims.txt\n"
    )
}

#[test]
fn a_real_accept_report_is_read_with_its_claims_and_key_ids() {
    let r = parse_accept_output(&real()).unwrap();
    assert_eq!(
        (r.patches, r.blobs, r.claims, r.tags, r.new_objects),
        (2, 2, 2, 0, 6)
    );
    assert_eq!(r.claim_lines.len(), 2);
    assert_eq!(r.claim_lines[0].claim_id, C1);
    assert_eq!(r.claim_lines[0].outcome, "Sound (ed25519-4e47dc83e8e323ef)");
    assert_eq!(r.claim_lines[0].key_ids, vec!["ed25519-4e47dc83e8e323ef"]);
    assert!(r.claim_lines[1].outcome.starts_with("Unverifiable ("));
    assert_eq!(r.claim_lines[1].key_ids, vec!["ed25519-c3911046e6a1c93c"]);
}

#[test]
fn an_idempotent_re_accept_with_nothing_new_is_a_report_like_any_other() {
    let text = "accepted sync artifact\npatches: 0\nblobs: 0\nclaims: 0\ntags: 0\nnew objects: 0\n";
    let r = parse_accept_output(text).unwrap();
    assert_eq!((r.claims, r.new_objects, r.claim_lines.len()), (0, 0, 0));
}

#[test]
fn unknown_lines_are_ignored_but_every_claim_line_must_have_the_shape() {
    let mut t = real();
    t.push_str("a new line a later prikk might print\n");
    assert!(parse_accept_output(&t).is_ok());
    for bad in [
        format!("claim {C1} Sound\n"),                   // no colon
        "claim abc: Sound\n".to_owned(),                 // short id
        format!("claim {}: Sound\n", C1.to_uppercase()), // upper case
    ] {
        let t = real() + &bad;
        assert!(parse_accept_output(&t).is_err(), "{bad:?}");
    }
}

#[test]
fn a_report_that_does_not_add_up_is_unreadable() {
    // fewer claim lines than `claims:`
    let t = real().replace("claims: 2", "claims: 3");
    assert!(parse_accept_output(&t).is_err());
    // a claim listed twice
    let t = format!(
        "accepted sync artifact\npatches: 1\nblobs: 1\nclaims: 2\ntags: 0\nnew objects: 2\nclaim {C1}: Sound (ed25519-aa)\nclaim {C1}: Sound (ed25519-aa)\n"
    );
    assert!(parse_accept_output(&t).is_err());
    // a missing summary line, a wrong first line, an empty output, a count that is not a number
    for t in [
        real().replace("  new objects: 6\n", ""),
        real().replace("accepted sync artifact", "something else"),
        String::new(),
        real().replace("patches: 2", "patches: two"),
    ] {
        assert!(parse_accept_output(&t).is_err(), "{t}");
    }
}

#[test]
fn the_claims_file_is_one_id_per_line() {
    assert_eq!(
        parse_claims_file(&format!("{C1}\n{C2}\n")).unwrap(),
        vec![C1, C2]
    );
    assert_eq!(parse_claims_file("").unwrap(), Vec::<String>::new());
    assert!(parse_claims_file(&format!("{C1}\nnot-an-id\n")).is_err());
}

#[test]
fn pending_output_is_read_and_its_count_must_agree() {
    let ok = format!("pending (accepted, unsealed) patches: 2\n  {C1}\n  {C2}\n");
    assert_eq!(parse_pending_output(&ok).unwrap(), vec![C1, C2]);
    assert_eq!(
        parse_pending_output("pending (accepted, unsealed) patches: 0\n").unwrap(),
        Vec::<String>::new()
    );
    for bad in [
        format!("pending (accepted, unsealed) patches: 3\n  {C1}\n  {C2}\n"),
        format!("pending (accepted, unsealed) patches: 1\n  {C1}\n  {C2}\n"),
        format!("{C1}\n"),
        String::new(),
        "pending (accepted, unsealed) patches: 1\n  zz\n".to_owned(),
        "pending (accepted, unsealed) patches: x\n".to_owned(),
    ] {
        assert!(parse_pending_output(&bad).is_err(), "{bad:?}");
    }
}
