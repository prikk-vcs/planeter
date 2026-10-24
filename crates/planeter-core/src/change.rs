//! The pure rules of a proposal (RFC 005 D-1…D-3, D-5…D-7, D-9, D-10; RFC 004 D-3): what a revision is
//! derived from, when a declaration is accepted, when a base is held, how a proposal's state may move, when landing
//! is *observed*, what each quota allows, and how much assurance a patch carries.
//!
//! **Words.** The code's internal name for a proposal is *change* (`ChangeRecord`, `ChangeState`, `Quota::OpenChanges`, …); in
//! everything a person or a program reads it is a **proposal**, and *change* keeps the glossary's meaning, a recorded unit of
//! history (D-38). Nothing stored is renamed.
//!
//! **No I/O.** Everything takes the results of `sync accept`, `sync pending` and `log` as arguments; nothing here
//! calls prikk, reads a clock or touches the store. The store ([`planeter_store::change`]) keeps the records; part 3b
//! wires the two to the network.
//!
//! # The invariants, in plain words
//!
//! * A revision is derived from an **intent record**, never from two loose sets, and only while the intent is
//!   `open`: a diff from an incomplete run is unrepresentable ([`derive_revision`]).
//! * What the accept added is `pending_after − intent.pending_before`, where `pending_before` is the list of the
//!   **first** attempt (the store never overwrites it on a retry).
//! * A declaration (the submitter's list of the patch ids its artifact contains) is a claim, checked: it must
//!   include everything the accept added and name only patches the forge holds.
//! * A change leaves `open` once; `landed` is only ever an observation; nothing reopens.
//! * A quota is read from the records at the moment of the request and refused with its name, its limit, what is
//!   used, and what the person can do.
//! * `Sealed` comes from a block read from `log`; "pending" comes from `sync pending`; **`queued` is never read**.

use std::collections::BTreeSet;
use std::fmt;

use planeter_store::{ChangeState, IntentRecord, IntentState, ProposalPolicy, RevisionRecord};

use crate::read::Assurance;

// ------------------------------------------------------------------------------------------------ derivation

/// What a revision is made of, derived by [`derive_revision`]. The caller adds the ids and the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRevision {
    pub artifact_digest: String,
    /// What this accept newly stored, in a stable (sorted) order.
    pub added_patch_ids: Vec<String>,
    /// What the submitter declared; [`Declared::None`] for a push by plain prikk.
    pub declared: Declared,
}

/// Whether the revision carries a declared patch set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// No declaration: the revision lists only what the push added, and says so.
    None,
    /// The submitter's list, checked (sorted, without repeats).
    Ids(Vec<String>),
}

impl Declared {
    /// The form the store keeps: `None` for no declaration.
    pub fn into_stored(self) -> Option<Vec<String>> {
        match self {
            Declared::None => None,
            Declared::Ids(v) => Some(v),
        }
    }
}

/// Why a revision cannot be derived. Each variant is a named reason the edge can show (RFC 005 D-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeriveError {
    /// The intent is not `open` (it is complete, or abandoned): nothing is derived from it.
    IntentNotOpen(IntentState),
    /// The intent is already complete and has this revision: a repeat of the same artifact (idempotence).
    AlreadyRecorded(String),
    /// The accept added nothing the forge did not already hold: nothing is recorded.
    NoNewContent,
    /// The declaration leaves out patches the accept added (`declared ⊉ added`).
    DeclarationMissesAdded { missing: Vec<String> },
    /// The declaration names patches the forge does not hold.
    DeclaredNotHeld { ids: Vec<String> },
}

impl fmt::Display for DeriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeriveError::IntentNotOpen(s) => write!(f, "the intent is {}, not open", s.as_str()),
            DeriveError::AlreadyRecorded(id) => write!(f, "already recorded as revision {id}"),
            DeriveError::NoNewContent => f.write_str("the accept added nothing new"),
            DeriveError::DeclarationMissesAdded { missing } => {
                write!(
                    f,
                    "the declaration leaves out {} added patch(es)",
                    missing.len()
                )
            }
            DeriveError::DeclaredNotHeld { ids } => {
                write!(
                    f,
                    "the declaration names {} patch(es) the forge does not hold",
                    ids.len()
                )
            }
        }
    }
}

