//! The records of review (RFC 005 part 6; D-5): **comments** in one-level threads, and **decisions** — one current decision per
//! reviewer per revision. (A *proposal* is the code's *change*: the records name it by `change_id`.)
//!
//! Plain data and one trait, [`ReviewStore`], with an in-memory and a SQLite implementation that pass the same suite.
//! Nothing here calls prikk and nothing here knows who may do what: the rules are `planeter_core::review`'s, and the
//! caller applies them before it writes. What the store itself guarantees, in the database where there is one:
//!
//! * **A comment is immutable and is never deleted.** There is no update and no delete in the trait, and in SQLite
//!   triggers refuse both (`UPDATE` of any column but the three moderation columns, and every `DELETE`). A typo is a new
//!   reply.
//! * **A thread is one level deep, inside one proposal**: a reply's parent must exist, belong to the same proposal and be
//!   a top-level comment (a trigger in SQLite, a check in memory).
//! * **Moderation is all or none**: `hidden_by`, `hidden_reason` and `hidden_at` are set together by [`hide_comment`]
//!   (a `CHECK`) and cleared together by [`unhide_comment`]; every hide and unhide is also appended to a log that nothing
//!   updates.
//! * **One current decision per (proposal, revision, reviewer)** — the primary key. Recording another for the same key
//!   *replaces* it in one transaction and moves the replaced one into an append-only history: newest wins, nothing is lost,
//!   and two writers at once leave exactly one current decision.
//!
//! [`hide_comment`]: ReviewStore::hide_comment
//! [`unhide_comment`]: ReviewStore::unhide_comment

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{OptionalExtension, params};

use crate::sqlite::{SqliteDb, backend};
use crate::store::{Result, StoreError};

/// The longest comment body, in bytes. (The database `CHECK` and `planeter_core::review` say the same number.)
pub const COMMENT_BODY_MAX_BYTES: usize = 16 * 1024;
/// The longest note on a decision, in bytes.
pub const DECISION_NOTE_MAX_BYTES: usize = 2 * 1024;
/// The longest path of an anchor, in characters.
pub const COMMENT_PATH_MAX_CHARS: usize = 1024;

/// A comment hidden by a maintainer: who, why, when. All three or none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenRecord {
    pub by: String,
    pub reason: String,
    pub at: u64,
}

/// One comment. Immutable once written, except for its moderation record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentRecord {
    pub comment_id: String,
    pub change_id: String,
    /// The revision that was the latest when the comment was written.
    pub revision_seq: u32,
    /// The patch it is about, kept as written even when a later revision drops the patch.
    pub patch_id: Option<String>,
    /// Free text, never opened.
    pub path: Option<String>,
    pub line_from: Option<u32>,
    pub line_to: Option<u32>,
    /// The thread's first comment, for a reply.
    pub parent: Option<String>,
    pub author: String,
    /// Markdown source.
    pub body: String,
    pub created_at: u64,
    pub hidden: Option<HiddenRecord>,
}

/// One reviewer's decision on one revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRecord {
    pub change_id: String,
    pub revision_seq: u32,
    pub reviewer: String,
    /// `approve` or `request-changes`.
    pub verdict: String,
    pub note: Option<String>,
    pub created_at: u64,
}

/// A hide or an unhide, as the log keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationEntry {
    pub comment_id: String,
    /// `hide` or `unhide`.
    pub action: String,
    pub by: String,
    /// The reason of a hide.
    pub reason: Option<String>,
    pub at: u64,
}

/// A failure point the tests can arm to prove atomicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewFailPoint {
    /// After a replaced decision was moved to the history, before the new one was written.
    AfterHistoryInsert,
}

