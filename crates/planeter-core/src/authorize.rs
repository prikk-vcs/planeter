//! `authorize()` — the single, pure, default-deny access decision (RFC 002 D-1…D-8).
//!
//! Every repository-touching surface (web, API, transport, CI) routes its decision through this one
//! function; there is no second authorization path (LAY-2 / INV-4). It is a **pure function** of the
//! principal, the action, the resource, and the resolved membership/credential state passed in
//! ([`AccessContext`]) — it performs no I/O. Resolution of raw membership into an effective role lives
//! in [`crate::permission`]; a surface resolves, then calls `authorize`.
//!
//! Two invariants shape the design:
//! - **Default-deny (INV-4):** an unmatched principal is denied. Only an explicit grant, or an explicit
//!   visibility rule, opens access.
//! - **No existence leak (T-7):** a denied private resource returns the same [`Decision`] as a denied
//!   nonexistent one — the [`DenyReason`] for a visibility/role denial is the coarse `NotAuthorized`,
//!   never anything that distinguishes "exists but forbidden" from "does not exist".
//!
//! `authorize()` never signs or seals (SEC-1 / INV-2): it gates *whether* an operation may proceed;
//! prikk decides whether the result verifies, and a seal is the maintainer's own client-side signature.

use planeter_store::{OrgRole, Owner, RepoId, Role, UserId, Visibility};

/// A principal is the authenticated (or anonymous) party a decision is made for. `planeter-auth`
/// produces it; `authorize()` sees only this (RFC 002 D-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The unauthenticated public.
    Anonymous,
    /// A human account acting directly (an interactive web/API session).
    User(UserId),
    /// A machine acting as a user via a scoped credential (PAT/OAuth/SSH/deploy key). Carries the
    /// **intersection** of the user's permissions and the credential's scope (D-7).
    MachineAsUser { user: UserId, scope: Scope },
    /// An ephemeral, job-scoped CI principal (minted by RFC 006). **Never a signing identity** (D-2): it
    /// is capped below merge-seal regardless of scope.
    CiJob { scope: Scope },
}

impl Principal {
    /// The acting user, if the principal is tied to one.
    pub fn user(&self) -> Option<&UserId> {
        match self {
            Principal::User(u) | Principal::MachineAsUser { user: u, .. } => Some(u),
            Principal::Anonymous | Principal::CiJob { .. } => None,
        }
    }

    /// The credential scope, if the principal carries one.
    pub fn scope(&self) -> Option<&Scope> {
        match self {
            Principal::MachineAsUser { scope, .. } | Principal::CiJob { scope } => Some(scope),
            Principal::Anonymous | Principal::User(_) => None,
        }
    }

    /// Whether the principal is authenticated (anything but [`Principal::Anonymous`]).
    pub fn is_authenticated(&self) -> bool {
        !matches!(self, Principal::Anonymous)
    }
}

/// The access ceiling a credential carries (D-7). Independent of, and intersected with, the user's own
/// permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeAccess {
    /// Read-class actions only (browse/clone/pull, comment).
    ReadOnly,
    /// Contribution: everything except merge-seal and administration.
    Write,
    /// The user's full permissions (still bounded by the user's actual role).
    Full,
}

impl ScopeAccess {
    /// Whether this access class permits `action` at all (before the user's role is even considered).
    fn permits(self, action: Action) -> bool {
        match self {
            ScopeAccess::ReadOnly => matches!(action, Action::Read | Action::Comment),
            ScopeAccess::Write => !matches!(
                action,
                Action::MergeSeal | Action::RepoAdmin | Action::OrgAdmin
            ),
            ScopeAccess::Full => true,
        }
    }
}

/// Which repositories a credential may touch (D-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoScope {
    /// Every repository the user can reach.
    All,
    /// Only these repositories (e.g. a deploy key scoped to one repo).
    Only(Vec<RepoId>),
}

/// A credential's scope: its access ceiling, its repository restriction, and an optional expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub access: ScopeAccess,
    pub repos: RepoScope,
    /// Unix seconds after which the credential is invalid, if it expires.
    pub expires_at: Option<u64>,
}

impl Scope {
    pub fn is_expired(&self, now_unix: u64) -> bool {
        self.expires_at.is_some_and(|exp| now_unix >= exp)
    }

    pub fn permits_repo(&self, repo_id: &RepoId) -> bool {
        match &self.repos {
            RepoScope::All => true,
            RepoScope::Only(set) => set.contains(repo_id),
        }
    }
}

