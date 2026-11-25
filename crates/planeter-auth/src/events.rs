//! **What happened to an account** (RFC 014 D-7): one row per act, written in the **same transaction** as the act, kept for good (the
//! audit trail of the account), readable by the account's holder and by the operator.
//!
//! **Never a secret.** A row has the account's handle, a kind from a closed set, who did it (`self`, `operator` or `provider`), a short
//! display-safe detail (a reason, a token's *name*, never the token; an issuer's host, never a subject), and the time. No password, no hash,
//! no token, no address, no session id: a test writes canaries through every act and finds none in this table or in a log.
//!
//! **A detail is data, never prose.** Stored history is read for years in whatever language the reader has, so **nothing the system
//! generates is written as a sentence**: an event's `detail` is either free text a person typed (a *reason*, a token's *name*), made
//! display-safe, or **language-neutral fields** in the fixed order below, separated by a tab (a character no display-safe text can hold:
//! [`crate::text::escape_display`] shows it as a visible escape). Every kind has one typed form, [`EventDetail`], with a constructor
//! ([`AccountEvent::typed`]) and a parser ([`AccountEvent::detail_data`]): nobody builds or reads these strings by hand. The catalog layer
//! renders them in the reader's language (`planeter_web::account_events::render_event`).
//!
//! | kind | stored `detail` (fields in this order) |
//! |---|---|
//! | `created`, `enabled`, `signed_in_new_method` | empty (`signed_in_new_method`'s data is defined with the step that writes it) |
//! | `password_set`, `password_changed`, `session_ended` | the number of sign-ins that ended, digits only |
//! | `identity_linked`, `identity_unlinked` | the issuer's host |
//! | `disabled` | the reason, as typed (display-safe) |
//! | `token_created` | name (as typed) `TAB` scope code `read-only`\|`write`\|`full` `TAB` number of repositories the token is limited to (`0` = all) `TAB` end date `YYYY-MM-DD` |
//! | `token_revoked` | name (as typed) |
//! | `role_granted`, `role_revoked` | `owner/name` `TAB` role code `read`\|`write`\|`maintain`\|`admin` |
//!
//! A detail that does not parse (an older or damaged row) is **shown as nothing**, never as the raw text, and breaks nothing.
//!
//! **There is no delete and no update**: the trait has none, the SQL has none (a test scans the source), and a row says "this happened",
//! never "this was undone" (an undoing is a new row).

use std::sync::Mutex;

use rusqlite::params;

use planeter_store::SqliteDb;

use planeter_core::ScopeAccess;

use crate::accounts::RepoRole;
use crate::identity::{AuthError, Result};
use crate::text::escape_display;

/// The longest `detail` a row keeps, in characters (after escaping). A longer text is cut, never refused.
pub const EVENT_DETAIL_MAX_CHARS: usize = 200;
/// The most rows `recent` returns, whatever is asked.
pub const RECENT_MAX: usize = 200;

/// What happened. A closed set: the table refuses any other word (a `CHECK`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountEventKind {
    Created,
    PasswordSet,
    PasswordChanged,
    IdentityLinked,
    IdentityUnlinked,
    TokenCreated,
    TokenRevoked,
    SessionEnded,
    RoleGranted,
    RoleRevoked,
    Disabled,
    Enabled,
    SignedInNewMethod,
}

