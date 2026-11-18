//! RFC 008 handoff 1, T3/T4: **a kill at every step converges.** The engine runs in a child process (this test binary again) over
//! on-disk SQLite stores; at a named point it stops, and the parent SIGKILLs the child's whole process group (the engine, and any prikk
//! it was running). The parent then does what a restart does — opens the same directory with a new boot id, runs
//! `reconcile_at_start` — and holds the result against the table: **the new state with every open proposal intact, or the old state
//! with the stage discarded**; `verify` ok, `doctor` no error, no delivery in flight, no stale lock, no flag.

#[path = "support/archives.rs"]
mod archives;
#[path = "support/delivery_world.rs"]
mod world;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use planeter_prikk::PrikkRepo;
use planeter_store::{ChangeStore, DeliveryState, DeliveryStore};
use world::*;

macro_rules! need_prikk {
    () => {
        if !prikk_ok() {
            eprintln!("skipping: prikk is not on PATH");
            return;
        }
    };
}

// ---------------------------------------------------------------------------------------------------- the child

/// Not a test of its own: the child process of the others. It does nothing unless `DELIVERY_CHILD` names a point.
#[test]
fn child_entry() {
    let Ok(point) = std::env::var("DELIVERY_CHILD") else {
        return;
    };
    let base = PathBuf::from(std::env::var("DELIVERY_BASE").unwrap());
    let archive = PathBuf::from(std::env::var("DELIVERY_ARCHIVE").unwrap());
    let mark = PathBuf::from(std::env::var("DELIVERY_MARK").unwrap());
    let hook: Hook = {
        let point = point.clone();
        Arc::new(move |name: &str| {
            if name == point {
                std::fs::write(&mark, name).unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
        })
    };
    let s = open(&base, Some(hook));
    let (id, report) = deliver(&s, &archive).expect("the delivery is staged");
    if report.is_acceptable() {
        let _ = s.delivery.confirm(&alice(), &s.record, &id);
    }
    // a point that was never reached: the parent sees no mark and fails
}

fn spawn_child(base: &Path, archive: &Path, point: &str) -> (Child, PathBuf) {
    use std::os::unix::process::CommandExt;
    let mark = base.join("mark");
    let _ = std::fs::remove_file(&mark);
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_entry", "--nocapture", "--test-threads=1"])
        .env("DELIVERY_CHILD", point)
        .env("DELIVERY_BASE", base)
        .env("DELIVERY_ARCHIVE", archive)
        .env("DELIVERY_MARK", &mark)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    (child, mark)
}

/// SIGKILL the child and everything in its process group (a prikk it was running).
fn kill_group(child: &mut Child) {
    let pid = child.id();
    let _ = Command::new("kill")
        .args(["-9", "--", &format!("-{pid}")])
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_for(path: &Path, what: &str) {
    let end = Instant::now() + Duration::from_secs(240);
    while !path.exists() {
        assert!(Instant::now() < end, "the child never reached {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------------------------------- the checks

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum After {
    /// The old repository, the stage discarded.
    Old,
    /// The delivered repository, every open proposal intact.
    New,
}

fn world_for(name: &str) -> (PathBuf, Scenario) {
    let fx = fixtures();
    let base = scratch(name);
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let s = open(&base, None);
    let sc = scenario(&s, fx);
    drop(s);
    (base, sc)
}

fn converged(base: &Path, sc: &Scenario, want: After, old_tip: &str, old_hash: &str, point: &str) {
    let s = open(base, None);
    let after_kill: Vec<&'static str> = s
        .deliveries
        .all()
        .unwrap()
        .iter()
        .map(|r| r.state.as_str())
        .collect();
    let r = s.delivery.reconcile_at_start();
    let after_restart: Vec<&'static str> = s
        .deliveries
        .all()
        .unwrap()
        .iter()
        .map(|r| r.state.as_str())
        .collect();
    eprintln!(
        "KILLPOINT {point}: state after the kill {after_kill:?}; after the restart {after_restart:?}"
    );
    let counted = s.metrics.snapshot().deliveries;
    match want {
        After::Old => assert!(
            counted.get("abandoned").copied().unwrap_or(0) >= 1,
            "{point}: {counted:?}"
        ),
        After::New => assert_eq!(
            counted.get("recovered").copied(),
            Some(1),
            "{point}: {counted:?}"
        ),
    }
    // the tree first: later reads (verify, doctor) are allowed to touch files
    let hash_after = tree_hash(&s.record.path);
    let live = s.live();
    let ctx = format!("{point}: {r:?}");
    // nothing in flight, no directory, no flag, no stale lock
    assert!(s.deliveries.non_final().unwrap().is_empty(), "{ctx}");
    assert!(deliveries_dir_is_empty(&s), "{ctx}");
    assert!(
        !s.changes
            .repo_flags(&s.record.repo_id)
            .unwrap()
            .needs_recovery,
        "{ctx}"
    );
    assert!(live.unlock_list().unwrap().is_empty(), "stale lock: {ctx}");
    // every open proposal intact
    let mut want_open = vec![sc.p1.clone(), sc.p2.clone(), sc.p3.clone()];
    want_open.sort();
    assert_eq!(open_ids(&s), want_open, "{ctx}");
    assert!(live.verify().unwrap().verdict.ok, "{ctx}");
    assert_eq!(live.doctor_report().unwrap().errors, 0, "{ctx}");
    match want {
        After::Old => {
            assert_eq!(s.live_tip(), old_tip, "{ctx}");
            assert_eq!(hash_after, old_hash, "the old tree is untouched: {ctx}");
            let rows = s.deliveries.all().unwrap();
            assert!(
                rows.iter().all(|r| r.state != DeliveryState::Done),
                "{ctx}: {rows:?}"
            );
        }
        After::New => {
            assert_eq!(s.live_tip(), sc.d_tip, "{ctx}");
            assert_eq!(s.pending(), sc.stays, "{ctx}");
            let rows = s.deliveries.all().unwrap();
            assert!(
                rows.iter().any(|r| r.state == DeliveryState::Done),
                "{ctx}: {rows:?}"
            );
            assert!(r.completed.len() <= 1, "{ctx}");
        }
    }
    // and the repository accepts the next delivery: the whole road works again from here
    let again = if want == After::New { &sc.d2 } else { &sc.d };
    let (id, report) = deliver(&s, again).unwrap();
    assert!(report.is_acceptable(), "{ctx}: {report:?}");
    s.delivery.confirm(&alice(), &s.record, &id).unwrap();
    assert!(s.live().verify().unwrap().verdict.ok, "{ctx}");
}

fn kill_at(point: &str, want: After) {
    let (base, sc) = world_for(&format!("kill-{point}"));
    let (old_tip, old_hash) = {
        let s = open(&base, None);
        // a first read settles whatever prikk derives on first use; the tree is hashed after it
        let tip = s.live_tip();
        (tip, tree_hash(&s.record.path))
    };
    let (mut child, mark) = spawn_child(&base, &sc.d, point);
    wait_for(&mark, point);
    kill_group(&mut child);
    converged(&base, &sc, want, &old_tip, &old_hash, point);
}

// ---------------------------------------------------------------------------------------------------- the table

#[test]
fn a_kill_during_the_stage_import_leaves_the_old_repository() {
    need_prikk!();
    kill_at("stage_before_import", After::Old);
}

#[test]
fn a_kill_after_the_report_leaves_the_old_repository() {
    need_prikk!();
    kill_at("after_report", After::Old);
}

#[test]
fn a_kill_after_the_intent_leaves_the_old_repository() {
    need_prikk!();
    kill_at("after_intent", After::Old);
}

#[test]
fn a_kill_before_the_exchange_leaves_the_old_repository() {
    need_prikk!();
    kill_at("before_exchange", After::Old);
}

#[test]
fn a_kill_between_the_two_renames_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("between_renames", After::New);
}

#[test]
fn a_kill_after_the_exchange_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("after_exchange", After::New);
}

#[test]
fn a_kill_in_the_exchanged_state_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("exchanged", After::New);
}

#[test]
fn a_kill_before_the_first_reaccept_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("reaccept_first", After::New);
}

#[test]
fn a_kill_before_the_middle_reaccept_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("reaccept_middle", After::New);
}

