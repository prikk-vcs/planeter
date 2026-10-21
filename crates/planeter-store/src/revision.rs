//! The per-repository **revision counter** (RFC 004 D-2): a number planeter bumps, under the repository's W lease,
//! every time a write to the repository completes. It is the primary signal of the transport's state fingerprint
//! (the cache key of a fetch and its `ETag`); the transport also cross-checks the repository's files, so an
//! out-of-band write invalidates the cache too. A repository that has never been written has revision 0.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::params;

use crate::model::RepoId;
use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

/// Where revision counters are kept. Implementations must be safe to share across threads.
pub trait RevisionStore: Send + Sync {
    /// The repository's current revision (0 if it has none yet).
    fn get(&self, repo_id: &RepoId) -> Result<u64>;
    /// Add one to the repository's revision and return the new value.
    fn bump(&self, repo_id: &RepoId) -> Result<u64>;
}

/// In-memory counters (tests, development).
#[derive(Default)]
pub struct InMemoryRevisions {
    counters: Mutex<HashMap<String, u64>>,
}

impl InMemoryRevisions {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RevisionStore for InMemoryRevisions {
    fn get(&self, repo_id: &RepoId) -> Result<u64> {
        Ok(*self
            .counters
            .lock()
            .map_err(|_| StoreError::Backend("revisions lock poisoned".to_owned()))?
            .get(repo_id.as_str())
            .unwrap_or(&0))
    }

    fn bump(&self, repo_id: &RepoId) -> Result<u64> {
        let mut m = self
            .counters
            .lock()
            .map_err(|_| StoreError::Backend("revisions lock poisoned".to_owned()))?;
        let n = m.entry(repo_id.as_str().to_owned()).or_insert(0);
        *n += 1;
        Ok(*n)
    }
}

/// Counters in the shared SQLite database.
#[derive(Clone)]
pub struct SqliteRevisions {
    db: SqliteDb,
}

const REVISION_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repo_revisions (
    repo_id  TEXT PRIMARY KEY,
    revision INTEGER NOT NULL
);
";

impl SqliteRevisions {
    /// Open over `db`, creating the table if it is not there.
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(REVISION_SCHEMA)?;
        Ok(Self { db })
    }
}

impl RevisionStore for SqliteRevisions {
    fn get(&self, repo_id: &RepoId) -> Result<u64> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT revision FROM repo_revisions WHERE repo_id = ?1")
            .map_err(backend)?;
        let mut rows = stmt.query(params![repo_id.as_str()]).map_err(backend)?;
        match rows.next().map_err(backend)? {
            Some(r) => Ok(u64::try_from(r.get::<_, i64>(0).map_err(backend)?).unwrap_or(0)),
            None => Ok(0),
        }
    }

    fn bump(&self, repo_id: &RepoId) -> Result<u64> {
        let conn = self.db.lock()?;
        // One statement: an upsert that adds one, so two bumps never read the same value.
        conn.execute(
            "INSERT INTO repo_revisions (repo_id, revision) VALUES (?1, 1) \
             ON CONFLICT(repo_id) DO UPDATE SET revision = revision + 1",
            params![repo_id.as_str()],
        )
        .map_err(backend)?;
        let n: i64 = conn
            .query_row(
                "SELECT revision FROM repo_revisions WHERE repo_id = ?1",
                params![repo_id.as_str()],
                |r| r.get(0),
            )
            .map_err(backend)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}
