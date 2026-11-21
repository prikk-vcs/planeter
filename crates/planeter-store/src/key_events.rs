//! **Where each trusted maintainer key came from** (RFC 013 D-4): one row per (repository, key), recorded the first time planeter learns
//! how the key came to be trusted — at creation, by a delivery, or by the operator's act.
//!
//! The truth about *which* keys a repository trusts is prikk's own list (`trust maintainer list`); this table says only *how* each came.
//! A key the list has and this table lacks is "not recorded" and is shown without a claim; a row for a key the list does not have is
//! ignored by the reader. **The first record wins** (`INSERT … ON CONFLICT DO NOTHING`): a key cannot become "added" twice, and a later
//! event never rewrites an earlier one. planeter never removes a key (RFC 013 D-1) and this table has no delete: a row says "came from",
//! never "retired".
//!
//! The table is created idempotently; no existing table changes.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::params;

use crate::model::RepoId;
use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

/// The longest `detail` a row keeps, in characters (the operator's reason; empty for the other sources).
pub const DETAIL_MAX_CHARS: usize = 200;

/// How a key came to be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeySource {
    /// Adopted when the repository was created (or, for a repository that existed before this record, found there when the record was
    /// first written: see [`KeyEvent::backfilled`]).
    Creation,
    /// Added by a maintainer's own delivery (RFC 013 D-3; not built yet).
    Delivery,
    /// Added by the server's operator (RFC 013 D-2).
    Operator,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            KeySource::Creation => "creation",
            KeySource::Delivery => "delivery",
            KeySource::Operator => "operator",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "creation" => Ok(KeySource::Creation),
            "delivery" => Ok(KeySource::Delivery),
            "operator" => Ok(KeySource::Operator),
            _ => Err(StoreError::Invalid("an unknown key source".to_owned())),
        }
    }
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub repo_id: RepoId,
    /// prikk's id of the key.
    pub key_id: String,
    /// The public key: 64 lowercase hexadecimal characters (anything else is refused).
    pub public_key: String,
    pub source: KeySource,
    /// The operator's reason; empty for the other sources. **At most [`DETAIL_MAX_CHARS`] characters: a longer text is cut, not refused.**
    pub detail: String,
    /// The delivery that added the key, for [`KeySource::Delivery`].
    pub delivery_id: Option<String>,
    /// Unix seconds.
    pub recorded_at: u64,
    /// The row was written by the one-time backfill (the key was found already trusted, and its source is *known to be* creation only
    /// because in 0.8.0 the creation form was the only way a key got there), not at the moment the key was adopted.
    pub backfilled: bool,
}

impl KeyEvent {
    /// The `detail` as the store keeps it: at most [`DETAIL_MAX_CHARS`] characters (cut at a character boundary).
    pub fn cut_detail(detail: &str) -> String {
        detail.chars().take(DETAIL_MAX_CHARS).collect()
    }

    fn checked(&self) -> Result<KeyEvent> {
        let hex = self.public_key.len() == 64
            && self
                .public_key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !hex {
            return Err(StoreError::Invalid(
                "a public key is 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        if self.key_id.is_empty() || self.key_id.chars().count() > 200 {
            return Err(StoreError::Invalid("a key id of an odd length".to_owned()));
        }
        let mut e = self.clone();
        e.detail = Self::cut_detail(&self.detail);
        Ok(e)
    }
}

/// Where the events are kept. Implementations are safe to share across threads.
pub trait KeyEventStore: Send + Sync {
    /// Record one event. **The first record of a (repository, key) wins**: `Ok(true)` when this one was written, `Ok(false)` when the key
    /// already had a row (nothing changes).
    fn record(&self, event: &KeyEvent) -> Result<bool>;

    /// Record all of `events` **only if the repository has no row at all**, in one step (all or none): `Ok(true)` when they were written,
    /// `Ok(false)` when the repository already had a row (nothing is written). The backfill uses it, so a kill never leaves a partial fill.
    fn record_all_if_none(&self, repo: &RepoId, events: &[KeyEvent]) -> Result<bool>;

    /// The repository's events, ordered by `recorded_at`, then by key id.
    fn events(&self, repo: &RepoId) -> Result<Vec<KeyEvent>>;

    /// Whether the repository has any row.
    fn has_any(&self, repo: &RepoId) -> Result<bool>;
}

fn sorted(mut v: Vec<KeyEvent>) -> Vec<KeyEvent> {
    v.sort_by(|a, b| {
        (a.recorded_at, &a.key_id, &a.public_key).cmp(&(b.recorded_at, &b.key_id, &b.public_key))
    });
    v
}

// ---------------------------------------------------------------------------------------------------- memory

/// In-memory events (tests, development).
#[derive(Default)]
pub struct InMemoryKeyEventStore {
    rows: Mutex<HashMap<(String, String), KeyEvent>>,
}

impl InMemoryKeyEventStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<(String, String), KeyEvent>>> {
        self.rows
            .lock()
            .map_err(|_| StoreError::Backend("key events lock poisoned".to_owned()))
    }
}

impl KeyEventStore for InMemoryKeyEventStore {
    fn record(&self, event: &KeyEvent) -> Result<bool> {
        let e = event.checked()?;
        let mut m = self.lock()?;
        let key = (e.repo_id.as_str().to_owned(), e.public_key.clone());
        if m.contains_key(&key) {
            return Ok(false);
        }
        m.insert(key, e);
        Ok(true)
    }

