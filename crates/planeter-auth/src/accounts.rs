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

use std::collections::BTreeSet;
use std::sync::Arc;

use rusqlite::params;

use planeter_core::creation::{HandleError, check_new_handle, handle_key};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::SqliteDb;
use planeter_store::{RepoId, UserId};

use crate::credential::{NewToken, TokenError, check_new_token};
use crate::events::{AccountEvent, AccountEventKind, EventBy, insert_event};
use crate::hashing::PasswordHasher;
use crate::hashing::TokenHash;
use crate::identity::AuthError;
use crate::password::NormalizedPassword;
use crate::sqlite::{handle_taken, immediate, insert_new_token, prepare};
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
    /// No (ready) repository has this owner and name.
    NoSuchRepository,
    /// The account is disabled, so it is not given a role.
    AccountDisabled,
    /// The grant to remove is the last administrator's: the repository would have none (RFC 014 D-3).
    LastAdministrator,
    /// The account has no role of its own on the repository to take away.
    NoGrant,
    /// A rule of a new token (the name, the expiry, the bound of 200) refused it.
    Token(TokenError),
    /// The account has no token with this id.
    NoSuchToken,
    /// A repository a token was to be limited to does not exist (`owner/name`).
    RepositoryUnknown(String),
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
            AccountError::NoSuchRepository => "no_such_repository",
            AccountError::AccountDisabled => "account_disabled",
            AccountError::LastAdministrator => "last_administrator",
            AccountError::NoGrant => "no_grant",
            AccountError::Token(_) => "token",
            AccountError::NoSuchToken => "no_such_token",
            AccountError::RepositoryUnknown(_) => "repository_unknown",
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

// ---------------------------------------------------------------------------------------------------- roles on a repository

/// A role on a repository, as `repo_grants` keeps it (RFC 002 D-4). Ordered: each includes what the one before it allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepoRole {
    Read,
    Write,
    Maintain,
    Admin,
}

impl RepoRole {
    pub const ALL: [RepoRole; 4] = [
        RepoRole::Read,
        RepoRole::Write,
        RepoRole::Maintain,
        RepoRole::Admin,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RepoRole::Read => "read",
            RepoRole::Write => "write",
            RepoRole::Maintain => "maintain",
            RepoRole::Admin => "admin",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == word)
    }
}

/// What a grant did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Granted {
    /// The display name of the account the role went to: shown back, so that a mistyped handle is seen at once.
    pub display_name: String,
    /// The role the account had of its own on the repository before (`Some(role)` equal to the one asked: nothing changed, no event).
    pub previous: Option<RepoRole>,
}

/// What a revoke did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revoked {
    pub display_name: String,
    pub was: RepoRole,
}

/// The repositories for which an account is the only administrator, bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoleAdministrator {
    /// `owner/name`, sorted, at most the limit asked.
    pub repos: Vec<String>,
    /// How many there are in all.
    pub total: usize,
}

/// One line of the account list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub handle: String,
    pub display_name: String,
    pub disabled: bool,
    pub has_password: bool,
    pub identities: usize,
    pub created_at: Option<u64>,
}

/// The accounts sorted by handle, bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountList {
    pub rows: Vec<AccountRow>,
    pub total: usize,
}

/// One account in full (never a hash, a token or a subject).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDetail {
    pub row: AccountRow,
    /// When and why it was disabled.
    pub disabled: Option<(u64, String)>,
    pub password_changed_at: Option<u64>,
    /// `(owner/name, role)`: the account's own grants and the repositories it owns (role `owner`), at most the limit asked.
    pub roles: Vec<(String, String)>,
    pub roles_total: usize,
}

