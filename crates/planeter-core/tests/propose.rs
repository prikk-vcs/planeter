//! RFC 004 D-9 / RFC 005: `Action::Propose` — the action list, the full matrix of principals x roles x the
//! repository's `proposals` setting x visibility, the audit of a proposal, and that the setting widens nothing
//! else. (Default-deny for every action stays in `authorize`'s own property test.)

use planeter_core::{
    AccessContext, Action, Decision, DenyReason, InMemoryAuditSink, Principal, RefAttempt,
    RefProtection, RefResource, RepoResource, RepoScope, Resource, Scope, ScopeAccess,
    audit_if_sensitive, authorize,
};
use planeter_store::{Owner, ProposalPolicy, RepoId, Role, UserId, Visibility};

fn ctx_role(role: Option<Role>) -> AccessContext {
    AccessContext {
        repo_role: role,
        org_role: None,
        now_unix: 1000,
    }
}

#[test]
fn the_action_list_has_ten_entries_and_propose_is_sensitive() {
    assert_eq!(Action::ALL.len(), 10);
    let mut seen = Action::ALL.to_vec();
    seen.dedup();
    assert_eq!(seen.len(), 10, "no action is listed twice");
    assert!(Action::ALL.contains(&Action::Propose));
    assert!(Action::Propose.is_sensitive(), "a proposal is audited");
    assert!(!Action::Read.is_sensitive());
}

fn proposing(vis: Visibility, policy: ProposalPolicy) -> Resource {
    Resource::Repo(RepoResource {
        repo_id: RepoId::new("r1"),
        owner: Owner::User("alice".into()),
        visibility: vis,
        proposals: policy,
    })
}

/// Every principal kind x role x setting x visibility, against the rule: `write` and above always; any
/// authenticated principal who can read the repository when the setting is `any-authenticated`; never
/// anonymous; a read-only credential never.
#[test]
fn the_propose_matrix() {
    let anon = Principal::Anonymous;
    let user = Principal::User(UserId::new("someone"));
    let ro = Principal::MachineAsUser {
        user: UserId::new("someone"),
        scope: Scope {
            access: ScopeAccess::ReadOnly,
            repos: RepoScope::All,
            expires_at: None,
        },
    };
    let rw = Principal::MachineAsUser {
        user: UserId::new("someone"),
        scope: Scope {
            access: ScopeAccess::Write,
            repos: RepoScope::All,
            expires_at: None,
        },
    };
    let roles = [
        None,
        Some(Role::Read),
        Some(Role::Write),
        Some(Role::Maintain),
        Some(Role::Admin),
    ];
    for vis in [
        Visibility::Public,
        Visibility::Internal,
        Visibility::Private,
    ] {
        for policy in [ProposalPolicy::Members, ProposalPolicy::AnyAuthenticated] {
            let res = proposing(vis, policy);
            for role in roles {
                let ctx = ctx_role(role);
                let member = role.is_some_and(|r| r >= Role::Write);
                let can_read = role.is_some() || vis != Visibility::Private;
                let open = policy == ProposalPolicy::AnyAuthenticated && can_read;
                let expect_user = member || open;
                let got = |p: &Principal| authorize(p, Action::Propose, &res, &ctx).is_allowed();
                // Anonymous: never, whatever the setting, role or visibility.
                assert!(!got(&anon), "anon {vis:?} {policy:?} {role:?}");
                assert_eq!(got(&user), expect_user, "user {vis:?} {policy:?} {role:?}");
                // A read-only credential: never.
                assert!(!got(&ro), "read-only {vis:?} {policy:?} {role:?}");
                // A write-scope credential: as the user, because the scope only caps.
                assert_eq!(
                    got(&rw),
                    expect_user,
                    "write-scope {vis:?} {policy:?} {role:?}"
                );
            }
        }
    }
}

#[test]
fn propose_applies_to_a_repository_only_and_the_setting_never_widens_other_actions() {
    let user = Principal::User(UserId::new("someone"));
    let ctx = ctx_role(None);
    // On a ref, an item or an org: no.
    let on_ref = Resource::Ref(RefResource {
        repo: RepoResource {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            visibility: Visibility::Public,
            proposals: ProposalPolicy::AnyAuthenticated,
        },
        ref_name: "heads/main".into(),
        protection: RefProtection::default(),
        attempt: RefAttempt::default(),
    });
    assert!(!authorize(&user, Action::Propose, &on_ref, &ctx).is_allowed());
    // An open `proposals` setting on a public repository gives a stranger **only** Propose.
    let open = proposing(Visibility::Public, ProposalPolicy::AnyAuthenticated);
    for action in Action::ALL {
        let allowed = authorize(&user, action, &open, &ctx).is_allowed();
        assert_eq!(
            allowed,
            matches!(action, Action::Read | Action::Propose),
            "{action:?} with an open proposals setting and no grant"
        );
    }
    // An expired credential is denied before anything else.
    let expired = Principal::MachineAsUser {
        user: UserId::new("someone"),
        scope: Scope {
            access: ScopeAccess::Full,
            repos: RepoScope::All,
            expires_at: Some(5),
        },
    };
    assert_eq!(
        authorize(&expired, Action::Propose, &open, &ctx),
        Decision::Deny(DenyReason::Expired)
    );
}

#[test]
fn a_proposal_is_audited_allowed_or_denied() {
    let sink = InMemoryAuditSink::new();
    let principal = Principal::User(UserId::new("stranger"));
    let resource = proposing(Visibility::Private, ProposalPolicy::default());
    audit_if_sensitive(
        &sink,
        &principal,
        Action::Propose,
        &resource,
        &Decision::Allow,
        1,
    );
    audit_if_sensitive(
        &sink,
        &principal,
        Action::Propose,
        &resource,
        &Decision::Deny(DenyReason::NotAuthorized),
        2,
    );
    let entries = sink.entries();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e.action == Action::Propose));
    assert_eq!(
        entries.iter().map(|e| e.allowed).collect::<Vec<_>>(),
        [true, false]
    );
}

/// A job does not propose changes: denied for a `CiJob` with any scope, either setting, any visibility.
#[test]
fn a_ci_job_never_proposes() {
    for access in [ScopeAccess::ReadOnly, ScopeAccess::Write, ScopeAccess::Full] {
        let job = Principal::CiJob {
            scope: Scope {
                access,
                repos: RepoScope::All,
                expires_at: None,
            },
        };
        for vis in [
            Visibility::Public,
            Visibility::Internal,
            Visibility::Private,
        ] {
            for policy in [ProposalPolicy::Members, ProposalPolicy::AnyAuthenticated] {
                let res = proposing(vis, policy);
                for role in [None, Some(Role::Read), Some(Role::Write), Some(Role::Admin)] {
                    assert!(
                        !authorize(&job, Action::Propose, &res, &ctx_role(role)).is_allowed(),
                        "a CI job ({access:?}) must not propose: {vis:?} {policy:?} {role:?}"
                    );
                }
            }
        }
        // It still pushes and reads as before (this is not a general restriction).
        let res = proposing(Visibility::Public, ProposalPolicy::Members);
        assert!(authorize(&job, Action::Read, &res, &ctx_role(None)).is_allowed());
    }
}
