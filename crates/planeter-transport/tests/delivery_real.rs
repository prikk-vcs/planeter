//! RFC 008 handoff 1: the delivery engine against a **real prikk**. The stage and the rule for every archive of the measurement, in
//! both states of the forge; the install end to end (proposals landing, staying, landing in part); the concurrency rules. Tests that
//! need prikk print why and skip without one. The sandbox is off here (the engine's sandbox use is the same call as creation's).

#[path = "support/archives.rs"]
mod archives;
#[path = "support/delivery_world.rs"]
mod world;

use std::path::Path;

use planeter_prikk::PrikkRepo;
use planeter_store::{ChangeStore, DeliveryState, DeliveryStore};
use world::Fixtures;
use world::*;

macro_rules! need_prikk {
    () => {
        if !prikk_ok() {
            eprintln!("skipping: prikk is not on PATH");
            return;
        }
    };
}

/// A world: the live repository created from H0 (the keys M1 and M2 adopted), and the services over it.
fn new_world(name: &str) -> (Services, std::path::PathBuf) {
    let fx = fixtures();
    let base = scratch(name);
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    (open(&base, None), base)
}

/// Deliver and **confirm** (the whole road); the live repository is then the delivered one.
fn install(s: &Services, archive: &Path) -> planeter_transport::Installed {
    let (id, report) = deliver(s, archive).unwrap();
    assert!(report.is_acceptable(), "{report:?}");
    s.delivery.confirm(&alice(), &s.record, &id).unwrap()
}

type Expect = (&'static str, Result<(), &'static str>);

fn cases(fx: &Fixtures) -> Vec<(&'static str, std::path::PathBuf)> {
    let mut v = vec![
        ("D1", fx.d1.clone()),
        ("D2", fx.d2.clone()),
        ("older (H0)", fx.h0.clone()),
        ("div", fx.div.clone()),
        ("m2", fx.m2_archive.clone()),
        ("m2div", fx.m2div.clone()),
        ("x", fx.x_archive.clone()),
        ("mix", fx.mix.clone()),
        ("xcarry", fx.xcarry.clone()),
        ("dev", fx.dev.clone()),
        ("unrelated", fx.unrelated.clone()),
        ("truncated", fx.truncated.clone()),
    ];
    for (n, p) in &fx.flips {
        v.push((Box::leak(format!("flip {n}").into_boxed_str()), p.clone()));
    }
    v
}

/// What the rule answers for each archive, against the forge as created (S0, history H0) and after D1 was delivered (S1).
fn expected(label: &str, state: &str) -> Result<(), &'static str> {
    let s1 = state == "S1";
    match label {
        "D1" => {
            if s1 {
                Err("nothing_to_deliver")
            } else {
                Ok(())
            }
        }
        "D2" | "m2" | "xcarry" => Ok(()),
        "older (H0)" => Err(if s1 {
            "older_or_diverged"
        } else {
            "nothing_to_deliver"
        }),
        "div" | "m2div" => {
            if s1 {
                Err("older_or_diverged")
            } else {
                Ok(())
            }
        }
        "x" | "mix" => Err("unadopted_key"),
        // a delivery that only adds a branch changes a ref: acceptable in both states
        "dev" => Ok(()),
        "unrelated" => Err("unrelated"),
        _ => Err("damaged_archive"),
    }
}

fn run_table(s: &Services, state: &str) {
    let fx = fixtures();
    for (label, archive) in cases(fx) {
        let before = tree_hash(&s.record.path);
        let (id, report) =
            deliver(s, &archive).unwrap_or_else(|e| panic!("{label} ({state}): {e:?}"));
        let want = expected(label, state);
        match want {
            Ok(()) => {
                assert!(report.is_acceptable(), "{label} ({state}): {report:?}");
                if label == "dev" {
                    assert_eq!(report.refs_added, vec!["heads/dev".to_owned()]);
                }
                // an acceptable report only reports: the live repository is untouched until the confirmation
                assert_eq!(tree_hash(&s.record.path), before, "{label} ({state})");
                s.delivery.cancel(&alice(), &id).unwrap();
            }
            Err(reason) => {
                assert!(!report.is_acceptable(), "{label} ({state}): {report:?}");
                assert_eq!(report.reason.as_deref(), Some(reason), "{label} ({state})");
                assert_eq!(
                    tree_hash(&s.record.path),
                    before,
                    "a refusal leaves the live .prikk tree byte-identical: {label} ({state})"
                );
                let row = s.deliveries.get(&id).unwrap().unwrap();
                assert_eq!(row.state, DeliveryState::Failed, "{label}");
                assert_eq!(row.reason.as_deref(), Some(reason));
            }
        }
        // nothing is left in flight and no stage directory remains
        assert!(
            s.deliveries.non_final().unwrap().is_empty(),
            "{label} ({state})"
        );
        let left: Vec<_> = std::fs::read_dir(s.base.join("root/.deliveries"))
            .unwrap()
            .flatten()
            .collect();
        assert!(left.is_empty(), "{label} ({state}): {left:?}");
    }
}

