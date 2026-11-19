//! RFC 008 D-11 (handoff 4, T1): the pure part of the landed view — **which blocks of the project's branch are a revision's one change** — over
//! blocks made by hand (the real thing, with real prikk and a real delivery, is `planeter-web/tests/changes_landed.rs`). `landed_run` is given the
//! revision's patch ids and the blocks `log` lists (newest first, as prikk gives them) and answers the last block and the block before the first, or
//! `None` when the patches do not form one single run.

use std::collections::BTreeSet;

use planeter_prikk::model::{LogBlock, PatchMessage};
use planeter_transport::changes::engine::{LandedRun, landed_run};

fn block(id: &str, parents: &[&str], patches: &[&str]) -> LogBlock {
    LogBlock {
        block_id: id.to_owned(),
        ref_state_id: format!("rs-{id}"),
        update_seq: 0,
        kind: "Normal".to_owned(),
        rollback_block: false,
        parent_count: parents.len() as u64,
        patch_count: patches.len() as u64,
        rollback_patch_count: 0,
        required_attestation_count: 0,
        patch_messages: patches
            .iter()
            .map(|p| PatchMessage {
                patch_id: (*p).to_owned(),
                message: "m".to_owned(),
            })
            .collect(),
        previous_ref_state_id: None,
        sealed_by: vec![],
        parent_block_ids: parents.iter().map(|p| (*p).to_owned()).collect(),
    }
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| (*s).to_owned()).collect()
}

fn run(before: Option<&str>, after: &str) -> Option<LandedRun> {
    Some(LandedRun {
        before: before.map(str::to_owned),
        after: after.to_owned(),
    })
}

/// A branch of five blocks, newest first: b5 (x4) ← b4 (p1) ← b3 (p2) ← b2 (x2) ← b1 (root, x1).
fn line() -> Vec<LogBlock> {
    vec![
        block("b5", &["b4"], &["x4"]),
        block("b4", &["b3"], &["p1"]),
        block("b3", &["b2"], &["p2"]),
        block("b2", &["b1"], &["x2"]),
        block("b1", &[], &["x1"]),
    ]
}

#[test]
fn one_block_is_a_run_and_its_parent_is_the_before() {
    assert_eq!(landed_run(&set(&["p1"]), &line()), run(Some("b3"), "b4"));
}

#[test]
fn consecutive_blocks_are_one_run_from_the_parent_of_the_oldest_to_the_newest() {
    assert_eq!(
        landed_run(&set(&["p1", "p2"]), &line()),
        run(Some("b2"), "b4")
    );
}

#[test]
fn a_run_that_starts_the_history_has_no_before() {
    assert_eq!(landed_run(&set(&["x1"]), &line()), run(None, "b1"));
    assert_eq!(landed_run(&set(&["x1", "x2"]), &line()), run(None, "b2"));
}

#[test]
fn blocks_with_another_block_between_them_are_not_one_run() {
    // p1 (b4) and x2 (b2) with b3 (p2: another revision's) between
    assert_eq!(landed_run(&set(&["p1", "x2"]), &line()), None);
}

#[test]
fn a_block_that_carries_another_patch_as_well_is_not_this_revisions_change() {
    let mut l = line();
    l[1] = block("b4", &["b3"], &["p1", "other"]);
    assert_eq!(landed_run(&set(&["p1"]), &l), None);
}

#[test]
fn a_merge_is_not_one_run() {
    let mut l = line();
    l[1] = block("b4", &["b3", "b9"], &["p1"]);
    assert_eq!(landed_run(&set(&["p1"]), &l), None);
}

#[test]
fn a_block_whose_parent_count_and_parent_ids_disagree_is_not_provable() {
    // prikk says one parent and names none: taken for a root it would show the whole tree as added
    let mut l = line();
    l[1].parent_block_ids.clear();
    assert_eq!(landed_run(&set(&["p1"]), &l), None);
}

#[test]
fn a_patch_in_two_blocks_is_not_one_run_and_never_a_mix() {
    let mut l = line();
    l[3] = block("b2", &["b1"], &["p1"]); // p1 again, in b2
    assert_eq!(landed_run(&set(&["p1"]), &l), None);
    // twice in one block
    let l = vec![
        block("b2", &["b1"], &["p1", "p1"]),
        block("b1", &[], &["x1"]),
    ];
    assert_eq!(landed_run(&set(&["p1"]), &l), None);
}

#[test]
fn two_separate_runs_are_not_one() {
    let l = vec![
        block("b4", &["b3"], &["q"]),
        block("b3", &["b2"], &["x"]),
        block("b2", &["b1"], &["p"]),
        block("b1", &[], &["r"]),
    ];
    assert_eq!(landed_run(&set(&["q", "p"]), &l), None);
}

#[test]
fn blocks_that_are_not_on_one_first_parent_chain_are_not_one_run() {
    // two feet: b3 and b4 both build on b2
    let l = vec![
        block("b4", &["b2"], &["p2"]),
        block("b3", &["b2"], &["p1"]),
        block("b2", &["b1"], &["x"]),
        block("b1", &[], &["r"]),
    ];
    assert_eq!(landed_run(&set(&["p1", "p2"]), &l), None);
}

#[test]
fn a_patch_that_is_in_no_block_is_not_a_run_at_all() {
    assert_eq!(landed_run(&set(&["nowhere"]), &line()), None);
    assert_eq!(landed_run(&set(&[]), &line()), None);
}

#[test]
fn a_three_patch_block_sealed_together_is_one_run_of_one_block() {
    let l = vec![
        block("b2", &["b1"], &["a", "b", "c"]),
        block("b1", &[], &["r"]),
    ];
    assert_eq!(
        landed_run(&set(&["a", "b", "c"]), &l),
        run(Some("b1"), "b2")
    );
    // and a revision that has only two of the three is not that block's change
    assert_eq!(landed_run(&set(&["a", "b"]), &l), None);
}
