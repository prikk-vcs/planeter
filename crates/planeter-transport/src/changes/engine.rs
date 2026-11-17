//! The engine of the review view (RFC 005 part 6): from one revision's claims to its [`ChangeSet`] and, per file, its diff.
//!
//! * Two extractions, **one after the other** (never concurrently), under the one admission the caller holds: the foot claim's tree
//!   (`extract --claim`) and the tree of the root's real parent (`extract --ref <block>`; none when the revision starts a history).
//! * The comparison is planeter's own, over trees it never trusts ([`super::tree`], [`super::compare`]).
//! * A result is a pure function of content-addressed inputs and is kept in the spool ([`super::cache`]); a transient failure (busy,
//!   timed out, killed, an unreadable answer) is never kept.
//! * **Single flight**: [`ChangesService::flight`] makes concurrent requests for the same revision take turns, so the second finds the
//!   first's result in the cache instead of extracting again.
//! * No file name, path or content is ever written to a log line or a counter.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use planeter_core::HostingService;
use planeter_core::admission::ReadVerb;
use planeter_prikk::CallLimits;
use planeter_prikk::extract::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES, ExtractError, ExtractOptions, ExtractPoint,
    ExtractWorkdir, RefusalId,
};
use planeter_store::{RepositoryRecord, RevisionRecord};

use super::cache::{Cache, ListEntry, Remembered, RevEntry};
use super::chain::{ChainError, build_chain};
use super::compare::{CompareError, Want, compare};
use super::model::{ChangeSet, Compared, FileDiff};
use super::tree::{Tree, WalkError};
use crate::metrics::Metrics;

/// Why the view of a revision is not available: the closed list of the API's `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The revision's work is no longer pending here (it arrived in a received history, say).
    ClaimNotHeld,
    NotOneChange,
    TreeTooLarge,
    Busy,
    TimedOut,
    Failed,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::ClaimNotHeld => "claim_not_held",
            Reason::NotOneChange => "not_one_change",
            Reason::TreeTooLarge => "tree_too_large",
            Reason::Busy => "busy",
            Reason::TimedOut => "timed_out",
            Reason::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangesError {
    Unavailable(Reason),
    /// The revision has no changed file with this id.
    NoSuchFile,
}

#[derive(Debug, Clone)]
pub struct ChangesConfig {
    /// The spool (`PLANETER_SPOOL_DIR`): the cache lives in it. `None`: nothing is cached.
    pub spool: Option<PathBuf>,
    /// The whole computation's wall time.
    pub wall: Duration,
    /// The cache's size, past which the least recently used entries go.
    pub cache_budget: u64,
    /// The extraction bounds (the defaults of the extract driver; no new setting).
    pub max_bytes: u64,
    pub max_files: u64,
}

impl Default for ChangesConfig {
    fn default() -> Self {
        ChangesConfig {
            spool: None,
            wall: Duration::from_secs(30),
            cache_budget: 64 << 20,
            max_bytes: DEFAULT_MAX_BYTES,
            max_files: DEFAULT_MAX_FILES,
        }
    }
}

struct Inner {
    hosting: Arc<HostingService>,
    metrics: Arc<Metrics>,
    config: ChangesConfig,
    cache: Option<Cache>,
    /// Per revision: whether a view is being computed, and how many requests wait for it.
    flights: Mutex<HashMap<String, FlightState>>,
    turn: Condvar,
}

#[derive(Clone)]
pub struct ChangesService {
    inner: Arc<Inner>,
}

/// The most requests that wait for one revision's view (the one that computes it is not a waiter). The next is refused at once, without parking a thread.
pub const MAX_WAITERS: usize = 8;

#[derive(Default)]
struct FlightState {
    running: bool,
    waiters: usize,
}

/// More than [`MAX_WAITERS`] requests are already waiting for this revision's view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyWaiters;

/// Held while one revision's view is being computed; the next request for the same revision waits for it to drop.
pub struct Flight {
    inner: Arc<Inner>,
    key: String,
}

impl Drop for Flight {
    fn drop(&mut self) {
        let mut flights = self
            .inner
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(f) = flights.get_mut(&self.key) {
            f.running = false;
            if f.waiters == 0 {
                flights.remove(&self.key);
            }
        }
        self.inner.turn.notify_all();
    }
}

