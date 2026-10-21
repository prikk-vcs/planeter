//! The recovery records (RFC 004 part 4, T1): one row per run of the recovery procedure, one row per step, written
//! **before** a step that changes something is taken and completed after it, so a record that has a step without an
//! end shows exactly where a procedure that was itself interrupted stopped.
//!
//! The text of each prikk call is kept (bounded to [`DETAIL_MAX_BYTES`] a step) because recovery removes locks of the
//! storage engine, and every removal must be justified by evidence the record shows.

use std::collections::BTreeMap;
use std::sync::Mutex;

use rusqlite::{OptionalExtension, params};

use crate::model::RepoId;
use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

/// The most bytes of prikk's text one step keeps.
pub const DETAIL_MAX_BYTES: usize = 64 * 1024;

/// One step of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryStep {
    pub seq: u32,
    /// A stable name (`witnesses`, `lease`, `doctor`, `unlock-list`, `unlock-force`, `repair`, `verify`,
    /// `clear-records`, `clear-flag`, `abandon-intent`, …).
    pub name: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// `ok`, `stopped`, `skipped`, … while unfinished: `None`.
    pub outcome: Option<String>,
    /// What was done and what prikk said, verbatim (bounded).
    pub detail: String,
}

/// One run of the procedure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub recovery_id: String,
    pub repo_id: RepoId,
    /// A user id, or `automatic`.
    pub actor: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// `recovered`, `stopped`, `not_now`, `nothing_to_do`; `None` while it runs (or if the server died in it).
    pub outcome: Option<String>,
    pub steps: Vec<RecoveryStep>,
}

/// Where recovery records are kept.
pub trait RecoveryStore: Send + Sync {
    fn begin(&self, recovery_id: &str, repo_id: &RepoId, actor: &str, now: u64) -> Result<()>;
    /// Write a step **before** it is taken. [`StoreError::Conflict`] if `seq` is used.
    fn begin_step(&self, recovery_id: &str, seq: u32, name: &str, now: u64) -> Result<()>;
    fn end_step(
        &self,
        recovery_id: &str,
        seq: u32,
        outcome: &str,
        detail: &str,
        now: u64,
    ) -> Result<()>;
    fn end(&self, recovery_id: &str, outcome: &str, now: u64) -> Result<()>;
    fn get(&self, recovery_id: &str) -> Result<Option<RecoveryRecord>>;
    /// The repository's runs, newest first, with their steps.
    fn recent(&self, repo_id: &RepoId, limit: usize) -> Result<Vec<RecoveryRecord>>;
}

fn bounded(detail: &str) -> String {
    if detail.len() <= DETAIL_MAX_BYTES {
        return detail.to_owned();
    }
    let mut end = DETAIL_MAX_BYTES;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [{} bytes more]", &detail[..end], detail.len() - end)
}

// ------------------------------------------------------------------------------------------------ in memory

/// An in-memory [`RecoveryStore`] (tests, development).
#[derive(Default)]
pub struct InMemoryRecoveries {
    runs: Mutex<BTreeMap<String, RecoveryRecord>>,
}

impl InMemoryRecoveries {
    pub fn new() -> Self {
        Self::default()
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, RecoveryRecord>>> {
        self.runs
            .lock()
            .map_err(|_| StoreError::Backend("recoveries lock poisoned".to_owned()))
    }
}

impl RecoveryStore for InMemoryRecoveries {
    fn begin(&self, id: &str, repo: &RepoId, actor: &str, now: u64) -> Result<()> {
        let mut m = self.lock()?;
        if m.contains_key(id) {
            return Err(StoreError::Conflict("recovery id".to_owned()));
        }
        m.insert(
            id.to_owned(),
            RecoveryRecord {
                recovery_id: id.to_owned(),
                repo_id: repo.clone(),
                actor: actor.to_owned(),
                started_at: now,
                ended_at: None,
                outcome: None,
                steps: Vec::new(),
            },
        );
        Ok(())
    }

    fn begin_step(&self, id: &str, seq: u32, name: &str, now: u64) -> Result<()> {
        let mut m = self.lock()?;
        let run = m.get_mut(id).ok_or(StoreError::NotFound)?;
        if run.steps.iter().any(|s| s.seq == seq) {
            return Err(StoreError::Conflict("step number".to_owned()));
        }
        run.steps.push(RecoveryStep {
            seq,
            name: name.to_owned(),
            started_at: now,
            ended_at: None,
            outcome: None,
            detail: String::new(),
        });
        Ok(())
    }

    fn end_step(&self, id: &str, seq: u32, outcome: &str, detail: &str, now: u64) -> Result<()> {
        let mut m = self.lock()?;
        let run = m.get_mut(id).ok_or(StoreError::NotFound)?;
        let step = run
            .steps
            .iter_mut()
            .find(|s| s.seq == seq)
            .ok_or(StoreError::NotFound)?;
        step.ended_at = Some(now);
        step.outcome = Some(outcome.to_owned());
        step.detail = bounded(detail);
        Ok(())
    }

