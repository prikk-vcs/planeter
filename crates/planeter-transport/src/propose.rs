//! Propose: the write side of the transport (RFC 004 D-3, D-5, D-6, D-7; RFC 005 D-1…D-3, D-9).
//!
//! One authenticated request puts an artifact on the forge as a **change revision**:
//!
//! 1. [`ProposeService::check`] — before any byte of the artifact is read: the `needs-recovery` flag, the quotas,
//!    the metadata, the change the push names, the declared base. Nothing here writes;
//! 2. [`ProposeService::start_upload`] — a bounded number of uploads at once, a working area of the proposal's own,
//!    the artifact streamed into it and hashed, never held in memory;
//! 3. [`ProposeService::accept`] — the same artifact already recorded answers with that revision; otherwise the
//!    repository's **W lease**, the **intent** (RFC 004 D-3), `sync accept` in the sandbox to completion with its own
//!    deadline and memory ceiling, and then, only after an exit 0, the revision **derived from the intent** by the
//!    pure model of part 3a and recorded in one transaction with the intent's closing.
//!
//! **Every path out of the accept ends in a state the next start can read.** A run that is killed, times out,
//! crashes or whose output cannot be read is *incomplete*: the intent stays open, the repository is flagged
//! `needs-recovery`, the uploaded artifact is kept, and nothing is derived. A run prikk **refused** and that changed
//! nothing (it exits 1 and `sync pending` is as it was) leaves nothing: the intent is abandoned and no flag is set.
//! Recovery itself is part 4; [`ProposeService::reconcile_at_start`] only makes sure a server that was killed in the
//! middle of a write comes back with the flag set.

use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use planeter_core::change::{
    BaseCheck, DeriveError, PatchFacts, QuotaPolicy, QuotaRefusal, QuotaRequest, QuotaTarget,
    QuotaUsage, check_base, check_quota, derive_revision, patch_assurance,
};
use planeter_core::read::Assurance;
use planeter_core::{CallLimits, HostingService, LeaseClass, LeaseError, LeaseManager};
use planeter_prikk::sandbox::{IncomingBounds, TIMEOUT_EXIT_CODE};
use planeter_prikk::{PrikkError, PrikkRepo};
use planeter_store::{
    ChangeRecord, ChangeState, ChangeStore, ClaimOutcome, IntentRecord, RepoId, RepositoryRecord,
    RevisionRecord, StoreError as RecordError,
};

use crate::artifacts::{AcceptDir, ArtifactStore, Digest, StoreError, UploadWriter};
use crate::export::ExportService;
use crate::metrics::{Metrics, ProposeOutcome, ProposeRecord};

/// The most metadata a proposal may carry (RFC 004 part 3b T1).
pub const METADATA_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The longest title, in characters.
pub const TITLE_MAX_CHARS: usize = 200;
/// The longest description, in bytes.
pub const DESCRIPTION_MAX_BYTES: usize = 64 * 1024;
/// The most patch ids a declaration may list (the object bound of an accept).
pub const DECLARATION_MAX_IDS: usize = 20_000;
/// The most patches a response lists with their assurance.
pub const RESPONSE_PATCH_LIMIT: usize = 1000;

// ------------------------------------------------------------------------------------------------ settings

/// Operator settings of the propose path.
#[derive(Debug, Clone)]
pub struct ProposeConfig {
    /// The largest artifact (`PRIKK_EXCHANGE_MAX_BYTES`), default 256 MiB: refused at the edge and again by prikk.
    pub max_artifact_bytes: u64,
    /// The most objects an artifact may declare (`PRIKK_EXCHANGE_MAX_OBJECTS`), default 20,000.
    pub max_objects: u64,
    /// The wall-time bound of the accept (default 600 s; never the 120 s read bound).
    pub accept_deadline: Duration,
    /// How long a proposal waits for an upload slot and for the repository's write lease before it is refused as busy.
    pub lease_wait: Duration,
    /// Proposals being uploaded or accepted at once on this host (default 2).
    pub max_concurrent: usize,
    /// The operator's ceiling on the artifacts a repository retains (RFC 005 Q-2: 4 GiB by default).
    pub repo_capacity_bytes: u64,
}

impl Default for ProposeConfig {
    fn default() -> Self {
        ProposeConfig {
            max_artifact_bytes: 256 << 20,
            max_objects: 20_000,
            accept_deadline: Duration::from_secs(600),
            lease_wait: Duration::from_secs(5),
            max_concurrent: 2,
            repo_capacity_bytes: 4096 << 20,
        }
    }
}

impl ProposeConfig {
    /// The admission cost of one accept of an artifact of `bytes` (RFC 004 D-5.3, measured *k* ≈ 4.2 in U-12):
    /// 10 MiB + 4.2 × the artifact.
    pub fn accept_cost(bytes: u64) -> u64 {
        const MIB: u64 = 1 << 20;
        10 * MIB + bytes.saturating_mul(42) / 10
    }

