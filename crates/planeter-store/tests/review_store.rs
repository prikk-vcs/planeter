//! RFC 005 part 6, T1: comments and decisions. **One suite, run against both implementations** (in memory and SQLite); the
//! tests only SQLite can have (the triggers and CHECKs, the migration over three reopens, the shared connection) are at the
//! bottom.

use planeter_store::{
    ChangeRecord, ChangeState, ChangeStore, CommentRecord, DecisionRecord, InMemoryReviewStore,
    RepoId, ReviewFailPoint, ReviewStore, SqliteChangeStore, SqliteDb, SqliteReviewStore,
    StoreError,
};

fn pid(n: u64) -> String {
    format!("{n:064x}")
}

/// A proposal the comments can refer to (SQLite refers to the `changes` table; memory does not care).
type MakeChange<'a> = &'a dyn Fn(&str);

fn comment(id: &str, change: &str, author: &str, at: u64) -> CommentRecord {
    CommentRecord {
        comment_id: id.to_owned(),
        change_id: change.to_owned(),
        revision_seq: 1,
        patch_id: None,
        path: None,
        line_from: None,
        line_to: None,
        parent: None,
        author: author.to_owned(),
        body: format!("body of {id} with \"quotes\", a newline\nand 日本語 and <b>markup</b>"),
        created_at: at,
        hidden: None,
    }
}

fn reply(id: &str, change: &str, parent: &str, author: &str, at: u64) -> CommentRecord {
    let mut c = comment(id, change, author, at);
    c.parent = Some(parent.to_owned());
    c
}

fn decision(change: &str, seq: u32, who: &str, verdict: &str, at: u64) -> DecisionRecord {
    DecisionRecord {
        change_id: change.to_owned(),
        revision_seq: seq,
        reviewer: who.to_owned(),
        verdict: verdict.to_owned(),
        note: Some(format!("note by {who} at {at}")),
        created_at: at,
    }
}

fn mem() -> InMemoryReviewStore {
    InMemoryReviewStore::new()
}

fn sqlite_env() -> (SqliteReviewStore, impl Fn(&str)) {
    let db = SqliteDb::open_in_memory().unwrap();
    let changes = SqliteChangeStore::new(db.clone()).unwrap();
    let store = SqliteReviewStore::new(db).unwrap();
    let make = move |id: &str| {
        let _ = changes.create_change(&ChangeRecord {
            change_id: id.to_owned(),
            repo_id: RepoId::new("r1"),
            title: format!("title {id}"),
            description: String::new(),
            target_ref: None,
            state: ChangeState::Open,
            closed_reason: None,
            opened_by: "alice".to_owned(),
            opened_at: 1,
            updated_at: 1,
            retain_until: None,
        });
    };
    (store, make)
}

