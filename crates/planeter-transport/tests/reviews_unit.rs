//! RFC 005 part 6, T2/T3: the review service over the in-memory stores: every rule of T2 with a negative, the stale approval,
//! the rate limits computed from the records, paging and nesting, moderation. No prikk and no HTTP (those are in
//! `planeter-web/tests/reviews_http.rs`).

use std::sync::Arc;

use planeter_core::review::{
    CommentRefusal, DecisionRefusal, HideRefusal, ReviewState, SelfApproval, Verdict,
};
use planeter_store::{
    ChangeRecord, ChangeState, ChangeStore, InMemoryChangeStore, InMemoryReviewStore, Owner,
    ProposalPolicy, RepoId, RepositoryRecord, ReviewStore, RevisionRecord, Visibility,
};
use planeter_transport::{Limited, NewComment, ReviewConfig, ReviewError, ReviewService};

thread_local! {
    /// The clock of the test: each test runs on its own thread and the service reads it on that thread, so tests that move
    /// the time do not disturb each other.
    static NOW: std::cell::Cell<u64> = const { std::cell::Cell::new(1_000_000) };
}
fn now() -> u64 {
    NOW.with(std::cell::Cell::get)
}
fn at(t: u64) {
    NOW.with(|n| n.set(t));
}

const GRACE: u64 = 30 * 86_400;

fn pid(n: u64) -> String {
    format!("{n:064x}")
}

struct Fixture {
    repo: RepositoryRecord,
    changes: Arc<InMemoryChangeStore>,
    reviews: Arc<InMemoryReviewStore>,
    svc: ReviewService,
}

fn repo(id: &str) -> RepositoryRecord {
    RepositoryRecord {
        repo_id: RepoId::new(id),
        owner: Owner::User("alice".into()),
        name: "app".into(),
        visibility: Visibility::Public,
        created_at: 0,
        prikk_format_version: None,
        proposals: ProposalPolicy::default(),
        path: std::path::PathBuf::from("/nonexistent"),
    }
}

impl Fixture {
    fn new(config: ReviewConfig) -> Fixture {
        at(1_000_000);
        let changes = Arc::new(InMemoryChangeStore::new());
        let reviews = Arc::new(InMemoryReviewStore::new());
        let svc = ReviewService::new(
            changes.clone() as Arc<dyn ChangeStore>,
            reviews.clone() as Arc<dyn ReviewStore>,
            config,
        )
        .with_clock(now);
        Fixture {
            repo: repo("r1"),
            changes,
            reviews,
            svc,
        }
    }

    /// A proposal by `opener` with one revision holding patches `ids`.
    fn propose(&self, id: &str, opener: &str, ids: &[u64], digest: u64) {
        let change = ChangeRecord {
            change_id: id.into(),
            repo_id: self.repo.repo_id.clone(),
            title: format!("title {id}"),
            description: String::new(),
            target_ref: None,
            state: ChangeState::Open,
            closed_reason: None,
            opened_by: opener.into(),
            opened_at: now(),
            updated_at: now(),
            retain_until: None,
        };
        self.revision(Some(&change), id, opener, 1, ids, digest);
    }

    fn revision(
        &self,
        new: Option<&ChangeRecord>,
        id: &str,
        submitter: &str,
        seq: u32,
        ids: &[u64],
        digest: u64,
    ) {
        let rev = RevisionRecord {
            revision_id: format!("v-{id}-{seq}"),
            change_id: id.into(),
            repo_id: self.repo.repo_id.clone(),
            seq,
            artifact_digest: format!("sha256:{digest:064x}"),
            artifact_bytes: 10,
            declared_base: None,
            claim_ids: vec![pid(500 + u64::from(seq))],
            added_patch_ids: ids.iter().map(|n| pid(*n)).collect(),
            declared_patch_ids: None,
            claim_outcomes: vec![],
            submitter: submitter.into(),
            author_key_ids: vec![],
            created_at: now(),
        };
        let intent = format!("i-{id}-{seq}");
        self.changes
            .open_or_resume_intent(
                &intent,
                &self.repo.repo_id,
                &rev.artifact_digest,
                0,
                &[],
                submitter,
                now(),
            )
            .unwrap();
        self.changes
            .complete_intent_with_revision(&intent, new, &rev, now())
            .unwrap();
    }