    /// The `RLIMIT_DATA` of the accept: the cost × 1.5 (RFC 011 D-2).
    pub fn accept_ceiling(bytes: u64) -> u64 {
        Self::accept_cost(bytes).saturating_mul(15) / 10
    }
}

// ------------------------------------------------------------------------------------------------ the request

/// The validated metadata of a proposal (the JSON of the request, read by the web layer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProposeMeta {
    /// An existing change to add a revision to; absent opens one.
    pub change: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The ref the submitter intends (advisory).
    pub target_ref: Option<String>,
    /// The block id the artifact was built against.
    pub base: Option<String>,
    /// The patch ids the artifact contains, as the submitter declares; absent for a push by plain prikk.
    pub declared_patches: Option<Vec<String>>,
    /// The client's name and version (logged nowhere; kept out of every record).
    pub client: Option<String>,
}

/// Why the metadata was refused. Each is a named reason the edge can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaError {
    /// A new change needs a title.
    TitleRequired,
    TitleTooLong,
    DescriptionTooLong,
    /// A target ref that is not a plain ref name.
    TargetRefInvalid,
    /// A base, a declared patch or a change id that is not what it should be.
    BaseInvalid,
    PatchIdInvalid,
    TooManyPatchIds,
    ChangeIdInvalid,
    ClientInvalid,
    /// A title, description or target ref was sent for a change that already exists.
    FieldsForExistingChange,
}

impl MetaError {
    pub fn reason(self) -> &'static str {
        match self {
            MetaError::TitleRequired => "title_required",
            MetaError::TitleTooLong => "title_too_long",
            MetaError::DescriptionTooLong => "description_too_long",
            MetaError::TargetRefInvalid => "target_ref_invalid",
            MetaError::BaseInvalid => "base_invalid",
            MetaError::PatchIdInvalid => "patch_id_invalid",
            MetaError::TooManyPatchIds => "too_many_patch_ids",
            MetaError::ChangeIdInvalid => "change_id_invalid",
            MetaError::ClientInvalid => "client_invalid",
            MetaError::FieldsForExistingChange => "fields_for_existing_change",
        }
    }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl ProposeMeta {
    /// Check the shape of every field. Nothing is trimmed or rewritten.
    pub fn validate(&self) -> Result<(), MetaError> {
        match &self.change {
            Some(id) => {
                if id.is_empty()
                    || id.len() > 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err(MetaError::ChangeIdInvalid);
                }
                if self.title.is_some() || self.description.is_some() || self.target_ref.is_some() {
                    return Err(MetaError::FieldsForExistingChange);
                }
            }
            None => match &self.title {
                None => return Err(MetaError::TitleRequired),
                Some(t) if t.trim().is_empty() => return Err(MetaError::TitleRequired),
                Some(t) if t.chars().count() > TITLE_MAX_CHARS => {
                    return Err(MetaError::TitleTooLong);
                }
                Some(t) if t.chars().any(char::is_control) => return Err(MetaError::TitleRequired),
                Some(_) => {}
            },
        }
        if self
            .description
            .as_ref()
            .is_some_and(|d| d.len() > DESCRIPTION_MAX_BYTES)
        {
            return Err(MetaError::DescriptionTooLong);
        }
        if let Some(r) = &self.target_ref
            && (r.is_empty()
                || r.len() > 200
                || r.contains("..")
                || r.chars().any(|c| c.is_control() || c.is_whitespace()))
        {
            return Err(MetaError::TargetRefInvalid);
        }
        if self.base.as_deref().is_some_and(|b| !is_hex64(b)) {
            return Err(MetaError::BaseInvalid);
        }
        if let Some(list) = &self.declared_patches {
            if list.len() > DECLARATION_MAX_IDS {
                return Err(MetaError::TooManyPatchIds);
            }
            if list.iter().any(|p| !is_hex64(p)) {
                return Err(MetaError::PatchIdInvalid);
            }
        }
        if self
            .client
            .as_deref()
            .is_some_and(|c| c.len() > 200 || c.chars().any(char::is_control))
        {
            return Err(MetaError::ClientInvalid);
        }
        Ok(())
    }
}

/// Who proposes: the stable id (the quotas are kept by it) and whether they hold `write` or above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submitter {
    pub id: String,
    pub can_write: bool,
}

// ------------------------------------------------------------------------------------------------ errors

/// Why the accept did not finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteCause {
    /// The accept hit its deadline and was killed.
    Deadline,
    /// The accept hit its memory ceiling.
    Ceiling,
    /// The run failed in a way that may have changed the repository (a crash, a signal, a lock left behind).
    Failed,
    /// The run ended 0, but what it printed could not be read, so nothing can be derived from it.
    Unreadable,
    /// The run ended 0 and was read, but the record of it could not be saved.
    Recording,
}

impl IncompleteCause {
    pub fn reason(self) -> &'static str {
        match self {
            IncompleteCause::Deadline => "accept_deadline",
            IncompleteCause::Ceiling => "accept_ceiling",
            IncompleteCause::Failed => "accept_failed",
            IncompleteCause::Unreadable => "accept_unreadable",
            IncompleteCause::Recording => "accept_not_recorded",
        }
    }
}

