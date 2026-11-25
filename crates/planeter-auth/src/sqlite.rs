//! SQLite [`AccountStore`] and [`CredentialStore`] (M1 pre-tag item 2) over `planeter-store`'s shared
//! [`SqliteDb`]. This crate applies its own tables to the shared database (idempotent), so the lower
//! store never learns the auth schema. Same traits, same tests as the in-memory stores.
//!
//! Secrets are stored only as hashes/fingerprints (never a plaintext token or password); a [`Scope`] is
//! stored as its access class, an optional newline-joined repository list (`NULL` = all), and an
//! optional expiry.

use rusqlite::{OptionalExtension, params};

use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{RepoId, SqliteDb, UserId};

use crate::credential::{CredentialStore, DeployKeyRecord, SshKeyRecord, TokenRecord};
use crate::events::EVENTS_SCHEMA;
use crate::hashing::{PasswordHash, TokenHash};
use crate::identity::{Account, AccountStore, AuthError, Result};
use crate::session::{Session, SessionId, SessionStore, random_token};

const AUTH_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    user          TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL,
    email         TEXT,
    password_hash TEXT
);
CREATE TABLE IF NOT EXISTS tokens (
    hash       TEXT PRIMARY KEY,
    user       TEXT NOT NULL,
    access     TEXT NOT NULL,          -- 'read-only' | 'write' | 'full'
    repos      TEXT,                   -- NULL = all; else newline-joined repo ids
    expires_at INTEGER
);
CREATE TABLE IF NOT EXISTS ssh_keys (
    fingerprint TEXT PRIMARY KEY,
    user        TEXT NOT NULL,
    access      TEXT NOT NULL,
    repos       TEXT,
    expires_at  INTEGER
);
CREATE TABLE IF NOT EXISTS oidc_links (
    issuer  TEXT NOT NULL,
    subject TEXT NOT NULL,
    user    TEXT NOT NULL,
    PRIMARY KEY (issuer, subject)
);
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY,
    user       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS deploy_keys (
    fingerprint TEXT PRIMARY KEY,
    registrant  TEXT NOT NULL,
    repo_id     TEXT NOT NULL,
    write       INTEGER NOT NULL
);
";

fn be(e: impl std::fmt::Display) -> AuthError {
    AuthError::Backend(e.to_string())
}

/// What the start-up migration of the account tables found and did (RFC 014 T3). **Counts only**: no handle, no token, no id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountsMigration {
    /// Accounts that had no `handle_key` and were given one now (0 on every start after the first).
    pub handles_keyed: usize,
    /// Tokens that had no public id and were given one now (0 on every start after the first).
    pub tokens_identified: usize,
    /// Accounts whose handle is the same as another account's when case is ignored (the state, not the work of this start).
    /// Nobody is renamed and the start does not fail: these accounts are left out of the unique index, and `planeter operator account
    /// list --check` (a later handoff) reports them.
    pub colliding_accounts: usize,
}

/// Run `work` on the shared connection inside one `BEGIN IMMEDIATE` transaction: it commits when `work` returns `Ok` and **rolls back
/// when it returns `Err`**, so a refused act changes nothing (and leaves no event). A writer waits at most the connection's busy timeout
/// (5 s) for another process's write to finish.
pub(crate) fn immediate<T>(
    db: &SqliteDb,
    work: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
) -> Result<T> {
    let conn = db.lock().map_err(be)?;
    conn.execute_batch("BEGIN IMMEDIATE").map_err(be)?;
    match work(&conn) {
        Ok(v) => {
            conn.execute_batch("COMMIT").map_err(be)?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(be(e))
        }
    }
}

/// 16 random lowercase hexadecimal characters: the public identifier of a token (it names the token to its holder and to the operator;
/// it is **not** the secret and cannot be used to present the token).
pub(crate) fn random_token_id() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The placeholder name of a token that was made before tokens had names.
pub const LEGACY_TOKEN_NAME: &str = "(created before 0.10.0)";