    fn second_revision(&self, id: &str, submitter: &str, ids: &[u64], digest: u64) {
        self.revision(None, id, submitter, 2, ids, digest);
    }

    fn close(&self, id: &str) {
        self.changes
            .leave_open(
                id,
                ChangeState::Closed,
                Some("done"),
                now(),
                Some(now() + GRACE),
            )
            .unwrap();
    }

    fn comment(
        &self,
        id: &str,
        who: &str,
        body: &str,
    ) -> Result<planeter_store::CommentRecord, ReviewError> {
        self.svc.add_comment(
            &self.repo,
            id,
            who,
            &NewComment {
                body: body.into(),
                ..NewComment::default()
            },
        )
    }

    fn decide(
        &self,
        id: &str,
        who: &str,
        v: Verdict,
    ) -> Result<planeter_store::DecisionRecord, ReviewError> {
        self.svc.decide(&self.repo, id, who, v, None, None)
    }
}

fn cfg() -> ReviewConfig {
    ReviewConfig::default()
}

// ------------------------------------------------------------------------------------------------ comments

#[test]
fn a_comment_is_recorded_with_the_latest_revision_and_nests_one_level() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    f.second_revision("p", "alice", &[2], 101);
    let top = f.comment("p", "bob", "First.").unwrap();
    assert_eq!(
        top.revision_seq, 2,
        "the revision that was latest when it was written"
    );
    assert!(top.comment_id.starts_with("m-") && top.comment_id.len() == 26);
    at(now() + 5);
    let rep = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "carol",
            &NewComment {
                body: "Reply.".into(),
                parent: Some(top.comment_id.clone()),
                ..NewComment::default()
            },
        )
        .unwrap();
    // A reply to a reply is refused.
    let err = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "dave",
            &NewComment {
                body: "x".into(),
                parent: Some(rep.comment_id.clone()),
                ..NewComment::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, ReviewError::Comment(CommentRefusal::ParentNotTopLevel)),
        "{err:?}"
    );
    let page = f.svc.threads(&f.repo, "p", 10, None).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.threads.len(), 1);
    assert_eq!(page.threads[0].replies.len(), 1);
    assert_eq!(page.threads[0].replies[0].comment_id, rep.comment_id);
}

#[test]
fn a_comment_anchors_to_a_patch_of_the_current_set_only_and_keeps_the_anchor_as_written() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1, 2], 100);
    let good = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "bob",
            &NewComment {
                body: "About this.".into(),
                patch_id: Some(pid(1)),
                path: Some("src/a.rs".into()),
                line_from: Some(12),
                line_to: Some(14),
                parent: None,
            },
        )
        .unwrap();
    assert_eq!(
        (
            good.patch_id.as_deref(),
            good.path.as_deref(),
            good.line_from,
            good.line_to
        ),
        (Some(pid(1).as_str()), Some("src/a.rs"), Some(12), Some(14))
    );
    let err = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "bob",
            &NewComment {
                body: "x".into(),
                patch_id: Some(pid(9)),
                ..NewComment::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, ReviewError::Comment(CommentRefusal::PatchNotInSet)),
        "{err:?}"
    );
    // A later undeclared revision adds patch 3; the set is 1, 2, 3 now. Patch 1 stays anchored as written either way.
    f.second_revision("p", "alice", &[3], 101);
    assert!(
        f.svc
            .add_comment(
                &f.repo,
                "p",
                "bob",
                &NewComment {
                    body: "x".into(),
                    patch_id: Some(pid(3)),
                    ..NewComment::default()
                }
            )
            .is_ok()
    );
    assert_eq!(
        f.reviews
            .get_comment(&good.comment_id)
            .unwrap()
            .unwrap()
            .patch_id
            .as_deref(),
        Some(pid(1).as_str())
    );
    // A lone line becomes a single-line range.
    let single = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "bob",
            &NewComment {
                body: "x".into(),
                path: Some("a".into()),
                line_from: Some(7),
                ..NewComment::default()
            },
        )
        .unwrap();
    assert_eq!((single.line_from, single.line_to), (Some(7), Some(7)));
}

