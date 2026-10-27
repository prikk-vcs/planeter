//! The [`RepositoryStore`] trait — persistence of repository records, behind an abstraction so the
//! backend is swappable (RFC 001 IQ-4: SQLite-first, PostgreSQL path; the concrete engine is deferred
//! behind this trait). An in-memory implementation lives in [`crate::memory`]; a SQLite backend is the
//! next increment.

use std::fmt;

use crate::model::{Owner, ProposalPolicy, RepoId, RepositoryRecord};

/// A repository-store failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum StoreError {
    /// A record with this `repo_id`, or this `owner/name`, already exists.
    Conflict(String),
    /// No record for the given key.
    NotFound,
    /// A backend (I/O, database) failure.
    Backend(String),
    /// A uniqueness rule was broken (RFC 005 D-1: an identical artifact is one revision; a change's revisions are
    /// numbered once; one open intent per artifact). The text names the rule, never a value.
    Duplicate(&'static str),
    /// A value the store refuses to keep (a malformed id, a state it does not know, an illegal transition).
    Invalid(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Conflict(what) => write!(f, "conflict: {what}"),
            StoreError::NotFound => f.write_str("repository record not found"),
            StoreError::Backend(e) => write!(f, "store backend error: {e}"),
            StoreError::Duplicate(rule) => write!(f, "duplicate: {rule}"),
            StoreError::Invalid(what) => write!(f, "invalid: {what}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Persistence of [`RepositoryRecord`]s. Implementations must be safe to share across threads (a forge
/// is concurrent) — hence the `Send + Sync` bound.
///
/// A record is `ready` or `creating` (RFC 004 part 7). Everything that reads a repository by id or name sees `ready`
/// records only; [`RepositoryStore::delete`] removes a record in either state.
pub trait RepositoryStore: Send + Sync {
    /// Insert a new record. Fails with [`StoreError::Conflict`] if its `repo_id` or its `owner/name`
    /// already exists.
    fn create(&self, record: RepositoryRecord) -> Result<()>;

    /// Fetch a record by its stable id.
    fn get(&self, repo_id: &RepoId) -> Result<Option<RepositoryRecord>>;

    /// Resolve an `owner/name` (the URL identity) to a repository id.
    fn resolve(&self, owner: &Owner, name: &str) -> Result<Option<RepoId>>;

    /// Rename and/or transfer a repository: change its `owner`/`name` while leaving `repo_id` and `path`
    /// **unchanged** (no bytes move — RFC 001 D-5). Fails [`StoreError::NotFound`] if the id is unknown,
    /// or [`StoreError::Conflict`] if the target `owner/name` is taken.
    fn rename(&self, repo_id: &RepoId, new_owner: Owner, new_name: String) -> Result<()>;

    /// Record (or update) the prikk on-disk format version this repo is hosted in (RFC 009).
    fn set_prikk_format_version(&self, repo_id: &RepoId, version: String) -> Result<()>;

    /// Set who may propose a change to this repository (RFC 005 D-9). Fails [`StoreError::NotFound`] if the id
    /// is unknown.
    fn set_proposals(&self, repo_id: &RepoId, policy: ProposalPolicy) -> Result<()>;

    /// Remove a record in either state (the caller removes the on-disk repository separately — **first**, so that a
    /// record never outlives its directory's absence while a directory outlives its record).
    fn delete(&self, repo_id: &RepoId) -> Result<()>;

    /// All **ready** records owned by `owner`.
    fn list_by_owner(&self, owner: &Owner) -> Result<Vec<RepositoryRecord>>;

    /// Reserve a name for a repository that is being created (RFC 004 part 7, T1): the record is stored in the state
    /// `creating`. A `creating` repository is **invisible**: [`RepositoryStore::get`], [`RepositoryStore::resolve`] and
    /// [`RepositoryStore::list_by_owner`] answer exactly as for an absent one, so no gate, route or listing can reach it.
    /// The name is taken from this moment (a second reservation or a [`RepositoryStore::create`] of the same
    /// `owner/name` fails [`StoreError::Conflict`]), checked before any byte of the repository is written.
    fn reserve(&self, record: RepositoryRecord) -> Result<()>;

    /// The last step of a creation: `creating` → `ready`. Fails [`StoreError::NotFound`] when no repository with this id is
    /// `creating` (a record that is already `ready`, or none).
    fn mark_ready(&self, repo_id: &RepoId) -> Result<()>;

    /// Every record in the state `creating` (the start-up sweep removes them: a creation that did not finish is
    /// removed whole).
    fn creating(&self) -> Result<Vec<RepositoryRecord>>;

    /// The ids of every record in either state (the sweep removes a directory that is in neither).
    fn known_ids(&self) -> Result<Vec<RepoId>>;

    /// How many repositories `owner` has: ready ones **and** the ones being created (a reservation holds its place, so
    /// two creations racing cannot both slip under a quota).
    fn count_owned(&self, owner: &Owner) -> Result<u64>;
}