    fn record_all_if_none(&self, repo: &RepoId, events: &[KeyEvent]) -> Result<bool> {
        let checked: Vec<KeyEvent> = events
            .iter()
            .map(KeyEvent::checked)
            .collect::<Result<_>>()?;
        if checked.iter().any(|e| &e.repo_id != repo) {
            return Err(StoreError::Invalid(
                "an event of another repository".to_owned(),
            ));
        }
        let mut m = self.lock()?;
        if m.keys().any(|(r, _)| r == repo.as_str()) {
            return Ok(false);
        }
        for e in checked {
            m.entry((e.repo_id.as_str().to_owned(), e.public_key.clone()))
                .or_insert(e);
        }
        Ok(true)
    }

    fn events(&self, repo: &RepoId) -> Result<Vec<KeyEvent>> {
        Ok(sorted(
            self.lock()?
                .iter()
                .filter(|((r, _), _)| r == repo.as_str())
                .map(|(_, e)| e.clone())
                .collect(),
        ))
    }

    fn has_any(&self, repo: &RepoId) -> Result<bool> {
        Ok(self.lock()?.keys().any(|(r, _)| r == repo.as_str()))
    }
}

// ---------------------------------------------------------------------------------------------------- SQLite

/// The table. Applied by [`SqliteKeyEventStore::new`]. The primary key is the "first record wins" rule.
pub const KEY_EVENT_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repo_key_events (
    repo_id     TEXT NOT NULL,
    key_id      TEXT NOT NULL,
    public_key  TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('creation','delivery','operator')),
    detail      TEXT NOT NULL DEFAULT '',
    delivery_id TEXT,
    recorded_at INTEGER NOT NULL,
    backfilled  INTEGER NOT NULL DEFAULT 0 CHECK (backfilled IN (0, 1)),
    PRIMARY KEY (repo_id, public_key)
);
";

/// [`KeyEventStore`] over SQLite.
#[derive(Clone)]
pub struct SqliteKeyEventStore {
    db: SqliteDb,
}

impl SqliteKeyEventStore {
    /// Open the store on `db`, creating its table (idempotent).
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(KEY_EVENT_SCHEMA)?;
        Ok(Self { db })
    }
}

const INSERT: &str = "INSERT INTO repo_key_events \
    (repo_id, key_id, public_key, source, detail, delivery_id, recorded_at, backfilled) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (repo_id, public_key) DO NOTHING";

fn insert(conn: &rusqlite::Connection, e: &KeyEvent) -> Result<usize> {
    conn.execute(
        INSERT,
        params![
            e.repo_id.as_str(),
            e.key_id,
            e.public_key,
            e.source.as_str(),
            e.detail,
            e.delivery_id,
            i64::try_from(e.recorded_at).unwrap_or(i64::MAX),
            i64::from(e.backfilled)
        ],
    )
    .map_err(backend)
}

impl KeyEventStore for SqliteKeyEventStore {
    fn record(&self, event: &KeyEvent) -> Result<bool> {
        let e = event.checked()?;
        let conn = self.db.lock()?;
        Ok(insert(&conn, &e)? == 1)
    }

    fn record_all_if_none(&self, repo: &RepoId, events: &[KeyEvent]) -> Result<bool> {
        let checked: Vec<KeyEvent> = events
            .iter()
            .map(KeyEvent::checked)
            .collect::<Result<_>>()?;
        if checked.iter().any(|e| &e.repo_id != repo) {
            return Err(StoreError::Invalid(
                "an event of another repository".to_owned(),
            ));
        }
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        let any: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM repo_key_events WHERE repo_id = ?1",
                params![repo.as_str()],
                |r| r.get(0),
            )
            .map_err(backend)?;
        if any > 0 {
            return Ok(false);
        }
        for e in &checked {
            insert(&tx, e)?;
        }
        tx.commit().map_err(backend)?;
        Ok(true)
    }

    fn events(&self, repo: &RepoId) -> Result<Vec<KeyEvent>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT key_id, public_key, source, detail, delivery_id, recorded_at, backfilled \
                 FROM repo_key_events WHERE repo_id = ?1 ORDER BY recorded_at, key_id, public_key",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![repo.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(backend)?;
        let mut out = Vec::new();
        for row in rows {
            let (key_id, public_key, source, detail, delivery_id, at, backfilled) =
                row.map_err(backend)?;
            out.push(KeyEvent {
                repo_id: repo.clone(),
                key_id,
                public_key,
                source: KeySource::parse(&source)?,
                detail,
                delivery_id,
                recorded_at: u64::try_from(at).unwrap_or(0),
                backfilled: backfilled != 0,
            });
        }
        Ok(out)
    }

    fn has_any(&self, repo: &RepoId) -> Result<bool> {
        let conn = self.db.lock()?;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM repo_key_events WHERE repo_id = ?1",
                params![repo.as_str()],
                |r| r.get(0),
            )
            .map_err(backend)?;
        Ok(n > 0)
    }
}
