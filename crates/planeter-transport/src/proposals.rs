//! What a person sees of what `propose` recorded (RFC 005 part 5; D-2, D-4, D-6, D-8, D-10, D-11): landing **observed from
//! facts**, the list and the detail of proposals, closing, and the retention sweep. (A *proposal* is the code's *change*:
//! `ChangeRecord`, `ChangeStore`; see `docs/proposals.md`.)
//!
//! **No decision here is made from `queued`**: a patch is *pending* if `sync pending` lists it and *signed into the history*
//! if a block that `log` lists for a ref the repository holds contains it; a patch in neither is *not listed*, and the
//! view says so (it is never called signed, and a proposal is never called landed because a patch merely vanished from
//! the pending list).
//!
//! **Cost.** One `sync pending` per repository per **state fingerprint** (the fetch routes' key), shared by every open
//! proposal of that repository and kept in memory; `log` is read **only when some proposal's patches have all left
//! pending** (the candidates), once per fingerprint. No prikk call is made per proposal listed.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use planeter_core::Assurance;
use planeter_core::HostingService;
use planeter_core::admission::{ReadVerb, ceiling_for, store_size};
use planeter_core::change::{
    Landing, PatchFacts, Transition, TransitionError, current_patch_set, observe_landing,
    patch_assurance, transition,
};
use planeter_prikk::{CallLimits, PrikkRepo};
use planeter_store::{
    ChangeRecord, ChangeState, ChangeStore, RepoId, RepositoryRecord, RevisionRecord,
};

use crate::artifacts::{ArtifactStore, Digest};
use crate::export::ExportService;
use crate::metrics::Metrics;

/// The default and the largest page of the list (bounded: RFC 005 T2).
pub const LIST_LIMIT_DEFAULT: usize = 50;
pub const LIST_LIMIT_MAX: usize = 100;
/// The most patches the detail lists (a proposal cannot make the page unbounded); `truncated` says when there are more.
pub const DETAIL_PATCH_LIMIT: usize = 1000;
/// The longest reason a close takes, in characters.
pub const REASON_MAX_CHARS: usize = 1000;
/// How many revisions one run of the retention sweep looks at (bounded work per run).
pub const SWEEP_BATCH: usize = 200;

/// Operator settings of the read side.
#[derive(Debug, Clone)]
pub struct ProposalConfig {
    /// How long the artifacts of a proposal that left `open` are kept (default 30 days).
    pub grace_secs: u64,
    /// How many revisions one sweep run examines.
    pub sweep_batch: usize,
}

impl Default for ProposalConfig {
    fn default() -> Self {
        ProposalConfig {
            grace_secs: planeter_core::change::DEFAULT_RETENTION_GRACE_SECS,
            sweep_batch: SWEEP_BATCH,
        }
    }
}

/// Which proposals a list asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFilter {
    Open,
    Landed,
    Closed,
    All,
}

impl StateFilter {
    pub fn parse(s: &str) -> Option<StateFilter> {
        match s {
            "open" => Some(StateFilter::Open),
            "landed" => Some(StateFilter::Landed),
            "closed" => Some(StateFilter::Closed),
            "all" => Some(StateFilter::All),
            _ => None,
        }
    }

    fn state(self) -> Option<ChangeState> {
        match self {
            StateFilter::Open => Some(ChangeState::Open),
            StateFilter::Landed => Some(ChangeState::Landed),
            StateFilter::Closed => Some(ChangeState::Closed),
            StateFilter::All => None,
        }
    }
}

/// Why a read or a close did not give an answer.
#[derive(Debug)]
pub enum ProposalError {
    /// No such proposal **in this repository** (an id of another repository is the same as an unknown one).
    NotFound,
    /// The `after` cursor is not one this service gave.
    BadCursor,
    /// A close needs a reason.
    ReasonRequired,
    /// The reason is longer than [`REASON_MAX_CHARS`].
    ReasonTooLong,
    /// The proposal has landed or been closed; neither ever changes again.
    AlreadyFinal(ChangeState),
    /// The repository or the records could not be read.
    Unavailable(String),
}

