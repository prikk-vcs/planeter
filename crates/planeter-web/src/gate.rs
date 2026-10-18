//! The blocking gates (RFC 003 D-8, RFC 001 D-3, IS-4, IS-24; threat model T-9 / C-9b).
//!
//! Two instances of one mechanism: the **prikk gate** ([`PrikkGate`]) that every read-path call spawning
//! a sandboxed prikk goes through, and the **auth gate** ([`AuthGate`]) that every password
//! verification goes through. They are separate semaphores, so a flood of one cannot starve the other.
//! [`offload`] is the ungated sibling for short blocking work (SQLite lookups, token and session
//! checks) that must still stay off the async workers.
//!
//! Two problems, one choke point. The read service is synchronous and blocks until prikk exits (up to
//! the sandbox's 120 s wall-time bound); called straight from an `async fn` it pins a runtime worker, so
//! as many slow requests as there are workers stall the whole server. And nothing capped the number of
//! concurrent prikk subprocesses. [`PrikkGate::run`] fixes both: it acquires a permit from a
//! process-wide semaphore (bounded wait, then [`GateError::Busy`]) and only then moves the work onto
//! `spawn_blocking`.
//!
//! # The byte budget (RFC 011 D-1)
//!
//! The prikk gate also holds a **byte budget**, kept as a second semaphore in MiB units. A read declares
//! its estimated memory ([`BlockingGate::run_weighted`]); it acquires its byte permits **and** a process
//! slot within **one** bounded wait, and releases both when its job ends. A cost above the *whole*
//! budget is refused at once ([`AdmitError::TooLarge`]) and never queued; a wait that ends is
//! [`AdmitError::Busy`]. The auth gate has no budget and keeps using [`BlockingGate::run`].
//!
//! The semaphore is fair (FIFO, and a waiting request accumulates released permits), so a cheap read
//! queued behind one large waiter is delayed until that waiter is served or gives up — at most its own
//! bounded wait, because the larger waiter arrived first and so times out first. `tests/admission.rs`
//! measures it.
//!
//! The semaphore, **not** the blocking pool, is the limiter. The permit travels *into* the blocking job
//! and is released when the job ends: a request dropped while its prikk is running does not free a
//! slot early, because the subprocess is still running and the cap counts running processes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::budget::MIB;

/// The default ceiling on concurrent prikk invocations: `min(available_parallelism, 8)`.
pub fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(8)
}

/// The default bounded wait for a prikk slot, in seconds.
pub const DEFAULT_QUEUE_WAIT_SECS: u64 = 10;

/// The default ceiling on concurrent password verifications: `min(available_parallelism, 4)`.
pub fn default_auth_concurrency() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(4)
}

/// The default bounded wait for an authentication slot, in seconds.
pub const DEFAULT_AUTH_QUEUE_WAIT_SECS: u64 = 5;

/// The gate on read-path prikk invocations (the original use of [`BlockingGate`]).
pub type PrikkGate = BlockingGate;

/// The gate on password verification. A separate instance, never shared with the prikk gate.
pub type AuthGate = BlockingGate;

/// Run short blocking work on the blocking pool, **without** a permit. Used for work that is cheap and
/// bounded (a SQLite lookup, a token hash, a session read) but must not run on an async worker.
pub async fn offload<T, F>(job: F) -> Result<T, GateError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| GateError::Aborted)
}

/// Why a gated job did not produce a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    /// No slot became free within the bounded wait: refuse with `503` and `Retry-After`.
    Busy,
    /// The blocking job panicked or the runtime is shutting down.
    Aborted,
}

/// Why a weighted job was not admitted (RFC 011 D-1, D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitError {
    /// The job's estimated memory exceeds the **whole** budget: it can never fit, so it is refused at
    /// once (`too_large`) and never queued.
    TooLarge { cost: u64, budget: u64 },
    /// No budget and slot became available within the bounded wait: `503` with `Retry-After`.
    Busy,
    /// The blocking job panicked or the runtime is shutting down.
    Aborted,
}

/// The gate's counters (RFC 011 D-6), readable by tests and, later, by metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateStats {
    /// Weighted jobs admitted.
    pub admitted: u64,
    /// Jobs that could not be admitted at once and had to wait (served later, or refused `busy`).
    pub waited: u64,
    /// Jobs refused `busy` (the bounded wait ended).
    pub refused_busy: u64,
    /// Jobs refused `too_large` (above the whole budget).
    pub refused_too_large: u64,
    /// Bytes held by running jobs right now.
    pub in_flight_bytes: u64,
    /// The most bytes held at once since start.
    pub high_water_bytes: u64,
}

#[derive(Debug, Default)]
struct Counters {
    admitted: AtomicU64,
    waited: AtomicU64,
    refused_busy: AtomicU64,
    refused_too_large: AtomicU64,
    in_flight_bytes: AtomicU64,
    high_water_bytes: AtomicU64,
}

#[derive(Debug)]
struct ByteBudget {
    permits: Arc<Semaphore>,
    /// Whole MiB; the semaphore holds this many permits.
    units: u32,
}

