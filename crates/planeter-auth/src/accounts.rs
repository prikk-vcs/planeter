//! The writes an account has (RFC 014 T6): create, set a password, link and unlink an identity, disable and enable. **Each is one
//! transaction** (`BEGIN IMMEDIATE`, so a second process writing the same database waits its turn for at most the 5 s busy timeout and is
//! never interleaved), **each writes its event in that same transaction**, and **each refuses with a closed error that changes nothing**:
//! a refusal leaves no row, no event and no ended session; a write that fails half way is rolled back whole, event included.
//!
//! No command and no page calls these yet: the operator's commands (`planeter operator account …`) are the next handoff, and the settings
//! pages the step after. What is decided here is what they will stand on: the rules, the transaction, and what is recorded.
//!
//! **What an event never holds:** a password, a hash, a token, a subject, an address. A password is hashed *before* the transaction opens
//! (Argon2id is slow and the write lock must not be held through it); an identity event names the provider's **host** and nothing of the
//! person's identity at it.

use std::sync::Arc;

use rusqlite::params;

use planeter_core::creation::{HandleError, check_new_handle, handle_key};
use planeter_store::SqliteDb;

use crate::events::{AccountEvent, AccountEventKind, EventBy, insert_event};
use crate::hashing::PasswordHasher;
use crate::identity::AuthError;
use crate::password::NormalizedPassword;
use crate::sqlite::{handle_taken, immediate, prepare};
use crate::text::{escape_display, is_display_safe};

/// The longest display name, in characters.
pub const DISPLAY_NAME_MAX: usize = 100;

/// Why a display name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayNameError {
    Empty,
    TooLong,
    /// It carries a control, bidirectional or zero-width character.
    Unsafe,
}

impl DisplayNameError {
    pub fn reason(self) -> &'static str {
        match self {
            DisplayNameError::Empty => "empty",
            DisplayNameError::TooLong => "too_long",
            DisplayNameError::Unsafe => "unsafe",
        }
    }
}

/// Why an account write was refused. Closed; **nothing was changed**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
    /// The handle does not follow the rule of a new handle (RFC 014 D-2).
    Handle(HandleError),
    /// An account, an organization or the owner of a repository already has this handle, compared case-insensitively.
    HandleTaken,
    DisplayName(DisplayNameError),
    NoSuchAccount,
    /// The identity is linked to **another** account.
    IdentityTaken,
    /// The identity is not linked to this account.
    NotLinked,
    Backend(String),
}

impl AccountError {
    /// A stable word for a message key and a log.
    pub fn reason(&self) -> &'static str {
        match self {
            AccountError::Handle(_) => "handle",
            AccountError::HandleTaken => "handle_taken",
            AccountError::DisplayName(_) => "display_name",
            AccountError::NoSuchAccount => "no_such_account",
            AccountError::IdentityTaken => "identity_taken",
            AccountError::NotLinked => "not_linked",
            AccountError::Backend(_) => "backend",
        }
    }
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountError::Handle(e) => write!(f, "the handle is refused: {}", e.reason()),
            AccountError::DisplayName(e) => {
                write!(f, "the display name is refused: {}", e.reason())
            }
            other => write!(f, "the account write was refused: {}", other.reason()),
        }
    }
}

impl std::error::Error for AccountError {}

impl From<AuthError> for AccountError {
    fn from(e: AuthError) -> Self {
        AccountError::Backend(e.to_string())
    }
}

/// The display name as it is stored: trimmed, 1 to [`DISPLAY_NAME_MAX`] characters, display-safe (refused, not escaped: a name is an
/// identity, not a log line).
pub fn check_display_name(name: &str) -> Result<String, DisplayNameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(DisplayNameError::Empty);
    }
    if trimmed.chars().count() > DISPLAY_NAME_MAX {
        return Err(DisplayNameError::TooLong);
    }
    if !is_display_safe(trimmed) {
        return Err(DisplayNameError::Unsafe);
    }
    Ok(trimmed.to_owned())
}

