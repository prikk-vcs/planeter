//! RFC 005 part 6, T2: the pure rules of review, each with its negative. No store, no prikk, no clock.

use std::collections::BTreeSet;

use planeter_core::review::{
    COMMENT_BODY_MAX_BYTES, COMMENT_PATH_MAX_CHARS, CommentInput, CommentRefusal, CurrentDecision,
    DECISION_NOTE_MAX_BYTES, DecisionContext, DecisionInput, DecisionRefusal,
    HIDE_REASON_MAX_CHARS, HideRefusal, ParentRef, RATE_WINDOW_SECS, ReviewState, SelfApproval,
    Verdict, check_comment, check_comment_window, check_decision, check_hide_reason, rate_check,
    review_standing,
};
use planeter_store::ChangeState;

fn pid(n: u64) -> String {
    format!("{n:064x}")
}

fn d(who: &str, revision: u32, verdict: Verdict) -> CurrentDecision {
    CurrentDecision {
        reviewer: who.to_owned(),
        revision,
        verdict,
    }
}

// ------------------------------------------------------------------------------------------------ the review state

#[test]
fn no_decision_is_awaiting_review() {
    let s = review_standing(&[], 3);
    assert_eq!(s.state, ReviewState::AwaitingReview);
    assert!(s.approved_by.is_empty() && s.changes_requested_by.is_empty());
    assert!(s.approved_on.is_empty() && s.requested_changes_on.is_empty());
    assert!(!s.approved_only_earlier());
}

#[test]
fn one_approval_on_the_latest_revision_is_approved_and_counts_distinct_reviewers() {
    let s = review_standing(
        &[
            d("bob", 2, Verdict::Approve),
            d("carol", 2, Verdict::Approve),
            d("bob", 2, Verdict::Approve), // a duplicate row is one reviewer
        ],
        2,
    );
    assert_eq!(s.state, ReviewState::Approved);
    assert_eq!(s.approved_by, ["bob", "carol"]);
}

#[test]
fn any_request_for_changes_on_the_latest_revision_outweighs_every_approval() {
    let s = review_standing(
        &[
            d("bob", 2, Verdict::Approve),
            d("carol", 2, Verdict::Approve),
            d("dave", 2, Verdict::RequestChanges),
        ],
        2,
    );
    assert_eq!(s.state, ReviewState::ChangesRequested);
    assert_eq!(s.changes_requested_by, ["dave"]);
    assert_eq!(
        s.approved_by,
        ["bob", "carol"],
        "the approvals are still listed"
    );
}

/// D-7: an approval of revision 1 is shown on revision 2 and never counts toward it.
#[test]
fn a_stale_approval_never_counts_toward_the_current_revision() {
    let s = review_standing(&[d("bob", 1, Verdict::Approve)], 2);
    assert_eq!(s.state, ReviewState::AwaitingReview);
    assert!(s.approved_by.is_empty());
    assert_eq!(s.approved_on, [1]);
    assert!(s.approved_only_earlier());
}

#[test]
fn a_stale_request_for_changes_does_not_block_the_current_revision_but_is_shown() {
    let s = review_standing(&[d("dave", 1, Verdict::RequestChanges)], 2);
    assert_eq!(s.state, ReviewState::AwaitingReview);
    assert_eq!(s.requested_changes_on, [1]);
}

#[test]
fn earlier_decisions_are_history_and_the_current_ones_decide_the_state() {
    let s = review_standing(
        &[
            d("bob", 1, Verdict::Approve),
            d("carol", 2, Verdict::Approve),
            d("dave", 1, Verdict::RequestChanges),
            d("bob", 3, Verdict::Approve),
        ],
        3,
    );
    assert_eq!(s.state, ReviewState::Approved);
    assert_eq!(s.approved_by, ["bob"]);
    assert_eq!(s.approved_on, [1, 2]);
    assert_eq!(s.requested_changes_on, [1]);
    assert!(
        !s.approved_only_earlier(),
        "there is an approval of the latest too"
    );
}

