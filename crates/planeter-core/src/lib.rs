#![forbid(unsafe_code)]
//! `planeter-core` — planeter core (domain + authz) layer. See RFC 001 (foundations).
//!
//! The forge domain above anonymous prikk: the [`hosting`] model (create/open a hosted repository over
//! the sandboxed prikk boundary and the [`layout`] on-disk placement), the **authorization decision**
//! ([`authorize`] — the pure, default-deny access function, RFC 002 — with [`permission`] resolution and
//! the [`audit`] seam), and the [`egress`] deny-by-default SSRF chokepoint (RFC 004). The reserved
//! off-by-default `forge-seal` write feature (LAY-4/ENF-2) rounds out the security seams.
//!
//! ## The `forge-seal` feature (INV-2 / LAY-4 / ENF-2)
//!
//! planeter is a **keyless forge**: it holds no history-signing key and never seals history itself
//! (RFC 001 §0a / OQ-1(a)). Any code that could sign history lives behind the `forge-seal` cargo
//! feature, which is **off by default**, so the shipped binary contains no forge-signer symbol. The
//! ENF-2 CI check (`tools/enf2-symbol-check`) greps the default release build to prove that symbol's
//! absence; it passes trivially today (there is nothing to find) and stays as the guard that keeps
//! INV-2 true as write code arrives. The feature is reserved here; **no seal code exists yet.**

pub mod audit;
pub mod authorize;
pub mod egress;
pub mod hosting;
pub mod layout;
pub mod permission;
pub mod read;

pub use audit::{AuditRecord, AuditSink, InMemoryAuditSink, NullAuditSink, audit_if_sensitive};
pub use authorize::{
    AccessContext, Action, Decision, DenyReason, ItemResource, OrgResource, Principal, RefAttempt,
    RefProtection, RefResource, RepoResource, RepoScope, Resource, Scope, ScopeAccess, authorize,
};
pub use egress::{DenyAllEgress, EgressError, EgressGuard};
pub use hosting::{HostingError, HostingService};
pub use layout::{DefaultRepoIdAllocator, RepoIdAllocator, RepoLayout};
pub use permission::resolve_repo_role;
pub use read::{
    Assurance, ChangeView, FileContentView, FileView, Freshness, HistoryView, ReadError,
    ReadService, RefsView, Served, VerifyView,
};