impl ChangesService {
    pub fn new(hosting: Arc<HostingService>, metrics: Arc<Metrics>, config: ChangesConfig) -> Self {
        let cache = config.spool.as_deref().and_then(|s| match Cache::open(s, config.cache_budget) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("planeter: the review view's cache could not be opened ({e}); change sets are computed per request");
                None
            }
        });
        ChangesService {
            inner: Arc::new(Inner {
                hosting,
                metrics,
                config,
                cache,
                flights: Mutex::new(HashMap::new()),
                turn: Condvar::new(),
            }),
        }
    }

    pub fn config(&self) -> &ChangesConfig {
        &self.inner.config
    }

    /// Whether answers are kept (a spool directory is set).
    pub fn caches(&self) -> bool {
        self.inner.cache.is_some()
    }

    /// The admission cost of one view: the cost of an `extract` at the configured bound (RFC 011). Both extractions run under it.
    pub fn cost(&self) -> u64 {
        ReadVerb::Extract.cost_with_bound(0, self.inner.config.max_bytes)
    }

    /// Wait for our turn on this revision. Blocks; call it on the blocking pool, before asking for memory. At most [`MAX_WAITERS`] requests wait for
    /// one revision: the next gets [`TooManyWaiters`] at once (counted as `busy`), so a burst of identical requests cannot park the blocking pool.
    pub fn flight(&self, revision_id: &str) -> Result<Flight, TooManyWaiters> {
        let mut flights = self
            .inner
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = flights.entry(revision_id.to_owned()).or_default();
        if entry.running {
            if entry.waiters >= MAX_WAITERS {
                return Err(TooManyWaiters);
            }
            entry.waiters += 1;
            loop {
                flights = self
                    .inner
                    .turn
                    .wait(flights)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !flights.get(revision_id).is_some_and(|f| f.running) {
                    break;
                }
            }
            if let Some(f) = flights.get_mut(revision_id) {
                f.waiters -= 1;
            }
        }
        flights.entry(revision_id.to_owned()).or_default().running = true;
        Ok(Flight {
            inner: Arc::clone(&self.inner),
            key: revision_id.to_owned(),
        })
    }

    fn count(&self, what: &str) {
        self.inner.metrics.changes(what);
    }

    fn fail(&self, reason: Reason) -> ChangesError {
        self.count(match reason {
            Reason::TimedOut => "timeout",
            other => other.code(),
        });
        ChangesError::Unavailable(reason)
    }

    /// What the cache already knows about the list of a revision: `None` when nothing (compute it).
    pub fn cached_list(&self, rev: &RevisionRecord) -> Option<Result<ChangeSet, ChangesError>> {
        let cache = self.inner.cache.as_ref()?;
        match cache.rev(&rev.revision_id)? {
            RevEntry::Failure(why) => Some(Err(self.remembered(why))),
            RevEntry::Key(key) => match cache.list(&key)? {
                ListEntry::Set(s) => {
                    self.count("cache_hit");
                    Some(Ok(*s))
                }
                ListEntry::Failure(why) => Some(Err(self.remembered(why))),
            },
        }
    }

    /// A file's diff, if the cache holds it. `Some(Err(NoSuchFile))` when the cached set has no such file.
    pub fn cached_file(
        &self,
        rev: &RevisionRecord,
        file_id: &str,
    ) -> Option<Result<FileDiff, ChangesError>> {
        let cache = self.inner.cache.as_ref()?;
        let key = match cache.rev(&rev.revision_id)? {
            RevEntry::Failure(why) => return Some(Err(self.remembered(why))),
            RevEntry::Key(key) => key,
        };
        match cache.list(&key)? {
            ListEntry::Set(set) => {
                if !set.files.iter().any(|f| f.file_id == file_id) {
                    return Some(Err(ChangesError::NoSuchFile));
                }
                let d = cache.file(&key, file_id)?;
                self.count("cache_hit");
                Some(Ok(d))
            }
            ListEntry::Failure(why) => Some(Err(self.remembered(why))),
        }
    }

    fn remembered(&self, why: Remembered) -> ChangesError {
        let reason = match why {
            Remembered::NotOneChange => Reason::NotOneChange,
            Remembered::TreeTooLarge => Reason::TreeTooLarge,
        };
        self.count("cache_hit");
        ChangesError::Unavailable(reason)
    }

    /// The change set of a revision, computed (and kept). Blocking; the caller holds the admission and the [`Flight`].
    pub fn list(
        &self,
        record: &RepositoryRecord,
        rev: &RevisionRecord,
        ceiling: Option<u64>,
    ) -> Result<ChangeSet, ChangesError> {
        if let Some(hit) = self.cached_list(rev) {
            return hit;
        }
        let run = self.compute(record, rev, ceiling, &Want::All)?;
        Ok(run.set)
    }

    /// One file's diff of a revision. Blocking; the caller holds the admission and the [`Flight`].
    pub fn file(
        &self,
        record: &RepositoryRecord,
        rev: &RevisionRecord,
        file_id: &str,
        ceiling: Option<u64>,
    ) -> Result<FileDiff, ChangesError> {
        if let Some(hit) = self.cached_file(rev, file_id) {
            return hit;
        }
        // not kept (no spool, evicted, or a set without it): compute just this file
        let run = self.compute(record, rev, ceiling, &Want::One(file_id))?;
        let diff = run.single.ok_or(ChangesError::NoSuchFile)?;
        if let (Some(cache), Some(key)) = (&self.inner.cache, &run.key) {
            cache.put_file(key, &diff);
        }
        Ok(diff)
    }

    fn compute(
        &self,
        record: &RepositoryRecord,
        rev: &RevisionRecord,
        ceiling: Option<u64>,
        want: &Want<'_>,
    ) -> Result<Run, ChangesError> {
        let started = Instant::now();
        let deadline = started + self.inner.config.wall;
        let remaining = |_: ()| -> u64 {
            deadline
                .saturating_duration_since(Instant::now())
                .as_secs()
                .max(1)
        };
        let limits = ceiling.map_or(CallLimits::NONE, CallLimits::data_ceiling);
        let driver = self
            .inner
            .hosting
            .open_record_with(record, limits)
            .map_err(|_| self.fail(Reason::Failed))?;
        let timed = |d: planeter_prikk::CliPrikkRepo| {
            let sandbox = d.sandbox().with_deadline(remaining(()));
            d.with_sandbox(sandbox)
        };

        // 1. the chain
        let pending = timed(driver.clone())
            .sync_pending_json()
            .map_err(|e| self.prikk_failure(&e))?;
        let chain = match build_chain(&rev.claim_ids, &pending) {
            Ok(c) => c,
            Err(ChainError::ClaimNotHeld) => return Err(self.fail(Reason::ClaimNotHeld)),
            Err(ChainError::NotOneChange) => {
                if let Some(cache) = &self.inner.cache {
                    cache.put_rev(
                        &rev.revision_id,
                        &RevEntry::Failure(Remembered::NotOneChange),
                    );
                }
                return Err(self.fail(Reason::NotOneChange));
            }
        };
        let key = format!(
            "{}-{}",
            super::cache::KEY_SCHEMA,
            &planeter_auth::sha256_hex(
                format!(
                    "{}\n{}",
                    chain.foot_claim,
                    chain.root_parent.as_deref().unwrap_or("-")
                )
                .as_bytes(),
            )[..40]
        );
        // another revision with the same claims may have been computed already
        if let Some(cache) = &self.inner.cache
            && let Some(entry) = cache.list(&key)
        {
            cache.put_rev(&rev.revision_id, &RevEntry::Key(key.clone()));
            match (entry, want) {
                (ListEntry::Failure(why), _) => return Err(self.remembered(why)),
                (ListEntry::Set(set), Want::All) => {
                    self.count("cache_hit");
                    return Ok(Run {
                        set: *set,
                        key: Some(key),
                        single: None,
                    });
                }
                (ListEntry::Set(_), Want::One(_)) => {}
            }
        }

        // 2. the two extractions, one after the other
        let parent = self.work_parent();
        let options = ExtractOptions {
            plan_only: false,
            max_bytes: Some(self.inner.config.max_bytes),
            max_files: Some(self.inner.config.max_files),
            allow_full_replay: true,
        };
        self.count("extractions");
        let after_dir = ExtractWorkdir::create(&parent, driver.root())
            .map_err(|_| self.fail(Reason::Failed))?;
        let after = timed(driver.clone())
            .extract(
                &ExtractPoint::Claim(chain.foot_claim.clone()),
                &after_dir,
                &options,
            )
            .map_err(|e| self.extract_failure(e, rev, &key))?;
        let before_dir;
        let mut provisional = after.report.provisional;
        let before_tree = match &chain.root_parent {
            None => Tree::empty(),
            Some(block) => {
                before_dir = ExtractWorkdir::create(&parent, driver.root())
                    .map_err(|_| self.fail(Reason::Failed))?;
                let before = timed(driver.clone())
                    .extract(&ExtractPoint::Block(block.clone()), &before_dir, &options)
                    .map_err(|e| self.extract_failure(e, rev, &key))?;
                provisional |= before.report.provisional;
                match before.tree {
                    Some(p) => self.walk(&p, rev, &key)?,
                    None => Tree::empty(),
                }
            }
        };
        let after_tree = match &after.tree {
            Some(p) => self.walk(p, rev, &key)?,
            None => Tree::empty(),
        };

        // 3. the comparison; the diffs go to the cache as they are made
        let writer = match (&self.inner.cache, want) {
            (Some(cache), Want::All) => cache.begin(&key),
            _ => None,
        };
        let mut single: Option<FileDiff> = None;
        let outcome = compare(&before_tree, &after_tree, want, deadline, &mut |d| {
            if let Some(w) = &writer {
                w.put(&d);
            }
            if matches!(want, Want::One(_)) {
                single = Some(d);
            }
        });
        let (files, totals) = match outcome {
            Ok(x) => x,
            Err(CompareError::TooMany) => return Err(self.too_large(rev, &key)),
            Err(CompareError::TimedOut) => return Err(self.fail(Reason::TimedOut)),
        };
        let declared = rev.declared_base.clone();
        let base_differs = match (&declared, &chain.root_parent) {
            (Some(d), Some(p)) => d != p,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let set = ChangeSet {
            compared: Compared {
                before_block: chain.root_parent.clone(),
                after_block: chain.foot_target.clone(),
                base_differs,
                declared_base: declared,
            },
            rebuilt: true,
            provisional,
            chain_signature: after.report.chain_signature.as_ref().map(signature_word),
            totals,
            files,
        };
        if matches!(want, Want::All)
            && let (Some(cache), Some(w)) = (&self.inner.cache, writer)
        {
            w.finish(&ListEntry::Set(Box::new(set.clone())));
            cache.put_rev(&rev.revision_id, &RevEntry::Key(key.clone()));
        }
        self.count("ok");
        Ok(Run {
            set,
            key: Some(key),
            single,
        })
    }

    fn work_parent(&self) -> PathBuf {
        let base = self
            .inner
            .config
            .spool
            .clone()
            .unwrap_or_else(std::env::temp_dir);
        let p = base.join("changes-work");
        let _ = std::fs::create_dir_all(&p);
        p
    }

    fn walk(
        &self,
        p: &std::path::Path,
        rev: &RevisionRecord,
        key: &str,
    ) -> Result<Tree, ChangesError> {
        Tree::walk(p).map_err(|e| match e {
            WalkError::TooMany => self.too_large(rev, key),
            WalkError::Io(_) => self.fail(Reason::Failed),
        })
    }

    fn too_large(&self, rev: &RevisionRecord, key: &str) -> ChangesError {
        if let Some(cache) = &self.inner.cache {
            cache.put_failure(key, Remembered::TreeTooLarge);
            cache.put_rev(&rev.revision_id, &RevEntry::Key(key.to_owned()));
        }
        self.fail(Reason::TreeTooLarge)
    }

    fn extract_failure(&self, e: ExtractError, rev: &RevisionRecord, key: &str) -> ChangesError {
        match e {
            ExtractError::Refused(r) => match r.refusal {
                RefusalId::OverMaxBytes | RefusalId::OverMaxFiles => self.too_large(rev, key),
                RefusalId::ClaimNotFound | RefusalId::NotAClaim => self.fail(Reason::ClaimNotHeld),
                _ => self.fail(Reason::Failed),
            },
            ExtractError::Prikk(p) => self.prikk_failure(&p),
            _ => self.fail(Reason::Failed),
        }
    }

    fn prikk_failure(&self, e: &planeter_prikk::PrikkError) -> ChangesError {
        // a call killed at its wall-time deadline reports the exit status `timeout` always did (124)
        let timed_out = matches!(
            e,
            planeter_prikk::PrikkError::Command {
                code: Some(planeter_prikk::sandbox::TIMEOUT_EXIT_CODE),
                ..
            }
        );
        self.fail(if timed_out {
            Reason::TimedOut
        } else {
            Reason::Failed
        })
    }

    /// A view refused for want of a slot or of memory: counted here, answered by the web layer.
    pub fn busy(&self) {
        self.count("busy");
    }
}

fn signature_word(s: &planeter_prikk::exchange::SignatureOutcome) -> String {
    use planeter_prikk::exchange::SignatureOutcome as S;
    match s {
        S::Sound => "sound".to_owned(),
        S::Unverifiable => "unverifiable".to_owned(),
        S::Invalid => "invalid".to_owned(),
        S::Other(o) => o.clone(),
    }
}

/// What a computation gives back.
struct Run {
    set: ChangeSet,
    key: Option<String>,
    single: Option<FileDiff>,
}