#[test]
fn a_parent_of_another_proposal_or_an_unknown_one_is_refused() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    f.propose("q", "alice", &[2], 101);
    let other = f.comment("q", "bob", "On q.").unwrap();
    let e = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "bob",
            &NewComment {
                body: "x".into(),
                parent: Some(other.comment_id),
                ..NewComment::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(e, ReviewError::Comment(CommentRefusal::ParentOtherProposal)),
        "{e:?}"
    );
    let e = f
        .svc
        .add_comment(
            &f.repo,
            "p",
            "bob",
            &NewComment {
                body: "x".into(),
                parent: Some("m-ghost".into()),
                ..NewComment::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(e, ReviewError::Comment(CommentRefusal::ParentUnknown)),
        "{e:?}"
    );
}

#[test]
fn an_unknown_proposal_and_another_repositorys_proposal_are_the_same_not_found() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    assert!(matches!(
        f.comment("nope", "bob", "x"),
        Err(ReviewError::NotFound)
    ));
    let mut other = f.repo.clone();
    other.repo_id = RepoId::new("r2");
    assert!(matches!(
        f.svc.add_comment(
            &other,
            "p",
            "bob",
            &NewComment {
                body: "x".into(),
                ..NewComment::default()
            }
        ),
        Err(ReviewError::NotFound)
    ));
    assert!(matches!(
        f.svc
            .decide(&other, "p", "bob", Verdict::Approve, None, None),
        Err(ReviewError::NotFound)
    ));
    assert!(matches!(
        f.svc.threads(&other, "p", 10, None),
        Err(ReviewError::NotFound)
    ));
    assert!(matches!(
        f.svc.review(&other, "p"),
        Err(ReviewError::NotFound)
    ));
}