impl std::fmt::Display for ProposalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposalError::NotFound => f.write_str("no such proposal in this repository"),
            ProposalError::BadCursor => f.write_str("the cursor is not one this service gave"),
            ProposalError::ReasonRequired => f.write_str("a proposal is closed with a reason"),
            ProposalError::ReasonTooLong => {
                write!(f, "the reason is longer than {REASON_MAX_CHARS} characters")
            }
            ProposalError::AlreadyFinal(s) => {
                write!(f, "the proposal is {} and never changes again", s.as_str())
            }
            ProposalError::Unavailable(why) => write!(f, "unavailable: {why}"),
        }
    }
}

impl std::error::Error for ProposalError {}

fn unavailable(what: &str, e: impl std::fmt::Display) -> ProposalError {
    eprintln!("planeter: proposals: {what}: {e}");
    ProposalError::Unavailable(what.to_owned())
}

/// What is left of a proposal's accepted artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// The proposal is open: its artifacts are kept.
    Open,
    /// It left `open`; its artifacts are kept until this time (unix seconds), after which a sweep may remove them.
    Kept { until: u64 },
    /// Some of the artifacts are gone and some are not (a sweep removes artifact by artifact).
    Partly { until: u64 },
    /// All of its artifacts have been removed; they were kept until this time.
    Removed { until: u64 },
}

/// One proposal as a list shows it. No prikk call is behind any field of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub proposal: ChangeRecord,
    /// How many revisions it has.
    pub revisions: u32,
    /// How many patches its current set holds.
    pub patches: usize,
    /// Whether the latest revision says what it was built on (the declared base).
    pub built_on: bool,
    pub retention: Retention,
}

/// A page of the list: newest-updated first, and the cursor of the next page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Summary>,
    pub next: Option<String>,
}

/// One revision of the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionView {
    pub record: RevisionRecord,
    /// The artifact is still in the store (derived from the store each time).
    pub artifact_available: bool,
}

/// One patch of the current set with what is known of it, from the facts of **one observation**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchEntry {
    pub patch_id: String,
    /// `ForgeApprovedUnsealed` — pending in the repository; `Sealed` — in a block `log` lists; `None` — neither (not
    /// listed: it is not claimed to be anything).
    pub assurance: Option<Assurance>,
}

/// A proposal in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub proposal: ChangeRecord,
    pub revisions: Vec<RevisionView>,
    /// The patches of the current set, at most [`DETAIL_PATCH_LIMIT`], in id order.
    pub patches: Vec<PatchEntry>,
    pub patch_total: usize,
    pub truncated: bool,
    /// How many patches of the whole current set are still pending (open proposals).
    pub still_pending: usize,
    pub retention: Retention,
}

/// The facts of one observation of a repository: valid for one state fingerprint.
struct Observed {
    fingerprint: String,
    pending: Arc<BTreeSet<String>>,
    /// The patch ids in the blocks `log` lists, read when first needed.
    sealed: Mutex<Option<Arc<BTreeSet<String>>>>,
    /// The landing pass has been made for this fingerprint.
    landing_done: AtomicBool,
}

/// What one run of the retention sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Revisions examined.
    pub examined: usize,
    /// Artifacts removed, and their bytes.
    pub removed: usize,
    pub removed_bytes: u64,
    /// Artifacts kept because something still needs them.
    pub kept: usize,
    /// Artifacts that were already gone.
    pub already_gone: usize,
    pub errors: usize,
    /// Another run was in progress: this one did nothing.
    pub skipped: bool,
}