/// The verbs a forge grants (RFC 002 D-3). Small and explicit — a new surface adds an action
/// deliberately, never a wildcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Read repository/ref contents (browse/clone/pull).
    Read,
    /// Push author-signed patches to a repo/ref (keyless — RFC 004).
    Push,
    /// Merge-seal a change on a repo/ref (the seal itself is the maintainer's client-side signature).
    MergeSeal,
    /// Administer a repository (settings, rename/transfer, delete, grants).
    RepoAdmin,
    /// Administer an organization.
    OrgAdmin,
    /// Create a repository under an org.
    CreateRepo,
    /// Comment on an issue/change.
    Comment,
    /// Close an issue/change.
    Close,
    /// Review a change.
    Review,
}

impl Action {
    /// Every defined action — used by the default-deny property test.
    pub const ALL: [Action; 9] = [
        Action::Read,
        Action::Push,
        Action::MergeSeal,
        Action::RepoAdmin,
        Action::OrgAdmin,
        Action::CreateRepo,
        Action::Comment,
        Action::Close,
        Action::Review,
    ];

    /// Sensitive actions whose decisions are audited (D-9).
    pub fn is_sensitive(self) -> bool {
        matches!(
            self,
            Action::Push | Action::MergeSeal | Action::RepoAdmin | Action::OrgAdmin
        )
    }
}

/// A repository, carrying the visibility the read gate depends on (D-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoResource {
    pub repo_id: RepoId,
    pub owner: Owner,
    pub visibility: Visibility,
}

/// Per-ref protection state (D-6): the rules `authorize()` evaluates for push/merge-seal on a ref.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefProtection {
    /// A push may not force-replace (non-fast-forward) this ref.
    pub no_force_replace: bool,
    /// Required review approvals before a merge-seal.
    pub required_approvals: u32,
    /// Required checks must be green before a merge-seal.
    pub required_checks: bool,
    /// If set, only these users may merge-seal this ref.
    pub restricted_sealers: Option<Vec<UserId>>,
}

/// Facts about the specific attempt on a ref, supplied by the calling surface (transport/review).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefAttempt {
    /// This push would force-replace (non-fast-forward) the ref.
    pub force_replace: bool,
    /// Review approvals present (for a merge-seal).
    pub approvals: u32,
    /// Required checks are green (for a merge-seal).
    pub checks_passed: bool,
}

/// A ref within a repository, with its protection and the current attempt (D-6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefResource {
    pub repo: RepoResource,
    pub ref_name: String,
    pub protection: RefProtection,
    pub attempt: RefAttempt,
}

/// An organization resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgResource {
    pub owner: Owner,
}

/// An issue/change on a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemResource {
    pub repo: RepoResource,
}

/// The thing an [`Action`] targets (RFC 002 D-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    Org(OrgResource),
    Repo(RepoResource),
    Ref(RefResource),
    Item(ItemResource),
}

impl Resource {
    /// The repository id this resource belongs to, for the credential repo-scope check (`None` for an
    /// org resource).
    fn repo_id(&self) -> Option<&RepoId> {
        match self {
            Resource::Org(_) => None,
            Resource::Repo(r) => Some(&r.repo_id),
            Resource::Ref(r) => Some(&r.repo.repo_id),
            Resource::Item(i) => Some(&i.repo.repo_id),
        }
    }
}

/// Why access was denied. Deliberately coarse for visibility/role denials so a private resource is
/// indistinguishable from a nonexistent one (T-7); specific only where the principal already has
/// visibility (protected-ref, credential scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// No grant, wrong resource for the action, or a hidden resource. Existence-neutral (T-7).
    NotAuthorized,
    /// A protected-ref rule blocked the operation (the principal already has repo visibility).
    ProtectedRef(&'static str),
    /// The credential's scope does not cover this action or repository.
    CredentialScope,
    /// The credential has expired.
    Expired,
}

/// The outcome of an authorization decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(DenyReason),
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// The resolved state `authorize()` reads (D-1: "reads state passed in"). The caller computes
/// `repo_role`/`org_role` for the principal's user via [`crate::permission`], and supplies `now_unix`
/// for credential-expiry checks.
#[derive(Debug, Clone, Default)]
pub struct AccessContext {
    /// The principal's user's effective role on the target repository (None if none).
    pub repo_role: Option<Role>,
    /// The principal's user's org role on the target org (None if none).
    pub org_role: Option<OrgRole>,
    /// Current Unix seconds (for expiry checks).
    pub now_unix: u64,
}

