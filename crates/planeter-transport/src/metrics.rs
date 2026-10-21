//! The transport's instrumentation (RFC 004 D-12): per fetch, per repository and per refusal reason. Counters
//! readable by tests and by a future metrics endpoint (none is added in this part); no repository name appears
//! except as the key of the per-repository table, which the access log already carries.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// How a fetch was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `200` or `206`: bytes were sent.
    Changed,
    /// `304`: the client's copy is current.
    Unchanged,
}

/// Where the artifact came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    /// Served from the spool without exporting.
    Hit,
    /// This request exported it.
    Miss,
    /// It waited for another request's export of the same key (single flight).
    Shared,
    /// No artifact was needed (a `304`, or a refusal).
    None,
}

/// What one fetch looked like.
#[derive(Debug, Clone)]
pub struct FetchRecord {
    pub kind: &'static str,
    pub outcome: Outcome,
    pub bytes: u64,
    pub wall: Duration,
    pub lease_wait: Duration,
    pub cache: CacheStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoCounters {
    /// Exports run for this repository.
    pub exports: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

impl RepoCounters {
    /// The share of artifact-needing fetches served without an export (0.0 when there were none).
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub fetches_changed: u64,
    pub fetches_unchanged: u64,
    pub by_kind: BTreeMap<String, u64>,
    pub bytes_served: u64,
    pub wall_total: Duration,
    pub lease_wait_total: Duration,
    pub per_repo: BTreeMap<String, RepoCounters>,
    pub refusals: BTreeMap<String, u64>,
}

/// The counters. Cheap to share (`Arc`); every method takes `&self`.
#[derive(Default)]
pub struct Metrics {
    inner: Mutex<Snapshot>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<T>(&self, f: impl FnOnce(&mut Snapshot) -> T) -> T {
        f(&mut self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// One fetch answered. Also writes the one structured log line (key=value; no query, no path, no body).
    pub fn fetch(&self, repo: &str, r: &FetchRecord) {
        self.with(|m| {
            match r.outcome {
                Outcome::Changed => m.fetches_changed += 1,
                Outcome::Unchanged => m.fetches_unchanged += 1,
            }
            *m.by_kind.entry(r.kind.to_owned()).or_insert(0) += 1;
            m.bytes_served += r.bytes;
            m.wall_total += r.wall;
            m.lease_wait_total += r.lease_wait;
            let c = m.per_repo.entry(repo.to_owned()).or_default();
            match r.cache {
                CacheStatus::Hit => c.cache_hits += 1,
                CacheStatus::Miss | CacheStatus::Shared => c.cache_misses += 1,
                CacheStatus::None => {}
            }
        });
        eprintln!(
            "planeter: transport fetch repo={repo} kind={} result={} cache={} bytes={} wall_ms={} lease_wait_ms={}",
            r.kind,
            match r.outcome {
                Outcome::Changed => "changed",
                Outcome::Unchanged => "unchanged",
            },
            match r.cache {
                CacheStatus::Hit => "hit",
                CacheStatus::Miss => "miss",
                CacheStatus::Shared => "shared",
                CacheStatus::None => "none",
            },
            r.bytes,
            r.wall.as_millis(),
            r.lease_wait.as_millis()
        );
    }

    /// One export run for `repo`.
    pub fn export(&self, repo: &str) {
        self.with(|m| m.per_repo.entry(repo.to_owned()).or_default().exports += 1);
    }

    /// One request refused, by the stable reason code (`busy`, `rate_limited`, `not_verified`, …).
    pub fn refusal(&self, reason: &str) {
        self.with(|m| *m.refusals.entry(reason.to_owned()).or_insert(0) += 1);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.with(|m| m.clone())
    }
}
