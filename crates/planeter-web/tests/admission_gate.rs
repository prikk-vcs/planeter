//! RFC 011 D-1, D-4, D-6: the memory-weighted gate and the budget configuration.
//!
//! Pure tests and gate tests with plain sleeping jobs — no prikk, nothing to skip.

use std::sync::Arc;
use std::time::{Duration, Instant};

use planeter_web::budget::{
    BudgetSource, DEFAULT_CAP_BYTES, DEFAULT_FLOOR_BYTES, MIB, budget_from_var, default_budget,
    usable_memory,
};
use planeter_web::gate::{AdmitError, GateStats, PrikkGate};

fn sleep(ms: u64) -> impl FnOnce() + Send + 'static {
    move || std::thread::sleep(Duration::from_millis(ms))
}

fn gate(budget_mib: u64, wait_ms: u64) -> Arc<PrikkGate> {
    Arc::new(PrikkGate::new(8, Duration::from_millis(wait_ms)).with_budget(budget_mib * MIB))
}

// ---------------------------------------------------------------- the budget's size

#[test]
fn the_default_is_a_quarter_floored_at_256_mib_and_capped_at_8_gib() {
    let gib = 1024 * MIB;
    assert_eq!(default_budget(16 * gib), (4 * gib, BudgetSource::Default));
    assert_eq!(default_budget(8 * gib), (2 * gib, BudgetSource::Default));
    // Floor.
    assert_eq!(
        default_budget(512 * MIB),
        (DEFAULT_FLOOR_BYTES, BudgetSource::DefaultFloored)
    );
    assert_eq!(DEFAULT_FLOOR_BYTES, 256 * MIB);
    // Cap, exactly at and above the boundary (a 32 GiB host is exactly the cap).
    assert_eq!(default_budget(32 * gib), (8 * gib, BudgetSource::Default));
    assert_eq!(
        default_budget(64 * gib),
        (DEFAULT_CAP_BYTES, BudgetSource::DefaultCapped)
    );
    assert_eq!(
        default_budget(1024 * gib),
        (DEFAULT_CAP_BYTES, BudgetSource::DefaultCapped)
    );
    assert_eq!(DEFAULT_CAP_BYTES, 8 * gib);
    assert_eq!(
        BudgetSource::DefaultCapped.label(),
        "default, capped at 8 GiB"
    );
    assert_eq!(BudgetSource::Default.label(), "default");
    assert_eq!(
        BudgetSource::Variable.label(),
        "PLANETER_PRIKK_MEMORY_BUDGET_MB"
    );
}

#[test]
fn an_explicit_value_is_never_capped_and_is_validated_like_its_siblings() {
    let host = 64 * 1024 * MIB;
    // Above the default cap, still taken as given.
    assert_eq!(
        budget_from_var(Some("65536"), host).unwrap(),
        (65536 * MIB, BudgetSource::Variable)
    );
    // Below the default floor, also as given (the operator chose it).
    assert_eq!(
        budget_from_var(Some("64"), host).unwrap(),
        (64 * MIB, BudgetSource::Variable)
    );
    // Unset or empty = the default.
    assert_eq!(
        budget_from_var(None, host).unwrap().1,
        BudgetSource::DefaultCapped
    );
    assert_eq!(
        budget_from_var(Some(""), host).unwrap().1,
        BudgetSource::DefaultCapped
    );
    for bad in ["0", "-1", "abc", "1.5", "12 MiB", "18446744073709551615"] {
        let e = budget_from_var(Some(bad), host).unwrap_err();
        assert!(e.contains("PLANETER_PRIKK_MEMORY_BUDGET_MB"), "{bad}: {e}");
    }
}

#[test]
fn usable_memory_is_the_cgroup_limit_when_it_is_a_number_else_memtotal() {
    let meminfo = "MemTotal:       16384000 kB\nMemFree:  100 kB\n";
    assert_eq!(usable_memory(None, meminfo), Some(16384000 * 1024));
    assert_eq!(usable_memory(Some("max\n"), meminfo), Some(16384000 * 1024));
    assert_eq!(
        usable_memory(Some("2147483648\n"), meminfo),
        Some(2147483648)
    );
    assert_eq!(
        usable_memory(Some("garbage"), meminfo),
        Some(16384000 * 1024)
    );
    assert_eq!(usable_memory(Some("max"), "no memtotal here"), None);
}