/// The single access decision (RFC 002 D-1). Pure and default-deny.
pub fn authorize(
    principal: &Principal,
    action: Action,
    resource: &Resource,
    ctx: &AccessContext,
) -> Decision {
    // Credential gates first (they fail without revealing anything about the resource).
    if let Some(scope) = principal.scope() {
        if scope.is_expired(ctx.now_unix) {
            return Decision::Deny(DenyReason::Expired);
        }
        if let Some(repo_id) = resource.repo_id()
            && !scope.permits_repo(repo_id)
        {
            return Decision::Deny(DenyReason::CredentialScope);
        }
        if !scope.access.permits(action) {
            return Decision::Deny(DenyReason::CredentialScope);
        }
    }

    match resource {
        Resource::Org(_) => authorize_org(action, ctx),
        Resource::Repo(repo) => authorize_repo(principal, action, repo, ctx),
        Resource::Ref(rf) => authorize_ref(principal, action, rf, ctx),
        Resource::Item(item) => authorize_item(principal, action, &item.repo, ctx),
    }
}

/// The principal's effective base role on `repo` (before the scope access-class cap, which is applied
/// in [`authorize`]). Anonymous has none; a CI job derives a capped role from its scope.
fn effective_repo_role(principal: &Principal, ctx: &AccessContext) -> Option<Role> {
    match principal {
        Principal::Anonymous => None,
        Principal::User(_) | Principal::MachineAsUser { .. } => ctx.repo_role,
        // A CI job holds no membership; its capability is its scope, capped below merge-seal (D-2).
        Principal::CiJob { scope } => match scope.access {
            ScopeAccess::ReadOnly => Some(Role::Read),
            ScopeAccess::Write | ScopeAccess::Full => Some(Role::Write),
        },
    }
}

/// Whether the principal may *read* a repo of this visibility, absent an explicit role (D-5).
fn visibility_allows_read(principal: &Principal, visibility: Visibility) -> bool {
    match visibility {
        Visibility::Public => true,
        Visibility::Internal => principal.is_authenticated(),
        Visibility::Private => false,
    }
}

fn require(role: Option<Role>, min: Role) -> Decision {
    if role.is_some_and(|r| r >= min) {
        Decision::Allow
    } else {
        Decision::Deny(DenyReason::NotAuthorized)
    }
}

fn authorize_repo(
    principal: &Principal,
    action: Action,
    repo: &RepoResource,
    ctx: &AccessContext,
) -> Decision {
    let role = effective_repo_role(principal, ctx);
    match action {
        Action::Read => {
            if role.is_some() || visibility_allows_read(principal, repo.visibility) {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotAuthorized)
            }
        }
        Action::Push => require(role, Role::Write),
        Action::MergeSeal => require(role, Role::Maintain),
        Action::RepoAdmin => require(role, Role::Admin),
        // Org/item actions do not apply to a repo resource.
        Action::OrgAdmin
        | Action::CreateRepo
        | Action::Comment
        | Action::Close
        | Action::Review => Decision::Deny(DenyReason::NotAuthorized),
    }
}

fn authorize_ref(
    principal: &Principal,
    action: Action,
    rf: &RefResource,
    ctx: &AccessContext,
) -> Decision {
    let role = effective_repo_role(principal, ctx);
    match action {
        Action::Read => authorize_repo(principal, Action::Read, &rf.repo, ctx),
        Action::Push => {
            if !role.is_some_and(|r| r >= Role::Write) {
                return Decision::Deny(DenyReason::NotAuthorized);
            }
            if rf.protection.no_force_replace && rf.attempt.force_replace {
                return Decision::Deny(DenyReason::ProtectedRef(
                    "force-replace of a protected ref",
                ));
            }
            Decision::Allow
        }
        Action::MergeSeal => {
            if !role.is_some_and(|r| r >= Role::Maintain) {
                return Decision::Deny(DenyReason::NotAuthorized);
            }
            if rf.attempt.approvals < rf.protection.required_approvals {
                return Decision::Deny(DenyReason::ProtectedRef("insufficient review approvals"));
            }
            if rf.protection.required_checks && !rf.attempt.checks_passed {
                return Decision::Deny(DenyReason::ProtectedRef("required checks not passed"));
            }
            if let Some(sealers) = &rf.protection.restricted_sealers {
                let allowed = principal.user().is_some_and(|u| sealers.contains(u));
                if !allowed {
                    return Decision::Deny(DenyReason::ProtectedRef(
                        "not in the restricted merge-seal set",
                    ));
                }
            }
            Decision::Allow
        }
        _ => Decision::Deny(DenyReason::NotAuthorized),
    }
}

