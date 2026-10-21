//! The records behind a proposed change (RFC 005 D-1…D-3, D-6, D-9) and the intent journal (RFC 004 D-3, D-6):
//! **changes**, their immutable **revisions**, the **intents** that make a revision derivable after a killed accept,
//! the per-repository **needs-recovery** flag, and the **usage queries** the quotas are computed from.
//!
//! Plain data and one trait, [`ChangeStore`], with an in-memory and a SQLite implementation that pass the same test
//! suite. **No history is copied** (NG-6): only ids. Nothing here calls prikk.
//!
//! # How the lists are stored, and why
//!
//! * **Patch ids and claim ids** (`pending_before`, `added_patch_ids`, `declared_patch_ids`, `claim_ids`) are
//!   64-character lowercase hexadecimal ids, packed into a **BLOB of 32 raw bytes each**: half the size of their
//!   text, order preserved, and a malformed id is refused at the door ([`StoreError::Invalid`]). `pending_before` can
//!   be large — an accept is bounded at 20,000 objects (RFC 004 D-5), so an intent's list is at most
//!   20,000 × 32 B = **640,000 bytes**; the row is written once (the first attempt's list is never overwritten).
//! * **Claim outcomes** and **author key ids** are short, heterogeneous lists (`ed25519-…` is not hex): JSON text.
//!
//! # The rules the store itself enforces
//!
//! `state` is one of three words (a CHECK constraint); `UNIQUE(change_id, seq)`; `UNIQUE(repo_id, artifact_digest)`
//! (an identical artifact is one revision); at most **one open intent per (repository, artifact)**; a change leaves
//! `open` once and never returns; and [`ChangeStore::complete_intent_with_revision`] inserts the revision (and the
//! change, if the push opened one) and closes the intent in **one transaction**: any failure leaves the intent
//! `open` and nothing else written.

use std::collections::BTreeMap;
use std::sync::Mutex;

use rusqlite::{OptionalExtension, params};

use crate::model::RepoId;
use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

// ------------------------------------------------------------------------------------------------ states

/// A change's state (RFC 005 D-2). `landed` and `closed` are final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeState {
    Open,
    Landed,
    Closed,
}

impl ChangeState {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeState::Open => "open",
            ChangeState::Landed => "landed",
            ChangeState::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(ChangeState::Open),
            "landed" => Ok(ChangeState::Landed),
            "closed" => Ok(ChangeState::Closed),
            other => Err(StoreError::Invalid(format!(
                "unknown change state {other:?}"
            ))),
        }
    }
}

/// An intent's state (RFC 004 D-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentState {
    Open,
    Complete,
    Abandoned,
}

impl IntentState {
    pub fn as_str(self) -> &'static str {
        match self {
            IntentState::Open => "open",
            IntentState::Complete => "complete",
            IntentState::Abandoned => "abandoned",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(IntentState::Open),
            "complete" => Ok(IntentState::Complete),
            "abandoned" => Ok(IntentState::Abandoned),
            other => Err(StoreError::Invalid(format!(
                "unknown intent state {other:?}"
            ))),
        }
    }
}

// ------------------------------------------------------------------------------------------------ records

/// A change (RFC 005 D-2). `description` is stored as text and rendered only through the sanitizer, later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRecord {
    pub change_id: String,
    pub repo_id: RepoId,
    pub title: String,
    pub description: String,
    /// The ref name the submitter intends (advisory only).
    pub target_ref: Option<String>,
    pub state: ChangeState,
    /// Why it was closed (`closed` only).
    pub closed_reason: Option<String>,
    pub opened_by: String,
    pub opened_at: u64,
    pub updated_at: u64,
    /// When its artifacts may be reclaimed: set when the change leaves `open`.
    pub retain_until: Option<u64>,
}

/// What accept reported for one claim: its id and the outcome as prikk said it (stored, not re-asserted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimOutcome {
    pub claim_id: String,
    pub outcome: String,
}

/// One revision of a change: one accepted artifact. Ids only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionRecord {
    pub revision_id: String,
    pub change_id: String,
    pub repo_id: RepoId,
    /// 1, 2, … within the change.
    pub seq: u32,
    pub artifact_digest: String,
    pub artifact_bytes: u64,
    /// The block id the submitter says it built against (D-3); `None` records `base: none`.
    pub declared_base: Option<String>,
    pub claim_ids: Vec<String>,
    /// What this accept newly stored (pending-after minus the intent's pending-before).
    pub added_patch_ids: Vec<String>,
    /// The patch ids the submitter says its artifact contains; `None` for a push by plain prikk.
    pub declared_patch_ids: Option<Vec<String>>,
    pub claim_outcomes: Vec<ClaimOutcome>,
    pub submitter: String,
    pub author_key_ids: Vec<String>,
    pub created_at: u64,
}

/// A journal entry for one artifact being accepted (RFC 004 D-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentRecord {
    pub intent_id: String,
    pub repo_id: RepoId,
    pub artifact_digest: String,
    /// The pending patch ids **before the first attempt** — never overwritten by a retry.
    pub pending_before: Vec<String>,
    pub principal: String,
    pub started_at: u64,
    pub attempts: u32,
    pub state: IntentState,
    pub completed_at: Option<u64>,
    pub revision_id: Option<String>,
    /// The size of the artifact being accepted, written when the intent is opened. While the intent is open and has
    /// no revision, this many bytes are **owed**: the patches are in the repository and cannot be removed, and the
    /// usage queries count them (RFC 004 part 4, IS-50). Rows made before this column existed read as 0.
    pub artifact_bytes: u64,
}

/// The repository's recovery flag (RFC 004 D-6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoFlags {
    pub needs_recovery: bool,
    pub reason: Option<String>,
    pub since: Option<u64>,
}

/// What the quotas are computed from (RFC 005 D-9), read straight from `changes` and `revisions`: there are no
/// separate counters, so they cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Open changes opened by the principal in the repository.
    pub open_changes: u64,
    /// Bytes of every artifact the principal has had accepted in the repository (they stay in the store).
    pub accepted_bytes: u64,
    /// Bytes of every artifact the repository **still retains**, whoever sent it: those of its open changes and those
    /// of changes that left `open` less than their grace ago (`retain_until` not yet passed). That is what occupies
    /// the disk (RFC 005 D-9; the review of part 3a).
    pub repo_retained_bytes: u64,
}

