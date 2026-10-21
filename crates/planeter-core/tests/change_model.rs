//! RFC 005 "Model tests (pure)": revision derivation, the killed-and-resumed accept, idempotence, declared sets,
//! approvals not carried, staleness, landing from a recorded pending sequence, quotas, and a pending patch never
//! rendered as sealed. No process, no store, no clock: the model takes its inputs as arguments.

use std::collections::BTreeSet;

use planeter_core::change::{
    ApprovalStanding, BaseCheck, DEFAULT_RETENTION_GRACE_SECS, Declared, DeriveError, Landing,
    PatchFacts, Quota, QuotaPolicy, QuotaRefusal, QuotaRequest, QuotaTarget, QuotaUsage, Staleness,
    Transition, TransitionError, already_recorded, approval_standing, check_base, check_quota,
    current_patch_set, derive_revision, observe_landing, patch_assurance, staleness, transition,
};
use planeter_core::read::Assurance;
use planeter_store::{
    ChangeState, ChangeStore, InMemoryChangeStore, IntentRecord, IntentState, ProposalPolicy,
    RepoId, RevisionRecord,
};

fn pid(n: u64) -> String {
    format!("{n:064x}")
}
fn set(r: std::ops::Range<u64>) -> BTreeSet<String> {
    r.map(pid).collect()
}
fn list(r: std::ops::Range<u64>) -> Vec<String> {
    r.map(pid).collect()
}

fn intent(before: std::ops::Range<u64>, state: IntentState) -> IntentRecord {
    IntentRecord {
        intent_id: "i1".to_owned(),
        repo_id: RepoId::new("r1"),
        artifact_digest: "sha256:aa".to_owned(),
        pending_before: list(before),
        principal: "alice".to_owned(),
        started_at: 1,
        attempts: 1,
        state,
        completed_at: None,
        revision_id: None,
    }
}

fn open(before: std::ops::Range<u64>) -> IntentRecord {
    intent(before, IntentState::Open)
}

fn all_held(_: &str) -> bool {
    true
}

// ------------------------------------------------------------------------------------------------ derivation

#[test]
fn a_revision_is_what_pending_gained_since_the_first_attempt() {
    let got = derive_revision(&open(0..3), &set(0..8), &all_held, None).unwrap();
    assert_eq!(got.added_patch_ids, list(3..8));
    assert_eq!(
        got.declared,
        Declared::None,
        "no declaration is recorded as none, not as empty"
    );
    assert_eq!(got.artifact_digest, "sha256:aa");
}

#[test]
fn patches_that_left_pending_meanwhile_are_not_added_and_not_missed() {
    // Pending before: 0..5. Meanwhile 0 and 1 were sealed elsewhere (so left pending); the accept added 5..7.
    let after: BTreeSet<String> = (2..7).map(pid).collect();
    let got = derive_revision(&open(0..5), &after, &all_held, None).unwrap();
    assert_eq!(got.added_patch_ids, list(5..7));
}

#[test]
fn an_accept_that_adds_nothing_records_nothing() {
    assert_eq!(
        derive_revision(&open(0..4), &set(0..4), &all_held, None),
        Err(DeriveError::NoNewContent)
    );
    // Even a declaration cannot make something of nothing.
    let declared = list(0..4);
    assert_eq!(
        derive_revision(&open(0..4), &set(0..4), &all_held, Some(&declared)),
        Err(DeriveError::NoNewContent)
    );
    // Pending shrank: still nothing added.
    assert_eq!(
        derive_revision(&open(0..4), &set(0..2), &all_held, None),
        Err(DeriveError::NoNewContent)
    );
}

#[test]
fn a_diff_is_never_derived_from_an_intent_that_is_not_open() {
    for state in [IntentState::Complete, IntentState::Abandoned] {
        let got = derive_revision(&intent(0..3, state), &set(0..9), &all_held, None);
        assert_eq!(got, Err(DeriveError::IntentNotOpen(state)), "{state:?}");
    }
    let mut done = intent(0..3, IntentState::Complete);
    done.revision_id = Some("v1".to_owned());
    assert_eq!(
        derive_revision(&done, &set(0..9), &all_held, None),
        Err(DeriveError::AlreadyRecorded("v1".to_owned())),
        "a finished intent that has its revision answers with that revision"
    );
}

