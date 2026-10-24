//! RFC 005 D-1…D-3, D-9 and RFC 004 D-3, D-6: the change, revision and intent records. **One suite, run against both
//! implementations** (in memory and SQLite); the tests that only SQLite can have (the schema, the migration, the
//! shared connection, the size of an intent) are at the bottom.

use planeter_store::{
    ChangeRecord, ChangeState, ChangeStore, ClaimOutcome, FailPoint, InMemoryChangeStore,
    IntentState, RepoId, RevisionRecord, SqliteChangeStore, SqliteDb, StoreError, pack_ids,
    unpack_ids,
};

/// The clock of the usage tests.
const NOW: u64 = 500;

fn pid(n: u64) -> String {
    format!("{n:064x}")
}
fn pids(r: std::ops::Range<u64>) -> Vec<String> {
    r.map(pid).collect()
}
fn repo() -> RepoId {
    RepoId::new("r1")
}
fn digest(n: u64) -> String {
    format!("sha256:{n:064x}")
}

fn change(id: &str, by: &str, at: u64) -> ChangeRecord {
    ChangeRecord {
        change_id: id.to_owned(),
        repo_id: repo(),
        title: format!("title {id}"),
        description: "text with \"quotes\", a newline\nand 日本語".to_owned(),
        target_ref: Some("heads/main".to_owned()),
        state: ChangeState::Open,
        closed_reason: None,
        opened_by: by.to_owned(),
        opened_at: at,
        updated_at: at,
        retain_until: None,
    }
}

fn revision(id: &str, change_id: &str, seq: u32, dig: u64, bytes: u64, by: &str) -> RevisionRecord {
    RevisionRecord {
        revision_id: id.to_owned(),
        change_id: change_id.to_owned(),
        repo_id: repo(),
        seq,
        artifact_digest: digest(dig),
        artifact_bytes: bytes,
        declared_base: Some(pid(900)),
        claim_ids: pids(500..502),
        added_patch_ids: pids(1..4),
        declared_patch_ids: Some(pids(1..6)),
        claim_outcomes: vec![
            ClaimOutcome {
                claim_id: pid(500),
                outcome: "adopted".to_owned(),
            },
            ClaimOutcome {
                claim_id: pid(501),
                outcome: "not \"adopted\"\n(key ed25519-ab)".to_owned(),
            },
        ],
        submitter: by.to_owned(),
        author_key_ids: vec!["ed25519-aa".to_owned(), "ed25519-bb".to_owned()],
        created_at: 100,
    }
}

/// Journal an intent for `dig`, then complete it with a revision that opens `change_id` (seq 1).
fn push_new_change(s: &dyn ChangeStore, n: u64, by: &str, bytes: u64) -> (String, String) {
    let (cid, rid, iid) = (format!("c{n}"), format!("v{n}"), format!("i{n}"));
    s.open_or_resume_intent(&iid, &repo(), &digest(n), 0, &[], by, 10)
        .unwrap();
    s.complete_intent_with_revision(
        &iid,
        Some(&change(&cid, by, 10)),
        &revision(&rid, &cid, 1, n, bytes, by),
        11,
    )
    .unwrap();
    (cid, rid)
}

fn mem() -> InMemoryChangeStore {
    InMemoryChangeStore::new()
}
fn sqlite() -> SqliteChangeStore {
    SqliteChangeStore::new(SqliteDb::open_in_memory().unwrap()).unwrap()
}

