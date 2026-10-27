//! Stages of a repository creation from an archive (RFC 004 part 7, T3/T4).
//!
//! A creator uploads a maintainer's archive; the forge verifies it and keeps it, for a day, as a **stage**: the record
//! here, and the archive file in a working area outside every repository. The record is what makes the limits
//! (at most N stages per principal, a total byte budget) and the two rules of the confirmation (only the principal who
//! staged it, once at a time) *atomic*: each is one transaction, never a read followed by a write.
//!
//! A stage is `staged` until a confirmation **claims** it (`confirming`); a confirmation that succeeds removes it, one
//! that fails releases it for another try, and a stage a killed server left `confirming` is removed at the next start
//! together with the repository it was creating.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{OptionalExtension, params};

use crate::model::{Owner, ProposalPolicy, Visibility};
use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

/// Where a stage is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageState {
    /// Verified and waiting for the creator's confirmation.
    Staged,
    /// A confirmation has claimed it and is running.
    Confirming,
}

impl StageState {
    fn as_str(self) -> &'static str {
        match self {
            StageState::Staged => "staged",
            StageState::Confirming => "confirming",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "staged" => Ok(StageState::Staged),
            "confirming" => Ok(StageState::Confirming),
            other => Err(StoreError::Backend(format!("bad stage state {other:?}"))),
        }
    }
}

/// One stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRecord {
    pub stage_id: String,
    /// The principal who staged it (a user id): the only one who may confirm it.
    pub principal: String,
    pub owner: Owner,
    pub name: String,
    pub visibility: Visibility,
    pub proposals: ProposalPolicy,
    /// SHA-256 of the staged archive, 64 lowercase hexadecimal characters (the confirmation checks the file against it).
    pub archive_digest: String,
    pub archive_bytes: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub state: StageState,
    /// What the verification found, as the transport wrote it (JSON): the carried keys, the refs, the counts. Opaque here.
    pub report: String,
}

/// The limits a new stage must fit under (decided atomically with the insert).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageLimits {
    /// The most stages one principal may hold at once.
    pub per_principal: u32,
    /// The most archive bytes all stages may hold together.
    pub total_bytes: u64,
}

/// Why a stage was not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageRefusal {
    /// The principal already holds [`StageLimits::per_principal`] stages.
    TooManyForPrincipal { limit: u32 },
    /// The stages would hold more than [`StageLimits::total_bytes`].
    BudgetExceeded { limit: u64 },
    /// A stage with this id exists (a bug or a collision of random ids).
    IdTaken,
}

/// Why a stage cannot be claimed for a confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimError {
    /// There is no such stage, or it is not this principal's: the two are not told apart.
    NotFound,
    /// Its day is over.
    Expired,
    /// A confirmation is running on it.
    InUse,
}

/// Persistence of [`StageRecord`]s.
pub trait StageStore: Send + Sync {
    /// Keep a new stage if it fits under `limits` (counting the rows now stored); the check and the insert are one step.
    fn put(
        &self,
        record: StageRecord,
        limits: StageLimits,
    ) -> Result<std::result::Result<(), StageRefusal>>;

    /// The stage, whoever it belongs to (the caller compares the principal).
    fn get(&self, stage_id: &str) -> Result<Option<StageRecord>>;

    /// `staged` → `confirming`, for `principal`, at `now`: one winner among concurrent callers.
    fn claim(
        &self,
        stage_id: &str,
        principal: &str,
        now: u64,
    ) -> Result<std::result::Result<StageRecord, ClaimError>>;

    /// `confirming` → `staged` (a confirmation that failed in a way the creator may try again).
    fn release(&self, stage_id: &str) -> Result<()>;

    /// Remove the record (its file is the caller's to remove).
    fn remove(&self, stage_id: &str) -> Result<()>;

    /// The `staged` stages whose time is over at `now`.
    fn expired(&self, now: u64) -> Result<Vec<StageRecord>>;

    /// Every stage.
    fn all(&self) -> Result<Vec<StageRecord>>;
}

fn refusal(rows: &[StageRecord], new: &StageRecord, limits: StageLimits) -> Option<StageRefusal> {
    if rows.iter().any(|r| r.stage_id == new.stage_id) {
        return Some(StageRefusal::IdTaken);
    }
    let mine = rows.iter().filter(|r| r.principal == new.principal).count();
    if mine as u64 >= u64::from(limits.per_principal) {
        return Some(StageRefusal::TooManyForPrincipal {
            limit: limits.per_principal,
        });
    }
    let held: u64 = rows.iter().map(|r| r.archive_bytes).sum();
    if held.saturating_add(new.archive_bytes) > limits.total_bytes {
        return Some(StageRefusal::BudgetExceeded {
            limit: limits.total_bytes,
        });
    }
    None
}

