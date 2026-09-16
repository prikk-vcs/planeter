//! Membership resolution (RFC 002 D-4): a principal's *user* + a repository → the **effective role**,
//! computed from direct grants, team grants, org role, and personal ownership. This is the only place
//! that reads the [`MembershipStore`]; `authorize()` itself is pure over the resolved role this yields
//! (RFC 002 "the decision reads state passed in"). Least privilege is the default — an unmatched
//! principal resolves to `None`, which `authorize()` treats as deny.

use planeter_store::membership::owner_org;
use planeter_store::{MembershipStore, OrgRole, Owner, RepoId, Role, StoreError, UserId};

/// The effective role a user holds on a repository, or `None` if they hold none.
///
/// Resolution takes the **maximum** privilege across every source (D-4, least-privilege by default):
/// - personal ownership: the user who *is* the repo's owner is `Admin`;
/// - org admin: an `Admin` of the owning org is `Admin` on its repos;
/// - direct per-repo grants to the user;
/// - grants to teams (in the owning org) the user belongs to.
pub fn resolve_repo_role(
    store: &dyn MembershipStore,
    user: &UserId,
    repo_id: &RepoId,
    owner: &Owner,
) -> Result<Option<Role>, StoreError> {
    let mut best: Option<Role> = None;
    let raise = |r: Role, best: &mut Option<Role>| {
        if best.is_none_or(|b| r > b) {
            *best = Some(r);
        }
    };

    // Personal repo: the owning user is admin of their own repo.
    if let Owner::User(handle) = owner
        && handle == user.as_str()
    {
        raise(Role::Admin, &mut best);
    }

    for r in store.direct_repo_grants(user, repo_id)? {
        raise(r, &mut best);
    }

    if let Some(org) = owner_org(owner) {
        // Org admins are admins on the org's repositories.
        if store.org_role(user, &org)? == Some(OrgRole::Admin) {
            raise(Role::Admin, &mut best);
        }
        for r in store.team_repo_grants(user, &org, repo_id)? {
            raise(r, &mut best);
        }
    }

    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use planeter_store::{InMemoryMembershipStore, Subject, TeamId};

    #[test]
    fn owner_of_a_personal_repo_is_admin() {
        let store = InMemoryMembershipStore::new();
        let role = resolve_repo_role(
            &store,
            &UserId::new("alice"),
            &RepoId::new("r1"),
            &Owner::User("alice".into()),
        )
        .unwrap();
        assert_eq!(role, Some(Role::Admin));
    }

    #[test]
    fn a_stranger_resolves_to_no_role() {
        let store = InMemoryMembershipStore::new();
        let role = resolve_repo_role(
            &store,
            &UserId::new("mallory"),
            &RepoId::new("r1"),
            &Owner::User("alice".into()),
        )
        .unwrap();
        assert_eq!(role, None);
    }

    #[test]
    fn max_privilege_wins_across_sources() {
        let store = InMemoryMembershipStore::new();
        let user = UserId::new("bob");
        let repo = RepoId::new("r1");
        let org = "acme";

        // Direct Write grant + a Maintain grant via a team → Maintain wins.
        store
            .grant_repo_role(Subject::User(user.clone()), repo.clone(), Role::Write)
            .unwrap();
        let team = TeamId::new("dev");
        store
            .create_team(team.clone(), planeter_store::OrgId::new(org))
            .unwrap();
        store.add_team_member(user.clone(), team.clone()).unwrap();
        store
            .grant_repo_role(Subject::Team(team), repo.clone(), Role::Maintain)
            .unwrap();

        let role = resolve_repo_role(&store, &user, &repo, &Owner::Org(org.into())).unwrap();
        assert_eq!(role, Some(Role::Maintain));
    }

    #[test]
    fn org_admin_is_admin_on_org_repos() {
        let store = InMemoryMembershipStore::new();
        let user = UserId::new("carol");
        let org = planeter_store::OrgId::new("acme");
        store
            .set_org_role(user.clone(), org.clone(), OrgRole::Admin)
            .unwrap();
        let role = resolve_repo_role(
            &store,
            &user,
            &RepoId::new("r1"),
            &Owner::Org("acme".into()),
        )
        .unwrap();
        assert_eq!(role, Some(Role::Admin));
    }
}
