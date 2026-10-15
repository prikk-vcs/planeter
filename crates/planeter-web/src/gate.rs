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
//! The semaphore, **not** the blocking pool, is the limiter. The permit travels *into* the blocking job
//! and is released when the job ends: a request dropped while its prikk is running does not free a
//! slot early, because the subprocess is still running and the cap counts running processes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::Semaphore;

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

/// A bounded gate on blocking prikk work. Cheap to share behind an `Arc`.
#[derive(Debug)]
pub struct BlockingGate {
    permits: Arc<Semaphore>,
    concurrency: usize,
    wait: Duration,
    rejected: AtomicU64,
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

    /// Requests refused with [`GateError::Busy`] since start.
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
}