/// What a running weighted job holds; released (permits and counter) when the job ends.
struct Held {
    _bytes: Option<OwnedSemaphorePermit>,
    _slot: OwnedSemaphorePermit,
    counters: Arc<Counters>,
    bytes: u64,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.counters
            .in_flight_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// A bounded gate on blocking prikk work. Cheap to share behind an `Arc`.
#[derive(Debug)]
pub struct BlockingGate {
    permits: Arc<Semaphore>,
    concurrency: usize,
    wait: Duration,
    rejected: AtomicU64,
    budget: Option<ByteBudget>,
    counters: Arc<Counters>,
}

impl Default for BlockingGate {
    fn default() -> Self {
        Self::new(
            default_concurrency(),
            Duration::from_secs(DEFAULT_QUEUE_WAIT_SECS),
        )
    }
}

impl BlockingGate {
    /// A gate allowing `concurrency` (at least 1) running jobs and waiting at most `wait` for a slot.
    pub fn new(concurrency: usize, wait: Duration) -> Self {
        let concurrency = concurrency.max(1);
        Self {
            permits: Arc::new(Semaphore::new(concurrency)),
            concurrency,
            wait,
            rejected: AtomicU64::new(0),
            budget: None,
            counters: Arc::new(Counters::default()),
        }
    }

    /// Give the gate a byte budget of `bytes` (in whole MiB, at least 1). Without one, weighted jobs are
    /// counted but never refused for size (tests and the auth gate); the server always sets one.
    #[must_use]
    pub fn with_budget(mut self, bytes: u64) -> Self {
        let units = u32::try_from((bytes / MIB).max(1)).unwrap_or(u32::MAX);
        self.budget = Some(ByteBudget {
            permits: Arc::new(Semaphore::new(units as usize)),
            units,
        });
        self
    }

    /// The whole budget in bytes, if one is set.
    pub fn budget_bytes(&self) -> Option<u64> {
        self.budget.as_ref().map(|b| u64::from(b.units) * MIB)
    }

    /// A snapshot of the counters.
    pub fn stats(&self) -> GateStats {
        let c = &self.counters;
        GateStats {
            admitted: c.admitted.load(Ordering::Relaxed),
            waited: c.waited.load(Ordering::Relaxed),
            refused_busy: c.refused_busy.load(Ordering::Relaxed),
            refused_too_large: c.refused_too_large.load(Ordering::Relaxed),
            in_flight_bytes: c.in_flight_bytes.load(Ordering::Relaxed),
            high_water_bytes: c.high_water_bytes.load(Ordering::Relaxed),
        }
    }

    /// The prikk gate from `PLANETER_PRIKK_CONCURRENCY` and `PLANETER_PRIKK_QUEUE_WAIT_SECS`, as given
    /// (`None` = unset → default). Each must be a positive integer; anything else is an error naming
    /// the variable.
    pub fn from_vars(
        concurrency: Option<&str>,
        queue_wait_secs: Option<&str>,
    ) -> Result<Self, String> {
        Self::parse(
            (
                "PLANETER_PRIKK_CONCURRENCY",
                concurrency,
                default_concurrency(),
            ),
            (
                "PLANETER_PRIKK_QUEUE_WAIT_SECS",
                queue_wait_secs,
                DEFAULT_QUEUE_WAIT_SECS,
            ),
        )
    }

    /// The auth gate from `PLANETER_AUTH_CONCURRENCY` and `PLANETER_AUTH_QUEUE_WAIT_SECS`.
    pub fn auth_from_vars(
        concurrency: Option<&str>,
        queue_wait_secs: Option<&str>,
    ) -> Result<Self, String> {
        Self::parse(
            (
                "PLANETER_AUTH_CONCURRENCY",
                concurrency,
                default_auth_concurrency(),
            ),
            (
                "PLANETER_AUTH_QUEUE_WAIT_SECS",
                queue_wait_secs,
                DEFAULT_AUTH_QUEUE_WAIT_SECS,
            ),
        )
    }

    /// A default-configured auth gate (`min(CPUs, 4)`, 5 s).
    pub fn auth_default() -> Self {
        Self::new(
            default_auth_concurrency(),
            Duration::from_secs(DEFAULT_AUTH_QUEUE_WAIT_SECS),
        )
    }

    fn parse(
        concurrency: (&str, Option<&str>, usize),
        wait: (&str, Option<&str>, u64),
    ) -> Result<Self, String> {
        let positive = |name: &str, v: Option<&str>, default: u64| -> Result<u64, String> {
            match v {
                None | Some("") => Ok(default),
                Some(s) => match s.trim().parse::<u64>() {
                    Ok(n) if n >= 1 => Ok(n),
                    _ => Err(format!("{name} must be a positive integer, got {s:?}")),
                },
            }
        };
        let c = positive(concurrency.0, concurrency.1, concurrency.2 as u64)?;
        let w = positive(wait.0, wait.1, wait.2)?;
        let c = usize::try_from(c).map_err(|_| format!("{} is too large", concurrency.0))?;
        Ok(Self::new(c, Duration::from_secs(w)))
    }