impl std::error::Error for DeriveError {}

/// Derive the revision of a **complete run** of accept (RFC 005 D-1).
///
/// * `intent` — the journal record of this accept. It must be `open`; a `complete` intent that has a revision
///   answers [`DeriveError::AlreadyRecorded`], any other state [`DeriveError::IntentNotOpen`].
/// * `pending_after` — what `sync pending` lists after the run exited 0.
/// * `held` — whether the forge holds a patch id (pending or reachable); supplied by the caller.
/// * `declaration` — the submitter's list of patch ids, or `None` for a push by plain prikk.
///
/// `added = pending_after − intent.pending_before`. The first attempt's list is the one in the record, so a
/// retry after a killed run derives the same prefix plus the rest, exactly once.
pub fn derive_revision(
    intent: &IntentRecord,
    pending_after: &BTreeSet<String>,
    held: &dyn Fn(&str) -> bool,
    declaration: Option<&[String]>,
) -> Result<NewRevision, DeriveError> {
    match intent.state {
        IntentState::Open => {}
        IntentState::Complete => {
            return Err(match &intent.revision_id {
                Some(id) => DeriveError::AlreadyRecorded(id.clone()),
                None => DeriveError::IntentNotOpen(IntentState::Complete),
            });
        }
        IntentState::Abandoned => return Err(DeriveError::IntentNotOpen(IntentState::Abandoned)),
    }
    let before: BTreeSet<&str> = intent.pending_before.iter().map(String::as_str).collect();
    let added: Vec<String> = pending_after
        .iter()
        .filter(|id| !before.contains(id.as_str()))
        .cloned()
        .collect();
    if added.is_empty() {
        return Err(DeriveError::NoNewContent);
    }
    let declared = match declaration {
        None => Declared::None,
        Some(list) => {
            let declared: BTreeSet<&str> = list.iter().map(String::as_str).collect();
            let missing: Vec<String> = added
                .iter()
                .filter(|id| !declared.contains(id.as_str()))
                .cloned()
                .collect();
            if !missing.is_empty() {
                return Err(DeriveError::DeclarationMissesAdded { missing });
            }
            let not_held: Vec<String> = declared
                .iter()
                .filter(|id| !held(id))
                .map(|id| (*id).to_owned())
                .collect();
            if !not_held.is_empty() {
                return Err(DeriveError::DeclaredNotHeld { ids: not_held });
            }
            Declared::Ids(declared.into_iter().map(str::to_owned).collect())
        }
    };
    Ok(NewRevision {
        artifact_digest: intent.artifact_digest.clone(),
        added_patch_ids: added,
        declared,
    })
}

/// Idempotence at the front door: an artifact whose digest already has a revision is that revision (RFC 005 D-1).
/// Call it before opening an intent; `existing` is the store's answer for the digest.
pub fn already_recorded(existing: Option<&RevisionRecord>) -> Result<(), DeriveError> {
    match existing {
        Some(r) => Err(DeriveError::AlreadyRecorded(r.revision_id.clone())),
        None => Ok(()),
    }
}

// ------------------------------------------------------------------------------------------------ base

/// What a revision's declared base amounts to (RFC 005 D-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseCheck {
    /// No base was declared: the revision is reviewable but its results never count (`base: none`).
    NoBase,
    /// The base is listed by `log` of a ref the forge holds.
    Held(String),
    /// The base is not held: refuse with "base unknown: push the base first".
    Unknown(String),
}

/// Check a declared base against the blocks `log` listed for the refs the forge holds. `show <base>` is never
/// the test (it fails for some blocks, U-9).
pub fn check_base(declared_base: Option<&str>, listed_blocks: &BTreeSet<String>) -> BaseCheck {
    match declared_base {
        None => BaseCheck::NoBase,
        Some(b) if listed_blocks.contains(b) => BaseCheck::Held(b.to_owned()),
        Some(b) => BaseCheck::Unknown(b.to_owned()),
    }
}

/// Whether a result or an approval given against a base still counts (RFC 005 D-7): only while the base is the
/// target's tip. No base never counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    Current,
    Stale,
    NoBase,
}