/// Why a proposal produced no revision. Each variant is a named refusal with a stable reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeError {
    /// No upload slot or no write lease within the bounded wait.
    Busy {
        retry_after: Duration,
    },
    /// The repository needs recovery: writes are refused, reads are not.
    NeedsRecovery {
        reason: String,
    },
    Meta(MetaError),
    /// The change the push names does not exist in this repository.
    UnknownChange,
    /// The change is not open any more.
    ChangeNotOpen(ChangeState),
    /// The change is somebody else's and the submitter does not hold `write`.
    NotYourChange,
    Quota(QuotaRefusal),
    /// The declared base is not held: push the base first.
    BaseUnknown,
    /// Somebody else started an accept of this exact artifact that has not finished.
    IntentHeldByAnother,
    /// The accept added nothing the forge did not already hold: nothing was recorded.
    NoNewContent,
    /// The declaration does not match what the accept added, or names patches the forge does not hold.
    Declaration(DeriveError),
    /// prikk refused the artifact and left nothing behind; its text, verbatim.
    Refused {
        text: String,
    },
    /// The accept did not finish: the intent `intent_id` is open and the repository needs recovery.
    Incomplete {
        intent_id: String,
        cause: IncompleteCause,
    },
    /// Something other than the accept failed before anything changed (the stores, the disk).
    Unavailable,
}

impl ProposeError {
    /// The stable reason code (the API's error code and the metrics' refusal key).
    pub fn reason(&self) -> &'static str {
        match self {
            ProposeError::Busy { .. } => "busy",
            ProposeError::NeedsRecovery { .. } => "needs_recovery",
            ProposeError::Meta(m) => m.reason(),
            ProposeError::UnknownChange => "unknown_change",
            ProposeError::ChangeNotOpen(_) => "change_not_open",
            ProposeError::NotYourChange => "not_your_change",
            ProposeError::Quota(_) => "quota_exceeded",
            ProposeError::BaseUnknown => "base_unknown",
            ProposeError::IntentHeldByAnother => "intent_held_by_another",
            ProposeError::NoNewContent => "no_new_content",
            ProposeError::Declaration(DeriveError::DeclarationMissesAdded { .. }) => {
                "declaration_misses_added"
            }
            ProposeError::Declaration(_) => "declared_not_held",
            ProposeError::Refused { .. } => "accept_refused",
            ProposeError::Incomplete { cause, .. } => cause.reason(),
            ProposeError::Unavailable => "unavailable",
        }
    }
}

// ------------------------------------------------------------------------------------------------ the plan and outcome

/// What the checks before the upload decided (see [`ProposeService::check`]).
#[derive(Debug, Clone)]
pub struct Plan {
    /// The change this push adds a revision to; `None` when it opens one.
    pub existing: Option<ChangeRecord>,
    /// The number of the revision this push would be (1 for a new change).
    pub next_seq: u32,
    /// A quota refusal that is held back until the artifact's digest is known: a repeat of an artifact that is
    /// already recorded is answered with its revision, whatever the quotas now say.
    deferred_quota: Option<QuotaRefusal>,
    /// The policy the checks used.
    pub policy: QuotaPolicy,
}

/// The result of a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    pub change_id: String,
    pub revision_id: String,
    pub seq: u32,
    /// This push opened the change.
    pub created_change: bool,
    /// The artifact had a revision already; nothing was recorded.
    pub already_recorded: bool,
    /// What accept said of each claim.
    pub claims: Vec<ClaimOutcome>,
    /// How many patches the push added; `patches` lists the first [`RESPONSE_PATCH_LIMIT`] of them.
    pub added_count: u64,
    pub patches: Vec<(String, Assurance)>,
    /// Whether the revision carries a declared patch set.
    pub declared: bool,
    pub artifact_bytes: u64,
}

/// An upload being written (see [`ProposeService::start_upload`]). Holds the proposal's working area and one of the
/// host's upload slots until it is dropped.
pub struct Upload {
    dir: AcceptDir,
    writer: Option<UploadWriter>,
    _slot: SlotGuard,
    started: Instant,
}

impl Upload {
    /// Append the next bytes of the artifact.
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        self.writer
            .as_mut()
            .expect("the writer lives until finish")
            .write(chunk)
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.writer.as_ref().map_or(0, UploadWriter::written)
    }

    /// The upload is complete: sync it and learn its digest.
    pub fn finish(mut self) -> Result<Uploaded, StoreError> {
        let (digest, size) = self
            .writer
            .take()
            .expect("the writer lives until finish")
            .finish()?;
        Ok(Uploaded {
            dir: self.dir,
            digest,
            size,
            started: self.started,
            _slot: self._slot,
        })
    }
}

/// A finished upload: the artifact is on disk in the proposal's working area, and its digest is known.
pub struct Uploaded {
    dir: AcceptDir,
    pub digest: Digest,
    pub size: u64,
    started: Instant,
    _slot: SlotGuard,
}