impl AccountEventKind {
    /// Every kind, in the order of the table's `CHECK`.
    pub const ALL: [AccountEventKind; 13] = [
        AccountEventKind::Created,
        AccountEventKind::PasswordSet,
        AccountEventKind::PasswordChanged,
        AccountEventKind::IdentityLinked,
        AccountEventKind::IdentityUnlinked,
        AccountEventKind::TokenCreated,
        AccountEventKind::TokenRevoked,
        AccountEventKind::SessionEnded,
        AccountEventKind::RoleGranted,
        AccountEventKind::RoleRevoked,
        AccountEventKind::Disabled,
        AccountEventKind::Enabled,
        AccountEventKind::SignedInNewMethod,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AccountEventKind::Created => "created",
            AccountEventKind::PasswordSet => "password_set",
            AccountEventKind::PasswordChanged => "password_changed",
            AccountEventKind::IdentityLinked => "identity_linked",
            AccountEventKind::IdentityUnlinked => "identity_unlinked",
            AccountEventKind::TokenCreated => "token_created",
            AccountEventKind::TokenRevoked => "token_revoked",
            AccountEventKind::SessionEnded => "session_ended",
            AccountEventKind::RoleGranted => "role_granted",
            AccountEventKind::RoleRevoked => "role_revoked",
            AccountEventKind::Disabled => "disabled",
            AccountEventKind::Enabled => "enabled",
            AccountEventKind::SignedInNewMethod => "signed_in_new_method",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Who did it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventBy {
    /// The account's own holder.
    SelfService,
    /// The server's operator (the `planeter operator` commands).
    Operator,
    /// The identity provider (a sign-in it verified).
    Provider,
}

impl EventBy {
    pub const ALL: [EventBy; 3] = [EventBy::SelfService, EventBy::Operator, EventBy::Provider];

    pub fn as_str(self) -> &'static str {
        match self {
            EventBy::SelfService => "self",
            EventBy::Operator => "operator",
            EventBy::Provider => "provider",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.as_str() == s)
    }
}

/// The data of an event's `detail`, one form for each shape in the table of this module's documentation. **Language-neutral**: codes, numbers,
/// a host, a date, and the two kinds of text a person typed (a reason, a name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventDetail {
    /// No detail (`created`, `enabled`, `signed_in_new_method`).
    None,
    /// How many sign-ins ended (`password_set`, `password_changed`, `session_ended`).
    SignInsEnded(u32),
    /// The issuer's host (`identity_linked`, `identity_unlinked`).
    Host(String),
    /// The reason the operator typed (`disabled`).
    Reason(String),
    /// `token_created`: the name as typed, the scope, how many repositories it is limited to (`0` = all), the end date `YYYY-MM-DD`.
    TokenCreated {
        name: String,
        access: ScopeAccess,
        repositories: u32,
        until: String,
    },
    /// `token_revoked`: the name as typed.
    TokenRevoked { name: String },
    /// `role_granted`, `role_revoked`: `owner/name` and the role.
    Role { repository: String, role: RepoRole },
}

/// The stable code of a token's scope in a stored detail.
pub fn scope_code(a: ScopeAccess) -> &'static str {
    match a {
        ScopeAccess::ReadOnly => "read-only",
        ScopeAccess::Write => "write",
        ScopeAccess::Full => "full",
    }
}

fn parse_scope_code(s: &str) -> Option<ScopeAccess> {
    [ScopeAccess::ReadOnly, ScopeAccess::Write, ScopeAccess::Full]
        .into_iter()
        .find(|a| scope_code(*a) == s)
}

