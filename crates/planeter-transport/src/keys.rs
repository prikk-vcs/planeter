//! **The trusted maintainer keys of a repository, and how each came to be trusted** (RFC 013 D-4, the data; nothing here is shown to a user yet).
//!
//! The *set* of keys is prikk's own list (`trust maintainer list`): the truth. The table `repo_key_events` (planeter-store) says only *how* each
//! came: at creation, by a delivery, by the operator. [`repo_keys`] joins the two:
//!
//! * a key prikk lists and the table lacks is [`Source::NotRecorded`] (it was added outside planeter, with prikk directly): shown without a claim;
//! * a row for a key prikk does not list is **ignored** (never shown, never an error);
//! * the order is: the keys recorded at creation first, then the others by when they were recorded, then the keys not recorded.
//!
//! [`record_creation`] writes the events of a creation; [`KeyService`] reads under a read lease and runs the **one-time backfill** that gives every
//! existing repository its rows (the keys it trusts, as source `creation`, `backfilled`: in 0.8.0 the creation form was the only way a key got
//! there). **planeter never removes a key** (RFC 013 D-1): nothing here, and nothing anywhere in the workspace, can spawn `trust maintainer remove`.

use std::sync::Arc;

use planeter_core::{HostingService, LeaseClass, LeaseManager};
use planeter_prikk::PrikkRepo;
use planeter_prikk::model::TrustedKey;
use planeter_store::{
    ChangeStore, KeyEvent, KeyEventStore, KeySource, RepoId, RepositoryRecord, RepositoryStore,
};

use crate::changes::display::display_line;
use crate::metrics::Metrics;

/// Where one key came from, as far as planeter knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Adopted at the repository's creation. `backfilled`: the record was written later, when the table was first filled for a repository that
    /// already existed (the key was found trusted; in 0.8.0 only the creation form could put it there).
    Creation { backfilled: bool },
    /// Added by a maintainer's own delivery.
    Delivery { delivery_id: Option<String> },
    /// Added by the server's operator, with a reason (display-safe, at most 200 characters).
    Operator { reason: String },
    /// prikk lists the key and the table has no row for it: **no claim is made** about how it came.
    NotRecorded,
}

/// One trusted key of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoKey {
    pub key_id: String,
    /// 64 lowercase hexadecimal characters.
    pub public_key: String,
    /// The same key in groups of eight, as the delivery and creation print it.
    pub fingerprint: String,
    pub source: Source,
    /// When the table recorded the key (`None` for [`Source::NotRecorded`]).
    pub recorded_at: Option<u64>,
}

/// Why the keys could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeysError {
    /// prikk or the store could not answer (the text names no key).
    Unavailable(String),
}

impl std::fmt::Display for KeysError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeysError::Unavailable(why) => write!(f, "the trusted keys cannot be read: {why}"),
        }
    }
}

impl std::error::Error for KeysError {}

/// The display-safe, bounded form of an operator's reason: disguising characters are shown as visible escapes (the project's one display function,
/// `changes::display`), then it is cut to [`planeter_store::DETAIL_MAX_CHARS`] characters. **A long or hostile text is cut, never refused.**
pub fn detail_for_record(raw: &str) -> String {
    let (shown, _) = display_line(raw);
    KeyEvent::cut_detail(&shown)
}

/// Join the truth (`trust`, prikk's list) to the record (`events`). Pure.
pub fn join_keys(trust: &[TrustedKey], events: &[KeyEvent]) -> Vec<RepoKey> {
    let mut out: Vec<RepoKey> = trust
        .iter()
        .map(|k| {
            let event = events.iter().find(|e| e.public_key == k.public_key);
            let (source, recorded_at) = match event {
                None => (Source::NotRecorded, None),
                Some(e) => (
                    match e.source {
                        KeySource::Creation => Source::Creation {
                            backfilled: e.backfilled,
                        },
                        KeySource::Delivery => Source::Delivery {
                            delivery_id: e.delivery_id.clone(),
                        },
                        // read through the display function again: a row written by another path is never trusted to be safe
                        KeySource::Operator => Source::Operator {
                            reason: detail_for_record(&e.detail),
                        },
                    },
                    Some(e.recorded_at),
                ),
            };
            RepoKey {
                key_id: k.key_id.clone(),
                public_key: k.public_key.clone(),
                fingerprint: planeter_core::creation::group_fingerprint(&k.public_key),
                source,
                recorded_at,
            }
        })
        .collect();
    // creation first, then the others by when they were recorded, then the unrecorded; a stable order inside each
    out.sort_by(|a, b| {
        let rank = |k: &RepoKey| match k.source {
            Source::Creation { .. } => 0u8,
            Source::Delivery { .. } | Source::Operator { .. } => 1,
            Source::NotRecorded => 2,
        };
        (rank(a), a.recorded_at, &a.key_id).cmp(&(rank(b), b.recorded_at, &b.key_id))
    });
    out
}