/// A bounded counting semaphore over a mutex (the upload slots), with a timed wait.
struct Slots {
    free: Mutex<usize>,
    changed: Condvar,
}

impl Slots {
    fn acquire(self: &Arc<Self>, wait: Duration) -> Option<SlotGuard> {
        let end = Instant::now() + wait;
        let mut free = self
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *free == 0 {
            let now = Instant::now();
            if now >= end {
                return None;
            }
            free = self
                .changed
                .wait_timeout(free, end - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        *free -= 1;
        Some(SlotGuard(Arc::clone(self)))
    }
}

struct SlotGuard(Arc<Slots>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        *self
            .0
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        self.0.changed.notify_one();
    }
}

// ------------------------------------------------------------------------------------------------ the service

/// The propose service. Cheap to share (`Arc`); every method is **blocking** (call it from `spawn_blocking`).
pub struct ProposeService {
    hosting: Arc<HostingService>,
    artifacts: Arc<ArtifactStore>,
    leases: Arc<LeaseManager>,
    changes: Arc<dyn ChangeStore>,
    export: Arc<ExportService>,
    config: ProposeConfig,
    metrics: Arc<Metrics>,
    slots: Arc<Slots>,
    now_unix: fn() -> u64,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A fresh identifier: 24 hexadecimal characters of a digest of OS-random bytes, behind a letter that says what it
/// names (`c` change, `v` revision, `i` intent).
fn new_id(kind: char) -> String {
    let token = planeter_auth::random_token();
    format!(
        "{kind}-{}",
        &planeter_auth::sha256_hex(token.as_bytes())[..24]
    )
}

impl ProposeService {
    pub fn new(
        hosting: Arc<HostingService>,
        artifacts: Arc<ArtifactStore>,
        leases: Arc<LeaseManager>,
        changes: Arc<dyn ChangeStore>,
        export: Arc<ExportService>,
        config: ProposeConfig,
        metrics: Arc<Metrics>,
    ) -> Self {
        let slots = Arc::new(Slots {
            free: Mutex::new(config.max_concurrent.max(1)),
            changed: Condvar::new(),
        });
        ProposeService {
            hosting,
            artifacts,
            leases,
            changes,
            export,
            config,
            metrics,
            slots,
            now_unix: system_now,
        }
    }

    /// Test seam: the clock of the records.
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    pub fn config(&self) -> &ProposeConfig {
        &self.config
    }

    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    pub fn changes(&self) -> &Arc<dyn ChangeStore> {
        &self.changes
    }

    fn unavailable(what: &str, e: impl std::fmt::Display) -> ProposeError {
        eprintln!("planeter: propose: {what}: {e}");
        ProposeError::Unavailable
    }

    // -------------------------------------------------------------------------------- 1. the checks before the upload

    /// Everything that can be decided before the artifact is read, in the order a person can act on it:
    /// the `needs-recovery` flag → the quota → the metadata → the change the push names → the declared base.
    /// `artifact_bytes` is the size the request declares (the framing makes it exact). Nothing is written.
    ///
    /// A quota refusal is **held back** (see [`Plan`]) rather than returned: the artifact may be a repeat of one
    /// already recorded, which is answered with its revision whatever the quotas say now.
    pub fn check(
        &self,
        record: &RepositoryRecord,
        who: &Submitter,
        meta: &ProposeMeta,
        artifact_bytes: u64,
    ) -> Result<Plan, ProposeError> {
        let repo = &record.repo_id;
        let flags = self
            .changes
            .repo_flags(repo)
            .map_err(|e| Self::unavailable("cannot read the repository flags", e))?;
        if flags.needs_recovery {
            return Err(ProposeError::NeedsRecovery {
                reason: flags.reason.unwrap_or_default(),
            });
        }
        // The change the push names (looked up now; its refusals come after the metadata's).
        let existing = match &meta.change {
            Some(id) if id_is_plain(id) => self
                .changes
                .get_change(id)
                .map_err(|e| Self::unavailable("cannot read the change", e))?
                .filter(|c| &c.repo_id == repo),
            _ => None,
        };
        let revisions = match &existing {
            Some(c) => self
                .changes
                .revisions_of(&c.change_id)
                .map_err(|e| Self::unavailable("cannot read the revisions", e))?
                .len() as u64,
            None => 0,
        };
        let mut policy = QuotaPolicy::defaults_for(record.proposals);
        policy.repo_retained_bytes = self.config.repo_capacity_bytes;
        let usage: QuotaUsage = self
            .changes
            .usage(repo, &who.id, (self.now_unix)())
            .map_err(|e| Self::unavailable("cannot read the usage", e))?
            .into();
        let target = match &existing {
            Some(_) => QuotaTarget::ExistingChange { revisions },
            None => QuotaTarget::NewChange,
        };
        let deferred_quota = check_quota(
            &usage,
            &policy,
            &QuotaRequest {
                bytes: artifact_bytes,
                target,
            },
        )
        .err();
        meta.validate().map_err(ProposeError::Meta)?;
        if meta.change.is_some() {
            let c = existing.as_ref().ok_or(ProposeError::UnknownChange)?;
            if c.state != ChangeState::Open {
                return Err(ProposeError::ChangeNotOpen(c.state));
            }
            if c.opened_by != who.id && !who.can_write {
                return Err(ProposeError::NotYourChange);
            }
        }
        if let Some(base) = &meta.base
            && !self.base_is_held(record, base)?
        {
            return Err(ProposeError::BaseUnknown);
        }
        let next_seq = u32::try_from(revisions + 1).unwrap_or(u32::MAX);
        Ok(Plan {
            existing,
            next_seq,
            deferred_quota,
            policy,
        })
    }