/// Apply the auth schema and migrate an older database to it, **idempotently** (a second call does nothing and changes nothing):
/// the new columns, the public id and name of every older token, the case-insensitive key of every handle and the unique indexes that
/// hold both rules in the database. Nothing existing is renamed, removed or rewritten; a token's hash and secret are untouched.
pub(crate) fn prepare(db: &SqliteDb) -> Result<AccountsMigration> {
    db.apply_schema(AUTH_SCHEMA).map_err(be)?;
    db.apply_schema(EVENTS_SCHEMA).map_err(be)?;
    for (table, column, declaration) in [
        ("accounts", "created_at", "INTEGER"),
        ("accounts", "disabled_at", "INTEGER"),
        ("accounts", "disabled_reason", "TEXT"),
        ("accounts", "password_changed_at", "INTEGER"),
        ("accounts", "handle_key", "TEXT"),
        ("accounts", "handle_collides", "INTEGER NOT NULL DEFAULT 0"),
        ("tokens", "id", "TEXT"),
        ("tokens", "name", "TEXT"),
        ("tokens", "created_at", "INTEGER"),
        ("tokens", "last_used_at", "INTEGER"),
    ] {
        db.ensure_column(table, column, declaration).map_err(be)?;
    }
    immediate(db, |conn| {
        // Every account has a key: its handle in ASCII lowercase (SQLite's `lower` folds ASCII only, as `handle_key` does).
        let handles_keyed = conn.execute(
            "UPDATE accounts SET handle_key = lower(user) WHERE handle_key IS NULL",
            [],
        )?;
        // Which accounts share a key with another. Recomputed every start from the data (it is state, and costs one pass).
        conn.execute(
            "UPDATE accounts SET handle_collides = \
               CASE WHEN (SELECT COUNT(*) FROM accounts a2 WHERE a2.handle_key = accounts.handle_key) > 1 THEN 1 ELSE 0 END",
            [],
        )?;
        let colliding_accounts = usize::try_from(conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE handle_collides = 1",
            [],
            |r| r.get::<_, i64>(0),
        )?)
        .unwrap_or(0);
        // The database holds the rule: unique among every account that does not collide. A colliding pair is left out of the index (it
        // could not be built with it), which is why the creation of a new account also asks the table, not only the index.
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS accounts_handle_key ON accounts (handle_key) WHERE handle_collides = 0",
        )?;
        // Every token has a public id and a name, assigned once, in this transaction.
        let missing: Vec<i64> = {
            let mut stmt =
                conn.prepare("SELECT rowid FROM tokens WHERE id IS NULL ORDER BY rowid")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        for rowid in &missing {
            conn.execute(
                "UPDATE tokens SET id = ?1 WHERE rowid = ?2",
                params![random_token_id(), rowid],
            )?;
        }
        let tokens_identified = missing.len();
        conn.execute(
            "UPDATE tokens SET name = ?1 WHERE name IS NULL",
            params![LEGACY_TOKEN_NAME],
        )?;
        conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS tokens_id ON tokens (id)")?;
        Ok(AccountsMigration {
            handles_keyed,
            tokens_identified,
            colliding_accounts,
        })
    })
}

/// Whether a handle is taken, **compared case-insensitively** (RFC 014 D-2): by an account (colliding ones included), by an organization
/// (a role, a team) or by the owner of a repository. The caller holds the transaction.
#[allow(dead_code)] // used by the account writes of RFC 014 T6
pub(crate) fn handle_taken(conn: &rusqlite::Connection, key: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM accounts WHERE handle_key = ?1) \
             OR EXISTS (SELECT 1 FROM org_roles WHERE lower(org) = ?1) \
             OR EXISTS (SELECT 1 FROM teams WHERE lower(org) = ?1) \
             OR EXISTS (SELECT 1 FROM repositories WHERE lower(owner) = ?1)",
        params![key],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n != 0)
}

