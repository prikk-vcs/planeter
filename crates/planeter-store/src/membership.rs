//! Roles and membership records — the *inputs* to an authorization decision (RFC 002 D-4). These are
//! pure data plus their persistence; the **decision** that reads them is [`planeter-core`]'s
//! `authorize()`, and the resolution algorithm (membership → effective role) is core's too. Kept in the
//! lower store layer because grant records must be persisted and because `authorize()` (core) may read
//! them, never the reverse (RFC 001 layering).

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use crate::model::{Owner, RepoId};
use crate::store::{Result, StoreError};

/// A user handle (the `<owner>` of a user-owned repo, and the identity a principal carries).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub String);

impl UserId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An organization handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OrgId(pub String);

impl OrgId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A team within an organization.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TeamId(pub String);

impl TeamId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A per-repository role, **totally ordered** by privilege (RFC 002 D-4): `Read ⊂ Write ⊂ Maintain ⊂
/// Admin`. The `Ord` derive follows declaration order, so `Read < Write < Maintain < Admin` — the
/// resolution algorithm takes the `max` across a principal's grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Browse/clone/pull.
    Read,
    /// `Read` + push (contribute author-signed patches).
    Write,
    /// `Write` + merge-seal and ref protection.
    Maintain,
    /// `Maintain` + repository administration (settings, rename/transfer, delete, grants).
    Admin,
}

/// An organization role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrgRole {
    /// A member of the org (may be granted repos; may create repos if the org allows).
    Member,
    /// An org administrator (implicitly `Admin` on the org's repositories).
    Admin,
}

/// The subject of a per-repo role grant: a user directly, or a team (all its members inherit it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subject {
    User(UserId),
    Team(TeamId),
}

/// The raw membership/grant records for one deployment. Read by core's resolution algorithm; never
/// interpreted here. A trait so the backend is swappable (in-memory now, SQLite later — as with
/// [`crate::RepositoryStore`]).
pub trait MembershipStore: Send + Sync {
    /// Direct per-repo role grants to this user (not via a team).
    fn direct_repo_grants(&self, user: &UserId, repo_id: &RepoId) -> Result<Vec<Role>>;

    /// Per-repo role grants to teams that `user` belongs to *and* that live in `org` (the repo's owning
    /// org). Returns the roles those team grants confer.
    fn team_repo_grants(&self, user: &UserId, org: &OrgId, repo_id: &RepoId) -> Result<Vec<Role>>;

    /// This user's org role in `org`, if any.
    fn org_role(&self, user: &UserId, org: &OrgId) -> Result<Option<OrgRole>>;
}

/// An in-memory [`MembershipStore`] (dev/test; the SQLite backend is the follow-on).
#[derive(Default)]
pub struct InMemoryMembershipStore {
    inner: RwLock<MembershipData>,
}

#[derive(Default)]
struct MembershipData {
    /// (subject, repo) → highest granted role. A subject may appear once per repo.
    repo_grants: HashMap<(SubjectKey, RepoId), Role>,
    /// user → teams they belong to.
    team_members: HashMap<UserId, HashSet<TeamId>>,
    /// team → its org.
    team_org: HashMap<TeamId, OrgId>,
    /// (user, org) → org role.
    org_roles: HashMap<(UserId, OrgId), OrgRole>,
}

/// Hashable key for a [`Subject`] (its enum isn't used directly as a map key elsewhere).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SubjectKey {
    User(UserId),
    Team(TeamId),
}

impl From<&Subject> for SubjectKey {
    fn from(s: &Subject) -> Self {
        match s {
            Subject::User(u) => SubjectKey::User(u.clone()),
            Subject::Team(t) => SubjectKey::Team(t.clone()),
        }
    }
}

impl InMemoryMembershipStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, MembershipData>> {
        self.inner
            .write()
            .map_err(|_| StoreError::Backend("membership lock poisoned".to_owned()))
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, MembershipData>> {
        self.inner
            .read()
            .map_err(|_| StoreError::Backend("membership lock poisoned".to_owned()))
    }

    /// Grant `subject` the role on `repo_id` (raising it if a lower grant exists).
    pub fn grant_repo_role(&self, subject: Subject, repo_id: RepoId, role: Role) -> Result<()> {
        let mut d = self.write()?;
        let key = (SubjectKey::from(&subject), repo_id);
        let entry = d.repo_grants.entry(key).or_insert(role);
        if role > *entry {
            *entry = role;
        }
        Ok(())
    }