// ---------------------------------------------------------------------------------------------------- in memory

/// An in-memory [`StageStore`] (tests and development).
#[derive(Default)]
pub struct InMemoryStageStore {
    rows: Mutex<HashMap<String, StageRecord>>,
}

impl InMemoryStageStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, StageRecord>>> {
        self.rows
            .lock()
            .map_err(|_| StoreError::Backend("stage store lock poisoned".to_owned()))
    }
}

impl StageStore for InMemoryStageStore {
    fn put(
        &self,
        record: StageRecord,
        limits: StageLimits,
    ) -> Result<std::result::Result<(), StageRefusal>> {
        let mut rows = self.lock()?;
        let all: Vec<StageRecord> = rows.values().cloned().collect();
        if let Some(r) = refusal(&all, &record, limits) {
            return Ok(Err(r));
        }
        rows.insert(record.stage_id.clone(), record);
        Ok(Ok(()))
    }

    fn get(&self, stage_id: &str) -> Result<Option<StageRecord>> {
        Ok(self.lock()?.get(stage_id).cloned())
    }

    fn claim(
        &self,
        stage_id: &str,
        principal: &str,
        now: u64,
    ) -> Result<std::result::Result<StageRecord, ClaimError>> {
        let mut rows = self.lock()?;
        let Some(row) = rows.get_mut(stage_id).filter(|r| r.principal == principal) else {
            return Ok(Err(ClaimError::NotFound));
        };
        if row.state == StageState::Confirming {
            return Ok(Err(ClaimError::InUse));
        }
        if row.expires_at <= now {
            return Ok(Err(ClaimError::Expired));
        }
        row.state = StageState::Confirming;
        Ok(Ok(row.clone()))
    }

    fn release(&self, stage_id: &str) -> Result<()> {
        if let Some(row) = self.lock()?.get_mut(stage_id) {
            row.state = StageState::Staged;
        }
        Ok(())
    }

    fn remove(&self, stage_id: &str) -> Result<()> {
        self.lock()?.remove(stage_id);
        Ok(())
    }

    fn expired(&self, now: u64) -> Result<Vec<StageRecord>> {
        let mut out: Vec<StageRecord> = self
            .lock()?
            .values()
            .filter(|r| r.state == StageState::Staged && r.expires_at <= now)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.stage_id.cmp(&b.stage_id));
        Ok(out)
    }

    fn all(&self) -> Result<Vec<StageRecord>> {
        let mut out: Vec<StageRecord> = self.lock()?.values().cloned().collect();
        out.sort_by(|a, b| a.stage_id.cmp(&b.stage_id));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------------- SQLite

/// The stage table. Applied by [`SqliteStageStore::new`].
pub const STAGE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repo_stages (
    stage_id       TEXT PRIMARY KEY,
    principal      TEXT NOT NULL,
    owner_kind     TEXT NOT NULL CHECK (owner_kind IN ('user', 'org')),
    owner          TEXT NOT NULL,
    name           TEXT NOT NULL,
    visibility     TEXT NOT NULL CHECK (visibility IN ('public', 'internal', 'private')),
    proposals      TEXT NOT NULL CHECK (proposals IN ('members', 'any-authenticated')),
    archive_digest TEXT NOT NULL,
    archive_bytes  INTEGER NOT NULL CHECK (archive_bytes >= 0),
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('staged', 'confirming')),
    report         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS repo_stages_by_principal ON repo_stages (principal);
";

/// [`StageStore`] over SQLite.
#[derive(Clone)]
pub struct SqliteStageStore {
    db: SqliteDb,
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    String,
    String,
);

const COLUMNS: &str = "stage_id, principal, owner_kind, owner, name, visibility, proposals, archive_digest, \
                       archive_bytes, created_at, expires_at, state, report";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
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
        r.get(11)?,
        r.get(12)?,
    ))
}

