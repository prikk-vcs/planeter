//! Deliveries of a maintainer's repository archive to a hosted repository (RFC 008, landing by delivery): the records of the
//! **delivery engine** (`planeter-transport::delivery`).
//!
//! A delivery is staged (the archive is verified in a scratch repository and the written rule is applied), reported to the maintainer,
//! confirmed, exchanged for the live repository and finished. **Every step that moves anything is preceded by its state here**, so a
//! restart knows what was done: the state is the *intent* (RFC 008 D-7).
//!
//! ```text
//!  staged ──► reported ──► confirmed ──► exchanged ──► reaccepting ──► done
//!    │           │            │             │              │
//!    └───────────┴────────────┴─────────────┴──────────────┴──► failed        (a refusal, or a failure that is never rolled back)
//!    └──► abandoned (a confirm that never exchanged, found at start; a stage nobody confirmed)      expired (the stage's day is over)
//! ```
//!
//! **At most one delivery per repository is in a non-final state** (a partial unique index, and the same check inside the transaction
//! of the in-memory store): a second `start` is refused as `RepoBusy`. Transitions go through [`transition`], which refuses an illegal
//! one, and are compare-and-set on the state the caller saw. The table is created idempotently; no existing table changes.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{OptionalExtension, params};

use crate::model::RepoId;
use crate::sqlite::{SqliteDb, backend};
use crate::stage::StageLimits;
use crate::store::{Result, StoreError};

/// Where a delivery is in its life (see the module documentation for the order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryState {
    /// The archive is uploaded and being verified in the stage.
    Staged,
    /// The report exists: acceptable (waiting for the confirmation) — a refusal is `Failed` with its reason.
    Reported,
    /// The confirmation holds the write lease and has written its intent; nothing has moved yet.
    Confirmed,
    /// The stage and the live repository were exchanged.
    Exchanged,
    /// The open proposals' retained artifacts are being accepted again.
    Reaccepting,
    /// Finished: the repository is the delivered one and the previous generation is gone.
    Done,
    /// Refused by the rule, or failed (the reason says which).
    Failed,
    /// Never confirmed, or a confirmation that never moved anything: the stage was discarded.
    Abandoned,
    /// The stage's day was over.
    Expired,
}

impl DeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::Staged => "staged",
            DeliveryState::Reported => "reported",
            DeliveryState::Confirmed => "confirmed",
            DeliveryState::Exchanged => "exchanged",
            DeliveryState::Reaccepting => "reaccepting",
            DeliveryState::Done => "done",
            DeliveryState::Failed => "failed",
            DeliveryState::Abandoned => "abandoned",
            DeliveryState::Expired => "expired",
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "staged" => DeliveryState::Staged,
            "reported" => DeliveryState::Reported,
            "confirmed" => DeliveryState::Confirmed,
            "exchanged" => DeliveryState::Exchanged,
            "reaccepting" => DeliveryState::Reaccepting,
            "done" => DeliveryState::Done,
            "failed" => DeliveryState::Failed,
            "abandoned" => DeliveryState::Abandoned,
            "expired" => DeliveryState::Expired,
            other => {
                return Err(StoreError::Backend(format!("bad delivery state {other:?}")));
            }
        })
    }

    /// No transition leaves a final state.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            DeliveryState::Done
                | DeliveryState::Failed
                | DeliveryState::Abandoned
                | DeliveryState::Expired
        )
    }

    /// Every state, in the order of the diagram (for tests and the metrics).
    pub const ALL: [DeliveryState; 9] = [
        DeliveryState::Staged,
        DeliveryState::Reported,
        DeliveryState::Confirmed,
        DeliveryState::Exchanged,
        DeliveryState::Reaccepting,
        DeliveryState::Done,
        DeliveryState::Failed,
        DeliveryState::Abandoned,
        DeliveryState::Expired,
    ];
}

/// Why a transition was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// There is no such delivery.
    NotFound,
    /// The diagram has no such step.
    Illegal {
        from: DeliveryState,
        to: DeliveryState,
    },
    /// The delivery is not in the state the caller saw (someone else moved it).
    Changed { actual: DeliveryState },
}