    /// Create a team in an org.
    pub fn create_team(&self, team: TeamId, org: OrgId) -> Result<()> {
        self.write()?.team_org.insert(team, org);
        Ok(())
    }

    /// Add `user` to `team`.
    pub fn add_team_member(&self, user: UserId, team: TeamId) -> Result<()> {
        self.write()?
            .team_members
            .entry(user)
            .or_default()
            .insert(team);
        Ok(())
    }

    /// Set `user`'s org role.
    pub fn set_org_role(&self, user: UserId, org: OrgId, role: OrgRole) -> Result<()> {
        self.write()?.org_roles.insert((user, org), role);
        Ok(())
    }
}

impl MembershipStore for InMemoryMembershipStore {
    fn direct_repo_grants(&self, user: &UserId, repo_id: &RepoId) -> Result<Vec<Role>> {
        let d = self.read()?;
        Ok(d.repo_grants
            .get(&(SubjectKey::User(user.clone()), repo_id.clone()))
            .copied()
            .into_iter()
            .collect())
    }

    fn team_repo_grants(&self, user: &UserId, org: &OrgId, repo_id: &RepoId) -> Result<Vec<Role>> {
        let d = self.read()?;
        let Some(teams) = d.team_members.get(user) else {
            return Ok(Vec::new());
        };
        let mut roles = Vec::new();
        for team in teams {
            // Only teams belonging to the repo's owning org confer grants.
            if d.team_org.get(team) != Some(org) {
                continue;
            }
            if let Some(role) = d
                .repo_grants
                .get(&(SubjectKey::Team(team.clone()), repo_id.clone()))
            {
                roles.push(*role);
            }
        }
        Ok(roles)
    }

    fn org_role(&self, user: &UserId, org: &OrgId) -> Result<Option<OrgRole>> {
        Ok(self
            .read()?
            .org_roles
            .get(&(user.clone(), org.clone()))
            .copied())
    }
}

/// Helper: the owner's org id, if the owner is an organization.
pub fn owner_org(owner: &Owner) -> Option<OrgId> {
    match owner {
        Owner::Org(h) => Some(OrgId::new(h.clone())),
        Owner::User(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_ordered_by_privilege() {
        assert!(Role::Read < Role::Write);
        assert!(Role::Write < Role::Maintain);
        assert!(Role::Maintain < Role::Admin);
        assert_eq!(
            [Role::Read, Role::Admin, Role::Write].iter().max(),
            Some(&Role::Admin)
        );
    }

    #[test]
    fn direct_and_team_grants_are_recorded() {
        let s = InMemoryMembershipStore::new();
        let user = UserId::new("alice");
        let repo = RepoId::new("r1");
        let org = OrgId::new("acme");

        s.grant_repo_role(Subject::User(user.clone()), repo.clone(), Role::Write)
            .unwrap();
        assert_eq!(
            s.direct_repo_grants(&user, &repo).unwrap(),
            vec![Role::Write]
        );

        let team = TeamId::new("dev");
        s.create_team(team.clone(), org.clone()).unwrap();
        s.add_team_member(user.clone(), team.clone()).unwrap();
        s.grant_repo_role(Subject::Team(team.clone()), repo.clone(), Role::Maintain)
            .unwrap();
        assert_eq!(
            s.team_repo_grants(&user, &org, &repo).unwrap(),
            vec![Role::Maintain]
        );

        // A team in a different org confers nothing.
        let other = OrgId::new("other");
        assert!(s.team_repo_grants(&user, &other, &repo).unwrap().is_empty());
    }

    #[test]
    fn grant_only_raises_never_lowers() {
        let s = InMemoryMembershipStore::new();
        let user = UserId::new("alice");
        let repo = RepoId::new("r1");
        s.grant_repo_role(Subject::User(user.clone()), repo.clone(), Role::Maintain)
            .unwrap();
        s.grant_repo_role(Subject::User(user.clone()), repo.clone(), Role::Read)
            .unwrap();
        assert_eq!(
            s.direct_repo_grants(&user, &repo).unwrap(),
            vec![Role::Maintain]
        );
    }
}