    /// Is `base` listed by `log` of a ref the repository holds (published or received)? Never `show <base>`
    /// (it fails for some blocks, PK-57).
    fn base_is_held(&self, record: &RepositoryRecord, base: &str) -> Result<bool, ProposeError> {
        let driver = self
            .hosting
            .open_record(record)
            .map_err(|e| Self::unavailable("cannot open the repository", e))?;
        let refs = driver
            .branches(true)
            .map_err(|e| Self::unavailable("cannot list the refs", e))?;
        let mut listed: BTreeSet<String> = BTreeSet::new();
        let names = refs
            .branches
            .iter()
            .map(|b| b.ref_name.as_str())
            .chain(refs.received.iter().map(|r| r.ref_name.as_str()));
        for name in names {
            let log = driver
                .log(Some(name), None)
                .map_err(|e| Self::unavailable("cannot read a log", e))?;
            listed.extend(log.blocks.into_iter().map(|b| b.block_id));
            if listed.contains(base) {
                break;
            }
        }
        Ok(matches!(
            check_base(Some(base), &listed),
            BaseCheck::Held(_)
        ))
    }

    // -------------------------------------------------------------------------------- 2. the upload

    /// Begin an upload of at most `expected` bytes: takes one of the host's upload slots (bounded wait; `Busy`
    /// past it) and a working area of the proposal's own. The slot is held until the proposal ends.
    pub fn start_upload(&self, expected: u64) -> Result<Upload, ProposeError> {
        let started = Instant::now();
        let slot = self
            .slots
            .acquire(self.config.lease_wait)
            .ok_or(ProposeError::Busy {
                retry_after: self.config.lease_wait,
            })?;
        let dir = self
            .artifacts
            .new_accept_dir()
            .map_err(|e| Self::unavailable("cannot create a working directory", e))?;
        let writer = dir
            .begin_upload(expected)
            .map_err(|e| Self::unavailable("cannot begin the upload", e))?;
        Ok(Upload {
            dir,
            writer: Some(writer),
            _slot: slot,
            started,
        })
    }

    // -------------------------------------------------------------------------------- 3. the accept

    /// Take the uploaded artifact through the accept (see the module documentation). `who` and `meta` are those of
    /// [`ProposeService::check`], whose `plan` is passed back.
    pub fn accept(
        &self,
        record: &RepositoryRecord,
        who: &Submitter,
        meta: &ProposeMeta,
        plan: &Plan,
        uploaded: Uploaded,
    ) -> Result<Proposed, ProposeError> {
        let result = self.accept_inner(record, who, meta, plan, &uploaded);
        if let Err(e) = &result {
            self.metrics.refusal(e.reason());
        }
        result
    }