#[test]
fn a_kill_before_the_last_reaccept_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("reaccept_last", After::New);
}

#[test]
fn a_kill_before_done_is_completed_to_the_new_repository() {
    need_prikk!();
    kill_at("before_done", After::New);
}

#[allow(dead_code)]
fn unused(_: &dyn ChangeStore) {}

// ---------------------------------------------------------------------------------------------------- the logs

/// A child that does what a server does with deliveries and prints its log: a branch added, a refusal, a damaged archive, an install.
#[test]
fn canary_child() {
    if std::env::var("DELIVERY_CANARY_CHILD").is_err() {
        return;
    }
    // the archives are the parent's (a child that built its own would have keys the repository never adopted)
    let archives: Vec<PathBuf> = std::env::var("DELIVERY_ARCHIVES")
        .unwrap()
        .split(':')
        .map(PathBuf::from)
        .collect();
    let base = PathBuf::from(std::env::var("DELIVERY_BASE").unwrap());
    let sc_d = PathBuf::from(std::env::var("DELIVERY_ARCHIVE").unwrap());
    let s = open(&base, None);
    for archive in &archives {
        let (id, report) = deliver(&s, archive).unwrap();
        if report.is_acceptable() {
            s.delivery.cancel(&alice(), &id).unwrap();
        }
    }
    let (id, report) = deliver(&s, &sc_d).unwrap();
    assert!(report.is_acceptable());
    s.delivery.confirm(&alice(), &s.record, &id).unwrap();
}