pub fn staleness(declared_base: Option<&str>, target_tip: Option<&str>) -> Staleness {
    match (declared_base, target_tip) {
        (None, _) => Staleness::NoBase,
        (Some(b), Some(t)) if b == t => Staleness::Current,
        _ => Staleness::Stale,
    }
}

/// An approval is bound to the revision it was given for and is never carried to a later one (RFC 005 D-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStanding {
    /// Given for the revision now current.
    Current,
    /// Given for an earlier revision: shown as "approved on revision N", counted for nothing now.
    OnEarlierRevision(u32),
}

pub fn approval_standing(approved_on_seq: u32, current_seq: u32) -> ApprovalStanding {
    if approved_on_seq == current_seq {
        ApprovalStanding::Current
    } else {
        ApprovalStanding::OnEarlierRevision(approved_on_seq)
    }
}

// ------------------------------------------------------------------------------------------------ states

/// The default grace, in seconds, an accepted artifact is kept after its change leaves `open` (30 days).
pub const DEFAULT_RETENTION_GRACE_SECS: u64 = 30 * 24 * 60 * 60;

/// What drives a change out of `open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// Prikk reported every patch of the current set no longer pending (see [`observe_landing`]).
    Landed,
    /// A principal the caller has **already authorized** (`Close`) closed it, with a reason.
    Closed { reason: String },
}

/// Why a transition is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The change is already `landed` or `closed`; neither is ever reopened or changed again.
    AlreadyFinal(ChangeState),
    /// A close needs a reason.
    ReasonRequired,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::AlreadyFinal(s) => {
                write!(f, "the change is {} and never changes again", s.as_str())
            }
            TransitionError::ReasonRequired => f.write_str("a change is closed with a reason"),
        }
    }
}

impl std::error::Error for TransitionError {}

/// The result of an allowed transition: what to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transitioned {
    pub state: ChangeState,
    pub closed_reason: Option<String>,
    /// When the artifacts may be reclaimed: `now + grace`.
    pub retain_until: u64,
}

/// The state machine of D-2: `open → landed` (by observation) and `open → closed` (with a reason); nothing leaves
/// `landed` or `closed`.
pub fn transition(
    from: ChangeState,
    to: &Transition,
    now: u64,
    grace_secs: u64,
) -> Result<Transitioned, TransitionError> {
    if from != ChangeState::Open {
        return Err(TransitionError::AlreadyFinal(from));
    }
    let retain_until = now.saturating_add(grace_secs);
    match to {
        Transition::Landed => Ok(Transitioned {
            state: ChangeState::Landed,
            closed_reason: None,
            retain_until,
        }),
        Transition::Closed { reason } => {
            if reason.trim().is_empty() {
                return Err(TransitionError::ReasonRequired);
            }
            Ok(Transitioned {
                state: ChangeState::Closed,
                closed_reason: Some(reason.clone()),
                retain_until,
            })
        }
    }
}

// ------------------------------------------------------------------------------------------------ landing

/// The change's **current patch set** (RFC 005 D-6, as decided in the review of part 3a): the **latest declared**
/// revision's declared set **plus the `added` sets of every revision after it**; when no revision is declared, the
/// union of all the `added` sets. A declared set therefore never drops out because a later push was undeclared, and
/// a change is never reported landed while an earlier declared patch is still pending. (A change with no revision
/// has an empty set, which is never observed as landed.)
pub fn current_patch_set(revisions: &[RevisionRecord]) -> BTreeSet<String> {
    let latest_declared = revisions
        .iter()
        .filter(|r| r.declared_patch_ids.is_some())
        .max_by_key(|r| r.seq);
    let mut set: BTreeSet<String> = BTreeSet::new();
    let after = match latest_declared {
        Some(r) => {
            set.extend(r.declared_patch_ids.iter().flatten().cloned());
            r.seq
        }
        None => 0,
    };
    for r in revisions.iter().filter(|r| r.seq > after) {
        set.extend(r.added_patch_ids.iter().cloned());
    }
    set
}

/// What an observation of `sync pending` says about a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// Every patch of the set has left the pending set.
    Landed,
    /// These are still pending.
    NotYet { remaining: Vec<String> },
}