    fn accept_inner(
        &self,
        record: &RepositoryRecord,
        who: &Submitter,
        meta: &ProposeMeta,
        plan: &Plan,
        up: &Uploaded,
    ) -> Result<Proposed, ProposeError> {
        let repo = &record.repo_id;
        let digest = up.digest.as_str().to_owned();
        let record_key = format!("sha256:{digest}");
        // Idempotence: the same artifact is the same revision (whatever the quotas now say).
        if let Some(existing) = self
            .changes
            .revision_by_digest(repo, &record_key)
            .map_err(|e| Self::unavailable("cannot look up the artifact", e))?
        {
            let attempts = 0;
            self.metrics.propose(
                repo.as_str(),
                &ProposeRecord {
                    outcome: ProposeOutcome::AlreadyRecorded,
                    bytes: up.size,
                    wall: up.started.elapsed(),
                    lease_wait: Duration::ZERO,
                    attempts,
                    added_patches: 0,
                },
            );
            return Ok(self.already_recorded(&existing));
        }
        if let Some(refusal) = plan.deferred_quota {
            return Err(ProposeError::Quota(refusal));
        }

        // The write lease: a conflict is a queue, never prikk's error (RFC 004 D-7).
        let waited = Instant::now();
        let lease = match self.leases.acquire(
            repo,
            LeaseClass::Write,
            "sync accept",
            self.config.lease_wait,
        ) {
            Ok(l) => l,
            Err(LeaseError::Busy { retry_after, .. }) => {
                return Err(ProposeError::Busy { retry_after });
            }
            Err(e) => return Err(Self::unavailable("cannot take the write lease", e)),
        };
        let lease_wait = waited.elapsed();

        // The driver of the accept: this proposal's two directories, the incoming bounds, the deadline and the
        // memory ceiling of the accept (not the read path's).
        let ceiling = ProposeConfig::accept_ceiling(up.size);
        let sandbox = self
            .hosting
            .sandbox()
            .with_artifact_in(up.dir.in_dir())
            .with_spool_out(
                up.dir.out_dir(),
                self.config.accept_deadline.as_secs().max(1),
            )
            .with_incoming_bounds(IncomingBounds {
                exchange_max_bytes: Some(self.config.max_artifact_bytes),
                exchange_max_objects: Some(self.config.max_objects),
                ..IncomingBounds::default()
            });
        let driver = self
            .hosting
            .open_record_sandboxed(record, CallLimits::data_ceiling(ceiling), sandbox)
            .map_err(|e| Self::unavailable("cannot open the repository", e))?;
        let reader = self
            .hosting
            .open_record(record)
            .map_err(|e| Self::unavailable("cannot open the repository", e))?;

        // The intent first: the only thing that survives a kill. Its `pending_before` is the first attempt's.
        let pending_before = reader
            .sync_pending_patches()
            .map_err(|e| Self::unavailable("cannot read the pending patches", e))?;
        let intent = self
            .changes
            .open_or_resume_intent(
                &new_id('i'),
                repo,
                &record_key,
                &pending_before,
                &who.id,
                (self.now_unix)(),
            )
            .map_err(|e| Self::unavailable("cannot journal the intent", e))?;
        if intent.principal != who.id {
            return Err(ProposeError::IntentHeldByAnother);
        }

        let run = driver.sync_accept_report(&up.dir.artifact(), &up.dir.claims());
        let report = match run {
            Ok(r) => r,
            Err(e) => {
                let outcome = self.classify_failure(&reader, &intent, &e);
                if matches!(outcome, Failure::Incomplete(_)) {
                    // The repository may have changed: its fingerprint moves, whatever the run left behind.
                    let _ = self.export.write_completed(&lease);
                }
                return Err(self.finish_failure(record, up, &intent, outcome, &e, lease_wait));
            }
        };
        // The accept finished (exit 0). The repository changed: the state fingerprint moves now, whatever follows.
        let _ = self.export.write_completed(&lease);

        // Derive. The pending set is read again; the declaration is checked against what the repository holds.
        let pending_after: BTreeSet<String> = match reader.sync_pending_patches() {
            Ok(v) => v.into_iter().collect(),
            Err(e) => {
                return Err(self.incomplete(
                    record,
                    up,
                    &intent,
                    IncompleteCause::Unreadable,
                    &e.to_string(),
                    lease_wait,
                ));
            }
        };
        let sealed = if meta.declared_patches.is_some() {
            match self.patches_in_logs(&reader) {
                Ok(s) => s,
                Err(_) => {
                    return Err(ProposeError::Unavailable);
                }
            }
        } else {
            BTreeSet::new()
        };
        let held = |id: &str| pending_after.contains(id) || sealed.contains(id);
        let derived = match derive_revision(
            &intent,
            &pending_after,
            &held,
            meta.declared_patches.as_deref(),
        ) {
            Ok(d) => d,
            Err(DeriveError::NoNewContent) => {
                // Nothing was added: nothing is recorded, and the intent is closed (a repeat is a new accept).
                let _ = self
                    .changes
                    .abandon_intent(&intent.intent_id, (self.now_unix)());
                return Err(ProposeError::NoNewContent);
            }
            // A declaration that does not hold: the accepted patches stay in the repository, the intent stays open
            // (so a corrected re-send derives from the first attempt's list), and the lease is released normally
            // (so a restart does not mistake this for a crash).
            Err(
                e @ (DeriveError::DeclarationMissesAdded { .. }
                | DeriveError::DeclaredNotHeld { .. }),
            ) => {
                return Err(ProposeError::Declaration(e));
            }
            Err(e) => {
                return Err(Self::unavailable("cannot derive the revision", e));
            }
        };

        // Retain the artifact (never evicted), then record the revision and close the intent in one transaction.
        if let Err(e) = self.artifacts.retain(&up.dir, &up.digest) {
            return Err(self.incomplete(
                record,
                up,
                &intent,
                IncompleteCause::Recording,
                &e.to_string(),
                lease_wait,
            ));
        }
        let now = (self.now_unix)();
        let (change_id, new_change) = match &plan.existing {
            Some(c) => (c.change_id.clone(), None),
            None => {
                let id = new_id('c');
                (
                    id.clone(),
                    Some(ChangeRecord {
                        change_id: id,
                        repo_id: repo.clone(),
                        title: meta.title.clone().unwrap_or_default(),
                        description: meta.description.clone().unwrap_or_default(),
                        target_ref: meta.target_ref.clone(),
                        state: ChangeState::Open,
                        closed_reason: None,
                        opened_by: who.id.clone(),
                        opened_at: now,
                        updated_at: now,
                        retain_until: None,
                    }),
                )
            }
        };
        let seq = plan.next_seq;
        let mut key_ids: Vec<String> = Vec::new();
        for line in &report.claim_lines {
            for k in &line.key_ids {
                if !key_ids.contains(k) {
                    key_ids.push(k.clone());
                }
            }
        }
        let claim_outcomes: Vec<ClaimOutcome> = report
            .claim_lines
            .iter()
            .map(|c| ClaimOutcome {
                claim_id: c.claim_id.clone(),
                outcome: c.outcome.clone(),
            })
            .collect();
        let declared_ids = derived.declared.clone().into_stored();
        let revision = RevisionRecord {
            revision_id: new_id('v'),
            change_id: change_id.clone(),
            repo_id: repo.clone(),
            seq,
            artifact_digest: record_key.clone(),
            artifact_bytes: up.size,
            declared_base: meta.base.clone(),
            claim_ids: report
                .claim_lines
                .iter()
                .map(|c| c.claim_id.clone())
                .collect(),
            added_patch_ids: derived.added_patch_ids.clone(),
            declared_patch_ids: declared_ids.clone(),
            claim_outcomes: claim_outcomes.clone(),
            submitter: who.id.clone(),
            author_key_ids: key_ids,
            created_at: now,
        };
        if let Err(e) = self.changes.complete_intent_with_revision(
            &intent.intent_id,
            new_change.as_ref(),
            &revision,
            now,
        ) {
            // The accept finished but its record did not save: the intent is still open, a re-send resumes it.
            let cause = match e {
                RecordError::Duplicate(_) => IncompleteCause::Recording,
                _ => IncompleteCause::Recording,
            };
            return Err(self.incomplete(record, up, &intent, cause, &e.to_string(), lease_wait));
        }
        drop(lease);

        // The response: every added patch is classified from facts (pending from `sync pending`, sealed from `log`),
        // never from `queued` (IS-47).
        let patches: Vec<(String, Assurance)> = derived
            .added_patch_ids
            .iter()
            .take(RESPONSE_PATCH_LIMIT)
            .map(|id| {
                let facts = PatchFacts {
                    in_block_from_log: sealed.contains(id),
                    listed_pending: pending_after.contains(id),
                    reported_queued: false,
                };
                (
                    id.clone(),
                    patch_assurance(&facts).unwrap_or(Assurance::ForgeApprovedUnsealed),
                )
            })
            .collect();
        self.metrics.propose(
            repo.as_str(),
            &ProposeRecord {
                outcome: ProposeOutcome::Accepted,
                bytes: up.size,
                wall: up.started.elapsed(),
                lease_wait,
                attempts: intent.attempts,
                added_patches: derived.added_patch_ids.len() as u64,
            },
        );
        Ok(Proposed {
            change_id,
            revision_id: revision.revision_id,
            seq,
            created_change: new_change.is_some(),
            already_recorded: false,
            claims: claim_outcomes,
            added_count: derived.added_patch_ids.len() as u64,
            patches,
            declared: declared_ids.is_some(),
            artifact_bytes: up.size,
        })
    }