/// The host (and port) of an issuer URL, for an event's detail: **never** the path, a subject or a credential in the URL.
fn issuer_host(issuer: &str) -> String {
    let rest = issuer.split_once("://").map_or(issuer, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    escape_display(host, 100)
}

/// The account writes, over the server's shared database.
#[derive(Clone)]
pub struct AccountService {
    db: SqliteDb,
    hasher: Arc<dyn PasswordHasher>,
    reserved: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

impl AccountService {
    /// `hasher` makes the stored hash of a password (Argon2id in the server); `reserved` says whether a word belongs to the server (a path
    /// it answers on), as the creation of a repository asks.
    pub fn new(
        db: SqliteDb,
        hasher: Arc<dyn PasswordHasher>,
        reserved: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    ) -> Result<Self, AccountError> {
        prepare(&db)?;
        Ok(Self {
            db,
            hasher,
            reserved,
        })
    }

    /// Run `work` in one immediate transaction; `Ok(Some(e))` means the work **refused** before writing anything (the empty transaction
    /// ends) and `Err` means a write failed (everything is rolled back, the event included).
    fn txn(
        &self,
        work: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<Option<AccountError>>,
    ) -> Result<(), AccountError> {
        match immediate(&self.db, work)? {
            None => Ok(()),
            Some(refusal) => Err(refusal),
        }
    }

    /// Make an account (RFC 014 D-2): the handle rule, case-insensitive uniqueness across accounts, organizations and repository owners,
    /// a display name, and **no password unless one is given** (then the account signs in only through a linked identity). Writes
    /// `created`, and `password_set` when there is a password.
    pub fn create_account(
        &self,
        handle: &str,
        display_name: &str,
        password: Option<&NormalizedPassword>,
        by: EventBy,
        now: u64,
    ) -> Result<(), AccountError> {
        check_new_handle(handle, &*self.reserved).map_err(AccountError::Handle)?;
        let name = check_display_name(display_name).map_err(AccountError::DisplayName)?;
        let key = handle_key(handle);
        let hash = password.map(|p| self.hasher.hash(p.as_str()).0);
        let at = i64::try_from(now).unwrap_or(i64::MAX);
        self.txn(|conn| {
            if handle_taken(conn, &key)? {
                return Ok(Some(AccountError::HandleTaken));
            }
            conn.execute(
                "INSERT INTO accounts (user, display_name, email, password_hash, created_at, password_changed_at, handle_key, handle_collides) \
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, 0)",
                params![handle, name, hash, at, hash.as_ref().map(|_| at), key],
            )?;
            insert_event(conn, &AccountEvent::new(handle, AccountEventKind::Created, by, "", now))?;
            if hash.is_some() {
                insert_event(conn, &AccountEvent::new(handle, AccountEventKind::PasswordSet, by, "", now))?;
            }
            Ok(None)
        })
    }

    /// Set an account's password: **all its sessions end** (in the same transaction) and nothing else changes. `password_set` when the
    /// operator or the provider did it, `password_changed` when the holder did.
    pub fn set_password(
        &self,
        handle: &str,
        password: &NormalizedPassword,
        by: EventBy,
        now: u64,
    ) -> Result<(), AccountError> {
        let hash = self.hasher.hash(password.as_str()).0;
        let at = i64::try_from(now).unwrap_or(i64::MAX);
        let kind = if by == EventBy::SelfService {
            AccountEventKind::PasswordChanged
        } else {
            AccountEventKind::PasswordSet
        };
        self.txn(|conn| {
            let changed = conn.execute(
                "UPDATE accounts SET password_hash = ?2, password_changed_at = ?3 WHERE user = ?1",
                params![handle, hash, at],
            )?;
            if changed == 0 {
                return Ok(Some(AccountError::NoSuchAccount));
            }
            let ended = conn.execute("DELETE FROM sessions WHERE user = ?1", params![handle])?;
            insert_event(
                conn,
                &AccountEvent::new(handle, kind, by, &format!("{ended} session(s) ended"), now),
            )?;
            Ok(None)
        })
    }

    /// Link an identity `(issuer, subject)` to an account. `Ok(true)` when the link was made, `Ok(false)` when it already was this
    /// account's (nothing changes, no event). An identity linked to **another** account is refused (`IdentityTaken`): it is never moved
    /// silently. The event names the issuer's **host** only.
    pub fn link_oidc(
        &self,
        handle: &str,
        issuer: &str,
        subject: &str,
        by: EventBy,
        now: u64,
    ) -> Result<bool, AccountError> {
        let made = std::cell::Cell::new(false);
        self.txn(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM accounts WHERE user = ?1",
                params![handle],
                |r| r.get(0),
            )?;
            if exists == 0 {
                return Ok(Some(AccountError::NoSuchAccount));
            }
            let linked: Option<String> = conn
                .query_row(
                    "SELECT user FROM oidc_links WHERE issuer = ?1 AND subject = ?2",
                    params![issuer, subject],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            match linked {
                Some(owner) if owner == handle => return Ok(None),
                Some(_) => return Ok(Some(AccountError::IdentityTaken)),
                None => {}
            }
            conn.execute(
                "INSERT INTO oidc_links (issuer, subject, user) VALUES (?1, ?2, ?3)",
                params![issuer, subject, handle],
            )?;
            insert_event(
                conn,
                &AccountEvent::new(
                    handle,
                    AccountEventKind::IdentityLinked,
                    by,
                    &issuer_host(issuer),
                    now,
                ),
            )?;
            made.set(true);
            Ok(None)
        })?;
        Ok(made.get())
    }

    /// Remove the link of `(issuer, subject)` **from this account** (`NotLinked` when it is not this account's).
    pub fn unlink_oidc(
        &self,
        handle: &str,
        issuer: &str,
        subject: &str,
        by: EventBy,
        now: u64,
    ) -> Result<(), AccountError> {
        self.txn(|conn| {
            let removed = conn.execute(
                "DELETE FROM oidc_links WHERE issuer = ?1 AND subject = ?2 AND user = ?3",
                params![issuer, subject, handle],
            )?;
            if removed == 0 {
                return Ok(Some(AccountError::NotLinked));
            }
            insert_event(
                conn,
                &AccountEvent::new(
                    handle,
                    AccountEventKind::IdentityUnlinked,
                    by,
                    &issuer_host(issuer),
                    now,
                ),
            )?;
            Ok(None)
        })
    }

    /// Disable an account (RFC 014 D-6): its sessions end in this transaction and the reason is the event's detail. `Ok(false)` when it was
    /// disabled already (nothing changes, no event).
    pub fn disable(
        &self,
        handle: &str,
        reason: &str,
        by: EventBy,
        now: u64,
    ) -> Result<bool, AccountError> {
        let done = std::cell::Cell::new(false);
        let at = i64::try_from(now).unwrap_or(i64::MAX);
        let reason = escape_display(reason, 200);
        self.txn(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM accounts WHERE user = ?1",
                params![handle],
                |r| r.get(0),
            )?;
            if exists == 0 {
                return Ok(Some(AccountError::NoSuchAccount));
            }
            let changed = conn.execute(
                "UPDATE accounts SET disabled_at = ?2, disabled_reason = ?3 WHERE user = ?1 AND disabled_at IS NULL",
                params![handle, at, reason],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            conn.execute("DELETE FROM sessions WHERE user = ?1", params![handle])?;
            insert_event(
                conn,
                &AccountEvent::new(handle, AccountEventKind::Disabled, by, &reason, now),
            )?;
            done.set(true);
            Ok(None)
        })?;
        Ok(done.get())
    }

    /// Enable a disabled account. `Ok(false)` when it was not disabled (nothing changes, no event).
    pub fn enable(&self, handle: &str, by: EventBy, now: u64) -> Result<bool, AccountError> {
        let done = std::cell::Cell::new(false);
        self.txn(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM accounts WHERE user = ?1",
                params![handle],
                |r| r.get(0),
            )?;
            if exists == 0 {
                return Ok(Some(AccountError::NoSuchAccount));
            }
            let changed = conn.execute(
                "UPDATE accounts SET disabled_at = NULL, disabled_reason = NULL WHERE user = ?1 AND disabled_at IS NOT NULL",
                params![handle],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            insert_event(
                conn,
                &AccountEvent::new(handle, AccountEventKind::Enabled, by, "", now),
            )?;
            done.set(true);
            Ok(None)
        })?;
        Ok(done.get())
    }
}