macro_rules! suite {
    ($($name:ident),* $(,)?) => {
        mod in_memory { $( #[test] fn $name() { super::$name(&super::mem()); } )* }
        mod sqlite { $( #[test] fn $name() { super::$name(&super::sqlite()); } )* }
    };
}

suite!(
    changes_are_created_read_and_listed,
    a_revision_round_trips_every_field,
    a_second_revision_is_numbered_two_and_touches_the_change,
    an_identical_artifact_is_one_revision,
    a_revision_number_is_used_once_per_change,
    resuming_keeps_the_first_attempts_pending_before,
    an_intent_for_another_artifact_is_a_separate_intent,
    an_abandoned_intent_does_not_block_a_fresh_one,
    a_killed_accept_records_nothing_and_the_resumed_one_records_exactly_one,
    completion_is_atomic_when_a_failure_follows_the_revision_insert,
    a_failed_completion_of_an_existing_change_leaves_it_untouched,
    completion_refuses_what_does_not_belong_together,
    malformed_ids_are_refused_and_nothing_is_written,
    only_an_open_change_changes_state,
    the_recovery_flag_is_per_repository,
    usage_is_computed_from_the_records_and_cannot_drift,
    changes_are_listed_per_repository_and_state_newest_first,
    an_intent_can_only_be_abandoned_while_open,
    an_open_intent_owes_its_bytes_until_a_revision_or_an_abandonment_replaces_it,
    flagged_repositories_are_listed,
    leaving_open_on_an_observation_wins_only_if_nothing_was_recorded_since,
    changes_are_counted_per_repository_and_state,
    expired_revisions_are_those_of_final_changes_past_their_grace,
    an_artifact_is_needed_by_an_open_change_a_change_in_grace_or_an_open_intent,
);

/// RFC 005 part 5, T1: the compare-and-set of landing.
fn leaving_open_on_an_observation_wins_only_if_nothing_was_recorded_since(s: &dyn ChangeStore) {
    let (cid, _) = push_new_change(s, 1, "alice", 10);
    // The observation saw one revision; it is still the only one: applied.
    assert!(
        s.leave_open_observed(&cid, 1, ChangeState::Landed, None, 20, Some(120))
            .unwrap()
    );
    let c = s.get_change(&cid).unwrap().unwrap();
    assert_eq!(
        (c.state, c.updated_at, c.retain_until),
        (ChangeState::Landed, 20, Some(120))
    );
    // A final change is not touched again, by an observation or otherwise.
    assert!(
        !s.leave_open_observed(&cid, 1, ChangeState::Closed, Some("x"), 30, Some(130))
            .unwrap()
    );
    assert_eq!(
        s.get_change(&cid).unwrap().unwrap().state,
        ChangeState::Landed
    );

    // A second revision recorded after the observation wins: the observation is discarded and the change stays open.
    let (cid2, _) = push_new_change(s, 2, "alice", 10);
    s.open_or_resume_intent("i2b", &repo(), &digest(22), 0, &[], "alice", 12)
        .unwrap();
    s.complete_intent_with_revision("i2b", None, &revision("v2b", &cid2, 2, 22, 10, "alice"), 13)
        .unwrap();
    assert!(
        !s.leave_open_observed(&cid2, 1, ChangeState::Landed, None, 40, Some(140))
            .unwrap(),
        "it saw one revision, there are two"
    );
    let c2 = s.get_change(&cid2).unwrap().unwrap();
    assert_eq!((c2.state, c2.retain_until), (ChangeState::Open, None));
    // Observing both revisions applies it.
    assert!(
        s.leave_open_observed(&cid2, 2, ChangeState::Landed, None, 41, Some(141))
            .unwrap()
    );
    // A close needs its reason here as everywhere; an unknown change is NotFound.
    let (cid3, _) = push_new_change(s, 3, "alice", 10);
    assert!(matches!(
        s.leave_open_observed(&cid3, 1, ChangeState::Closed, None, 50, None),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        s.get_change(&cid3).unwrap().unwrap().state,
        ChangeState::Open
    );
    assert!(matches!(
        s.leave_open_observed("nope", 1, ChangeState::Landed, None, 50, None),
        Err(StoreError::NotFound)
    ));
}

fn changes_are_counted_per_repository_and_state(s: &dyn ChangeStore) {
    let other = RepoId::new("elsewhere");
    s.create_change(&change("a", "alice", 1)).unwrap();
    s.create_change(&change("b", "alice", 2)).unwrap();
    let mut c = change("c", "bob", 3);
    c.repo_id = other.clone();
    s.create_change(&c).unwrap();
    s.leave_open("b", ChangeState::Closed, Some("done"), 5, Some(50))
        .unwrap();
    assert_eq!(s.count_changes(&repo(), None).unwrap(), 2);
    assert_eq!(
        s.count_changes(&repo(), Some(ChangeState::Open)).unwrap(),
        1
    );
    assert_eq!(
        s.count_changes(&repo(), Some(ChangeState::Closed)).unwrap(),
        1
    );
    assert_eq!(
        s.count_changes(&repo(), Some(ChangeState::Landed)).unwrap(),
        0
    );
    assert_eq!(s.count_changes(&other, None).unwrap(), 1);
    assert_eq!(s.count_changes(&RepoId::new("none"), None).unwrap(), 0);
}

fn expired_revisions_are_those_of_final_changes_past_their_grace(s: &dyn ChangeStore) {
    let (open, _) = push_new_change(s, 1, "alice", 10);
    let (landed, _) = push_new_change(s, 2, "alice", 10);
    let (closed, _) = push_new_change(s, 3, "alice", 10);
    let (late, _) = push_new_change(s, 4, "alice", 10);
    s.leave_open(&landed, ChangeState::Landed, None, 20, Some(100))
        .unwrap();
    s.leave_open(&closed, ChangeState::Closed, Some("x"), 20, Some(200))
        .unwrap();
    s.leave_open(&late, ChangeState::Closed, Some("x"), 20, Some(1000))
        .unwrap();
    let ids = |now: u64, limit: usize| -> Vec<String> {
        s.expired_revisions(now, None, limit)
            .unwrap()
            .into_iter()
            .map(|r| r.change_id)
            .collect()
    };
    assert!(
        ids(100, 10).is_empty(),
        "at retain_until itself it is still kept (the boundary is inclusive)"
    );
    assert_eq!(ids(101, 10), vec![landed.clone()]);
    assert_eq!(ids(201, 10), vec![landed.clone(), closed.clone()]);
    assert_eq!(ids(201, 1), vec![landed.clone()], "bounded work per call");
    // The next call goes on after the last revision it was given, so the same ones are not asked for again.
    let first = s.expired_revisions(5000, None, 2).unwrap();
    assert_eq!(first.len(), 2);
    let rest = s
        .expired_revisions(5000, Some(&first[1].revision_id), 2)
        .unwrap();
    assert_eq!(rest.len(), 1);
    assert!(first.iter().all(|r| r.revision_id < rest[0].revision_id));
    assert!(
        s.expired_revisions(5000, Some(&rest[0].revision_id), 2)
            .unwrap()
            .is_empty()
    );
    assert!(
        !ids(5000, 10).contains(&open),
        "an open change is never a candidate"
    );
    assert_eq!(ids(5000, 10).len(), 3);
}

fn an_artifact_is_needed_by_an_open_change_a_change_in_grace_or_an_open_intent(
    s: &dyn ChangeStore,
) {
    let (open, _) = push_new_change(s, 1, "alice", 10);
    let (done, _) = push_new_change(s, 2, "alice", 10);
    s.leave_open(&done, ChangeState::Closed, Some("x"), 20, Some(100))
        .unwrap();
    assert!(s.artifact_needed(&digest(1), 5000).unwrap(), "open change");
    assert!(
        s.artifact_needed(&digest(2), 100).unwrap(),
        "in grace at the boundary"
    );
    assert!(
        !s.artifact_needed(&digest(2), 101).unwrap(),
        "past its grace"
    );
    assert!(
        !s.artifact_needed(&digest(99), 0).unwrap(),
        "an unknown digest"
    );
    // An open intent names the digest of an artifact that is not recorded at all.
    s.open_or_resume_intent("iX", &repo(), &digest(77), 5, &[], "alice", 30)
        .unwrap();
    assert!(s.artifact_needed(&digest(77), 5000).unwrap());
    s.abandon_intent("iX", 31).unwrap();
    assert!(!s.artifact_needed(&digest(77), 5000).unwrap());
    // The same digest in a second repository: a revision of an open change there keeps it, whatever happened here.
    let other = RepoId::new("elsewhere");
    let mut c = change("other-c", "bob", 40);
    c.repo_id = other.clone();
    let mut r = revision("other-v", "other-c", 1, 2, 10, "bob");
    r.repo_id = other.clone();
    s.open_or_resume_intent("iO", &other, &digest(2), 0, &[], "bob", 40)
        .unwrap();
    s.complete_intent_with_revision("iO", Some(&c), &r, 41)
        .unwrap();
    assert!(s.artifact_needed(&digest(2), 5000).unwrap());
    let _ = open;
}

fn changes_are_created_read_and_listed(s: &dyn ChangeStore) {
    let c = change("c1", "alice", 5);
    s.create_change(&c).unwrap();
    assert_eq!(s.get_change("c1").unwrap(), Some(c.clone()));
    assert_eq!(s.get_change("nope").unwrap(), None);
    assert!(
        matches!(s.create_change(&c), Err(StoreError::Conflict(_))),
        "a change id is used once"
    );
}

fn a_revision_round_trips_every_field(s: &dyn ChangeStore) {
    let (cid, _) = push_new_change(s, 1, "alice", 1234);
    let got = s.revisions_of(&cid).unwrap();
    assert_eq!(got, vec![revision("v1", "c1", 1, 1, 1234, "alice")]);
    assert_eq!(
        s.revision_by_digest(&repo(), &digest(1)).unwrap(),
        Some(got[0].clone())
    );
    assert_eq!(s.revision_by_digest(&repo(), &digest(2)).unwrap(), None);
    assert_eq!(
        s.revision_by_digest(&RepoId::new("other"), &digest(1))
            .unwrap(),
        None
    );
}

fn a_second_revision_is_numbered_two_and_touches_the_change(s: &dyn ChangeStore) {
    let (cid, _) = push_new_change(s, 1, "alice", 10);
    s.open_or_resume_intent("i-2", &repo(), &digest(2), 0, &pids(1..4), "alice", 20)
        .unwrap();
    s.complete_intent_with_revision("i-2", None, &revision("v-2", &cid, 2, 2, 20, "alice"), 25)
        .unwrap();
    let seqs: Vec<u32> = s
        .revisions_of(&cid)
        .unwrap()
        .iter()
        .map(|r| r.seq)
        .collect();
    assert_eq!(seqs, vec![1, 2]);
    assert_eq!(s.get_change(&cid).unwrap().unwrap().updated_at, 25);
    // A declared set of none and of "empty" are two different facts and stay different.
    s.open_or_resume_intent("i-3", &repo(), &digest(3), 0, &[], "alice", 30)
        .unwrap();
    let mut r3 = revision("v-3", &cid, 3, 3, 1, "alice");
    r3.declared_patch_ids = Some(vec![]);
    r3.added_patch_ids = vec![];
    r3.claim_ids = vec![];
    r3.claim_outcomes = vec![];
    r3.author_key_ids = vec![];
    s.complete_intent_with_revision("i-3", None, &r3, 31)
        .unwrap();
    assert_eq!(s.revisions_of(&cid).unwrap()[2], r3);
    s.open_or_resume_intent("i-4", &repo(), &digest(4), 0, &[], "alice", 40)
        .unwrap();
    let mut r4 = r3.clone();
    r4.revision_id = "v-4".to_owned();
    r4.seq = 4;
    r4.artifact_digest = digest(4);
    r4.declared_patch_ids = None;
    s.complete_intent_with_revision("i-4", None, &r4, 41)
        .unwrap();
    assert_eq!(s.revisions_of(&cid).unwrap()[3].declared_patch_ids, None);
}

fn an_identical_artifact_is_one_revision(s: &dyn ChangeStore) {
    push_new_change(s, 1, "alice", 10);
    // A second intent for the same digest can be opened once the first is complete (a race, a repeated push) ...
    s.open_or_resume_intent("i-again", &repo(), &digest(1), 0, &[], "bob", 50)
        .unwrap();
    // ... but its revision is refused by the store, in a typed way, and the intent stays open.
    let err = s
        .complete_intent_with_revision(
            "i-again",
            Some(&change("c-again", "bob", 50)),
            &revision("v-again", "c-again", 1, 1, 10, "bob"),
            51,
        )
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Duplicate(rule) if rule.contains("identical artifact")),
        "{err:?}"
    );
    assert_eq!(
        s.get_intent("i-again").unwrap().unwrap().state,
        IntentState::Open
    );
    assert_eq!(
        s.get_change("c-again").unwrap(),
        None,
        "the change the push would have opened was not created"
    );
}

fn a_revision_number_is_used_once_per_change(s: &dyn ChangeStore) {
    let (cid, _) = push_new_change(s, 1, "alice", 10);
    s.open_or_resume_intent("i-2", &repo(), &digest(2), 0, &[], "alice", 20)
        .unwrap();
    let err = s
        .complete_intent_with_revision("i-2", None, &revision("v-2", &cid, 1, 2, 10, "alice"), 21)
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Duplicate(rule) if rule.contains("numbered once")),
        "{err:?}"
    );
    assert_eq!(s.revisions_of(&cid).unwrap().len(), 1);
    assert_eq!(
        s.get_intent("i-2").unwrap().unwrap().state,
        IntentState::Open
    );
}

fn resuming_keeps_the_first_attempts_pending_before(s: &dyn ChangeStore) {
    let first = s
        .open_or_resume_intent("i-1", &repo(), &digest(1), 0, &pids(0..3), "alice", 10)
        .unwrap();
    assert_eq!((first.attempts, first.state), (1, IntentState::Open));
    assert_eq!(first.pending_before, pids(0..3));
    // The retry sees a longer pending list (the killed run left a prefix) and a new intent id: neither is used.
    let again = s
        .open_or_resume_intent("i-2", &repo(), &digest(1), 0, &pids(0..9), "alice", 20)
        .unwrap();
    assert_eq!(again.intent_id, "i-1", "same digest, same open intent");
    assert_eq!(again.attempts, 2);
    assert_eq!(
        again.pending_before,
        pids(0..3),
        "pending_before is never overwritten"
    );
    assert_eq!(again.started_at, 10);
    assert_eq!(s.get_intent("i-2").unwrap(), None);
    let third = s
        .open_or_resume_intent("i-3", &repo(), &digest(1), 0, &pids(0..12), "alice", 30)
        .unwrap();
    assert_eq!((third.attempts, third.pending_before), (3, pids(0..3)));
    assert_eq!(s.get_intent("i-1").unwrap().unwrap().attempts, 3);
}

fn an_intent_for_another_artifact_is_a_separate_intent(s: &dyn ChangeStore) {
    // Two open intents in one repository are possible at this layer; that they never run at once is the W lease's
    // job (part 3b). The query exposes them so a caller can refuse or recover.
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &[], "alice", 10)
        .unwrap();
    s.open_or_resume_intent("i-2", &repo(), &digest(2), 0, &[], "bob", 11)
        .unwrap();
    s.open_or_resume_intent("i-x", &RepoId::new("other"), &digest(1), 0, &[], "bob", 12)
        .unwrap();
    let open: Vec<String> = s
        .open_intents(&repo())
        .unwrap()
        .into_iter()
        .map(|i| i.intent_id)
        .collect();
    assert_eq!(
        open,
        vec!["i-1", "i-2"],
        "oldest first, this repository only"
    );
}