/// A failure point the tests can arm to prove atomicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailPoint {
    /// After the revision (and the change) were written, before the intent was closed.
    AfterRevisionInsert,
}

// ------------------------------------------------------------------------------------------------ ids

/// Pack 64-character lowercase hex ids into 32 raw bytes each.
pub fn pack_ids(ids: &[String]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(ids.len() * 32);
    for id in ids {
        let b = id.as_bytes();
        if b.len() != 64 || !b.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(StoreError::Invalid(
                "an id is 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        for pair in b.chunks(2) {
            let hex = |c: u8| if c <= b'9' { c - b'0' } else { c - b'a' + 10 };
            out.push(hex(pair[0]) << 4 | hex(pair[1]));
        }
    }
    Ok(out)
}

/// The inverse of [`pack_ids`].
pub fn unpack_ids(blob: &[u8]) -> Result<Vec<String>> {
    if !blob.len().is_multiple_of(32) {
        return Err(StoreError::Invalid(
            "a packed id list has a length that is a multiple of 32".to_owned(),
        ));
    }
    Ok(blob
        .chunks(32)
        .map(|c| c.iter().map(|b| format!("{b:02x}")).collect())
        .collect())
}

// ------------------------------------------------------------------------------------------------ the trait

/// Persistence of changes, revisions, intents and the recovery flag. Safe to share across threads.
pub trait ChangeStore: Send + Sync {
    // -- changes
    /// Insert a change. [`StoreError::Conflict`] if its id exists.
    fn create_change(&self, change: &ChangeRecord) -> Result<()>;
    fn get_change(&self, change_id: &str) -> Result<Option<ChangeRecord>>;
    /// The repository's changes, newest first, optionally only those in `state`.
    fn list_changes(
        &self,
        repo_id: &RepoId,
        state: Option<ChangeState>,
    ) -> Result<Vec<ChangeRecord>>;
    /// Move an `open` change to `landed` or `closed`. Any other transition is [`StoreError::Invalid`]; a
    /// change that is not `open` never changes again. `reason` is required for `closed`.
    fn leave_open(
        &self,
        change_id: &str,
        to: ChangeState,
        reason: Option<&str>,
        now: u64,
        retain_until: Option<u64>,
    ) -> Result<()>;

    // -- revisions
    /// The change's revisions in `seq` order.
    fn revisions_of(&self, change_id: &str) -> Result<Vec<RevisionRecord>>;
    /// The revision of an artifact (by digest) in a repository, if one was recorded.
    fn revision_by_digest(
        &self,
        repo_id: &RepoId,
        artifact_digest: &str,
    ) -> Result<Option<RevisionRecord>>;

    // -- the intent journal
    /// Journal an intent for an artifact **before** its accept starts. If an intent for the same artifact is already
    /// `open`, that one is **resumed**: `attempts += 1`, its `pending_before` **not overwritten**, and it is
    /// returned. Otherwise a new open intent with `attempts = 1` is written. A repository may hold open intents for
    /// different artifacts only if the caller allows it; serializing them is the W lease's job.
    fn open_or_resume_intent(
        &self,
        intent_id: &str,
        repo_id: &RepoId,
        artifact_digest: &str,
        artifact_bytes: u64,
        pending_before: &[String],
        principal: &str,
        now: u64,
    ) -> Result<IntentRecord>;
    fn get_intent(&self, intent_id: &str) -> Result<Option<IntentRecord>>;
    /// The repository's open intents, oldest first.
    fn open_intents(&self, repo_id: &RepoId) -> Result<Vec<IntentRecord>>;
    /// Give up an open intent. [`StoreError::Invalid`] unless it is open.
    fn abandon_intent(&self, intent_id: &str, now: u64) -> Result<()>;
    /// **Atomically**: insert `new_change` (if the push opened one) and `revision`, and close the intent as
    /// `complete` with the revision's id. The intent must be `open`; its repository, digest and the revision's must
    /// agree. Any failure — a constraint, an invalid id, an armed [`FailPoint`] — leaves the intent `open` and
    /// writes nothing.
    fn complete_intent_with_revision(
        &self,
        intent_id: &str,
        new_change: Option<&ChangeRecord>,
        revision: &RevisionRecord,
        now: u64,
    ) -> Result<()>;

    // -- the recovery flag
    fn repo_flags(&self, repo_id: &RepoId) -> Result<RepoFlags>;
    fn set_needs_recovery(&self, repo_id: &RepoId, reason: &str, now: u64) -> Result<()>;
    fn clear_needs_recovery(&self, repo_id: &RepoId) -> Result<()>;
    /// Every repository whose recovery flag is set, in a stable order.
    fn flagged_repos(&self) -> Result<Vec<RepoId>>;

    // -- usage (for the quotas)
    fn usage(&self, repo_id: &RepoId, principal: &str, now: u64) -> Result<Usage>;

    /// Test seam: arm or disarm a failure point.
    #[doc(hidden)]
    fn inject_failure(&self, point: FailPoint, on: bool);
}

// ------------------------------------------------------------------------------------------------ in memory

#[derive(Default, Clone)]
struct MemState {
    changes: BTreeMap<String, ChangeRecord>,
    revisions: Vec<RevisionRecord>,
    intents: Vec<IntentRecord>,
    flags: BTreeMap<String, RepoFlags>,
}

/// An in-memory [`ChangeStore`] (tests, development). Every write is validated before anything is changed, so a
/// failed write leaves no trace.
#[derive(Default)]
pub struct InMemoryChangeStore {
    state: Mutex<MemState>,
    fail_after_revision: std::sync::atomic::AtomicBool,
}

impl InMemoryChangeStore {
    pub fn new() -> Self {
        Self::default()
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemState>> {
        self.state
            .lock()
            .map_err(|_| StoreError::Backend("changes lock poisoned".to_owned()))
    }
}

fn check_ids(revision: &RevisionRecord, intent: Option<&IntentRecord>) -> Result<()> {
    pack_ids(&revision.claim_ids)?;
    pack_ids(&revision.added_patch_ids)?;
    if let Some(d) = &revision.declared_patch_ids {
        pack_ids(d)?;
    }
    if let Some(i) = intent {
        pack_ids(&i.pending_before)?;
    }
    if revision.seq < 1 {
        return Err(StoreError::Invalid(
            "a revision's seq starts at 1".to_owned(),
        ));
    }
    Ok(())
}

impl ChangeStore for InMemoryChangeStore {
    fn create_change(&self, c: &ChangeRecord) -> Result<()> {
        let mut m = self.lock()?;
        if m.changes.contains_key(&c.change_id) {
            return Err(StoreError::Conflict("change id".to_owned()));
        }
        m.changes.insert(c.change_id.clone(), c.clone());
        Ok(())
    }

    fn get_change(&self, id: &str) -> Result<Option<ChangeRecord>> {
        Ok(self.lock()?.changes.get(id).cloned())
    }

    fn list_changes(
        &self,
        repo_id: &RepoId,
        state: Option<ChangeState>,
    ) -> Result<Vec<ChangeRecord>> {
        let mut v: Vec<ChangeRecord> = self
            .lock()?
            .changes
            .values()
            .filter(|c| &c.repo_id == repo_id && state.is_none_or(|s| c.state == s))
            .cloned()
            .collect();
        v.sort_by(|a, b| (b.opened_at, &b.change_id).cmp(&(a.opened_at, &a.change_id)));
        Ok(v)
    }

    fn leave_open(
        &self,
        id: &str,
        to: ChangeState,
        reason: Option<&str>,
        now: u64,
        retain_until: Option<u64>,
    ) -> Result<()> {
        let mut m = self.lock()?;
        let c = m.changes.get_mut(id).ok_or(StoreError::NotFound)?;
        validate_leave(c.state, to, reason)?;
        c.state = to;
        c.closed_reason =
            (to == ChangeState::Closed).then(|| reason.unwrap_or_default().to_owned());
        c.updated_at = now;
        c.retain_until = retain_until;
        Ok(())
    }

    fn revisions_of(&self, id: &str) -> Result<Vec<RevisionRecord>> {
        let mut v: Vec<RevisionRecord> = self
            .lock()?
            .revisions
            .iter()
            .filter(|r| r.change_id == id)
            .cloned()
            .collect();
        v.sort_by_key(|r| r.seq);
        Ok(v)
    }

    fn revision_by_digest(&self, repo: &RepoId, digest: &str) -> Result<Option<RevisionRecord>> {
        Ok(self
            .lock()?
            .revisions
            .iter()
            .find(|r| &r.repo_id == repo && r.artifact_digest == digest)
            .cloned())
    }

    fn open_or_resume_intent(
        &self,
        intent_id: &str,
        repo: &RepoId,
        digest: &str,
        artifact_bytes: u64,
        before: &[String],
        principal: &str,
        now: u64,
    ) -> Result<IntentRecord> {
        pack_ids(before)?;
        let mut m = self.lock()?;
        if let Some(i) = m.intents.iter_mut().find(|i| {
            &i.repo_id == repo && i.artifact_digest == digest && i.state == IntentState::Open
        }) {
            i.attempts += 1;
            return Ok(i.clone());
        }
        if m.intents.iter().any(|i| i.intent_id == intent_id) {
            return Err(StoreError::Conflict("intent id".to_owned()));
        }
        let rec = IntentRecord {
            intent_id: intent_id.to_owned(),
            repo_id: repo.clone(),
            artifact_digest: digest.to_owned(),
            pending_before: before.to_vec(),
            principal: principal.to_owned(),
            started_at: now,
            attempts: 1,
            state: IntentState::Open,
            completed_at: None,
            revision_id: None,
            artifact_bytes,
        };
        m.intents.push(rec.clone());
        Ok(rec)
    }

    fn get_intent(&self, id: &str) -> Result<Option<IntentRecord>> {
        Ok(self
            .lock()?
            .intents
            .iter()
            .find(|i| i.intent_id == id)
            .cloned())
    }

    fn open_intents(&self, repo: &RepoId) -> Result<Vec<IntentRecord>> {
        let mut v: Vec<IntentRecord> = self
            .lock()?
            .intents
            .iter()
            .filter(|i| &i.repo_id == repo && i.state == IntentState::Open)
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.started_at, &a.intent_id).cmp(&(b.started_at, &b.intent_id)));
        Ok(v)
    }

    fn abandon_intent(&self, id: &str, now: u64) -> Result<()> {
        let mut m = self.lock()?;
        let i = m
            .intents
            .iter_mut()
            .find(|i| i.intent_id == id)
            .ok_or(StoreError::NotFound)?;
        if i.state != IntentState::Open {
            return Err(StoreError::Invalid(
                "only an open intent can be abandoned".to_owned(),
            ));
        }
        i.state = IntentState::Abandoned;
        i.completed_at = Some(now);
        Ok(())
    }

    fn complete_intent_with_revision(
        &self,
        intent_id: &str,
        new_change: Option<&ChangeRecord>,
        revision: &RevisionRecord,
        now: u64,
    ) -> Result<()> {
        let mut m = self.lock()?;
        // Validate everything first; only then write (a failure leaves the state untouched).
        let intent = m
            .intents
            .iter()
            .find(|i| i.intent_id == intent_id)
            .cloned()
            .ok_or(StoreError::NotFound)?;
        if intent.state != IntentState::Open {
            return Err(StoreError::Invalid(
                "only an open intent can be completed".to_owned(),
            ));
        }
        if intent.repo_id != revision.repo_id || intent.artifact_digest != revision.artifact_digest
        {
            return Err(StoreError::Invalid(
                "the revision is not of the intent's artifact".to_owned(),
            ));
        }
        check_ids(revision, Some(&intent))?;
        if let Some(c) = new_change {
            if m.changes.contains_key(&c.change_id) {
                return Err(StoreError::Conflict("change id".to_owned()));
            }
            if c.change_id != revision.change_id || c.repo_id != revision.repo_id {
                return Err(StoreError::Invalid(
                    "the change is not the revision's".to_owned(),
                ));
            }
        } else if !m.changes.contains_key(&revision.change_id) {
            return Err(StoreError::NotFound);
        }
        if m.revisions
            .iter()
            .any(|r| r.repo_id == revision.repo_id && r.artifact_digest == revision.artifact_digest)
        {
            return Err(StoreError::Duplicate(
                "an identical artifact is one revision",
            ));
        }
        if m.revisions
            .iter()
            .any(|r| r.change_id == revision.change_id && r.seq == revision.seq)
        {
            return Err(StoreError::Duplicate(
                "a change's revisions are numbered once",
            ));
        }
        if m.revisions
            .iter()
            .any(|r| r.revision_id == revision.revision_id)
        {
            return Err(StoreError::Conflict("revision id".to_owned()));
        }
        if self
            .fail_after_revision
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(StoreError::Backend(
                "injected failure after the revision insert".to_owned(),
            ));
        }
        if let Some(c) = new_change {
            m.changes.insert(c.change_id.clone(), c.clone());
        }
        m.revisions.push(revision.clone());
        let i = m
            .intents
            .iter_mut()
            .find(|i| i.intent_id == intent_id)
            .expect("checked above");
        i.state = IntentState::Complete;
        i.completed_at = Some(now);
        i.revision_id = Some(revision.revision_id.clone());
        if let Some(c) = m.changes.get_mut(&revision.change_id) {
            c.updated_at = now;
        }
        Ok(())
    }

    fn repo_flags(&self, repo: &RepoId) -> Result<RepoFlags> {
        Ok(self
            .lock()?
            .flags
            .get(repo.as_str())
            .cloned()
            .unwrap_or_default())
    }

    fn set_needs_recovery(&self, repo: &RepoId, reason: &str, now: u64) -> Result<()> {
        self.lock()?.flags.insert(
            repo.as_str().to_owned(),
            RepoFlags {
                needs_recovery: true,
                reason: Some(reason.to_owned()),
                since: Some(now),
            },
        );
        Ok(())
    }

    fn clear_needs_recovery(&self, repo: &RepoId) -> Result<()> {
        self.lock()?
            .flags
            .insert(repo.as_str().to_owned(), RepoFlags::default());
        Ok(())
    }

    fn flagged_repos(&self) -> Result<Vec<RepoId>> {
        Ok(self
            .lock()?
            .flags
            .iter()
            .filter(|(_, f)| f.needs_recovery)
            .map(|(r, _)| RepoId::new(r.clone()))
            .collect())
    }

    fn usage(&self, repo: &RepoId, principal: &str, now: u64) -> Result<Usage> {
        let m = self.lock()?;
        // An open intent has no revision: its artifact is owed (IS-50), counted until a revision or an
        // abandonment replaces it.
        let owed = |who: Option<&str>| -> u64 {
            m.intents
                .iter()
                .filter(|i| {
                    &i.repo_id == repo
                        && i.state == IntentState::Open
                        && who.is_none_or(|w| i.principal == w)
                })
                .map(|i| i.artifact_bytes)
                .sum()
        };
        let open = m
            .changes
            .values()
            .filter(|c| &c.repo_id == repo && c.state == ChangeState::Open);
        let retained: Vec<&ChangeRecord> = m
            .changes
            .values()
            .filter(|c| &c.repo_id == repo && retains(c.state, c.retain_until, now))
            .collect();
        Ok(Usage {
            open_changes: open.filter(|c| c.opened_by == principal).count() as u64,
            accepted_bytes: m
                .revisions
                .iter()
                .filter(|r| &r.repo_id == repo && r.submitter == principal)
                .map(|r| r.artifact_bytes)
                .sum::<u64>()
                + owed(Some(principal)),
            repo_retained_bytes: m
                .revisions
                .iter()
                .filter(|r| retained.iter().any(|c| c.change_id == r.change_id))
                .map(|r| r.artifact_bytes)
                .sum::<u64>()
                + owed(None),
        })
    }

    fn inject_failure(&self, point: FailPoint, on: bool) {
        match point {
            FailPoint::AfterRevisionInsert => self
                .fail_after_revision
                .store(on, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// Whether a change's artifacts are still retained at `now`: while it is open, and after it left `open` until its
/// `retain_until` (inclusive). A change that left `open` with no `retain_until` retains nothing.
fn retains(state: ChangeState, retain_until: Option<u64>, now: u64) -> bool {
    state == ChangeState::Open || retain_until.is_some_and(|t| now <= t)
}

fn validate_leave(from: ChangeState, to: ChangeState, reason: Option<&str>) -> Result<()> {
    if from != ChangeState::Open {
        return Err(StoreError::Invalid(
            "a change that is landed or closed never changes again".to_owned(),
        ));
    }
    match to {
        ChangeState::Open => Err(StoreError::Invalid(
            "a change cannot be reopened".to_owned(),
        )),
        ChangeState::Closed if reason.is_none_or(|r| r.trim().is_empty()) => Err(
            StoreError::Invalid("a change is closed with a reason".to_owned()),
        ),
        _ => Ok(()),
    }
}

// ------------------------------------------------------------------------------------------------ SQLite

/// The schema, as created (idempotent). Indexes serve the usage queries and the listings.
pub const CHANGE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS changes (
    change_id     TEXT PRIMARY KEY,
    repo_id       TEXT NOT NULL,
    title         TEXT NOT NULL,
    description   TEXT NOT NULL,
    target_ref    TEXT,
    state         TEXT NOT NULL CHECK (state IN ('open', 'landed', 'closed')),
    closed_reason TEXT,
    opened_by     TEXT NOT NULL,
    opened_at     INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    retain_until  INTEGER
);
CREATE INDEX IF NOT EXISTS changes_by_repo_state ON changes (repo_id, state);
CREATE INDEX IF NOT EXISTS changes_open_by_principal ON changes (repo_id, opened_by, state);

CREATE TABLE IF NOT EXISTS revisions (
    revision_id        TEXT PRIMARY KEY,
    change_id          TEXT NOT NULL REFERENCES changes (change_id),
    repo_id            TEXT NOT NULL,
    seq                INTEGER NOT NULL CHECK (seq >= 1),
    artifact_digest    TEXT NOT NULL,
    artifact_bytes     INTEGER NOT NULL CHECK (artifact_bytes >= 0),
    declared_base      TEXT,
    claim_ids          BLOB NOT NULL,      -- 32 raw bytes per id
    added_patch_ids    BLOB NOT NULL,      -- 32 raw bytes per id
    declared_patch_ids BLOB,               -- NULL: no declaration (a push by plain prikk)
    claim_outcomes     TEXT NOT NULL,      -- JSON
    submitter          TEXT NOT NULL,
    author_key_ids     TEXT NOT NULL,      -- JSON
    created_at         INTEGER NOT NULL,
    UNIQUE (change_id, seq),
    UNIQUE (repo_id, artifact_digest)
);
CREATE INDEX IF NOT EXISTS revisions_by_submitter ON revisions (repo_id, submitter);

CREATE TABLE IF NOT EXISTS intents (
    intent_id       TEXT PRIMARY KEY,
    repo_id         TEXT NOT NULL,
    artifact_digest TEXT NOT NULL,
    pending_before  BLOB NOT NULL,         -- 32 raw bytes per id; written once, by the first attempt
    principal       TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    attempts        INTEGER NOT NULL CHECK (attempts >= 1),
    state           TEXT NOT NULL CHECK (state IN ('open', 'complete', 'abandoned')),
    completed_at    INTEGER,
    revision_id     TEXT REFERENCES revisions (revision_id),
    artifact_bytes  INTEGER NOT NULL DEFAULT 0   -- owed while the intent is open (IS-50); 0 for rows made before
);
CREATE UNIQUE INDEX IF NOT EXISTS intents_one_open_per_artifact ON intents (repo_id, artifact_digest) WHERE state = 'open';

CREATE TABLE IF NOT EXISTS repo_flags (
    repo_id        TEXT PRIMARY KEY,
    needs_recovery INTEGER NOT NULL CHECK (needs_recovery IN (0, 1)),
    reason         TEXT,
    since          INTEGER
);
";

/// The SQLite [`ChangeStore`], over the server's shared database. Transactions are short and run under the
/// connection's mutex.
#[derive(Clone)]
pub struct SqliteChangeStore {
    db: SqliteDb,
    fail_after_revision: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SqliteChangeStore {
    /// Open over `db`, creating the tables and indexes if they are not there (idempotent).
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(CHANGE_SCHEMA)?;
        // A database made before the column existed gets it now (idempotent).
        db.ensure_column("intents", "artifact_bytes", "INTEGER NOT NULL DEFAULT 0")?;
        Ok(Self {
            db,
            fail_after_revision: Default::default(),
        })
    }
}

type ChangeRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
    i64,
    Option<i64>,
);
const SELECT_CHANGE: &str = "SELECT change_id, repo_id, title, description, target_ref, state, closed_reason, opened_by, opened_at, updated_at, retain_until FROM changes";

fn change_from(r: ChangeRow) -> Result<ChangeRecord> {
    Ok(ChangeRecord {
        change_id: r.0,
        repo_id: RepoId::new(r.1),
        title: r.2,
        description: r.3,
        target_ref: r.4,
        state: ChangeState::parse(&r.5)?,
        closed_reason: r.6,
        opened_by: r.7,
        opened_at: u64::try_from(r.8).unwrap_or(0),
        updated_at: u64::try_from(r.9).unwrap_or(0),
        retain_until: r.10.map(|v| u64::try_from(v).unwrap_or(0)),
    })
}

fn row_change(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ))
}

