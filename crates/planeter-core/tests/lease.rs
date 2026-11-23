//! RFC 004 D-7: the repository lease. The matrix of class pairs, bounded waits, fairness, release on drop and on
//! panic, cancel-safety, a bounded table, and the durable holder record across a simulated restart.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use planeter_core::lease::{Lease, LeaseClass, LeaseError, LeaseManager, new_boot_id};
use planeter_store::{
    InMemoryLeaseRecords, LeaseRecordStore, RepoId, SqliteDb, SqliteLeaseRecords,
};

const R: LeaseClass = LeaseClass::Read;
const X: LeaseClass = LeaseClass::Export;
const W: LeaseClass = LeaseClass::Write;

fn manager() -> LeaseManager {
    LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        || 1_000,
        Duration::from_secs(3),
    )
}

fn repo(n: &str) -> RepoId {
    RepoId::new(n)
}

const SHORT: Duration = Duration::from_millis(60);

fn granted(m: &LeaseManager, r: &RepoId, c: LeaseClass, wait: Duration) -> bool {
    match m.acquire(r, c, "test", wait) {
        Ok(l) => {
            drop(l);
            true
        }
        Err(LeaseError::Busy { .. }) => false,
        Err(e) => panic!("{e}"),
    }
}

// ------------------------------------------------------------------------------------------------ the matrix

/// The RFC's table: R is allowed with R, X, W; X excludes W and X; W excludes X and W. Every ordered pair.
#[test]
fn the_full_matrix_of_class_pairs() {
    let expect = |first: LeaseClass, second: LeaseClass| first == R || second == R;
    for first in [R, X, W] {
        for second in [R, X, W] {
            let m = manager();
            let r = repo("r1");
            let _held = m.acquire(&r, first, "first", Duration::ZERO).unwrap();
            let got = granted(&m, &r, second, SHORT);
            assert_eq!(
                got,
                expect(first, second),
                "{} held, {} requested",
                first.letter(),
                second.letter()
            );
            assert_eq!(first.compatible_with(second), expect(first, second));
            // Another repository is never affected.
            assert!(granted(&m, &repo("r2"), second, Duration::ZERO));
        }
    }
}

#[test]
fn readers_do_not_exclude_each_other_or_the_exclusive_classes() {
    let m = manager();
    let r = repo("r1");
    let readers: Vec<Lease> = (0..50)
        .map(|_| m.acquire(&r, R, "read", Duration::ZERO).unwrap())
        .collect();
    // With 50 readers in, X and W are granted (one at a time), and with X in, readers are still granted.
    let x = m.acquire(&r, X, "export", Duration::ZERO).unwrap();
    assert!(granted(&m, &r, R, Duration::ZERO));
    drop(x);
    let w = m.acquire(&r, W, "write", Duration::ZERO).unwrap();
    assert!(granted(&m, &r, R, Duration::ZERO));
    assert_eq!(m.holder_of(&r), Some(W));
    drop(w);
    drop(readers);
    assert_eq!(m.table_len(), 0);
}

// ------------------------------------------------------------------------------------------------ bounded wait