    fn end(&self, id: &str, outcome: &str, now: u64) -> Result<()> {
        let mut m = self.lock()?;
        let run = m.get_mut(id).ok_or(StoreError::NotFound)?;
        run.ended_at = Some(now);
        run.outcome = Some(outcome.to_owned());
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<RecoveryRecord>> {
        Ok(self.lock()?.get(id).cloned().map(sorted))
    }

    fn recent(&self, repo: &RepoId, limit: usize) -> Result<Vec<RecoveryRecord>> {
        let mut v: Vec<RecoveryRecord> = self
            .lock()?
            .values()
            .filter(|r| &r.repo_id == repo)
            .cloned()
            .map(sorted)
            .collect();
        v.sort_by(|a, b| (b.started_at, &b.recovery_id).cmp(&(a.started_at, &a.recovery_id)));
        v.truncate(limit);
        Ok(v)
    }
}

fn sorted(mut r: RecoveryRecord) -> RecoveryRecord {
    r.steps.sort_by_key(|s| s.seq);
    r
}

// ------------------------------------------------------------------------------------------------ SQLite

/// The schema, as created (idempotent).
pub const RECOVERY_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS recoveries (
    recovery_id TEXT PRIMARY KEY,
    repo_id     TEXT NOT NULL,
    actor       TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    outcome     TEXT
);
CREATE INDEX IF NOT EXISTS recoveries_by_repo ON recoveries (repo_id, started_at);

CREATE TABLE IF NOT EXISTS recovery_steps (
    recovery_id TEXT NOT NULL REFERENCES recoveries (recovery_id),
    seq         INTEGER NOT NULL CHECK (seq >= 1),
    name        TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    outcome     TEXT,
    detail      TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (recovery_id, seq)
);
";

/// The SQLite [`RecoveryStore`].
#[derive(Clone)]
pub struct SqliteRecoveries {
    db: SqliteDb,
}

impl SqliteRecoveries {
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(RECOVERY_SCHEMA)?;
        Ok(Self { db })
    }
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn steps_of(conn: &rusqlite::Connection, id: &str) -> Result<Vec<RecoveryStep>> {
    let mut stmt = conn
        .prepare("SELECT seq, name, started_at, ended_at, outcome, detail FROM recovery_steps WHERE recovery_id = ?1 ORDER BY seq")
        .map_err(backend)?;
    let rows = stmt
        .query_map(params![id], |r| {
            Ok(RecoveryStep {
                seq: u32::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                name: r.get(1)?,
                started_at: u(r.get(2)?),
                ended_at: r.get::<_, Option<i64>>(3)?.map(u),
                outcome: r.get(4)?,
                detail: r.get(5)?,
            })
        })
        .map_err(backend)?;
    rows.map(|r| r.map_err(backend)).collect()
}

fn run_from(
    conn: &rusqlite::Connection,
    row: (String, String, String, i64, Option<i64>, Option<String>),
) -> Result<RecoveryRecord> {
    let steps = steps_of(conn, &row.0)?;
    Ok(RecoveryRecord {
        recovery_id: row.0,
        repo_id: RepoId::new(row.1),
        actor: row.2,
        started_at: u(row.3),
        ended_at: row.4.map(u),
        outcome: row.5,
        steps,
    })
}

const SELECT_RUN: &str =
    "SELECT recovery_id, repo_id, actor, started_at, ended_at, outcome FROM recoveries";

impl RecoveryStore for SqliteRecoveries {
    fn begin(&self, id: &str, repo: &RepoId, actor: &str, now: u64) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "INSERT INTO recoveries (recovery_id, repo_id, actor, started_at) VALUES (?1,?2,?3,?4)",
                params![id, repo.as_str(), actor, now as i64],
            )
            .map(drop)
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                    StoreError::Conflict("recovery id".to_owned())
                }
                other => backend(other),
            })
    }

    fn begin_step(&self, id: &str, seq: u32, name: &str, now: u64) -> Result<()> {
        let conn = self.db.lock()?;
        let known: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM recoveries WHERE recovery_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if known.is_none() {
            return Err(StoreError::NotFound);
        }
        conn.execute(
            "INSERT INTO recovery_steps (recovery_id, seq, name, started_at) VALUES (?1,?2,?3,?4)",
            params![id, seq as i64, name, now as i64],
        )
        .map(drop)
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                StoreError::Conflict("step number".to_owned())
            }
            other => backend(other),
        })
    }

    fn end_step(&self, id: &str, seq: u32, outcome: &str, detail: &str, now: u64) -> Result<()> {
        let n = self
            .db
            .lock()?
            .execute(
                "UPDATE recovery_steps SET ended_at = ?3, outcome = ?4, detail = ?5 WHERE recovery_id = ?1 AND seq = ?2",
                params![id, seq as i64, now as i64, outcome, bounded(detail)],
            )
            .map_err(backend)?;
        if n == 1 {
            Ok(())
        } else {
            Err(StoreError::NotFound)
        }
    }

    fn end(&self, id: &str, outcome: &str, now: u64) -> Result<()> {
        let n = self
            .db
            .lock()?
            .execute(
                "UPDATE recoveries SET ended_at = ?2, outcome = ?3 WHERE recovery_id = ?1",
                params![id, now as i64, outcome],
            )
            .map_err(backend)?;
        if n == 1 {
            Ok(())
        } else {
            Err(StoreError::NotFound)
        }
    }

    fn get(&self, id: &str) -> Result<Option<RecoveryRecord>> {
        let conn = self.db.lock()?;
        let row = conn
            .query_row(
                &format!("{SELECT_RUN} WHERE recovery_id = ?1"),
                params![id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?;
        row.map(|row| run_from(&conn, row)).transpose()
    }

    fn recent(&self, repo: &RepoId, limit: usize) -> Result<Vec<RecoveryRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(&format!("{SELECT_RUN} WHERE repo_id = ?1 ORDER BY started_at DESC, recovery_id DESC LIMIT ?2"))
            .map_err(backend)?;
        let rows: Vec<_> = stmt
            .query_map(params![repo.as_str(), limit as i64], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(backend)?
            .collect::<std::result::Result<_, _>>()
            .map_err(backend)?;
        rows.into_iter().map(|row| run_from(&conn, row)).collect()
    }
}