/// The read side of the proposals, over the records, the repository and the artifact store.
pub struct ProposalService {
    hosting: Arc<HostingService>,
    changes: Arc<dyn ChangeStore>,
    export: Arc<ExportService>,
    artifacts: Arc<ArtifactStore>,
    metrics: Arc<Metrics>,
    config: ProposalConfig,
    now_unix: fn() -> u64,
    observed: Mutex<HashMap<RepoId, Arc<Observed>>>,
    sweeping: AtomicBool,
    /// The last revision the sweep looked at: the next run goes on after it (and starts over at the end).
    sweep_after: Mutex<Option<String>>,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl ProposalService {
    pub fn new(
        hosting: Arc<HostingService>,
        changes: Arc<dyn ChangeStore>,
        export: Arc<ExportService>,
        artifacts: Arc<ArtifactStore>,
        metrics: Arc<Metrics>,
        config: ProposalConfig,
    ) -> Self {
        ProposalService {
            hosting,
            changes,
            export,
            artifacts,
            metrics,
            config,
            now_unix: system_now,
            observed: Mutex::new(HashMap::new()),
            sweeping: AtomicBool::new(false),
            sweep_after: Mutex::new(None),
        }
    }

    /// Replace the clock (tests pin it).
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    pub fn config(&self) -> &ProposalConfig {
        &self.config
    }

    pub fn artifacts(&self) -> &Arc<ArtifactStore> {
        &self.artifacts
    }

    /// How many proposals the repository has (the navigation shows *Proposals* when this is not zero).
    pub fn count(&self, repo: &RepoId) -> Result<u64, ProposalError> {
        self.changes
            .count_changes(repo, None)
            .map_err(|e| unavailable("cannot count the proposals", e))
    }

    /// How many are open (the repository page says so, and nothing more).
    pub fn count_open(&self, repo: &RepoId) -> Result<u64, ProposalError> {
        self.changes
            .count_changes(repo, Some(ChangeState::Open))
            .map_err(|e| unavailable("cannot count the proposals", e))
    }

    // ------------------------------------------------------------------------------------------ observation

    /// A driver for the observation's reads, with the memory ceiling of a flat read of the repository.
    fn reader(
        &self,
        record: &RepositoryRecord,
    ) -> Result<planeter_prikk::CliPrikkRepo, ProposalError> {
        let size =
            store_size(&record.path).map_err(|e| unavailable("cannot size the repository", e))?;
        let ceiling = ceiling_for(ReadVerb::Log.cost(size).max(OBSERVATION_FLOOR), None);
        self.hosting
            .open_record_with(record, CallLimits::data_ceiling(ceiling))
            .map_err(|e| unavailable("cannot open the repository", e))
    }

    /// The facts of the repository's current state: `sync pending` once per fingerprint.
    fn observation(&self, record: &RepositoryRecord) -> Result<Arc<Observed>, ProposalError> {
        let fingerprint = self
            .export
            .state(record)
            .map_err(|e| unavailable("cannot fingerprint the repository", format!("{e:?}")))?;
        if let Some(o) = self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&record.repo_id)
            && o.fingerprint == fingerprint
        {
            return Ok(Arc::clone(o));
        }
        let pending = self
            .reader(record)?
            .sync_pending_patches()
            .map_err(|e| unavailable("cannot read the pending patches", e))?;
        let observed = Arc::new(Observed {
            fingerprint,
            pending: Arc::new(pending.into_iter().collect()),
            sealed: Mutex::new(None),
            landing_done: AtomicBool::new(false),
        });
        self.observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(record.repo_id.clone(), Arc::clone(&observed));
        Ok(observed)
    }

    /// The patch ids in every block `log` lists, over every ref the repository holds (published or received): read once
    /// per observation, and only when somebody needs it.
    fn sealed(
        &self,
        record: &RepositoryRecord,
        obs: &Observed,
    ) -> Result<Arc<BTreeSet<String>>, ProposalError> {
        let mut slot = obs
            .sealed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(s) = &*slot {
            return Ok(Arc::clone(s));
        }
        let driver = self.reader(record)?;
        let refs = driver
            .branches(true)
            .map_err(|e| unavailable("cannot list the refs", e))?;
        let mut ids = BTreeSet::new();
        let names = refs
            .branches
            .iter()
            .map(|b| b.ref_name.as_str())
            .chain(refs.received.iter().map(|r| r.ref_name.as_str()));
        for name in names {
            // prikk's own default is the newest 20 blocks: a patch sealed longer ago would never be found.
            let log = driver
                .log(Some(name), Some(LOG_BLOCK_LIMIT))
                .map_err(|e| unavailable("cannot read a log", e))?;
            if log.blocks.len() >= LOG_BLOCK_LIMIT {
                // Possibly cut short: a patch not found here cannot be told from one not looked for. Nothing lands.
                return Err(unavailable(
                    "the history is longer than an observation reads",
                    name,
                ));
            }
            for block in log.blocks {
                ids.extend(block.patch_messages.into_iter().map(|p| p.patch_id));
            }
        }
        let ids = Arc::new(ids);
        *slot = Some(Arc::clone(&ids));
        Ok(ids)
    }