/// The one function that decides whether a step is allowed.
pub fn transition(
    from: DeliveryState,
    to: DeliveryState,
) -> std::result::Result<(), TransitionError> {
    use DeliveryState as S;
    let ok = matches!(
        (from, to),
        (
            S::Staged,
            S::Reported | S::Failed | S::Abandoned | S::Expired
        ) | (
            S::Reported,
            S::Confirmed | S::Failed | S::Abandoned | S::Expired
        ) | (S::Confirmed, S::Exchanged | S::Failed | S::Abandoned)
            | (S::Exchanged, S::Reaccepting | S::Failed)
            | (S::Reaccepting, S::Done | S::Failed)
    );
    if ok {
        Ok(())
    } else {
        Err(TransitionError::Illegal { from, to })
    }
}

/// One delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRecord {
    /// `dl-` and 24 hexadecimal characters.
    pub delivery_id: String,
    pub repo_id: RepoId,
    /// The user who started it: the only one who may confirm it.
    pub principal: String,
    pub state: DeliveryState,
    /// SHA-256 of the staged archive (the confirmation checks the file against it).
    pub archive_digest: String,
    pub archive_bytes: u64,
    /// The directory's **name** under the repositories' `.deliveries` directory (never a path).
    pub stage_dir: String,
    /// The live `heads/main` tip block id when the delivery was reported, and the staged one.
    pub old_tip: Option<String>,
    pub new_tip: Option<String>,
    /// The closed report of RFC 008 D-5, serialised by the transport. Opaque here.
    pub report_json: Option<String>,
    /// The closed reason when `Failed` (or a note when `Abandoned`).
    pub reason: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub finished_at: Option<u64>,
}

/// What a transition may set besides the state.
#[derive(Debug, Clone, Default)]
pub struct DeliveryUpdate {
    pub old_tip: Option<String>,
    pub new_tip: Option<String>,
    pub report_json: Option<String>,
    pub reason: Option<String>,
}

/// Why a delivery was not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusal {
    /// The repository already has a delivery in a non-final state.
    RepoBusy,
    /// The principal already has [`StageLimits::per_principal`] deliveries in a non-final state.
    TooManyForPrincipal { limit: u32 },
    /// The non-final deliveries would hold more than [`StageLimits::total_bytes`].
    BudgetExceeded { limit: u64 },
    /// A delivery with this id exists (a collision of random ids).
    IdTaken,
}

/// Persistence of [`DeliveryRecord`]s.
pub trait DeliveryStore: Send + Sync {
    /// Keep a new delivery (state `Staged`) if the repository has none in flight and it fits under `limits`; the checks and the
    /// insert are one step.
    fn start(
        &self,
        record: DeliveryRecord,
        limits: StageLimits,
    ) -> Result<std::result::Result<(), StartRefusal>>;

    fn get(&self, delivery_id: &str) -> Result<Option<DeliveryRecord>>;

    /// Record what the upload turned out to be (its digest and size), while the delivery is `Staged`. `false` when it is not.
    fn set_archive(&self, delivery_id: &str, digest: &str, bytes: u64) -> Result<bool>;

    /// Move a delivery from `from` to `to` (compare-and-set; [`transition`] decides legality) and set what `update` carries.
    /// `finished_at` is set when `to` is final.
    fn advance(
        &self,
        delivery_id: &str,
        from: DeliveryState,
        to: DeliveryState,
        update: DeliveryUpdate,
        now: u64,
    ) -> Result<std::result::Result<DeliveryRecord, TransitionError>>;

    /// Every delivery not in a final state, oldest first.
    fn non_final(&self) -> Result<Vec<DeliveryRecord>>;

    /// The non-final delivery of a repository, if any.
    fn in_flight(&self, repo_id: &RepoId) -> Result<Option<DeliveryRecord>>;

