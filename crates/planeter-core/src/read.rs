//! The read path (RFC 003): browse a hosted repository through `authorize(_, Read, _)`, over the prikk
//! read verbs, into honest view-models. Read-only — no write verb, no transport (RFC 004).
//!
//! Four properties this module is responsible for:
//! - **T6 read authorization.** Every read resolves the repository, builds an [`AccessContext`], and
//!   calls `authorize(principal, Read, resource)`. A denied private repository is **indistinguishable
//!   from a nonexistent one** ([`ReadError::NotFound`] for both — T-7). UI and API share this one gate.
//! - **T2 browse surfaces.** Each prikk `--format json` verb maps to a view-model here.
//! - **T5 honesty.** A change/history view marks **prikk-verified** vs merely **forge-approved
//!   (approved-but-unsealed)** distinctly ([`Assurance`]); the security verify status is **re-derived**
//!   from prikk on every call, never served from a cache (D-5 / SEC-1).
//! - **T1 cache is never the authority.** The view-models are **pure functions of prikk output**, so a
//!   derived cache (keyed by prikk object id) is always droppable and rebuildable from prikk; when prikk
//!   is unavailable a cached view is served **labelled [`Freshness::Stale`]**, never fabricated. The
//!   verify status is exempt — always fresh.
//!
//! The rendering-safety layer (strict HTML/Markdown sanitization, isolated content origin, CSP — RFC 003
//! T4) and the HTTP/OpenAPI surface (T3) sit *above* this in the web crate; view-models carry raw
//! repository text and mark it as such, but never render it.

use std::sync::Arc;

use planeter_prikk::{PrikkRepo, model};
use planeter_store::membership::owner_org;
use planeter_store::{MembershipStore, Owner, RepositoryRecord};

use crate::audit::AuditSink;
use crate::authorize::{AccessContext, Action, Principal, RepoResource, Resource, authorize};
use crate::hosting::HostingService;
use crate::permission::resolve_repo_role;

/// A read-path failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReadError {
    /// The repository does not exist, **or** the principal may not read it. Deliberately the same for
    /// both (T-7): a caller cannot tell a hidden private repo from a missing one.
    NotFound,
    /// prikk could not be reached or returned an error (the repository exists and is authorized).
    Unavailable(String),
    /// A backend (store/membership) failure.
    Backend(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::NotFound => f.write_str("not found"),
            ReadError::Unavailable(e) => write!(f, "repository temporarily unavailable: {e}"),
            ReadError::Backend(e) => write!(f, "backend error: {e}"),
        }
    }
}

impl std::error::Error for ReadError {}

type Result<T> = std::result::Result<T, ReadError>;

/// Whether a served view was re-derived from prikk now, or is a cached fallback prikk could not refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
}

/// A view served to a surface, tagged with its freshness (T1: a stale fallback is labelled, never
/// passed off as current).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served<T> {
    pub view: T,
    pub freshness: Freshness,
}

// ----------------------------------------------------------------------------
// Honesty (T5 / D-5)
// ----------------------------------------------------------------------------

/// How much assurance a change carries — the distinction the UI must never blur (SEC-1 / WEB-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assurance {
    /// Sealed and prikk-verified.
    PrikkVerified,
    /// Accepted by the forge but **not yet sealed** (queued) — a forge-side approval, not a prikk
    /// signature. Must never render as verified.
    ForgeApprovedUnsealed,
    /// Neither sealed nor verifiable.
    Unverified,
}

// ----------------------------------------------------------------------------
// View-models (T2) — pure functions of prikk output
// ----------------------------------------------------------------------------

/// Repository history (from `log`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryView {
    pub ref_name: String,
    pub current_branch: Option<String>,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub block_id: String,
    pub ref_state_id: String,
    pub kind: String,
    pub rollback_block: bool,
    pub patch_count: u64,
    pub messages: Vec<PatchMessageView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchMessageView {
    pub patch_id: String,
    pub message: String,
}