    /// The patch ids of every block `log` lists, over every ref the repository holds.
    fn patches_in_logs(
        &self,
        driver: &planeter_prikk::CliPrikkRepo,
    ) -> Result<BTreeSet<String>, PrikkError> {
        let refs = driver.branches(true)?;
        let mut out = BTreeSet::new();
        let names = refs
            .branches
            .iter()
            .map(|b| b.ref_name.as_str())
            .chain(refs.received.iter().map(|r| r.ref_name.as_str()));
        for name in names {
            let log = driver.log(Some(name), None)?;
            for block in log.blocks {
                out.extend(block.patch_messages.into_iter().map(|p| p.patch_id));
            }
        }
        Ok(out)
    }

    fn already_recorded(&self, r: &RevisionRecord) -> Proposed {
        let added = r.added_patch_ids.len() as u64;
        Proposed {
            change_id: r.change_id.clone(),
            revision_id: r.revision_id.clone(),
            seq: r.seq,
            created_change: false,
            already_recorded: true,
            claims: r.claim_outcomes.clone(),
            added_count: added,
            // Not re-classified: the repository may have moved since (those patches may be sealed now); the list of
            // ids is the record's, and nothing here claims more than the record knows.
            patches: r
                .added_patch_ids
                .iter()
                .take(RESPONSE_PATCH_LIMIT)
                .map(|id| (id.clone(), Assurance::ForgeApprovedUnsealed))
                .collect(),
            declared: r.declared_patch_ids.is_some(),
            artifact_bytes: r.artifact_bytes,
        }
    }

    // -------------------------------------------------------------------------------- failures