fn digits(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

impl EventDetail {
    /// The stored text: each field display-safe, the fields joined by a tab.
    pub fn to_stored(&self) -> String {
        let f = |s: &str| escape_display(s, EVENT_DETAIL_MAX_CHARS);
        match self {
            EventDetail::None => String::new(),
            EventDetail::SignInsEnded(n) => n.to_string(),
            EventDetail::Host(h) => f(h),
            EventDetail::Reason(r) => f(r),
            EventDetail::TokenCreated {
                name,
                access,
                repositories,
                until,
            } => format!(
                "{}\t{}\t{}\t{}",
                f(name),
                scope_code(*access),
                repositories,
                f(until)
            ),
            EventDetail::TokenRevoked { name } => f(name),
            EventDetail::Role { repository, role } => {
                format!("{}\t{}", f(repository), role.as_str())
            }
        }
    }

    /// The typed form of the stored text of a `kind`, or `None` when the text is not the shape of that kind (**never a guess, never the raw
    /// text**).
    pub fn parse(kind: AccountEventKind, text: &str) -> Option<EventDetail> {
        use AccountEventKind as K;
        let fields: Vec<&str> = text.split('\t').collect();
        match kind {
            K::Created | K::Enabled | K::SignedInNewMethod => {
                text.is_empty().then_some(EventDetail::None)
            }
            K::PasswordSet | K::PasswordChanged | K::SessionEnded => {
                digits(text).map(EventDetail::SignInsEnded)
            }
            K::IdentityLinked | K::IdentityUnlinked => {
                (fields.len() == 1 && !text.is_empty()).then(|| EventDetail::Host(text.to_owned()))
            }
            K::Disabled => (fields.len() == 1).then(|| EventDetail::Reason(text.to_owned())),
            K::TokenCreated => match fields.as_slice() {
                [name, scope, repos, until] if !name.is_empty() && is_date(until) => {
                    Some(EventDetail::TokenCreated {
                        name: (*name).to_owned(),
                        access: parse_scope_code(scope)?,
                        repositories: digits(repos)?,
                        until: (*until).to_owned(),
                    })
                }
                _ => None,
            },
            K::TokenRevoked => {
                (fields.len() == 1 && !text.is_empty()).then(|| EventDetail::TokenRevoked {
                    name: text.to_owned(),
                })
            }
            K::RoleGranted | K::RoleRevoked => match fields.as_slice() {
                [repository, role] if repository.contains('/') => Some(EventDetail::Role {
                    repository: (*repository).to_owned(),
                    role: RepoRole::parse(role)?,
                }),
                _ => None,
            },
        }
    }
}

/// What a stored detail may hold: every tab-separated field display-safe, the whole cut at [`EVENT_DETAIL_MAX_CHARS`]. Idempotent.
pub(crate) fn sanitize_detail(text: &str) -> String {
    text.split('\t')
        .map(|f| escape_display(f, EVENT_DETAIL_MAX_CHARS))
        .collect::<Vec<_>>()
        .join("\t")
        .chars()
        .take(EVENT_DETAIL_MAX_CHARS)
        .collect()
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvent {
    /// The account's handle.
    pub account: String,
    pub kind: AccountEventKind,
    pub by: EventBy,
    /// Display-safe, at most [`EVENT_DETAIL_MAX_CHARS`] characters (see [`AccountEvent::new`]).
    pub detail: String,
    /// Unix seconds.
    pub at: u64,
}

impl AccountEvent {
    /// An event with its `detail` made display-safe and cut at a character boundary (**cut, never refused**; a disguise character is
    /// shown as a visible escape).
    pub fn new(account: &str, kind: AccountEventKind, by: EventBy, detail: &str, at: u64) -> Self {
        Self {
            account: account.to_owned(),
            kind,
            by,
            detail: escape_display(detail, EVENT_DETAIL_MAX_CHARS),
            at,
        }
    }
}

impl AccountEvent {
    /// An event whose detail is **data**: the typed form of `kind`, encoded (see [`EventDetail`]). Use this for everything the system
    /// generates; [`AccountEvent::new`] is for text a person typed.
    pub fn typed(
        account: &str,
        kind: AccountEventKind,
        by: EventBy,
        detail: &EventDetail,
        at: u64,
    ) -> Self {
        Self {
            account: account.to_owned(),
            kind,
            by,
            detail: sanitize_detail(&detail.to_stored()),
            at,
        }
    }

    /// The typed form of this event's detail, or `None` when it does not parse (an older or damaged row): the reader shows nothing for it.
    pub fn detail_data(&self) -> Option<EventDetail> {
        EventDetail::parse(self.kind, &self.detail)
    }
}

/// Where the events are kept. **No method deletes or changes a row.**
pub trait AccountEventStore: Send + Sync {
    /// Append one event (its detail is made display-safe and cut again here, so a caller that built the struct by hand cannot write a
    /// disguise or an overlong text).
    fn record(&self, event: &AccountEvent) -> Result<()>;
    /// The account's newest events first, at most `limit` (and never more than [`RECENT_MAX`]).
    fn recent(&self, account: &str, limit: usize) -> Result<Vec<AccountEvent>>;
    /// Every event of the account, oldest first.
    fn all(&self, account: &str) -> Result<Vec<AccountEvent>>;
}

/// In-memory events (tests, development).
#[derive(Default)]
pub struct InMemoryAccountEventStore {
    rows: Mutex<Vec<AccountEvent>>,
}

impl InMemoryAccountEventStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<AccountEvent>>> {
        self.rows
            .lock()
            .map_err(|_| AuthError::Backend("event lock poisoned".to_owned()))
    }
}

impl AccountEventStore for InMemoryAccountEventStore {
    fn record(&self, event: &AccountEvent) -> Result<()> {
        let mut e = event.clone();
        e.detail = sanitize_detail(&event.detail);
        self.lock()?.push(e);
        Ok(())
    }

    fn recent(&self, account: &str, limit: usize) -> Result<Vec<AccountEvent>> {
        let rows = self.lock()?;
        Ok(rows
            .iter()
            .rev()
            .filter(|e| e.account == account)
            .take(limit.min(RECENT_MAX))
            .cloned()
            .collect())
    }

    fn all(&self, account: &str) -> Result<Vec<AccountEvent>> {
        Ok(self
            .lock()?
            .iter()
            .filter(|e| e.account == account)
            .cloned()
            .collect())
    }
}

/// The table, applied idempotently with the rest of the auth schema.
pub(crate) const EVENTS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS account_events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    kind    TEXT NOT NULL CHECK (kind IN ('created', 'password_set', 'password_changed', 'identity_linked', 'identity_unlinked',
                                          'token_created', 'token_revoked', 'session_ended', 'role_granted', 'role_revoked',
                                          'disabled', 'enabled', 'signed_in_new_method')),
    \"by\"    TEXT NOT NULL CHECK (\"by\" IN ('self', 'operator', 'provider')),
    detail  TEXT NOT NULL DEFAULT '',
    at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS account_events_by_account ON account_events (account, id);
";

/// Write one event on an open connection (used by the store below and, inside a transaction, by the account writes).
pub(crate) fn insert_event(
    conn: &rusqlite::Connection,
    event: &AccountEvent,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO account_events (account, kind, \"by\", detail, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            event.account,
            event.kind.as_str(),
            event.by.as_str(),
            sanitize_detail(&event.detail),
            i64::try_from(event.at).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(())
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<AccountEvent> {
    let kind: String = r.get(1)?;
    let by: String = r.get(2)?;
    Ok(AccountEvent {
        account: r.get(0)?,
        kind: AccountEventKind::parse(&kind).unwrap_or(AccountEventKind::Created),
        by: EventBy::parse(&by).unwrap_or(EventBy::Operator),
        detail: r.get(3)?,
        at: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
    })
}

/// The SQLite [`AccountEventStore`], over the server's shared database.
#[derive(Clone)]
pub struct SqliteAccountEventStore {
    db: SqliteDb,
}

impl SqliteAccountEventStore {
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(EVENTS_SCHEMA)
            .map_err(|e| AuthError::Backend(e.to_string()))?;
        Ok(Self { db })
    }
}

fn be(e: impl std::fmt::Display) -> AuthError {
    AuthError::Backend(e.to_string())
}

impl AccountEventStore for SqliteAccountEventStore {
    fn record(&self, event: &AccountEvent) -> Result<()> {
        let conn = self.db.lock().map_err(be)?;
        insert_event(&conn, event).map_err(be)
    }

    fn recent(&self, account: &str, limit: usize) -> Result<Vec<AccountEvent>> {
        let conn = self.db.lock().map_err(be)?;
        let mut stmt = conn
            .prepare(
                "SELECT account, kind, \"by\", detail, at FROM account_events WHERE account = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(be)?;
        let rows = stmt
            .query_map(
                params![account, i64::try_from(limit.min(RECENT_MAX)).unwrap_or(0)],
                row_to_event,
            )
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)?;
        Ok(rows)
    }

    fn all(&self, account: &str) -> Result<Vec<AccountEvent>> {
        let conn = self.db.lock().map_err(be)?;
        let mut stmt = conn
            .prepare("SELECT account, kind, \"by\", detail, at FROM account_events WHERE account = ?1 ORDER BY id")
            .map_err(be)?;
        let rows = stmt
            .query_map(params![account], row_to_event)
            .map_err(be)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(be)?;
        Ok(rows)
    }
}