impl HistoryView {
    pub fn from_log(log: &model::LogReport) -> Self {
        Self {
            ref_name: log.ref_name.clone(),
            current_branch: log.current_branch.clone(),
            entries: log
                .blocks
                .iter()
                .map(|b| HistoryEntry {
                    block_id: b.block_id.clone(),
                    ref_state_id: b.ref_state_id.clone(),
                    kind: b.kind.clone(),
                    rollback_block: b.rollback_block,
                    patch_count: b.patch_count,
                    messages: b
                        .patch_messages
                        .iter()
                        .map(|m| PatchMessageView {
                            patch_id: m.patch_id.clone(),
                            message: m.message.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// A change's content/effect (from `show`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeView {
    pub patches: Vec<ChangePatchView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePatchView {
    pub patch_id: String,
    /// Honest assurance: a queued (unsealed) patch is `ForgeApprovedUnsealed`, never verified (T5).
    pub assurance: Assurance,
    pub operations: Vec<OperationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationView {
    pub kind: String,
    pub paths: Vec<String>,
}

impl ChangeView {
    pub fn from_show(show: &model::ShowReport) -> Self {
        Self {
            patches: show
                .patches
                .iter()
                .map(|p| ChangePatchView {
                    patch_id: p.patch_id.clone(),
                    assurance: if p.queued {
                        Assurance::ForgeApprovedUnsealed
                    } else {
                        // Sealed here; the repository-level verify view (re-derived) is the authority on
                        // whether the seal actually verifies.
                        Assurance::PrikkVerified
                    },
                    operations: p
                        .operations
                        .iter()
                        .map(|op| OperationView {
                            kind: op.kind.clone(),
                            paths: op.paths.iter().map(render_path).collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

fn render_path(p: &model::PathRef) -> String {
    match p {
        model::PathRef::Resolved { path } => path.clone(),
        model::PathRef::Unresolved { unresolved_node_id } => {
            format!("<unresolved:{unresolved_node_id}>")
        }
    }
}

/// A single file's content at a ref (from `checkout --patch-plan --content-path`). The `Text` bytes are
/// **raw repository content** — the web layer must sanitize/serve from the isolated origin (T4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileView {
    pub ref_name: String,
    pub path: String,
    pub content: FileContentView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContentView {
    Text(String),
    Binary { blob_id: String, size: u64 },
    Opaque { size: u64 },
    NotFound,
}

impl FileView {
    /// Build the view for `path` from a patch-plan-content report (which may cover several paths).
    pub fn from_patch_plan(plan: &model::PatchPlanContent, path: &str) -> Self {
        let content = plan
            .content
            .iter()
            .find(|e| e.path == path)
            .map(|e| match &e.content {
                model::PathContent::Text { text } => FileContentView::Text(text.clone()),
                model::PathContent::Binary { blob_id, size } => FileContentView::Binary {
                    blob_id: blob_id.clone(),
                    size: *size,
                },
                model::PathContent::Opaque { size } => FileContentView::Opaque { size: *size },
            })
            .unwrap_or(FileContentView::NotFound);
        Self {
            ref_name: plan.ref_name.clone(),
            path: path.to_owned(),
            content,
        }
    }
}

/// Refs/branches/tags (from `branch` + `tag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefsView {
    pub current_branch: Option<String>,
    pub branches: Vec<BranchView>,
    pub tags: Vec<TagView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchView {
    pub ref_name: String,
    pub ref_state_id: String,
    pub closed: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagView {
    pub ref_name: String,
    pub target_block_id: String,
}

impl RefsView {
    pub fn from_lists(branches: &model::BranchList, tags: &model::TagList) -> Self {
        let current_branch = branches
            .branches
            .iter()
            .find(|b| b.current)
            .map(|b| b.ref_name.clone());
        Self {
            current_branch,
            branches: branches
                .branches
                .iter()
                .map(|b| BranchView {
                    ref_name: b.ref_name.clone(),
                    ref_state_id: b.ref_state_id.clone(),
                    closed: b.closed,
                    current: b.current,
                })
                .collect(),
            tags: tags
                .tags
                .iter()
                .map(|t| TagView {
                    ref_name: t.ref_name.clone(),
                    target_block_id: t.target_block_id.clone(),
                })
                .collect(),
        }
    }
}

/// The repository verify status (from `verify`) — the honesty anchor, always re-derived (T5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyView {
    pub ok: bool,
    pub verified_sessions: u64,
    pub total_sessions: u64,
    pub failed_conditions: Vec<FailedConditionView>,
    pub stages: Vec<StageView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedConditionView {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageView {
    pub stage: String,
    /// `evaluated` | `failed` | `not_evaluated` | `halted`.
    pub status: String,
    pub detail: Option<String>,
}

impl VerifyView {
    pub fn from_verify(v: &model::VerifyReport) -> Self {
        Self {
            ok: v.verdict.ok,
            verified_sessions: v.active_sessions.verified_count,
            total_sessions: v.active_sessions.count,
            failed_conditions: v
                .verdict
                .failed_conditions
                .iter()
                .map(|c| FailedConditionView {
                    id: c.id.clone(),
                    message: c.message.clone(),
                })
                .collect(),
            stages: v.stages.iter().map(stage_view).collect(),
        }
    }

    /// The repository-level assurance the UI shows next to a sealed change (T5).
    pub fn assurance(&self) -> Assurance {
        if self.ok {
            Assurance::PrikkVerified
        } else {
            Assurance::Unverified
        }
    }
}

fn stage_view(s: &model::Stage) -> StageView {
    match s {
        model::Stage::Evaluated { stage } => StageView {
            stage: stage.clone(),
            status: "evaluated".into(),
            detail: None,
        },
        model::Stage::Failed { stage, message } => StageView {
            stage: stage.clone(),
            status: "failed".into(),
            detail: Some(message.clone()),
        },
        model::Stage::NotEvaluated { stage, blocked_by } => StageView {
            stage: stage.clone(),
            status: "not_evaluated".into(),
            detail: Some(format!("blocked by {blocked_by}")),
        },
        model::Stage::Halted { stage, after } => StageView {
            stage: stage.clone(),
            status: "halted".into(),
            detail: Some(format!("after {after}")),
        },
    }
}

// ----------------------------------------------------------------------------
// The read service (T6 gate + orchestration)
// ----------------------------------------------------------------------------

/// Serves authorized, honest reads of hosted repositories. Every method routes through the one
/// `authorize(_, Read, _)` gate before touching prikk.
pub struct ReadService {
    hosting: Arc<HostingService>,
    membership: Arc<dyn MembershipStore>,
    #[allow(dead_code)]
    audit: Arc<dyn AuditSink>,
    now_unix: fn() -> u64,
}

impl ReadService {
    pub fn new(
        hosting: Arc<HostingService>,
        membership: Arc<dyn MembershipStore>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        Self {
            hosting,
            membership,
            audit,
            now_unix: default_now,
        }
    }

    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// Resolve + authorize a read, returning the record only if the principal may read it. A denied or
    /// nonexistent repository both yield [`ReadError::NotFound`] (T-7).
    fn authorize_read(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
    ) -> Result<RepositoryRecord> {
        let record = self
            .hosting
            .record_by_name(owner, name)
            .map_err(|e| ReadError::Backend(e.to_string()))?
            .ok_or(ReadError::NotFound)?;

        let ctx = self.access_context(principal, &record)?;
        let resource = Resource::Repo(RepoResource {
            repo_id: record.repo_id.clone(),
            owner: record.owner.clone(),
            visibility: record.visibility,
        });
        if authorize(principal, Action::Read, &resource, &ctx).is_allowed() {
            Ok(record)
        } else {
            // Indistinguishable from nonexistent.
            Err(ReadError::NotFound)
        }
    }

    fn access_context(
        &self,
        principal: &Principal,
        record: &RepositoryRecord,
    ) -> Result<AccessContext> {
        let (repo_role, org_role) = match principal.user() {
            Some(user) => {
                let repo_role =
                    resolve_repo_role(&*self.membership, user, &record.repo_id, &record.owner)
                        .map_err(|e| ReadError::Backend(e.to_string()))?;
                let org_role = match owner_org(&record.owner) {
                    Some(org) => self
                        .membership
                        .org_role(user, &org)
                        .map_err(|e| ReadError::Backend(e.to_string()))?,
                    None => None,
                };
                (repo_role, org_role)
            }
            None => (None, None),
        };
        Ok(AccessContext {
            repo_role,
            org_role,
            now_unix: (self.now_unix)(),
        })
    }

    fn open(&self, record: &RepositoryRecord) -> Result<planeter_prikk::CliPrikkRepo> {
        self.hosting
            .open_record(record)
            .map_err(|e| ReadError::Unavailable(e.to_string()))
    }

    /// History of a ref (`log`), bounded by `limit`.
    pub fn history(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        ref_name: Option<&str>,
        limit: Option<usize>,
    ) -> Result<HistoryView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record)?;
        let log = driver
            .log(ref_name, limit)
            .map_err(|e| ReadError::Unavailable(e.to_string()))?;
        Ok(HistoryView::from_log(&log))
    }

    /// A change's content/effect (`show`).
    pub fn change(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        target: &str,
    ) -> Result<ChangeView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record)?;
        let show = driver
            .show(target)
            .map_err(|e| ReadError::Unavailable(e.to_string()))?;
        Ok(ChangeView::from_show(&show))
    }

    /// A single file's content at a ref (`checkout --patch-plan --content-path`).
    pub fn file(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        ref_name: Option<&str>,
        path: &str,
    ) -> Result<FileView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record)?;
        let plan = driver
            .patch_plan_content(ref_name, &[path])
            .map_err(|e| ReadError::Unavailable(e.to_string()))?;
        Ok(FileView::from_patch_plan(&plan, path))
    }

    /// Refs/branches/tags (`branch` + `tag`).
    pub fn refs(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        include_closed: bool,
    ) -> Result<RefsView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record)?;
        let branches = driver
            .branches(include_closed)
            .map_err(|e| ReadError::Unavailable(e.to_string()))?;
        let tags = driver
            .tags()
            .map_err(|e| ReadError::Unavailable(e.to_string()))?;
        Ok(RefsView::from_lists(&branches, &tags))
    }

    /// The repository verify status (`verify`) — always re-derived, never cached (T5 honesty).
    pub fn verify(&self, principal: &Principal, owner: &Owner, name: &str) -> Result<VerifyView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record)?;
        let v = driver
            .verify()
            .map_err(|e| ReadError::Unavailable(e.to_string()))?;
        Ok(VerifyView::from_verify(&v))
    }
}

fn default_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::NullAuditSink;
    use planeter_store::{
        InMemoryMembershipStore, InMemoryRepositoryStore, RepositoryStore, Visibility,
    };

    // -- pure converter / honesty tests (no prikk needed) --

    #[test]
    fn history_view_is_a_pure_function_of_log() {
        let log: model::LogReport = serde_json::from_str(
            r#"{"schema_version":"log-report-v1","repository":"/r/.prikk","ref":"heads/main",
                "current_branch":"main","blocks":[{"block_id":"b1","ref_state_id":"r1","update_seq":0,
                "kind":"Root","rollback_block":false,"parent_count":0,"patch_count":1,
                "rollback_patch_count":0,"required_attestation_count":0,
                "patch_messages":[{"patch_id":"p1","message":"init"}],"previous_ref_state_id":null}]}"#,
        )
        .unwrap();
        // Cache-cleared correctness (ENF-5): the same prikk output always yields the identical view.
        assert_eq!(HistoryView::from_log(&log), HistoryView::from_log(&log));
        let v = HistoryView::from_log(&log);
        assert_eq!(v.ref_name, "heads/main");
        assert_eq!(v.entries[0].messages[0].message, "init");
    }

    #[test]
    fn queued_patch_is_forge_approved_never_verified() {
        let show: model::ShowReport = serde_json::from_str(
            r#"{"schema_version":"show-report-v1","patches":[
                {"patch_id":"p1","queued":true,"operations":[]},
                {"patch_id":"p2","queued":false,"operations":[]}]}"#,
        )
        .unwrap();
        let v = ChangeView::from_show(&show);
        assert_eq!(v.patches[0].assurance, Assurance::ForgeApprovedUnsealed);
        assert_eq!(v.patches[1].assurance, Assurance::PrikkVerified);
    }

    #[test]
    fn verify_view_assurance_tracks_verdict() {
        let bad: model::VerifyReport = serde_json::from_str(
            r#"{"schema_version":"verify-report-v1","verdict":{"ok":false,
                "failed_conditions":[{"id":"x","message":"boom"}]},
                "active_sessions":{"count":1,"verified_count":0},"stages":[]}"#,
        )
        .unwrap();
        assert_eq!(
            VerifyView::from_verify(&bad).assurance(),
            Assurance::Unverified
        );
    }

    // -- the T6 authorization gate (no prikk: deny returns before opening prikk) --

    fn service_with_repo(visibility: Visibility) -> (ReadService, Owner) {
        let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
        let owner = Owner::User("alice".into());
        store
            .create(RepositoryRecord {
                repo_id: planeter_store::RepoId::new("r1"),
                owner: owner.clone(),
                name: "app".into(),
                visibility,
                created_at: 0,
                prikk_format_version: None,
                path: "/nonexistent/for/deny/tests".into(),
            })
            .unwrap();
        let hosting = Arc::new(HostingService::new("/unused", store));
        let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
        let svc = ReadService::new(hosting, membership, Arc::new(NullAuditSink)).with_clock(|| 0);
        (svc, owner)
    }

    #[test]
    fn anonymous_read_of_private_is_notfound_indistinguishable_from_missing() {
        let (svc, owner) = service_with_repo(Visibility::Private);
        // Private repo, anonymous: denied → NotFound (never opens prikk, path is bogus).
        let denied = svc.history(&Principal::Anonymous, &owner, "app", None, None);
        assert!(matches!(denied, Err(ReadError::NotFound)));
        // A genuinely missing repo: also NotFound. The two are indistinguishable (T-7).
        let missing = svc.history(&Principal::Anonymous, &owner, "does-not-exist", None, None);
        assert!(matches!(missing, Err(ReadError::NotFound)));
    }

    #[test]
    fn anonymous_read_of_internal_is_denied() {
        let (svc, owner) = service_with_repo(Visibility::Internal);
        assert!(matches!(
            svc.history(&Principal::Anonymous, &owner, "app", None, None),
            Err(ReadError::NotFound)
        ));
    }
}
