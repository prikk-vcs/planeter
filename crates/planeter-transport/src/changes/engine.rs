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
use planeter_core::admission::{ReadVerb, ceiling_for, store_size};
use planeter_core::change::{Landing, observe_landing};
use planeter_prikk::extract::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES, ExtractError, ExtractOptions, ExtractPoint,
    ExtractWorkdir, RefusalId,
};
use planeter_prikk::{CallLimits, PrikkRepo};
use planeter_store::{RepositoryRecord, RevisionRecord};

use super::cache::{Cache, ListEntry, Remembered, RevEntry};
use super::chain::{ChainError, build_chain};
use super::compare::{CompareError, Want, compare};
use super::model::{ChangeSet, Compared, FileDiff};
use super::tree::{Tree, WalkError};
use crate::fingerprint::files_digest;
use crate::metrics::Metrics;
use crate::proposals::{FOLLOWED_REF, OBSERVATION_BLOCKS_DEFAULT, log_blocks};

/// Why the view of a revision is not available: the closed list of the API's `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The revision's work is no longer pending here (it arrived in a received history, say).
    ClaimNotHeld,
    /// Some of the revision's work has landed in the project's branch and some is still waiting here (RFC 008 D-11): it cannot be shown as one change.
    PartlyLanded,
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
            Reason::PartlyLanded => "partly_landed",
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
    /// How many blocks of the project's branch the landing observation reads (`PLANETER_OBSERVATION_BLOCKS`, the proposals' own setting): the same
    /// window, so a revision that landed longer ago than it is answered the same "unknown" way (`claim_not_held`), never as a landed view.
    pub observation_blocks: usize,
}

