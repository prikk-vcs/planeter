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
use crate::hashing::{PasswordHash, TokenHash};
use crate::identity::{Account, AccountStore, AuthError, Result};

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
}

impl SqliteAccountStore {
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(AUTH_SCHEMA).map_err(be)?;
        Ok(Self { db })
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
        let conn = self.db.lock().map_err(be)?;
        conn.execute(
            "INSERT INTO accounts (user, display_name, email, password_hash) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(user) DO UPDATE SET display_name = excluded.display_name, \
             email = excluded.email, password_hash = excluded.password_hash",
            params![
                account.user.as_str(),
                account.display_name,
                account.email,
                account.password.map(|p| p.0),
            ],
        )
        .map_err(be)?;
        Ok(())
    }
}

/// [`CredentialStore`] over SQLite, with the same writer methods as the in-memory store.
#[derive(Clone)]
pub struct SqliteCredentialStore {
    db: SqliteDb,
}

impl SqliteCredentialStore {
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.apply_schema(AUTH_SCHEMA).map_err(be)?;
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