const SELECT_REVISION: &str = "SELECT revision_id, change_id, repo_id, seq, artifact_digest, artifact_bytes, declared_base, claim_ids, added_patch_ids, declared_patch_ids, claim_outcomes, submitter, author_key_ids, created_at FROM revisions";

fn revision_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<RevisionRecord>> {
    let claim_ids: Vec<u8> = r.get(7)?;
    let added: Vec<u8> = r.get(8)?;
    let declared: Option<Vec<u8>> = r.get(9)?;
    let outcomes: String = r.get(10)?;
    let keys: String = r.get(12)?;
    let rec = (|| -> Result<RevisionRecord> {
        let outcomes: Vec<(String, String)> = serde_json_lite::parse_pairs(&outcomes)
            .ok_or_else(|| StoreError::Invalid("claim_outcomes".to_owned()))?;
        Ok(RevisionRecord {
            revision_id: r.get(0).map_err(backend)?,
            change_id: r.get(1).map_err(backend)?,
            repo_id: RepoId::new(r.get::<_, String>(2).map_err(backend)?),
            seq: u32::try_from(r.get::<_, i64>(3).map_err(backend)?).unwrap_or(0),
            artifact_digest: r.get(4).map_err(backend)?,
            artifact_bytes: u64::try_from(r.get::<_, i64>(5).map_err(backend)?).unwrap_or(0),
            declared_base: r.get(6).map_err(backend)?,
            claim_ids: unpack_ids(&claim_ids)?,
            added_patch_ids: unpack_ids(&added)?,
            declared_patch_ids: declared.as_deref().map(unpack_ids).transpose()?,
            claim_outcomes: outcomes
                .into_iter()
                .map(|(claim_id, outcome)| ClaimOutcome { claim_id, outcome })
                .collect(),
            submitter: r.get(11).map_err(backend)?,
            author_key_ids: serde_json_lite::parse_strings(&keys)
                .ok_or_else(|| StoreError::Invalid("author_key_ids".to_owned()))?,
            created_at: u64::try_from(r.get::<_, i64>(13).map_err(backend)?).unwrap_or(0),
        })
    })();
    Ok(rec)
}