    /// Observe the repository and move to `landed` every open proposal whose **whole** current patch set has left
    /// `sync pending` and is present in a block `log` lists (RFC 005 D-6). Once per state fingerprint. The transition is a
    /// compare-and-set: a revision recorded since the proposal's records were read wins, and the observation is discarded.
    /// Returns the facts, for the caller's own classification.
    fn refresh(&self, record: &RepositoryRecord) -> Result<Arc<Observed>, ProposalError> {
        let obs = self.observation(record)?;
        if obs.landing_done.load(Ordering::Acquire) {
            return Ok(obs);
        }
        let repo = &record.repo_id;
        let open = self
            .changes
            .list_changes(repo, Some(ChangeState::Open))
            .map_err(|e| unavailable("cannot list the proposals", e))?;
        // Candidates: every patch of the set has left the pending list. Absent from pending is not yet landed.
        let mut candidates: Vec<(ChangeRecord, u32, BTreeSet<String>)> = Vec::new();
        for c in open {
            let revisions = self
                .changes
                .revisions_of(&c.change_id)
                .map_err(|e| unavailable("cannot read the revisions", e))?;
            let set = current_patch_set(&revisions);
            if observe_landing(&set, &obs.pending) == Landing::Landed {
                candidates.push((c, revisions.len() as u32, set));
            }
        }
        if !candidates.is_empty() {
            let sealed = self.sealed(record, &obs)?;
            let now = (self.now_unix)();
            for (c, revisions_seen, set) in candidates {
                if !set.iter().all(|id| sealed.contains(id)) {
                    continue;
                }
                let t = transition(
                    ChangeState::Open,
                    &Transition::Landed,
                    now,
                    self.config.grace_secs,
                )
                .map_err(|e| unavailable("cannot move a proposal to landed", e))?;
                match self.changes.leave_open_observed(
                    &c.change_id,
                    revisions_seen,
                    ChangeState::Landed,
                    None,
                    now,
                    Some(t.retain_until),
                ) {
                    Ok(true) => eprintln!(
                        "planeter: proposals: landed repo={} proposal={} patches={}",
                        repo.as_str(),
                        c.change_id,
                        set.len()
                    ),
                    Ok(false) => {} // a revision was recorded meanwhile (or it was closed): that wins
                    Err(e) => eprintln!("planeter: proposals: cannot record a landing: {e}"),
                }
            }
        }
        obs.landing_done.store(true, Ordering::Release);
        Ok(obs)
    }

    // ------------------------------------------------------------------------------------------ reads

    fn retention_of(&self, proposal: &ChangeRecord, revisions: &[RevisionRecord]) -> Retention {
        let Some(until) = proposal.retain_until else {
            return Retention::Open;
        };
        if proposal.state == ChangeState::Open {
            return Retention::Open;
        }
        let present = revisions
            .iter()
            .filter(|r| self.available(&r.artifact_digest))
            .count();
        match (present, revisions.len()) {
            (_, 0) => Retention::Kept { until },
            (0, _) => Retention::Removed { until },
            (p, n) if p == n => Retention::Kept { until },
            _ => Retention::Partly { until },
        }
    }

    fn available(&self, digest: &str) -> bool {
        record_digest(digest).is_ok_and(|d| self.artifacts.contains(&d))
    }