/// Landing is **observed, never asserted** (D-6): the caller feeds the pending set it just read; this reports
/// what that observation implies. An empty patch set is never landed (nothing was observed to leave).
pub fn observe_landing(patch_set: &BTreeSet<String>, pending_now: &BTreeSet<String>) -> Landing {
    if patch_set.is_empty() {
        return Landing::NotYet {
            remaining: Vec::new(),
        };
    }
    let remaining: Vec<String> = patch_set
        .iter()
        .filter(|id| pending_now.contains(*id))
        .cloned()
        .collect();
    if remaining.is_empty() {
        Landing::Landed
    } else {
        Landing::NotYet { remaining }
    }
}

// ------------------------------------------------------------------------------------------------ quotas

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// The limits in force for one repository (RFC 005 Q-2): the column of the `proposals` setting, plus the
/// operator-set repository ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaPolicy {
    /// Open changes per principal per repository.
    pub open_changes: u64,
    /// Cumulative accepted bytes per principal per repository.
    pub accepted_bytes: u64,
    /// Revisions per change.
    pub revisions_per_change: u64,
    /// Bytes of every artifact the repository still retains — open changes and changes within their grace — the
    /// ceiling ("repository capacity").
    pub repo_retained_bytes: u64,
    /// How long an accepted artifact is kept after its change leaves `open`.
    pub retention_grace_secs: u64,
}

impl QuotaPolicy {
    /// The defaults of RFC 005 Q-2 for a repository's `proposals` setting.
    pub fn defaults_for(policy: ProposalPolicy) -> Self {
        match policy {
            ProposalPolicy::Members => Self {
                open_changes: 10,
                accepted_bytes: 512 * MIB,
                revisions_per_change: 20,
                repo_retained_bytes: 4 * GIB,
                retention_grace_secs: DEFAULT_RETENTION_GRACE_SECS,
            },
            ProposalPolicy::AnyAuthenticated => Self {
                open_changes: 3,
                accepted_bytes: 64 * MIB,
                revisions_per_change: 10,
                repo_retained_bytes: 4 * GIB,
                retention_grace_secs: DEFAULT_RETENTION_GRACE_SECS,
            },
        }
    }
}

/// What the records say is in use now (the store's usage queries; see [`planeter_store::Usage`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuotaUsage {
    pub open_changes: u64,
    pub accepted_bytes: u64,
    pub repo_retained_bytes: u64,
}

impl From<planeter_store::Usage> for QuotaUsage {
    fn from(u: planeter_store::Usage) -> Self {
        Self {
            open_changes: u.open_changes,
            accepted_bytes: u.accepted_bytes,
            repo_retained_bytes: u.repo_retained_bytes,
        }
    }
}

/// One push the quotas are asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaRequest {
    /// The artifact's size in bytes.
    pub bytes: u64,
    /// Which change it goes to.
    pub target: QuotaTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaTarget {
    /// The push opens a new change.
    NewChange,
    /// The push adds a revision to a change that already has this many.
    ExistingChange { revisions: u64 },
}

/// The quota that refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quota {
    OpenChanges,
    AcceptedBytes,
    RevisionsPerChange,
    RepositoryCapacity,
}

impl Quota {
    /// The stable name (the API's `quota` field).
    pub fn name(self) -> &'static str {
        match self {
            Quota::OpenChanges => "open_proposals",
            Quota::AcceptedBytes => "accepted_bytes",
            Quota::RevisionsPerChange => "revisions_per_proposal",
            Quota::RepositoryCapacity => "repository_capacity",
        }
    }
}

/// A refusal that names the quota, the limit, the current use and (through the catalog keys) what to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaRefusal {
    pub quota: Quota,
    /// The limit, in the quota's own unit (a count, or bytes).
    pub limit: u64,
    /// The current use, same unit.
    pub used: u64,
    /// What this request would have added, same unit.
    pub requested: u64,
}

