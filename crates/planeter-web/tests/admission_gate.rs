//! RFC 011 D-1, D-4, D-6: the memory-weighted gate and the budget configuration.
//!
//! Pure tests and gate tests with plain sleeping jobs — no prikk, nothing to skip.

use std::sync::Arc;
use std::time::{Duration, Instant};

use planeter_core::admission::{COST_FLOOR, LIGHT_CLASS_MAX_COST};
use planeter_web::budget::{
    BudgetSource, DEFAULT_CAP_BYTES, DEFAULT_FLOOR_BYTES, MIB, budget_from_var, default_budget,
    usable_memory,
};
use planeter_web::gate::{AdmitError, GateStats, Pool, PoolStats, PrikkGate};

fn sleep(ms: u64) -> impl FnOnce() + Send + 'static {
    move || std::thread::sleep(Duration::from_millis(ms))
}

/// A job that holds its permit **until the test lets go**, and says when it has started: the ordering a test is about is made of these two
/// events, not of how long a sleep lasts (a sleep that is "long enough" is short on a loaded host: IS-69).
struct Held {
    handle: tokio::task::JoinHandle<Result<(), AdmitError>>,
    release: std::sync::mpsc::Sender<()>,
}

impl Held {
    /// Let the job end, and wait for the gate's answer.
    async fn release(self) -> Result<(), AdmitError> {
        let _ = self.release.send(());
        self.handle.await.unwrap()
    }
}