fn count(
    conn: &rusqlite::Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> rusqlite::Result<usize> {
    conn.query_row(sql, p, |r| r.get::<_, i64>(0))
        .map(|n| usize::try_from(n).unwrap_or(0))
}

/// The ready repository with this owner and name: `(repo_id, owner_kind, owner)`. A user owner is looked for first (as every other
/// resolution in the project does).
fn resolve_repo(
    conn: &rusqlite::Connection,
    owner: &str,
    name: &str,
) -> rusqlite::Result<Option<(String, String, String)>> {
    for kind in ["user", "org"] {
        let found = conn.query_row(
            "SELECT repo_id, owner_kind, owner FROM repositories WHERE owner_kind = ?1 AND owner = ?2 AND name = ?3 AND state = 'ready'",
            params![kind, owner, name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match found {
            Ok(v) => return Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// The handles that can administer a repository **and are not disabled**: its user owner, the admins of its owning organization, the users
/// with an admin grant of their own and the members of a team that has one (the same sources as the project's role resolution).
/// `without_direct_grant` leaves one user's own admin grant out (what a revoke would leave).
fn administrators(
    conn: &rusqlite::Connection,
    repo_id: &str,
    owner_kind: &str,
    owner: &str,
    without_direct_grant: Option<&str>,
) -> rusqlite::Result<BTreeSet<String>> {
    fn column(
        conn: &rusqlite::Connection,
        sql: &str,
        p: &[&dyn rusqlite::ToSql],
    ) -> rusqlite::Result<Vec<String>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(p, |r| r.get::<_, String>(0))?;
        rows.collect()
    }
    let mut all: BTreeSet<String> = BTreeSet::new();
    if owner_kind == "user" {
        all.insert(owner.to_owned());
    }
    // The users' own admin grants (leaving out the one a revoke would take away: the user stays an administrator by any other source).
    let skip = without_direct_grant.unwrap_or("");
    all.extend(column(
        conn,
        "SELECT subject FROM repo_grants WHERE repo_id = ?1 AND subject_kind = 'user' AND role = 'admin' AND subject != ?2",
        &[&repo_id, &skip],
    )?);
    if owner_kind == "org" {
        all.extend(column(
            conn,
            "SELECT user FROM org_roles WHERE org = ?1 AND role = 'admin'",
            &[&owner],
        )?);
        all.extend(column(
            conn,
            "SELECT m.user FROM repo_grants g JOIN teams t ON t.team = g.subject JOIN team_members m ON m.team = t.team \
             WHERE g.repo_id = ?1 AND g.subject_kind = 'team' AND g.role = 'admin' AND t.org = ?2",
            &[&repo_id, &owner],
        )?);
    }
    // A disabled account administers nothing: it cannot act.
    let gone: BTreeSet<String> = column(
        conn,
        "SELECT user FROM accounts WHERE disabled_at IS NOT NULL",
        &[],
    )?
    .into_iter()
    .collect();
    Ok(all.difference(&gone).cloned().collect())
}

impl AccountService {
    /// Give an account `role` on a repository, **exactly** that role (a higher role it had is lowered, a lower one raised): refused for a
    /// repository that does not exist, an account that does not exist and an account that is **disabled**. The event (`role_granted`,
    /// with the repository and the role) is the person's own record of it. A grant the account already has changes nothing and writes
    /// nothing (`previous` equals the role asked).
    pub fn grant_role(
        &self,
        handle: &str,
        owner: &str,
        name: &str,
        role: RepoRole,
        by: EventBy,
        now: u64,
    ) -> Result<Granted, AccountError> {
        let granted = std::cell::RefCell::new(None);
        self.txn(|conn| {
            let Some((repo_id, _, shown_owner)) = resolve_repo(conn, owner, name)? else {
                return Ok(Some(AccountError::NoSuchRepository));
            };
            let found: Option<(String, Option<i64>)> = conn
                .query_row(
                    "SELECT display_name, disabled_at FROM accounts WHERE user = ?1",
                    params![handle],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some((display_name, disabled_at)) = found else {
                return Ok(Some(AccountError::NoSuchAccount));
            };
            if disabled_at.is_some() {
                return Ok(Some(AccountError::AccountDisabled));
            }
            let previous = conn
                .query_row(
                    "SELECT role FROM repo_grants WHERE subject_kind = 'user' AND subject = ?1 AND repo_id = ?2",
                    params![handle, repo_id],
                    |r| r.get::<_, String>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?
                .and_then(|w| RepoRole::parse(&w));
            if previous != Some(role) {
                conn.execute(
                    "INSERT INTO repo_grants (subject_kind, subject, repo_id, role) VALUES ('user', ?1, ?2, ?3) \
                     ON CONFLICT(subject_kind, subject, repo_id) DO UPDATE SET role = excluded.role",
                    params![handle, repo_id, role.as_str()],
                )?;
                insert_event(
                    conn,
                    &AccountEvent::new(
                        handle,
                        AccountEventKind::RoleGranted,
                        by,
                        &format!("{shown_owner}/{name} {}", role.as_str()),
                        now,
                    ),
                )?;
            }
            *granted.borrow_mut() = Some(Granted {
                display_name,
                previous,
            });
            Ok(None)
        })?;
        granted
            .into_inner()
            .ok_or_else(|| AccountError::Backend("no result".to_owned()))
    }

    /// Take away the role an account has **of its own** on a repository. An administrator grant that is the **last** administrator of the
    /// repository (counting its owner, its organization's administrators and team grants, and leaving out disabled accounts) is refused
    /// unless `allow_no_admin`. A repository's owner stays its administrator whatever grants are taken away.
    pub fn revoke_role(
        &self,
        handle: &str,
        owner: &str,
        name: &str,
        allow_no_admin: bool,
        by: EventBy,
        now: u64,
    ) -> Result<Revoked, AccountError> {
        let revoked = std::cell::RefCell::new(None);
        self.txn(|conn| {
            let Some((repo_id, owner_kind, shown_owner)) = resolve_repo(conn, owner, name)? else {
                return Ok(Some(AccountError::NoSuchRepository));
            };
            let display_name: Option<String> = conn
                .query_row(
                    "SELECT display_name FROM accounts WHERE user = ?1",
                    params![handle],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some(display_name) = display_name else {
                return Ok(Some(AccountError::NoSuchAccount));
            };
            let was = conn
                .query_row(
                    "SELECT role FROM repo_grants WHERE subject_kind = 'user' AND subject = ?1 AND repo_id = ?2",
                    params![handle, repo_id],
                    |r| r.get::<_, String>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?
                .and_then(|w| RepoRole::parse(&w));
            let Some(was) = was else {
                return Ok(Some(AccountError::NoGrant));
            };
            if was == RepoRole::Admin && !allow_no_admin {
                let left = administrators(conn, &repo_id, &owner_kind, &shown_owner, Some(handle))?;
                if left.is_empty() {
                    return Ok(Some(AccountError::LastAdministrator));
                }
            }
            conn.execute(
                "DELETE FROM repo_grants WHERE subject_kind = 'user' AND subject = ?1 AND repo_id = ?2",
                params![handle, repo_id],
            )?;
            insert_event(
                conn,
                &AccountEvent::new(
                    handle,
                    AccountEventKind::RoleRevoked,
                    by,
                    &format!("{shown_owner}/{name} {}", was.as_str()),
                    now,
                ),
            )?;
            *revoked.borrow_mut() = Some(Revoked { display_name, was });
            Ok(None)
        })?;
        revoked
            .into_inner()
            .ok_or_else(|| AccountError::Backend("no result".to_owned()))
    }

    /// The repositories for which `handle` is the **only** administrator (counting the other sources of the role, and leaving out disabled
    /// accounts), `owner/name` sorted, at most `limit` of them and the total.
    pub fn sole_administrator_of(
        &self,
        handle: &str,
        limit: usize,
    ) -> Result<SoleAdministrator, AccountError> {
        let conn = self
            .db
            .lock()
            .map_err(|e| AccountError::Backend(e.to_string()))?;
        let be = |e: rusqlite::Error| AccountError::Backend(e.to_string());
        let mut stmt = conn
            .prepare(
                "SELECT repo_id FROM repositories WHERE owner_kind = 'user' AND owner = ?1 AND state = 'ready' \
                 UNION SELECT g.repo_id FROM repo_grants g JOIN repositories r ON r.repo_id = g.repo_id AND r.state = 'ready' \
                       WHERE g.subject_kind = 'user' AND g.subject = ?1 AND g.role = 'admin' \
                 UNION SELECT r.repo_id FROM repositories r JOIN org_roles o ON o.org = r.owner AND r.owner_kind = 'org' \
                       WHERE o.user = ?1 AND o.role = 'admin' AND r.state = 'ready' \
                 UNION SELECT g.repo_id FROM repo_grants g JOIN teams t ON t.team = g.subject JOIN team_members m ON m.team = t.team \
                       JOIN repositories r ON r.repo_id = g.repo_id AND r.owner_kind = 'org' AND r.owner = t.org AND r.state = 'ready' \
                       WHERE g.subject_kind = 'team' AND g.role = 'admin' AND m.user = ?1",
            )
            .map_err(be)?;
        let candidates: Vec<String> = stmt
            .query_map(params![handle], |r| r.get(0))
            .map_err(be)?
            .collect::<Result<_, _>>()
            .map_err(be)?;
        drop(stmt);
        let mut sole: Vec<String> = Vec::new();
        for repo_id in candidates {
            let (kind, owner, name): (String, String, String) = conn
                .query_row(
                    "SELECT owner_kind, owner, name FROM repositories WHERE repo_id = ?1",
                    params![repo_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(be)?;
            let admins = administrators(&conn, &repo_id, &kind, &owner, None).map_err(be)?;
            if admins.len() == 1 && admins.contains(handle) {
                sole.push(format!("{owner}/{name}"));
            }
        }
        sole.sort();
        let total = sole.len();
        sole.truncate(limit);
        Ok(SoleAdministrator { repos: sole, total })
    }

    /// Whether the repository has an administrator who can act now (`None` when there is no such repository).
    pub fn repo_has_administrator(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<bool>, AccountError> {
        let conn = self
            .db
            .lock()
            .map_err(|e| AccountError::Backend(e.to_string()))?;
        let be = |e: rusqlite::Error| AccountError::Backend(e.to_string());
        let Some((repo_id, kind, shown_owner)) = resolve_repo(&conn, owner, name).map_err(be)?
        else {
            return Ok(None);
        };
        Ok(Some(
            !administrators(&conn, &repo_id, &kind, &shown_owner, None)
                .map_err(be)?
                .is_empty(),
        ))
    }

    /// The accounts sorted by handle, at most `limit` of them, and how many there are.
    pub fn list_accounts(&self, limit: usize) -> Result<AccountList, AccountError> {
        let conn = self
            .db
            .lock()
            .map_err(|e| AccountError::Backend(e.to_string()))?;
        let be = |e: rusqlite::Error| AccountError::Backend(e.to_string());
        let total = count(&conn, "SELECT COUNT(*) FROM accounts", []).map_err(be)?;
        let mut stmt = conn
            .prepare(
                "SELECT user, display_name, disabled_at IS NOT NULL, password_hash IS NOT NULL, \
                        (SELECT COUNT(*) FROM oidc_links l WHERE l.user = accounts.user), created_at \
                 FROM accounts ORDER BY user LIMIT ?1",
            )
            .map_err(be)?;
        let rows = stmt
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], row_of)
            .map_err(be)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(be)?;
        Ok(AccountList { rows, total })
    }

    /// The handles that differ from another's only by case, grouped by key (`handle_collides = 1`), sorted.
    pub fn collisions(&self) -> Result<Vec<(String, Vec<String>)>, AccountError> {
        let conn = self
            .db
            .lock()
            .map_err(|e| AccountError::Backend(e.to_string()))?;
        let be = |e: rusqlite::Error| AccountError::Backend(e.to_string());
        let mut stmt = conn
            .prepare("SELECT handle_key, user FROM accounts WHERE handle_collides = 1 ORDER BY handle_key, user")
            .map_err(be)?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(be)?
            .collect::<Result<_, _>>()
            .map_err(be)?;
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for (key, user) in rows {
            match groups.last_mut() {
                Some((k, v)) if *k == key => v.push(user),
                _ => groups.push((key, vec![user])),
            }
        }
        Ok(groups)
    }

    /// One account in full, or `None`. `roles_limit` bounds the roles listed.
    pub fn show_account(
        &self,
        handle: &str,
        roles_limit: usize,
    ) -> Result<Option<AccountDetail>, AccountError> {
        let conn = self
            .db
            .lock()
            .map_err(|e| AccountError::Backend(e.to_string()))?;
        let be = |e: rusqlite::Error| AccountError::Backend(e.to_string());
        let found = conn.query_row(
            "SELECT user, display_name, disabled_at IS NOT NULL, password_hash IS NOT NULL, \
                    (SELECT COUNT(*) FROM oidc_links l WHERE l.user = accounts.user), created_at, \
                    disabled_at, disabled_reason, password_changed_at FROM accounts WHERE user = ?1",
            params![handle],
            |r| Ok((row_of(r)?, r.get::<_, Option<i64>>(6)?, r.get::<_, Option<String>>(7)?, r.get::<_, Option<i64>>(8)?)),
        );
        let (row, disabled_at, reason, changed) = match found {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(be(e)),
        };
        let secs = |v: Option<i64>| v.map(|n| u64::try_from(n).unwrap_or(0));
        let mut stmt = conn
            .prepare(
                "SELECT owner || '/' || name, 'owner' FROM repositories WHERE owner_kind = 'user' AND owner = ?1 AND state = 'ready' \
                 UNION ALL SELECT r.owner || '/' || r.name, g.role FROM repo_grants g JOIN repositories r ON r.repo_id = g.repo_id \
                       WHERE g.subject_kind = 'user' AND g.subject = ?1 AND r.state = 'ready' \
                 ORDER BY 1, 2",
            )
            .map_err(be)?;
        let all: Vec<(String, String)> = stmt
            .query_map(params![handle], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(be)?
            .collect::<Result<_, _>>()
            .map_err(be)?;
        let roles_total = all.len();
        Ok(Some(AccountDetail {
            row,
            disabled: secs(disabled_at).map(|at| (at, reason.unwrap_or_default())),
            password_changed_at: secs(changed),
            roles: all.into_iter().take(roles_limit).collect(),
            roles_total,
        }))
    }
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<AccountRow> {
    Ok(AccountRow {
        handle: r.get(0)?,
        display_name: r.get(1)?,
        disabled: r.get(2)?,
        has_password: r.get(3)?,
        identities: usize::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
        created_at: r
            .get::<_, Option<i64>>(5)?
            .map(|n| u64::try_from(n).unwrap_or(0)),
    })
}

/// A token to make for an account (the operator's `token create`). The **secret is not here**: the caller minted it and gives only its
/// hash, so this layer never holds a token it could leak.
pub struct TokenSpec<'a> {
    pub handle: &'a str,
    pub hash: TokenHash,
    pub name: &'a str,
    pub access: ScopeAccess,
    /// The repositories the token is limited to, as `(owner, name)`; empty means every repository the account can reach.
    pub repos: &'a [(String, String)],
    /// Unix seconds.
    pub expires_at: u64,
}

/// What a token creation made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MadeToken {
    /// The public identifier (not the secret).
    pub id: String,
    pub display_name: String,
    /// The repositories the token is limited to, as `owner/name` (empty: all).
    pub repos: Vec<String>,
}

fn scope_word(a: ScopeAccess) -> &'static str {
    match a {
        ScopeAccess::ReadOnly => "read-only",
        ScopeAccess::Write => "write",
        ScopeAccess::Full => "full",
    }
}

impl AccountService {
    /// Make a token for an account, **in one transaction with its `token_created` event**: the account exists and is not disabled, every
    /// named repository exists, the rules of a new token (a name of 1 to 60 safe characters; an expiry in the future and within 365 days;
    /// at most 200 unexpired tokens) hold. A refusal changes nothing. The event's detail is the token's **name, scope and expiry date
    /// only**: never the secret, its hash or its id.
    pub fn create_token_for(
        &self,
        spec: TokenSpec<'_>,
        by: EventBy,
        now: u64,
    ) -> Result<MadeToken, AccountError> {
        let handle = spec.handle;
        // The rules that need no store first (the same function the store applies).
        let probe = NewToken {
            hash: spec.hash.clone(),
            user: UserId::new(handle),
            name: spec.name.to_owned(),
            scope: Scope {
                access: spec.access,
                repos: RepoScope::All,
                expires_at: Some(spec.expires_at),
            },
        };
        check_new_token(&probe, now).map_err(AccountError::Token)?;
        let made = std::cell::RefCell::new(None);
        self.txn(|conn| {
            let found: Option<(String, Option<i64>)> = conn
                .query_row(
                    "SELECT display_name, disabled_at FROM accounts WHERE user = ?1",
                    params![handle],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some((display_name, disabled_at)) = found else {
                return Ok(Some(AccountError::NoSuchAccount));
            };
            if disabled_at.is_some() {
                return Ok(Some(AccountError::AccountDisabled));
            }
            let mut ids: Vec<RepoId> = Vec::new();
            let mut shown: Vec<String> = Vec::new();
            for (owner, name) in spec.repos {
                match resolve_repo(conn, owner, name)? {
                    Some((id, _, shown_owner)) => {
                        ids.push(RepoId::new(id));
                        shown.push(format!("{shown_owner}/{name}"));
                    }
                    None => {
                        return Ok(Some(AccountError::RepositoryUnknown(format!(
                            "{owner}/{name}"
                        ))));
                    }
                }
            }
            let repo_scope = if ids.is_empty() {
                RepoScope::All
            } else {
                RepoScope::Only(ids)
            };
            let new = NewToken {
                hash: spec.hash.clone(),
                user: UserId::new(handle),
                name: spec.name.to_owned(),
                scope: Scope {
                    access: spec.access,
                    repos: repo_scope,
                    expires_at: Some(spec.expires_at),
                },
            };
            let id = match insert_new_token(conn, &new, now)? {
                Ok(id) => id,
                Err(TokenError::Backend(why)) => return Ok(Some(AccountError::Backend(why))),
                Err(e) => return Ok(Some(AccountError::Token(e))),
            };
            let reach = if shown.is_empty() {
                String::new()
            } else {
                format!(", {} repositories", shown.len())
            };
            insert_event(
                conn,
                &AccountEvent::new(
                    handle,
                    AccountEventKind::TokenCreated,
                    by,
                    &format!(
                        "{}; {}{reach}; until {}",
                        spec.name,
                        scope_word(spec.access),
                        crate::text::ymd(spec.expires_at)
                    ),
                    now,
                ),
            )?;
            *made.borrow_mut() = Some(MadeToken {
                id,
                display_name,
                repos: shown,
            });
            Ok(None)
        })?;
        made.into_inner()
            .ok_or_else(|| AccountError::Backend("no result".to_owned()))
    }

    /// Revoke the token with this public `id` **that belongs to `handle`**, in one transaction with the `token_revoked` event (the token's
    /// **name** only). Another account's id is `NoSuchToken` and nothing is written. Returns the token's name.
    pub fn revoke_token_of(
        &self,
        handle: &str,
        id: &str,
        by: EventBy,
        now: u64,
    ) -> Result<String, AccountError> {
        let name = std::cell::RefCell::new(None);
        self.txn(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM accounts WHERE user = ?1",
                params![handle],
                |r| r.get(0),
            )?;
            if exists == 0 {
                return Ok(Some(AccountError::NoSuchAccount));
            }
            let found: Option<String> = conn
                .query_row(
                    "SELECT COALESCE(name, '') FROM tokens WHERE id = ?1 AND user = ?2",
                    params![id, handle],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some(token_name) = found else {
                return Ok(Some(AccountError::NoSuchToken));
            };
            conn.execute(
                "DELETE FROM tokens WHERE id = ?1 AND user = ?2",
                params![id, handle],
            )?;
            insert_event(
                conn,
                &AccountEvent::new(handle, AccountEventKind::TokenRevoked, by, &token_name, now),
            )?;
            *name.borrow_mut() = Some(token_name);
            Ok(None)
        })?;
        name.into_inner()
            .ok_or_else(|| AccountError::Backend("no result".to_owned()))
    }
}