#[test]
fn a_decision_on_a_revision_that_does_not_exist_yet_is_ignored() {
    let s = review_standing(&[d("bob", 5, Verdict::Approve)], 2);
    assert_eq!(s.state, ReviewState::AwaitingReview);
    assert!(s.approved_by.is_empty() && s.approved_on.is_empty());
}

#[test]
fn the_verdict_words_round_trip_and_nothing_else_parses() {
    for v in [Verdict::Approve, Verdict::RequestChanges] {
        assert_eq!(Verdict::parse(v.as_str()), Some(v));
    }
    for bad in [
        "",
        "Approve",
        "approved",
        "request_changes",
        "reject",
        " approve",
    ] {
        assert_eq!(Verdict::parse(bad), None, "{bad:?}");
    }
}

// ------------------------------------------------------------------------------------------------ comments

fn set(ids: &[u64]) -> BTreeSet<String> {
    ids.iter().map(|n| pid(*n)).collect()
}

fn ok_input<'a>() -> CommentInput<'a> {
    CommentInput {
        body: "Looks fine to me.",
        patch_id: None,
        path: None,
        line_from: None,
        line_to: None,
        parent: ParentRef::None,
    }
}

#[test]
fn a_plain_comment_is_accepted_with_no_anchor() {
    let a = check_comment(&ok_input(), &set(&[1])).unwrap();
    assert_eq!((a.patch_id, a.path, a.lines), (None, None, None));
}

#[test]
fn the_body_must_say_something_and_fit() {
    for body in ["", "   ", "\n\t "] {
        let mut i = ok_input();
        i.body = body;
        assert_eq!(
            check_comment(&i, &set(&[])),
            Err(CommentRefusal::BodyEmpty),
            "{body:?}"
        );
    }
    let long = "x".repeat(COMMENT_BODY_MAX_BYTES);
    let mut i = ok_input();
    i.body = &long;
    assert!(check_comment(&i, &set(&[])).is_ok(), "exactly the limit");
    let over = "x".repeat(COMMENT_BODY_MAX_BYTES + 1);
    i.body = &over;
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::BodyTooLong)
    );
    // Bytes, not characters: 16 KiB of 3-byte characters is over.
    let wide = "日".repeat(COMMENT_BODY_MAX_BYTES / 3 + 1);
    i.body = &wide;
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::BodyTooLong)
    );
}

#[test]
fn a_patch_anchor_must_be_a_patch_id_in_the_current_set() {
    let mut i = ok_input();
    let in_set = pid(1);
    i.patch_id = Some(&in_set);
    let a = check_comment(&i, &set(&[1, 2])).unwrap();
    assert_eq!(a.patch_id.as_deref(), Some(in_set.as_str()));
    let out = pid(9);
    i.patch_id = Some(&out);
    assert_eq!(
        check_comment(&i, &set(&[1, 2])),
        Err(CommentRefusal::PatchNotInSet)
    );
    for bad in [
        "",
        "abc",
        &"A".repeat(64),
        &"g".repeat(64),
        &"a".repeat(63),
        &"a".repeat(65),
    ] {
        i.patch_id = Some(bad);
        assert_eq!(
            check_comment(&i, &set(&[1])),
            Err(CommentRefusal::PatchIdInvalid),
            "{bad:?}"
        );
    }
}

#[test]
fn a_path_is_text_of_bounded_length_without_control_characters() {
    let mut i = ok_input();
    i.path = Some("src/lib.rs");
    assert!(check_comment(&i, &set(&[])).is_ok());
    // Never opened, so anything printable is fine, including `..` and an absolute path.
    for ok in [
        "../../etc/passwd",
        "/abs/olute",
        "日本語/ファイル.txt",
        "a b",
    ] {
        i.path = Some(ok);
        assert!(check_comment(&i, &set(&[])).is_ok(), "{ok:?}");
    }
    for empty in ["", "  "] {
        i.path = Some(empty);
        assert_eq!(check_comment(&i, &set(&[])), Err(CommentRefusal::PathEmpty));
    }
    let exact = "p".repeat(COMMENT_PATH_MAX_CHARS);
    i.path = Some(&exact);
    assert!(check_comment(&i, &set(&[])).is_ok());
    let over = "p".repeat(COMMENT_PATH_MAX_CHARS + 1);
    i.path = Some(&over);
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::PathTooLong)
    );
    let wide = "日".repeat(COMMENT_PATH_MAX_CHARS);
    i.path = Some(&wide);
    assert!(
        check_comment(&i, &set(&[])).is_ok(),
        "characters, not bytes"
    );
    for bad in ["a\nb", "a\0b", "a\tb", "a\u{7f}b"] {
        i.path = Some(bad);
        assert_eq!(
            check_comment(&i, &set(&[])),
            Err(CommentRefusal::PathInvalid),
            "{bad:?}"
        );
    }
}