impl QuotaRefusal {
    /// The catalog key of the sentence that names the quota. Placeholders: `{limit}` and `{used}`
    /// (see [`QuotaRefusal::figures`]).
    pub fn message_key(&self) -> &'static str {
        match self.quota {
            Quota::OpenChanges => "quota.open_proposals.message",
            Quota::AcceptedBytes => "quota.accepted_bytes.message",
            Quota::RevisionsPerChange => "quota.revisions.message",
            Quota::RepositoryCapacity => "quota.capacity.message",
        }
    }

    /// The catalog key of the one-sentence hint: what the person can do.
    pub fn hint_key(&self) -> &'static str {
        match self.quota {
            Quota::OpenChanges => "quota.open_proposals.hint",
            Quota::AcceptedBytes => "quota.accepted_bytes.hint",
            Quota::RevisionsPerChange => "quota.revisions.hint",
            Quota::RepositoryCapacity => "quota.capacity.hint",
        }
    }

    /// `(limit, used)` in the unit the sentence says: a count, or whole mebibytes (the limit rounded down, the
    /// use rounded up, so the sentence never claims more room than there is).
    pub fn figures(&self) -> (u64, u64) {
        match self.quota {
            Quota::OpenChanges | Quota::RevisionsPerChange => (self.limit, self.used),
            Quota::AcceptedBytes | Quota::RepositoryCapacity => {
                (self.limit / MIB, self.used.div_ceil(MIB))
            }
        }
    }

    /// Every catalog key this module can ask for (the web crate's tests check each exists in every language).
    pub const ALL_KEYS: [&'static str; 8] = [
        "quota.open_proposals.message",
        "quota.open_proposals.hint",
        "quota.accepted_bytes.message",
        "quota.accepted_bytes.hint",
        "quota.revisions.message",
        "quota.revisions.hint",
        "quota.capacity.message",
        "quota.capacity.hint",
    ];
}

/// Check one push against the quotas (RFC 005 D-9, Q-2). **At the limit is allowed; one over is refused.** The
/// order is the order a person can act in: the repository's capacity (nothing they can do but ask), then the
/// change's revisions, their open changes, their bytes. Per-push bounds are part 3b's edge, not here.
pub fn check_quota(
    usage: &QuotaUsage,
    policy: &QuotaPolicy,
    request: &QuotaRequest,
) -> Result<(), QuotaRefusal> {
    if usage.repo_retained_bytes.saturating_add(request.bytes) > policy.repo_retained_bytes {
        return Err(QuotaRefusal {
            quota: Quota::RepositoryCapacity,
            limit: policy.repo_retained_bytes,
            used: usage.repo_retained_bytes,
            requested: request.bytes,
        });
    }
    match request.target {
        QuotaTarget::ExistingChange { revisions } => {
            if revisions.saturating_add(1) > policy.revisions_per_change {
                return Err(QuotaRefusal {
                    quota: Quota::RevisionsPerChange,
                    limit: policy.revisions_per_change,
                    used: revisions,
                    requested: 1,
                });
            }
        }
        QuotaTarget::NewChange => {
            if usage.open_changes.saturating_add(1) > policy.open_changes {
                return Err(QuotaRefusal {
                    quota: Quota::OpenChanges,
                    limit: policy.open_changes,
                    used: usage.open_changes,
                    requested: 1,
                });
            }
        }
    }
    if usage.accepted_bytes.saturating_add(request.bytes) > policy.accepted_bytes {
        return Err(QuotaRefusal {
            quota: Quota::AcceptedBytes,
            limit: policy.accepted_bytes,
            used: usage.accepted_bytes,
            requested: request.bytes,
        });
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ assurance

/// What was read about one patch from prikk, in the order of trust. `queued` is carried because `show` reports it,
/// and is **deliberately never consulted**: a pending patch reports `queued: false` (U-10, IS-47).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchFacts {
    /// The patch id is in a block listed by `log` of a ref the forge holds.
    pub in_block_from_log: bool,
    /// `sync pending` lists the patch id.
    pub listed_pending: bool,
    /// What `show` reported as `queued`. Never read by [`patch_assurance`].
    pub reported_queued: bool,
}

/// The assurance of one patch (RFC 005 D-10): `Sealed` only from membership in a block read from `log`;
/// `ForgeApprovedUnsealed` only from `sync pending`; otherwise nothing is known, and `None` says so. A patch in
/// both (a block reaches it and it is still listed) is not claimed sealed until the listing agrees: pending wins,
/// the cautious reading.
pub fn patch_assurance(facts: &PatchFacts) -> Option<Assurance> {
    if facts.listed_pending {
        Some(Assurance::ForgeApprovedUnsealed)
    } else if facts.in_block_from_log {
        Some(Assurance::Sealed)
    } else {
        None
    }
}