/// A tiny JSON writer/reader for the two short lists kept as text (no new dependency: the shapes are fixed).
mod serde_json_lite {
    pub fn write_strings(items: &[String]) -> String {
        let mut out = String::from("[");
        for (i, s) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            quote(s, &mut out);
        }
        out.push(']');
        out
    }

    pub fn write_pairs(items: &[(String, String)]) -> String {
        let mut out = String::from("[");
        for (i, (a, b)) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            quote(a, &mut out);
            out.push(',');
            quote(b, &mut out);
            out.push(']');
        }
        out.push(']');
        out
    }

    fn quote(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    }

    struct Cur<'a> {
        s: &'a [u8],
        i: usize,
    }

    impl Cur<'_> {
        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }
        fn eat(&mut self, c: u8) -> bool {
            if self.peek() == Some(c) {
                self.i += 1;
                true
            } else {
                false
            }
        }
        fn string(&mut self) -> Option<String> {
            if !self.eat(b'"') {
                return None;
            }
            let mut out = Vec::new();
            loop {
                let c = self.peek()?;
                self.i += 1;
                match c {
                    b'"' => return String::from_utf8(out).ok(),
                    b'\\' => {
                        let e = self.peek()?;
                        self.i += 1;
                        match e {
                            b'"' => out.push(b'"'),
                            b'\\' => out.push(b'\\'),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let hex =
                                    std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                                self.i += 4;
                                let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                            _ => return None,
                        }
                    }
                    c => out.push(c),
                }
            }
        }
    }

    pub fn parse_strings(s: &str) -> Option<Vec<String>> {
        let mut c = Cur {
            s: s.as_bytes(),
            i: 0,
        };
        let mut out = Vec::new();
        if !c.eat(b'[') {
            return None;
        }
        if c.eat(b']') {
            return (c.i == s.len()).then_some(out);
        }
        loop {
            out.push(c.string()?);
            if c.eat(b']') {
                return (c.i == s.len()).then_some(out);
            }
            if !c.eat(b',') {
                return None;
            }
        }
    }

    pub fn parse_pairs(s: &str) -> Option<Vec<(String, String)>> {
        let mut c = Cur {
            s: s.as_bytes(),
            i: 0,
        };
        let mut out = Vec::new();
        if !c.eat(b'[') {
            return None;
        }
        if c.eat(b']') {
            return (c.i == s.len()).then_some(out);
        }
        loop {
            if !c.eat(b'[') {
                return None;
            }
            let a = c.string()?;
            if !c.eat(b',') {
                return None;
            }
            let b = c.string()?;
            if !c.eat(b']') {
                return None;
            }
            out.push((a, b));
            if c.eat(b']') {
                return (c.i == s.len()).then_some(out);
            }
            if !c.eat(b',') {
                return None;
            }
        }
    }
}