impl Default for ChangesConfig {
    fn default() -> Self {
        ChangesConfig {
            spool: None,
            wall: Duration::from_secs(30),
            cache_budget: 64 << 20,
            max_bytes: DEFAULT_MAX_BYTES,
            max_files: DEFAULT_MAX_FILES,
            observation_blocks: OBSERVATION_BLOCKS_DEFAULT,
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
    /// What the repository's history says about revisions, once per state of the repository (the files' digest): see [`Facts`].
    facts: Mutex<HashMap<String, Arc<Facts>>>,
}

/// The most repositories whose facts are kept (a small memo; the oldest go first).
const FACTS_KEPT: usize = 64;

/// What is read of a repository once per state of its files: the claims that are **pending** (the revisions whose claims are in this set are
/// shown from their claims, as before) and, read when somebody needs it, the blocks of the **project's branch** (a revision that is not pending
/// may have landed in it). Both are prikk's own words, read with a flat read's ceiling, never per request.
struct Facts {
    digest: String,
    pending_claims: std::collections::BTreeSet<String>,
    followed: Mutex<Option<Arc<Followed>>>,
}

/// The blocks of [`FOLLOWED_REF`] within the observation's window, and whether the window cut the log short; `None` blocks: the repository has
/// no such branch.
struct Followed {
    blocks: Option<Vec<planeter_prikk::model::LogBlock>>,
    truncated: bool,
}

/// What a revision is, from the facts: shown from its pending claims, from the project's branch, or not at all.
enum Status {
    Pending,
    Landed(LandedRun),
    PartlyLanded,
    ClaimNotHeld,
    NotOneChange,
}

/// The one run of consecutive blocks of the project's branch that carries a landed revision's patches: the block before it (`None`: it starts
/// the history) and the last block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandedRun {
    pub before: Option<String>,
    pub after: String,
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
                facts: Mutex::new(HashMap::new()),
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

    // ------------------------------------------------------------------------------------------------ what the history says

    /// A driver for the observation's reads: the memory ceiling of a flat read of the repository (the proposals' own), and the engine's wall time.
    fn reader(
        &self,
        record: &RepositoryRecord,
    ) -> Result<planeter_prikk::CliPrikkRepo, ChangesError> {
        const FLOOR: u64 = 64 << 20;
        let size = store_size(&record.path).map_err(|_| self.fail(Reason::Failed))?;
        let ceiling = ceiling_for(ReadVerb::Log.cost(size).max(FLOOR), None);
        let d = self
            .inner
            .hosting
            .open_record_with(record, CallLimits::data_ceiling(ceiling))
            .map_err(|_| self.fail(Reason::Failed))?;
        let sandbox = d
            .sandbox()
            .with_deadline(self.inner.config.wall.as_secs().max(1));
        Ok(d.with_sandbox(sandbox))
    }

    /// The facts of the repository's present state: `sync pending` once per state of its files (the digest of their lengths and times, the cache
    /// directory and the locks apart), shared by every revision of the repository.
    fn facts(&self, record: &RepositoryRecord) -> Result<Arc<Facts>, ChangesError> {
        let digest =
            files_digest(&record.path.join(".prikk")).map_err(|_| self.fail(Reason::Failed))?;
        if let Some(f) = self
            .inner
            .facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(record.repo_id.as_str())
            && f.digest == digest
        {
            return Ok(Arc::clone(f));
        }
        let pending = self
            .reader(record)?
            .sync_pending_json()
            .map_err(|e| self.prikk_failure(&e))?;
        let facts = Arc::new(Facts {
            digest,
            pending_claims: pending
                .patches
                .iter()
                .flat_map(|p| p.claims.iter().map(|c| c.claim_id.clone()))
                .collect(),
            followed: Mutex::new(None),
        });
        let mut memo = self
            .inner
            .facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if memo.len() >= FACTS_KEPT && !memo.contains_key(record.repo_id.as_str()) {
            memo.clear();
        }
        memo.insert(record.repo_id.as_str().to_owned(), Arc::clone(&facts));
        Ok(facts)
    }

    /// The blocks of the project's branch, read once per state of the repository and only when a revision that is not pending needs them.
    fn followed(
        &self,
        record: &RepositoryRecord,
        facts: &Facts,
    ) -> Result<Arc<Followed>, ChangesError> {
        let mut slot = facts
            .followed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(f) = &*slot {
            return Ok(Arc::clone(f));
        }
        let driver = self.reader(record)?;
        let has_ref = driver
            .branches(true)
            .map_err(|e| self.prikk_failure(&e))?
            .branches
            .iter()
            .any(|b| b.ref_name == FOLLOWED_REF);
        let followed = if has_ref {
            let (blocks, truncated) =
                log_blocks(&driver, FOLLOWED_REF, self.inner.config.observation_blocks)
                    .map_err(|e| self.prikk_failure(&e))?;
            Followed {
                blocks: Some(blocks),
                truncated,
            }
        } else {
            Followed {
                blocks: None,
                truncated: false,
            }
        };
        let followed = Arc::new(followed);
        *slot = Some(Arc::clone(&followed));
        Ok(followed)
    }

    /// What a revision is **now**: shown from its claims while they are all still pending here; otherwise asked of the project's branch, with the
    /// landing observation's own function ([`observe_landing`]) over the blocks the proposals' own reading gives ([`log_blocks`]).
    ///
    /// A revision's patches are the ids it **declared** (what its artifact contains, held already or not) together with the ids it **added**
    /// (what its accept newly stored): a push by plain prikk declares nothing, a revision that adds nothing new (everything was held already)
    /// has only its declaration; a revision with neither has nothing to ask the history about, and its answer is `claim_not_held`.
    fn status(
        &self,
        record: &RepositoryRecord,
        rev: &RevisionRecord,
    ) -> Result<Status, ChangesError> {
        let facts = self.facts(record)?;
        if !rev.claim_ids.is_empty()
            && rev
                .claim_ids
                .iter()
                .all(|c| facts.pending_claims.contains(c))
        {
            return Ok(Status::Pending);
        }
        let set: std::collections::BTreeSet<String> = rev
            .declared_patch_ids
            .iter()
            .flatten()
            .chain(rev.added_patch_ids.iter())
            .cloned()
            .collect();
        if set.is_empty() {
            return Ok(Status::ClaimNotHeld);
        }
        let followed = self.followed(record, &facts)?;
        let Some(blocks) = &followed.blocks else {
            return Ok(Status::ClaimNotHeld);
        };
        let in_branch: std::collections::BTreeSet<String> = blocks
            .iter()
            .flat_map(|b| b.patch_messages.iter().map(|p| p.patch_id.clone()))
            .collect();
        Ok(match observe_landing(&set, &in_branch) {
            Landing::Landed => match landed_run(&set, blocks) {
                Some(run) => Status::Landed(run),
                None => Status::NotOneChange,
            },
            // None of it is in the branch (the work left the queue another way, or the artifact is gone): the old answer. Some of it is, and the
            // rest may be older than the window: that cannot be told from "waiting here", so it is not claimed either.
            Landing::NotYet { remaining } if remaining.len() == set.len() || followed.truncated => {
                Status::ClaimNotHeld
            }
            Landing::NotYet { .. } => Status::PartlyLanded,
        })
    }

    /// What is already known about the list of a revision without extracting anything: a cached change set, or an answer the facts decide
    /// (`partly_landed`, `claim_not_held`, `not_one_change`). `None`: compute it. **Which cache entry may be read depends on what the revision is
    /// now** (`Self::status`): a revision that has landed is looked for under the key of the two blocks of the project's branch, never under
    /// the pointer written while its claims were pending.
    pub fn cached_list(
        &self,
        record: &RepositoryRecord,
        rev: &RevisionRecord,
    ) -> Option<Result<ChangeSet, ChangesError>> {
        match self.status(record, rev) {
            Err(e) => Some(Err(e)),
            // a work that is in no block of the project's branch and no longer waiting: what was cached while it waited is still right (a pure
            // function of immutable inputs), and is served as in 0.7.0; nothing cached: the answer is `claim_not_held`
            Ok(Status::Pending | Status::ClaimNotHeld) => {
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
            Ok(Status::Landed(run)) => {
                let cache = self.inner.cache.as_ref()?;
                match cache.list(&landed_key(&run))? {
                    ListEntry::Set(s) => {
                        self.count("cache_hit");
                        Some(Ok(*s))
                    }
                    ListEntry::Failure(why) => Some(Err(self.remembered(why))),
                }
            }
            Ok(other) => Some(Err(self.decided(other))),
        }
    }

    /// A file's diff, if the cache holds it. `Some(Err(NoSuchFile))` when the cached set has no such file.
    pub fn cached_file(
        &self,
        record: &RepositoryRecord,
        rev: &RevisionRecord,
        file_id: &str,
    ) -> Option<Result<FileDiff, ChangesError>> {
        let key = match self.status(record, rev) {
            Err(e) => return Some(Err(e)),
            Ok(Status::Pending | Status::ClaimNotHeld) => {
                let cache = self.inner.cache.as_ref()?;
                match cache.rev(&rev.revision_id)? {
                    RevEntry::Failure(why) => return Some(Err(self.remembered(why))),
                    RevEntry::Key(key) => key,
                }
            }
            Ok(Status::Landed(run)) => landed_key(&run),
            Ok(other) => return Some(Err(self.decided(other))),
        };
        let cache = self.inner.cache.as_ref()?;
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

    /// The error of a status that is not shown from files.
    fn decided(&self, status: Status) -> ChangesError {
        match status {
            Status::PartlyLanded => self.fail(Reason::PartlyLanded),
            Status::NotOneChange => self.fail(Reason::NotOneChange),
            _ => self.fail(Reason::ClaimNotHeld),
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
        if let Some(hit) = self.cached_list(record, rev) {
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
        if let Some(hit) = self.cached_file(record, rev, file_id) {
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

        // 1. what the revision is now: its pending claims (a chain), or the project's branch (a landed run)
        let plan = match self.status(record, rev)? {
            Status::Pending => {
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
                Plan {
                    after_point: ExtractPoint::Claim(chain.foot_claim.clone()),
                    after_block: chain.foot_target.clone(),
                    before: chain.root_parent.clone(),
                    key,
                    landed: false,
                }
            }
            Status::Landed(run) => Plan {
                after_point: ExtractPoint::Block(run.after.clone()),
                key: landed_key(&run),
                after_block: run.after,
                before: run.before,
                landed: true,
            },
            other => return Err(self.decided(other)),
        };
        let key = plan.key.clone();
        // another revision with the same claims (or the same two blocks) may have been computed already
        if let Some(cache) = &self.inner.cache
            && let Some(entry) = cache.list(&key)
        {
            if !plan.landed {
                cache.put_rev(&rev.revision_id, &RevEntry::Key(key.clone()));
            }
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
            .extract(&plan.after_point, &after_dir, &options)
            .map_err(|e| self.extract_failure(e, rev, &key))?;
        let before_dir;
        let mut provisional = after.report.provisional;
        let before_tree = match &plan.before {
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
        // For a landed revision the parent is whatever the maintainer's sealed history says (the patches were sealed again, on another base): the
        // base the submitter declared is not a claim about it, so it is not compared.
        let base_differs = match (&declared, &plan.before) {
            _ if plan.landed => false,
            (Some(d), Some(p)) => d != p,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let set = ChangeSet {
            compared: Compared {
                before_block: plan.before.clone(),
                after_block: plan.after_block.clone(),
                base_differs,
                declared_base: declared,
                landed: plan.landed,
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
            if !plan.landed {
                cache.put_rev(&rev.revision_id, &RevEntry::Key(key.clone()));
            }
        }
        self.count(if plan.landed { "landed_ok" } else { "ok" });
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

/// What one computation extracts and compares: the "after" point and block, the "before" block, the cache key, and whether it is a landed revision.
struct Plan {
    after_point: ExtractPoint,
    after_block: String,
    before: Option<String>,
    key: String,
    landed: bool,
}

/// The cache key of a landed revision's change set: the two blocks of the project's branch, which are content-addressed and never change, so an
/// entry is right for ever (a later delivery cannot make it stale). A different prefix from a pending revision's key (which is made of a claim).
fn landed_key(run: &LandedRun) -> String {
    format!(
        "{}-{}",
        super::cache::KEY_SCHEMA,
        &planeter_auth::sha256_hex(
            format!(
                "landed\n{}\n{}",
                run.after,
                run.before.as_deref().unwrap_or("-")
            )
            .as_bytes(),
        )[..40]
    )
}

/// The one run of consecutive blocks of the project's branch that carries **exactly** the patches `set`, or `None` when they do not form one.
///
/// * a patch that is in two blocks, a block that carries a patch of another revision (or any other patch), a merge, a block whose parent count and
///   parent ids disagree, several feet, or blocks that do not follow one another by their **first parent**: not one run;
/// * otherwise the last block is `after`, and `before` is the first parent of the oldest (`None`: it has none, the revision starts the history).
pub fn landed_run(
    set: &std::collections::BTreeSet<String>,
    blocks: &[planeter_prikk::model::LogBlock],
) -> Option<LandedRun> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut carrying: BTreeMap<&str, &planeter_prikk::model::LogBlock> = BTreeMap::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for b in blocks {
        let mine = b
            .patch_messages
            .iter()
            .filter(|p| set.contains(&p.patch_id))
            .count();
        if mine == 0 {
            continue;
        }
        // a block that carries something else as well is not "this revision's change"
        if mine != b.patch_messages.len() {
            return None;
        }
        // a repeated patch (in two blocks, or twice in one) is not a single sequence
        for p in &b.patch_messages {
            if !seen.insert(p.patch_id.as_str()) {
                return None;
            }
        }
        if carrying.insert(b.block_id.as_str(), b).is_some() {
            return None;
        }
        if b.parent_count > 1 || b.parent_block_ids.len() as u64 != b.parent_count {
            return None;
        }
    }
    if carrying.is_empty() {
        return None;
    }
    fn first_parent(b: &planeter_prikk::model::LogBlock) -> Option<&str> {
        b.parent_block_ids.first().map(String::as_str)
    }
    let parents: BTreeSet<&str> = carrying.values().filter_map(|b| first_parent(b)).collect();
    let feet: Vec<&str> = carrying
        .keys()
        .copied()
        .filter(|id| !parents.contains(id))
        .collect();
    let [foot] = feet.as_slice() else {
        return None;
    };
    let mut visited = 0usize;
    let mut cur = carrying[foot];
    let before = loop {
        visited += 1;
        match first_parent(cur) {
            None => break None,
            Some(parent) => match carrying.get(parent) {
                Some(next) => cur = next,
                None => break Some(parent.to_owned()),
            },
        }
        if visited > carrying.len() {
            return None;
        }
    };
    (visited == carrying.len()).then(|| LandedRun {
        before,
        after: (*foot).to_owned(),
    })
}
