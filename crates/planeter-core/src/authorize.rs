//! The `authorize()` seam (RFC 001 D-6, threat model LAY-2/INV-4).
//!
//! Every future surface that touches a repository — read path (RFC 003), transport (RFC 004),
//! review, CI, registry — must route its access decision through **one** [`Authorizer`]. A0 ships
//! only the *seam*: the trait, the request/decision types, and a **default-deny** stub. RFC 002 fills
//! the contract (accounts, org membership, roles, visibility). Shipping the seam now — with deny as
//! the default — is what makes "closed unless a rule opens it" structural from the first commit rather
//! than a policy bolted on later.

use planeter_store::{Owner, RepoId, Visibility};

/// Who is making a request. A0 models only the coarse shape; RFC 002 defines accounts and sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// An unauthenticated caller (the public).
    Anonymous,
    /// An authenticated account, referenced by its handle. (The account model is RFC 002.)
    Account(String),
}

/// What a caller wants to do. Deliberately small for A0; RFC 002/003/004 extend it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// Read repository contents (browse/clone/pull).
    Read,
    /// Contribute changes (push author-signed patches — keyless, RFC 004).
    Contribute,
    /// Administer the repository (settings, rename/transfer, delete).
    Administer,
}

/// The resource an [`Action`] targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// A specific repository, with the visibility the decision may depend on.
    Repository {
        repo_id: RepoId,
        owner: Owner,
        visibility: Visibility,
    },
    /// An owner-scoped resource (e.g. creating a repo under an owner).
    Owner(Owner),
}

/// A complete access request: `actor` wants to perform `action` on `resource`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRequest {
    pub actor: Actor,
    pub action: Action,
    pub resource: Resource,
}

/// The outcome of an authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

impl Decision {
    pub fn is_allowed(self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// The single access-control chokepoint. One implementation is installed per deployment; every
/// repository-touching surface calls [`Authorizer::authorize`] before acting.
pub trait Authorizer: Send + Sync {
    fn authorize(&self, request: &AccessRequest) -> Decision;
}

/// The A0 default: **deny everything**. It is the safe placeholder until RFC 002 installs the real
/// policy — a forge that shipped with this authorizer would serve nothing, which is the correct
/// failure direction (LAY-2). Never keep this in a deployment that is meant to serve.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAll;

impl Authorizer for DenyAll {
    fn authorize(&self, _request: &AccessRequest) -> Decision {
        Decision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_authorizer_denies_even_public_reads() {
        let auth = DenyAll;
        let req = AccessRequest {
            actor: Actor::Anonymous,
            action: Action::Read,
            resource: Resource::Repository {
                repo_id: RepoId::new("r1"),
                owner: Owner::User("alice".into()),
                visibility: Visibility::Public,
            },
        };
        assert_eq!(auth.authorize(&req), Decision::Deny);
        assert!(!auth.authorize(&req).is_allowed());
    }
}