#[test]
fn every_archive_of_the_measurement_against_the_forge_as_created() {
    need_prikk!();
    let (s, _) = new_world("table-s0");
    run_table(&s, "S0");
}

#[test]
fn every_archive_of_the_measurement_against_the_forge_after_a_delivery() {
    need_prikk!();
    let (s, _) = new_world("table-s1");
    let fx = fixtures();
    install(&s, &fx.d1);
    run_table(&s, "S1");
    let _: Option<Expect> = None;
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn a_delivery_lands_one_proposal_keeps_two_and_leaves_the_repository_the_delivered_one() {
    need_prikk!();
    let (s, _) = new_world("scenario");
    let fx = fixtures();
    let sc = scenario(&s, fx);
    assert_eq!(
        open_ids(&s),
        sorted(vec![sc.p1.clone(), sc.p2.clone(), sc.p3.clone()])
    );
    let revisions_before: Vec<_> = [&sc.p1, &sc.p2, &sc.p3]
        .iter()
        .map(|c| s.changes.revisions_of(c).unwrap())
        .collect();
    let fingerprint_before = s.export.state(&s.record).unwrap();
    let old_tip = s.live_tip();

    let (id, report) = deliver(&s, &sc.d).unwrap();
    assert!(report.is_acceptable(), "{report:?}");
    // the report says what will happen
    assert_eq!(report.old_tip.as_deref(), Some(old_tip.as_str()));
    assert_eq!(report.new_tip.as_deref(), Some(sc.d_tip.as_str()));
    assert_eq!(report.new_blocks, 4); // p1, p3a, e1, e2
    assert_eq!(report.proposals_land, vec![sc.p1.clone()]);
    assert_eq!(
        report.proposals_stay,
        sorted(vec![sc.p2.clone(), sc.p3.clone()])
    );
    assert!(report.proposals_cannot_be_kept.is_empty());
    assert_eq!(report.trusted_keys.len(), 2);
    assert_eq!(s.live_tip(), old_tip, "a report moves nothing");

    let done = s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    assert_eq!(done.new_tip, sc.d_tip);
    assert_eq!((done.reaccepted, done.skipped), (3, 0));

    // heads/main is the delivered tip, and nothing was received
    assert_eq!(s.live_tip(), sc.d_tip);
    let branches = s.live().branches(true).unwrap();
    assert!(branches.received.is_empty(), "no remotes/ ref");
    assert_eq!(
        branches
            .branches
            .iter()
            .map(|b| b.ref_name.as_str())
            .collect::<Vec<_>>(),
        vec!["heads/main"]
    );
    // `sync pending` is the open proposals' patches that the delivery does not carry
    assert_eq!(s.pending(), sc.stays);
    // every open proposal intact: still open (landing is observed later, never asserted here), revisions and claim ids unchanged
    assert_eq!(
        open_ids(&s),
        sorted(vec![sc.p1.clone(), sc.p2.clone(), sc.p3.clone()])
    );
    let revisions_after: Vec<_> = [&sc.p1, &sc.p2, &sc.p3]
        .iter()
        .map(|c| s.changes.revisions_of(c).unwrap())
        .collect();
    assert_eq!(revisions_before, revisions_after);
    // the fingerprint moved; the previous generation, the stage and the archive are gone
    assert_ne!(s.export.state(&s.record).unwrap(), fingerprint_before);
    assert!(deliveries_dir_is_empty(&s));
    // the repository verifies and doctor finds no error
    assert!(s.live().verify().unwrap().verdict.ok);
    assert_eq!(s.live().doctor_report().unwrap().errors, 0);
    // the row says who and when
    let last = s.delivery.last_done(&s.record.repo_id).unwrap().unwrap();
    assert_eq!(last.principal, "alice");
    assert_eq!(last.new_tip.as_deref(), Some(sc.d_tip.as_str()));
    assert_eq!(last.old_tip.as_deref(), Some(old_tip.as_str()));
    assert_eq!(last.state, DeliveryState::Done);

    // a second delivery that lands nothing new: the proposals are as they were
    let tip_of_d = s.live_tip();
    let (id2, report2) = deliver(&s, &sc.d2).unwrap();
    assert!(report2.is_acceptable());
    assert_eq!(report2.new_blocks, 1);
    // P1 is still `open` in the store (landing is observed, never asserted by the engine) and every patch of it is in the staged
    // chain: the report says it will land, as before; nothing else is new
    assert_eq!(report2.proposals_land, vec![sc.p1.clone()]);
    assert_eq!(
        report2.proposals_stay,
        sorted(vec![sc.p2.clone(), sc.p3.clone()])
    );
    let done2 = s.delivery.confirm(&alice(), &s.record, &id2).unwrap();
    assert_eq!(done2.reaccepted, 3);
    assert_ne!(s.live_tip(), tip_of_d);
    assert_eq!(s.pending(), sc.stays);
    assert!(s.live().verify().unwrap().verdict.ok);
}

#[test]
fn a_proposal_whose_retained_artifact_is_gone_is_listed_and_kept() {
    need_prikk!();
    let (s, _) = new_world("artifact-gone");
    let sc = scenario(&s, fixtures());
    // remove the retained artifact of P2
    let rev = &s.changes.revisions_of(&sc.p2).unwrap()[0];
    let digest = planeter_transport::artifacts::Digest::parse(
        rev.artifact_digest.strip_prefix("sha256:").unwrap(),
    )
    .unwrap();
    assert!(s.artifacts.remove(&digest).unwrap());
    let (id, report) = deliver(&s, &sc.d).unwrap();
    assert_eq!(report.proposals_cannot_be_kept, vec![sc.p2.clone()]);
    let done = s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    assert_eq!((done.reaccepted, done.skipped), (2, 1));
    assert!(open_ids(&s).contains(&sc.p2), "kept and readable");
    // P2's patch is not in the new repository (it could not be accepted again); P3's second patch is
    assert!(!s.pending().iter().any(|p| sc.p2_patches.contains(p)));
    assert!(s.live().verify().unwrap().verdict.ok);
}

// ---------------------------------------------------------------------------------------------------- concurrency and the edges

use planeter_core::LeaseClass;
use planeter_transport::{DeliveryError, DeliveryService, DeliveryUpload};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[test]
fn a_second_delivery_is_busy_while_one_is_in_flight_and_the_engine_takes_the_user_and_the_record() {
    need_prikk!();
    let (s, _) = new_world("busy");
    let fx = fixtures();
    // the shape of the entry points: the user and the repository's record, never a bare path
    let _: fn(
        &DeliveryService,
        &planeter_store::UserId,
        &planeter_store::RepositoryRecord,
        u64,
    ) -> Result<DeliveryUpload, DeliveryError> = DeliveryService::start;
    let _: fn(
        &DeliveryService,
        &planeter_store::UserId,
        &planeter_store::RepositoryRecord,
        &str,
    ) -> Result<planeter_transport::Installed, DeliveryError> = DeliveryService::confirm;
    let (id, report) = deliver(&s, &fx.d1).unwrap();
    assert!(report.is_acceptable());
    // in flight (reported): a second start, by anyone, is busy
    let busy = s.delivery.start(&alice(), &s.record, 10);
    assert!(
        matches!(busy, Err(DeliveryError::Busy { .. })),
        "{:?}",
        busy.err()
    );
    let other = planeter_store::UserId::new("bob");
    assert!(matches!(
        s.delivery.start(&other, &s.record, 10),
        Err(DeliveryError::Busy { .. })
    ));
    // an upload that is begun counts too, and one that is dropped frees the repository
    s.delivery.cancel(&alice(), &id).unwrap();
    let up = s.delivery.start(&alice(), &s.record, 10).unwrap();
    assert!(matches!(
        s.delivery.start(&alice(), &s.record, 10),
        Err(DeliveryError::Busy { .. })
    ));
    drop(up);
    assert!(s.deliveries.non_final().unwrap().is_empty());
    assert!(deliveries_dir_is_empty(&s));
    assert!(s.delivery.start(&alice(), &s.record, 10).is_ok());
    // the bound is checked against what the upload declares, before a byte is read
    let big = s.delivery.config().max_archive_bytes + 1;
    assert!(matches!(
        s.delivery.start(&alice(), &s.record, big),
        Err(DeliveryError::TooLarge { .. })
    ));
}

#[test]
fn two_confirmations_of_one_delivery_install_it_once() {
    need_prikk!();
    let (s, _) = new_world("two-confirms");
    let sc = scenario(&s, fixtures());
    let (id, report) = deliver(&s, &sc.d).unwrap();
    assert!(report.is_acceptable());
    let results: Vec<_> = std::thread::scope(|scope| {
        let a = scope.spawn(|| s.delivery.confirm(&alice(), &s.record, &id));
        let b = scope.spawn(|| s.delivery.confirm(&alice(), &s.record, &id));
        vec![a.join().unwrap(), b.join().unwrap()]
    });
    let oks = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(oks, 1, "{results:?}");
    for r in results.iter().filter(|r| r.is_err()) {
        assert!(
            matches!(
                r,
                Err(DeliveryError::Busy { .. }
                    | DeliveryError::NotConfirmable(_)
                    | DeliveryError::Failed(_))
            ),
            "{r:?}"
        );
    }
    assert_eq!(s.live_tip(), sc.d_tip);
    assert_eq!(s.pending(), sc.stays);
    let done = s
        .deliveries
        .all()
        .unwrap()
        .into_iter()
        .filter(|r| r.state == DeliveryState::Done)
        .count();
    assert_eq!(done, 1);
    assert!(s.live().verify().unwrap().verdict.ok);
}

#[test]
fn a_confirmation_waits_for_an_accept_and_stays_reported_when_the_lease_does_not_come() {
    need_prikk!();
    let (s, _) = new_world("accept-vs-confirm");
    let sc = scenario(&s, fixtures());
    let (id, _) = deliver(&s, &sc.d).unwrap();
    // an accept holds the write lease
    let lease = s
        .leases
        .acquire(
            &s.record.repo_id,
            LeaseClass::Write,
            "sync accept",
            Duration::from_secs(1),
        )
        .unwrap();
    let busy = s.delivery.confirm(&alice(), &s.record, &id);
    assert!(matches!(busy, Err(DeliveryError::Busy { .. })), "{busy:?}");
    // nothing was touched and the delivery can still be confirmed
    let view = s.delivery.view(&alice(), &id).unwrap();
    assert_eq!(view.state, DeliveryState::Reported);
    drop(lease);
    let done = s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    assert_eq!(done.new_tip, sc.d_tip);
    assert!(
        s.metrics
            .snapshot()
            .deliveries
            .get("busy")
            .copied()
            .unwrap_or(0)
            >= 1
    );
}

#[test]
fn an_accept_waits_while_a_confirmation_holds_the_lease() {
    need_prikk!();
    let fx = fixtures();
    let base = scratch("confirm-vs-accept");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    // The confirmation is **held** inside its re-accepting by a latch the test releases once it has seen what it came to see (IS-72: it was
    // held for five seconds by a sleep, which a slow host could use up before the proposal below was made).
    let inside = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let hook_flag = inside.clone();
    let hook_release = release.clone();
    let hook: Hook = Arc::new(move |name: &str| {
        if name == "reaccept_first" {
            hook_flag.store(true, Ordering::SeqCst);
            let end = std::time::Instant::now() + Duration::from_secs(120);
            while !hook_release.load(Ordering::SeqCst) && std::time::Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    });
    let s = open(&base, Some(hook));
    let sc = scenario(&s, fx);
    let (id, _) = deliver(&s, &sc.d).unwrap();
    std::thread::scope(|scope| {
        let confirming = scope.spawn(|| s.delivery.confirm(&alice(), &s.record, &id));
        while !inside.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        // a proposal now meets the write lease the confirmation holds: busy, never a prikk error
        let who = planeter_transport::Submitter {
            id: "alice".into(),
            can_write: true,
        };
        let meta = planeter_transport::ProposeMeta {
            title: Some("late".into()),
            ..Default::default()
        };
        let artifact = b"not an artifact";
        let plan = s
            .propose
            .check(&s.record, &who, &meta, artifact.len() as u64)
            .unwrap();
        let mut up = s.propose.start_upload(artifact.len() as u64).unwrap();
        up.write(artifact).unwrap();
        let up = up.finish().unwrap();
        let r = s.propose.accept(&s.record, &who, &meta, &plan, up);
        assert!(
            matches!(r, Err(planeter_transport::ProposeError::Busy { .. })),
            "{r:?}"
        );
        release.store(true, Ordering::SeqCst);
        assert!(confirming.join().unwrap().is_ok());
    });
}

#[test]
fn a_confirmation_that_finds_the_repository_moved_fails_and_touches_nothing() {
    need_prikk!();
    let (s, base) = new_world("moved");
    let fx = fixtures();
    let (id, report) = deliver(&s, &fx.d1).unwrap();
    assert!(report.is_acceptable());
    // The live tip is no longer the one the report saw (a delivery cannot be moved by anything but another delivery; the row is edited
    // here to the tip the repository would have had).
    let db = planeter_store::SqliteDb::open(base.join("state.db")).unwrap();
    db.lock()
        .unwrap()
        .execute(
            "UPDATE deliveries SET old_tip = 'not-the-live-tip' WHERE delivery_id = ?1",
            [&id],
        )
        .unwrap();
    let before = tree_hash(&s.record.path);
    let r = s.delivery.confirm(&alice(), &s.record, &id);
    assert_eq!(
        r.as_ref().err(),
        Some(&DeliveryError::Failed("moved".into())),
        "{r:?}"
    );
    assert_eq!(tree_hash(&s.record.path), before);
    let row = s.deliveries.get(&id).unwrap().unwrap();
    assert_eq!(
        (row.state, row.reason.as_deref()),
        (DeliveryState::Failed, Some("moved"))
    );
    assert!(deliveries_dir_is_empty(&s));
}

#[test]
fn only_the_user_who_started_a_delivery_may_see_or_confirm_it_and_only_once_reported() {
    need_prikk!();
    let (s, _) = new_world("principal");
    let fx = fixtures();
    let (id, _) = deliver(&s, &fx.d1).unwrap();
    let bob = planeter_store::UserId::new("bob");
    assert_eq!(
        s.delivery.view(&bob, &id).err(),
        Some(DeliveryError::NotFound)
    );
    assert_eq!(
        s.delivery.confirm(&bob, &s.record, &id).err(),
        Some(DeliveryError::NotFound)
    );
    assert_eq!(
        s.delivery.view(&alice(), "not-an-id").err(),
        Some(DeliveryError::NotFound)
    );
    s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    // confirmed already: not confirmable again
    assert!(matches!(
        s.delivery.confirm(&alice(), &s.record, &id),
        Err(DeliveryError::NotConfirmable(DeliveryState::Done))
    ));
    // a refused report cannot be confirmed
    let (id2, report) = deliver(&s, &fx.d1).unwrap();
    assert_eq!(report.reason.as_deref(), Some("nothing_to_deliver"));
    assert!(matches!(
        s.delivery.confirm(&alice(), &s.record, &id2),
        Err(DeliveryError::NotConfirmable(DeliveryState::Failed))
    ));
}

#[test]
fn a_delivery_whose_day_is_over_expires_and_a_flagged_repository_refuses_a_delivery() {
    need_prikk!();
    let fx = fixtures();
    let base = scratch("expiry");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let s = open_with(&base, None, |c| c.stage_ttl = Duration::from_secs(1));
    let (id, report) = deliver(&s, &fx.d1).unwrap();
    assert!(report.is_acceptable());
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        s.delivery.confirm(&alice(), &s.record, &id).err(),
        Some(DeliveryError::Expired)
    );
    assert_eq!(
        s.deliveries.get(&id).unwrap().unwrap().state,
        DeliveryState::Expired
    );
    assert!(deliveries_dir_is_empty(&s));
    // a repository that needs recovery refuses to start a delivery
    s.changes
        .set_needs_recovery(
            &s.record.repo_id,
            "a write did not finish (accept_failed)",
            1,
        )
        .unwrap();
    assert!(matches!(
        s.delivery.start(&alice(), &s.record, 10),
        Err(DeliveryError::NeedsRecovery { .. })
    ));
}

#[test]
fn every_outcome_is_counted_once_under_its_own_word() {
    need_prikk!();
    let (s, _) = new_world("counters");
    let fx = fixtures();
    install(&s, &fx.d1); // accepted_reports, confirmed, exchange_window, done
    let (_, r) = deliver(&s, &fx.d1).unwrap(); // nothing_to_deliver
    assert_eq!(r.reason.as_deref(), Some("nothing_to_deliver"));
    let (_, r) = deliver(&s, &fx.unrelated).unwrap(); // unrelated
    assert_eq!(r.reason.as_deref(), Some("unrelated"));
    let (id, _) = deliver(&s, &fx.d2).unwrap();
    s.delivery.cancel(&alice(), &id).unwrap(); // abandoned
    let c = s.metrics.snapshot().deliveries;
    for (word, n) in [
        ("accepted_reports", 2),
        ("confirmed", 1),
        ("exchange_window", 1),
        ("done", 1),
        ("nothing_to_deliver", 1),
        ("unrelated", 1),
        ("abandoned", 1),
    ] {
        assert_eq!(c.get(word).copied().unwrap_or(0), n, "{word}: {c:?}");
    }
}

// ---------------------------------------------------------------------------------------------------- timings

fn random_bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15 ^ seed;
    let mut data = vec![0u8; n];
    for chunk in data.chunks_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let b = x.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
    data
}