fn an_abandoned_intent_does_not_block_a_fresh_one(s: &dyn ChangeStore) {
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &pids(0..2), "alice", 10)
        .unwrap();
    s.abandon_intent("i-1", 15).unwrap();
    let old = s.get_intent("i-1").unwrap().unwrap();
    assert_eq!(
        (old.state, old.completed_at),
        (IntentState::Abandoned, Some(15))
    );
    let fresh = s
        .open_or_resume_intent("i-2", &repo(), &digest(1), 0, &pids(0..5), "alice", 20)
        .unwrap();
    assert_eq!((fresh.intent_id.as_str(), fresh.attempts), ("i-2", 1));
    assert_eq!(
        fresh.pending_before,
        pids(0..5),
        "a new intent takes its own first-attempt list"
    );
}

/// The whole story of RFC 004 D-6 at the record level: open the intent, a prefix lands in `pending`, the process
/// dies (nothing is completed), the repository is flagged, the retry resumes, and exactly one revision results.
fn a_killed_accept_records_nothing_and_the_resumed_one_records_exactly_one(s: &dyn ChangeStore) {
    s.open_or_resume_intent("i-1", &repo(), &digest(7), 0, &pids(0..3), "alice", 10)
        .unwrap();
    // -- the accept is killed here; after the restart the forge flags the repository and finds an open intent --
    s.set_needs_recovery(&repo(), "accept did not finish", 12)
        .unwrap();
    assert_eq!(
        s.revision_by_digest(&repo(), &digest(7)).unwrap(),
        None,
        "a killed accept records nothing"
    );
    assert_eq!(s.get_change("c7").unwrap(), None);
    assert_eq!(s.open_intents(&repo()).unwrap().len(), 1);
    // -- the retry --
    let resumed = s
        .open_or_resume_intent("i-retry", &repo(), &digest(7), 0, &pids(0..8), "alice", 20)
        .unwrap();
    assert_eq!((resumed.intent_id.as_str(), resumed.attempts), ("i-1", 2));
    assert_eq!(
        resumed.pending_before,
        pids(0..3),
        "derivation starts from the first attempt's list"
    );
    let mut rev = revision("v7", "c7", 1, 7, 99, "alice");
    rev.added_patch_ids = pids(3..8);
    s.complete_intent_with_revision(
        &resumed.intent_id,
        Some(&change("c7", "alice", 20)),
        &rev,
        21,
    )
    .unwrap();
    s.clear_needs_recovery(&repo()).unwrap();
    assert_eq!(
        s.revisions_of("c7").unwrap().len(),
        1,
        "exactly one revision"
    );
    let done = s.get_intent("i-1").unwrap().unwrap();
    assert_eq!(
        (done.state, done.revision_id.as_deref(), done.completed_at),
        (IntentState::Complete, Some("v7"), Some(21))
    );
    assert!(s.open_intents(&repo()).unwrap().is_empty());
    assert!(!s.repo_flags(&repo()).unwrap().needs_recovery);
    // A repeat of the finished push finds the revision by its digest; the journal can be asked first.
    assert_eq!(
        s.revision_by_digest(&repo(), &digest(7))
            .unwrap()
            .unwrap()
            .revision_id,
        "v7"
    );
}