#[test]
fn a_wait_is_bounded_and_ends_in_busy_with_a_retry_hint_and_the_holder_class() {
    let m = manager();
    let r = repo("r1");
    let _held = m.acquire(&r, W, "sync accept", Duration::ZERO).unwrap();
    let wait = Duration::from_millis(120);
    // The call runs on a thread of its own and the test waits for its answer with a guard no loaded host reaches: a wait that were not bounded
    // fails here instead of hanging the run. How long a bounded wait took says nothing about the bound on a host of unknown speed (the old
    // upper limit of the wait plus half a second did fail on a loaded one); that it **ends in Busy although the holder never lets go** does.
    let (tx, rx) = mpsc::channel();
    {
        let (m, r) = (m.clone(), r.clone());
        thread::spawn(move || {
            let t = Instant::now();
            let answer = m.acquire(&r, X, "archive export", wait);
            let _ = tx.send((answer.map(|_| ()), t.elapsed()));
        });
    }
    let (answer, took) = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("the wait ended: it is bounded although the holder never lets go");
    match answer {
        Err(LeaseError::Busy {
            retry_after,
            held_by,
        }) => {
            assert_eq!(retry_after, Duration::from_secs(3));
            assert_eq!(held_by, Some(W));
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    // a wait is a wait, and the least it can be is its bound: a lower bound cannot fail by load
    assert!(took >= wait, "returned early: {took:?}");
    // And the message names what to do, in a sentence the transport can turn into a 503.
    assert!(
        LeaseError::Busy {
            retry_after: Duration::from_secs(3),
            held_by: None
        }
        .to_string()
        .contains("try again in about 3 s")
    );
}

#[test]
fn a_waiter_gets_in_as_soon_as_the_holder_lets_go() {
    let m = manager();
    let r = repo("r1");
    let held = m.acquire(&r, W, "first", Duration::ZERO).unwrap();
    let (m2, r2) = (m.clone(), r.clone());
    // The waiter's bound is two minutes. What this holds is that **the release is what lets it in**, not the end of its bound: the answer
    // must be there long before the bound could end (a guard of 30 s, which no loaded host reaches), whereas a waiter woken only by its bound
    // would not answer for two minutes. (The old form timed the answer: under one second after a release.)
    let (tx, rx) = mpsc::channel();
    let waiter = thread::spawn(move || {
        let _ = tx.send(
            m2.acquire(&r2, W, "second", Duration::from_secs(120))
                .map(|l| l.class()),
        );
    });
    // A head start for the waiter to begin waiting. It is not an assertion about time: a waiter that has not begun when the holder lets go
    // simply takes the lease at once and the test holds less, it cannot fail by it. A writer is never let in beside a writer.
    thread::sleep(Duration::from_millis(100));
    assert!(
        rx.try_recv().is_err(),
        "the waiter was let in beside the holder"
    );
    drop(held);
    let answer = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the waiter was let in when the holder let go, not when its bound ended");
    assert_eq!(answer.unwrap(), W);
    waiter.join().unwrap();
}

// ------------------------------------------------------------------------------------------------ fairness

/// A waiting writer is served before an export that arrived later, even though the slot was "free" for a moment
/// when the first holder let go.
#[test]
fn a_waiting_writer_is_served_before_new_exports() {
    let m = manager();
    let r = repo("r1");
    let held = m.acquire(&r, X, "export 1", Duration::ZERO).unwrap();
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let mut handles = Vec::new();
    // The writer queues first; two exporters queue after it.
    for (name, class, delay) in [("W", W, 0u64), ("X2", X, 60), ("X3", X, 120)] {
        let (m, r, order) = (m.clone(), r.clone(), order.clone());
        handles.push(thread::spawn(move || {
            thread::sleep(Duration::from_millis(delay));
            let lease = m.acquire(&r, class, name, Duration::from_secs(10)).unwrap();
            order.lock().unwrap().push(name);
            thread::sleep(Duration::from_millis(30));
            drop(lease);
        }));
    }
    thread::sleep(Duration::from_millis(250)); // all three are in line
    drop(held);
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(*order.lock().unwrap(), ["W", "X2", "X3"]);
}

/// An export that asks while a writer is already waiting does not overtake it, even if it arrives the instant the
/// slot frees.
#[test]
fn a_new_export_does_not_take_a_free_slot_ahead_of_a_waiting_writer() {
    let m = manager();
    let r = repo("r1");
    for _ in 0..20 {
        let held = m.acquire(&r, W, "holder", Duration::ZERO).unwrap();
        let (tx, rx) = mpsc::channel::<&'static str>();
        let writer = {
            let (m, r, tx) = (m.clone(), r.clone(), tx.clone());
            thread::spawn(move || {
                let l = m.acquire(&r, W, "writer", Duration::from_secs(10)).unwrap();
                tx.send("W").unwrap();
                thread::sleep(Duration::from_millis(5));
                drop(l);
            })
        };
        thread::sleep(Duration::from_millis(30)); // the writer is waiting
        let exporter = {
            let (m, r, tx) = (m.clone(), r.clone(), tx.clone());
            thread::spawn(move || {
                let l = m
                    .acquire(&r, X, "exporter", Duration::from_secs(10))
                    .unwrap();
                tx.send("X").unwrap();
                drop(l);
            })
        };
        // The exporter asks at the very moment the holder lets go.
        drop(held);
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        writer.join().unwrap();
        exporter.join().unwrap();
        assert_eq!((first, second), ("W", "X"));
    }
}

/// Readers never queue, so a stream of them cannot keep a writer out.
#[test]
fn a_stream_of_readers_cannot_starve_a_writer() {
    let m = manager();
    let r = repo("r1");
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let (m, r, stop, reads) = (m.clone(), r.clone(), stop.clone(), reads.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let l = m.acquire(&r, R, "read", Duration::ZERO).unwrap();
                    reads.fetch_add(1, Ordering::Relaxed);
                    thread::yield_now();
                    drop(l);
                }
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(50));
    for _ in 0..20 {
        // the stream is running around every writer: it has made progress since the last one (a count of reads over a fixed time would
        // depend on how loaded the machine is; this does not)
        let seen = reads.load(Ordering::Relaxed);
        // **with no wait at all**: a writer is compatible with readers, so it is granted at once or the lease is wrong; how long a grant took
        // on this host says nothing about that (the old form timed it: under half a second)
        let w = m
            .acquire(&r, W, "write", Duration::ZERO)
            .expect("a writer gets in beside the readers, without waiting");
        drop(w);
        let end = Instant::now() + Duration::from_secs(20);
        while reads.load(Ordering::Relaxed) <= seen && Instant::now() < end {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            reads.load(Ordering::Relaxed) > seen,
            "the readers ran the whole time"
        );
    }
    stop.store(true, Ordering::Relaxed);
    for h in readers {
        h.join().unwrap();
    }
}

/// A writer waiting behind a long run of exports is served once the running one finishes, not after all of them.
#[test]
fn a_writer_is_not_kept_waiting_by_a_stream_of_exports() {
    let m = manager();
    let r = repo("r1");
    let running = m.acquire(&r, X, "export 0", Duration::ZERO).unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let mut handles = Vec::new();
    // Ten exports queue first…
    for i in 1..=10 {
        let (m, r, tx) = (m.clone(), r.clone(), tx.clone());
        handles.push(thread::spawn(move || {
            thread::sleep(Duration::from_millis(5 * i));
            let l = m.acquire(&r, X, "export", Duration::from_secs(20)).unwrap();
            tx.send(format!("X{i}")).unwrap();
            thread::sleep(Duration::from_millis(10));
            drop(l);
        }));
    }
    thread::sleep(Duration::from_millis(100));
    // …then a writer arrives. The exports already in line may not all run before it: the writer goes next.
    let writer = {
        let (m, r, tx) = (m.clone(), r.clone(), tx.clone());
        thread::spawn(move || {
            let l = m.acquire(&r, W, "writer", Duration::from_secs(20)).unwrap();
            tx.send("W".to_owned()).unwrap();
            drop(l);
        })
    };
    thread::sleep(Duration::from_millis(60));
    drop(running);
    drop(tx);
    let order: Vec<String> = rx.iter().collect();
    writer.join().unwrap();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(order.first().map(String::as_str), Some("W"), "{order:?}");
    assert_eq!(order.len(), 11);
}

// ------------------------------------------------------------------------------------------------ safety

#[test]
fn a_lease_is_released_on_drop_and_on_panic() {
    let m = manager();
    let r = repo("r1");
    // On drop.
    drop(m.acquire(&r, W, "w", Duration::ZERO).unwrap());
    assert!(granted(&m, &r, W, Duration::ZERO));
    // On panic: the holder's thread unwinds while holding the lease.
    let (m2, r2) = (m.clone(), r.clone());
    let result = thread::spawn(move || {
        let _lease = m2.acquire(&r2, W, "w", Duration::ZERO).unwrap();
        panic!("the holder died");
    })
    .join();
    assert!(result.is_err());
    assert!(
        granted(&m, &r, W, Duration::ZERO),
        "the lease must be free after a panic"
    );
    assert_eq!(m.table_len(), 0);
    // A panic while others wait does not strand them.
    let (m3, r3) = (m.clone(), r.clone());
    let waiter = {
        let held = m.acquire(&r, X, "x", Duration::ZERO).unwrap();
        let w = thread::spawn(move || m3.acquire(&r3, W, "w", Duration::from_secs(5)).is_ok());
        thread::sleep(Duration::from_millis(60));
        let (m4, r4) = (m.clone(), r.clone());
        // The holder is dropped inside a thread that then panics.
        let h = thread::spawn(move || {
            let _keep = (&m4, &r4);
            drop(held);
            panic!("after the release");
        });
        let _ = h.join();
        w
    };
    assert!(waiter.join().unwrap());
}

/// A waiter that gives up (a timeout, or a cancelled async caller) leaves the line and does not block the next.
#[test]
fn a_waiter_that_gives_up_does_not_block_the_one_behind_it() {
    let m = manager();
    let r = repo("r1");
    let held = m.acquire(&r, W, "holder", Duration::ZERO).unwrap();
    // The first waiter in line gives up quickly; the second waits long.
    let (m1, r1) = (m.clone(), r.clone());
    let quitter = thread::spawn(move || {
        m1.acquire(&r1, W, "quitter", Duration::from_millis(80))
            .is_err()
    });
    thread::sleep(Duration::from_millis(20));
    let (m2, r2) = (m.clone(), r.clone());
    let patient = thread::spawn(move || {
        m2.acquire(&r2, W, "patient", Duration::from_secs(5))
            .is_ok()
    });
    assert!(quitter.join().unwrap());
    thread::sleep(Duration::from_millis(40));
    drop(held);
    assert!(
        patient.join().unwrap(),
        "the waiter behind the one that quit must be served"
    );
    assert_eq!(m.table_len(), 0);
}

#[test]
fn the_table_does_not_grow_with_the_repositories_touched() {
    let m = manager();
    for i in 0..5_000 {
        let r = repo(&format!("repo-{i}"));
        let a = m.acquire(&r, R, "read", Duration::ZERO).unwrap();
        let b = m.acquire(&r, W, "write", Duration::ZERO).unwrap();
        assert!(m.try_acquire(&r, X, "export").is_err());
        drop(a);
        drop(b);
    }
    assert_eq!(m.table_len(), 0, "an idle manager holds no entries");
    // A timed-out waiter leaves nothing behind either.
    let r = repo("busy");
    let held = m.acquire(&r, W, "w", Duration::ZERO).unwrap();
    for _ in 0..100 {
        assert!(m.acquire(&r, W, "w", Duration::from_millis(1)).is_err());
    }
    assert_eq!(m.table_len(), 1);
    drop(held);
    assert_eq!(m.table_len(), 0);
}

/// Many threads, random classes, a few repositories: no two X/W holders of one repository ever overlap.
#[test]
fn exclusive_leases_never_overlap_under_contention() {
    let m = manager();
    let repos = ["a", "b", "c"].map(repo);
    let inside: Arc<Vec<AtomicUsize>> = Arc::new((0..3).map(|_| AtomicUsize::new(0)).collect());
    let violations = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(12));
    let threads: Vec<_> = (0..12)
        .map(|t| {
            let (m, repos, inside, violations, barrier) = (
                m.clone(),
                repos.clone(),
                inside.clone(),
                violations.clone(),
                barrier.clone(),
            );
            thread::spawn(move || {
                barrier.wait();
                let mut x: u64 = 0x9E3779B97F4A7C15 ^ (t as u64 + 1);
                for _ in 0..300 {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    let i = (x % 3) as usize;
                    let class = [R, X, W][((x >> 8) % 3) as usize];
                    if let Ok(l) = m.acquire(&repos[i], class, "stress", Duration::from_millis(200))
                    {
                        if class != R {
                            if inside[i].fetch_add(1, Ordering::SeqCst) != 0 {
                                violations.fetch_add(1, Ordering::SeqCst);
                            }
                            thread::sleep(Duration::from_micros(200));
                            inside[i].fetch_sub(1, Ordering::SeqCst);
                        }
                        drop(l);
                    }
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(
        violations.load(Ordering::SeqCst),
        0,
        "two exclusive holders overlapped"
    );
    assert_eq!(m.table_len(), 0);
}

// ------------------------------------------------------------------------------------------------ durability

#[test]
fn an_exclusive_grant_is_recorded_and_the_record_is_removed_on_release_and_a_read_leaves_none() {
    let records = Arc::new(InMemoryLeaseRecords::new());
    let m = LeaseManager::new(
        records.clone(),
        new_boot_id(),
        || 4_242,
        Duration::from_secs(1),
    );
    let r = repo("r1");
    let read = m.acquire(&r, R, "read", Duration::ZERO).unwrap();
    assert!(records.list().unwrap().is_empty(), "reads are not recorded");
    let w = m.acquire(&r, W, "sync accept", Duration::ZERO).unwrap();
    let rows = records.list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].class.as_str(),
            rows[0].operation.as_str(),
            rows[0].since_unix
        ),
        ("W", "sync accept", 4_242)
    );
    assert_eq!(rows[0].repo_id, r);
    assert!(
        m.orphans().unwrap().is_empty(),
        "this process's own holder is not an orphan"
    );
    drop(w);
    drop(read);
    assert!(records.list().unwrap().is_empty());
}

/// A server that dies holding a lease leaves its record; the next server sees a dead holder: who, since when,
/// doing what.
#[test]
fn the_durable_record_survives_a_simulated_restart() {
    let dir = std::env::temp_dir().join(format!("planeter-lease-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("planeter.db");
    // Boot 1: takes an X on one repository and a W on another, then "crashes" (no release runs).
    {
        let db = SqliteDb::open(&db_path).unwrap();
        let m = LeaseManager::new(
            Arc::new(SqliteLeaseRecords::new(db).unwrap()),
            "boot-one".to_owned(),
            || 111,
            Duration::from_secs(1),
        );
        std::mem::forget(
            m.acquire(&repo("alice-app"), W, "sync accept", Duration::ZERO)
                .unwrap(),
        );
        std::mem::forget(
            m.acquire(&repo("bob-lib"), X, "archive export", Duration::ZERO)
                .unwrap(),
        );
        // A clean holder on a third repository is gone before the crash and must leave nothing.
        drop(
            m.acquire(&repo("carol-x"), W, "seal", Duration::ZERO)
                .unwrap(),
        );
    }
    // Boot 2: the same database, a new process.
    let db = SqliteDb::open(&db_path).unwrap();
    let m = LeaseManager::new(
        Arc::new(SqliteLeaseRecords::new(db).unwrap()),
        "boot-two".to_owned(),
        || 222,
        Duration::from_secs(1),
    );
    let orphans = m.orphans().unwrap();
    assert_eq!(orphans.len(), 2, "{orphans:?}");
    let by_repo = |n: &str| orphans.iter().find(|o| o.repo_id == repo(n)).unwrap();
    assert_eq!(
        (
            by_repo("alice-app").class.as_str(),
            by_repo("alice-app").operation.as_str(),
            by_repo("alice-app").since_unix,
            by_repo("alice-app").boot_id.as_str()
        ),
        ("W", "sync accept", 111, "boot-one")
    );
    assert_eq!(by_repo("bob-lib").class, "X");
    // The in-memory table of the new process is empty: the orphans do not hold anything (part 4 decides).
    assert_eq!(m.table_len(), 0);
    assert!(granted(&m, &repo("alice-app"), W, Duration::ZERO));
    // This process's own grant is never listed as an orphan, and recovery can clear one.
    let own = m
        .acquire(&repo("dave"), W, "write", Duration::ZERO)
        .unwrap();
    assert_eq!(m.orphans().unwrap().len(), 2);
    drop(own);
    m.clear_orphan(&by_repo("alice-app").holder_id).unwrap();
    assert_eq!(m.orphans().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// If the record cannot be written, the lease is not granted and the slot is free again.
#[test]
fn a_failing_record_store_refuses_the_grant_and_frees_the_slot() {
    struct Failing;
    impl LeaseRecordStore for Failing {
        fn record(&self, _: &planeter_store::HolderRecord) -> planeter_store::store::Result<()> {
            Err(planeter_store::StoreError::Backend("disk full".to_owned()))
        }
        fn clear(&self, _: &str) -> planeter_store::store::Result<()> {
            Ok(())
        }
        fn list(&self) -> planeter_store::store::Result<Vec<planeter_store::HolderRecord>> {
            Ok(vec![])
        }
    }
    let m = LeaseManager::new(
        Arc::new(Failing),
        new_boot_id(),
        || 1,
        Duration::from_secs(1),
    );
    let r = repo("r1");
    assert!(matches!(
        m.acquire(&r, W, "w", Duration::ZERO),
        Err(LeaseError::Store(_))
    ));
    assert_eq!(m.table_len(), 0, "the slot was given back");
    // Reads need no record and still work.
    assert!(m.acquire(&r, R, "read", Duration::ZERO).is_ok());
}

/// A clean shutdown removes this process's own holder records and nobody else's.
#[test]
fn clear_own_records_removes_this_boots_records_only() {
    let records = Arc::new(InMemoryLeaseRecords::new());
    let old = LeaseManager::new(
        records.clone(),
        "boot-old".to_owned(),
        || 1,
        Duration::from_secs(1),
    );
    std::mem::forget(old.acquire(&repo("a"), W, "w", Duration::ZERO).unwrap());
    let m = LeaseManager::new(
        records.clone(),
        "boot-new".to_owned(),
        || 2,
        Duration::from_secs(1),
    );
    std::mem::forget(m.acquire(&repo("b"), X, "x", Duration::ZERO).unwrap());
    std::mem::forget(m.acquire(&repo("c"), W, "w", Duration::ZERO).unwrap());
    assert_eq!(records.list().unwrap().len(), 3);
    assert_eq!(m.clear_own_records().unwrap(), 2);
    let left = records.list().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(
        left[0].boot_id, "boot-old",
        "another process's record is not ours to remove"
    );
    assert_eq!(m.clear_own_records().unwrap(), 0);
}