/// Start a read of `cost` that holds until released, and return when its job is running (so the permit is certainly held).
async fn hold(g: &Arc<PrikkGate>, cost: u64, what: &'static str) -> Held {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let handle = {
        let g = Arc::clone(g);
        tokio::spawn(async move {
            g.run_weighted(cost, what, move || {
                let _ = started_tx.send(());
                // ends when released, or when the test has dropped its end (a test that fails must not leave the job blocked)
                let _ = release_rx.recv();
            })
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(120), started_rx)
        .await
        .expect("the held read never started")
        .expect("the held read was refused");
    Held {
        handle,
        release: release_tx,
    }
}

/// Wait (bounded by a guard no loaded host reaches) until `done` holds.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(120);
    while !done() {
        assert!(Instant::now() < end, "never: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
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

// ---------------------------------------------------------------- the two pools (RFC 011 R-1)

#[test]
fn the_budget_is_split_one_eighth_light_never_below_four_floors_never_above_half() {
    let size = |mib: u64| gate(mib, 10).pool_bytes().unwrap();
    // The usual case: an eighth, or four floors (64 MiB) if that is more.
    assert_eq!(size(4096), (512 * MIB, 3584 * MIB));
    assert_eq!(size(1024), (128 * MIB, 896 * MIB));
    assert_eq!(size(512), (64 * MIB, 448 * MIB));
    assert_eq!(size(256), (64 * MIB, 192 * MIB));
    assert_eq!(4 * COST_FLOOR, 64 * MIB);
    // A small explicit budget still leaves the heavy pool half of it.
    assert_eq!(size(64), (32 * MIB, 32 * MIB));
    assert_eq!(size(24), (12 * MIB, 12 * MIB));
    // The two always add up to the whole.
    let g = gate(1000, 10);
    let (l, h) = g.pool_bytes().unwrap();
    assert_eq!(g.budget_bytes(), Some(l + h));
    assert_eq!(g.stats().light.size_bytes, l);
    assert_eq!(g.stats().heavy.size_bytes, h);
    // No budget, no pools.
    assert_eq!(
        PrikkGate::new(8, Duration::from_millis(10)).pool_bytes(),
        None
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_draws_from_the_pool_of_its_cost_class_and_only_from_it() {
    let g = gate(1024, 1_000);
    // At the class boundary: light. One byte above: heavy.
    g.run_weighted(LIGHT_CLASS_MAX_COST, "show", sleep(1))
        .await
        .unwrap();
    g.run_weighted(LIGHT_CLASS_MAX_COST + 1, "tree", sleep(1))
        .await
        .unwrap();
    g.run_weighted(COST_FLOOR, "log", sleep(1)).await.unwrap();
    g.run_weighted(300 * MIB, "verify", sleep(1)).await.unwrap();
    let s = g.stats();
    assert_eq!(
        (s.light.high_water_bytes, s.heavy.high_water_bytes),
        (LIGHT_CLASS_MAX_COST, 300 * MIB),
        "{s:?}"
    );
    assert_eq!(s.high_water_bytes, 300 * MIB);

    // While a heavy read holds its pool, a light one is not affected, and vice versa. The heavy read holds until this test lets go, so
    // that "the light read was served" can only mean "served while the heavy one still held": a light read that had to wait for the heavy
    // pool would be refused busy at the end of its wait, which no host speed changes.
    let holder = hold(&g, 896 * MIB, "verify").await;
    let s = g.stats();
    assert_eq!(
        (s.heavy.in_flight_bytes, s.light.in_flight_bytes),
        (896 * MIB, 0)
    );
    g.run_weighted(COST_FLOOR, "log", sleep(1))
        .await
        .expect("a light read is served while the heavy pool is held");
    assert_eq!(
        g.stats().heavy.in_flight_bytes,
        896 * MIB,
        "the heavy read was still holding when the light one was served"
    );
    holder.release().await.unwrap();
    assert_eq!(g.stats().in_flight_bytes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_light_read_cannot_use_the_heavy_pool_nor_a_heavy_one_the_light_pool() {
    // More slots than holders, so only the byte pools are in play.
    let g = Arc::new(PrikkGate::new(16, Duration::from_millis(100)).with_budget(1024 * MIB)); // light 128, heavy 896
    // Fill the light pool with floor reads (held until released); another floor read waits and is refused — although the heavy
    // pool is empty: no borrowing between pools.
    let mut holders = Vec::new();
    for _ in 0..8 {
        holders.push(hold(&g, COST_FLOOR, "log").await);
    }
    assert_eq!(g.stats().light.in_flight_bytes, 128 * MIB);
    assert_eq!(
        g.run_weighted(COST_FLOOR, "log", sleep(1)).await,
        Err(AdmitError::Busy)
    );
    // …and a heavy read (100 MiB, light has none to lend) is unaffected.
    g.run_weighted(100 * MIB, "tree", sleep(1)).await.unwrap();
    for h in holders {
        h.release().await.unwrap();
    }

    // Conversely: the whole heavy pool is full of heavy reads; a heavy read waits, the light pool is idle.
    let holder = hold(&g, 896 * MIB, "verify").await;
    assert_eq!(
        g.run_weighted(100 * MIB, "tree", sleep(1)).await,
        Err(AdmitError::Busy),
        "a heavy read must not be served from the light pool"
    );
    holder.release().await.unwrap();
}

// ---------------------------------------------------------------- admission

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cost_above_its_own_pool_is_too_large_at_once_and_never_queued() {
    let g = gate(512, 10_000); // light 64, heavy 448; a 10 s wait that must NOT be used
    // Above the whole budget, and above the heavy pool: 449 MiB would fit the whole but not its own pool.
    for cost in [500 * MIB, 449 * MIB] {
        let r = g.run_weighted(cost, "verify", sleep(1)).await;
        assert_eq!(
            r,
            Err(AdmitError::TooLarge {
                cost,
                budget: 448 * MIB,
                pool: Pool::Heavy
            })
        );
    }
    // "Never queued" is the counter's to say: a read that waited, however briefly, is counted in `waited` (a stopwatch on a host that may be
    // slow could not tell a short queue from a slow host).
    let s = g.stats();
    assert_eq!((s.refused_too_large, s.admitted, s.waited), (2, 0, 0));
    // It does not even occupy the budget: the whole heavy pool is admitted at once.
    assert!(g.run_weighted(448 * MIB, "tree", sleep(1)).await.is_ok());

    // A floor-class read against a tiny budget whose light pool is below the class: own pool, too.
    let g = gate(32, 10_000); // light 16, heavy 16
    assert_eq!(
        g.run_weighted(LIGHT_CLASS_MAX_COST, "show", sleep(1)).await,
        Err(AdmitError::TooLarge {
            cost: LIGHT_CLASS_MAX_COST,
            budget: 16 * MIB,
            pool: Pool::Light
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_request_waits_for_budget_and_is_served_when_the_first_finishes() {
    let g = gate(1024, 120_000); // heavy 896; the wait is long: this test ends the wait by releasing A, never by waiting it out
    let a = hold(&g, 600 * MIB, "tree").await; // A holds 600 of 896
    assert_eq!(g.stats().in_flight_bytes, 600 * MIB);
    assert_eq!(g.stats().heavy.in_flight_bytes, 600 * MIB);
    let b = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(600 * MIB, "tree", sleep(1)).await })
    };
    // B is waiting (the gate counts it when it begins to wait) and is not served while A holds
    until("B to be waiting for the budget", || g.stats().waited == 1).await;
    assert!(
        !b.is_finished(),
        "B was served although A still holds the budget"
    );
    assert_eq!(g.stats().admitted, 1);
    // A lets go: B is served after A
    a.release().await.unwrap();
    b.await.unwrap().expect("served after A");
    let s = g.stats();
    assert_eq!((s.admitted, s.waited, s.refused_busy), (2, 1, 0));
    assert_eq!(s.in_flight_bytes, 0);
    assert_eq!(s.heavy.in_flight_bytes, 0);
    assert!(
        s.high_water_bytes <= 896 * MIB && s.high_water_bytes >= 600 * MIB,
        "{s:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_budget_is_released_on_completion_panic_cancellation_and_a_timed_out_wait() {
    let whole = 448 * MIB; // the whole heavy pool of a 512 MiB budget

    // completion
    let g = gate(512, 2_000);
    g.run_weighted(whole, "verify", sleep(5)).await.unwrap();
    assert_eq!(g.stats().in_flight_bytes, 0);
    g.run_weighted(whole, "verify", sleep(5))
        .await
        .expect("the whole pool is free again");

    // a panicking job
    let g = gate(512, 2_000);
    let r = g
        .run_weighted(whole, "verify", || -> () { panic!("boom") })
        .await;
    assert_eq!(r, Err(AdmitError::Aborted));
    g.run_weighted(whole, "verify", sleep(5))
        .await
        .expect("released after a panic");

    // cancellation: the request is dropped while its job runs. The subprocess still holds the memory, so
    // the budget stays held until the job ends — and is then released.
    let g = gate(512, 100);
    let running = hold(&g, whole, "verify").await;
    running.handle.abort(); // nobody is waiting for the result any more
    assert_eq!(
        g.run_weighted(whole, "verify", sleep(1)).await,
        Err(AdmitError::Busy),
        "released early: the job is still running"
    );
    let _ = running.release.send(()); // the job ends
    until("the budget to come back", || g.stats().in_flight_bytes == 0).await;
    g.run_weighted(whole, "verify", sleep(1))
        .await
        .expect("released once the job ended");

    // a wait that times out leaves nothing held
    let g = gate(512, 100);
    let holder = hold(&g, whole, "verify").await;
    assert_eq!(
        g.run_weighted(whole, "tree", sleep(1)).await,
        Err(AdmitError::Busy)
    );
    holder.release().await.unwrap();
    g.run_weighted(whole, "tree", sleep(1))
        .await
        .expect("no leak from the refused waiter");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_counters_say_what_happened() {
    let g = gate(512, 100); // light 64, heavy 448
    g.run_weighted(500 * MIB, "verify", sleep(1))
        .await
        .unwrap_err(); // too_large
    let holder = hold(&g, 448 * MIB, "verify").await;
    g.run_weighted(100 * MIB, "tree", sleep(1))
        .await
        .unwrap_err(); // busy after the wait
    holder.release().await.unwrap();
    g.run_weighted(16 * MIB, "log", sleep(1)).await.unwrap();
    assert_eq!(
        g.stats(),
        GateStats {
            admitted: 2,
            waited: 1,
            refused_busy: 1,
            refused_too_large: 1,
            in_flight_bytes: 0,
            high_water_bytes: 448 * MIB,
            light: PoolStats {
                size_bytes: 64 * MIB,
                in_flight_bytes: 0,
                high_water_bytes: 16 * MIB,
            },
            heavy: PoolStats {
                size_bytes: 448 * MIB,
                in_flight_bytes: 0,
                high_water_bytes: 448 * MIB,
            },
        }
    );
}

// ---------------------------------------------------------------- the FIFO risk, answered (R-1)

/// A `log` (16 MiB) arrives while a `verify` waits for budget behind a running one. With one FIFO the `log`
/// queued behind the `verify` (measured: it waited as long, and was refused `busy` in 16 of 200 races
/// though it fit). With the light pool the `log` is served at once, and the `verify` is still refused when
/// its own wait ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_light_read_never_waits_behind_a_waiting_heavy_one() {
    // Part one, the ordering: with a heavy read holding and a second heavy read waiting for it, a light read is served — **while the waiter
    // is still waiting and the holder still holds**. The wait is long, so that the waiter is waiting for the whole of this part whatever
    // the speed of the host; with one FIFO the light read would queue behind it and not be served until the waiter was.
    let g = gate(1000, 120_000); // light 125, heavy 875
    let a = hold(&g, 700 * MIB, "verify").await;
    let b = {
        let g = Arc::clone(&g);
        tokio::spawn(async move { g.run_weighted(700 * MIB, "verify", sleep(1)).await })
    };
    until("the second verify to be waiting", || g.stats().waited == 1).await;
    let c = g.run_weighted(16 * MIB, "log", sleep(1)).await;
    assert!(c.is_ok(), "the log was refused although it fits: {c:?}");
    assert!(
        !b.is_finished(),
        "the log was served only after the waiting verify: it waited behind it"
    );
    assert_eq!(
        g.stats().heavy.in_flight_bytes,
        700 * MIB,
        "the first verify was still holding when the log was served"
    );
    // The waiter gets its turn when the holder lets go.
    a.release().await.unwrap();
    b.await
        .unwrap()
        .expect("the waiting verify is served after the first");

    // Part two: the verify is still refused when its own wait ends. The holder never ends on its own, so the refusal is the end of the
    // wait and nothing else.
    let g = gate(1000, 100);
    let a = hold(&g, 700 * MIB, "verify").await;
    assert_eq!(
        g.run_weighted(700 * MIB, "verify", sleep(1)).await,
        Err(AdmitError::Busy)
    );
    assert!(g.run_weighted(16 * MIB, "log", sleep(1)).await.is_ok());
    a.release().await.unwrap();
}

/// The race of the measurement: the `log` arrives together with the waiting `verify`, so the two bounded
/// waits end together. With one FIFO the `log` was refused `busy` in 16 of 200 such races though it fit; it
/// is never refused while the light pool has room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_light_read_arriving_with_a_heavy_waiter_is_never_refused() {
    let mut races = Vec::new();
    for _ in 0..200 {
        races.push(tokio::spawn(async move {
            let g = gate(1000, 100);
            let a = {
                let g = Arc::clone(&g);
                tokio::spawn(async move { g.run_weighted(800 * MIB, "verify", sleep(400)).await })
            };
            tokio::time::sleep(Duration::from_millis(20)).await; // A is running
            let b = {
                let g = Arc::clone(&g);
                tokio::spawn(async move { g.run_weighted(800 * MIB, "verify", sleep(1)).await })
            };
            let c = {
                let g = Arc::clone(&g);
                tokio::spawn(async move { g.run_weighted(16 * MIB, "log", sleep(1)).await })
            };
            let c = c.await.unwrap();
            let _ = b.await;
            let _ = a.await;
            c
        }));
    }
    let mut refused = 0;
    for r in races {
        if r.await.unwrap().is_err() {
            refused += 1;
        }
    }
    assert_eq!(refused, 0, "{refused} of 200 logs were refused");
}