fn access_str(a: ScopeAccess) -> &'static str {
    match a {
        ScopeAccess::ReadOnly => "read-only",
        ScopeAccess::Write => "write",
        ScopeAccess::Full => "full",
    }
}
fn access_from(s: &str) -> Result<ScopeAccess> {
    match s {
        "read-only" => Ok(ScopeAccess::ReadOnly),
        "write" => Ok(ScopeAccess::Write),
        "full" => Ok(ScopeAccess::Full),
        other => Err(AuthError::Backend(format!("bad scope access {other:?}"))),
    }
}
fn scope_cols(s: &Scope) -> (&'static str, Option<String>, Option<i64>) {
    let repos = match &s.repos {
        RepoScope::All => None,
        RepoScope::Only(ids) => Some(
            ids.iter()
                .map(|r| r.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    };
    let exp = s.expires_at.map(|e| i64::try_from(e).unwrap_or(i64::MAX));
    (access_str(s.access), repos, exp)
}
fn scope_from(access: String, repos: Option<String>, exp: Option<i64>) -> Result<Scope> {
    Ok(Scope {
        access: access_from(&access)?,
        repos: match repos {
            None => RepoScope::All,
            Some(joined) => RepoScope::Only(
                joined
                    .split('\n')
                    .filter(|s| !s.is_empty())
                    .map(RepoId::new)
                    .collect(),
            ),
        },
        expires_at: exp.map(|e| u64::try_from(e).unwrap_or(0)),
    })
}

/// [`AccountStore`] over SQLite.
#[derive(Clone)]
pub struct SqliteAccountStore {
    db: SqliteDb,
    migration: AccountsMigration,
}

impl SqliteAccountStore {
    /// Open over `db`: apply the schema and migrate an older database (idempotent; see [`AccountsMigration`]).
    pub fn new(db: SqliteDb) -> Result<Self> {
        let migration = prepare(&db)?;
        Ok(Self { db, migration })
    }

    /// What the migration at construction found and did.
    pub fn migration(&self) -> AccountsMigration {
        self.migration
    }
}

impl AccountStore for SqliteAccountStore {
    fn get(&self, user: &UserId) -> Result<Option<Account>> {
        let conn = self.db.lock().map_err(be)?;
        conn.query_row(
            "SELECT user, display_name, email, password_hash FROM accounts WHERE user = ?1",
            params![user.as_str()],
            |r| {
                Ok(Account {
                    user: UserId::new(r.get::<_, String>(0)?),
                    display_name: r.get(1)?,
                    email: r.get(2)?,
                    password: r.get::<_, Option<String>>(3)?.map(PasswordHash),
                })
            },
        )
        .optional()
        .map_err(be)
    }

    fn upsert(&self, account: Account) -> Result<()> {
        // The legacy write: it never refuses a handle (existing tests and hand-seeded data), but keeps the case-insensitive key and the
        // collision mark true. It does not touch the state columns (disabled, created, password changed).
        let key = planeter_core::creation::handle_key(account.user.as_str());
        immediate(&self.db, |conn| {
            // Another account with the same key (a different handle): both are marked colliding *before* the row is written, so the
            // unique index (which leaves colliding accounts out) never refuses the write.
            let others: i64 = conn.query_row(
                "SELECT COUNT(*) FROM accounts WHERE handle_key = ?1 AND user != ?2",
                params![key, account.user.as_str()],
                |r| r.get(0),
            )?;
            if others > 0 {
                conn.execute(
                    "UPDATE accounts SET handle_collides = 1 WHERE handle_key = ?1",
                    params![key],
                )?;
            }
            conn.execute(
                "INSERT INTO accounts (user, display_name, email, password_hash, handle_key, handle_collides) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(user) DO UPDATE SET display_name = excluded.display_name, \
                 email = excluded.email, password_hash = excluded.password_hash",
                params![
                    account.user.as_str(),
                    account.display_name,
                    account.email,
                    account.password.map(|p| p.0),
                    key,
                    i64::from(others > 0),
                ],
            )?;
            Ok(())
        })
    }

    fn set_password_hash(&self, user: &UserId, hash: PasswordHash) -> Result<()> {
        self.db
            .lock()
            .map_err(be)?
            .execute(
                "UPDATE accounts SET password_hash = ?2 WHERE user = ?1",
                params![user.as_str(), hash.0],
            )
            .map_err(be)?;
        Ok(())
    }

    fn link_oidc(&self, issuer: &str, subject: &str, user: UserId) -> Result<()> {
        self.db
            .lock()
            .map_err(be)?
            .execute(
                "INSERT INTO oidc_links (issuer, subject, user) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(issuer, subject) DO UPDATE SET user = excluded.user",
                params![issuer, subject, user.as_str()],
            )
            .map_err(be)?;
        Ok(())
    }

    fn lookup_oidc(&self, issuer: &str, subject: &str) -> Result<Option<UserId>> {
        let conn = self.db.lock().map_err(be)?;
        conn.query_row(
            "SELECT user FROM oidc_links WHERE issuer = ?1 AND subject = ?2",
            params![issuer, subject],
            |r| Ok(UserId::new(r.get::<_, String>(0)?)),
        )
        .optional()
        .map_err(be)
    }
}

/// [`CredentialStore`] over SQLite, with the same writer methods as the in-memory store.
#[derive(Clone)]
pub struct SqliteCredentialStore {
    db: SqliteDb,
}

impl SqliteCredentialStore {
    pub fn new(db: SqliteDb) -> Result<Self> {
        prepare(&db)?;
        Ok(Self { db })
    }

    pub fn add_token(&self, record: TokenRecord) -> Result<()> {
        let (access, repos, exp) = scope_cols(&record.scope);
        self.db
            .lock()
            .map_err(be)?
            .execute(
                "INSERT INTO tokens (hash, user, access, repos, expires_at) VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(hash) DO UPDATE SET user = excluded.user, access = excluded.access, \
                 repos = excluded.repos, expires_at = excluded.expires_at",
                params![record.hash.0, record.user.as_str(), access, repos, exp],
            )
            .map_err(be)?;
        Ok(())
    }

    pub fn add_ssh_key(&self, record: SshKeyRecord) -> Result<()> {
        let (access, repos, exp) = scope_cols(&record.scope);
        self.db
            .lock()
            .map_err(be)?
            .execute(
                "INSERT INTO ssh_keys (fingerprint, user, access, repos, expires_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(fingerprint) DO UPDATE SET user = excluded.user, access = excluded.access, \
                 repos = excluded.repos, expires_at = excluded.expires_at",
                params![record.fingerprint, record.user.as_str(), access, repos, exp],
            )
            .map_err(be)?;
        Ok(())
    }

    pub fn add_deploy_key(&self, record: DeployKeyRecord) -> Result<()> {
        self.db
            .lock()
            .map_err(be)?
            .execute(
                "INSERT INTO deploy_keys (fingerprint, registrant, repo_id, write) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(fingerprint) DO UPDATE SET registrant = excluded.registrant, \
                 repo_id = excluded.repo_id, write = excluded.write",
                params![
                    record.fingerprint,
                    record.registrant.as_str(),
                    record.repo.as_str(),
                    i64::from(record.write)
                ],
            )
            .map_err(be)?;
        Ok(())
    }
}

impl CredentialStore for SqliteCredentialStore {
    fn find_token(&self, hash: &TokenHash) -> Result<Option<TokenRecord>> {
        let conn = self.db.lock().map_err(be)?;
        let row = conn
            .query_row(
                "SELECT hash, user, access, repos, expires_at FROM tokens WHERE hash = ?1",
                params![hash.0],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(be)?;
        row.map(|(h, user, access, repos, exp)| {
            Ok(TokenRecord {
                hash: TokenHash(h),
                user: UserId::new(user),
                scope: scope_from(access, repos, exp)?,
            })
        })
        .transpose()
    }

    fn find_ssh_key(&self, fingerprint: &str) -> Result<Option<SshKeyRecord>> {
        let conn = self.db.lock().map_err(be)?;
        let row = conn
            .query_row(
                "SELECT fingerprint, user, access, repos, expires_at FROM ssh_keys WHERE fingerprint = ?1",
                params![fingerprint],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(be)?;
        row.map(|(fp, user, access, repos, exp)| {
            Ok(SshKeyRecord {
                fingerprint: fp,
                user: UserId::new(user),
                scope: scope_from(access, repos, exp)?,
            })
        })
        .transpose()
    }

    fn find_deploy_key(&self, fingerprint: &str) -> Result<Option<DeployKeyRecord>> {
        let conn = self.db.lock().map_err(be)?;
        conn.query_row(
            "SELECT fingerprint, registrant, repo_id, write FROM deploy_keys WHERE fingerprint = ?1",
            params![fingerprint],
            |r| {
                Ok(DeployKeyRecord {
                    fingerprint: r.get(0)?,
                    registrant: UserId::new(r.get::<_, String>(1)?),
                    repo: RepoId::new(r.get::<_, String>(2)?),
                    write: r.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()
        .map_err(be)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authenticator::Authenticator;
    use crate::hashing::{
        Argon2idHasher, PasswordHasher as _, Sha256TokenHasher, TokenHasher as _,
    };
    use planeter_core::Principal;
    use std::sync::Arc;

    #[test]
    fn accounts_and_credentials_round_trip_on_sqlite() {
        let db = SqliteDb::open_in_memory().unwrap();
        let accounts = SqliteAccountStore::new(db.clone()).unwrap();
        let creds = SqliteCredentialStore::new(db).unwrap();

        accounts
            .upsert(Account {
                user: UserId::new("alice"),
                display_name: "Alice".into(),
                email: Some("a@example".into()),
                password: Some(Argon2idHasher.hash("pw")),
            })
            .unwrap();
        let a = accounts.get(&UserId::new("alice")).unwrap().unwrap();
        assert_eq!(a.display_name, "Alice");
        assert!(a.password.is_some());
        assert!(accounts.get(&UserId::new("ghost")).unwrap().is_none());

        let scope = Scope {
            access: ScopeAccess::Write,
            repos: RepoScope::Only(vec![RepoId::new("r1"), RepoId::new("r2")]),
            expires_at: Some(2000),
        };
        creds
            .add_token(TokenRecord {
                hash: Sha256TokenHasher.hash("tok"),
                user: UserId::new("alice"),
                scope: scope.clone(),
            })
            .unwrap();
        let t = creds
            .find_token(&Sha256TokenHasher.hash("tok"))
            .unwrap()
            .unwrap();
        assert_eq!(t.user, UserId::new("alice"));
        assert_eq!(t.scope, scope); // access + repo list + expiry survive the round trip
        assert!(
            creds
                .find_token(&TokenHash("nope".into()))
                .unwrap()
                .is_none()
        );

        creds
            .add_ssh_key(SshKeyRecord {
                fingerprint: "SHA256:x".into(),
                user: UserId::new("alice"),
                scope: Scope {
                    access: ScopeAccess::Full,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
        let k = creds.find_ssh_key("SHA256:x").unwrap().unwrap();
        assert_eq!(k.scope.repos, RepoScope::All);

        creds
            .add_deploy_key(DeployKeyRecord {
                fingerprint: "SHA256:d".into(),
                registrant: UserId::new("alice"),
                repo: RepoId::new("r1"),
                write: true,
            })
            .unwrap();
        let d = creds.find_deploy_key("SHA256:d").unwrap().unwrap();
        assert!(d.write);
        assert_eq!(d.repo, RepoId::new("r1"));
    }

    #[test]
    fn authenticator_works_over_sqlite_with_production_hashers() {
        let db = SqliteDb::open_in_memory().unwrap();
        let accounts = Arc::new(SqliteAccountStore::new(db.clone()).unwrap());
        let creds = Arc::new(SqliteCredentialStore::new(db).unwrap());
        accounts
            .upsert(Account {
                user: UserId::new("bob"),
                display_name: "Bob".into(),
                email: None,
                password: Some(Argon2idHasher.hash("hunter2")),
            })
            .unwrap();
        let tok = crate::hashing::mint_token();
        creds
            .add_token(TokenRecord {
                hash: Sha256TokenHasher.hash(&tok),
                user: UserId::new("bob"),
                scope: Scope {
                    access: ScopeAccess::ReadOnly,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
        let auth = Authenticator::new(
            accounts,
            creds,
            Arc::new(Argon2idHasher),
            Arc::new(Sha256TokenHasher),
        );
        assert_eq!(
            auth.authenticate_password("bob", "hunter2").unwrap(),
            Principal::User(UserId::new("bob"))
        );
        assert!(auth.authenticate_password("bob", "wrong").is_err());
        assert!(matches!(
            auth.authenticate_token(&tok).unwrap(),
            Principal::MachineAsUser { .. }
        ));
        assert!(auth.authenticate_token("plt_forged").is_err());
    }
}

/// [`SessionStore`] over SQLite.
#[derive(Clone)]
pub struct SqliteSessionStore {
    db: SqliteDb,
}

impl SqliteSessionStore {
    pub fn new(db: SqliteDb) -> Result<Self> {
        prepare(&db)?;
        Ok(Self { db })
    }
}

impl SessionStore for SqliteSessionStore {
    fn create(&self, user: UserId, now_unix: u64, expires_at: u64) -> Result<Session> {
        let s = Session {
            id: SessionId(random_token()),
            user,
            created_at: now_unix,
            expires_at,
        };
        self.db
            .lock()
            .map_err(be)?
            .execute(
                "INSERT INTO sessions (id, user, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    s.id.0,
                    s.user.as_str(),
                    i64::try_from(s.created_at).unwrap_or(i64::MAX),
                    i64::try_from(s.expires_at).unwrap_or(i64::MAX)
                ],
            )
            .map_err(be)?;
        Ok(s)
    }

    fn get(&self, id: &SessionId, now_unix: u64) -> Result<Option<Session>> {
        let conn = self.db.lock().map_err(be)?;
        conn.query_row(
            "SELECT id, user, created_at, expires_at FROM sessions WHERE id = ?1 AND expires_at > ?2",
            params![id.0, i64::try_from(now_unix).unwrap_or(i64::MAX)],
            |r| {
                Ok(Session {
                    id: SessionId(r.get(0)?),
                    user: UserId::new(r.get::<_, String>(1)?),
                    created_at: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                    expires_at: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                })
            },
        )
        .optional()
        .map_err(be)
    }

    fn delete(&self, id: &SessionId) -> Result<()> {
        self.db
            .lock()
            .map_err(be)?
            .execute("DELETE FROM sessions WHERE id = ?1", params![id.0])
            .map_err(be)?;
        Ok(())
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn sqlite_sessions_round_trip_and_expire() {
        let s = SqliteSessionStore::new(SqliteDb::open_in_memory().unwrap()).unwrap();
        let sess = s.create(UserId::new("bob"), 10, 20).unwrap();
        assert_eq!(
            s.get(&sess.id, 15).unwrap().unwrap().user,
            UserId::new("bob")
        );
        assert!(s.get(&sess.id, 20).unwrap().is_none());
        s.delete(&sess.id).unwrap();
        assert!(s.get(&sess.id, 15).unwrap().is_none());
    }
}
