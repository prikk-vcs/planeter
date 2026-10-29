//! RFC 004 D-8, the comparison before the download: the recorded sequence and RefState id of each branch against the advertisement.
//! Pure (no network, no prikk): the decisions the fetch command acts on.

use std::collections::BTreeMap;

use planeter_client::commands::{Advert, FreshRefusal, judge_freshness};
use planeter_client::state::{Record, RefRecord};

fn record(refs: &[(&str, Option<u64>, &str)]) -> Record {
    Record {
        key: "https://f.example/a/b".into(),
        refs: refs
            .iter()
            .map(|(n, seq, rs)| {
                (
                    (*n).to_owned(),
                    RefRecord {
                        tip_block: Some("tip".into()),
                        ref_state_id: Some((*rs).into()),
                        update_seq: *seq,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        ..Record::default()
    }
}

fn advert(refs: &[(&str, Option<u64>, &str)]) -> Advert {
    let refs: Vec<_> = refs
        .iter()
        .map(|(n, seq, rs)| {
            let mut r = serde_json::json!({"name": n, "state": "published", "kind": "branch", "closed": false,
                "tip_block": "tip2", "ref_state_id": rs, "fetch": []});
            if let Some(s) = seq {
                r["update_seq"] = serde_json::json!(s);
            }
            r
        })
        .collect();
    Advert::parse(serde_json::json!({"fingerprint": "f", "refs": refs, "archive": {"kind": "archive", "ref": null, "size": null, "etag": "\"e\""}, "limits": null}))
        .unwrap()
}

#[test]
fn a_lower_number_is_refused_whatever_else_is_true() {
    let j = judge_freshness(
        &record(&[("heads/main", Some(7), "rs-7")]),
        &advert(&[("heads/main", Some(6), "rs-6")]),
    );
    assert_eq!(
        j.refuse,
        Some(FreshRefusal::Older {
            name: "heads/main".into(),
            seen: 7,
            shown: 6
        })
    );
}

#[test]
fn the_same_number_is_the_same_state_or_a_refusal() {
    let same = judge_freshness(
        &record(&[("heads/main", Some(7), "rs-7")]),
        &advert(&[("heads/main", Some(7), "rs-7")]),
    );
    assert!(
        same.refuse.is_none() && same.continuation.is_empty() && !same.without_number,
        "{same:?}"
    );
    let other = judge_freshness(
        &record(&[("heads/main", Some(7), "rs-7")]),
        &advert(&[("heads/main", Some(7), "rs-x")]),
    );
    assert_eq!(
        other.refuse,
        Some(FreshRefusal::SameNumberOtherState {
            name: "heads/main".into(),
            seq: 7
        })
    );
}

#[test]
fn a_higher_number_must_still_continue_the_recorded_tip_after_the_download() {
    let j = judge_freshness(
        &record(&[("heads/main", Some(7), "rs-7")]),
        &advert(&[("heads/main", Some(9), "rs-9")]),
    );
    assert!(j.refuse.is_none() && !j.without_number);
    assert_eq!(j.continuation, ["heads/main"]);
}

#[test]
fn no_number_on_either_side_falls_back_to_continuation_and_says_so_only_when_the_server_gave_none()
{
    // The record has no number (made against an older server) and the server now gives one: continuation, no fallback sentence.
    let j = judge_freshness(
        &record(&[("heads/main", None, "rs-1")]),
        &advert(&[("heads/main", Some(2), "rs-2")]),
    );
    assert_eq!(j.continuation, ["heads/main"]);
    assert!(!j.without_number);
    // The server gives none: the fallback, and a recorded number never excuses it.
    let j = judge_freshness(
        &record(&[("heads/main", Some(7), "rs-7")]),
        &advert(&[("heads/main", None, "rs-8")]),
    );
    assert!(j.without_number && j.refuse.is_none());
    assert_eq!(j.continuation, ["heads/main"]);
}

#[test]
fn a_branch_that_vanished_is_left_to_the_landed_history_and_a_new_branch_is_nobodys_business() {
    let j = judge_freshness(
        &record(&[("heads/main", Some(7), "rs-7")]),
        &advert(&[("heads/other", Some(1), "rs-o")]),
    );
    assert!(j.refuse.is_none());
    assert_eq!(j.continuation, ["heads/main"]);
    // Nothing recorded: nothing to compare.
    let j = judge_freshness(
        &Record::default(),
        &advert(&[("heads/main", Some(1), "rs-1")]),
    );
    assert!(j.refuse.is_none() && j.continuation.is_empty());
}

#[test]
fn the_record_only_ever_raises_a_number() {
    let mut rec = record(&[("heads/main", Some(7), "rs-7")]);
    planeter_client::state::record_refs(&mut rec, &advert(&[("heads/main", Some(9), "rs-9")]));
    assert_eq!(rec.refs["heads/main"].update_seq, Some(9));
    assert_eq!(rec.update_seq, Some(9));
    // A server that stops saying numbers does not erase what was seen.
    planeter_client::state::record_refs(&mut rec, &advert(&[("heads/main", None, "rs-10")]));
    assert_eq!(rec.refs["heads/main"].update_seq, Some(9));
}