    /// The proposals of a repository, `limit` at a time (at most [`LIST_LIMIT_MAX`]), newest-updated first. The landing
    /// observation is made first (once per state fingerprint), so a state shown is a state observed.
    pub fn list(
        &self,
        record: &RepositoryRecord,
        filter: StateFilter,
        limit: usize,
        after: Option<&str>,
    ) -> Result<Page, ProposalError> {
        let after = after.map(parse_cursor).transpose()?;
        // The landing observation can fail (no prikk, an unreadable repository); the records still list. A state that
        // could not be observed is simply shown as the records say.
        if let Err(e) = self.refresh(record) {
            eprintln!("planeter: proposals: the landing observation failed: {e}");
        }
        let limit = limit.clamp(1, LIST_LIMIT_MAX);
        let mut rows: Vec<ChangeRecord> = self
            .changes
            .list_changes(&record.repo_id, filter.state())
            .map_err(|e| unavailable("cannot list the proposals", e))?;
        rows.sort_by(|a, b| (b.updated_at, &b.change_id).cmp(&(a.updated_at, &a.change_id)));
        let mut items = Vec::new();
        let mut next = None;
        for c in rows {
            if let Some((t, id)) = &after
                && (c.updated_at, c.change_id.as_str()) >= (*t, id.as_str())
            {
                continue;
            }
            if items.len() == limit {
                next = items.last().map(|s: &Summary| cursor(&s.proposal));
                break;
            }
            let revisions = self
                .changes
                .revisions_of(&c.change_id)
                .map_err(|e| unavailable("cannot read the revisions", e))?;
            let patches = current_patch_set(&revisions).len();
            let built_on = revisions
                .iter()
                .max_by_key(|r| r.seq)
                .is_some_and(|r| r.declared_base.is_some());
            let retention = self.retention_of(&c, &revisions);
            items.push(Summary {
                proposal: c,
                revisions: revisions.len() as u32,
                patches,
                built_on,
                retention,
            });
        }
        Ok(Page { items, next })
    }

    /// One proposal of this repository, in full. An id of another repository is [`ProposalError::NotFound`], the same as
    /// an unknown one.
    pub fn detail(&self, record: &RepositoryRecord, id: &str) -> Result<Detail, ProposalError> {
        let obs = match self.refresh(record) {
            Ok(o) => Some(o),
            Err(e) => {
                eprintln!("planeter: proposals: the landing observation failed: {e}");
                None
            }
        };
        let proposal = self.own(record, id)?;
        let revisions = self
            .changes
            .revisions_of(&proposal.change_id)
            .map_err(|e| unavailable("cannot read the revisions", e))?;
        let set = current_patch_set(&revisions);
        let patch_total = set.len();
        let mut patches = Vec::new();
        let mut still_pending = 0;
        if let Some(obs) = &obs {
            still_pending = set.iter().filter(|id| obs.pending.contains(*id)).count();
            // `log` is read only when a listed patch is not pending: otherwise nothing could be signed in.
            let shown: Vec<&String> = set.iter().take(DETAIL_PATCH_LIMIT).collect();
            let needs_log = shown.iter().any(|id| !obs.pending.contains(*id));
            let sealed = if needs_log {
                self.sealed(record, obs).ok()
            } else {
                None
            };
            for id in shown {
                let facts = PatchFacts {
                    in_block_from_log: sealed.as_ref().is_some_and(|s| s.contains(id)),
                    listed_pending: obs.pending.contains(id),
                    reported_queued: false,
                };
                patches.push(PatchEntry {
                    patch_id: id.clone(),
                    assurance: patch_assurance(&facts),
                });
            }
        } else {
            // Without an observation nothing is claimed about any patch.
            patches.extend(set.iter().take(DETAIL_PATCH_LIMIT).map(|id| PatchEntry {
                patch_id: id.clone(),
                assurance: None,
            }));
        }
        let retention = self.retention_of(&proposal, &revisions);
        let revisions = revisions
            .into_iter()
            .map(|record| RevisionView {
                artifact_available: self.available(&record.artifact_digest),
                record,
            })
            .collect();
        Ok(Detail {
            proposal,
            revisions,
            truncated: patch_total > patches.len(),
            patches,
            patch_total,
            still_pending,
            retention,
        })
    }