    /// The configured ceiling.
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// The bounded wait for a slot.
    pub fn wait(&self) -> Duration {
        self.wait
    }

    /// Slots free right now (a gauge for tests and diagnostics).
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// Requests refused as busy since start (by [`BlockingGate::run`] and [`BlockingGate::run_weighted`]).
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// Run `job` on the blocking pool once a slot is free. The wait is bounded; past it the request is
    /// refused ([`GateError::Busy`]) and counted — never dropped silently.
    pub async fn run<T, F>(&self, job: F) -> Result<T, GateError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let acquire = Arc::clone(&self.permits).acquire_owned();
        let permit = match tokio::time::timeout(self.wait, acquire).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_closed)) => return Err(GateError::Aborted),
            Err(_elapsed) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(GateError::Busy);
            }
        };
        tokio::task::spawn_blocking(move || {
            // Held until the job ends, whether or not anyone is still waiting for its result.
            let _permit = permit;
            job()
        })
        .await
        .map_err(|_| GateError::Aborted)
    }

    /// Take the byte permits and a slot without waiting; `None` if either is not free right now.
    fn try_hold(&self, units: u32, bytes: u64) -> Option<Held> {
        let byte_permit = match &self.budget {
            Some(b) => Some(Arc::clone(&b.permits).try_acquire_many_owned(units).ok()?),
            None => None,
        };
        let slot = Arc::clone(&self.permits).try_acquire_owned().ok()?;
        Some(self.held(byte_permit, slot, bytes))
    }

    /// Wait for the byte permits, then a slot (always in that order, so two waiters cannot deadlock).
    async fn hold(&self, units: u32, bytes: u64) -> Option<Held> {
        let byte_permit = match &self.budget {
            Some(b) => Some(
                Arc::clone(&b.permits)
                    .acquire_many_owned(units)
                    .await
                    .ok()?,
            ),
            None => None,
        };
        let slot = Arc::clone(&self.permits).acquire_owned().await.ok()?;
        Some(self.held(byte_permit, slot, bytes))
    }

    fn held(
        &self,
        byte_permit: Option<OwnedSemaphorePermit>,
        slot: OwnedSemaphorePermit,
        bytes: u64,
    ) -> Held {
        let c = &self.counters;
        let now = c.in_flight_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        c.high_water_bytes.fetch_max(now, Ordering::Relaxed);
        Held {
            _bytes: byte_permit,
            _slot: slot,
            counters: Arc::clone(c),
            bytes,
        }
    }

    /// Run `job` on the blocking pool once its estimated memory (`cost_bytes`) fits the budget **and** a
    /// slot is free — both within the one bounded wait. `what` names the read for the refusal log line (a
    /// verb; never a path or a repository name).
    ///
    /// * a cost above the whole budget → [`AdmitError::TooLarge`] at once, never queued;
    /// * the wait ends → [`AdmitError::Busy`];
    /// * both permits move into the blocking job and are released when it ends, so a request dropped
    ///   while its prikk is running does not free budget early (the subprocess still holds the memory).
    pub async fn run_weighted<T, F>(
        &self,
        cost_bytes: u64,
        what: &str,
        job: F,
    ) -> Result<T, AdmitError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let units64 = cost_bytes.div_ceil(MIB).max(1);
        let bytes = units64.saturating_mul(MIB);
        let budget = self.budget_bytes();
        if let Some(total) = budget
            && bytes > total
        {
            self.counters
                .refused_too_large
                .fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "planeter: a read was refused (too_large): verb={what} cost={cost_bytes} budget={total} bytes"
            );
            return Err(AdmitError::TooLarge {
                cost: cost_bytes,
                budget: total,
            });
        }
        let units = u32::try_from(units64).unwrap_or(u32::MAX);
        let held = match self.try_hold(units, bytes) {
            Some(h) => h,
            None => {
                self.counters.waited.fetch_add(1, Ordering::Relaxed);
                match tokio::time::timeout(self.wait, self.hold(units, bytes)).await {
                    Ok(Some(h)) => h,
                    Ok(None) => return Err(AdmitError::Aborted),
                    Err(_elapsed) => {
                        self.counters.refused_busy.fetch_add(1, Ordering::Relaxed);
                        self.rejected.fetch_add(1, Ordering::Relaxed);
                        eprintln!(
                            "planeter: a read was refused (busy): verb={what} cost={cost_bytes} budget={} bytes",
                            budget.map_or_else(|| "none".to_owned(), |b| b.to_string())
                        );
                        return Err(AdmitError::Busy);
                    }
                }
            }
        };
        self.counters.admitted.fetch_add(1, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || {
            // Held until the job ends, whether or not anyone is still waiting for its result.
            let _held = held;
            job()
        })
        .await
        .map_err(|_| AdmitError::Aborted)
    }
}
