//! The [`RepositoryStore`] trait — persistence of repository records, behind an abstraction so the
//! backend is swappable (RFC 001 IQ-4: SQLite-first, PostgreSQL path; the concrete engine is deferred
//! behind this trait). An in-memory implementation lives in [`crate::memory`]; a SQLite backend is the
//! next increment.

use std::fmt;

use crate::model::{Owner, RepoId, RepositoryRecord};

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
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Conflict(what) => write!(f, "conflict: {what}"),
            StoreError::NotFound => f.write_str("repository record not found"),
            StoreError::Backend(e) => write!(f, "store backend error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Persistence of [`RepositoryRecord`]s. Implementations must be safe to share across threads (a forge
/// is concurrent) — hence the `Send + Sync` bound.
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

    /// Remove a record (the caller removes the on-disk repository separately).
    fn delete(&self, repo_id: &RepoId) -> Result<()>;

    /// All records owned by `owner`.
    fn list_by_owner(&self, owner: &Owner) -> Result<Vec<RepositoryRecord>>;
}