    /// The proposal `id` if it belongs to this repository.
    fn own(&self, record: &RepositoryRecord, id: &str) -> Result<ChangeRecord, ProposalError> {
        match self.changes.get_change(id) {
            Ok(Some(c)) if c.repo_id == record.repo_id => Ok(c),
            Ok(_) => Err(ProposalError::NotFound),
            Err(e) => Err(unavailable("cannot read the proposal", e)),
        }
    }

    /// The proposal (and its revision) a download names: `Ok(None)` when the proposal exists but has no such revision.
    pub fn revision(
        &self,
        record: &RepositoryRecord,
        id: &str,
        seq: u32,
    ) -> Result<(ChangeRecord, Option<RevisionRecord>), ProposalError> {
        let proposal = self.own(record, id)?;
        let revisions = self
            .changes
            .revisions_of(&proposal.change_id)
            .map_err(|e| unavailable("cannot read the revisions", e))?;
        let found = revisions.into_iter().find(|r| r.seq == seq);
        Ok((proposal, found))
    }

    /// The opener's account id of a proposal of this repository (the web layer decides who may close).
    pub fn opener(&self, record: &RepositoryRecord, id: &str) -> Result<String, ProposalError> {
        Ok(self.own(record, id)?.opened_by)
    }

    // ------------------------------------------------------------------------------------------ closing

    /// Close an **open** proposal with a reason. The caller has already decided that the person may (the opener, or
    /// `authorize(Close)`). On success the artifacts are kept for the grace period (`retain_until = now + grace`).
    pub fn close(
        &self,
        record: &RepositoryRecord,
        id: &str,
        reason: &str,
    ) -> Result<ChangeRecord, ProposalError> {
        let proposal = self.own(record, id)?;
        let reason = reason.trim();
        if reason.chars().count() > REASON_MAX_CHARS {
            return Err(ProposalError::ReasonTooLong);
        }
        let now = (self.now_unix)();
        let moved = transition(
            proposal.state,
            &Transition::Closed {
                reason: reason.to_owned(),
            },
            now,
            self.config.grace_secs,
        )
        .map_err(|e| match e {
            TransitionError::AlreadyFinal(s) => ProposalError::AlreadyFinal(s),
            TransitionError::ReasonRequired => ProposalError::ReasonRequired,
        })?;
        match self.changes.leave_open(
            &proposal.change_id,
            ChangeState::Closed,
            Some(reason),
            now,
            Some(moved.retain_until),
        ) {
            Ok(()) => {}
            // It left `open` between the read and the write (landed, or closed by somebody else): final.
            Err(planeter_store::StoreError::Invalid(_)) => {
                let state = self.own(record, id)?.state;
                return Err(ProposalError::AlreadyFinal(state));
            }
            Err(e) => return Err(unavailable("cannot close the proposal", e)),
        }
        eprintln!(
            "planeter: proposals: closed repo={} proposal={}",
            record.repo_id.as_str(),
            proposal.change_id
        );
        self.own(record, id)
    }

    // ------------------------------------------------------------------------------------------ retention

    /// One run of the retention sweep (RFC 005 D-9, T6). An artifact is removed only when **every** revision that names its
    /// digest belongs to a proposal that left `open` more than its grace ago, **and** no open intent names it. The
    /// records stay. One run at a time (a second returns at once); bounded work (`sweep_batch` revisions).
    ///
    /// The check is made **again after the object is out of reach**: a proposal may take the same artifact between the
    /// first look and the removal (it opens its intent before it keeps the upload), and then the object is put back.
    pub fn sweep(&self) -> SweepReport {
        if self.sweeping.swap(true, Ordering::AcqRel) {
            return SweepReport {
                skipped: true,
                ..SweepReport::default()
            };
        }
        let report = self.sweep_once();
        self.sweeping.store(false, Ordering::Release);
        self.metrics.sweep(
            report.examined,
            report.removed,
            report.removed_bytes,
            report.kept,
            report.errors,
        );
        eprintln!(
            "planeter: retention sweep: examined={} removed={} removed_bytes={} kept={} already_gone={} errors={}",
            report.examined,
            report.removed,
            report.removed_bytes,
            report.kept,
            report.already_gone,
            report.errors
        );
        report
    }