/// Persistence of comments and decisions. Safe to share across threads.
pub trait ReviewStore: Send + Sync {
    /// Write a comment. [`StoreError::Invalid`] for a rule the store enforces (empty or oversized body, a bad anchor, a
    /// parent that is not a top-level comment of the same proposal); [`StoreError::Conflict`] for a taken id.
    fn add_comment(&self, c: &CommentRecord) -> Result<()>;
    fn get_comment(&self, comment_id: &str) -> Result<Option<CommentRecord>>;
    /// The proposal's top-level comments, oldest first by `(created_at, comment_id)`, those after the cursor, at most `limit`.
    fn comment_threads(
        &self,
        change_id: &str,
        after: Option<(u64, &str)>,
        limit: usize,
    ) -> Result<Vec<CommentRecord>>;
    /// Every reply to any of `parents`, oldest first.
    fn replies_of(&self, change_id: &str, parents: &[String]) -> Result<Vec<CommentRecord>>;
    /// How many comments (top-level and replies) the proposal has.
    fn count_comments(&self, change_id: &str) -> Result<u64>;
    /// When `author` commented on the proposal after `since` (exclusive), for the rate limit.
    fn comment_times(&self, change_id: &str, author: &str, since: u64) -> Result<Vec<u64>>;

    /// Hide a comment with a reason. [`StoreError::NotFound`]; [`StoreError::Duplicate`] if it is hidden already.
    fn hide_comment(&self, comment_id: &str, by: &str, reason: &str, now: u64) -> Result<()>;
    /// Make a hidden comment visible again. [`StoreError::NotFound`]; [`StoreError::Duplicate`] if it is not hidden.
    fn unhide_comment(&self, comment_id: &str, by: &str, now: u64) -> Result<()>;
    /// Every hide and unhide of a comment, oldest first.
    fn moderation_log(&self, comment_id: &str) -> Result<Vec<ModerationEntry>>;

    /// Record a decision. If the reviewer has a current decision on that revision it is replaced (and kept in the history).
    fn record_decision(&self, d: &DecisionRecord) -> Result<()>;
    /// Every current decision of the proposal, by revision then reviewer.
    fn current_decisions(&self, change_id: &str) -> Result<Vec<DecisionRecord>>;
    /// The current decisions of several proposals, each on the named revision only (the list's review word).
    fn decisions_on(&self, pairs: &[(String, u32)]) -> Result<Vec<DecisionRecord>>;
    /// The decisions that were replaced, oldest first (with `created_at` of the replaced decision).
    fn decision_history(&self, change_id: &str) -> Result<Vec<DecisionRecord>>;
    /// When `reviewer` decided on the proposal after `since` (exclusive), current and replaced alike, for the rate limit.
    fn decision_times(&self, change_id: &str, reviewer: &str, since: u64) -> Result<Vec<u64>>;

    /// Arm or disarm a failure point (tests).
    fn inject_failure(&self, point: ReviewFailPoint, on: bool);
}

