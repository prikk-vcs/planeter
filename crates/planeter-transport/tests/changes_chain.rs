//! The chain of a revision's claims (RFC 005 part 6): one chain, its foot and the real parent of its root; everything else is named.

use planeter_prikk::exchange::{
    ClaimSignature, PendingClaim, PendingPatch, SignatureOutcome, SyncPending,
};
use planeter_transport::changes::chain::{ChainError, build_chain};

fn id(n: u8) -> String {
    format!("{:064x}", n)
}

fn claim(c: u8, target: u8, parent: Option<u8>) -> PendingClaim {
    PendingClaim {
        claim_id: id(c),
        target_block_id: id(target),
        parent_block_ids: parent.map(id).into_iter().collect(),
        signature: ClaimSignature {
            outcome: SignatureOutcome::Unverifiable,
            key_id: None,
        },
    }
}

fn pending(claims: Vec<PendingClaim>) -> SyncPending {
    // each claim names one patch; the same claim listed under two patches is one claim
    let patches = claims
        .iter()
        .enumerate()
        .flat_map(|(i, c)| {
            [
                PendingPatch {
                    patch_id: id(100 + i as u8),
                    claims: vec![c.clone()],
                },
                PendingPatch {
                    patch_id: id(150 + i as u8),
                    claims: vec![c.clone()],
                },
            ]
        })
        .collect();
    SyncPending {
        schema_version: "sync-pending-v1".to_owned(),
        patches,
    }
}

fn ids(v: &[u8]) -> Vec<String> {
    v.iter().map(|n| id(*n)).collect()
}

#[test]
fn one_claim_is_its_own_foot_and_root_and_its_parent_is_the_before_block() {
    let p = pending(vec![claim(1, 11, Some(10))]);
    let c = build_chain(&ids(&[1]), &p).unwrap();
    assert_eq!(
        (c.foot_claim, c.foot_target, c.root_parent, c.claims),
        (id(1), id(11), Some(id(10)), 1)
    );
}

#[test]
fn a_chain_of_three_has_the_newest_as_foot_and_the_oldest_ones_parent_as_root_parent_whatever_the_order()
 {
    let p = pending(vec![
        claim(3, 13, Some(12)),
        claim(1, 11, Some(10)),
        claim(2, 12, Some(11)),
    ]);
    for order in [[1, 2, 3], [3, 2, 1], [2, 3, 1]] {
        let c = build_chain(&ids(&order), &p).unwrap();
        assert_eq!(
            (c.foot_claim.as_str(), c.root_parent.clone(), c.claims),
            (id(3).as_str(), Some(id(10)), 3),
            "{order:?}"
        );
    }
}

#[test]
fn a_revision_that_starts_a_history_has_no_before_block() {
    let p = pending(vec![claim(1, 11, None), claim(2, 12, Some(11))]);
    let c = build_chain(&ids(&[1, 2]), &p).unwrap();
    assert_eq!((c.foot_claim, c.root_parent), (id(2), None));
}

#[test]
fn claims_of_other_proposals_in_the_store_are_not_part_of_this_revisions_chain() {
    // claim 9 builds on this revision's foot (another proposal on top of it): not ours, ignored
    let p = pending(vec![
        claim(1, 11, Some(10)),
        claim(2, 12, Some(11)),
        claim(9, 19, Some(12)),
    ]);
    let c = build_chain(&ids(&[1, 2]), &p).unwrap();
    assert_eq!(c.foot_claim, id(2));
}

#[test]
fn a_claim_that_is_no_longer_pending_is_claim_not_held_and_so_is_a_revision_without_claims() {
    let p = pending(vec![claim(1, 11, Some(10))]);
    assert_eq!(
        build_chain(&ids(&[1, 2]), &p).unwrap_err(),
        ChainError::ClaimNotHeld
    );
    assert_eq!(
        build_chain(&ids(&[2]), &p).unwrap_err(),
        ChainError::ClaimNotHeld
    );
    assert_eq!(build_chain(&[], &p).unwrap_err(), ChainError::ClaimNotHeld);
    assert_eq!(
        build_chain(&ids(&[1]), &pending(vec![])).unwrap_err(),
        ChainError::ClaimNotHeld
    );
}

#[test]
fn two_heads_an_unconnected_set_a_merge_and_two_claims_for_one_block_are_not_one_change() {
    // two heads on the same parent
    let two = pending(vec![claim(1, 11, Some(10)), claim(2, 12, Some(10))]);
    assert_eq!(
        build_chain(&ids(&[1, 2]), &two).unwrap_err(),
        ChainError::NotOneChange
    );
    // unconnected: 1 -> 11 on 10, 2 -> 22 on 20
    let apart = pending(vec![claim(1, 11, Some(10)), claim(2, 22, Some(20))]);
    assert_eq!(
        build_chain(&ids(&[1, 2]), &apart).unwrap_err(),
        ChainError::NotOneChange
    );
    // a merge: two parents
    let mut m = claim(3, 13, Some(11));
    m.parent_block_ids.push(id(12));
    let merge = pending(vec![claim(1, 11, Some(10)), claim(2, 12, Some(10)), m]);
    assert_eq!(
        build_chain(&ids(&[1, 2, 3]), &merge).unwrap_err(),
        ChainError::NotOneChange
    );
    // two claims for one block
    let dup = pending(vec![claim(1, 11, Some(10)), claim(2, 11, Some(10))]);
    assert_eq!(
        build_chain(&ids(&[1, 2]), &dup).unwrap_err(),
        ChainError::NotOneChange
    );
}

#[test]
fn a_cycle_is_not_one_change_and_does_not_loop() {
    let cyc = pending(vec![claim(1, 11, Some(12)), claim(2, 12, Some(11))]);
    assert_eq!(
        build_chain(&ids(&[1, 2]), &cyc).unwrap_err(),
        ChainError::NotOneChange
    );
    let selfloop = pending(vec![claim(1, 11, Some(11))]);
    assert_eq!(
        build_chain(&ids(&[1]), &selfloop).unwrap_err(),
        ChainError::NotOneChange
    );
}