#[test]
fn comments_close_thirty_days_after_the_proposal_finished_and_not_before() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    f.close("p"); // retain_until = now + 30 days
    let end = now() + GRACE;
    at(end - 1);
    assert!(
        f.comment("p", "bob", "Afterthought.").is_ok(),
        "discussion after the fact"
    );
    at(end);
    assert!(
        f.comment("p", "carol", "Still fine on the last second.")
            .is_ok()
    );
    at(end + 1);
    match f.comment("p", "dave", "Too late.") {
        Err(ReviewError::CommentsClosed(c)) => {
            assert_eq!((c.state, c.until), (ChangeState::Closed, end))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn nothing_is_recorded_by_a_refused_comment() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    for body in ["", "  ", &"x".repeat(16 * 1024 + 1)] {
        assert!(f.comment("p", "bob", body).is_err());
    }
    assert_eq!(f.svc.threads(&f.repo, "p", 10, None).unwrap().total, 0);
}

// ------------------------------------------------------------------------------------------------ paging and rate

#[test]
fn threads_are_paged_oldest_first_with_all_their_replies_and_a_cursor() {
    let f = Fixture::new(ReviewConfig {
        comments_per_hour: 1000,
        decisions_per_hour: 1000,
    });
    f.propose("p", "alice", &[1], 100);
    let mut tops = Vec::new();
    for i in 0..5u64 {
        at(1_000_000 + i * 10);
        tops.push(f.comment("p", "bob", &format!("top {i}")).unwrap());
    }
    at(2_000_000);
    for t in [&tops[0], &tops[3]] {
        for j in 0..2 {
            at(now() + 1);
            f.svc
                .add_comment(
                    &f.repo,
                    "p",
                    "carol",
                    &NewComment {
                        body: format!("reply {j}"),
                        parent: Some(t.comment_id.clone()),
                        ..NewComment::default()
                    },
                )
                .unwrap();
        }
    }
    let p1 = f.svc.threads(&f.repo, "p", 2, None).unwrap();
    assert_eq!(p1.total, 9);
    assert_eq!(
        p1.threads
            .iter()
            .map(|t| t.comment.body.as_str())
            .collect::<Vec<_>>(),
        ["top 0", "top 1"]
    );
    assert_eq!(p1.threads[0].replies.len(), 2);
    assert!(p1.threads[1].replies.is_empty());
    let p2 = f.svc.threads(&f.repo, "p", 2, p1.next.as_deref()).unwrap();
    assert_eq!(
        p2.threads
            .iter()
            .map(|t| t.comment.body.as_str())
            .collect::<Vec<_>>(),
        ["top 2", "top 3"]
    );
    assert_eq!(
        p2.threads[1]
            .replies
            .iter()
            .map(|r| r.body.as_str())
            .collect::<Vec<_>>(),
        ["reply 0", "reply 1"]
    );
    let p3 = f.svc.threads(&f.repo, "p", 2, p2.next.as_deref()).unwrap();
    assert_eq!(p3.threads.len(), 1);
    assert!(p3.next.is_none());
    for bad in ["", "x", "1.", ".m", "1.../etc", "a.b"] {
        assert!(
            matches!(
                f.svc.threads(&f.repo, "p", 2, Some(bad)),
                Err(ReviewError::BadCursor)
            ),
            "{bad:?}"
        );
    }
    // A limit of 0 or past the maximum is clamped, not refused.
    assert_eq!(
        f.svc.threads(&f.repo, "p", 0, None).unwrap().threads.len(),
        1
    );
    assert_eq!(
        f.svc
            .threads(&f.repo, "p", 10_000, None)
            .unwrap()
            .threads
            .len(),
        5
    );
}

#[test]
fn the_comment_limit_is_per_principal_per_proposal_per_hour_and_counted_from_the_records() {
    let f = Fixture::new(ReviewConfig {
        comments_per_hour: 3,
        decisions_per_hour: 10,
    });
    f.propose("p", "alice", &[1], 100);
    f.propose("q", "alice", &[2], 101);
    let start = now();
    for i in 0..3u64 {
        at(start + i * 100);
        f.comment("p", "bob", &format!("c{i}")).unwrap();
    }
    at(start + 300);
    match f.comment("p", "bob", "one too many") {
        Err(ReviewError::RateLimited {
            what: Limited::Comments,
            retry_after,
        }) => assert_eq!(retry_after, 3600 - 300, "the oldest leaves the window"),
        other => panic!("{other:?}"),
    }
    // Another principal, and another proposal, are not limited by bob's use of this one.
    assert!(f.comment("p", "carol", "mine").is_ok());
    assert!(f.comment("q", "bob", "elsewhere").is_ok());
    // A refused comment was not recorded, so it does not count: when the oldest has left, there is room for exactly one.
    at(start + 3600);
    assert!(f.comment("p", "bob", "room again").is_ok());
    assert!(matches!(
        f.comment("p", "bob", "and now full"),
        Err(ReviewError::RateLimited { .. })
    ));
}

#[test]
fn a_comment_refused_for_a_rule_does_not_count_toward_the_limit() {
    let f = Fixture::new(ReviewConfig {
        comments_per_hour: 1,
        decisions_per_hour: 10,
    });
    f.propose("p", "alice", &[1], 100);
    for _ in 0..5 {
        assert!(matches!(
            f.comment("p", "bob", ""),
            Err(ReviewError::Comment(CommentRefusal::BodyEmpty))
        ));
    }
    assert!(f.comment("p", "bob", "the first real one").is_ok());
}

// ------------------------------------------------------------------------------------------------ decisions

#[test]
fn a_decision_is_recorded_on_the_latest_revision_and_replaced_by_the_next() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    let d = f.decide("p", "bob", Verdict::RequestChanges).unwrap();
    assert_eq!((d.revision_seq, d.verdict.as_str()), (1, "request-changes"));
    assert_eq!(
        f.svc.review(&f.repo, "p").unwrap().standing.state,
        ReviewState::ChangesRequested
    );
    at(now() + 10);
    f.decide("p", "bob", Verdict::Approve).unwrap();
    let r = f.svc.review(&f.repo, "p").unwrap();
    assert_eq!(r.standing.state, ReviewState::Approved, "newest wins");
    assert_eq!(r.current.len(), 1);
    assert_eq!(f.reviews.decision_history("p").unwrap().len(), 1);
}

/// The test the handoff names: approve revision 1, record revision 2.
#[test]
fn the_stale_approval_is_shown_on_the_new_revision_and_counts_for_nothing() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    f.decide("p", "bob", Verdict::Approve).unwrap();
    assert_eq!(
        f.svc.review(&f.repo, "p").unwrap().standing.state,
        ReviewState::Approved
    );
    f.second_revision("p", "alice", &[2], 101);
    let r = f.svc.review(&f.repo, "p").unwrap();
    assert_eq!(r.latest, 2);
    assert_eq!(r.standing.state, ReviewState::AwaitingReview);
    assert_eq!(r.standing.approved_on, [1]);
    assert!(r.standing.approved_by.is_empty() && r.current.is_empty());
    assert!(r.standing.approved_only_earlier());
    assert_eq!(r.earlier.len(), 1);
    assert_eq!(
        (r.earlier[0].reviewer.as_str(), r.earlier[0].revision),
        ("bob", 1)
    );
    // Naming the old revision is refused; the new one is accepted and is bob's to give again.
    assert!(matches!(
        f.svc
            .decide(&f.repo, "p", "bob", Verdict::Approve, Some(1), None),
        Err(ReviewError::Decision(DecisionRefusal::RevisionNotCurrent {
            current: 2
        }))
    ));
    f.decide("p", "bob", Verdict::Approve).unwrap();
    let r = f.svc.review(&f.repo, "p").unwrap();
    assert_eq!(r.standing.state, ReviewState::Approved);
    assert_eq!(
        r.standing.approved_on,
        [1],
        "revision 1's approval is still history"
    );
}