#[test]
fn lines_need_a_path_are_positive_and_ordered() {
    let mut i = ok_input();
    i.line_from = Some(12);
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::LinesNeedAPath)
    );
    i.path = Some("a.rs");
    assert_eq!(
        check_comment(&i, &set(&[])).unwrap().lines,
        Some((12, 12)),
        "a lone line is a single line"
    );
    i.line_to = Some(14);
    assert_eq!(check_comment(&i, &set(&[])).unwrap().lines, Some((12, 14)));
    i.line_to = Some(12);
    assert_eq!(
        check_comment(&i, &set(&[])).unwrap().lines,
        Some((12, 12)),
        "from == to is allowed"
    );
    i.line_to = Some(11);
    assert_eq!(check_comment(&i, &set(&[])), Err(CommentRefusal::LineOrder));
    i.line_from = Some(0);
    i.line_to = Some(3);
    assert_eq!(check_comment(&i, &set(&[])), Err(CommentRefusal::LineZero));
    i.line_from = Some(3);
    i.line_to = Some(0);
    assert_eq!(check_comment(&i, &set(&[])), Err(CommentRefusal::LineZero));
    i.line_from = None;
    i.line_to = Some(3);
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::LinesIncomplete)
    );
}

#[test]
fn a_thread_is_one_level_deep_inside_one_proposal() {
    let mut i = ok_input();
    i.parent = ParentRef::Found {
        in_this_proposal: true,
        top_level: true,
    };
    assert!(check_comment(&i, &set(&[])).is_ok());
    i.parent = ParentRef::Found {
        in_this_proposal: true,
        top_level: false,
    };
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::ParentNotTopLevel)
    );
    i.parent = ParentRef::Found {
        in_this_proposal: false,
        top_level: true,
    };
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::ParentOtherProposal)
    );
    i.parent = ParentRef::Found {
        in_this_proposal: false,
        top_level: false,
    };
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::ParentOtherProposal),
        "the other proposal is said first"
    );
    i.parent = ParentRef::Missing;
    assert_eq!(
        check_comment(&i, &set(&[])),
        Err(CommentRefusal::ParentUnknown)
    );
}

#[test]
fn every_comment_refusal_has_its_own_stable_reason() {
    use CommentRefusal::*;
    let all = [
        BodyEmpty,
        BodyTooLong,
        PatchIdInvalid,
        PatchNotInSet,
        PathEmpty,
        PathTooLong,
        PathInvalid,
        LinesNeedAPath,
        LinesIncomplete,
        LineZero,
        LineOrder,
        ParentUnknown,
        ParentOtherProposal,
        ParentNotTopLevel,
    ];
    let reasons: BTreeSet<&str> = all.iter().map(|r| r.reason()).collect();
    assert_eq!(reasons.len(), all.len());
    assert!(
        reasons
            .iter()
            .all(|r| r.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
    );
}

// ------------------------------------------------------------------------------------------------ the comment window

#[test]
fn comments_are_accepted_while_open_and_until_the_retention_ends_after() {
    assert!(check_comment_window(ChangeState::Open, None, 10_000_000).is_ok());
    assert!(
        check_comment_window(ChangeState::Open, Some(5), 10_000_000).is_ok(),
        "an open proposal has no end"
    );
    for s in [ChangeState::Landed, ChangeState::Closed] {
        assert!(check_comment_window(s, Some(1000), 999).is_ok());
        assert!(
            check_comment_window(s, Some(1000), 1000).is_ok(),
            "inclusive, like the retention"
        );
        let e = check_comment_window(s, Some(1000), 1001).unwrap_err();
        assert_eq!((e.state, e.until), (s, 1000));
        // A finished proposal with no end recorded accepts none.
        assert!(check_comment_window(s, None, 0).is_err());
    }
}

// ------------------------------------------------------------------------------------------------ decisions

fn subs(rows: &[(u32, &str)]) -> Vec<(u32, String)> {
    rows.iter().map(|(n, w)| (*n, (*w).to_owned())).collect()
}

fn ctx<'a>(state: ChangeState, latest: u32, s: &'a [(u32, String)]) -> DecisionContext<'a> {
    DecisionContext {
        state,
        latest,
        submitters: s,
    }
}