fn check_comment(c: &CommentRecord) -> Result<()> {
    let invalid = |what: &str| Err(StoreError::Invalid(what.to_owned()));
    if c.comment_id.is_empty() || c.change_id.is_empty() || c.author.is_empty() {
        return invalid("a comment has an id, a proposal and an author");
    }
    if c.revision_seq < 1 {
        return invalid("a comment's revision starts at 1");
    }
    if c.body.trim().is_empty() {
        return invalid("a comment says something");
    }
    if c.body.len() > COMMENT_BODY_MAX_BYTES {
        return invalid("a comment fits in 16 KiB");
    }
    if let Some(p) = &c.patch_id
        && !(p.len() == 64 && p.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    {
        return invalid("a patch id is 64 lowercase hexadecimal characters");
    }
    if let Some(p) = &c.path
        && (p.is_empty() || p.chars().count() > COMMENT_PATH_MAX_CHARS)
    {
        return invalid("a path is 1 to 1,024 characters");
    }
    match (c.line_from, c.line_to) {
        (None, None) => {}
        (Some(f), Some(t)) if f >= 1 && f <= t && c.path.is_some() => {}
        _ => return invalid("lines are positive, ordered, both given, and anchored to a path"),
    }
    if let Some(h) = &c.hidden {
        let _ = h;
        return invalid("a comment is written visible; hiding is a separate act");
    }
    Ok(())
}

fn check_decision(d: &DecisionRecord) -> Result<()> {
    if d.change_id.is_empty() || d.reviewer.is_empty() || d.revision_seq < 1 {
        return Err(StoreError::Invalid(
            "a decision has a proposal, a revision and a reviewer".to_owned(),
        ));
    }
    if d.verdict != "approve" && d.verdict != "request-changes" {
        return Err(StoreError::Invalid(
            "a decision is approve or request-changes".to_owned(),
        ));
    }
    if d.note
        .as_ref()
        .is_some_and(|n| n.len() > DECISION_NOTE_MAX_BYTES)
    {
        return Err(StoreError::Invalid("a note fits in 2 KiB".to_owned()));
    }
    Ok(())
}

fn check_hide(reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        return Err(StoreError::Invalid("hiding needs a reason".to_owned()));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ in memory

#[derive(Default)]
struct MemState {
    comments: Vec<CommentRecord>,
    log: Vec<ModerationEntry>,
    decisions: Vec<DecisionRecord>,
    history: Vec<DecisionRecord>,
}

/// The in-memory [`ReviewStore`] (tests and development).
#[derive(Default)]
pub struct InMemoryReviewStore {
    state: Mutex<MemState>,
    fail_after_history: AtomicBool,
}

impl InMemoryReviewStore {
    pub fn new() -> Self {
        Self::default()
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemState>> {
        self.state
            .lock()
            .map_err(|_| StoreError::Backend("review state lock poisoned".to_owned()))
    }
}

impl ReviewStore for InMemoryReviewStore {
    fn add_comment(&self, c: &CommentRecord) -> Result<()> {
        check_comment(c)?;
        let mut m = self.lock()?;
        if m.comments.iter().any(|x| x.comment_id == c.comment_id) {
            return Err(StoreError::Conflict("comment id".to_owned()));
        }
        if let Some(parent) = &c.parent {
            let ok = m.comments.iter().any(|p| {
                &p.comment_id == parent && p.parent.is_none() && p.change_id == c.change_id
            });
            if !ok {
                return Err(StoreError::Invalid(
                    "a thread is one level deep within one proposal".to_owned(),
                ));
            }
        }
        m.comments.push(c.clone());
        Ok(())
    }

    fn get_comment(&self, id: &str) -> Result<Option<CommentRecord>> {
        Ok(self
            .lock()?
            .comments
            .iter()
            .find(|c| c.comment_id == id)
            .cloned())
    }

    fn comment_threads(
        &self,
        change_id: &str,
        after: Option<(u64, &str)>,
        limit: usize,
    ) -> Result<Vec<CommentRecord>> {
        let m = self.lock()?;
        let mut v: Vec<CommentRecord> = m
            .comments
            .iter()
            .filter(|c| c.change_id == change_id && c.parent.is_none())
            .filter(|c| after.is_none_or(|(t, id)| (c.created_at, c.comment_id.as_str()) > (t, id)))
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.created_at, &a.comment_id).cmp(&(b.created_at, &b.comment_id)));
        v.truncate(limit);
        Ok(v)
    }

    fn replies_of(&self, change_id: &str, parents: &[String]) -> Result<Vec<CommentRecord>> {
        let m = self.lock()?;
        let mut v: Vec<CommentRecord> = m
            .comments
            .iter()
            .filter(|c| {
                c.change_id == change_id && c.parent.as_ref().is_some_and(|p| parents.contains(p))
            })
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.created_at, &a.comment_id).cmp(&(b.created_at, &b.comment_id)));
        Ok(v)
    }

    fn count_comments(&self, change_id: &str) -> Result<u64> {
        Ok(self
            .lock()?
            .comments
            .iter()
            .filter(|c| c.change_id == change_id)
            .count() as u64)
    }

    fn comment_times(&self, change_id: &str, author: &str, since: u64) -> Result<Vec<u64>> {
        Ok(self
            .lock()?
            .comments
            .iter()
            .filter(|c| c.change_id == change_id && c.author == author && c.created_at > since)
            .map(|c| c.created_at)
            .collect())
    }

    fn hide_comment(&self, id: &str, by: &str, reason: &str, now: u64) -> Result<()> {
        check_hide(reason)?;
        let mut m = self.lock()?;
        let c = m
            .comments
            .iter_mut()
            .find(|c| c.comment_id == id)
            .ok_or(StoreError::NotFound)?;
        if c.hidden.is_some() {
            return Err(StoreError::Duplicate("the comment is hidden already"));
        }
        c.hidden = Some(HiddenRecord {
            by: by.to_owned(),
            reason: reason.to_owned(),
            at: now,
        });
        m.log.push(ModerationEntry {
            comment_id: id.to_owned(),
            action: "hide".to_owned(),
            by: by.to_owned(),
            reason: Some(reason.to_owned()),
            at: now,
        });
        Ok(())
    }

    fn unhide_comment(&self, id: &str, by: &str, now: u64) -> Result<()> {
        let mut m = self.lock()?;
        let c = m
            .comments
            .iter_mut()
            .find(|c| c.comment_id == id)
            .ok_or(StoreError::NotFound)?;
        if c.hidden.is_none() {
            return Err(StoreError::Duplicate("the comment is not hidden"));
        }
        c.hidden = None;
        m.log.push(ModerationEntry {
            comment_id: id.to_owned(),
            action: "unhide".to_owned(),
            by: by.to_owned(),
            reason: None,
            at: now,
        });
        Ok(())
    }

    fn moderation_log(&self, id: &str) -> Result<Vec<ModerationEntry>> {
        Ok(self
            .lock()?
            .log
            .iter()
            .filter(|e| e.comment_id == id)
            .cloned()
            .collect())
    }

    fn record_decision(&self, d: &DecisionRecord) -> Result<()> {
        check_decision(d)?;
        let mut m = self.lock()?;
        let key = |x: &DecisionRecord| {
            x.change_id == d.change_id
                && x.revision_seq == d.revision_seq
                && x.reviewer == d.reviewer
        };
        let old = m.decisions.iter().position(key);
        // Everything that can fail is done on a copy first, so a failure leaves the state untouched (atomic).
        let mut next_decisions = m.decisions.clone();
        let mut next_history = m.history.clone();
        if let Some(i) = old {
            next_history.push(next_decisions.remove(i));
        }
        if self.fail_after_history.load(Ordering::Relaxed) {
            return Err(StoreError::Backend(
                "injected failure after the history insert".to_owned(),
            ));
        }
        next_decisions.push(d.clone());
        m.decisions = next_decisions;
        m.history = next_history;
        Ok(())
    }

    fn current_decisions(&self, change_id: &str) -> Result<Vec<DecisionRecord>> {
        let mut v: Vec<DecisionRecord> = self
            .lock()?
            .decisions
            .iter()
            .filter(|d| d.change_id == change_id)
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.revision_seq, &a.reviewer).cmp(&(b.revision_seq, &b.reviewer)));
        Ok(v)
    }

    fn decisions_on(&self, pairs: &[(String, u32)]) -> Result<Vec<DecisionRecord>> {
        Ok(self
            .lock()?
            .decisions
            .iter()
            .filter(|d| {
                pairs
                    .iter()
                    .any(|(c, s)| *c == d.change_id && *s == d.revision_seq)
            })
            .cloned()
            .collect())
    }

    fn decision_history(&self, change_id: &str) -> Result<Vec<DecisionRecord>> {
        Ok(self
            .lock()?
            .history
            .iter()
            .filter(|d| d.change_id == change_id)
            .cloned()
            .collect())
    }

    fn decision_times(&self, change_id: &str, reviewer: &str, since: u64) -> Result<Vec<u64>> {
        let m = self.lock()?;
        Ok(m.decisions
            .iter()
            .chain(m.history.iter())
            .filter(|d| d.change_id == change_id && d.reviewer == reviewer && d.created_at > since)
            .map(|d| d.created_at)
            .collect())
    }

    fn inject_failure(&self, point: ReviewFailPoint, on: bool) {
        match point {
            ReviewFailPoint::AfterHistoryInsert => {
                self.fail_after_history.store(on, Ordering::Relaxed)
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ SQLite

/// The schema, as created (idempotent). It sits beside the change tables (`comments.change_id` and `decisions.change_id`
/// refer to `changes`), so a database made by part 5 gets these tables and nothing else changes.
pub const REVIEW_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS comments (
    comment_id    TEXT PRIMARY KEY,
    change_id     TEXT NOT NULL REFERENCES changes (change_id),
    revision_seq  INTEGER NOT NULL CHECK (revision_seq >= 1),
    patch_id      TEXT CHECK (patch_id IS NULL OR length(patch_id) = 64),
    path          TEXT CHECK (path IS NULL OR (length(path) >= 1 AND length(path) <= 1024)),
    line_from     INTEGER CHECK (line_from IS NULL OR line_from >= 1),
    line_to       INTEGER CHECK (line_to IS NULL OR line_to >= 1),
    parent_id     TEXT REFERENCES comments (comment_id),
    author        TEXT NOT NULL CHECK (length(author) >= 1),
    body          TEXT NOT NULL CHECK (length(trim(body, ' ' || char(9, 10, 13))) >= 1 AND length(CAST(body AS BLOB)) <= 16384),
    created_at    INTEGER NOT NULL,
    hidden_by     TEXT,
    hidden_reason TEXT,
    hidden_at     INTEGER,
    CHECK ((line_from IS NULL) = (line_to IS NULL)),
    CHECK (line_from IS NULL OR line_from <= line_to),
    CHECK (line_from IS NULL OR path IS NOT NULL),
    CHECK ((hidden_by IS NULL AND hidden_reason IS NULL AND hidden_at IS NULL)
        OR (hidden_by IS NOT NULL AND hidden_reason IS NOT NULL AND hidden_at IS NOT NULL
            AND length(trim(hidden_reason, ' ' || char(9, 10, 13))) >= 1))
);
CREATE INDEX IF NOT EXISTS comments_by_change ON comments (change_id, created_at, comment_id);
CREATE INDEX IF NOT EXISTS comments_by_author ON comments (change_id, author, created_at);
CREATE INDEX IF NOT EXISTS comments_by_parent ON comments (parent_id);

CREATE TRIGGER IF NOT EXISTS comments_never_deleted BEFORE DELETE ON comments
BEGIN SELECT RAISE(ABORT, 'comments are never deleted'); END;
CREATE TRIGGER IF NOT EXISTS comments_immutable
BEFORE UPDATE OF comment_id, change_id, revision_seq, patch_id, path, line_from, line_to, parent_id, author, body, created_at ON comments
BEGIN SELECT RAISE(ABORT, 'comments are immutable'); END;
CREATE TRIGGER IF NOT EXISTS comments_thread_depth BEFORE INSERT ON comments WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a thread is one level deep within one proposal')
    WHERE NOT EXISTS (SELECT 1 FROM comments p WHERE p.comment_id = NEW.parent_id AND p.parent_id IS NULL AND p.change_id = NEW.change_id);
END;
CREATE TRIGGER IF NOT EXISTS comments_written_visible BEFORE INSERT ON comments WHEN NEW.hidden_by IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'a comment is written visible; hiding is a separate act'); END;

CREATE TABLE IF NOT EXISTS comment_moderation (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id TEXT NOT NULL REFERENCES comments (comment_id),
    action     TEXT NOT NULL CHECK (action IN ('hide', 'unhide')),
    by_user    TEXT NOT NULL,
    reason     TEXT,
    at         INTEGER NOT NULL,
    CHECK ((action = 'hide') = (reason IS NOT NULL))
);
CREATE TRIGGER IF NOT EXISTS comment_moderation_append_only_u BEFORE UPDATE ON comment_moderation
BEGIN SELECT RAISE(ABORT, 'the moderation log is append-only'); END;
CREATE TRIGGER IF NOT EXISTS comment_moderation_append_only_d BEFORE DELETE ON comment_moderation
BEGIN SELECT RAISE(ABORT, 'the moderation log is append-only'); END;

CREATE TABLE IF NOT EXISTS decisions (
    change_id    TEXT NOT NULL REFERENCES changes (change_id),
    revision_seq INTEGER NOT NULL CHECK (revision_seq >= 1),
    reviewer     TEXT NOT NULL CHECK (length(reviewer) >= 1),
    verdict      TEXT NOT NULL CHECK (verdict IN ('approve', 'request-changes')),
    note         TEXT CHECK (note IS NULL OR length(CAST(note AS BLOB)) <= 2048),
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (change_id, revision_seq, reviewer)
);
CREATE INDEX IF NOT EXISTS decisions_by_reviewer ON decisions (change_id, reviewer, created_at);

CREATE TABLE IF NOT EXISTS decision_history (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    change_id    TEXT NOT NULL REFERENCES changes (change_id),
    revision_seq INTEGER NOT NULL CHECK (revision_seq >= 1),
    reviewer     TEXT NOT NULL,
    verdict      TEXT NOT NULL CHECK (verdict IN ('approve', 'request-changes')),
    note         TEXT,
    created_at   INTEGER NOT NULL,
    superseded_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS decision_history_by_reviewer ON decision_history (change_id, reviewer, created_at);
CREATE TRIGGER IF NOT EXISTS decision_history_append_only_u BEFORE UPDATE ON decision_history
BEGIN SELECT RAISE(ABORT, 'the decision history is append-only'); END;
CREATE TRIGGER IF NOT EXISTS decision_history_append_only_d BEFORE DELETE ON decision_history
BEGIN SELECT RAISE(ABORT, 'the decision history is append-only'); END;
";

/// The SQLite [`ReviewStore`], over the server's shared database.
#[derive(Clone)]
pub struct SqliteReviewStore {
    db: SqliteDb,
    fail_after_history: std::sync::Arc<AtomicBool>,
}

impl SqliteReviewStore {
    /// Open over `db`, creating the tables, triggers and indexes if they are not there (idempotent). The `changes` table
    /// must exist (open [`crate::SqliteChangeStore`] on the same database first).
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(REVIEW_SCHEMA)?;
        Ok(Self {
            db,
            fail_after_history: Default::default(),
        })
    }
}

const SELECT_COMMENT: &str = "SELECT comment_id, change_id, revision_seq, patch_id, path, line_from, line_to, parent_id, author, body, created_at, hidden_by, hidden_reason, hidden_at FROM comments";

fn comment_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<CommentRecord> {
    let hidden_by: Option<String> = r.get(11)?;
    let hidden_reason: Option<String> = r.get(12)?;
    let hidden_at: Option<i64> = r.get(13)?;
    Ok(CommentRecord {
        comment_id: r.get(0)?,
        change_id: r.get(1)?,
        revision_seq: r.get::<_, i64>(2)? as u32,
        patch_id: r.get(3)?,
        path: r.get(4)?,
        line_from: r.get::<_, Option<i64>>(5)?.map(|n| n as u32),
        line_to: r.get::<_, Option<i64>>(6)?.map(|n| n as u32),
        parent: r.get(7)?,
        author: r.get(8)?,
        body: r.get(9)?,
        created_at: r.get::<_, i64>(10)? as u64,
        hidden: match (hidden_by, hidden_reason, hidden_at) {
            (Some(by), Some(reason), Some(at)) => Some(HiddenRecord {
                by,
                reason,
                at: at as u64,
            }),
            _ => None,
        },
    })
}

fn decision_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<DecisionRecord> {
    Ok(DecisionRecord {
        change_id: r.get(0)?,
        revision_seq: r.get::<_, i64>(1)? as u32,
        reviewer: r.get(2)?,
        verdict: r.get(3)?,
        note: r.get(4)?,
        created_at: r.get::<_, i64>(5)? as u64,
    })
}

/// A database rule that says no is the caller's mistake (`Invalid`), a taken key is a `Conflict`; anything else is the
/// backend's.
fn map_write(e: rusqlite::Error, taken: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(f, msg) = &e
        && f.code == rusqlite::ErrorCode::ConstraintViolation
    {
        let m = msg.clone().unwrap_or_default();
        if m.contains("UNIQUE") || m.contains("PRIMARY KEY") {
            return StoreError::Conflict(taken.to_owned());
        }
        return StoreError::Invalid(if m.is_empty() {
            "a rule of the store".to_owned()
        } else {
            m
        });
    }
    backend(e)
}

impl ReviewStore for SqliteReviewStore {
    fn add_comment(&self, c: &CommentRecord) -> Result<()> {
        check_comment(c)?;
        let conn = self.db.lock()?;
        conn.execute(
            "INSERT INTO comments (comment_id, change_id, revision_seq, patch_id, path, line_from, line_to, parent_id, author, body, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                c.comment_id,
                c.change_id,
                i64::from(c.revision_seq),
                c.patch_id,
                c.path,
                c.line_from.map(i64::from),
                c.line_to.map(i64::from),
                c.parent,
                c.author,
                c.body,
                c.created_at as i64
            ],
        )
        .map_err(|e| map_write(e, "comment id"))?;
        Ok(())
    }

    fn get_comment(&self, id: &str) -> Result<Option<CommentRecord>> {
        let conn = self.db.lock()?;
        conn.query_row(
            &format!("{SELECT_COMMENT} WHERE comment_id = ?1"),
            params![id],
            comment_from,
        )
        .optional()
        .map_err(backend)
    }

    fn comment_threads(
        &self,
        change_id: &str,
        after: Option<(u64, &str)>,
        limit: usize,
    ) -> Result<Vec<CommentRecord>> {
        let conn = self.db.lock()?;
        let (t, id) = after.map(|(t, id)| (t as i64, id)).unwrap_or((-1, ""));
        let mut stmt = conn
            .prepare(&format!(
                "{SELECT_COMMENT} WHERE change_id = ?1 AND parent_id IS NULL AND (created_at > ?2 OR (created_at = ?2 AND comment_id > ?3)) ORDER BY created_at, comment_id LIMIT ?4"
            ))
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![change_id, t, id, limit as i64], comment_from)
            .map_err(backend)?;
        rows.collect::<std::result::Result<_, _>>().map_err(backend)
    }

    fn replies_of(&self, change_id: &str, parents: &[String]) -> Result<Vec<CommentRecord>> {
        if parents.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.db.lock()?;
        let marks = vec!["?"; parents.len()].join(",");
        let mut stmt = conn
            .prepare(&format!(
                "{SELECT_COMMENT} WHERE change_id = ?1 AND parent_id IN ({marks}) ORDER BY created_at, comment_id"
            ))
            .map_err(backend)?;
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&change_id];
        args.extend(parents.iter().map(|p| p as &dyn rusqlite::ToSql));
        let rows = stmt
            .query_map(args.as_slice(), comment_from)
            .map_err(backend)?;
        rows.collect::<std::result::Result<_, _>>().map_err(backend)
    }

    fn count_comments(&self, change_id: &str) -> Result<u64> {
        let conn = self.db.lock()?;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comments WHERE change_id = ?1",
                params![change_id],
                |r| r.get(0),
            )
            .map_err(backend)?;
        Ok(n as u64)
    }

    fn comment_times(&self, change_id: &str, author: &str, since: u64) -> Result<Vec<u64>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT created_at FROM comments WHERE change_id = ?1 AND author = ?2 AND created_at > ?3")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![change_id, author, since as i64], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(backend)?;
        rows.map(|r| r.map(|n| n as u64).map_err(backend)).collect()
    }

    fn hide_comment(&self, id: &str, by: &str, reason: &str, now: u64) -> Result<()> {
        check_hide(reason)?;
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        let hidden: Option<Option<String>> = tx
            .query_row(
                "SELECT hidden_by FROM comments WHERE comment_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        match hidden {
            None => return Err(StoreError::NotFound),
            Some(Some(_)) => return Err(StoreError::Duplicate("the comment is hidden already")),
            Some(None) => {}
        }
        tx.execute(
            "UPDATE comments SET hidden_by = ?2, hidden_reason = ?3, hidden_at = ?4 WHERE comment_id = ?1",
            params![id, by, reason, now as i64],
        )
        .map_err(|e| map_write(e, "comment"))?;
        tx.execute(
            "INSERT INTO comment_moderation (comment_id, action, by_user, reason, at) VALUES (?1, 'hide', ?2, ?3, ?4)",
            params![id, by, reason, now as i64],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn unhide_comment(&self, id: &str, by: &str, now: u64) -> Result<()> {
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        let hidden: Option<Option<String>> = tx
            .query_row(
                "SELECT hidden_by FROM comments WHERE comment_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        match hidden {
            None => return Err(StoreError::NotFound),
            Some(None) => return Err(StoreError::Duplicate("the comment is not hidden")),
            Some(Some(_)) => {}
        }
        tx.execute(
            "UPDATE comments SET hidden_by = NULL, hidden_reason = NULL, hidden_at = NULL WHERE comment_id = ?1",
            params![id],
        )
        .map_err(backend)?;
        tx.execute(
            "INSERT INTO comment_moderation (comment_id, action, by_user, reason, at) VALUES (?1, 'unhide', ?2, NULL, ?3)",
            params![id, by, now as i64],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn moderation_log(&self, id: &str) -> Result<Vec<ModerationEntry>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT comment_id, action, by_user, reason, at FROM comment_moderation WHERE comment_id = ?1 ORDER BY id")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id], |r| {
                Ok(ModerationEntry {
                    comment_id: r.get(0)?,
                    action: r.get(1)?,
                    by: r.get(2)?,
                    reason: r.get(3)?,
                    at: r.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(backend)?;
        rows.collect::<std::result::Result<_, _>>().map_err(backend)
    }

    fn record_decision(&self, d: &DecisionRecord) -> Result<()> {
        check_decision(d)?;
        let mut conn = self.db.lock()?;
        let tx = conn.transaction().map_err(backend)?;
        // The replaced decision, if any, goes to the history in the same transaction.
        tx.execute(
            "INSERT INTO decision_history (change_id, revision_seq, reviewer, verdict, note, created_at, superseded_at)
             SELECT change_id, revision_seq, reviewer, verdict, note, created_at, ?4 FROM decisions
             WHERE change_id = ?1 AND revision_seq = ?2 AND reviewer = ?3",
            params![d.change_id, i64::from(d.revision_seq), d.reviewer, d.created_at as i64],
        )
        .map_err(|e| map_write(e, "decision"))?;
        if self.fail_after_history.load(Ordering::Relaxed) {
            return Err(StoreError::Backend(
                "injected failure after the history insert".to_owned(),
            ));
        }
        tx.execute(
            "INSERT INTO decisions (change_id, revision_seq, reviewer, verdict, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (change_id, revision_seq, reviewer)
             DO UPDATE SET verdict = excluded.verdict, note = excluded.note, created_at = excluded.created_at",
            params![
                d.change_id,
                i64::from(d.revision_seq),
                d.reviewer,
                d.verdict,
                d.note,
                d.created_at as i64
            ],
        )
        .map_err(|e| map_write(e, "decision"))?;
        tx.commit().map_err(backend)
    }

    fn current_decisions(&self, change_id: &str) -> Result<Vec<DecisionRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT change_id, revision_seq, reviewer, verdict, note, created_at FROM decisions WHERE change_id = ?1 ORDER BY revision_seq, reviewer")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![change_id], decision_from)
            .map_err(backend)?;
        rows.collect::<std::result::Result<_, _>>().map_err(backend)
    }

    fn decisions_on(&self, pairs: &[(String, u32)]) -> Result<Vec<DecisionRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT change_id, revision_seq, reviewer, verdict, note, created_at FROM decisions WHERE change_id = ?1 AND revision_seq = ?2")
            .map_err(backend)?;
        let mut out = Vec::new();
        for (c, s) in pairs {
            let rows = stmt
                .query_map(params![c, i64::from(*s)], decision_from)
                .map_err(backend)?;
            for r in rows {
                out.push(r.map_err(backend)?);
            }
        }
        Ok(out)
    }

    fn decision_history(&self, change_id: &str) -> Result<Vec<DecisionRecord>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare("SELECT change_id, revision_seq, reviewer, verdict, note, created_at FROM decision_history WHERE change_id = ?1 ORDER BY id")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![change_id], decision_from)
            .map_err(backend)?;
        rows.collect::<std::result::Result<_, _>>().map_err(backend)
    }

    fn decision_times(&self, change_id: &str, reviewer: &str, since: u64) -> Result<Vec<u64>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT created_at FROM decisions WHERE change_id = ?1 AND reviewer = ?2 AND created_at > ?3
                 UNION ALL
                 SELECT created_at FROM decision_history WHERE change_id = ?1 AND reviewer = ?2 AND created_at > ?3",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![change_id, reviewer, since as i64], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(backend)?;
        rows.map(|r| r.map(|n| n as u64).map_err(backend)).collect()
    }

    fn inject_failure(&self, point: ReviewFailPoint, on: bool) {
        match point {
            ReviewFailPoint::AfterHistoryInsert => {
                self.fail_after_history.store(on, Ordering::Relaxed)
            }
        }
    }
}