/// Prints the timings the review request quotes (`cargo test … -- --nocapture timings`): a small delivery with three proposals to
/// accept again, and a 63 MiB delivery over a 20 MiB proposal. Both go through the default ceilings and the default deadline.
#[test]
fn timings_of_a_small_and_a_63_mib_delivery_end_to_end() {
    need_prikk!();
    use std::time::Instant;
    let fx = fixtures();
    // small
    let (s, _) = new_world("timing-small");
    let sc = scenario(&s, fx);
    let t = Instant::now();
    let (id, report) = deliver(&s, &sc.d).unwrap();
    let stage = t.elapsed();
    assert!(report.is_acceptable());
    let t = Instant::now();
    let done = s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    let confirm = t.elapsed();
    eprintln!(
        "TIMING small: archive {} bytes; stage+report {stage:?}; confirm {confirm:?} (exchange window {:?}, {} revisions accepted again)",
        report.archive_bytes, done.window, done.reaccepted
    );
    // 63 MiB
    let (s, base) = new_world("timing-big");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let at_h0 = archives::Maintainer {
        work: fx.m1_at_h0.clone(),
        keys: fx.m1_at_h0_keys.clone(),
    };
    let blob = clone_of(&at_h0, &work, "blob");
    blob.write("blob.bin", "");
    std::fs::write(blob.work.join("blob.bin"), random_bytes(20 << 20, 1)).unwrap();
    blob.commit("a 20 MiB file");
    blob.seal(None);
    let artifact = artifact_of(
        &blob,
        &s.record.path,
        &base.join("forge-keys"),
        "heads/main",
        "blob",
    );
    let t = Instant::now();
    s.propose_artifact("a 20 MiB proposal", &artifact, None);
    let first_accept = t.elapsed();
    let big = clone_of(&at_h0, &work, "big");
    std::fs::write(big.work.join("big.bin"), random_bytes(63 << 20, 2)).unwrap();
    big.commit("a 63 MiB file");
    big.seal(None);
    let archive = work.join("big.archive");
    big.archive(&archive);
    let size = std::fs::metadata(&archive).unwrap().len();
    let t = Instant::now();
    let (id, report) = deliver(&s, &archive).unwrap();
    let stage = t.elapsed();
    assert!(report.is_acceptable(), "{report:?}");
    let t = Instant::now();
    let done = s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    let confirm = t.elapsed();
    eprintln!(
        "TIMING large: archive {size} bytes; stage+report {stage:?}; confirm {confirm:?} (exchange window {:?}, {} revision of {} bytes accepted again; the same artifact's first accept took {first_accept:?})",
        done.window,
        done.reaccepted,
        artifact.len()
    );
    assert!(s.live().verify().unwrap().verdict.ok);
}