#[test]
fn derivation_is_a_function_of_the_intent_alone_not_of_the_attempt() {
    // The record holds the FIRST attempt's list. Whatever the retry saw, the same record derives the same thing.
    let rec = open(0..3);
    let a = derive_revision(&rec, &set(0..8), &all_held, None).unwrap();
    let b = derive_revision(&rec, &set(0..8), &all_held, None).unwrap();
    assert_eq!(a, b);
    // And a retry that took a later, longer list as "before" would lose the prefix: that is what the journal prevents.
    let wrong = open(0..6);
    let c = derive_revision(&wrong, &set(0..8), &all_held, None).unwrap();
    assert_ne!(a.added_patch_ids, c.added_patch_ids);
}

#[test]
fn a_killed_accept_records_nothing_and_the_resumed_one_records_exactly_one_from_the_first_attempts_list()
 {
    let store = InMemoryChangeStore::new();
    let repo = RepoId::new("r1");
    // 1. the intent is journaled with the pending set as it was; 2. the accept dies after storing a prefix
    let first = store
        .open_or_resume_intent("i1", &repo, "sha256:aa", &list(0..3), "alice", 10)
        .unwrap();
    let pending_after_kill = set(0..6); // 3 of 5 patches reached pending
    assert!(
        store
            .revision_by_digest(&repo, "sha256:aa")
            .unwrap()
            .is_none(),
        "nothing recorded by the killed run"
    );
    // 3. restart; the retry resumes the SAME intent (its list is still the first attempt's)
    let resumed = store
        .open_or_resume_intent(
            "i2",
            &repo,
            "sha256:aa",
            &pending_after_kill.iter().cloned().collect::<Vec<_>>(),
            "alice",
            20,
        )
        .unwrap();
    assert_eq!(resumed.intent_id, first.intent_id);
    // 4. the retry completes: pending now holds all five new patches
    let derived = derive_revision(&resumed, &set(0..8), &all_held, None).unwrap();
    assert_eq!(
        derived.added_patch_ids,
        list(3..8),
        "the prefix stored by the killed run is part of the revision"
    );
    // 5. recorded once
    let rev = RevisionRecord {
        revision_id: "v1".to_owned(),
        change_id: "c1".to_owned(),
        repo_id: repo.clone(),
        seq: 1,
        artifact_digest: derived.artifact_digest.clone(),
        artifact_bytes: 10,
        declared_base: None,
        claim_ids: vec![],
        added_patch_ids: derived.added_patch_ids.clone(),
        declared_patch_ids: derived.declared.clone().into_stored(),
        claim_outcomes: vec![],
        submitter: "alice".to_owned(),
        author_key_ids: vec![],
        created_at: 21,
    };
    let change = planeter_store::ChangeRecord {
        change_id: "c1".to_owned(),
        repo_id: repo.clone(),
        title: "t".to_owned(),
        description: String::new(),
        target_ref: None,
        state: ChangeState::Open,
        closed_reason: None,
        opened_by: "alice".to_owned(),
        opened_at: 21,
        updated_at: 21,
        retain_until: None,
    };
    store
        .complete_intent_with_revision(&resumed.intent_id, Some(&change), &rev, 21)
        .unwrap();
    assert_eq!(store.revisions_of("c1").unwrap().len(), 1);
    // 6. a repeat of the same artifact is the same revision; the intent is complete, so nothing derives from it
    let existing = store.revision_by_digest(&repo, "sha256:aa").unwrap();
    assert_eq!(
        already_recorded(existing.as_ref()),
        Err(DeriveError::AlreadyRecorded("v1".to_owned()))
    );
    let done = store.get_intent(&resumed.intent_id).unwrap().unwrap();
    assert_eq!(
        derive_revision(&done, &set(0..8), &all_held, None),
        Err(DeriveError::AlreadyRecorded("v1".to_owned()))
    );
}