#[test]
fn self_approval_is_refused_in_both_forms_and_a_request_for_changes_is_not() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    assert!(matches!(
        f.decide("p", "alice", Verdict::Approve),
        Err(ReviewError::Decision(DecisionRefusal::SelfApproval(
            SelfApproval::OwnRevision
        )))
    ));
    f.second_revision("p", "carol", &[2], 101);
    // carol submitted the latest revision, alice an earlier one.
    assert!(matches!(
        f.decide("p", "carol", Verdict::Approve),
        Err(ReviewError::Decision(DecisionRefusal::SelfApproval(
            SelfApproval::OwnRevision
        )))
    ));
    assert!(matches!(
        f.decide("p", "alice", Verdict::Approve),
        Err(ReviewError::Decision(DecisionRefusal::SelfApproval(
            SelfApproval::OwnProposal
        )))
    ));
    assert!(f.decide("p", "alice", Verdict::RequestChanges).is_ok());
    assert!(f.comment("p", "alice", "My own comment is fine.").is_ok());
    assert!(f.decide("p", "bob", Verdict::Approve).is_ok());
}

#[test]
fn decisions_are_refused_once_the_proposal_has_left_open() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    f.close("p");
    for v in [Verdict::Approve, Verdict::RequestChanges] {
        assert!(matches!(
            f.decide("p", "bob", v),
            Err(ReviewError::Decision(DecisionRefusal::NotOpen(
                ChangeState::Closed
            )))
        ));
    }
}

#[test]
fn the_decision_limit_counts_replaced_decisions_too() {
    let f = Fixture::new(ReviewConfig {
        comments_per_hour: 20,
        decisions_per_hour: 3,
    });
    f.propose("p", "alice", &[1], 100);
    let start = now();
    for (i, v) in [Verdict::Approve, Verdict::RequestChanges, Verdict::Approve]
        .into_iter()
        .enumerate()
    {
        at(start + i as u64 * 60);
        f.decide("p", "bob", v).unwrap();
    }
    at(start + 200);
    match f.decide("p", "bob", Verdict::RequestChanges) {
        Err(ReviewError::RateLimited {
            what: Limited::Decisions,
            retry_after,
        }) => assert_eq!(retry_after, 3600 - 200),
        other => panic!("{other:?}"),
    }
    assert!(
        f.decide("p", "carol", Verdict::Approve).is_ok(),
        "another reviewer is not limited"
    );
    // The state did not change by the refusal.
    assert_eq!(
        f.svc
            .review(&f.repo, "p")
            .unwrap()
            .current
            .iter()
            .find(|d| d.reviewer == "bob")
            .unwrap()
            .verdict,
        Verdict::Approve
    );
}

#[test]
fn a_note_is_trimmed_and_an_over_long_one_is_refused() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    let d = f
        .svc
        .decide(
            &f.repo,
            "p",
            "bob",
            Verdict::Approve,
            None,
            Some("  Looks right.  "),
        )
        .unwrap();
    assert_eq!(d.note.as_deref(), Some("Looks right."));
    let d = f
        .svc
        .decide(&f.repo, "p", "carol", Verdict::Approve, None, Some("   "))
        .unwrap();
    assert_eq!(d.note, None, "a blank note is no note");
    assert!(matches!(
        f.svc.decide(
            &f.repo,
            "p",
            "dave",
            Verdict::Approve,
            None,
            Some(&"n".repeat(2049))
        ),
        Err(ReviewError::Decision(DecisionRefusal::NoteTooLong))
    ));
}

