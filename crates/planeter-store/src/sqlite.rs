//! The SQLite backend (RFC 001 IQ-4; M1 pre-tag item 2): [`SqliteDb`] — one bundled-SQLite connection
//! behind a mutex, with idempotent schema application — and the [`SqliteRepositoryStore`] /
//! [`SqliteMembershipStore`] implementations over it. Caller-invisible: they implement the same traits
//! as the in-memory stores and pass the same tests.
//!
//! Layering: this crate owns the connection and its own tables; `planeter-auth` applies its own tables
//! to the same [`SqliteDb`] via [`SqliteDb::apply_schema`] (idempotent `CREATE TABLE IF NOT EXISTS`),
//! so the lower store never learns the upper crate's schema.
//!
//! Concurrency: a single process serializes access through the mutex (SQLite is not a multi-writer
//! database); WAL mode lets readers proceed during a write. A `busy_timeout` covers the rare lock wait.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use crate::membership::{MembershipStore, OrgId, OrgRole, Role, Subject, TeamId, UserId};
use crate::model::{Owner, RepoId, RepositoryRecord, Visibility};
use crate::store::{RepositoryStore, Result, StoreError};

/// A shared SQLite database handle (cheap to clone; one connection behind a mutex).
#[derive(Clone)]
pub struct SqliteDb {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteDb {
    /// Open (creating if absent) the database file at `path`, set WAL + busy timeout, and apply this
    /// crate's schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref()).map_err(backend)?;
        Self::init(conn)
    }

    /// An in-memory database (tests/dev). State lives only as long as this handle.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(backend)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;\n\
             PRAGMA foreign_keys = ON;\n\
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(backend)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.apply_schema(STORE_SCHEMA)?;
        Ok(db)
    }

    /// Apply an idempotent schema batch (`CREATE TABLE IF NOT EXISTS …`). Upper crates use this to add
    /// their own tables to the shared database.
    pub fn apply_schema(&self, sql: &str) -> Result<()> {
        self.lock()?.execute_batch(sql).map_err(backend)
    }

    /// Lock the connection. Public so upper crates' stores can run their own queries.
    pub fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| StoreError::Backend("sqlite connection lock poisoned".to_owned()))
    }
}

/// Map any rusqlite error to a backend error.
pub fn backend(e: rusqlite::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

const STORE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repositories (
    repo_id              TEXT PRIMARY KEY,
    owner_kind           TEXT NOT NULL,      -- 'user' | 'org'
    owner                TEXT NOT NULL,
    name                 TEXT NOT NULL,
    visibility           TEXT NOT NULL,      -- 'public' | 'internal' | 'private'
    created_at           INTEGER NOT NULL,
    prikk_format_version TEXT,
    path                 TEXT NOT NULL,
    UNIQUE (owner_kind, owner, name)
);
CREATE TABLE IF NOT EXISTS repo_grants (
    subject_kind TEXT NOT NULL,              -- 'user' | 'team'
    subject      TEXT NOT NULL,
    repo_id      TEXT NOT NULL,
    role         TEXT NOT NULL,              -- 'read' | 'write' | 'maintain' | 'admin'
    PRIMARY KEY (subject_kind, subject, repo_id)
);
CREATE TABLE IF NOT EXISTS teams (
    team TEXT PRIMARY KEY,
    org  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS team_members (
    user TEXT NOT NULL,
    team TEXT NOT NULL,
    PRIMARY KEY (user, team)
);
CREATE TABLE IF NOT EXISTS org_roles (
    user TEXT NOT NULL,
    org  TEXT NOT NULL,
    role TEXT NOT NULL,                      -- 'member' | 'admin'
    PRIMARY KEY (user, org)
);
";

// -- encoding helpers (stable text forms; never Debug output) --

fn owner_parts(o: &Owner) -> (&'static str, &str) {
    match o {
        Owner::User(h) => ("user", h),
        Owner::Org(h) => ("org", h),
    }
}
fn owner_from(kind: &str, handle: String) -> Result<Owner> {
    match kind {
        "user" => Ok(Owner::User(handle)),
        "org" => Ok(Owner::Org(handle)),
        other => Err(StoreError::Backend(format!("bad owner_kind {other:?}"))),
    }
}
fn vis_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Internal => "internal",
        Visibility::Private => "private",
    }
}
fn vis_from(s: &str) -> Result<Visibility> {
    match s {
        "public" => Ok(Visibility::Public),
        "internal" => Ok(Visibility::Internal),
        "private" => Ok(Visibility::Private),
        other => Err(StoreError::Backend(format!("bad visibility {other:?}"))),
    }
}
fn role_str(r: Role) -> &'static str {
    match r {
        Role::Read => "read",
        Role::Write => "write",
        Role::Maintain => "maintain",
        Role::Admin => "admin",
    }
}
fn role_from(s: &str) -> Result<Role> {
    match s {
        "read" => Ok(Role::Read),
        "write" => Ok(Role::Write),
        "maintain" => Ok(Role::Maintain),
        "admin" => Ok(Role::Admin),
        other => Err(StoreError::Backend(format!("bad role {other:?}"))),
    }
}
fn org_role_str(r: OrgRole) -> &'static str {
    match r {
        OrgRole::Member => "member",
        OrgRole::Admin => "admin",
    }
}
fn org_role_from(s: &str) -> Result<OrgRole> {
    match s {
        "member" => Ok(OrgRole::Member),
        "admin" => Ok(OrgRole::Admin),
        other => Err(StoreError::Backend(format!("bad org role {other:?}"))),
    }
}