#[test]
fn an_unseen_digest_is_not_already_recorded() {
    assert_eq!(already_recorded(None), Ok(()));
}

// ------------------------------------------------------------------------------------------------ declarations

#[test]
fn a_declaration_that_covers_what_was_added_and_names_only_held_patches_is_accepted() {
    // The artifact carries 0..2 (already pending before: an amended series) plus the newly added 3..5.
    let declared: Vec<String> = [0, 1, 3, 4].into_iter().map(pid).collect();
    let held = |id: &str| set(0..5).contains(id);
    let got = derive_revision(&open(0..3), &set(0..5), &held, Some(&declared)).unwrap();
    assert_eq!(got.added_patch_ids, list(3..5));
    assert_eq!(got.declared, Declared::Ids(declared));
}

#[test]
fn a_declaration_is_stored_sorted_and_without_repeats() {
    let declared: Vec<String> = [4, 3, 3, 4].into_iter().map(pid).collect();
    let got = derive_revision(&open(0..3), &set(0..5), &all_held, Some(&declared)).unwrap();
    assert_eq!(got.declared, Declared::Ids(list(3..5)));
}

#[test]
fn a_declaration_that_leaves_out_an_added_patch_is_refused_and_names_it() {
    let declared = list(3..4); // misses 4
    let err = derive_revision(&open(0..3), &set(0..5), &all_held, Some(&declared)).unwrap_err();
    assert_eq!(
        err,
        DeriveError::DeclarationMissesAdded {
            missing: vec![pid(4)]
        }
    );
    // An empty declaration misses everything.
    let err = derive_revision(&open(0..3), &set(0..5), &all_held, Some(&[])).unwrap_err();
    assert_eq!(
        err,
        DeriveError::DeclarationMissesAdded {
            missing: list(3..5)
        }
    );
}

#[test]
fn a_declaration_that_names_a_patch_the_forge_does_not_hold_is_refused_and_names_it() {
    let declared: Vec<String> = [3, 4, 77].into_iter().map(pid).collect();
    let held = |id: &str| id != pid(77);
    let err = derive_revision(&open(0..3), &set(0..5), &held, Some(&declared)).unwrap_err();
    assert_eq!(err, DeriveError::DeclaredNotHeld { ids: vec![pid(77)] });
}

#[test]
fn what_the_declaration_misses_is_reported_before_what_is_not_held() {
    let declared: Vec<String> = vec![pid(3), pid(77)]; // misses 4, and 77 is not held
    let held = |id: &str| id != pid(77);
    let err = derive_revision(&open(0..3), &set(0..5), &held, Some(&declared)).unwrap_err();
    assert!(
        matches!(err, DeriveError::DeclarationMissesAdded { .. }),
        "{err:?}"
    );
}

/// Universal rule, checked exhaustively on a small universe: for every added set and every declaration of the
/// universe, derivation succeeds exactly when `declared ⊇ added` and every declared id is held.
#[test]
fn declared_not_a_superset_of_added_is_always_refused() {
    let universe: Vec<String> = list(0..5);
    let before = open(0..2); // ids 2,3,4 can be added
    for after_mask in 0..(1u32 << 5) {
        let after: BTreeSet<String> = (0..5)
            .filter(|i| after_mask >> i & 1 == 1)
            .map(pid)
            .collect();
        let added: BTreeSet<String> = after
            .iter()
            .filter(|id| !before.pending_before.contains(id))
            .cloned()
            .collect();
        for decl_mask in 0..(1u32 << 5) {
            let declared: Vec<String> = (0..5)
                .filter(|i| decl_mask >> i & 1 == 1)
                .map(|i| universe[i as usize].clone())
                .collect();
            for held_mask in [0b11111u32, 0b01111, 0b00111] {
                let held =
                    move |id: &str| universe_index(id).is_some_and(|i| held_mask >> i & 1 == 1);
                let got = derive_revision(&before, &after, &held, Some(&declared));
                let declared_set: BTreeSet<String> = declared.iter().cloned().collect();
                let covers = added.is_subset(&declared_set);
                let all_held = declared_set.iter().all(|d| held(d));
                match (added.is_empty(), covers, all_held) {
                    (true, _, _) => assert_eq!(got, Err(DeriveError::NoNewContent)),
                    (false, true, true) => assert!(
                        got.is_ok(),
                        "after {after_mask:05b} declared {decl_mask:05b} held {held_mask:05b}"
                    ),
                    (false, false, _) => assert!(matches!(
                        got,
                        Err(DeriveError::DeclarationMissesAdded { .. })
                    )),
                    (false, true, false) => {
                        assert!(matches!(got, Err(DeriveError::DeclaredNotHeld { .. })))
                    }
                }
            }
        }
    }
}