// ---------------------------------------------------------------- admission

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cost_above_the_whole_budget_is_too_large_at_once_and_never_queued() {
    let g = gate(64, 10_000); // a 10 s wait that must NOT be used
    let t = Instant::now();
    let r = g.run_weighted(100 * MIB, "verify", sleep(1)).await;
    assert_eq!(
        r,
        Err(AdmitError::TooLarge {
            cost: 100 * MIB,
            budget: 64 * MIB
        })
    );
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "queued: {:?}",
        t.elapsed()
    );
    let s = g.stats();
    assert_eq!((s.refused_too_large, s.admitted, s.waited), (1, 0, 0));
    // It does not even occupy the budget: a request that fits is admitted at once.
    assert!(g.run_weighted(64 * MIB, "tree", sleep(1)).await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_request_waits_for_budget_and_is_served_when_the_first_finishes() {
    let g = gate(64, 5_000);
    let a = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(48 * MIB, "tree", sleep(400)).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await; // A holds 48 of 64
    assert_eq!(g.stats().in_flight_bytes, 48 * MIB);
    let t = Instant::now();
    g.run_weighted(48 * MIB, "tree", sleep(10))
        .await
        .expect("served after A");
    let waited = t.elapsed();
    assert!(
        waited >= Duration::from_millis(300),
        "did not wait for budget: {waited:?}"
    );
    a.await.unwrap().unwrap();
    let s = g.stats();
    assert_eq!((s.admitted, s.waited, s.refused_busy), (2, 1, 0));
    assert_eq!(s.in_flight_bytes, 0);
    assert!(
        s.high_water_bytes <= 64 * MIB && s.high_water_bytes >= 48 * MIB,
        "{s:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_budget_is_released_on_completion_panic_cancellation_and_a_timed_out_wait() {
    let whole = 64 * MIB;

    // completion
    let g = gate(64, 2_000);
    g.run_weighted(whole, "verify", sleep(5)).await.unwrap();
    assert_eq!(g.stats().in_flight_bytes, 0);
    g.run_weighted(whole, "verify", sleep(5))
        .await
        .expect("the whole budget is free again");

    // a panicking job
    let g = gate(64, 2_000);
    let r = g
        .run_weighted(whole, "verify", || -> () { panic!("boom") })
        .await;
    assert_eq!(r, Err(AdmitError::Aborted));
    g.run_weighted(whole, "verify", sleep(5))
        .await
        .expect("released after a panic");

    // cancellation: the request is dropped while its job runs. The subprocess still holds the memory, so
    // the budget stays held until the job ends — and is then released.
    let g = gate(64, 100);
    let handle = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(whole, "verify", sleep(500)).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort(); // nobody is waiting for the result any more
    assert_eq!(
        g.run_weighted(whole, "verify", sleep(1)).await,
        Err(AdmitError::Busy),
        "released early: the job is still running"
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    g.run_weighted(whole, "verify", sleep(1))
        .await
        .expect("released once the job ended");

    // a wait that times out leaves nothing held
    let g = gate(64, 100);
    let holder = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(whole, "verify", sleep(400)).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        g.run_weighted(whole, "tree", sleep(1)).await,
        Err(AdmitError::Busy)
    );
    holder.await.unwrap().unwrap();
    g.run_weighted(whole, "tree", sleep(1))
        .await
        .expect("no leak from the refused waiter");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_counters_say_what_happened() {
    let g = gate(64, 100);
    g.run_weighted(100 * MIB, "verify", sleep(1))
        .await
        .unwrap_err(); // too_large
    let holder = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(64 * MIB, "verify", sleep(300)).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    g.run_weighted(16 * MIB, "log", sleep(1)).await.unwrap_err(); // busy after the wait
    holder.await.unwrap().unwrap();
    g.run_weighted(16 * MIB, "log", sleep(1)).await.unwrap();
    assert_eq!(
        g.stats(),
        GateStats {
            admitted: 2,
            waited: 1,
            refused_busy: 1,
            refused_too_large: 1,
            in_flight_bytes: 0,
            high_water_bytes: 64 * MIB,
        }
    );
}

// ---------------------------------------------------------------- the FIFO risk (T3)

/// A `log` (16 MiB) queued behind a `verify` (80 MiB of a 100 MiB budget) that is waiting for budget. The
/// semaphore is FIFO, so the cheap read waits for the large one to be served or to give up, **though 20 MiB
/// of budget is free throughout**. It never waits longer than its own bounded wait (the large waiter
/// arrived first and so times out first) — which is what this test pins. The race where both deadlines
/// coincide, and the refusal rate it causes, is measured in the review request, not asserted here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cheap_read_behind_a_waiting_large_one_is_delayed_but_never_beyond_its_own_wait() {
    // A runs 900 ms; the bounded wait is 500 ms, so B (the large waiter) gives up at ~520 ms.
    let g = gate(100, 500);
    let a = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(80 * MIB, "verify", sleep(900)).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let b = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(80 * MIB, "verify", sleep(1)).await })
    };
    tokio::time::sleep(Duration::from_millis(60)).await;
    let t = Instant::now();
    let c = g.run_weighted(16 * MIB, "log", sleep(1)).await;
    let c_wait = t.elapsed();

    assert_eq!(b.await.unwrap(), Err(AdmitError::Busy));
    assert!(c.is_ok(), "the log was refused although it fits: {c:?}");
    // Delayed: it had to wait for B to resolve (B's deadline, ~440 ms after C arrived)…
    assert!(
        c_wait >= Duration::from_millis(300),
        "not queued behind B? {c_wait:?}"
    );
    // …but not beyond its own bounded wait.
    assert!(c_wait <= Duration::from_millis(550), "{c_wait:?}");
    a.await.unwrap().unwrap();
    eprintln!("log behind a waiting verify: waited {c_wait:?} with 20 MiB of budget free");
}