macro_rules! suite {
    ($($name:ident),* $(,)?) => {
        mod in_memory { $( #[test] fn $name() { super::$name(&super::mem(), &|_| {}); } )* }
        mod sqlite { $( #[test] fn $name() { let (s, make) = super::sqlite_env(); super::$name(&s, &make); } )* }
    };
}

suite!(
    a_comment_round_trips_every_field,
    top_level_comments_are_paged_oldest_first_by_a_cursor,
    replies_are_found_under_their_parents_oldest_first,
    a_thread_is_one_level_deep_and_inside_one_proposal,
    a_comment_id_is_used_once,
    the_store_refuses_what_a_comment_may_not_be,
    hiding_records_who_why_and_when_and_unhiding_clears_it_and_both_are_logged,
    hiding_twice_or_unhiding_a_visible_comment_is_refused_and_an_unknown_comment_is_not_found,
    a_comment_is_written_visible,
    comment_times_are_per_author_and_per_proposal_after_the_cutoff,
    a_decision_round_trips_and_the_current_ones_come_back_in_order,
    a_second_decision_replaces_the_first_and_moves_it_to_the_history,
    a_decision_on_another_revision_is_another_decision,
    the_store_refuses_what_a_decision_may_not_be,
    decision_times_count_the_replaced_ones_too,
    decisions_on_picks_the_named_revision_of_each_proposal,
    a_failure_after_the_history_insert_leaves_the_current_decision_untouched,
    comments_and_decisions_of_one_proposal_are_not_another_s,
);

fn a_comment_round_trips_every_field(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    let mut c = comment("m1", "c1", "bob", 100);
    c.revision_seq = 3;
    c.patch_id = Some(pid(7));
    c.path = Some("src/日本語 file.rs".to_owned());
    c.line_from = Some(12);
    c.line_to = Some(14);
    s.add_comment(&c).unwrap();
    assert_eq!(s.get_comment("m1").unwrap(), Some(c));
    assert_eq!(s.get_comment("nope").unwrap(), None);
    assert_eq!(s.count_comments("c1").unwrap(), 1);
}

fn top_level_comments_are_paged_oldest_first_by_a_cursor(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    // Same second twice: the id breaks the tie. Inserted out of order on purpose.
    for (id, at) in [("m3", 30), ("m1", 10), ("m2b", 20), ("m2a", 20), ("m4", 40)] {
        s.add_comment(&comment(id, "c1", "bob", at)).unwrap();
    }
    let ids = |v: Vec<CommentRecord>| v.into_iter().map(|c| c.comment_id).collect::<Vec<_>>();
    assert_eq!(
        ids(s.comment_threads("c1", None, 100).unwrap()),
        ["m1", "m2a", "m2b", "m3", "m4"]
    );
    let first = s.comment_threads("c1", None, 2).unwrap();
    assert_eq!(ids(first.clone()), ["m1", "m2a"]);
    let last = first.last().unwrap();
    let second = s
        .comment_threads("c1", Some((last.created_at, &last.comment_id)), 2)
        .unwrap();
    assert_eq!(ids(second.clone()), ["m2b", "m3"]);
    let last = second.last().unwrap();
    assert_eq!(
        ids(s
            .comment_threads("c1", Some((last.created_at, &last.comment_id)), 2)
            .unwrap()),
        ["m4"]
    );
    assert!(
        s.comment_threads("c1", Some((40, "m4")), 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(s.count_comments("c1").unwrap(), 5);
}

fn replies_are_found_under_their_parents_oldest_first(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    s.add_comment(&comment("a", "c1", "bob", 10)).unwrap();
    s.add_comment(&comment("b", "c1", "bob", 11)).unwrap();
    s.add_comment(&reply("a2", "c1", "a", "carol", 30)).unwrap();
    s.add_comment(&reply("a1", "c1", "a", "dave", 20)).unwrap();
    s.add_comment(&reply("b1", "c1", "b", "carol", 25)).unwrap();
    let ids = |v: Vec<CommentRecord>| v.into_iter().map(|c| c.comment_id).collect::<Vec<_>>();
    assert_eq!(
        ids(s.replies_of("c1", &["a".to_owned()]).unwrap()),
        ["a1", "a2"]
    );
    assert_eq!(
        ids(s
            .replies_of("c1", &["a".to_owned(), "b".to_owned()])
            .unwrap()),
        ["a1", "b1", "a2"]
    );
    assert!(s.replies_of("c1", &[]).unwrap().is_empty());
    assert!(s.replies_of("c1", &["a1".to_owned()]).unwrap().is_empty());
    // The top-level page never carries a reply.
    assert_eq!(ids(s.comment_threads("c1", None, 10).unwrap()), ["a", "b"]);
}

fn a_thread_is_one_level_deep_and_inside_one_proposal(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    make("c2");
    s.add_comment(&comment("a", "c1", "bob", 10)).unwrap();
    s.add_comment(&reply("a1", "c1", "a", "bob", 11)).unwrap();
    // A reply to a reply.
    assert!(matches!(
        s.add_comment(&reply("a1x", "c1", "a1", "bob", 12)),
        Err(StoreError::Invalid(_))
    ));
    // A reply to a comment of another proposal.
    assert!(matches!(
        s.add_comment(&reply("x", "c2", "a", "bob", 12)),
        Err(StoreError::Invalid(_))
    ));
    // A reply to a comment that is not there.
    assert!(matches!(
        s.add_comment(&reply("y", "c1", "ghost", "bob", 12)),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(s.count_comments("c1").unwrap(), 2);
    assert_eq!(s.count_comments("c2").unwrap(), 0);
}

fn a_comment_id_is_used_once(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    s.add_comment(&comment("a", "c1", "bob", 10)).unwrap();
    assert!(matches!(
        s.add_comment(&comment("a", "c1", "carol", 11)),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        s.get_comment("a").unwrap().unwrap().author,
        "bob",
        "the first one stands"
    );
}

fn the_store_refuses_what_a_comment_may_not_be(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    let bad: Vec<(&str, CommentRecord)> = vec![
        (
            "empty body",
            CommentRecord {
                body: String::new(),
                ..comment("e1", "c1", "bob", 1)
            },
        ),
        (
            "blank body",
            CommentRecord {
                body: " \n\t ".to_owned(),
                ..comment("e2", "c1", "bob", 1)
            },
        ),
        (
            "body over 16 KiB",
            CommentRecord {
                body: "x".repeat(16 * 1024 + 1),
                ..comment("e3", "c1", "bob", 1)
            },
        ),
        (
            "no author",
            CommentRecord {
                author: String::new(),
                ..comment("e4", "c1", "", 1)
            },
        ),
        (
            "revision 0",
            CommentRecord {
                revision_seq: 0,
                ..comment("e5", "c1", "bob", 1)
            },
        ),
        (
            "a short patch id",
            CommentRecord {
                patch_id: Some("abc".to_owned()),
                ..comment("e6", "c1", "bob", 1)
            },
        ),
        (
            "an upper-case patch id",
            CommentRecord {
                patch_id: Some("A".repeat(64)),
                ..comment("e7", "c1", "bob", 1)
            },
        ),
        (
            "an empty path",
            CommentRecord {
                path: Some(String::new()),
                ..comment("e8", "c1", "bob", 1)
            },
        ),
        (
            "a path over 1,024 characters",
            CommentRecord {
                path: Some("p".repeat(1025)),
                ..comment("e9", "c1", "bob", 1)
            },
        ),
        (
            "lines without a path",
            CommentRecord {
                line_from: Some(1),
                line_to: Some(2),
                ..comment("e10", "c1", "bob", 1)
            },
        ),
        (
            "a line without its end",
            CommentRecord {
                path: Some("a".to_owned()),
                line_from: Some(1),
                ..comment("e11", "c1", "bob", 1)
            },
        ),
        (
            "line 0",
            CommentRecord {
                path: Some("a".to_owned()),
                line_from: Some(0),
                line_to: Some(2),
                ..comment("e12", "c1", "bob", 1)
            },
        ),
        (
            "lines out of order",
            CommentRecord {
                path: Some("a".to_owned()),
                line_from: Some(5),
                line_to: Some(2),
                ..comment("e13", "c1", "bob", 1)
            },
        ),
    ];
    for (what, c) in bad {
        assert!(
            matches!(s.add_comment(&c), Err(StoreError::Invalid(_))),
            "{what}"
        );
    }
    assert_eq!(s.count_comments("c1").unwrap(), 0, "nothing was written");
    // The limits themselves are fine.
    let mut ok = comment("ok1", "c1", "bob", 1);
    ok.body = "x".repeat(16 * 1024);
    ok.path = Some("p".repeat(1024));
    ok.line_from = Some(1);
    ok.line_to = Some(1);
    s.add_comment(&ok).unwrap();
}

fn a_comment_is_written_visible(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    let mut c = comment("a", "c1", "bob", 1);
    c.hidden = Some(planeter_store::HiddenRecord {
        by: "x".to_owned(),
        reason: "r".to_owned(),
        at: 2,
    });
    assert!(matches!(s.add_comment(&c), Err(StoreError::Invalid(_))));
}

fn hiding_records_who_why_and_when_and_unhiding_clears_it_and_both_are_logged(
    s: &dyn ReviewStore,
    make: MakeChange,
) {
    make("c1");
    s.add_comment(&comment("a", "c1", "bob", 10)).unwrap();
    s.hide_comment("a", "alice", "Off topic.", 50).unwrap();
    let c = s.get_comment("a").unwrap().unwrap();
    let h = c.hidden.expect("hidden");
    assert_eq!(
        (h.by.as_str(), h.reason.as_str(), h.at),
        ("alice", "Off topic.", 50)
    );
    assert_eq!(
        c.body,
        comment("a", "c1", "bob", 10).body,
        "the body is untouched"
    );
    s.unhide_comment("a", "dave", 60).unwrap();
    assert_eq!(s.get_comment("a").unwrap().unwrap().hidden, None);
    // Hidden again, with another reason: a comment can go back and forth, and the log keeps all of it.
    s.hide_comment("a", "alice", "Again.", 70).unwrap();
    let log = s.moderation_log("a").unwrap();
    assert_eq!(
        log.iter()
            .map(|e| (e.action.as_str(), e.by.as_str(), e.reason.as_deref(), e.at))
            .collect::<Vec<_>>(),
        [
            ("hide", "alice", Some("Off topic."), 50),
            ("unhide", "dave", None, 60),
            ("hide", "alice", Some("Again."), 70)
        ]
    );
    assert!(s.moderation_log("nobody").unwrap().is_empty());
}

fn hiding_twice_or_unhiding_a_visible_comment_is_refused_and_an_unknown_comment_is_not_found(
    s: &dyn ReviewStore,
    make: MakeChange,
) {
    make("c1");
    s.add_comment(&comment("a", "c1", "bob", 10)).unwrap();
    assert!(matches!(
        s.unhide_comment("a", "alice", 20),
        Err(StoreError::Duplicate(_))
    ));
    s.hide_comment("a", "alice", "r", 20).unwrap();
    assert!(matches!(
        s.hide_comment("a", "alice", "again", 30),
        Err(StoreError::Duplicate(_))
    ));
    assert_eq!(
        s.get_comment("a").unwrap().unwrap().hidden.unwrap().reason,
        "r",
        "the first hide stands"
    );
    assert!(matches!(
        s.hide_comment("ghost", "alice", "r", 1),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.unhide_comment("ghost", "alice", 1),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.hide_comment("a", "alice", "  ", 1),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        s.moderation_log("a").unwrap().len(),
        1,
        "refusals are not logged"
    );
}

fn comment_times_are_per_author_and_per_proposal_after_the_cutoff(
    s: &dyn ReviewStore,
    make: MakeChange,
) {
    make("c1");
    make("c2");
    for (id, ch, who, at) in [
        ("a", "c1", "bob", 100),
        ("b", "c1", "bob", 200),
        ("c", "c1", "carol", 150),
        ("d", "c2", "bob", 160),
    ] {
        s.add_comment(&comment(id, ch, who, at)).unwrap();
    }
    let mut t = s.comment_times("c1", "bob", 0).unwrap();
    t.sort_unstable();
    assert_eq!(t, [100, 200]);
    assert_eq!(
        s.comment_times("c1", "bob", 100).unwrap(),
        [200],
        "exclusive at the cutoff"
    );
    assert_eq!(s.comment_times("c1", "carol", 0).unwrap(), [150]);
    assert_eq!(s.comment_times("c2", "bob", 0).unwrap(), [160]);
    assert!(s.comment_times("c1", "dave", 0).unwrap().is_empty());
}

fn a_decision_round_trips_and_the_current_ones_come_back_in_order(
    s: &dyn ReviewStore,
    make: MakeChange,
) {
    make("c1");
    s.record_decision(&decision("c1", 2, "carol", "approve", 20))
        .unwrap();
    s.record_decision(&decision("c1", 1, "dave", "request-changes", 10))
        .unwrap();
    s.record_decision(&decision("c1", 2, "bob", "approve", 30))
        .unwrap();
    let cur = s.current_decisions("c1").unwrap();
    assert_eq!(
        cur.iter()
            .map(|d| (d.revision_seq, d.reviewer.as_str(), d.verdict.as_str()))
            .collect::<Vec<_>>(),
        [
            (1, "dave", "request-changes"),
            (2, "bob", "approve"),
            (2, "carol", "approve")
        ]
    );
    assert_eq!(cur[1].note.as_deref(), Some("note by bob at 30"));
    assert!(s.decision_history("c1").unwrap().is_empty());
    assert!(s.current_decisions("other").unwrap().is_empty());
}

fn a_second_decision_replaces_the_first_and_moves_it_to_the_history(
    s: &dyn ReviewStore,
    make: MakeChange,
) {
    make("c1");
    s.record_decision(&decision("c1", 1, "bob", "request-changes", 10))
        .unwrap();
    s.record_decision(&decision("c1", 1, "bob", "approve", 20))
        .unwrap();
    let mut third = decision("c1", 1, "bob", "request-changes", 30);
    third.note = None;
    s.record_decision(&third).unwrap();
    let cur = s.current_decisions("c1").unwrap();
    assert_eq!(
        cur,
        [third],
        "newest wins; exactly one current decision for the key"
    );
    let hist = s.decision_history("c1").unwrap();
    assert_eq!(
        hist.iter()
            .map(|d| (d.verdict.as_str(), d.created_at))
            .collect::<Vec<_>>(),
        [("request-changes", 10), ("approve", 20)],
        "oldest first, each as it was written"
    );
}

fn a_decision_on_another_revision_is_another_decision(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    s.record_decision(&decision("c1", 1, "bob", "approve", 10))
        .unwrap();
    s.record_decision(&decision("c1", 2, "bob", "request-changes", 20))
        .unwrap();
    assert_eq!(
        s.current_decisions("c1").unwrap().len(),
        2,
        "revision 1's approval is not replaced by revision 2's decision"
    );
    assert!(s.decision_history("c1").unwrap().is_empty());
}

fn the_store_refuses_what_a_decision_may_not_be(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    let mut bad = vec![
        decision("c1", 1, "bob", "approved", 1),
        decision("c1", 1, "bob", "Approve", 1),
        decision("c1", 1, "bob", "", 1),
        decision("c1", 0, "bob", "approve", 1),
        decision("c1", 1, "", "approve", 1),
        decision("", 1, "bob", "approve", 1),
    ];
    let mut long = decision("c1", 1, "bob", "approve", 1);
    long.note = Some("n".repeat(2049));
    bad.push(long);
    for d in bad {
        assert!(
            matches!(s.record_decision(&d), Err(StoreError::Invalid(_))),
            "{d:?}"
        );
    }
    assert!(s.current_decisions("c1").unwrap().is_empty());
    let mut ok = decision("c1", 1, "bob", "approve", 1);
    ok.note = Some("n".repeat(2048));
    s.record_decision(&ok).unwrap();
}

fn decision_times_count_the_replaced_ones_too(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    make("c2");
    s.record_decision(&decision("c1", 1, "bob", "approve", 100))
        .unwrap();
    s.record_decision(&decision("c1", 1, "bob", "request-changes", 200))
        .unwrap();
    s.record_decision(&decision("c1", 2, "bob", "approve", 300))
        .unwrap();
    s.record_decision(&decision("c1", 1, "carol", "approve", 150))
        .unwrap();
    s.record_decision(&decision("c2", 1, "bob", "approve", 120))
        .unwrap();
    let mut t = s.decision_times("c1", "bob", 0).unwrap();
    t.sort_unstable();
    assert_eq!(
        t,
        [100, 200, 300],
        "a replaced decision still counts toward the hour"
    );
    assert_eq!(
        s.decision_times("c1", "bob", 100).unwrap().len(),
        2,
        "exclusive at the cutoff"
    );
    assert_eq!(s.decision_times("c1", "carol", 0).unwrap(), [150]);
    assert_eq!(s.decision_times("c2", "bob", 0).unwrap(), [120]);
}

fn decisions_on_picks_the_named_revision_of_each_proposal(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    make("c2");
    s.record_decision(&decision("c1", 1, "bob", "approve", 10))
        .unwrap();
    s.record_decision(&decision("c1", 2, "carol", "approve", 20))
        .unwrap();
    s.record_decision(&decision("c2", 1, "bob", "request-changes", 30))
        .unwrap();
    let got = s
        .decisions_on(&[("c1".to_owned(), 2), ("c2".to_owned(), 1)])
        .unwrap();
    let mut keys: Vec<_> = got
        .iter()
        .map(|d| (d.change_id.as_str(), d.revision_seq, d.reviewer.as_str()))
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, [("c1", 2, "carol"), ("c2", 1, "bob")]);
    assert!(s.decisions_on(&[]).unwrap().is_empty());
}

fn a_failure_after_the_history_insert_leaves_the_current_decision_untouched(
    s: &dyn ReviewStore,
    make: MakeChange,
) {
    make("c1");
    s.record_decision(&decision("c1", 1, "bob", "approve", 10))
        .unwrap();
    s.inject_failure(ReviewFailPoint::AfterHistoryInsert, true);
    let err = s
        .record_decision(&decision("c1", 1, "bob", "request-changes", 20))
        .unwrap_err();
    assert!(matches!(err, StoreError::Backend(_)), "{err}");
    s.inject_failure(ReviewFailPoint::AfterHistoryInsert, false);
    let cur = s.current_decisions("c1").unwrap();
    assert_eq!(
        (cur.len(), cur[0].verdict.as_str(), cur[0].created_at),
        (1, "approve", 10),
        "the first decision stands"
    );
    assert!(
        s.decision_history("c1").unwrap().is_empty(),
        "and nothing leaked into the history"
    );
    // The same call works once the failure is gone.
    s.record_decision(&decision("c1", 1, "bob", "request-changes", 20))
        .unwrap();
    assert_eq!(
        s.current_decisions("c1").unwrap()[0].verdict,
        "request-changes"
    );
    assert_eq!(s.decision_history("c1").unwrap().len(), 1);
}

fn comments_and_decisions_of_one_proposal_are_not_another_s(s: &dyn ReviewStore, make: MakeChange) {
    make("c1");
    make("c2");
    s.add_comment(&comment("a", "c1", "bob", 1)).unwrap();
    s.record_decision(&decision("c1", 1, "bob", "approve", 1))
        .unwrap();
    assert!(s.comment_threads("c2", None, 10).unwrap().is_empty());
    assert!(s.current_decisions("c2").unwrap().is_empty());
}

// ------------------------------------------------------------------------------------------------ only SQLite

fn raw_sqlite() -> (SqliteDb, SqliteReviewStore) {
    let db = SqliteDb::open_in_memory().unwrap();
    let (_, make) = {
        let changes = SqliteChangeStore::new(db.clone()).unwrap();
        (0, move |id: &str| {
            changes
                .create_change(&ChangeRecord {
                    change_id: id.to_owned(),
                    repo_id: RepoId::new("r1"),
                    title: "t".to_owned(),
                    description: String::new(),
                    target_ref: None,
                    state: ChangeState::Open,
                    closed_reason: None,
                    opened_by: "alice".to_owned(),
                    opened_at: 1,
                    updated_at: 1,
                    retain_until: None,
                })
                .unwrap()
        })
    };
    make("c1");
    let s = SqliteReviewStore::new(db.clone()).unwrap();
    (db, s)
}

/// The database itself refuses to change a comment or to delete anything, whoever asks and however.
#[test]
fn sqlite_refuses_an_edit_or_a_delete_of_a_comment_and_of_the_logs() {
    let (db, s) = raw_sqlite();
    s.add_comment(&comment("a", "c1", "bob", 10)).unwrap();
    s.hide_comment("a", "alice", "r", 20).unwrap();
    s.record_decision(&decision("c1", 1, "bob", "approve", 10))
        .unwrap();
    s.record_decision(&decision("c1", 1, "bob", "request-changes", 20))
        .unwrap();
    let conn = db.lock().unwrap();
    for sql in [
        "UPDATE comments SET body = 'edited' WHERE comment_id = 'a'",
        "UPDATE comments SET author = 'mallory' WHERE comment_id = 'a'",
        "UPDATE comments SET created_at = 0 WHERE comment_id = 'a'",
        "UPDATE comments SET parent_id = NULL, patch_id = NULL WHERE comment_id = 'a'",
        "DELETE FROM comments WHERE comment_id = 'a'",
        "DELETE FROM comments",
        "UPDATE comment_moderation SET by_user = 'x'",
        "DELETE FROM comment_moderation",
        "UPDATE decision_history SET verdict = 'approve'",
        "DELETE FROM decision_history",
    ] {
        assert!(conn.execute(sql, []).is_err(), "{sql} must be refused");
    }
    // The moderation columns are the one thing that can change, and only all together.
    assert!(
        conn.execute(
            "UPDATE comments SET hidden_by = NULL WHERE comment_id = 'a'",
            []
        )
        .is_err(),
        "half a moderation record"
    );
    assert!(
        conn.execute(
            "UPDATE comments SET hidden_reason = '   ' WHERE comment_id = 'a'",
            []
        )
        .is_err(),
        "a blank reason"
    );
    drop(conn);
    assert_eq!(
        s.get_comment("a").unwrap().unwrap().body,
        comment("a", "c1", "bob", 10).body
    );
}

/// The CHECKs are the database's own, not only the Rust code's: a statement that goes round the store is refused too.
#[test]
fn sqlite_checks_hold_for_a_statement_that_bypasses_the_store() {
    let (db, _s) = raw_sqlite();
    let conn = db.lock().unwrap();
    let insert = |extra_cols: &str, extra_vals: &str| {
        conn.execute(
            &format!("INSERT INTO comments (comment_id, change_id, revision_seq, author, body, created_at{extra_cols}) VALUES ('z', 'c1', 1, 'bob', 'hi', 1{extra_vals})"),
            [],
        )
    };
    assert!(insert("", "").is_ok());
    conn.execute("DELETE FROM comments WHERE 0", []).unwrap();
    for (cols, vals) in [
        (", line_from", ", 3"),
        (", path, line_from, line_to", ", 'p', 5, 2"),
        (", path, line_from, line_to", ", 'p', 0, 2"),
        (", hidden_by", ", 'x'"),
        (", parent_id", ", 'ghost'"),
        (", patch_id", ", 'abc'"),
    ] {
        let id_fixed = format!(
            "INSERT INTO comments (comment_id, change_id, revision_seq, author, body, created_at{cols}) VALUES ('y', 'c1', 1, 'bob', 'hi', 1{vals})"
        );
        assert!(conn.execute(&id_fixed, []).is_err(), "{cols}{vals}");
    }
    assert!(conn
        .execute("INSERT INTO comments (comment_id, change_id, revision_seq, author, body, created_at) VALUES ('w', 'no-such-change', 1, 'bob', 'hi', 1)", [])
        .is_err(), "a comment of a proposal that does not exist");
    assert!(conn
        .execute("INSERT INTO decisions (change_id, revision_seq, reviewer, verdict, created_at) VALUES ('c1', 1, 'bob', 'maybe', 1)", [])
        .is_err());
    assert!(conn
        .execute("INSERT INTO decisions (change_id, revision_seq, reviewer, verdict, created_at) VALUES ('no-such', 1, 'bob', 'approve', 1)", [])
        .is_err());
    conn.execute("INSERT INTO decisions (change_id, revision_seq, reviewer, verdict, created_at) VALUES ('c1', 1, 'bob', 'approve', 1)", []).unwrap();
    assert!(conn
        .execute("INSERT INTO decisions (change_id, revision_seq, reviewer, verdict, created_at) VALUES ('c1', 1, 'bob', 'approve', 2)", [])
        .is_err(), "one current decision per (proposal, revision, reviewer)");
}

/// A database made by part 5 has no review tables. Opening the review store creates them, over and over, and loses nothing.
#[test]
fn the_part_5_schema_gets_the_review_tables_once_and_survives_three_reopens() {
    let dir = std::env::temp_dir().join(format!("planeter-review-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("planeter.db");
    // Part 5's database: the change tables, a proposal in them, and no review tables.
    {
        let db = SqliteDb::open(&path).unwrap();
        let changes = SqliteChangeStore::new(db.clone()).unwrap();
        changes
            .create_change(&ChangeRecord {
                change_id: "c1".to_owned(),
                repo_id: RepoId::new("r1"),
                title: "an old proposal".to_owned(),
                description: String::new(),
                target_ref: None,
                state: ChangeState::Open,
                closed_reason: None,
                opened_by: "alice".to_owned(),
                opened_at: 1,
                updated_at: 1,
                retain_until: None,
            })
            .unwrap();
        let conn = db.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('comments', 'decisions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "part 5 has no review tables");
    }
    for round in 1..=3u64 {
        let db = SqliteDb::open(&path).unwrap();
        let _changes = SqliteChangeStore::new(db.clone()).unwrap();
        let s = SqliteReviewStore::new(db.clone()).unwrap();
        // What earlier rounds wrote is there.
        assert_eq!(s.count_comments("c1").unwrap(), round - 1);
        assert_eq!(
            s.current_decisions("c1").unwrap().len(),
            (round - 1).min(1) as usize
        );
        s.add_comment(&comment(&format!("m{round}"), "c1", "bob", round))
            .unwrap();
        if round == 1 {
            s.record_decision(&decision("c1", 1, "bob", "approve", round))
                .unwrap();
        } else {
            s.record_decision(&decision("c1", 1, "bob", "request-changes", round))
                .unwrap();
        }
        // The schema is the same object count after every round.
        let conn = db.lock().unwrap();
        let objects: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'comment%' OR name LIKE 'decision%'", [], |r| r.get(0))
            .unwrap();
        assert!(objects >= 12, "tables, indexes and triggers: {objects}");
        let again: i64 = {
            drop(conn);
            let _ = SqliteReviewStore::new(db.clone()).unwrap();
            let conn = db.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'comment%' OR name LIKE 'decision%'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(again, objects, "a second open changes nothing");
    }
    let db = SqliteDb::open(&path).unwrap();
    let s = SqliteReviewStore::new(db).unwrap();
    assert_eq!(s.count_comments("c1").unwrap(), 3);
    assert_eq!(s.decision_history("c1").unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two reviewers at once, and one reviewer twice at once, on the shared connection.
#[test]
fn sqlite_concurrent_decisions_leave_one_current_decision_per_reviewer() {
    let (db, s) = raw_sqlite();
    let s = std::sync::Arc::new(s);
    let mut handles = Vec::new();
    for t in 0..8u64 {
        let s = std::sync::Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            for i in 0..10u64 {
                let who = if t % 2 == 0 { "bob" } else { "carol" };
                let verdict = if (t + i) % 2 == 0 {
                    "approve"
                } else {
                    "request-changes"
                };
                s.record_decision(&decision("c1", 1, who, verdict, 1000 + t * 100 + i))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let cur = s.current_decisions("c1").unwrap();
    assert_eq!(
        cur.len(),
        2,
        "exactly one current decision for bob and one for carol"
    );
    assert_eq!(
        s.decision_history("c1").unwrap().len(),
        80 - 2,
        "every other decision is in the history, none lost"
    );
    let _ = db;
}

/// Two writers racing to comment never lose or duplicate one.
#[test]
fn sqlite_concurrent_comments_are_all_kept() {
    let (_db, s) = raw_sqlite();
    let s = std::sync::Arc::new(s);
    let mut handles = Vec::new();
    for t in 0..6u64 {
        let s = std::sync::Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            for i in 0..10u64 {
                s.add_comment(&comment(&format!("m-{t}-{i}"), "c1", "bob", 10 + i))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(s.count_comments("c1").unwrap(), 60);
}