fn universe_index(id: &str) -> Option<u32> {
    (0..5u32).find(|i| pid(u64::from(*i)) == id)
}

#[test]
fn nothing_derives_from_a_non_open_intent_whatever_the_sets_are() {
    for state in [IntentState::Complete, IntentState::Abandoned] {
        for before_n in 0..4u64 {
            for after_n in 0..6u64 {
                let rec = intent(0..before_n, state);
                assert!(derive_revision(&rec, &set(0..after_n), &all_held, None).is_err());
                assert!(
                    derive_revision(&rec, &set(0..after_n), &all_held, Some(&list(0..after_n)))
                        .is_err()
                );
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ base, staleness, approvals

#[test]
fn a_base_is_held_only_if_a_log_of_a_held_ref_lists_it() {
    let listed: BTreeSet<String> = ["b1", "b2"].iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        check_base(Some("b1"), &listed),
        BaseCheck::Held("b1".to_owned())
    );
    assert_eq!(
        check_base(Some("zz"), &listed),
        BaseCheck::Unknown("zz".to_owned())
    );
    assert_eq!(
        check_base(None, &listed),
        BaseCheck::NoBase,
        "no base: reviewable, results never count"
    );
    assert_eq!(
        check_base(Some("b1"), &BTreeSet::new()),
        BaseCheck::Unknown("b1".to_owned())
    );
}

#[test]
fn a_result_given_against_a_base_is_stale_once_the_tip_moves() {
    assert_eq!(staleness(Some("b1"), Some("b1")), Staleness::Current);
    assert_eq!(staleness(Some("b1"), Some("b2")), Staleness::Stale);
    assert_eq!(
        staleness(Some("b1"), None),
        Staleness::Stale,
        "no known tip: not claimed current"
    );
    assert_eq!(staleness(None, Some("b1")), Staleness::NoBase);
    assert_eq!(staleness(None, None), Staleness::NoBase);
}

#[test]
fn an_approval_is_not_carried_to_a_later_revision() {
    assert_eq!(approval_standing(2, 2), ApprovalStanding::Current);
    assert_eq!(
        approval_standing(1, 2),
        ApprovalStanding::OnEarlierRevision(1)
    );
    assert_eq!(
        approval_standing(2, 5),
        ApprovalStanding::OnEarlierRevision(2)
    );
}

// ------------------------------------------------------------------------------------------------ states

#[test]
fn an_open_change_lands_or_closes_and_gets_its_retention_from_the_grace() {
    let landed = transition(
        ChangeState::Open,
        &Transition::Landed,
        1_000,
        DEFAULT_RETENTION_GRACE_SECS,
    )
    .unwrap();
    assert_eq!(
        (landed.state, landed.closed_reason.as_deref()),
        (ChangeState::Landed, None)
    );
    assert_eq!(
        landed.retain_until,
        1_000 + 30 * 24 * 3600,
        "thirty days by default"
    );
    let closed = transition(
        ChangeState::Open,
        &Transition::Closed {
            reason: "withdrawn".to_owned(),
        },
        1_000,
        60,
    )
    .unwrap();
    assert_eq!(
        (
            closed.state,
            closed.closed_reason.as_deref(),
            closed.retain_until
        ),
        (ChangeState::Closed, Some("withdrawn"), 1_060)
    );
    assert_eq!(
        transition(ChangeState::Open, &Transition::Landed, u64::MAX, 5)
            .unwrap()
            .retain_until,
        u64::MAX,
        "no overflow"
    );
}

#[test]
fn closing_needs_a_reason() {
    for reason in ["", "   ", "\n\t"] {
        assert_eq!(
            transition(
                ChangeState::Open,
                &Transition::Closed {
                    reason: reason.to_owned()
                },
                1,
                1
            ),
            Err(TransitionError::ReasonRequired)
        );
    }
}

#[test]
fn landed_and_closed_never_change_again() {
    for from in [ChangeState::Landed, ChangeState::Closed] {
        for to in [
            Transition::Landed,
            Transition::Closed {
                reason: "x".to_owned(),
            },
        ] {
            assert_eq!(
                transition(from, &to, 1, 1),
                Err(TransitionError::AlreadyFinal(from)),
                "{from:?} -> {to:?}"
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------ landing

fn rev(
    seq: u32,
    added: std::ops::Range<u64>,
    declared: Option<std::ops::Range<u64>>,
) -> RevisionRecord {
    RevisionRecord {
        revision_id: format!("v{seq}"),
        change_id: "c1".to_owned(),
        repo_id: RepoId::new("r1"),
        seq,
        artifact_digest: format!("sha256:{seq}"),
        artifact_bytes: 1,
        declared_base: None,
        claim_ids: vec![],
        added_patch_ids: list(added),
        declared_patch_ids: declared.map(list),
        claim_outcomes: vec![],
        submitter: "alice".to_owned(),
        author_key_ids: vec![],
        created_at: 1,
    }
}

#[test]
fn the_current_patch_set_is_the_latest_declared_set_or_the_union_of_what_undeclared_revisions_added()
 {
    // Declared: the latest revision's declared set, regardless of order in the slice.
    let revs = [rev(2, 5..7, Some(3..8)), rev(1, 0..3, Some(0..3))];
    assert_eq!(current_patch_set(&revs), set(3..8));
    // Undeclared: the union of their added sets.
    let revs = [rev(1, 0..3, None), rev(2, 3..5, None)];
    assert_eq!(current_patch_set(&revs), set(0..5));
    // A change with no revision has no patch set.
    assert!(current_patch_set(&[]).is_empty());
    // Latest undeclared after a declared one: only undeclared revisions' added sets count (the declared one is not
    // the latest, so its declaration does not define the set).
    let revs = [rev(1, 0..3, Some(0..3)), rev(2, 3..5, None)];
    assert_eq!(current_patch_set(&revs), set(3..5));
}

#[test]
fn landing_is_observed_from_a_recorded_pending_sequence_and_never_asserted() {
    let patches = set(10..13);
    // The recorded sequence of `sync pending` observations, one per poll.
    let observations: Vec<BTreeSet<String>> = vec![
        set(10..16),                                        // all three pending
        [10, 12, 14, 15].iter().map(|n| pid(*n)).collect(), // 11 left (sealed elsewhere, delivered)
        [14, 15].iter().map(|n| pid(*n)).collect(),         // all three gone
        set(10..16), // a later, odd observation (e.g. a stale read): not our business
    ];
    let outcomes: Vec<Landing> = observations
        .iter()
        .map(|o| observe_landing(&patches, o))
        .collect();
    assert_eq!(
        outcomes[0],
        Landing::NotYet {
            remaining: list(10..13)
        }
    );
    assert_eq!(
        outcomes[1],
        Landing::NotYet {
            remaining: vec![pid(10), pid(12)]
        }
    );
    assert_eq!(outcomes[2], Landing::Landed);
    // The function reports each observation; keeping a change landed once observed is the state machine's job
    // (`landed` never reopens), not a reason for this function to remember.
    assert_eq!(
        outcomes[3],
        Landing::NotYet {
            remaining: list(10..13)
        }
    );
}

#[test]
fn an_empty_patch_set_is_never_observed_as_landed() {
    assert_eq!(
        observe_landing(&BTreeSet::new(), &BTreeSet::new()),
        Landing::NotYet { remaining: vec![] }
    );
}

#[test]
fn a_superseded_patch_that_no_one_seals_keeps_an_undeclared_change_open() {
    let revs = [rev(1, 0..2, None), rev(2, 2..4, None)];
    let pending_now: BTreeSet<String> = [pid(0)].into_iter().collect(); // revision 1's patch 0 never sealed
    assert_eq!(
        observe_landing(&current_patch_set(&revs), &pending_now),
        Landing::NotYet {
            remaining: vec![pid(0)]
        }
    );
}

// ------------------------------------------------------------------------------------------------ quotas

const MIB: u64 = 1024 * 1024;

fn usage(open_changes: u64, accepted_mib: u64, pending_mib: u64) -> QuotaUsage {
    QuotaUsage {
        open_changes,
        accepted_bytes: accepted_mib * MIB,
        repo_pending_bytes: pending_mib * MIB,
    }
}

fn new_change(mib: u64) -> QuotaRequest {
    QuotaRequest {
        bytes: mib * MIB,
        target: QuotaTarget::NewChange,
    }
}

fn more_revisions(have: u64, mib: u64) -> QuotaRequest {
    QuotaRequest {
        bytes: mib * MIB,
        target: QuotaTarget::ExistingChange { revisions: have },
    }
}

#[test]
fn the_defaults_are_the_table_of_rfc_005_q2() {
    let m = QuotaPolicy::defaults_for(ProposalPolicy::Members);
    assert_eq!(
        (m.open_changes, m.accepted_bytes, m.revisions_per_change),
        (10, 512 * MIB, 20)
    );
    let a = QuotaPolicy::defaults_for(ProposalPolicy::AnyAuthenticated);
    assert_eq!(
        (a.open_changes, a.accepted_bytes, a.revisions_per_change),
        (3, 64 * MIB, 10)
    );
    for p in [m, a] {
        assert_eq!(
            p.repo_pending_bytes,
            4 * 1024 * MIB,
            "the repository ceiling is the same in both"
        );
        assert_eq!(p.retention_grace_secs, 30 * 24 * 3600);
    }
}

#[test]
fn open_changes_the_limit_is_allowed_and_one_over_is_refused() {
    for (policy, limit) in [
        (ProposalPolicy::Members, 10u64),
        (ProposalPolicy::AnyAuthenticated, 3),
    ] {
        let p = QuotaPolicy::defaults_for(policy);
        assert_eq!(
            check_quota(&usage(limit - 1, 0, 0), &p, &new_change(1)),
            Ok(()),
            "opening the last allowed one"
        );
        let err = check_quota(&usage(limit, 0, 0), &p, &new_change(1)).unwrap_err();
        assert_eq!(
            (err.quota, err.limit, err.used),
            (Quota::OpenChanges, limit, limit)
        );
        // Adding a revision to an existing change does not open one.
        assert_eq!(
            check_quota(&usage(limit, 0, 0), &p, &more_revisions(1, 1)),
            Ok(())
        );
    }
}

#[test]
fn accepted_bytes_the_limit_is_allowed_and_one_byte_over_is_refused() {
    for (policy, limit_mib) in [
        (ProposalPolicy::Members, 512u64),
        (ProposalPolicy::AnyAuthenticated, 64),
    ] {
        let p = QuotaPolicy::defaults_for(policy);
        assert_eq!(
            check_quota(&usage(0, limit_mib - 1, 0), &p, &new_change(1)),
            Ok(()),
            "exactly at the limit"
        );
        let one_over = QuotaRequest {
            bytes: MIB + 1,
            target: QuotaTarget::NewChange,
        };
        let err = check_quota(&usage(0, limit_mib - 1, 0), &p, &one_over).unwrap_err();
        assert_eq!(
            (err.quota, err.limit),
            (Quota::AcceptedBytes, limit_mib * MIB)
        );
        assert_eq!(err.used, (limit_mib - 1) * MIB);
        assert_eq!(err.requested, MIB + 1);
    }
}

#[test]
fn revisions_per_change_the_limit_is_allowed_and_one_over_is_refused() {
    for (policy, limit) in [
        (ProposalPolicy::Members, 20u64),
        (ProposalPolicy::AnyAuthenticated, 10),
    ] {
        let p = QuotaPolicy::defaults_for(policy);
        assert_eq!(
            check_quota(&usage(0, 0, 0), &p, &more_revisions(limit - 1, 1)),
            Ok(()),
            "the last allowed revision"
        );
        let err = check_quota(&usage(0, 0, 0), &p, &more_revisions(limit, 1)).unwrap_err();
        assert_eq!(
            (err.quota, err.limit, err.used),
            (Quota::RevisionsPerChange, limit, limit)
        );
    }
}

#[test]
fn the_repository_ceiling_is_allowed_at_the_limit_and_refused_one_over_by_name() {
    let p = QuotaPolicy::defaults_for(ProposalPolicy::Members);
    let ceiling = p.repo_pending_bytes;
    let at = QuotaUsage {
        open_changes: 0,
        accepted_bytes: 0,
        repo_pending_bytes: ceiling - 10,
    };
    assert_eq!(
        check_quota(
            &at,
            &p,
            &QuotaRequest {
                bytes: 10,
                target: QuotaTarget::NewChange
            }
        ),
        Ok(())
    );
    let err = check_quota(
        &at,
        &p,
        &QuotaRequest {
            bytes: 11,
            target: QuotaTarget::NewChange,
        },
    )
    .unwrap_err();
    assert_eq!(
        (err.quota, err.limit, err.used),
        (Quota::RepositoryCapacity, ceiling, ceiling - 10)
    );
}

#[test]
fn a_custom_operator_ceiling_is_honoured() {
    let mut p = QuotaPolicy::defaults_for(ProposalPolicy::Members);
    p.repo_pending_bytes = 100 * MIB;
    assert!(check_quota(&usage(0, 0, 99), &p, &new_change(1)).is_ok());
    assert_eq!(
        check_quota(&usage(0, 0, 99), &p, &new_change(2))
            .unwrap_err()
            .quota,
        Quota::RepositoryCapacity
    );
}

#[test]
fn when_two_quotas_would_refuse_the_repository_is_named_first_and_then_the_one_the_person_can_act_on()
 {
    let p = QuotaPolicy::defaults_for(ProposalPolicy::AnyAuthenticated);
    let everything = QuotaUsage {
        open_changes: 3,
        accepted_bytes: 64 * MIB,
        repo_pending_bytes: p.repo_pending_bytes,
    };
    assert_eq!(
        check_quota(&everything, &p, &new_change(1))
            .unwrap_err()
            .quota,
        Quota::RepositoryCapacity
    );
    let person = QuotaUsage {
        open_changes: 3,
        accepted_bytes: 64 * MIB,
        repo_pending_bytes: 0,
    };
    assert_eq!(
        check_quota(&person, &p, &new_change(1)).unwrap_err().quota,
        Quota::OpenChanges
    );
    assert_eq!(
        check_quota(&person, &p, &more_revisions(10, 1))
            .unwrap_err()
            .quota,
        Quota::RevisionsPerChange
    );
    assert_eq!(
        check_quota(&person, &p, &more_revisions(1, 1))
            .unwrap_err()
            .quota,
        Quota::AcceptedBytes
    );
}

#[test]
fn a_huge_request_cannot_overflow_its_way_past_a_quota() {
    let p = QuotaPolicy::defaults_for(ProposalPolicy::Members);
    let req = QuotaRequest {
        bytes: u64::MAX,
        target: QuotaTarget::NewChange,
    };
    assert!(check_quota(&usage(0, 1, 1), &p, &req).is_err());
    let full = QuotaUsage {
        open_changes: u64::MAX,
        accepted_bytes: u64::MAX,
        repo_pending_bytes: 0,
    };
    assert!(check_quota(&full, &p, &new_change(1)).is_err());
}

#[test]
fn every_refusal_names_its_quota_with_message_and_hint_keys_and_figures_in_the_unit_the_sentence_uses()
 {
    let all = [
        QuotaRefusal {
            quota: Quota::OpenChanges,
            limit: 10,
            used: 10,
            requested: 1,
        },
        QuotaRefusal {
            quota: Quota::AcceptedBytes,
            limit: 512 * MIB,
            used: 511 * MIB + 1,
            requested: 3 * MIB,
        },
        QuotaRefusal {
            quota: Quota::RevisionsPerChange,
            limit: 20,
            used: 20,
            requested: 1,
        },
        QuotaRefusal {
            quota: Quota::RepositoryCapacity,
            limit: 4096 * MIB,
            used: 4000 * MIB,
            requested: 200 * MIB,
        },
    ];
    let mut keys = BTreeSet::new();
    for r in all {
        assert!(r.message_key().starts_with("quota.") && r.message_key().ends_with(".message"));
        assert!(r.hint_key().starts_with("quota.") && r.hint_key().ends_with(".hint"));
        assert!(
            keys.insert(r.message_key()) && keys.insert(r.hint_key()),
            "keys are distinct"
        );
        assert!(
            QuotaRefusal::ALL_KEYS.contains(&r.message_key())
                && QuotaRefusal::ALL_KEYS.contains(&r.hint_key())
        );
    }
    assert_eq!(
        keys.len(),
        QuotaRefusal::ALL_KEYS.len(),
        "ALL_KEYS lists exactly the keys the refusals use"
    );
    assert_eq!(all[0].figures(), (10, 10));
    assert_eq!(
        all[1].figures(),
        (512, 512),
        "the use is rounded up so a sentence never claims more room than there is"
    );
    assert_eq!(all[3].figures(), (4096, 4000));
}

// ------------------------------------------------------------------------------------------------ assurance

#[test]
fn sealed_only_from_a_block_read_from_log() {
    let f = PatchFacts {
        in_block_from_log: true,
        listed_pending: false,
        reported_queued: false,
    };
    assert_eq!(patch_assurance(&f), Some(Assurance::Sealed));
}

#[test]
fn a_pending_patch_whose_queued_is_false_is_never_sealed() {
    // U-10: a pending patch reports `queued: false`.
    let f = PatchFacts {
        in_block_from_log: false,
        listed_pending: true,
        reported_queued: false,
    };
    assert_eq!(patch_assurance(&f), Some(Assurance::ForgeApprovedUnsealed));
}

#[test]
fn a_patch_nothing_vouches_for_has_no_assurance_and_queued_does_not_change_that() {
    for queued in [false, true] {
        let f = PatchFacts {
            in_block_from_log: false,
            listed_pending: false,
            reported_queued: queued,
        };
        assert_eq!(
            patch_assurance(&f),
            None,
            "queued={queued}: not in a block, not listed pending: unknown"
        );
    }
}

/// Universal rule over all eight combinations: `Sealed` is returned only when a block from `log` contains the
/// patch and `sync pending` does not list it; `queued` never changes the answer.
#[test]
fn no_combination_makes_a_pending_patch_sealed_and_queued_never_matters() {
    for bits in 0..8u8 {
        let facts = |queued| PatchFacts {
            in_block_from_log: bits & 1 != 0,
            listed_pending: bits & 2 != 0,
            reported_queued: queued,
        };
        let (a, b) = (
            patch_assurance(&facts(false)),
            patch_assurance(&facts(true)),
        );
        assert_eq!(a, b, "queued must not matter (bits {bits:03b})");
        if bits & 2 != 0 {
            assert_ne!(
                a,
                Some(Assurance::Sealed),
                "a pending patch is never sealed (bits {bits:03b})"
            );
        }
        if a == Some(Assurance::Sealed) {
            assert!(bits & 1 != 0, "sealed only from a block read from log");
        }
        assert_ne!(
            a,
            Some(Assurance::PrikkVerified),
            "verification is not this function's to claim"
        );
    }
}