// ----------------------------------------------------------------------------
// RepositoryStore
// ----------------------------------------------------------------------------

/// [`RepositoryStore`] over SQLite.
#[derive(Clone)]
pub struct SqliteRepositoryStore {
    db: SqliteDb,
}

impl SqliteRepositoryStore {
    pub fn new(db: SqliteDb) -> Self {
        Self { db }
    }

    fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRow> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ))
    }

    fn decode(
        (repo_id, owner_kind, owner, name, visibility, created_at, fmt, path): RepoRow,
    ) -> Result<RepositoryRecord> {
        Ok(RepositoryRecord {
            repo_id: RepoId::new(repo_id),
            owner: owner_from(&owner_kind, owner)?,
            name,
            visibility: vis_from(&visibility)?,
            created_at: u64::try_from(created_at).unwrap_or(0),
            prikk_format_version: fmt,
            path: PathBuf::from(path),
        })
    }
}

/// One `repositories` row, in column order: repo_id, owner_kind, owner, name, visibility,
/// created_at, prikk_format_version, path.
type RepoRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    String,
);

const SELECT_REPO: &str = "SELECT repo_id, owner_kind, owner, name, visibility, created_at, \
                           prikk_format_version, path FROM repositories";

impl RepositoryStore for SqliteRepositoryStore {
    fn create(&self, record: RepositoryRecord) -> Result<()> {
        let conn = self.db.lock()?;
        let (kind, handle) = owner_parts(&record.owner);
        // Explicit conflict checks give the same messages as the in-memory store.
        let by_id: Option<String> = conn
            .query_row(
                "SELECT repo_id FROM repositories WHERE repo_id = ?1",
                params![record.repo_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if by_id.is_some() {
            return Err(StoreError::Conflict(format!(
                "repo_id {} already exists",
                record.repo_id
            )));
        }
        let by_name: Option<String> = conn
            .query_row(
                "SELECT repo_id FROM repositories WHERE owner_kind = ?1 AND owner = ?2 AND name = ?3",
                params![kind, handle, record.name],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if by_name.is_some() {
            return Err(StoreError::Conflict(format!(
                "{}/{} already exists",
                record.owner.handle(),
                record.name
            )));
        }
        conn.execute(
            "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, \
             prikk_format_version, path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.repo_id.as_str(),
                kind,
                handle,
                record.name,
                vis_str(record.visibility),
                i64::try_from(record.created_at).unwrap_or(i64::MAX),
                record.prikk_format_version,
                record.path.to_string_lossy().into_owned(),
            ],
        )
        .map_err(backend)?;
        Ok(())
    }

    fn get(&self, repo_id: &RepoId) -> Result<Option<RepositoryRecord>> {
        let conn = self.db.lock()?;
        let row = conn
            .query_row(
                &format!("{SELECT_REPO} WHERE repo_id = ?1"),
                params![repo_id.as_str()],
                Self::row_to_record,
            )
            .optional()
            .map_err(backend)?;
        row.map(Self::decode).transpose()
    }

    fn resolve(&self, owner: &Owner, name: &str) -> Result<Option<RepoId>> {
        let conn = self.db.lock()?;
        let (kind, handle) = owner_parts(owner);
        let id: Option<String> = conn
            .query_row(
                "SELECT repo_id FROM repositories WHERE owner_kind = ?1 AND owner = ?2 AND name = ?3",
                params![kind, handle, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        Ok(id.map(RepoId::new))
    }

    fn rename(&self, repo_id: &RepoId, new_owner: Owner, new_name: String) -> Result<()> {
        let conn = self.db.lock()?;
        let (kind, handle) = owner_parts(&new_owner);
        let exists: Option<String> = conn
            .query_row(
                "SELECT repo_id FROM repositories WHERE repo_id = ?1",
                params![repo_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if exists.is_none() {
            return Err(StoreError::NotFound);
        }
        let taken: Option<String> = conn
            .query_row(
                "SELECT repo_id FROM repositories WHERE owner_kind = ?1 AND owner = ?2 AND name = ?3",
                params![kind, handle, new_name],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if taken.is_some_and(|t| t != repo_id.as_str()) {
            return Err(StoreError::Conflict(format!(
                "{}/{} already exists",
                new_owner.handle(),
                new_name
            )));
        }
        // repo_id and path are intentionally left unchanged (RFC 001 D-5).
        conn.execute(
            "UPDATE repositories SET owner_kind = ?1, owner = ?2, name = ?3 WHERE repo_id = ?4",
            params![kind, handle, new_name, repo_id.as_str()],
        )
        .map_err(backend)?;
        Ok(())
    }

    fn set_prikk_format_version(&self, repo_id: &RepoId, version: String) -> Result<()> {
        let conn = self.db.lock()?;
        let n = conn
            .execute(
                "UPDATE repositories SET prikk_format_version = ?1 WHERE repo_id = ?2",
                params![version, repo_id.as_str()],
            )
            .map_err(backend)?;
        if n == 0 {
            Err(StoreError::NotFound)
        } else {
            Ok(())
        }
    }

    fn delete(&self, repo_id: &RepoId) -> Result<()> {
        let conn = self.db.lock()?;
        let n = conn
            .execute(
                "DELETE FROM repositories WHERE repo_id = ?1",
                params![repo_id.as_str()],
            )
            .map_err(backend)?;
        if n == 0 {
            Err(StoreError::NotFound)
        } else {
            Ok(())
        }
    }

    fn list_by_owner(&self, owner: &Owner) -> Result<Vec<RepositoryRecord>> {
        let conn = self.db.lock()?;
        let (kind, handle) = owner_parts(owner);
        let mut stmt = conn
            .prepare(&format!(
                "{SELECT_REPO} WHERE owner_kind = ?1 AND owner = ?2 ORDER BY name"
            ))
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![kind, handle], Self::row_to_record)
            .map_err(backend)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(Self::decode(row.map_err(backend)?)?);
        }
        Ok(out)
    }
}

// ----------------------------------------------------------------------------
// MembershipStore
// ----------------------------------------------------------------------------

/// [`MembershipStore`] over SQLite, with the same writer methods as the in-memory store.
#[derive(Clone)]
pub struct SqliteMembershipStore {
    db: SqliteDb,
}

impl SqliteMembershipStore {
    pub fn new(db: SqliteDb) -> Self {
        Self { db }
    }

    /// Grant `subject` the role on `repo_id` (raising it if a lower grant exists).
    pub fn grant_repo_role(&self, subject: Subject, repo_id: RepoId, role: Role) -> Result<()> {
        let conn = self.db.lock()?;
        let (kind, id) = match &subject {
            Subject::User(u) => ("user", u.as_str().to_owned()),
            Subject::Team(t) => ("team", t.as_str().to_owned()),
        };
        let current: Option<String> = conn
            .query_row(
                "SELECT role FROM repo_grants WHERE subject_kind = ?1 AND subject = ?2 AND repo_id = ?3",
                params![kind, id, repo_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        let effective = match current {
            Some(c) => role.max(role_from(&c)?),
            None => role,
        };
        conn.execute(
            "INSERT INTO repo_grants (subject_kind, subject, repo_id, role) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(subject_kind, subject, repo_id) DO UPDATE SET role = excluded.role",
            params![kind, id, repo_id.as_str(), role_str(effective)],
        )
        .map_err(backend)?;
        Ok(())
    }

    pub fn create_team(&self, team: TeamId, org: OrgId) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "INSERT INTO teams (team, org) VALUES (?1, ?2) ON CONFLICT(team) DO UPDATE SET org = excluded.org",
                params![team.as_str(), org.as_str()],
            )
            .map_err(backend)?;
        Ok(())
    }

    pub fn add_team_member(&self, user: UserId, team: TeamId) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "INSERT OR IGNORE INTO team_members (user, team) VALUES (?1, ?2)",
                params![user.as_str(), team.as_str()],
            )
            .map_err(backend)?;
        Ok(())
    }

    pub fn set_org_role(&self, user: UserId, org: OrgId, role: OrgRole) -> Result<()> {
        self.db
            .lock()?
            .execute(
                "INSERT INTO org_roles (user, org, role) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(user, org) DO UPDATE SET role = excluded.role",
                params![user.as_str(), org.as_str(), org_role_str(role)],
            )
            .map_err(backend)?;
        Ok(())
    }
}

impl MembershipStore for SqliteMembershipStore {
    fn direct_repo_grants(&self, user: &UserId, repo_id: &RepoId) -> Result<Vec<Role>> {
        let conn = self.db.lock()?;
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM repo_grants WHERE subject_kind = 'user' AND subject = ?1 AND repo_id = ?2",
                params![user.as_str(), repo_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        role.map(|r| role_from(&r))
            .transpose()
            .map(|o| o.into_iter().collect())
    }

    fn team_repo_grants(&self, user: &UserId, org: &OrgId, repo_id: &RepoId) -> Result<Vec<Role>> {
        let conn = self.db.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT g.role FROM repo_grants g \
                 JOIN team_members m ON m.team = g.subject AND g.subject_kind = 'team' \
                 JOIN teams t ON t.team = m.team \
                 WHERE m.user = ?1 AND t.org = ?2 AND g.repo_id = ?3",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(
                params![user.as_str(), org.as_str(), repo_id.as_str()],
                |r| r.get::<_, String>(0),
            )
            .map_err(backend)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(role_from(&r.map_err(backend)?)?);
        }
        Ok(out)
    }

    fn org_role(&self, user: &UserId, org: &OrgId) -> Result<Option<OrgRole>> {
        let conn = self.db.lock()?;
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM org_roles WHERE user = ?1 AND org = ?2",
                params![user.as_str(), org.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        role.map(|r| org_role_from(&r)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, owner: Owner, name: &str) -> RepositoryRecord {
        RepositoryRecord {
            repo_id: RepoId::new(id),
            owner,
            name: name.to_owned(),
            visibility: Visibility::Private,
            created_at: 42,
            prikk_format_version: None,
            path: PathBuf::from(format!("/repos/{id}")),
        }
    }

    #[test]
    fn repository_store_invariants_hold_on_sqlite() {
        let s = SqliteRepositoryStore::new(SqliteDb::open_in_memory().unwrap());
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        let got = s.get(&RepoId::new("r1")).unwrap().unwrap();
        assert_eq!(got.name, "app");
        assert_eq!(got.created_at, 42);
        assert_eq!(
            s.resolve(&Owner::User("alice".into()), "app").unwrap(),
            Some(RepoId::new("r1"))
        );
        // Duplicate owner/name and duplicate id both conflict.
        assert!(matches!(
            s.create(record("r2", Owner::User("alice".into()), "app")),
            Err(StoreError::Conflict(_))
        ));
        assert!(matches!(
            s.create(record("r1", Owner::User("bob".into()), "other")),
            Err(StoreError::Conflict(_))
        ));
        // Rename/transfer keeps id + path (D-5).
        let before = s.get(&RepoId::new("r1")).unwrap().unwrap();
        s.rename(&RepoId::new("r1"), Owner::Org("acme".into()), "core".into())
            .unwrap();
        let after = s.get(&RepoId::new("r1")).unwrap().unwrap();
        assert_eq!(after.repo_id, before.repo_id);
        assert_eq!(after.path, before.path);
        assert_eq!(after.owner, Owner::Org("acme".into()));
        assert!(
            s.resolve(&Owner::User("alice".into()), "app")
                .unwrap()
                .is_none()
        );
        // Rename into a taken name conflicts.
        s.create(record("r3", Owner::Org("acme".into()), "lib"))
            .unwrap();
        assert!(matches!(
            s.rename(&RepoId::new("r3"), Owner::Org("acme".into()), "core".into()),
            Err(StoreError::Conflict(_))
        ));
        // Format version, list, delete, NotFound.
        s.set_prikk_format_version(&RepoId::new("r1"), "7".into())
            .unwrap();
        assert_eq!(
            s.get(&RepoId::new("r1"))
                .unwrap()
                .unwrap()
                .prikk_format_version
                .as_deref(),
            Some("7")
        );
        assert_eq!(
            s.list_by_owner(&Owner::Org("acme".into())).unwrap().len(),
            2
        );
        s.delete(&RepoId::new("r1")).unwrap();
        assert!(matches!(
            s.delete(&RepoId::new("r1")),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            s.set_prikk_format_version(&RepoId::new("ghost"), "7".into()),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn membership_store_resolves_direct_team_and_org_grants() {
        let s = SqliteMembershipStore::new(SqliteDb::open_in_memory().unwrap());
        let user = UserId::new("bob");
        let repo = RepoId::new("r1");
        let org = OrgId::new("acme");
        s.grant_repo_role(Subject::User(user.clone()), repo.clone(), Role::Write)
            .unwrap();
        // Raising works, lowering does not.
        s.grant_repo_role(Subject::User(user.clone()), repo.clone(), Role::Read)
            .unwrap();
        assert_eq!(
            s.direct_repo_grants(&user, &repo).unwrap(),
            vec![Role::Write]
        );

        let team = TeamId::new("dev");
        s.create_team(team.clone(), org.clone()).unwrap();
        s.add_team_member(user.clone(), team.clone()).unwrap();
        s.grant_repo_role(Subject::Team(team), repo.clone(), Role::Maintain)
            .unwrap();
        assert_eq!(
            s.team_repo_grants(&user, &repo_org(&org), &repo).unwrap(),
            vec![Role::Maintain]
        );
        // A team in another org confers nothing.
        assert!(
            s.team_repo_grants(&user, &OrgId::new("other"), &repo)
                .unwrap()
                .is_empty()
        );

        assert_eq!(s.org_role(&user, &org).unwrap(), None);
        s.set_org_role(user.clone(), org.clone(), OrgRole::Admin)
            .unwrap();
        assert_eq!(s.org_role(&user, &org).unwrap(), Some(OrgRole::Admin));
    }

    fn repo_org(o: &OrgId) -> OrgId {
        o.clone()
    }

    #[test]
    fn state_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("planeter-sqlite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.db");
        {
            let s = SqliteRepositoryStore::new(SqliteDb::open(&path).unwrap());
            s.create(record("r1", Owner::User("alice".into()), "app"))
                .unwrap();
        }
        // Reopen: the record is still there, and the schema apply is idempotent.
        let s = SqliteRepositoryStore::new(SqliteDb::open(&path).unwrap());
        assert_eq!(s.get(&RepoId::new("r1")).unwrap().unwrap().name, "app");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