fn completion_is_atomic_when_a_failure_follows_the_revision_insert(s: &dyn ChangeStore) {
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &pids(0..2), "alice", 10)
        .unwrap();
    s.inject_failure(FailPoint::AfterRevisionInsert, true);
    let err = s
        .complete_intent_with_revision(
            "i-1",
            Some(&change("c1", "alice", 10)),
            &revision("v1", "c1", 1, 1, 10, "alice"),
            11,
        )
        .unwrap_err();
    assert!(matches!(err, StoreError::Backend(_)), "{err:?}");
    let i = s.get_intent("i-1").unwrap().unwrap();
    assert_eq!(
        (i.state, i.revision_id, i.completed_at),
        (IntentState::Open, None, None),
        "the intent is still open"
    );
    assert_eq!(
        s.revision_by_digest(&repo(), &digest(1)).unwrap(),
        None,
        "no revision"
    );
    assert!(s.revisions_of("c1").unwrap().is_empty());
    assert_eq!(
        s.get_change("c1").unwrap(),
        None,
        "the change was not created"
    );
    assert_eq!(s.usage(&repo(), "alice", NOW).unwrap().accepted_bytes, 0);
    s.inject_failure(FailPoint::AfterRevisionInsert, false);
    s.complete_intent_with_revision(
        "i-1",
        Some(&change("c1", "alice", 10)),
        &revision("v1", "c1", 1, 1, 10, "alice"),
        12,
    )
    .unwrap();
    assert_eq!(s.revisions_of("c1").unwrap().len(), 1);
    assert_eq!(
        s.get_intent("i-1").unwrap().unwrap().state,
        IntentState::Complete
    );
}

fn a_failed_completion_of_an_existing_change_leaves_it_untouched(s: &dyn ChangeStore) {
    let (cid, _) = push_new_change(s, 1, "alice", 10);
    let before = s.get_change(&cid).unwrap().unwrap();
    s.open_or_resume_intent("i-2", &repo(), &digest(2), 0, &[], "alice", 20)
        .unwrap();
    s.inject_failure(FailPoint::AfterRevisionInsert, true);
    assert!(
        s.complete_intent_with_revision("i-2", None, &revision("v-2", &cid, 2, 2, 5, "alice"), 21)
            .is_err()
    );
    s.inject_failure(FailPoint::AfterRevisionInsert, false);
    assert_eq!(
        s.get_change(&cid).unwrap().unwrap(),
        before,
        "updated_at did not move"
    );
    assert_eq!(s.revisions_of(&cid).unwrap().len(), 1);
    assert_eq!(
        s.get_intent("i-2").unwrap().unwrap().state,
        IntentState::Open
    );
}

fn completion_refuses_what_does_not_belong_together(s: &dyn ChangeStore) {
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &[], "alice", 10)
        .unwrap();
    let ok = revision("v1", "c1", 1, 1, 10, "alice");
    // unknown intent
    assert!(matches!(
        s.complete_intent_with_revision("nope", Some(&change("c1", "a", 1)), &ok, 11),
        Err(StoreError::NotFound)
    ));
    // a revision of another artifact than the intent's
    let other = revision("v1", "c1", 1, 2, 10, "alice");
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &other, 11),
        Err(StoreError::Invalid(_))
    ));
    // a revision of another repository
    let mut elsewhere = ok.clone();
    elsewhere.repo_id = RepoId::new("other");
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &elsewhere, 11),
        Err(StoreError::Invalid(_))
    ));
    // a new change that is not the revision's
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c-other", "a", 1)), &ok, 11),
        Err(StoreError::Invalid(_))
    ));
    // no change given and none exists
    assert!(matches!(
        s.complete_intent_with_revision("i-1", None, &ok, 11),
        Err(StoreError::NotFound)
    ));
    // seq 0
    let mut zero = ok.clone();
    zero.seq = 0;
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &zero, 11),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        s.get_intent("i-1").unwrap().unwrap().state,
        IntentState::Open
    );
    assert_eq!(s.get_change("c1").unwrap(), None);
    // once complete, it cannot complete again, nor be abandoned
    s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &ok, 12)
        .unwrap();
    let again = revision("v1b", "c1", 2, 1, 10, "alice");
    assert!(matches!(
        s.complete_intent_with_revision("i-1", None, &again, 13),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.abandon_intent("i-1", 14),
        Err(StoreError::Invalid(_))
    ));
}