    fn sweep_once(&self) -> SweepReport {
        let mut report = SweepReport::default();
        let now = (self.now_unix)();
        let after = self
            .sweep_after
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let candidates =
            match self
                .changes
                .expired_revisions(now, after.as_deref(), self.config.sweep_batch)
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("planeter: retention sweep: cannot read the records: {e}");
                    report.errors += 1;
                    return report;
                }
            };
        // A full batch means there may be more after it: the next run goes on from here. A short one reached the end.
        *self
            .sweep_after
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            if candidates.len() >= self.config.sweep_batch {
                candidates.last().map(|r| r.revision_id.clone())
            } else {
                None
            };
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for r in candidates {
            report.examined += 1;
            if !seen.insert(r.artifact_digest.clone()) {
                continue; // one decision per digest, however many revisions name it
            }
            let Ok(digest) = record_digest(&r.artifact_digest) else {
                report.errors += 1;
                continue;
            };
            match self.changes.artifact_needed(&r.artifact_digest, now) {
                Ok(true) => {
                    report.kept += 1;
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("planeter: retention sweep: cannot read the records: {e}");
                    report.errors += 1;
                    continue;
                }
            }
            let removal = match self.artifacts.begin_remove(&digest) {
                Ok(Some(removal)) => removal,
                Ok(None) => {
                    report.already_gone += 1;
                    continue;
                }
                Err(e) => {
                    eprintln!("planeter: retention sweep: cannot remove an artifact: {e}");
                    report.errors += 1;
                    continue;
                }
            };
            // Look again now that the object is out of reach.
            match self
                .changes
                .artifact_needed(&r.artifact_digest, (self.now_unix)())
            {
                Ok(false) => match self.artifacts.finish_remove(removal) {
                    Ok(()) => {
                        report.removed += 1;
                        report.removed_bytes += r.artifact_bytes;
                    }
                    Err(e) => {
                        eprintln!("planeter: retention sweep: cannot unlink an artifact: {e}");
                        report.errors += 1;
                    }
                },
                Ok(true) | Err(_) => {
                    if let Err(e) = self.artifacts.undo_remove(removal) {
                        eprintln!("planeter: retention sweep: cannot put an artifact back: {e}");
                        report.errors += 1;
                    }
                    report.kept += 1;
                }
            }
        }
        report
    }
}

/// The store's digest of the one a revision record names: the record writes `sha256:<hex>`, the store keys by `<hex>`.
pub fn record_digest(recorded: &str) -> Result<Digest, crate::artifacts::StoreError> {
    Digest::parse(recorded.strip_prefix("sha256:").unwrap_or(recorded))
}

/// The least a flat observation read may be given. `sync pending` holds the pending list in memory: measured on prikk
/// 0.54.1, a repository of 200 sealed blocks needs under 1 MiB with nothing pending and under 6 MiB with 5,000 patches
/// pending (about 1 KiB a patch), so 64 MiB holds some 60,000 pending patches; the floor of a `log` read is raised to it.
const OBSERVATION_FLOOR: u64 = 64 << 20;

/// How many blocks of one ref an observation reads (`prikk log --limit`). A log that fills it may have been cut short, so
/// no proposal lands on it.
const LOG_BLOCK_LIMIT: usize = 100_000;

/// The cursor of the page after `c`: its sort key, `<updated_at>.<id>`.
fn cursor(c: &ChangeRecord) -> String {
    format!("{}.{}", c.updated_at, c.change_id)
}

fn parse_cursor(s: &str) -> Result<(u64, String), ProposalError> {
    let (t, id) = s.split_once('.').ok_or(ProposalError::BadCursor)?;
    let t: u64 = t.parse().map_err(|_| ProposalError::BadCursor)?;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ProposalError::BadCursor);
    }
    Ok((t, id.to_owned()))
}