fn dec<'a>(who: &'a str, v: Verdict, rev: Option<u32>) -> DecisionInput<'a> {
    DecisionInput {
        reviewer: who,
        verdict: v,
        revision: rev,
        note: None,
    }
}

#[test]
fn a_decision_is_for_the_latest_revision_by_default() {
    let s = subs(&[(1, "alice"), (2, "alice")]);
    assert_eq!(
        check_decision(
            &dec("bob", Verdict::Approve, None),
            &ctx(ChangeState::Open, 2, &s)
        ),
        Ok(2)
    );
    assert_eq!(
        check_decision(
            &dec("bob", Verdict::Approve, Some(2)),
            &ctx(ChangeState::Open, 2, &s)
        ),
        Ok(2)
    );
}

#[test]
fn naming_another_revision_is_refused_whether_older_or_not_yet_there() {
    let s = subs(&[(1, "alice"), (2, "alice")]);
    for rev in [1, 3, 99, 0] {
        assert_eq!(
            check_decision(
                &dec("bob", Verdict::Approve, Some(rev)),
                &ctx(ChangeState::Open, 2, &s)
            ),
            Err(DecisionRefusal::RevisionNotCurrent { current: 2 }),
            "{rev}"
        );
    }
}

#[test]
fn decisions_are_accepted_only_while_the_proposal_is_open() {
    let s = subs(&[(1, "alice")]);
    for st in [ChangeState::Landed, ChangeState::Closed] {
        for v in [Verdict::Approve, Verdict::RequestChanges] {
            assert_eq!(
                check_decision(&dec("bob", v, None), &ctx(st, 1, &s)),
                Err(DecisionRefusal::NotOpen(st))
            );
        }
    }
}

#[test]
fn the_submitter_of_a_revision_cannot_approve_it() {
    let s = subs(&[(1, "alice"), (2, "carol")]);
    assert_eq!(
        check_decision(
            &dec("carol", Verdict::Approve, None),
            &ctx(ChangeState::Open, 2, &s)
        ),
        Err(DecisionRefusal::SelfApproval(SelfApproval::OwnRevision))
    );
}

#[test]
fn a_person_who_submitted_any_revision_cannot_approve_the_latest_either() {
    let s = subs(&[(1, "alice"), (2, "carol")]);
    assert_eq!(
        check_decision(
            &dec("alice", Verdict::Approve, None),
            &ctx(ChangeState::Open, 2, &s)
        ),
        Err(DecisionRefusal::SelfApproval(SelfApproval::OwnProposal))
    );
}

#[test]
fn the_submitter_may_still_ask_for_changes() {
    let s = subs(&[(1, "alice")]);
    assert_eq!(
        check_decision(
            &dec("alice", Verdict::RequestChanges, None),
            &ctx(ChangeState::Open, 1, &s)
        ),
        Ok(1)
    );
}

