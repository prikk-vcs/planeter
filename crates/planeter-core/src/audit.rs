//! The audit seam (RFC 002 D-9 / SEC-5 / T-12): sensitive authorization outcomes are recorded to an
//! **off-box** sink — "who did what". A0 reserved the seam; this defines the record and the sink trait
//! and provides the null and in-memory sinks. The concrete off-box shipper (append-only, tamper-evident
//! store) is a deployment/ops follow-on behind [`AuditSink`], not a decision-path concern.
//!
//! The record carries the *decision*, never a secret: a principal is summarized by identity, never by
//! its token/key material.

use crate::authorize::{Action, Decision, Principal, Resource};

/// One audit entry: the decision made on a sensitive action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// A non-secret summary of the acting principal (identity only).
    pub principal: String,
    pub action: Action,
    /// A non-secret summary of the resource acted on.
    pub resource: String,
    pub allowed: bool,
    /// Unix seconds the decision was made.
    pub at_unix: u64,
}

impl AuditRecord {
    /// Build a record from a decision. Summarizes the principal/resource without leaking credentials.
    pub fn new(
        principal: &Principal,
        action: Action,
        resource: &Resource,
        decision: &Decision,
        at_unix: u64,
    ) -> Self {
        Self {
            principal: summarize_principal(principal),
            action,
            resource: summarize_resource(resource),
            allowed: decision.is_allowed(),
            at_unix,
        }
    }
}

fn summarize_principal(p: &Principal) -> String {
    match p {
        Principal::Anonymous => "anonymous".to_owned(),
        Principal::User(u) => format!("user:{}", u.as_str()),
        Principal::MachineAsUser { user, .. } => format!("machine-as:{}", user.as_str()),
        Principal::CiJob { .. } => "ci-job".to_owned(),
    }
}

fn summarize_resource(r: &Resource) -> String {
    match r {
        Resource::Org(o) => format!("org:{}", o.owner.handle()),
        Resource::Repo(repo) => format!("repo:{}/{}", repo.owner.handle(), repo.repo_id),
        Resource::Ref(rf) => {
            format!(
                "ref:{}/{}@{}",
                rf.repo.owner.handle(),
                rf.repo.repo_id,
                rf.ref_name
            )
        }
        Resource::Item(item) => format!("item:{}/{}", item.repo.owner.handle(), item.repo.repo_id),
    }
}

/// Where audit records go. Deployments install an off-box shipper; tests use [`InMemoryAuditSink`].
pub trait AuditSink: Send + Sync {
    fn record(&self, entry: AuditRecord);
}

/// Records only sensitive decisions (D-9): a convenience wrapper every surface can call after a
/// decision, letting the sink decide durability. Non-sensitive reads are not audited.
pub fn audit_if_sensitive(
    sink: &dyn AuditSink,
    principal: &Principal,
    action: Action,
    resource: &Resource,
    decision: &Decision,
    now_unix: u64,
) {
    if action.is_sensitive() {
        sink.record(AuditRecord::new(
            principal, action, resource, decision, now_unix,
        ));
    }
}

/// A sink that drops records (for contexts where auditing is disabled — never a default in production).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullAuditSink;

impl AuditSink for NullAuditSink {
    fn record(&self, _entry: AuditRecord) {}
}

/// An in-memory sink for tests.
#[derive(Debug, Default)]
pub struct InMemoryAuditSink {
    entries: std::sync::Mutex<Vec<AuditRecord>>,
}

impl InMemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }
    /// A snapshot of recorded entries.
    pub fn entries(&self) -> Vec<AuditRecord> {
        self.entries.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, entry: AuditRecord) {
        if let Ok(mut g) = self.entries.lock() {
            g.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorize::{DenyReason, RepoResource};
    use planeter_store::{Owner, RepoId, UserId, Visibility};

    fn repo_resource() -> Resource {
        Resource::Repo(RepoResource {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            visibility: Visibility::Private,
        })
    }

    #[test]
    fn sensitive_decisions_are_recorded_reads_are_not() {
        let sink = InMemoryAuditSink::new();
        let principal = Principal::User(UserId::new("alice"));
        let resource = repo_resource();

        // A push (sensitive) is recorded.
        audit_if_sensitive(
            &sink,
            &principal,
            Action::Push,
            &resource,
            &Decision::Deny(DenyReason::NotAuthorized),
            42,
        );
        // A read (not sensitive) is not.
        audit_if_sensitive(
            &sink,
            &principal,
            Action::Read,
            &resource,
            &Decision::Allow,
            43,
        );

        let entries = sink.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, Action::Push);
        assert!(!entries[0].allowed);
        assert_eq!(entries[0].principal, "user:alice");
        assert!(entries[0].resource.starts_with("repo:alice/"));
    }
}