fn malformed_ids_are_refused_and_nothing_is_written(s: &dyn ChangeStore) {
    for bad in [
        "",
        "abc",
        &"A".repeat(64),
        &"g".repeat(64),
        &"a".repeat(63),
        &"a".repeat(65),
    ] {
        assert!(
            matches!(
                s.open_or_resume_intent(
                    "i-bad",
                    &repo(),
                    &digest(1),
                    0,
                    &[bad.to_owned()],
                    "alice",
                    1
                ),
                Err(StoreError::Invalid(_))
            ),
            "pending_before {bad:?}"
        );
    }
    assert_eq!(s.get_intent("i-bad").unwrap(), None);
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &[], "alice", 1)
        .unwrap();
    let mut r = revision("v1", "c1", 1, 1, 1, "alice");
    r.added_patch_ids.push("not-hex".to_owned());
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &r, 2),
        Err(StoreError::Invalid(_))
    ));
    let mut r = revision("v1", "c1", 1, 1, 1, "alice");
    r.declared_patch_ids = Some(vec!["x".repeat(64)]);
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &r, 2),
        Err(StoreError::Invalid(_))
    ));
    let mut r = revision("v1", "c1", 1, 1, 1, "alice");
    r.claim_ids = vec!["Z".repeat(64)];
    assert!(matches!(
        s.complete_intent_with_revision("i-1", Some(&change("c1", "a", 1)), &r, 2),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(s.get_change("c1").unwrap(), None);
    assert_eq!(
        s.get_intent("i-1").unwrap().unwrap().state,
        IntentState::Open
    );
}

fn only_an_open_change_changes_state(s: &dyn ChangeStore) {
    for id in ["a", "b", "c", "d"] {
        s.create_change(&change(id, "alice", 1)).unwrap();
    }
    s.leave_open("a", ChangeState::Landed, None, 5, Some(100))
        .unwrap();
    let a = s.get_change("a").unwrap().unwrap();
    assert_eq!(
        (a.state, a.closed_reason, a.retain_until, a.updated_at),
        (ChangeState::Landed, None, Some(100), 5)
    );
    s.leave_open("b", ChangeState::Closed, Some("withdrawn"), 6, Some(200))
        .unwrap();
    let b = s.get_change("b").unwrap().unwrap();
    assert_eq!(
        (b.state, b.closed_reason.as_deref(), b.retain_until),
        (ChangeState::Closed, Some("withdrawn"), Some(200))
    );
    // illegal: out of a final state, back to open, closing without a reason, an unknown change
    assert!(matches!(
        s.leave_open("a", ChangeState::Closed, Some("x"), 7, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.leave_open("b", ChangeState::Landed, None, 7, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.leave_open("b", ChangeState::Open, None, 7, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.leave_open("c", ChangeState::Open, None, 7, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.leave_open("c", ChangeState::Closed, None, 7, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.leave_open("c", ChangeState::Closed, Some("  "), 7, None),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.leave_open("zz", ChangeState::Landed, None, 7, None),
        Err(StoreError::NotFound)
    ));
    assert_eq!(s.get_change("c").unwrap().unwrap().state, ChangeState::Open);
    assert_eq!(
        s.get_change("a").unwrap().unwrap().state,
        ChangeState::Landed,
        "a final state is final"
    );
    assert_eq!(
        s.get_change("b").unwrap().unwrap().state,
        ChangeState::Closed
    );
}

fn the_recovery_flag_is_per_repository(s: &dyn ChangeStore) {
    let other = RepoId::new("other");
    assert_eq!(s.repo_flags(&repo()).unwrap(), Default::default());
    s.set_needs_recovery(&repo(), "accept did not finish", 5)
        .unwrap();
    let f = s.repo_flags(&repo()).unwrap();
    assert_eq!(
        (f.needs_recovery, f.reason.as_deref(), f.since),
        (true, Some("accept did not finish"), Some(5))
    );
    assert!(!s.repo_flags(&other).unwrap().needs_recovery);
    s.set_needs_recovery(&repo(), "again", 9).unwrap();
    assert_eq!(
        s.repo_flags(&repo()).unwrap().reason.as_deref(),
        Some("again")
    );
    s.clear_needs_recovery(&repo()).unwrap();
    assert_eq!(s.repo_flags(&repo()).unwrap(), Default::default());
    s.clear_needs_recovery(&other).unwrap();
}

fn usage_is_computed_from_the_records_and_cannot_drift(s: &dyn ChangeStore) {
    let (c1, _) = push_new_change(s, 1, "alice", 100);
    let (_c2, _) = push_new_change(s, 2, "alice", 200);
    let (_c3, _) = push_new_change(s, 3, "bob", 400);
    let u = s.usage(&repo(), "alice", NOW).unwrap();
    assert_eq!(
        (u.open_changes, u.accepted_bytes, u.repo_retained_bytes),
        (2, 300, 700)
    );
    let u = s.usage(&repo(), "bob", NOW).unwrap();
    assert_eq!(
        (u.open_changes, u.accepted_bytes, u.repo_retained_bytes),
        (1, 400, 700)
    );
    assert_eq!(s.usage(&repo(), "carol", NOW).unwrap().open_changes, 0);
    assert_eq!(
        s.usage(&RepoId::new("other"), "alice", NOW).unwrap(),
        Default::default()
    );
    // A second revision adds to the bytes, not to the open changes.
    s.open_or_resume_intent("i-x", &repo(), &digest(9), 0, &[], "alice", 30)
        .unwrap();
    s.complete_intent_with_revision("i-x", None, &revision("v-x", &c1, 2, 9, 50, "alice"), 31)
        .unwrap();
    let u = s.usage(&repo(), "alice", NOW).unwrap();
    assert_eq!(
        (u.open_changes, u.accepted_bytes, u.repo_retained_bytes),
        (2, 350, 750)
    );
    // Leaving `open` frees an open-change slot, and an accepted artifact stays in the store (PK-41) so the accepted
    // bytes stay; the repository's *retained* bytes still count the closed change's artifacts until its grace
    // (`retain_until`, here 1000) has passed, because that is what occupies the disk.
    s.leave_open(&c1, ChangeState::Closed, Some("withdrawn"), 40, Some(1000))
        .unwrap();
    let u = s.usage(&repo(), "alice", NOW).unwrap();
    assert_eq!(
        (u.open_changes, u.accepted_bytes, u.repo_retained_bytes),
        (1, 350, 750),
        "within its grace, a closed change's artifacts are still retained"
    );
    let u = s.usage(&repo(), "alice", 1000).unwrap();
    assert_eq!(
        u.repo_retained_bytes, 750,
        "the grace includes its last second"
    );
    let u = s.usage(&repo(), "alice", 1001).unwrap();
    assert_eq!(
        (u.open_changes, u.accepted_bytes, u.repo_retained_bytes),
        (1, 350, 600),
        "past its grace the artifacts of the closed change no longer count"
    );
    // A change that left `open` without any retention date retains nothing.
    s.leave_open("c2", ChangeState::Landed, None, 41, None)
        .unwrap();
    assert_eq!(
        s.usage(&repo(), "alice", NOW).unwrap().repo_retained_bytes,
        750 - 200
    );
}

fn changes_are_listed_per_repository_and_state_newest_first(s: &dyn ChangeStore) {
    s.create_change(&change("old", "a", 1)).unwrap();
    s.create_change(&change("new", "a", 3)).unwrap();
    s.create_change(&change("mid", "a", 2)).unwrap();
    let mut elsewhere = change("else", "a", 9);
    elsewhere.repo_id = RepoId::new("other");
    s.create_change(&elsewhere).unwrap();
    s.leave_open("mid", ChangeState::Landed, None, 5, None)
        .unwrap();
    let ids = |v: Vec<ChangeRecord>| v.into_iter().map(|c| c.change_id).collect::<Vec<_>>();
    assert_eq!(
        ids(s.list_changes(&repo(), None).unwrap()),
        vec!["new", "mid", "old"]
    );
    assert_eq!(
        ids(s.list_changes(&repo(), Some(ChangeState::Open)).unwrap()),
        vec!["new", "old"]
    );
    assert_eq!(
        ids(s.list_changes(&repo(), Some(ChangeState::Landed)).unwrap()),
        vec!["mid"]
    );
    assert!(
        s.list_changes(&repo(), Some(ChangeState::Closed))
            .unwrap()
            .is_empty()
    );
}

fn an_intent_can_only_be_abandoned_while_open(s: &dyn ChangeStore) {
    assert!(matches!(
        s.abandon_intent("nope", 1),
        Err(StoreError::NotFound)
    ));
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &[], "alice", 1)
        .unwrap();
    s.abandon_intent("i-1", 2).unwrap();
    assert!(matches!(
        s.abandon_intent("i-1", 3),
        Err(StoreError::Invalid(_))
    ));
}

/// IS-50: an accepted-but-unrecorded artifact is a debt. Its patches are in the repository and cannot be removed, so
/// while its intent is open (and has no revision) its bytes count against the submitter's accepted bytes and the
/// repository's retained bytes; a revision or an abandonment replaces the debt, and the bytes are counted **once**.
fn an_open_intent_owes_its_bytes_until_a_revision_or_an_abandonment_replaces_it(
    s: &dyn ChangeStore,
) {
    s.open_or_resume_intent("i-1", &repo(), &digest(1), 100, &[], "alice", 10)
        .unwrap();
    s.open_or_resume_intent("i-2", &repo(), &digest(2), 40, &[], "bob", 10)
        .unwrap();
    let a = s.usage(&repo(), "alice", NOW).unwrap();
    let b = s.usage(&repo(), "bob", NOW).unwrap();
    assert_eq!(
        (a.accepted_bytes, a.repo_retained_bytes, a.open_changes),
        (100, 140, 0)
    );
    assert_eq!(
        (b.accepted_bytes, b.repo_retained_bytes),
        (40, 140),
        "each pays for their own; the repository for both"
    );
    // A resume does not owe it twice.
    s.open_or_resume_intent("i-1b", &repo(), &digest(1), 100, &[], "alice", 11)
        .unwrap();
    assert_eq!(s.usage(&repo(), "alice", NOW).unwrap().accepted_bytes, 100);
    // The corrected re-send records its revision: the bytes are counted once, now as the revision's.
    s.complete_intent_with_revision(
        "i-1",
        Some(&change("c1", "alice", 12)),
        &revision("v1", "c1", 1, 1, 100, "alice"),
        12,
    )
    .unwrap();
    let a = s.usage(&repo(), "alice", NOW).unwrap();
    assert_eq!(
        (a.accepted_bytes, a.repo_retained_bytes, a.open_changes),
        (100, 140, 1)
    );
    // An abandonment clears the debt (the artifact leaves the retained set; the patches stay in the repository).
    s.abandon_intent("i-2", 13).unwrap();
    let b = s.usage(&repo(), "bob", NOW).unwrap();
    assert_eq!((b.accepted_bytes, b.repo_retained_bytes), (0, 100));
    // Another repository is not charged.
    assert_eq!(
        s.usage(&RepoId::new("other"), "alice", NOW).unwrap(),
        Default::default()
    );
}

fn flagged_repositories_are_listed(s: &dyn ChangeStore) {
    assert!(s.flagged_repos().unwrap().is_empty());
    s.set_needs_recovery(&RepoId::new("b"), "x", 1).unwrap();
    s.set_needs_recovery(&RepoId::new("a"), "y", 1).unwrap();
    s.set_needs_recovery(&RepoId::new("c"), "z", 1).unwrap();
    s.clear_needs_recovery(&RepoId::new("c")).unwrap();
    let mut flagged: Vec<String> = s
        .flagged_repos()
        .unwrap()
        .into_iter()
        .map(|r| r.as_str().to_owned())
        .collect();
    flagged.sort();
    assert_eq!(flagged, vec!["a", "b"]);
}

// ------------------------------------------------------------------------------------------------ id packing

#[test]
fn packed_ids_round_trip_and_a_malformed_one_is_refused() {
    for n in [0u64, 1, 255, 256, u64::MAX] {
        let ids = vec![
            pid(n),
            format!("{:x}", n)
                .repeat(1)
                .chars()
                .cycle()
                .take(64)
                .collect(),
        ];
        let blob = pack_ids(&ids).unwrap();
        assert_eq!(blob.len(), 64);
        assert_eq!(unpack_ids(&blob).unwrap(), ids);
    }
    let all: Vec<String> = (0..300)
        .map(|n| format!("{:064x}", (n as u128) * 0x1_0000_0001_0000_0001))
        .collect();
    assert_eq!(
        unpack_ids(&pack_ids(&all).unwrap()).unwrap(),
        all,
        "order is kept"
    );
    assert_eq!(pack_ids(&[]).unwrap(), Vec::<u8>::new());
    assert!(unpack_ids(&[0u8; 31]).is_err());
    assert!(
        pack_ids(&["A".repeat(64)]).is_err(),
        "upper case is not an id"
    );
}

// ------------------------------------------------------------------------------------------------ SQLite only

/// The `intents` table as part 3a created it (no `artifact_bytes`).
const INTENTS_3A: &str = "
CREATE TABLE intents (
    intent_id TEXT PRIMARY KEY, repo_id TEXT NOT NULL, artifact_digest TEXT NOT NULL, pending_before BLOB NOT NULL,
    principal TEXT NOT NULL, started_at INTEGER NOT NULL, attempts INTEGER NOT NULL CHECK (attempts >= 1),
    state TEXT NOT NULL CHECK (state IN ('open', 'complete', 'abandoned')), completed_at INTEGER,
    revision_id TEXT
);
INSERT INTO intents VALUES ('old-1','r1','sha256:old',x'','alice',1,1,'open',NULL,NULL);
";

#[test]
fn a_part_3a_database_gains_the_intent_size_and_three_reopens_change_nothing() {
    let (dir, path) = temp_db("migrate-intents");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(INTENTS_3A).unwrap();
    }
    for round in 0..3 {
        let db = SqliteDb::open(&path).unwrap();
        let s = SqliteChangeStore::new(db.clone()).unwrap();
        let old = s.get_intent("old-1").unwrap().unwrap();
        assert_eq!(
            old.artifact_bytes, 0,
            "round {round}: a row made before the column reads as 0"
        );
        assert_eq!(
            s.usage(&repo(), "alice", NOW).unwrap().accepted_bytes,
            if round == 0 { 0 } else { 70 },
            "round {round}"
        );
        if round == 0 {
            s.open_or_resume_intent("new-1", &repo(), &digest(5), 70, &[], "alice", 2)
                .unwrap();
        }
        let cols: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('intents') WHERE name = 'artifact_bytes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cols, 1, "round {round}: the column exists once");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn temp_db(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("planeter-changes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.db");
    (dir, path)
}

/// Names and SQL of everything the change tables created, for the review request and the constraint tests.
fn schema_of(db: &SqliteDb) -> Vec<(String, String, String)> {
    let conn = db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT type, name, COALESCE(sql, '') FROM sqlite_master WHERE name IN ('changes','revisions','intents','repo_flags') OR tbl_name IN ('changes','revisions','intents','repo_flags') ORDER BY type DESC, name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn the_schema_has_the_tables_the_constraints_and_the_indexes() {
    let db = SqliteDb::open_in_memory().unwrap();
    SqliteChangeStore::new(db.clone()).unwrap();
    let schema = schema_of(&db);
    let names: Vec<&str> = schema.iter().map(|(_, n, _)| n.as_str()).collect();
    for t in ["changes", "revisions", "intents", "repo_flags"] {
        assert!(names.contains(&t), "table {t}");
    }
    for i in [
        "changes_by_repo_state",
        "changes_open_by_principal",
        "revisions_by_submitter",
        "intents_one_open_per_artifact",
    ] {
        assert!(names.contains(&i), "index {i}");
    }
    let sql: String = schema
        .iter()
        .map(|(_, _, s)| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "CHECK (state IN ('open', 'landed', 'closed'))",
        "CHECK (state IN ('open', 'complete', 'abandoned'))",
        "UNIQUE (change_id, seq)",
        "UNIQUE (repo_id, artifact_digest)",
        "WHERE state = 'open'",
    ] {
        assert!(sql.contains(needle), "{needle}");
    }
    if let Ok(out) = std::env::var("PLANETER_PRINT_SCHEMA") {
        let text: String = schema
            .iter()
            .map(|(t, n, s)| format!("-- {t} {n}\n{s};\n"))
            .collect();
        std::fs::write(out, text).unwrap();
    }
}

#[test]
fn the_database_itself_refuses_an_unknown_state_and_a_second_open_intent() {
    let db = SqliteDb::open_in_memory().unwrap();
    SqliteChangeStore::new(db.clone()).unwrap();
    let conn = db.lock().unwrap();
    let bad_change = conn.execute(
        "INSERT INTO changes (change_id, repo_id, title, description, state, opened_by, opened_at, updated_at) VALUES ('c','r','t','d','merged','a',1,1)",
        [],
    );
    assert!(
        bad_change.is_err(),
        "the CHECK refuses a state that is not one of the three"
    );
    let intent = |id: &str, state: &str| {
        conn.execute(
            "INSERT INTO intents (intent_id, repo_id, artifact_digest, pending_before, principal, started_at, attempts, state) VALUES (?1,'r','d',x'',?3,1,1,?2)",
            rusqlite::params![id, state, "p"],
        )
    };
    assert!(intent("i1", "open").is_ok());
    assert!(
        intent("i2", "open").is_err(),
        "one open intent per artifact, in the database itself"
    );
    assert!(
        intent("i3", "complete").is_ok(),
        "a finished intent for the same artifact is allowed"
    );
    assert!(
        intent("i4", "paused").is_err(),
        "the CHECK refuses an intent state that is not one of the three"
    );
    let seq0 = conn.execute("INSERT INTO changes (change_id, repo_id, title, description, state, opened_by, opened_at, updated_at) VALUES ('c1','r','t','d','open','a',1,1)", []).and_then(|_| conn.execute(
        "INSERT INTO revisions (revision_id, change_id, repo_id, seq, artifact_digest, artifact_bytes, claim_ids, added_patch_ids, claim_outcomes, submitter, author_key_ids, created_at) VALUES ('v','c1','r',0,'d',1,x'',x'','[]','a','[]',1)",
        [],
    ));
    assert!(seq0.is_err(), "seq starts at 1");
    // The two uniqueness rules, in the database itself (the store checks first to give a typed error, but the
    // constraint is what holds if a second writer ever appears).
    let rev = |id: &str, seq: i64, digest: &str| {
        conn.execute(
            "INSERT INTO revisions (revision_id, change_id, repo_id, seq, artifact_digest, artifact_bytes, claim_ids, added_patch_ids, claim_outcomes, submitter, author_key_ids, created_at) VALUES (?1,'c1','r',?2,?3,1,x'',x'','[]','a','[]',1)",
            rusqlite::params![id, seq, digest],
        )
    };
    assert!(rev("v1", 1, "d1").is_ok());
    assert!(rev("v2", 1, "d2").is_err(), "UNIQUE (change_id, seq)");
    assert!(
        rev("v3", 2, "d1").is_err(),
        "UNIQUE (repo_id, artifact_digest)"
    );
    assert!(rev("v4", 2, "d2").is_ok());
    let orphan = conn.execute("INSERT INTO revisions (revision_id, change_id, repo_id, seq, artifact_digest, artifact_bytes, claim_ids, added_patch_ids, claim_outcomes, submitter, author_key_ids, created_at) VALUES ('v5','no-such-change','r',1,'d9',1,x'',x'','[]','a','[]',1)", []);
    assert!(
        orphan.is_err(),
        "a revision belongs to a change (foreign key)"
    );
}

/// The tables `0.4.0` created (its `repositories` has no `proposals` column yet).
const DDL_0_4_0: &str = "
CREATE TABLE repositories (
    repo_id TEXT PRIMARY KEY, owner_kind TEXT NOT NULL, owner TEXT NOT NULL, name TEXT NOT NULL,
    visibility TEXT NOT NULL, created_at INTEGER NOT NULL, prikk_format_version TEXT, path TEXT NOT NULL,
    UNIQUE (owner_kind, owner, name)
);
INSERT INTO repositories VALUES ('r1','user','alice','demo','public',1,NULL,'/srv/r1');
";

#[test]
fn a_0_4_0_database_gains_the_change_tables_and_three_reopens_change_nothing() {
    let (dir, path) = temp_db("migrate");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(DDL_0_4_0).unwrap();
    }
    let mut seen = Vec::new();
    for round in 0..3 {
        let db = SqliteDb::open(&path).unwrap();
        let s = SqliteChangeStore::new(db.clone()).unwrap();
        if round == 0 {
            push_new_change(&s, 1, "alice", 10);
            s.set_needs_recovery(&repo(), "x", 1).unwrap();
        }
        // The old data survived; the new data survived; nothing was added twice.
        let repos: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM repositories WHERE proposals = 'members'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            repos, 1,
            "round {round}: the 0.4.0 row is there and has the new column's default"
        );
        assert_eq!(s.revisions_of("c1").unwrap().len(), 1, "round {round}");
        assert!(
            s.repo_flags(&repo()).unwrap().needs_recovery,
            "round {round}"
        );
        seen.push(schema_of(&db));
    }
    assert_eq!(seen[0], seen[1]);
    assert_eq!(
        seen[1], seen[2],
        "the schema after the third open is the schema after the first"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_intent_survives_a_restart_and_the_retry_resumes_it() {
    let (dir, path) = temp_db("restart");
    {
        let s = SqliteChangeStore::new(SqliteDb::open(&path).unwrap()).unwrap();
        s.open_or_resume_intent("i-1", &repo(), &digest(1), 0, &pids(0..40), "alice", 10)
            .unwrap();
        s.set_needs_recovery(&repo(), "accept did not finish", 11)
            .unwrap();
        // The process dies here: the handle is dropped with nothing completed.
    }
    let s = SqliteChangeStore::new(SqliteDb::open(&path).unwrap()).unwrap();
    assert!(s.repo_flags(&repo()).unwrap().needs_recovery);
    let open = s.open_intents(&repo()).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].pending_before, pids(0..40));
    let r = s
        .open_or_resume_intent("i-2", &repo(), &digest(1), 0, &pids(0..60), "alice", 20)
        .unwrap();
    assert_eq!(
        (r.intent_id.as_str(), r.attempts, r.pending_before.len()),
        ("i-1", 2, 40)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_pushes_on_the_shared_connection_each_get_exactly_one_revision() {
    let db = SqliteDb::open_in_memory().unwrap();
    let s = SqliteChangeStore::new(db).unwrap();
    std::thread::scope(|sc| {
        for t in 0..8u64 {
            let s = &s;
            sc.spawn(move || {
                for k in 0..10u64 {
                    let n = t * 100 + k;
                    push_new_change(s, n, &format!("user{t}"), 10);
                }
            });
        }
    });
    let all = s.list_changes(&repo(), None).unwrap();
    assert_eq!(all.len(), 80);
    for t in 0..8u64 {
        let u = s.usage(&repo(), &format!("user{t}"), NOW).unwrap();
        assert_eq!((u.open_changes, u.accepted_bytes), (10, 100));
    }
    // Racing resumes of one intent: every call counts as an attempt, none overwrites the first list.
    s.open_or_resume_intent(
        "i-race",
        &repo(),
        &digest(99_999),
        0,
        &pids(0..5),
        "alice",
        1,
    )
    .unwrap();
    std::thread::scope(|sc| {
        for _ in 0..8 {
            sc.spawn(|| {
                for _ in 0..10 {
                    let r = s
                        .open_or_resume_intent(
                            "i-other",
                            &repo(),
                            &digest(99_999),
                            0,
                            &pids(0..9),
                            "alice",
                            2,
                        )
                        .unwrap();
                    assert_eq!(r.intent_id, "i-race");
                    assert_eq!(r.pending_before.len(), 5);
                }
            });
        }
    });
    assert_eq!(s.get_intent("i-race").unwrap().unwrap().attempts, 81);
}

#[test]
fn an_intent_at_the_object_bound_is_640_kilobytes_of_ids_and_survives_a_round_trip() {
    // RFC 004 D-5 bounds an accept at 20,000 objects: the largest `pending_before` an intent can hold.
    let ids: Vec<String> = (0..20_000u64)
        .map(|n| format!("{:064x}", n.wrapping_mul(0x9E37_79B9_7F4A_7C15)))
        .collect();
    let (dir, path) = temp_db("size");
    let db = SqliteDb::open(&path).unwrap();
    let s = SqliteChangeStore::new(db.clone()).unwrap();
    let t = std::time::Instant::now();
    s.open_or_resume_intent("i-big", &repo(), &digest(1), 0, &ids, "alice", 1)
        .unwrap();
    let write = t.elapsed();
    let t = std::time::Instant::now();
    let back = s.get_intent("i-big").unwrap().unwrap();
    let read = t.elapsed();
    assert_eq!(back.pending_before, ids);
    let (blob_len, row_len): (i64, i64) = db
        .lock()
        .unwrap()
        .query_row("SELECT length(pending_before), length(pending_before) + length(intent_id) + length(repo_id) + length(artifact_digest) + length(principal) FROM intents", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(blob_len, 20_000 * 32);
    eprintln!(
        "intent at 20,000 ids: blob {blob_len} bytes, row about {row_len} bytes; write {write:?}, read {read:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