fn unique_violation(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

fn map_insert(e: rusqlite::Error, rule: &'static str) -> StoreError {
    if unique_violation(&e) {
        StoreError::Duplicate(rule)
    } else {
        backend(e)
    }
}

fn intent_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<IntentRecord>> {
    let blob: Vec<u8> = r.get(3)?;
    let state: String = r.get(7)?;
    let rec = (|| -> Result<IntentRecord> {
        Ok(IntentRecord {
            intent_id: r.get(0).map_err(backend)?,
            repo_id: RepoId::new(r.get::<_, String>(1).map_err(backend)?),
            artifact_digest: r.get(2).map_err(backend)?,
            pending_before: unpack_ids(&blob)?,
            principal: r.get(4).map_err(backend)?,
            started_at: u64::try_from(r.get::<_, i64>(5).map_err(backend)?).unwrap_or(0),
            attempts: u32::try_from(r.get::<_, i64>(6).map_err(backend)?).unwrap_or(0),
            state: IntentState::parse(&state)?,
            completed_at: r
                .get::<_, Option<i64>>(8)
                .map_err(backend)?
                .map(|v| u64::try_from(v).unwrap_or(0)),
            revision_id: r.get(9).map_err(backend)?,
            artifact_bytes: u64::try_from(r.get::<_, i64>(10).map_err(backend)?).unwrap_or(0),
        })
    })();
    Ok(rec)
}

const SELECT_INTENT: &str = "SELECT intent_id, repo_id, artifact_digest, pending_before, principal, started_at, attempts, state, completed_at, revision_id, artifact_bytes FROM intents";

impl ChangeStore for SqliteChangeStore {
    fn create_change(&self, c: &ChangeRecord) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "INSERT INTO changes (change_id, repo_id, title, description, target_ref, state, closed_reason, opened_by, opened_at, updated_at, retain_until) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![c.change_id, c.repo_id.as_str(), c.title, c.description, c.target_ref, c.state.as_str(), c.closed_reason, c.opened_by, c.opened_at as i64, c.updated_at as i64, c.retain_until.map(|v| v as i64)],
            )
            .map_err(|e| if unique_violation(&e) { StoreError::Conflict("change id".to_owned()) } else { backend(e) })?;
        Ok(())
    }

    fn get_change(&self, id: &str) -> Result<Option<ChangeRecord>> {
        let conn = self.db.lock()?;
        let row = conn
            .query_row(
                &format!("{SELECT_CHANGE} WHERE change_id = ?1"),
                params![id],
                row_change,
            )
            .optional()
            .map_err(backend)?;
        row.map(change_from).transpose()
    }

    fn list_changes(
        &self,
        repo_id: &RepoId,
        state: Option<ChangeState>,
    ) -> Result<Vec<ChangeRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(&format!("{SELECT_CHANGE} WHERE repo_id = ?1 AND (?2 IS NULL OR state = ?2) ORDER BY opened_at DESC, change_id DESC"))
            .map_err(backend)?;
        let rows = stmt
            .query_map(
                params![repo_id.as_str(), state.map(ChangeState::as_str)],
                row_change,
            )
            .map_err(backend)?;
        rows.map(|r| change_from(r.map_err(backend)?)).collect()
    }

    fn leave_open(
        &self,
        id: &str,
        to: ChangeState,
        reason: Option<&str>,
        now: u64,
        retain_until: Option<u64>,
    ) -> Result<()> {
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        let state: Option<String> = tx
            .query_row(
                "SELECT state FROM changes WHERE change_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        let from = ChangeState::parse(&state.ok_or(StoreError::NotFound)?)?;
        validate_leave(from, to, reason)?;
        tx.execute(
            "UPDATE changes SET state = ?2, closed_reason = ?3, updated_at = ?4, retain_until = ?5 WHERE change_id = ?1",
            params![id, to.as_str(), (to == ChangeState::Closed).then(|| reason.unwrap_or_default().to_owned()), now as i64, retain_until.map(|v| v as i64)],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn revisions_of(&self, id: &str) -> Result<Vec<RevisionRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(&format!(
                "{SELECT_REVISION} WHERE change_id = ?1 ORDER BY seq"
            ))
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id], revision_from)
            .map_err(backend)?;
        rows.map(|r| r.map_err(backend)?).collect()
    }

    fn revision_by_digest(&self, repo: &RepoId, digest: &str) -> Result<Option<RevisionRecord>> {
        let conn = self.db.lock()?;
        let row = conn
            .query_row(
                &format!("{SELECT_REVISION} WHERE repo_id = ?1 AND artifact_digest = ?2"),
                params![repo.as_str(), digest],
                revision_from,
            )
            .optional()
            .map_err(backend)?;
        row.transpose()
    }

    fn open_or_resume_intent(
        &self,
        intent_id: &str,
        repo: &RepoId,
        digest: &str,
        artifact_bytes: u64,
        before: &[String],
        principal: &str,
        now: u64,
    ) -> Result<IntentRecord> {
        let blob = pack_ids(before)?;
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        let existing: Option<String> = tx
            .query_row("SELECT intent_id FROM intents WHERE repo_id = ?1 AND artifact_digest = ?2 AND state = 'open'", params![repo.as_str(), digest], |r| r.get(0))
            .optional()
            .map_err(backend)?;
        let id = match existing {
            Some(existing) => {
                tx.execute(
                    "UPDATE intents SET attempts = attempts + 1 WHERE intent_id = ?1",
                    params![existing],
                )
                .map_err(backend)?;
                existing
            }
            None => {
                tx.execute(
                    "INSERT INTO intents (intent_id, repo_id, artifact_digest, pending_before, principal, started_at, attempts, state, artifact_bytes) VALUES (?1,?2,?3,?4,?5,?6,1,'open',?7)",
                    params![intent_id, repo.as_str(), digest, blob, principal, now as i64, artifact_bytes as i64],
                )
                .map_err(|e| if unique_violation(&e) { StoreError::Conflict("intent id".to_owned()) } else { backend(e) })?;
                intent_id.to_owned()
            }
        };
        let rec = tx
            .query_row(
                &format!("{SELECT_INTENT} WHERE intent_id = ?1"),
                params![id],
                intent_from,
            )
            .map_err(backend)??;
        tx.commit().map_err(backend)?;
        Ok(rec)
    }

    fn get_intent(&self, id: &str) -> Result<Option<IntentRecord>> {
        let conn = self.db.lock()?;
        let row = conn
            .query_row(
                &format!("{SELECT_INTENT} WHERE intent_id = ?1"),
                params![id],
                intent_from,
            )
            .optional()
            .map_err(backend)?;
        row.transpose()
    }

    fn open_intents(&self, repo: &RepoId) -> Result<Vec<IntentRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(&format!("{SELECT_INTENT} WHERE repo_id = ?1 AND state = 'open' ORDER BY started_at, intent_id"))
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![repo.as_str()], intent_from)
            .map_err(backend)?;
        rows.map(|r| r.map_err(backend)?).collect()
    }

    fn abandon_intent(&self, id: &str, now: u64) -> Result<()> {
        let conn = self.db.lock()?;
        let n = conn
            .execute("UPDATE intents SET state = 'abandoned', completed_at = ?2 WHERE intent_id = ?1 AND state = 'open'", params![id, now as i64])
            .map_err(backend)?;
        if n == 1 {
            return Ok(());
        }
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM intents WHERE intent_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        match exists {
            None => Err(StoreError::NotFound),
            Some(_) => Err(StoreError::Invalid(
                "only an open intent can be abandoned".to_owned(),
            )),
        }
    }

    fn complete_intent_with_revision(
        &self,
        intent_id: &str,
        new_change: Option<&ChangeRecord>,
        revision: &RevisionRecord,
        now: u64,
    ) -> Result<()> {
        // Everything that can be refused without the database is refused first.
        let claim_ids = pack_ids(&revision.claim_ids)?;
        let added = pack_ids(&revision.added_patch_ids)?;
        let declared = revision
            .declared_patch_ids
            .as_deref()
            .map(pack_ids)
            .transpose()?;
        if revision.seq < 1 {
            return Err(StoreError::Invalid(
                "a revision's seq starts at 1".to_owned(),
            ));
        }
        let outcomes = serde_json_lite::write_pairs(
            &revision
                .claim_outcomes
                .iter()
                .map(|o| (o.claim_id.clone(), o.outcome.clone()))
                .collect::<Vec<_>>(),
        );
        let keys = serde_json_lite::write_strings(&revision.author_key_ids);
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        let intent = tx
            .query_row(
                &format!("{SELECT_INTENT} WHERE intent_id = ?1"),
                params![intent_id],
                intent_from,
            )
            .optional()
            .map_err(backend)?
            .transpose()?
            .ok_or(StoreError::NotFound)?;
        if intent.state != IntentState::Open {
            return Err(StoreError::Invalid(
                "only an open intent can be completed".to_owned(),
            ));
        }
        if intent.repo_id != revision.repo_id || intent.artifact_digest != revision.artifact_digest
        {
            return Err(StoreError::Invalid(
                "the revision is not of the intent's artifact".to_owned(),
            ));
        }
        if let Some(c) = new_change {
            if c.change_id != revision.change_id || c.repo_id != revision.repo_id {
                return Err(StoreError::Invalid(
                    "the change is not the revision's".to_owned(),
                ));
            }
            tx.execute(
                "INSERT INTO changes (change_id, repo_id, title, description, target_ref, state, closed_reason, opened_by, opened_at, updated_at, retain_until) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![c.change_id, c.repo_id.as_str(), c.title, c.description, c.target_ref, c.state.as_str(), c.closed_reason, c.opened_by, c.opened_at as i64, c.updated_at as i64, c.retain_until.map(|v| v as i64)],
            )
            .map_err(|e| if unique_violation(&e) { StoreError::Conflict("change id".to_owned()) } else { backend(e) })?;
        } else {
            let known: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM changes WHERE change_id = ?1",
                    params![revision.change_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(backend)?;
            if known.is_none() {
                return Err(StoreError::NotFound);
            }
        }
        // The two uniqueness rules are told apart by checking before the insert, so each refusal is typed.
        let same_artifact: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM revisions WHERE repo_id = ?1 AND artifact_digest = ?2",
                params![revision.repo_id.as_str(), revision.artifact_digest],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if same_artifact.is_some() {
            return Err(StoreError::Duplicate(
                "an identical artifact is one revision",
            ));
        }
        let same_seq: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM revisions WHERE change_id = ?1 AND seq = ?2",
                params![revision.change_id, revision.seq as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if same_seq.is_some() {
            return Err(StoreError::Duplicate(
                "a change's revisions are numbered once",
            ));
        }
        tx.execute(
            "INSERT INTO revisions (revision_id, change_id, repo_id, seq, artifact_digest, artifact_bytes, declared_base, claim_ids, added_patch_ids, declared_patch_ids, claim_outcomes, submitter, author_key_ids, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![revision.revision_id, revision.change_id, revision.repo_id.as_str(), revision.seq as i64, revision.artifact_digest, revision.artifact_bytes as i64, revision.declared_base, claim_ids, added, declared, outcomes, revision.submitter, keys, revision.created_at as i64],
        )
        .map_err(|e| map_insert(e, "a revision's identity and numbering are unique"))?;
        if self
            .fail_after_revision
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            // Dropping the transaction rolls everything back.
            return Err(StoreError::Backend(
                "injected failure after the revision insert".to_owned(),
            ));
        }
        tx.execute("UPDATE intents SET state = 'complete', completed_at = ?2, revision_id = ?3 WHERE intent_id = ?1", params![intent_id, now as i64, revision.revision_id]).map_err(backend)?;
        tx.execute(
            "UPDATE changes SET updated_at = ?2 WHERE change_id = ?1",
            params![revision.change_id, now as i64],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn repo_flags(&self, repo: &RepoId) -> Result<RepoFlags> {
        let conn = self.db.lock()?;
        let row: Option<(i64, Option<String>, Option<i64>)> = conn
            .query_row(
                "SELECT needs_recovery, reason, since FROM repo_flags WHERE repo_id = ?1",
                params![repo.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(backend)?;
        Ok(
            row.map_or_else(RepoFlags::default, |(n, reason, since)| RepoFlags {
                needs_recovery: n == 1,
                reason,
                since: since.map(|v| u64::try_from(v).unwrap_or(0)),
            }),
        )
    }

    fn set_needs_recovery(&self, repo: &RepoId, reason: &str, now: u64) -> Result<()> {
        self.db
            .lock()?
            .execute("INSERT INTO repo_flags (repo_id, needs_recovery, reason, since) VALUES (?1, 1, ?2, ?3) ON CONFLICT(repo_id) DO UPDATE SET needs_recovery = 1, reason = ?2, since = ?3", params![repo.as_str(), reason, now as i64])
            .map_err(backend)?;
        Ok(())
    }

    fn clear_needs_recovery(&self, repo: &RepoId) -> Result<()> {
        self.db
            .lock()?
            .execute("INSERT INTO repo_flags (repo_id, needs_recovery, reason, since) VALUES (?1, 0, NULL, NULL) ON CONFLICT(repo_id) DO UPDATE SET needs_recovery = 0, reason = NULL, since = NULL", params![repo.as_str()])
            .map_err(backend)?;
        Ok(())
    }

    fn flagged_repos(&self) -> Result<Vec<RepoId>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT repo_id FROM repo_flags WHERE needs_recovery = 1 ORDER BY repo_id")
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        rows.map(|r| r.map(RepoId::new).map_err(backend)).collect()
    }

    fn usage(&self, repo: &RepoId, principal: &str, now: u64) -> Result<Usage> {
        let conn = self.db.lock()?;
        let owed_by: i64 = conn
            .query_row("SELECT COALESCE(SUM(artifact_bytes), 0) FROM intents WHERE repo_id = ?1 AND state = 'open' AND principal = ?2", params![repo.as_str(), principal], |r| r.get(0))
            .map_err(backend)?;
        let owed_all: i64 = conn
            .query_row("SELECT COALESCE(SUM(artifact_bytes), 0) FROM intents WHERE repo_id = ?1 AND state = 'open'", params![repo.as_str()], |r| r.get(0))
            .map_err(backend)?;
        let open_changes: i64 = conn
            .query_row("SELECT COUNT(*) FROM changes WHERE repo_id = ?1 AND opened_by = ?2 AND state = 'open'", params![repo.as_str(), principal], |r| r.get(0))
            .map_err(backend)?;
        let accepted: i64 = conn
            .query_row("SELECT COALESCE(SUM(artifact_bytes), 0) FROM revisions WHERE repo_id = ?1 AND submitter = ?2", params![repo.as_str(), principal], |r| r.get(0))
            .map_err(backend)?;
        let retained: i64 = conn
            .query_row("SELECT COALESCE(SUM(r.artifact_bytes), 0) FROM revisions r JOIN changes c ON c.change_id = r.change_id WHERE r.repo_id = ?1 AND (c.state = 'open' OR (c.retain_until IS NOT NULL AND c.retain_until >= ?2))", params![repo.as_str(), now as i64], |r| r.get(0))
            .map_err(backend)?;
        Ok(Usage {
            open_changes: u64::try_from(open_changes).unwrap_or(0),
            accepted_bytes: u64::try_from(accepted.saturating_add(owed_by)).unwrap_or(0),
            repo_retained_bytes: u64::try_from(retained.saturating_add(owed_all)).unwrap_or(0),
        })
    }

    fn inject_failure(&self, point: FailPoint, on: bool) {
        match point {
            FailPoint::AfterRevisionInsert => self
                .fail_after_revision
                .store(on, std::sync::atomic::Ordering::Relaxed),
        }
    }
}