#[test]
fn under_the_real_sandbox_the_whole_road_works_with_the_ceilings_of_the_cost_model() {
    need_prikk!();
    if !sandbox_works() {
        eprintln!("skipping: bubblewrap or prlimit does not work here");
        return;
    }
    let fx = fixtures();
    let base = scratch("sandboxed");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let s = open_full(
        &base,
        None,
        planeter_prikk::Sandbox::default(),
        None,
        |_| {},
    );
    let sc = scenario(&s, fx);
    let (id, report) = deliver(&s, &sc.d).unwrap();
    assert!(report.is_acceptable(), "{report:?}");
    let done = s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    assert_eq!(done.reaccepted, 3);
    assert_eq!(s.live_tip(), sc.d_tip);
    assert_eq!(s.pending(), sc.stays);
    // and the refusals under the sandbox
    let (_, r) = deliver(&s, &fx.truncated).unwrap();
    assert_eq!(r.reason.as_deref(), Some("damaged_archive"));
    let (_, r) = deliver(&s, &fx.x_archive).unwrap();
    assert_eq!(r.reason.as_deref(), Some("unadopted_key"));
}

/// A `prikk` that is the real one except for the calls whose first two words are `pattern`: those run `body` instead.
fn wrapper(base: &Path, pattern: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = base.join("prikk-wrapper.sh");
    let real = String::from_utf8(
        std::process::Command::new("which")
            .arg("prikk")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"{pattern}\"*)\n    echo x >> \"{log}\"\n    {body} ;;\nesac\nexec {real} \"$@\"\n",
            log = base.join("wrapper-calls").display(),
            real = real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    wait_until_runs(&script);
    script
}

#[test]
fn a_prikk_call_that_is_killed_or_times_out_fails_the_delivery_and_is_never_resumed() {
    need_prikk!();
    let fx = fixtures();
    for (name, body, reason) in [
        ("killed", "kill -9 $$", "unavailable"),
        ("timed-out", "exit 124", "timed_out"),
    ] {
        let base = scratch(&format!("killed-call-{name}"));
        create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
        let wrap = wrapper(&base, "archive import", body);
        let s = open_full(
            &base,
            None,
            planeter_prikk::Sandbox::Unconfined,
            Some(wrap),
            |_| {},
        );
        let before = tree_hash(&s.record.path);
        let r = deliver(&s, &fx.d1);
        let error = r
            .err()
            .unwrap_or_else(|| panic!("{name}: the delivery went through"));
        assert_eq!(
            error.reason(),
            if reason == "timed_out" {
                "timed_out"
            } else {
                "unavailable"
            },
            "{name}"
        );
        // the one delivery is failed with the closed word, its stage is gone, and the call was made once: never resumed
        let rows = s.deliveries.all().unwrap();
        assert_eq!(rows.len(), 1, "{name}");
        assert_eq!(
            (rows[0].state, rows[0].reason.as_deref()),
            (DeliveryState::Failed, Some(reason)),
            "{name}"
        );
        assert!(deliveries_dir_is_empty(&s), "{name}");
        assert_eq!(
            std::fs::read_to_string(base.join("wrapper-calls"))
                .unwrap()
                .lines()
                .count(),
            1,
            "{name}"
        );
        assert_eq!(tree_hash(&s.record.path), before, "{name}");
        assert!(s.deliveries.non_final().unwrap().is_empty());
        assert!(
            s.metrics
                .snapshot()
                .deliveries
                .get("failed")
                .copied()
                .unwrap_or(0)
                >= 1
        );
    }
}

/// A `prikk` that is the real one, except that from the `from`-th call whose first words are `pattern` it runs `body` instead.
fn counting_wrapper(base: &Path, pattern: &str, from: u32, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = base.join("prikk-counting.sh");
    let real = String::from_utf8(
        std::process::Command::new("which")
            .arg("prikk")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let count = base.join("counting");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"{pattern}\"*)\n    n=$(cat \"{c}\" 2>/dev/null || echo 0); n=$((n+1)); echo $n > \"{c}\"\n    if [ \"$n\" -ge {from} ]; then {body}; fi ;;\nesac\nexec {real} \"$@\"\n",
            c = count.display(),
            real = real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    wait_until_runs(&script);
    script
}

#[test]
fn a_reaccept_that_fails_stops_flags_the_repository_never_rolls_back_and_the_restart_finishes() {
    need_prikk!();
    let fx = fixtures();
    let base = scratch("reaccept-fails");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    // the scenario is built with the real prikk; the delivery runs over a prikk whose 2nd `sync accept` fails
    let sc = scenario(&open(&base, None), fx);
    let wrap = counting_wrapper(
        &base,
        "sync accept",
        2,
        "echo 'error: injected failure' >&2; exit 1",
    );
    let s = open_full(
        &base,
        None,
        planeter_prikk::Sandbox::Unconfined,
        Some(wrap),
        |_| {},
    );
    let (id, _) = deliver(&s, &sc.d).unwrap();
    let r = s.delivery.confirm(&alice(), &s.record, &id);
    assert_eq!(
        r.err(),
        Some(DeliveryError::Incomplete {
            delivery_id: id.clone()
        })
    );
    // not rolled back: the repository is the delivered one, the delivery stays `reaccepting`, the repository is flagged, the previous
    // generation is kept
    assert_eq!(s.live_tip(), sc.d_tip);
    assert_eq!(
        s.deliveries.get(&id).unwrap().unwrap().state,
        DeliveryState::Reaccepting
    );
    let flags = s.changes.repo_flags(&s.record.repo_id).unwrap();
    assert!(flags.needs_recovery);
    assert_eq!(
        flags.reason.as_deref(),
        Some("a write did not finish (delivery_unfinished)")
    );
    assert!(
        base.join("root/.deliveries")
            .join(&id)
            .join("previous")
            .is_dir()
    );
    // the flag is ours: the recovery service reads its cause as one of ours
    assert_eq!(
        planeter_transport::classify_cause(flags.reason.as_deref().unwrap()).code(),
        "delivery_unfinished"
    );
    // a new delivery is refused while it is unfinished
    assert!(matches!(
        s.delivery.start(&alice(), &s.record, 10),
        Err(DeliveryError::NeedsRecovery { .. })
    ));
    drop(s);
    // the restart, with a prikk that works, finishes it from its intent
    let s = open(&base, None);
    let r = s.delivery.reconcile_at_start();
    assert_eq!(r.completed, vec![id.clone()], "{r:?}");
    assert_eq!(s.pending(), sc.stays);
    assert_eq!(
        s.deliveries.get(&id).unwrap().unwrap().state,
        DeliveryState::Done
    );
    assert!(
        !s.changes
            .repo_flags(&s.record.repo_id)
            .unwrap()
            .needs_recovery
    );
    assert!(deliveries_dir_is_empty(&s));
    assert!(s.live().verify().unwrap().verdict.ok);
}

#[test]
fn a_delivered_repository_that_fails_publication_trust_is_failed_flagged_and_kept_for_an_administrator()
 {
    need_prikk!();
    let fx = fixtures();
    let base = scratch("publication-trust");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    // verify is called at the report, at the re-check, and after the install: the third answers a publication-trust failure
    let wrap = counting_wrapper(
        &base,
        "verify --format",
        3,
        r#"echo '{"schema_version":"verify-report-v1","verdict":{"ok":false,"failed_conditions":[{"id":"publication-trust","message":"x"}]},"active_sessions":{"count":0,"verified_count":0},"stages":[{"status":"evaluated","stage":"publication"}]}'; exit 1"#,
    );
    let s = open_full(
        &base,
        None,
        planeter_prikk::Sandbox::Unconfined,
        Some(wrap),
        |_| {},
    );
    let (id, report) = deliver(&s, &fx.d1).unwrap();
    assert!(report.is_acceptable());
    let r = s.delivery.confirm(&alice(), &s.record, &id);
    assert_eq!(
        r.err(),
        Some(DeliveryError::Failed("publication_trust".into()))
    );
    let row = s.deliveries.get(&id).unwrap().unwrap();
    assert_eq!(
        (row.state, row.reason.as_deref()),
        (DeliveryState::Failed, Some("publication_trust"))
    );
    assert!(
        s.changes
            .repo_flags(&s.record.repo_id)
            .unwrap()
            .needs_recovery
    );
    // kept: the previous generation is left where it is, and a restart does not remove it
    let prev = base.join("root/.deliveries").join(&id).join("previous");
    assert!(prev.is_dir());
    let r = s.delivery.reconcile_at_start();
    assert!(r.swept.is_empty(), "{r:?}");
    assert!(prev.is_dir());
}

#[test]
fn an_unfinished_install_is_completed_by_resume_without_a_restart_and_stray_directories_are_swept()
{
    need_prikk!();
    let fx = fixtures();
    let base = scratch("resume");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let sc = scenario(&open(&base, None), fx);
    let wrap = counting_wrapper(
        &base,
        "sync accept",
        1,
        "echo 'error: injected failure' >&2; exit 1",
    );
    let broken = open_full(
        &base,
        None,
        planeter_prikk::Sandbox::Unconfined,
        Some(wrap),
        |_| {},
    );
    let (id, _) = deliver(&broken, &sc.d).unwrap();
    assert!(
        broken
            .delivery
            .confirm(&alice(), &broken.record, &id)
            .is_err()
    );
    assert_eq!(
        broken.deliveries.get(&id).unwrap().unwrap().state,
        DeliveryState::Reaccepting
    );
    // an administrator clears the flag; the delivery is completed by `resume`
    let s = open(&base, None);
    s.changes.clear_needs_recovery(&s.record.repo_id).unwrap();
    assert_eq!(s.delivery.resume(&s.record).unwrap(), Some(id.clone()));
    assert_eq!(s.pending(), sc.stays);
    assert_eq!(
        s.delivery.resume(&s.record).unwrap(),
        None,
        "nothing is left to complete"
    );
    // directories no delivery owns are swept at start; what is not a delivery's name is left alone
    let root = base.join("root/.deliveries");
    let stray = root.join("dl-000000000000000000000000");
    std::fs::create_dir_all(stray.join("repo")).unwrap();
    std::fs::create_dir_all(root.join("not-ours")).unwrap();
    let r = s.delivery.reconcile_at_start();
    assert_eq!(
        r.swept,
        vec!["dl-000000000000000000000000".to_owned()],
        "{r:?}"
    );
    assert!(!stray.exists());
    assert!(root.join("not-ours").exists());
}

#[test]
fn a_repository_on_another_filesystem_is_refused_with_unavailable() {
    need_prikk!();
    let (s, _) = new_world("other-fs");
    let shm = std::path::Path::new("/dev/shm");
    let same = std::fs::metadata(shm)
        .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
        .ok()
        == std::fs::metadata(s.base.join("root"))
            .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
            .ok();
    if !shm.is_dir() || same {
        eprintln!("skipping: /dev/shm is missing or on the same filesystem");
        return;
    }
    let mut elsewhere = s.record.clone();
    elsewhere.path = shm.join("planeter-fs-check");
    assert_eq!(
        s.delivery.start(&alice(), &elsewhere, 10).err(),
        Some(DeliveryError::Unavailable)
    );
    assert!(
        s.deliveries.non_final().unwrap().is_empty(),
        "no delivery was begun"
    );
}

#[test]
fn the_stage_takes_the_live_directorys_mode_whatever_it_is() {
    need_prikk!();
    use std::os::unix::fs::PermissionsExt;
    let fx = fixtures();
    for mode in [0o750, 0o755, 0o700] {
        let (s, _) = new_world(&format!("mode-{mode:o}"));
        std::fs::set_permissions(&s.record.path, std::fs::Permissions::from_mode(mode)).unwrap();
        install(&s, &fx.d1);
        let got = std::os::unix::fs::MetadataExt::mode(&std::fs::metadata(&s.record.path).unwrap())
            & 0o7777;
        assert_eq!(got, mode, "{mode:o}");
        assert!(s.live().verify().unwrap().verdict.ok);
    }
}

#[test]
fn a_recovery_run_that_clears_the_flag_completes_the_delivery_without_a_restart() {
    need_prikk!();
    let fx = fixtures();
    let base = scratch("recovery-completes");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let sc = scenario(&open(&base, None), fx);
    let wrap = counting_wrapper(
        &base,
        "sync accept",
        2,
        "echo 'error: injected failure' >&2; exit 1",
    );
    let broken = open_full(
        &base,
        None,
        planeter_prikk::Sandbox::Unconfined,
        Some(wrap),
        |_| {},
    );
    let (id, _) = deliver(&broken, &sc.d).unwrap();
    assert!(
        broken
            .delivery
            .confirm(&alice(), &broken.record, &id)
            .is_err()
    );
    drop(broken);
    // the repository is flagged and the delivery is `reaccepting`; the administrator runs the recovery procedure (not a restart)
    let s = open(&base, None);
    assert_eq!(
        s.deliveries.get(&id).unwrap().unwrap().state,
        DeliveryState::Reaccepting
    );
    let result = s.recovery.recover(&s.record, "administrator", &[]).unwrap();
    assert_eq!(
        result.outcome,
        planeter_transport::RecoveryOutcome::Recovered
    );
    assert_eq!(
        s.deliveries.get(&id).unwrap().unwrap().state,
        DeliveryState::Done
    );
    assert_eq!(s.pending(), sc.stays);
    assert!(
        !s.changes
            .repo_flags(&s.record.repo_id)
            .unwrap()
            .needs_recovery
    );
    assert!(deliveries_dir_is_empty(&s));
    assert!(s.live().verify().unwrap().verdict.ok);
}