fn decode(row: Row) -> Result<StageRecord> {
    let (
        stage_id,
        principal,
        kind,
        owner,
        name,
        vis,
        proposals,
        digest,
        bytes,
        created,
        expires,
        state,
        report,
    ) = row;
    Ok(StageRecord {
        stage_id,
        principal,
        owner: match kind.as_str() {
            "user" => Owner::User(owner),
            "org" => Owner::Org(owner),
            other => return Err(StoreError::Backend(format!("bad owner_kind {other:?}"))),
        },
        name,
        visibility: match vis.as_str() {
            "public" => Visibility::Public,
            "internal" => Visibility::Internal,
            "private" => Visibility::Private,
            other => return Err(StoreError::Backend(format!("bad visibility {other:?}"))),
        },
        proposals: ProposalPolicy::parse(&proposals)
            .map_err(|e| StoreError::Backend(e.to_string()))?,
        archive_digest: digest,
        archive_bytes: u64::try_from(bytes).unwrap_or(0),
        created_at: u64::try_from(created).unwrap_or(0),
        expires_at: u64::try_from(expires).unwrap_or(0),
        state: StageState::parse(&state)?,
        report,
    })
}

impl SqliteStageStore {
    /// Open the store on `db`, creating its table.
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(STAGE_SCHEMA)?;
        Ok(Self { db })
    }

    fn query(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<StageRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn.prepare(sql).map_err(backend)?;
        let rows = stmt.query_map(args, read_row).map_err(backend)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(decode(row.map_err(backend)?)?);
        }
        Ok(out)
    }
}

impl StageStore for SqliteStageStore {
    fn put(
        &self,
        record: StageRecord,
        limits: StageLimits,
    ) -> Result<std::result::Result<(), StageRefusal>> {
        let mut conn = self.db.lock()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(backend)?;
        let existing: Vec<StageRecord> = {
            let mut stmt = tx
                .prepare(&format!("SELECT {COLUMNS} FROM repo_stages"))
                .map_err(backend)?;
            let rows = stmt.query_map([], read_row).map_err(backend)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(decode(row.map_err(backend)?)?);
            }
            out
        };
        if let Some(r) = refusal(&existing, &record, limits) {
            return Ok(Err(r));
        }
        let (kind, handle) = match &record.owner {
            Owner::User(h) => ("user", h.as_str()),
            Owner::Org(h) => ("org", h.as_str()),
        };
        tx.execute(
            &format!("INSERT INTO repo_stages ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
            params![
                record.stage_id,
                record.principal,
                kind,
                handle,
                record.name,
                match record.visibility {
                    Visibility::Public => "public",
                    Visibility::Internal => "internal",
                    Visibility::Private => "private",
                },
                record.proposals.as_str(),
                record.archive_digest,
                i64::try_from(record.archive_bytes).unwrap_or(i64::MAX),
                i64::try_from(record.created_at).unwrap_or(i64::MAX),
                i64::try_from(record.expires_at).unwrap_or(i64::MAX),
                record.state.as_str(),
                record.report,
            ],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(Ok(()))
    }

    fn get(&self, stage_id: &str) -> Result<Option<StageRecord>> {
        Ok(self
            .query(
                &format!("SELECT {COLUMNS} FROM repo_stages WHERE stage_id = ?1"),
                &[&stage_id],
            )?
            .into_iter()
            .next())
    }

    fn claim(
        &self,
        stage_id: &str,
        principal: &str,
        now: u64,
    ) -> Result<std::result::Result<StageRecord, ClaimError>> {
        let mut conn = self.db.lock()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(backend)?;
        let row = tx
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM repo_stages WHERE stage_id = ?1 AND principal = ?2"
                ),
                params![stage_id, principal],
                read_row,
            )
            .optional()
            .map_err(backend)?;
        let Some(row) = row else {
            return Ok(Err(ClaimError::NotFound));
        };
        let mut record = decode(row)?;
        if record.state == StageState::Confirming {
            return Ok(Err(ClaimError::InUse));
        }
        if record.expires_at <= now {
            return Ok(Err(ClaimError::Expired));
        }
        tx.execute(
            "UPDATE repo_stages SET state = 'confirming' WHERE stage_id = ?1 AND state = 'staged'",
            params![stage_id],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        record.state = StageState::Confirming;
        Ok(Ok(record))
    }

    fn release(&self, stage_id: &str) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "UPDATE repo_stages SET state = 'staged' WHERE stage_id = ?1",
                params![stage_id],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn remove(&self, stage_id: &str) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "DELETE FROM repo_stages WHERE stage_id = ?1",
                params![stage_id],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn expired(&self, now: u64) -> Result<Vec<StageRecord>> {
        let now = i64::try_from(now).unwrap_or(i64::MAX);
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM repo_stages WHERE state = 'staged' AND expires_at <= ?1 ORDER BY stage_id"
            ),
            &[&now],
        )
    }

    fn all(&self) -> Result<Vec<StageRecord>> {
        self.query(
            &format!("SELECT {COLUMNS} FROM repo_stages ORDER BY stage_id"),
            &[],
        )
    }
}