    /// What a failed run amounts to: refused (nothing changed, prikk said no) or incomplete (it may have changed
    /// the repository, or we cannot tell).
    fn classify_failure(
        &self,
        reader: &planeter_prikk::CliPrikkRepo,
        intent: &IntentRecord,
        e: &PrikkError,
    ) -> Failure {
        match e {
            PrikkError::CeilingExceeded { .. } => Failure::Incomplete(IncompleteCause::Ceiling),
            PrikkError::Command { code: Some(c), .. } if *c == TIMEOUT_EXIT_CODE => {
                Failure::Incomplete(IncompleteCause::Deadline)
            }
            PrikkError::Command {
                code: Some(1),
                stderr,
            } if !stderr.contains("lock conflict") => {
                // Exit 1 is how prikk refuses. It "leaves nothing" only if the pending set is as it was.
                let before: BTreeSet<&String> = intent.pending_before.iter().collect();
                match reader.sync_pending_patches() {
                    Ok(now) if now.iter().collect::<BTreeSet<_>>() == before => {
                        Failure::Refused(stderr.clone())
                    }
                    _ => Failure::Incomplete(IncompleteCause::Failed),
                }
            }
            PrikkError::Parse(_) => Failure::Incomplete(IncompleteCause::Unreadable),
            _ => Failure::Incomplete(IncompleteCause::Failed),
        }
    }

    fn finish_failure(
        &self,
        record: &RepositoryRecord,
        up: &Uploaded,
        intent: &IntentRecord,
        outcome: Failure,
        e: &PrikkError,
        lease_wait: Duration,
    ) -> ProposeError {
        match outcome {
            Failure::Refused(text) => {
                // Nothing was left: the intent is closed, no flag is set, the uploaded bytes are not kept.
                let _ = self
                    .changes
                    .abandon_intent(&intent.intent_id, (self.now_unix)());
                ProposeError::Refused { text }
            }
            Failure::Incomplete(cause) => {
                self.incomplete(record, up, intent, cause, &e.to_string(), lease_wait)
            }
        }
    }

    /// The accept did not finish. Flag the repository, keep the intent open, keep the artifact, say so.
    fn incomplete(
        &self,
        record: &RepositoryRecord,
        up: &Uploaded,
        intent: &IntentRecord,
        cause: IncompleteCause,
        detail: &str,
        lease_wait: Duration,
    ) -> ProposeError {
        let repo = &record.repo_id;
        let reason = format!("a write did not finish ({})", cause.reason());
        if let Err(e) = self
            .changes
            .set_needs_recovery(repo, &reason, (self.now_unix)())
        {
            eprintln!("planeter: propose: cannot set the recovery flag: {e}");
        }
        // Keep the uploaded artifact: the retry is of this very artifact, and an accepted patch cannot be removed.
        if let Err(e) = self.artifacts.retain(&up.dir, &up.digest) {
            eprintln!("planeter: propose: cannot keep the artifact of an unfinished accept: {e}");
        }
        eprintln!(
            "planeter: propose: the accept did not finish repo={} intent={} cause={} detail={}",
            repo.as_str(),
            intent.intent_id,
            cause.reason(),
            detail.lines().next().unwrap_or("")
        );
        self.metrics.propose(
            repo.as_str(),
            &ProposeRecord {
                outcome: ProposeOutcome::Incomplete,
                bytes: up.size,
                wall: up.started.elapsed(),
                lease_wait,
                attempts: intent.attempts,
                added_patches: 0,
            },
        );
        ProposeError::Incomplete {
            intent_id: intent.intent_id.clone(),
            cause,
        }
    }

    // -------------------------------------------------------------------------------- start-up

    /// A server that was **killed in the middle of a write** comes back with the repository flagged. The evidence is
    /// the durable lease table: an exclusive `W` holder record written by a process that is not this one. The flag
    /// is set (idempotently) for each such repository; the record is left for recovery (part 4) to clear. Returns the
    /// repositories flagged.
    pub fn reconcile_at_start(&self) -> Vec<RepoId> {
        let mut flagged = Vec::new();
        let orphans = match self.leases.orphans() {
            Ok(o) => o,
            Err(e) => {
                eprintln!("planeter: cannot read the lease records: {e}");
                return flagged;
            }
        };
        for o in orphans.into_iter().filter(|o| o.class == "W") {
            let already = self
                .changes
                .repo_flags(&o.repo_id)
                .map(|f| f.needs_recovery)
                .unwrap_or(false);
            if already {
                continue;
            }
            let open = self
                .changes
                .open_intents(&o.repo_id)
                .map(|v| v.len())
                .unwrap_or(0);
            let reason = format!(
                "the server stopped during a write ({}, {} open intent(s))",
                o.operation, open
            );
            if self
                .changes
                .set_needs_recovery(&o.repo_id, &reason, (self.now_unix)())
                .is_ok()
            {
                eprintln!(
                    "planeter: repository {} needs recovery: {reason}",
                    o.repo_id.as_str()
                );
                flagged.push(o.repo_id);
            }
        }
        flagged
    }
}

enum Failure {
    Refused(String),
    Incomplete(IncompleteCause),
}

fn id_is_plain(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
