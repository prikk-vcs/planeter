//! The durable record of who holds a repository's exclusive lease (RFC 004 D-7).
//!
//! The lease itself lives in `planeter-core` and in memory. What survives a restart is this **record**: one row
//! per holder of an `X` or `W` lease, written when the lease is granted and removed when it is released. After a
//! crash the rows of the dead process are still here, so recovery (RFC 004 part 4) can see that a holder died and
//! what it was doing. A row is told apart from a live one by its **boot id**, a random value each server process
//! picks at start: a row whose boot id is not the current one belongs to a process that no longer exists.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::params;

use crate::model::RepoId;
use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

/// One holder of an exclusive lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderRecord {
    pub repo_id: RepoId,
    /// `"X"` (export) or `"W"` (write).
    pub class: String,
    /// What the holder is doing (`"sync accept"`, `"archive export"`, …): a short label, never user input.
    pub operation: String,
    /// Unique per grant.
    pub holder_id: String,
    /// The server process that granted it.
    pub boot_id: String,
    /// Unix seconds at the grant.
    pub since_unix: u64,
}

/// Where holder records are kept. Implementations must be safe to share across threads.
pub trait LeaseRecordStore: Send + Sync {
    /// Record a grant.
    fn record(&self, holder: &HolderRecord) -> Result<()>;
    /// Remove a holder's record (a no-op if it is already gone).
    fn clear(&self, holder_id: &str) -> Result<()>;
    /// Every record, oldest first.
    fn list(&self) -> Result<Vec<HolderRecord>>;
}

/// An in-memory record store (tests, development).
#[derive(Default)]
pub struct InMemoryLeaseRecords {
    rows: Mutex<HashMap<String, HolderRecord>>,
}

impl InMemoryLeaseRecords {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LeaseRecordStore for InMemoryLeaseRecords {
    fn record(&self, holder: &HolderRecord) -> Result<()> {
        self.rows
            .lock()
            .map_err(|_| StoreError::Backend("lease records lock poisoned".to_owned()))?
            .insert(holder.holder_id.clone(), holder.clone());
        Ok(())
    }

    fn clear(&self, holder_id: &str) -> Result<()> {
        self.rows
            .lock()
            .map_err(|_| StoreError::Backend("lease records lock poisoned".to_owned()))?
            .remove(holder_id);
        Ok(())
    }

    fn list(&self) -> Result<Vec<HolderRecord>> {
        let mut v: Vec<HolderRecord> = self
            .rows
            .lock()
            .map_err(|_| StoreError::Backend("lease records lock poisoned".to_owned()))?
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.since_unix, &a.holder_id).cmp(&(b.since_unix, &b.holder_id)));
        Ok(v)
    }
}

/// Holder records in the shared SQLite database.
#[derive(Clone)]
pub struct SqliteLeaseRecords {
    db: SqliteDb,
}

const LEASE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS lease_holders (
    holder_id  TEXT PRIMARY KEY,
    repo_id    TEXT NOT NULL,
    class      TEXT NOT NULL,        -- 'X' | 'W'
    operation  TEXT NOT NULL,
    boot_id    TEXT NOT NULL,
    since_unix INTEGER NOT NULL
);
";

impl SqliteLeaseRecords {
    /// Open over `db`, creating the table if it is not there.
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(LEASE_SCHEMA)?;
        Ok(Self { db })
    }
}

impl LeaseRecordStore for SqliteLeaseRecords {
    fn record(&self, h: &HolderRecord) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "INSERT OR REPLACE INTO lease_holders (holder_id, repo_id, class, operation, boot_id, since_unix) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    h.holder_id,
                    h.repo_id.as_str(),
                    h.class,
                    h.operation,
                    h.boot_id,
                    i64::try_from(h.since_unix).unwrap_or(i64::MAX)
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn clear(&self, holder_id: &str) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "DELETE FROM lease_holders WHERE holder_id = ?1",
                params![holder_id],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<HolderRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT holder_id, repo_id, class, operation, boot_id, since_unix FROM lease_holders \
                 ORDER BY since_unix, holder_id",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(HolderRecord {
                    holder_id: r.get(0)?,
                    repo_id: RepoId::new(r.get::<_, String>(1)?),
                    class: r.get(2)?,
                    operation: r.get(3)?,
                    boot_id: r.get(4)?,
                    since_unix: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
                })
            })
            .map_err(backend)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(backend)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, repo: &str, class: &str, since: u64) -> HolderRecord {
        HolderRecord {
            repo_id: RepoId::new(repo),
            class: class.to_owned(),
            operation: "test".to_owned(),
            holder_id: id.to_owned(),
            boot_id: "boot".to_owned(),
            since_unix: since,
        }
    }

    fn exercise(store: &dyn LeaseRecordStore) {
        assert!(store.list().unwrap().is_empty());
        store.record(&rec("b", "r2", "X", 20)).unwrap();
        store.record(&rec("a", "r1", "W", 10)).unwrap();
        // Oldest first.
        let ids: Vec<String> = store
            .list()
            .unwrap()
            .into_iter()
            .map(|r| r.holder_id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
        // Recording the same holder again replaces, it does not duplicate.
        store.record(&rec("a", "r1", "W", 11)).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        // Clearing is exact, and clearing what is gone is fine.
        store.clear("a").unwrap();
        store.clear("a").unwrap();
        store.clear("never-was").unwrap();
        let left = store.list().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0], rec("b", "r2", "X", 20));
    }

    #[test]
    fn the_in_memory_records_behave() {
        exercise(&InMemoryLeaseRecords::new());
    }

    #[test]
    fn the_sqlite_records_behave_and_the_table_creation_is_idempotent() {
        let db = SqliteDb::open_in_memory().unwrap();
        exercise(&SqliteLeaseRecords::new(db.clone()).unwrap());
        // A second open over the same database neither fails nor loses rows.
        let again = SqliteLeaseRecords::new(db).unwrap();
        assert_eq!(again.list().unwrap().len(), 1);
    }
}