fn authorize_org(action: Action, ctx: &AccessContext) -> Decision {
    match action {
        Action::OrgAdmin => {
            if ctx.org_role == Some(OrgRole::Admin) {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotAuthorized)
            }
        }
        // Any org member (or admin) may create a repository under the org.
        Action::CreateRepo => {
            if ctx.org_role.is_some() {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotAuthorized)
            }
        }
        _ => Decision::Deny(DenyReason::NotAuthorized),
    }
}

fn authorize_item(
    principal: &Principal,
    action: Action,
    repo: &RepoResource,
    ctx: &AccessContext,
) -> Decision {
    let role = effective_repo_role(principal, ctx);
    let can_read = role.is_some() || visibility_allows_read(principal, repo.visibility);
    match action {
        // Commenting needs an authenticated principal who can read the repo.
        Action::Comment => {
            if principal.is_authenticated() && can_read {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotAuthorized)
            }
        }
        Action::Close | Action::Review => require(role, Role::Write),
        _ => Decision::Deny(DenyReason::NotAuthorized),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(vis: Visibility) -> RepoResource {
        RepoResource {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            visibility: vis,
        }
    }

    fn ctx_role(role: Option<Role>) -> AccessContext {
        AccessContext {
            repo_role: role,
            org_role: None,
            now_unix: 1000,
        }
    }

    #[test]
    fn default_deny_for_every_action_on_a_private_resource_with_no_grant() {
        // The property test the RFC asks for: an ungranted principal is denied every action.
        let user = Principal::User(UserId::new("stranger"));
        let ctx = ctx_role(None);
        let resources = [
            Resource::Repo(repo(Visibility::Private)),
            Resource::Ref(RefResource {
                repo: repo(Visibility::Private),
                ref_name: "heads/main".into(),
                protection: RefProtection::default(),
                attempt: RefAttempt::default(),
            }),
            Resource::Org(OrgResource {
                owner: Owner::Org("acme".into()),
            }),
            Resource::Item(ItemResource {
                repo: repo(Visibility::Private),
            }),
        ];
        for resource in &resources {
            for action in Action::ALL {
                assert_eq!(
                    authorize(&user, action, resource, &ctx),
                    Decision::Deny(DenyReason::NotAuthorized),
                    "expected default-deny for {action:?} on {resource:?}"
                );
            }
        }
    }

    #[test]
    fn private_denied_is_indistinguishable_from_nonexistent() {
        // T-7: whether the private repo exists or not, the caller shapes both as a private RepoResource
        // with no grant; the decision — value AND reason — is identical.
        let anon = Principal::Anonymous;
        let ctx = ctx_role(None);
        let existent = authorize(
            &anon,
            Action::Read,
            &Resource::Repo(repo(Visibility::Private)),
            &ctx,
        );
        let nonexistent = authorize(
            &anon,
            Action::Read,
            &Resource::Repo(repo(Visibility::Private)),
            &ctx,
        );
        assert_eq!(existent, nonexistent);
        assert_eq!(existent, Decision::Deny(DenyReason::NotAuthorized));
    }

    #[test]
    fn visibility_read_gate() {
        let anon = Principal::Anonymous;
        let user = Principal::User(UserId::new("someone"));
        let ctx = ctx_role(None);

        // public: anyone, even anonymous
        assert!(
            authorize(
                &anon,
                Action::Read,
                &Resource::Repo(repo(Visibility::Public)),
                &ctx
            )
            .is_allowed()
        );
        // internal: any authenticated user, not anonymous
        assert!(
            authorize(
                &user,
                Action::Read,
                &Resource::Repo(repo(Visibility::Internal)),
                &ctx
            )
            .is_allowed()
        );
        assert!(
            !authorize(
                &anon,
                Action::Read,
                &Resource::Repo(repo(Visibility::Internal)),
                &ctx
            )
            .is_allowed()
        );
        // private: neither, without a grant
        assert!(
            !authorize(
                &user,
                Action::Read,
                &Resource::Repo(repo(Visibility::Private)),
                &ctx
            )
            .is_allowed()
        );
    }

    #[test]
    fn role_thresholds_for_write_actions() {
        let user = Principal::User(UserId::new("dev"));
        let private = Resource::Repo(repo(Visibility::Private));

        assert!(
            authorize(&user, Action::Push, &private, &ctx_role(Some(Role::Write))).is_allowed()
        );
        assert!(
            !authorize(&user, Action::Push, &private, &ctx_role(Some(Role::Read))).is_allowed()
        );
        assert!(
            authorize(
                &user,
                Action::MergeSeal,
                &private,
                &ctx_role(Some(Role::Maintain))
            )
            .is_allowed()
        );
        assert!(
            !authorize(
                &user,
                Action::MergeSeal,
                &private,
                &ctx_role(Some(Role::Write))
            )
            .is_allowed()
        );
        assert!(
            authorize(
                &user,
                Action::RepoAdmin,
                &private,
                &ctx_role(Some(Role::Admin))
            )
            .is_allowed()
        );
        assert!(
            !authorize(
                &user,
                Action::RepoAdmin,
                &private,
                &ctx_role(Some(Role::Maintain))
            )
            .is_allowed()
        );
    }

    #[test]
    fn read_only_token_is_blocked_from_writes() {
        let ro = Principal::MachineAsUser {
            user: UserId::new("dev"),
            scope: Scope {
                access: ScopeAccess::ReadOnly,
                repos: RepoScope::All,
                expires_at: None,
            },
        };
        let private = Resource::Repo(repo(Visibility::Private));
        // Even with a Maintain role, a read-only credential cannot push.
        assert_eq!(
            authorize(&ro, Action::Push, &private, &ctx_role(Some(Role::Maintain))),
            Decision::Deny(DenyReason::CredentialScope)
        );
        // ...but it can read.
        assert!(
            authorize(&ro, Action::Read, &private, &ctx_role(Some(Role::Maintain))).is_allowed()
        );
    }

    #[test]
    fn single_repo_credential_is_blocked_from_another_repo() {
        let scoped = Principal::MachineAsUser {
            user: UserId::new("dev"),
            scope: Scope {
                access: ScopeAccess::Write,
                repos: RepoScope::Only(vec![RepoId::new("r1")]),
                expires_at: None,
            },
        };
        // r1 is in scope → push allowed with Write role.
        let r1 = Resource::Repo(RepoResource {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            visibility: Visibility::Private,
        });
        assert!(authorize(&scoped, Action::Push, &r1, &ctx_role(Some(Role::Write))).is_allowed());
        // r2 is out of scope → denied even with a Write role.
        let r2 = Resource::Repo(RepoResource {
            repo_id: RepoId::new("r2"),
            owner: Owner::User("alice".into()),
            visibility: Visibility::Private,
        });
        assert_eq!(
            authorize(&scoped, Action::Push, &r2, &ctx_role(Some(Role::Write))),
            Decision::Deny(DenyReason::CredentialScope)
        );
    }

    #[test]
    fn expired_credential_is_denied() {
        let expired = Principal::MachineAsUser {
            user: UserId::new("dev"),
            scope: Scope {
                access: ScopeAccess::Full,
                repos: RepoScope::All,
                expires_at: Some(500),
            },
        };
        let private = Resource::Repo(repo(Visibility::Private));
        // now_unix (1000) >= expiry (500) → Expired, before any resource check.
        assert_eq!(
            authorize(
                &expired,
                Action::Read,
                &private,
                &ctx_role(Some(Role::Admin))
            ),
            Decision::Deny(DenyReason::Expired)
        );
    }

    fn protected_ref(protection: RefProtection, attempt: RefAttempt) -> Resource {
        Resource::Ref(RefResource {
            repo: repo(Visibility::Private),
            ref_name: "heads/main".into(),
            protection,
            attempt,
        })
    }

    #[test]
    fn protected_ref_denies_force_replace() {
        let maint = Principal::User(UserId::new("m"));
        let resource = protected_ref(
            RefProtection {
                no_force_replace: true,
                ..Default::default()
            },
            RefAttempt {
                force_replace: true,
                ..Default::default()
            },
        );
        assert_eq!(
            authorize(
                &maint,
                Action::Push,
                &resource,
                &ctx_role(Some(Role::Maintain))
            ),
            Decision::Deny(DenyReason::ProtectedRef("force-replace of a protected ref"))
        );
        // A normal (fast-forward) push by the same maintainer is allowed.
        let ff = protected_ref(
            RefProtection {
                no_force_replace: true,
                ..Default::default()
            },
            RefAttempt::default(),
        );
        assert!(authorize(&maint, Action::Push, &ff, &ctx_role(Some(Role::Maintain))).is_allowed());
    }

    #[test]
    fn protected_ref_denies_merge_lacking_approvals_or_checks() {
        let maint = Principal::User(UserId::new("m"));
        let protection = RefProtection {
            required_approvals: 2,
            required_checks: true,
            ..Default::default()
        };
        // Missing approvals.
        let short = protected_ref(
            protection.clone(),
            RefAttempt {
                approvals: 1,
                checks_passed: true,
                ..Default::default()
            },
        );
        assert_eq!(
            authorize(
                &maint,
                Action::MergeSeal,
                &short,
                &ctx_role(Some(Role::Maintain))
            ),
            Decision::Deny(DenyReason::ProtectedRef("insufficient review approvals"))
        );
        // Approvals met but checks red.
        let red = protected_ref(
            protection.clone(),
            RefAttempt {
                approvals: 2,
                checks_passed: false,
                ..Default::default()
            },
        );
        assert_eq!(
            authorize(
                &maint,
                Action::MergeSeal,
                &red,
                &ctx_role(Some(Role::Maintain))
            ),
            Decision::Deny(DenyReason::ProtectedRef("required checks not passed"))
        );
        // All satisfied → allowed.
        let ok = protected_ref(
            protection,
            RefAttempt {
                approvals: 2,
                checks_passed: true,
                ..Default::default()
            },
        );
        assert!(
            authorize(
                &maint,
                Action::MergeSeal,
                &ok,
                &ctx_role(Some(Role::Maintain))
            )
            .is_allowed()
        );
    }

    #[test]
    fn restricted_sealer_set_is_enforced() {
        let protection = RefProtection {
            restricted_sealers: Some(vec![UserId::new("release-manager")]),
            ..Default::default()
        };
        let ok_attempt = RefAttempt {
            approvals: 0,
            checks_passed: true,
            ..Default::default()
        };
        let resource = protected_ref(protection, ok_attempt);
        // A maintainer not in the set is denied.
        let other = Principal::User(UserId::new("dev"));
        assert_eq!(
            authorize(
                &other,
                Action::MergeSeal,
                &resource,
                &ctx_role(Some(Role::Maintain))
            ),
            Decision::Deny(DenyReason::ProtectedRef(
                "not in the restricted merge-seal set"
            ))
        );
        // The designated sealer is allowed.
        let rm = Principal::User(UserId::new("release-manager"));
        assert!(
            authorize(
                &rm,
                Action::MergeSeal,
                &resource,
                &ctx_role(Some(Role::Maintain))
            )
            .is_allowed()
        );
    }

    #[test]
    fn ci_job_can_read_but_never_merge_seal() {
        let ci = Principal::CiJob {
            scope: Scope {
                access: ScopeAccess::Full,
                repos: RepoScope::All,
                expires_at: None,
            },
        };
        let private = Resource::Repo(repo(Visibility::Private));
        assert!(authorize(&ci, Action::Read, &private, &ctx_role(None)).is_allowed());
        // Even a Full CI scope is capped below merge-seal (D-2: never a signing identity).
        assert!(!authorize(&ci, Action::MergeSeal, &private, &ctx_role(None)).is_allowed());
    }

    #[test]
    fn org_admin_and_create_repo() {
        let org = Resource::Org(OrgResource {
            owner: Owner::Org("acme".into()),
        });
        let member_ctx = AccessContext {
            repo_role: None,
            org_role: Some(OrgRole::Member),
            now_unix: 0,
        };
        let admin_ctx = AccessContext {
            repo_role: None,
            org_role: Some(OrgRole::Admin),
            now_unix: 0,
        };
        let user = Principal::User(UserId::new("u"));
        // A member may create a repo but not administer the org.
        assert!(authorize(&user, Action::CreateRepo, &org, &member_ctx).is_allowed());
        assert!(!authorize(&user, Action::OrgAdmin, &org, &member_ctx).is_allowed());
        // An admin may do both.
        assert!(authorize(&user, Action::OrgAdmin, &org, &admin_ctx).is_allowed());
    }
}