/// The keys of `repo`: prikk's list through `driver`, joined to the table. (The caller holds the read lease: see [`KeyService::keys`].)
pub fn repo_keys(
    driver: &impl PrikkRepo,
    store: &dyn KeyEventStore,
    repo: &RepoId,
) -> Result<Vec<RepoKey>, KeysError> {
    let trust = driver
        .trust_list()
        .map_err(|e| KeysError::Unavailable(format!("prikk: {e}")))?;
    let events = store
        .events(repo)
        .map_err(|e| KeysError::Unavailable(format!("store: {e}")))?;
    Ok(join_keys(&trust.keys, &events))
}

/// Write the events of a creation: one per adopted key, source `creation`, not backfilled. **A failure is logged (no key named) and counted
/// (`key_events_creation_failed`); it is never returned**: the repository stays created and the backfill covers the keys. Returns how many rows
/// were written.
pub fn record_creation(
    store: &dyn KeyEventStore,
    metrics: &Metrics,
    repo: &RepoId,
    adopted: &[(String, String)],
    now: u64,
) -> usize {
    let mut written = 0;
    let mut failed = false;
    for (key_id, public_key) in adopted {
        let event = KeyEvent {
            repo_id: repo.clone(),
            key_id: key_id.clone(),
            public_key: public_key.clone(),
            source: KeySource::Creation,
            detail: String::new(),
            delivery_id: None,
            recorded_at: now,
            backfilled: false,
        };
        match store.record(&event) {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => {
                if !failed {
                    eprintln!(
                        "planeter: creation: the record of how the repository's keys came was not written ({e}); the start-up pass fills it in"
                    );
                }
                failed = true;
            }
        }
    }
    if failed {
        metrics.key_event("key_events_creation_failed");
    }
    written
}

/// What the backfill did with one repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backfilled {
    /// Its keys were recorded (`n` rows).
    Filled(usize),
    /// It already had rows: nothing was done (a repository that gained a key later keeps its partial rows and is not touched).
    AlreadyRecorded,
    /// prikk lists no key: there is nothing to record.
    NothingToRecord,
    /// It cannot be read now (flagged for recovery, an unreadable store, a failed write): retried at the next start.
    Skipped,
}

/// What a whole pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub filled: usize,
    pub skipped: usize,
    /// Repositories that needed nothing (already recorded, or no key to record).
    pub untouched: usize,
}

/// The reader and the backfill. Cheap to clone.
#[derive(Clone)]
pub struct KeyService {
    hosting: Arc<HostingService>,
    leases: Arc<LeaseManager>,
    store: Arc<dyn KeyEventStore>,
    repos: Arc<dyn RepositoryStore>,
    changes: Arc<dyn ChangeStore>,
    metrics: Arc<Metrics>,
    now_unix: fn() -> u64,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl KeyService {
    pub fn new(
        hosting: Arc<HostingService>,
        leases: Arc<LeaseManager>,
        store: Arc<dyn KeyEventStore>,
        repos: Arc<dyn RepositoryStore>,
        changes: Arc<dyn ChangeStore>,
        metrics: Arc<Metrics>,
    ) -> Self {
        KeyService {
            hosting,
            leases,
            store,
            repos,
            changes,
            metrics,
            now_unix: system_now,
        }
    }

    /// Test seam: the clock.
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// The memory ceiling of a call that reads the trust list (a few KB of output): the light class.
    pub fn read_ceiling(&self) -> planeter_prikk::CallLimits {
        planeter_prikk::CallLimits::data_ceiling(planeter_core::admission::ceiling_for(
            planeter_core::admission::COST_FLOOR,
            None,
        ))
    }

    /// The keys of `record`: **under a read lease**, through the sandboxed driver. Blocking.
    pub fn keys(&self, record: &RepositoryRecord) -> Result<Vec<RepoKey>, KeysError> {
        let lease = self
            .leases
            .acquire(
                &record.repo_id,
                LeaseClass::Read,
                "trust maintainer list",
                std::time::Duration::from_secs(5),
            )
            .map_err(|e| KeysError::Unavailable(format!("lease: {e}")))?;
        let driver = self
            .hosting
            .open_record_with(record, self.read_ceiling())
            .map_err(|e| KeysError::Unavailable(format!("open: {e}")))?;
        let keys = repo_keys(&driver, &*self.store, &record.repo_id);
        drop(lease);
        keys
    }

    /// The ready repositories that have **no event row at all**, in a stable order: what the backfill has to look at.
    pub fn unrecorded(&self) -> Vec<RepositoryRecord> {
        let mut ids = match self.repos.known_ids() {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("planeter: key records: cannot list the repositories: {e}");
                return Vec::new();
            }
        };
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.into_iter()
            // `get` answers for ready records only: a repository being created is invisible
            .filter_map(|id| self.repos.get(&id).ok().flatten())
            .filter(|r| !self.store.has_any(&r.repo_id).unwrap_or(true))
            .collect()
    }

