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
    // a merge whose first-parent line is a clean chain: the second parent must still make it not one change
    let mut m2 = claim(2, 12, Some(11));
    m2.parent_block_ids.push(id(99));
    let merge2 = pending(vec![claim(1, 11, Some(10)), m2]);
    assert_eq!(
        build_chain(&ids(&[1, 2]), &merge2).unwrap_err(),
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

// ------------------------------------------------------------------------------------------------ the bound on waiters

use planeter_transport::{ChangesConfig, ChangesService, MAX_WAITERS, Metrics, TooManyWaiters};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn service() -> ChangesService {
    let hosting = Arc::new(planeter_core::HostingService::new(
        std::env::temp_dir().join("changes-flight"),
        Arc::new(planeter_store::InMemoryRepositoryStore::new())
            as Arc<dyn planeter_store::RepositoryStore>,
    ));
    ChangesService::new(hosting, Arc::new(Metrics::new()), ChangesConfig::default())
}

#[test]
fn at_most_eight_requests_wait_for_one_revision_and_the_ninth_is_refused_without_parking() {
    let svc = service();
    let holder = svc.flight("rev-a").unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut waiters = Vec::new();
    for _ in 0..MAX_WAITERS {
        let (svc, started, finished) = (svc.clone(), Arc::clone(&started), Arc::clone(&finished));
        waiters.push(std::thread::spawn(move || {
            started.fetch_add(1, Ordering::SeqCst);
            let f = svc.flight("rev-a").expect("a waiter within the bound");
            finished.fetch_add(1, Ordering::SeqCst);
            drop(f);
        }));
    }
    // wait until all eight are parked (they cannot finish: the holder still holds)
    let t = std::time::Instant::now();
    while started.load(Ordering::SeqCst) < MAX_WAITERS && t.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(finished.load(Ordering::SeqCst), 0, "they wait");
    // the ninth: at once (on a thread of its own, so that a missing bound fails this test instead of hanging it)
    let (tx, rx) = std::sync::mpsc::channel();
    let ninth = svc.clone();
    std::thread::spawn(move || {
        let r = ninth.flight("rev-a").map(|_| ());
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(r) => assert_eq!(r, Err(TooManyWaiters), "the ninth request is refused"),
        Err(_) => panic!(
            "the ninth request parked instead of being refused: there is no bound on the waiters"
        ),
    }
    // another revision is not affected
    drop(svc.flight("rev-b").expect("another key has its own queue"));
    drop(holder);
    for w in waiters {
        w.join().unwrap();
    }
    assert_eq!(
        finished.load(Ordering::SeqCst),
        MAX_WAITERS,
        "every waiter got its turn"
    );
    // and the bookkeeping is clean: a new request is the holder again
    let again = svc.flight("rev-a").expect("free again");
    drop(again);
}