#[test]
fn the_log_names_no_file_branch_or_key_and_no_content() {
    need_prikk!();
    let fx = fixtures();
    let (base, sc) = world_for("canary");
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "canary_child", "--nocapture", "--test-threads=1"])
        .env("DELIVERY_CANARY_CHILD", "1")
        .env("DELIVERY_BASE", &base)
        .env("DELIVERY_ARCHIVE", &sc.d)
        .env(
            "DELIVERY_ARCHIVES",
            [&fx.dev, &fx.unrelated, &fx.truncated, &fx.x_archive]
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(":"),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    // it did log (the canary is not blind) …
    assert!(log.contains("acceptable ("), "{log}");
    assert!(log.contains("refused (unrelated)"), "{log}");
    assert!(log.contains("refused (damaged_archive)"), "{log}");
    assert!(log.contains("refused (unadopted_key)"), "{log}");
    assert!(log.contains("done ("), "{log}");
    // … and it says none of these
    let mut canaries: Vec<String> = [
        "src/",
        ".txt",
        "heads/dev",
        "heads/main",
        "change p",
        "change e",
        "content of",
        "ed25519-",
        "README",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    for k in [&fx.m1, &fx.m2, &fx.x] {
        canaries.push(k.key_id.clone());
        canaries.push(k.public_key.clone());
    }
    for c in &canaries {
        assert!(!log.contains(c.as_str()), "the log names {c:?}:\n{log}");
    }
}

// ---------------------------------------------------------------------------------------------------- a real kill of a real prikk

/// The mid-call kill: a revision of **1,000 patches** is accepted again while the process group is SIGKILLed — prikk dies holding its
/// lock. The restart clears that lock (`unlock --force`, after the witnesses agree), accepts everything again and finishes.
#[test]
fn a_kill_in_the_middle_of_a_reaccept_of_a_thousand_patches_leaves_a_stale_lock_the_restart_clears()
{
    need_prikk!();
    let fx = fixtures();
    let base = scratch("stale-lock");
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let s = open(&base, None);
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let at_h0 = archives::Maintainer {
        work: fx.m1_at_h0.clone(),
        keys: fx.m1_at_h0_keys.clone(),
    };
    // the proposal: 1,000 patches
    let started = Instant::now();
    let big = clone_of(&at_h0, &work, "big");
    big.history(1000);
    let artifact = artifact_of(
        &big,
        &s.record.path,
        &base.join("forge-keys"),
        "heads/main",
        "big",
    );
    let built = started.elapsed();
    let proposed = Instant::now();
    let p = s.propose_artifact("a thousand", &artifact, None);
    let propose_secs = proposed.elapsed();
    eprintln!(
        "1,000-patch revision: built in {built:?}, artifact {} bytes, first accepted in {propose_secs:?}",
        artifact.len()
    );
    // the delivery: two more blocks, none of the thousand
    let d = clone_of(&at_h0, &work, "d");
    step(&d, "e1");
    step(&d, "e2");
    let archive = work.join("delivery.archive");
    d.archive(&archive);
    let old_pending = s.pending();
    assert_eq!(old_pending.len(), 1000);
    let d_tip = blocks_of(&d)[0].clone();
    drop(s);

    let (mut child, _mark) = spawn_child(&base, &archive, "never");
    // wait for a lock in the live repository while the delivery is re-accepting
    let db = planeter_store::SqliteDb::open(base.join("state.db")).unwrap();
    let live_prikk = world::repo_path(&base).join(".prikk");
    let end = Instant::now() + Duration::from_secs(300);
    let mut seen = false;
    while Instant::now() < end {
        let reaccepting: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM deliveries WHERE state = 'reaccepting'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if reaccepting > 0 && has_lock(&live_prikk) {
            seen = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    kill_group(&mut child);
    assert!(
        seen,
        "the accept of the thousand patches was never caught holding its lock"
    );

    let s = open(&base, None);
    assert!(
        !s.live().unlock_list().unwrap().is_empty(),
        "the kill left a stale lock (the test is about it)"
    );
    let restarted = Instant::now();
    let r = s.delivery.reconcile_at_start();
    eprintln!(
        "restart completed the delivery in {:?}: {r:?}",
        restarted.elapsed()
    );
    assert_eq!(r.completed.len(), 1, "{r:?}");
    assert!(s.live().unlock_list().unwrap().is_empty(), "no stale lock");
    assert_eq!(s.live_tip(), d_tip);
    assert_eq!(
        s.pending(),
        old_pending,
        "the thousand patches are accepted again"
    );
    assert_eq!(open_ids(&s), vec![p.proposal_id.clone()]);
    assert!(s.live().verify().unwrap().verdict.ok);
    assert_eq!(s.live().doctor_report().unwrap().errors, 0);
    assert!(s.deliveries.non_final().unwrap().is_empty());
    assert!(
        !s.changes
            .repo_flags(&s.record.repo_id)
            .unwrap()
            .needs_recovery
    );
}

fn has_lock(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if has_lock(&p) {
                return true;
            }
        } else if p.extension().is_some_and(|x| x == "lock") {
            return true;
        }
    }
    false
}