#[test]
fn a_note_has_a_limit_in_bytes_and_nothing_else_is_required() {
    let s = subs(&[(1, "alice")]);
    let c = ctx(ChangeState::Open, 1, &s);
    let exact = "n".repeat(DECISION_NOTE_MAX_BYTES);
    let mut i = dec("bob", Verdict::Approve, None);
    i.note = Some(&exact);
    assert_eq!(check_decision(&i, &c), Ok(1));
    let over = "n".repeat(DECISION_NOTE_MAX_BYTES + 1);
    i.note = Some(&over);
    assert_eq!(check_decision(&i, &c), Err(DecisionRefusal::NoteTooLong));
    i.note = Some("");
    assert_eq!(check_decision(&i, &c), Ok(1), "an empty note is no note");
}

#[test]
fn the_checks_run_in_a_fixed_order_state_revision_self_note() {
    let s = subs(&[(1, "alice")]);
    let over = "n".repeat(DECISION_NOTE_MAX_BYTES + 1);
    let mut i = dec("alice", Verdict::Approve, Some(7));
    i.note = Some(&over);
    assert_eq!(
        check_decision(&i, &ctx(ChangeState::Closed, 1, &s)),
        Err(DecisionRefusal::NotOpen(ChangeState::Closed))
    );
    assert_eq!(
        check_decision(&i, &ctx(ChangeState::Open, 1, &s)),
        Err(DecisionRefusal::RevisionNotCurrent { current: 1 })
    );
    i.revision = None;
    assert_eq!(
        check_decision(&i, &ctx(ChangeState::Open, 1, &s)),
        Err(DecisionRefusal::SelfApproval(SelfApproval::OwnRevision))
    );
    i.verdict = Verdict::RequestChanges;
    assert_eq!(
        check_decision(&i, &ctx(ChangeState::Open, 1, &s)),
        Err(DecisionRefusal::NoteTooLong)
    );
}

// ------------------------------------------------------------------------------------------------ hiding and limits

#[test]
fn a_hide_needs_a_reason_that_fits() {
    assert_eq!(check_hide_reason("  Off topic.  ").unwrap(), "Off topic.");
    for empty in ["", "   ", "\n"] {
        assert_eq!(check_hide_reason(empty), Err(HideRefusal::ReasonRequired));
    }
    assert!(check_hide_reason(&"r".repeat(HIDE_REASON_MAX_CHARS)).is_ok());
    assert_eq!(
        check_hide_reason(&"r".repeat(HIDE_REASON_MAX_CHARS + 1)),
        Err(HideRefusal::ReasonTooLong)
    );
    assert!(
        check_hide_reason(&"理".repeat(HIDE_REASON_MAX_CHARS)).is_ok(),
        "characters, not bytes"
    );
}

#[test]
fn the_rate_limit_counts_the_last_hour_and_says_when_there_is_room() {
    let now = 100_000;
    // Under the limit: fine. Old ones do not count.
    assert_eq!(rate_check(&[], 3, now), Ok(()));
    assert_eq!(rate_check(&[now - 10, now - 20], 3, now), Ok(()));
    assert_eq!(
        rate_check(
            &[now - RATE_WINDOW_SECS, now - RATE_WINDOW_SECS - 5, now - 1],
            2,
            now
        ),
        Ok(()),
        "exactly an hour ago is out"
    );
    // At the limit: refused until the deciding one leaves the window.
    let times = [now - 3000, now - 2000, now - 100];
    assert_eq!(
        rate_check(&times, 3, now),
        Err(600),
        "the oldest leaves in 600 s"
    );
    assert_eq!(
        rate_check(&times, 2, now),
        Err(RATE_WINDOW_SECS - 2000),
        "with a limit of 2 the middle one decides"
    );
    // Order of the input does not matter.
    assert_eq!(
        rate_check(&[times[2], times[0], times[1]], 3, now),
        Err(600)
    );
    // Over the limit (the limit was lowered): wait for enough of them to leave.
    let many = [now - 3500, now - 3000, now - 2000, now - 1000, now - 10];
    assert_eq!(rate_check(&many, 2, now), Err(RATE_WINDOW_SECS - 1000));
    // Never zero seconds.
    assert_eq!(rate_check(&[now - RATE_WINDOW_SECS + 1], 1, now), Err(1));
}