#[test]
fn the_review_state_follows_the_rules_for_several_reviewers() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    assert_eq!(
        f.svc.review(&f.repo, "p").unwrap().standing.state,
        ReviewState::AwaitingReview
    );
    f.decide("p", "bob", Verdict::Approve).unwrap();
    f.decide("p", "carol", Verdict::Approve).unwrap();
    let r = f.svc.review(&f.repo, "p").unwrap();
    assert_eq!(
        (r.standing.state, r.standing.approved_by.len()),
        (ReviewState::Approved, 2)
    );
    f.decide("p", "dave", Verdict::RequestChanges).unwrap();
    assert_eq!(
        f.svc.review(&f.repo, "p").unwrap().standing.state,
        ReviewState::ChangesRequested
    );
    // Dave changes his mind: approved again, by three.
    at(now() + 5);
    f.decide("p", "dave", Verdict::Approve).unwrap();
    let r = f.svc.review(&f.repo, "p").unwrap();
    assert_eq!(
        (r.standing.state, r.standing.approved_by.len()),
        (ReviewState::Approved, 3)
    );
}

#[test]
fn the_lists_review_words_come_from_the_latest_revision_of_each_proposal() {
    let f = Fixture::new(cfg());
    f.propose("p1", "alice", &[1], 100);
    f.propose("p2", "alice", &[2], 101);
    f.propose("p3", "alice", &[3], 102);
    f.decide("p1", "bob", Verdict::Approve).unwrap();
    f.decide("p2", "bob", Verdict::RequestChanges).unwrap();
    f.second_revision("p1", "alice", &[4], 103); // p1's approval is now stale
    let w = f
        .svc
        .review_words(&[("p1".into(), 2), ("p2".into(), 1), ("p3".into(), 1)]);
    assert_eq!(w["p1"], ReviewState::AwaitingReview);
    assert_eq!(w["p2"], ReviewState::ChangesRequested);
    assert_eq!(w["p3"], ReviewState::AwaitingReview);
}

// ------------------------------------------------------------------------------------------------ moderation

#[test]
fn hiding_needs_a_reason_is_reversible_and_never_changes_the_body() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    let c = f.comment("p", "bob", "Something unkind.").unwrap();
    assert!(matches!(
        f.svc.hide(&f.repo, "p", &c.comment_id, "alice", "  "),
        Err(ReviewError::HideReason(HideRefusal::ReasonRequired))
    ));
    assert!(matches!(
        f.svc
            .hide(&f.repo, "p", &c.comment_id, "alice", &"r".repeat(501)),
        Err(ReviewError::HideReason(HideRefusal::ReasonTooLong))
    ));
    let h = f
        .svc
        .hide(&f.repo, "p", &c.comment_id, "alice", " Not kind. ")
        .unwrap();
    let hid = h.hidden.unwrap();
    assert_eq!(
        (hid.by.as_str(), hid.reason.as_str()),
        ("alice", "Not kind.")
    );
    assert_eq!(h.body, "Something unkind.");
    assert!(matches!(
        f.svc.hide(&f.repo, "p", &c.comment_id, "alice", "again"),
        Err(ReviewError::CommentState { hidden: true })
    ));
    let u = f.svc.unhide(&f.repo, "p", &c.comment_id, "alice").unwrap();
    assert!(u.hidden.is_none());
    assert!(matches!(
        f.svc.unhide(&f.repo, "p", &c.comment_id, "alice"),
        Err(ReviewError::CommentState { hidden: false })
    ));
    assert_eq!(f.reviews.moderation_log(&c.comment_id).unwrap().len(), 2);
}

#[test]
fn a_comment_of_another_proposal_or_an_unknown_one_cannot_be_hidden_through_this_one() {
    let f = Fixture::new(cfg());
    f.propose("p", "alice", &[1], 100);
    f.propose("q", "alice", &[2], 101);
    let on_q = f.comment("q", "bob", "On q.").unwrap();
    assert!(matches!(
        f.svc.hide(&f.repo, "p", &on_q.comment_id, "alice", "r"),
        Err(ReviewError::NotFound)
    ));
    assert!(matches!(
        f.svc.hide(&f.repo, "p", "m-ghost", "alice", "r"),
        Err(ReviewError::NotFound)
    ));
    assert!(matches!(
        f.svc.unhide(&f.repo, "p", &on_q.comment_id, "alice"),
        Err(ReviewError::NotFound)
    ));
    assert!(
        f.reviews
            .get_comment(&on_q.comment_id)
            .unwrap()
            .unwrap()
            .hidden
            .is_none()
    );
}