    /// The last delivery of the repository that finished `Done` (who, when, old and new tip), for the repository page.
    fn last_done(&self, repo_id: &RepoId) -> Result<Option<DeliveryRecord>>;

    /// Every delivery (tests and the sweep), in id order.
    fn all(&self) -> Result<Vec<DeliveryRecord>>;
}

fn refusal(
    rows: &[DeliveryRecord],
    new: &DeliveryRecord,
    limits: StageLimits,
) -> Option<StartRefusal> {
    if rows.iter().any(|r| r.delivery_id == new.delivery_id) {
        return Some(StartRefusal::IdTaken);
    }
    let live: Vec<&DeliveryRecord> = rows.iter().filter(|r| !r.state.is_final()).collect();
    if live.iter().any(|r| r.repo_id == new.repo_id) {
        return Some(StartRefusal::RepoBusy);
    }
    let mine = live.iter().filter(|r| r.principal == new.principal).count();
    if mine as u64 >= u64::from(limits.per_principal) {
        return Some(StartRefusal::TooManyForPrincipal {
            limit: limits.per_principal,
        });
    }
    let held: u64 = live.iter().map(|r| r.archive_bytes).sum();
    if held.saturating_add(new.archive_bytes) > limits.total_bytes {
        return Some(StartRefusal::BudgetExceeded {
            limit: limits.total_bytes,
        });
    }
    None
}

fn applied(
    mut row: DeliveryRecord,
    to: DeliveryState,
    update: DeliveryUpdate,
    now: u64,
) -> DeliveryRecord {
    row.state = to;
    row.updated_at = now;
    if to.is_final() {
        row.finished_at = Some(now);
    }
    if update.old_tip.is_some() {
        row.old_tip = update.old_tip;
    }
    if update.new_tip.is_some() {
        row.new_tip = update.new_tip;
    }
    if update.report_json.is_some() {
        row.report_json = update.report_json;
    }
    if update.reason.is_some() {
        row.reason = update.reason;
    }
    row
}

// ---------------------------------------------------------------------------------------------------- in memory

/// An in-memory [`DeliveryStore`] (tests and development).
#[derive(Default)]
pub struct InMemoryDeliveryStore {
    rows: Mutex<HashMap<String, DeliveryRecord>>,
}

impl InMemoryDeliveryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, DeliveryRecord>>> {
        self.rows
            .lock()
            .map_err(|_| StoreError::Backend("delivery store lock poisoned".to_owned()))
    }
}

impl DeliveryStore for InMemoryDeliveryStore {
    fn start(
        &self,
        record: DeliveryRecord,
        limits: StageLimits,
    ) -> Result<std::result::Result<(), StartRefusal>> {
        let mut rows = self.lock()?;
        let all: Vec<DeliveryRecord> = rows.values().cloned().collect();
        if let Some(r) = refusal(&all, &record, limits) {
            return Ok(Err(r));
        }
        rows.insert(record.delivery_id.clone(), record);
        Ok(Ok(()))
    }

    fn get(&self, delivery_id: &str) -> Result<Option<DeliveryRecord>> {
        Ok(self.lock()?.get(delivery_id).cloned())
    }