    /// Fill one repository: read its trust list and record each listed key as source `creation`, `backfilled`, in one all-or-none step. **Never
    /// changes the repository.** Blocking; the caller admits it (the server runs it through the heavy pool's gate).
    pub fn backfill_one(&self, record: &RepositoryRecord) -> Backfilled {
        let repo = &record.repo_id;
        if self
            .changes
            .repo_flags(repo)
            .map(|f| f.needs_recovery)
            .unwrap_or(true)
        {
            return Backfilled::Skipped;
        }
        // the table may have been filled since the list was made (a creation, a second pass): do nothing then
        if self.store.has_any(repo).unwrap_or(true) {
            return Backfilled::AlreadyRecorded;
        }
        let Ok(lease) = self.leases.acquire(
            repo,
            LeaseClass::Read,
            "trust maintainer list",
            std::time::Duration::from_secs(5),
        ) else {
            return Backfilled::Skipped;
        };
        let trust = self
            .hosting
            .open_record_with(record, self.read_ceiling())
            .ok()
            .and_then(|d| d.trust_list().ok());
        drop(lease);
        let Some(trust) = trust else {
            return Backfilled::Skipped;
        };
        if trust.keys.is_empty() {
            return Backfilled::NothingToRecord;
        }
        let now = (self.now_unix)();
        let events: Vec<KeyEvent> = trust
            .keys
            .iter()
            .map(|k| KeyEvent {
                repo_id: repo.clone(),
                key_id: k.key_id.clone(),
                public_key: k.public_key.clone(),
                source: KeySource::Creation,
                detail: String::new(),
                delivery_id: None,
                recorded_at: now,
                backfilled: true,
            })
            .collect();
        match self.store.record_all_if_none(repo, &events) {
            Ok(true) => Backfilled::Filled(events.len()),
            Ok(false) => Backfilled::AlreadyRecorded,
            Err(_) => Backfilled::Skipped,
        }
    }

    /// Count what a pass did and log **one line with counts only** (no repository, no key): the stop counters get `key_events_backfilled` and
    /// `key_events_skipped`.
    pub fn finish_pass(&self, outcomes: &[Backfilled]) -> BackfillReport {
        let mut r = BackfillReport::default();
        for o in outcomes {
            match o {
                Backfilled::Filled(_) => r.filled += 1,
                Backfilled::Skipped => r.skipped += 1,
                Backfilled::AlreadyRecorded | Backfilled::NothingToRecord => r.untouched += 1,
            }
        }
        for _ in 0..r.filled {
            self.metrics.key_event("key_events_backfilled");
        }
        for _ in 0..r.skipped {
            self.metrics.key_event("key_events_skipped");
        }
        eprintln!(
            "planeter: key records: {} repositories filled, {} skipped",
            r.filled, r.skipped
        );
        r
    }

    /// The whole pass without any admission (tests, tools): [`Self::unrecorded`], [`Self::backfill_one`] for each, [`Self::finish_pass`]. The server runs
    /// the same steps with each `backfill_one` admitted by the heavy pool.
    pub fn backfill_all(&self) -> BackfillReport {
        let outcomes: Vec<Backfilled> = self
            .unrecorded()
            .iter()
            .map(|r| self.backfill_one(r))
            .collect();
        self.finish_pass(&outcomes)
    }
}