    fn set_archive(&self, delivery_id: &str, digest: &str, bytes: u64) -> Result<bool> {
        let mut rows = self.lock()?;
        match rows
            .get_mut(delivery_id)
            .filter(|r| r.state == DeliveryState::Staged)
        {
            Some(r) => {
                r.archive_digest = digest.to_owned();
                r.archive_bytes = bytes;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn advance(
        &self,
        delivery_id: &str,
        from: DeliveryState,
        to: DeliveryState,
        update: DeliveryUpdate,
        now: u64,
    ) -> Result<std::result::Result<DeliveryRecord, TransitionError>> {
        let mut rows = self.lock()?;
        let Some(row) = rows.get(delivery_id).cloned() else {
            return Ok(Err(TransitionError::NotFound));
        };
        if row.state != from {
            return Ok(Err(TransitionError::Changed { actual: row.state }));
        }
        if let Err(e) = transition(from, to) {
            return Ok(Err(e));
        }
        let next = applied(row, to, update, now);
        rows.insert(delivery_id.to_owned(), next.clone());
        Ok(Ok(next))
    }

    fn non_final(&self) -> Result<Vec<DeliveryRecord>> {
        let mut out: Vec<DeliveryRecord> = self
            .lock()?
            .values()
            .filter(|r| !r.state.is_final())
            .cloned()
            .collect();
        out.sort_by(|a, b| (a.created_at, &a.delivery_id).cmp(&(b.created_at, &b.delivery_id)));
        Ok(out)
    }

    fn in_flight(&self, repo_id: &RepoId) -> Result<Option<DeliveryRecord>> {
        Ok(self
            .lock()?
            .values()
            .find(|r| &r.repo_id == repo_id && !r.state.is_final())
            .cloned())
    }

    fn last_done(&self, repo_id: &RepoId) -> Result<Option<DeliveryRecord>> {
        Ok(self
            .lock()?
            .values()
            .filter(|r| &r.repo_id == repo_id && r.state == DeliveryState::Done)
            .max_by_key(|r| (r.finished_at, r.delivery_id.clone()))
            .cloned())
    }

    fn all(&self) -> Result<Vec<DeliveryRecord>> {
        let mut out: Vec<DeliveryRecord> = self.lock()?.values().cloned().collect();
        out.sort_by(|a, b| a.delivery_id.cmp(&b.delivery_id));
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------------- SQLite

/// The delivery table. Applied by [`SqliteDeliveryStore::new`]. The partial unique index is the "one in flight per repository" rule.
pub const DELIVERY_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS deliveries (
    delivery_id    TEXT PRIMARY KEY,
    repo_id        TEXT NOT NULL,
    principal      TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('staged','reported','confirmed','exchanged','reaccepting','done','failed','abandoned','expired')),
    archive_digest TEXT NOT NULL,
    archive_bytes  INTEGER NOT NULL CHECK (archive_bytes >= 0),
    stage_dir      TEXT NOT NULL,
    old_tip        TEXT,
    new_tip        TEXT,
    report_json    TEXT,
    reason         TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    finished_at    INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS deliveries_one_in_flight ON deliveries (repo_id)
    WHERE state NOT IN ('done', 'failed', 'abandoned', 'expired');
CREATE INDEX IF NOT EXISTS deliveries_by_repo ON deliveries (repo_id, finished_at);
";

const COLUMNS: &str = "delivery_id, repo_id, principal, state, archive_digest, archive_bytes, stage_dir, old_tip, new_tip, \
                       report_json, reason, created_at, updated_at, finished_at";

/// [`DeliveryStore`] over SQLite.
#[derive(Clone)]
pub struct SqliteDeliveryStore {
    db: SqliteDb,
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
);

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
        r.get(13)?,
    ))
}

fn decode(row: Row) -> Result<DeliveryRecord> {
    let (
        id,
        repo,
        principal,
        state,
        digest,
        bytes,
        dir,
        old,
        new,
        report,
        reason,
        created,
        updated,
        finished,
    ) = row;
    Ok(DeliveryRecord {
        delivery_id: id,
        repo_id: RepoId::new(repo),
        principal,
        state: DeliveryState::parse(&state)?,
        archive_digest: digest,
        archive_bytes: u64::try_from(bytes).unwrap_or(0),
        stage_dir: dir,
        old_tip: old,
        new_tip: new,
        report_json: report,
        reason,
        created_at: u64::try_from(created).unwrap_or(0),
        updated_at: u64::try_from(updated).unwrap_or(0),
        finished_at: finished.map(|f| u64::try_from(f).unwrap_or(0)),
    })
}

impl SqliteDeliveryStore {
    /// Open the store on `db`, creating its table (idempotent).
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(DELIVERY_SCHEMA)?;
        Ok(Self { db })
    }

    fn query(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<DeliveryRecord>> {
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

fn i(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl DeliveryStore for SqliteDeliveryStore {
    fn start(
        &self,
        record: DeliveryRecord,
        limits: StageLimits,
    ) -> Result<std::result::Result<(), StartRefusal>> {
        let mut conn = self.db.lock()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(backend)?;
        let existing: Vec<DeliveryRecord> = {
            let mut stmt = tx
                .prepare(&format!("SELECT {COLUMNS} FROM deliveries"))
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
        tx.execute(
            &format!("INSERT INTO deliveries ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"),
            params![
                record.delivery_id,
                record.repo_id.as_str(),
                record.principal,
                record.state.as_str(),
                record.archive_digest,
                i(record.archive_bytes),
                record.stage_dir,
                record.old_tip,
                record.new_tip,
                record.report_json,
                record.reason,
                i(record.created_at),
                i(record.updated_at),
                record.finished_at.map(i),
            ],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(Ok(()))
    }

    fn get(&self, delivery_id: &str) -> Result<Option<DeliveryRecord>> {
        Ok(self
            .query(
                &format!("SELECT {COLUMNS} FROM deliveries WHERE delivery_id = ?1"),
                &[&delivery_id],
            )?
            .into_iter()
            .next())
    }

    fn set_archive(&self, delivery_id: &str, digest: &str, bytes: u64) -> Result<bool> {
        let n = self
            .db
            .lock()?
            .execute(
                "UPDATE deliveries SET archive_digest = ?2, archive_bytes = ?3 WHERE delivery_id = ?1 AND state = 'staged'",
                params![delivery_id, digest, i(bytes)],
            )
            .map_err(backend)?;
        Ok(n == 1)
    }

    fn advance(
        &self,
        delivery_id: &str,
        from: DeliveryState,
        to: DeliveryState,
        update: DeliveryUpdate,
        now: u64,
    ) -> Result<std::result::Result<DeliveryRecord, TransitionError>> {
        let mut conn = self.db.lock()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(backend)?;
        let row = tx
            .query_row(
                &format!("SELECT {COLUMNS} FROM deliveries WHERE delivery_id = ?1"),
                params![delivery_id],
                read_row,
            )
            .optional()
            .map_err(backend)?;
        let Some(row) = row else {
            return Ok(Err(TransitionError::NotFound));
        };
        let row = decode(row)?;
        if row.state != from {
            return Ok(Err(TransitionError::Changed { actual: row.state }));
        }
        if let Err(e) = transition(from, to) {
            return Ok(Err(e));
        }
        let next = applied(row, to, update, now);
        tx.execute(
            "UPDATE deliveries SET state = ?2, old_tip = ?3, new_tip = ?4, report_json = ?5, reason = ?6, updated_at = ?7, finished_at = ?8 \
             WHERE delivery_id = ?1",
            params![
                delivery_id,
                next.state.as_str(),
                next.old_tip,
                next.new_tip,
                next.report_json,
                next.reason,
                i(next.updated_at),
                next.finished_at.map(i),
            ],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(Ok(next))
    }

    fn non_final(&self) -> Result<Vec<DeliveryRecord>> {
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM deliveries WHERE state NOT IN ('done','failed','abandoned','expired') ORDER BY created_at, delivery_id"
            ),
            &[],
        )
    }

    fn in_flight(&self, repo_id: &RepoId) -> Result<Option<DeliveryRecord>> {
        let repo = repo_id.as_str();
        Ok(self
            .query(
                &format!(
                    "SELECT {COLUMNS} FROM deliveries WHERE repo_id = ?1 AND state NOT IN ('done','failed','abandoned','expired')"
                ),
                &[&repo],
            )?
            .into_iter()
            .next())
    }

    fn last_done(&self, repo_id: &RepoId) -> Result<Option<DeliveryRecord>> {
        let repo = repo_id.as_str();
        Ok(self
            .query(
                &format!(
                    "SELECT {COLUMNS} FROM deliveries WHERE repo_id = ?1 AND state = 'done' ORDER BY finished_at DESC, delivery_id DESC LIMIT 1"
                ),
                &[&repo],
            )?
            .into_iter()
            .next())
    }

    fn all(&self) -> Result<Vec<DeliveryRecord>> {
        self.query(
            &format!("SELECT {COLUMNS} FROM deliveries ORDER BY delivery_id"),
            &[],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> StageLimits {
        StageLimits {
            per_principal: 2,
            total_bytes: 1000,
        }
    }

    fn rec(id: &str, repo: &str, who: &str, bytes: u64) -> DeliveryRecord {
        DeliveryRecord {
            delivery_id: id.into(),
            repo_id: RepoId::new(repo),
            principal: who.into(),
            state: DeliveryState::Staged,
            archive_digest: "d".into(),
            archive_bytes: bytes,
            stage_dir: id.into(),
            old_tip: None,
            new_tip: None,
            report_json: None,
            reason: None,
            created_at: 10,
            updated_at: 10,
            finished_at: None,
        }
    }

    fn up() -> DeliveryUpdate {
        DeliveryUpdate::default()
    }

    /// Every property of the store, over any implementation.
    fn contract(s: &dyn DeliveryStore) {
        assert_eq!(s.start(rec("a", "r1", "u", 10), limits()).unwrap(), Ok(()));
        // one in flight per repository
        assert_eq!(
            s.start(rec("b", "r1", "v", 10), limits()).unwrap(),
            Err(StartRefusal::RepoBusy)
        );
        // an id is not reused
        assert_eq!(
            s.start(rec("a", "r2", "u", 10), limits()).unwrap(),
            Err(StartRefusal::IdTaken)
        );
        // another repository is free
        assert_eq!(s.start(rec("c", "r2", "u", 10), limits()).unwrap(), Ok(()));
        // the principal's limit
        assert_eq!(
            s.start(rec("d", "r3", "u", 10), limits()).unwrap(),
            Err(StartRefusal::TooManyForPrincipal { limit: 2 })
        );
        // the byte budget
        assert_eq!(
            s.start(rec("e", "r3", "w", 981), limits()).unwrap(),
            Err(StartRefusal::BudgetExceeded { limit: 1000 })
        );
        assert_eq!(s.non_final().unwrap().len(), 2);
        assert_eq!(
            s.in_flight(&RepoId::new("r1"))
                .unwrap()
                .unwrap()
                .delivery_id,
            "a"
        );
        // the upload's digest is kept while staged, and only then
        assert!(s.set_archive("a", "sha", 77).unwrap());
        assert_eq!(s.get("a").unwrap().unwrap().archive_bytes, 77);
        assert!(!s.set_archive("zz", "sha", 1).unwrap());
        // an illegal step is refused and changes nothing
        assert_eq!(
            s.advance("a", DeliveryState::Staged, DeliveryState::Done, up(), 11)
                .unwrap(),
            Err(TransitionError::Illegal {
                from: DeliveryState::Staged,
                to: DeliveryState::Done
            })
        );
        // a stale view is refused
        assert_eq!(
            s.advance(
                "a",
                DeliveryState::Reported,
                DeliveryState::Confirmed,
                up(),
                11
            )
            .unwrap(),
            Err(TransitionError::Changed {
                actual: DeliveryState::Staged
            })
        );
        assert_eq!(
            s.advance(
                "zz",
                DeliveryState::Staged,
                DeliveryState::Reported,
                up(),
                11
            )
            .unwrap(),
            Err(TransitionError::NotFound)
        );
        // the whole road
        let steps = [
            (DeliveryState::Staged, DeliveryState::Reported),
            (DeliveryState::Reported, DeliveryState::Confirmed),
            (DeliveryState::Confirmed, DeliveryState::Exchanged),
            (DeliveryState::Exchanged, DeliveryState::Reaccepting),
            (DeliveryState::Reaccepting, DeliveryState::Done),
        ];
        for (n, (from, to)) in steps.into_iter().enumerate() {
            let update = if n == 0 {
                DeliveryUpdate {
                    old_tip: Some("o".into()),
                    new_tip: Some("n".into()),
                    report_json: Some("{}".into()),
                    reason: None,
                }
            } else {
                up()
            };
            let got = s
                .advance("a", from, to, update, 20 + n as u64)
                .unwrap()
                .unwrap();
            assert_eq!(got.state, to);
            assert_eq!(got.finished_at.is_some(), to.is_final());
        }
        let done = s.get("a").unwrap().unwrap();
        assert_eq!(done.old_tip.as_deref(), Some("o"));
        assert_eq!(done.new_tip.as_deref(), Some("n"));
        assert_eq!(done.finished_at, Some(24));
        assert!(!s.set_archive("a", "other", 1).unwrap(), "not after staged");
        // a final state is final
        assert_eq!(
            s.advance("a", DeliveryState::Done, DeliveryState::Failed, up(), 30)
                .unwrap(),
            Err(TransitionError::Illegal {
                from: DeliveryState::Done,
                to: DeliveryState::Failed
            })
        );
        // the repository is free again, and the last done delivery is readable
        assert_eq!(s.in_flight(&RepoId::new("r1")).unwrap(), None);
        assert_eq!(
            s.last_done(&RepoId::new("r1"))
                .unwrap()
                .unwrap()
                .delivery_id,
            "a"
        );
        assert_eq!(s.last_done(&RepoId::new("r2")).unwrap(), None);
        assert_eq!(s.start(rec("f", "r1", "u", 10), limits()).unwrap(), Ok(()));
        let failed = s
            .advance(
                "f",
                DeliveryState::Staged,
                DeliveryState::Failed,
                DeliveryUpdate {
                    reason: Some("not_a_successor".into()),
                    ..up()
                },
                40,
            )
            .unwrap()
            .unwrap();
        assert_eq!(failed.reason.as_deref(), Some("not_a_successor"));
        // a failed delivery is not "done"
        assert_eq!(
            s.last_done(&RepoId::new("r1"))
                .unwrap()
                .unwrap()
                .delivery_id,
            "a"
        );
        assert_eq!(s.all().unwrap().len(), 3);
    }

    #[test]
    fn the_in_memory_store_keeps_the_contract() {
        contract(&InMemoryDeliveryStore::new());
    }

    #[test]
    fn the_sqlite_store_keeps_the_contract() {
        contract(&SqliteDeliveryStore::new(SqliteDb::open_in_memory().unwrap()).unwrap());
    }

    #[test]
    fn the_schema_is_idempotent_and_the_index_alone_keeps_one_in_flight() {
        let db = SqliteDb::open_in_memory().unwrap();
        let s = SqliteDeliveryStore::new(db.clone()).unwrap();
        let again = SqliteDeliveryStore::new(db.clone()).unwrap();
        s.start(rec("a", "r1", "u", 1), limits()).unwrap().unwrap();
        // behind the store's back: a second in-flight row for the repository is refused by the index
        let conn = db.lock().unwrap();
        let r = conn.execute(
            "INSERT INTO deliveries (delivery_id, repo_id, principal, state, archive_digest, archive_bytes, stage_dir, created_at, updated_at) \
             VALUES ('x', 'r1', 'u', 'staged', 'd', 1, 'x', 1, 1)",
            [],
        );
        assert!(r.is_err());
        drop(conn);
        assert_eq!(again.all().unwrap().len(), 1);
    }

    #[test]
    fn the_diagram_is_the_function() {
        use DeliveryState as S;
        let mut legal = 0;
        for from in S::ALL {
            for to in S::ALL {
                if transition(from, to).is_ok() {
                    legal += 1;
                    assert!(!from.is_final(), "{from:?} is final");
                }
            }
        }
        assert_eq!(legal, 4 + 4 + 3 + 2 + 2);
        for s in S::ALL {
            assert_eq!(S::parse(s.as_str()).unwrap(), s);
        }
        assert!(S::parse("nope").is_err());
        // exchanged and later are never abandoned: nothing after the exchange is rolled back
        assert!(transition(S::Exchanged, S::Abandoned).is_err());
        assert!(transition(S::Reaccepting, S::Abandoned).is_err());
    }
}
