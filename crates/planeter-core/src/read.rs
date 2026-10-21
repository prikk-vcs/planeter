//! The read path (RFC 003): browse a hosted repository through `authorize(_, Read, _)`, over the prikk
//! read verbs, into honest view-models. Read-only — no write verb, no transport (RFC 004).
//!
//! Four properties this module is responsible for:
//! - **T6 read authorization.** Every read resolves the repository, builds an [`AccessContext`], and
//!   calls `authorize(principal, Read, resource)`. A denied private repository is **indistinguishable
//!   from a nonexistent one** ([`ReadError::NotFound`] for both — T-7). UI and API share this one gate.
//! - **T2 browse surfaces.** Each prikk `--format json` verb maps to a view-model here.
//! - **T5 honesty.** A change/history view marks **signed into the history** (sealed) vs merely
//!   **forge-approved (approved-but-unsealed)** distinctly ([`Assurance`]); only the repository-level
//!   verification path says **verified**, and it is **re-derived** from prikk on every call, never served
//!   from a cache (D-5 / SEC-1).
//! - **T1 cache is never the authority.** The view-models are **pure functions of prikk output**, so a
//!   derived cache (keyed by prikk object id) is always droppable and rebuildable from prikk; when prikk
//!   is unavailable a cached view is served **labelled [`Freshness::Stale`]**, never fabricated. The
//!   verify status is exempt — always fresh.
//!
//! The rendering-safety layer (strict HTML/Markdown sanitization, isolated content origin, CSP — RFC 003
//! T4) and the HTTP/OpenAPI surface (T3) sit *above* this in the web crate; view-models carry raw
//! repository text and mark it as such, but never render it.

use std::sync::Arc;

use planeter_prikk::{CallLimits, PrikkRepo, model};
use planeter_store::membership::owner_org;
use planeter_store::{MembershipStore, Owner, RepositoryRecord};

use crate::admission::CONTENT_MAX_BYTES;
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
    /// The reader **may read the repository**, and the path they asked for is not in it (IS-45). Produced only
    /// **after** `authorize_read` succeeded, so it tells a stranger nothing: a hidden or absent repository is
    /// still [`ReadError::NotFound`], byte for byte. The API answers it exactly as `NotFound`; the UI says "this
    /// file or folder was not found" to a reader who is entitled to know the repository is there.
    NoSuchPath,
    /// prikk could not be reached or returned an error (the repository exists and is authorized).
    Unavailable(String),
    /// The response would exceed what this server serves (a file above
    /// [`CONTENT_MAX_BYTES`]), or the repository needs more memory
    /// than the read budget permits (RFC 011 D-3, D-4). Only ever produced for a repository the caller
    /// may read, so it reveals nothing about a hidden one.
    TooLarge,
    /// A backend (store/membership) failure.
    Backend(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::NotFound => f.write_str("not found"),
            ReadError::NoSuchPath => f.write_str("no such path in this repository"),
            ReadError::Unavailable(e) => write!(f, "repository temporarily unavailable: {e}"),
            ReadError::TooLarge => f.write_str("too large to serve here"),
            ReadError::Backend(e) => write!(f, "backend error: {e}"),
        }
    }
}

impl std::error::Error for ReadError {}

type Result<T> = std::result::Result<T, ReadError>;

/// Why a principal cannot propose to a repository ([`ReadService::propose_access`]).
#[derive(Debug)]
#[non_exhaustive]
pub enum ProposeAccess {
    /// The repository does not exist **or** the principal may not read it (indistinguishable, T-7).
    NotFound,
    /// The principal can read the repository but may not propose a change to it.
    NotPermitted,
    /// A backend failure.
    Backend(String),
}

/// Whether a served view was re-derived from prikk now, or is a cached fallback prikk could not refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Freshness {
    Fresh,
    Stale,
}

/// A view served to a surface, tagged with its freshness (T1: a stale fallback is labelled, never
/// passed off as current).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Served<T> {
    pub view: T,
    pub freshness: Freshness,
}

// ----------------------------------------------------------------------------
// Honesty (T5 / D-5)
// ----------------------------------------------------------------------------

/// How much assurance a change carries — the distinction the UI must never blur (SEC-1 / WEB-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Assurance {
    /// The repository's verification **ran and passed**: prikk's verdict is `ok` and every stage was
    /// evaluated. Produced only by [`VerifyView::assurance`] (the repository, history and verification
    /// pages' badge) — never from a `show` or `log` alone (IS-44).
    PrikkVerified,
    /// Sealed: `show` reports the patch as signed into the history. That is all that is known about it
    /// where no verification ran (the change page); it is **not** a claim that anything was checked.
    Sealed,
    /// Accepted by the forge but **not yet sealed** (queued) — a forge-side approval, not a prikk
    /// signature. Must never render as verified.
    ForgeApprovedUnsealed,
    /// The repository's verification ran and did not pass, or could not be completed as a pass.
    Unverified,
}

// ----------------------------------------------------------------------------
// View-models (T2) — pure functions of prikk output
// ----------------------------------------------------------------------------

/// Repository history (from `log`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HistoryView {
    pub ref_name: String,
    pub current_branch: Option<String>,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HistoryEntry {
    pub block_id: String,
    pub ref_state_id: String,
    pub kind: String,
    pub rollback_block: bool,
    pub patch_count: u64,
    pub messages: Vec<PatchMessageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangeView {
    pub patches: Vec<ChangePatchView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangePatchView {
    pub patch_id: String,
    /// Honest assurance: a queued patch is `ForgeApprovedUnsealed`, a sealed one `Sealed`; never verified,
    /// because no verification runs for this view (T5, IS-44).
    pub assurance: Assurance,
    pub operations: Vec<OperationView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
                        // Sealed, and nothing more is known here: whether the repository verifies is the
                        // repository-level verify view's to say (re-derived), not this view's.
                        Assurance::Sealed
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileView {
    pub ref_name: String,
    pub path: String,
    pub content: FileContentView,
}

/// A file's content as the API serializes it: a tagged object, `{"kind": "text", "text": …}`,
/// `{"kind": "binary", "blob_id": …, "size": …}` or `{"kind": "opaque", "size": …}`. A path that is not
/// in the repository is not a content value: [`FileView::from_patch_plan`] returns `None` for it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FileContentView {
    Text { text: String },
    Binary { blob_id: String, size: u64 },
    Opaque { size: u64 },
}

impl FileView {
    /// Build the view for `path` from a patch-plan-content report (which may cover several paths).
    /// `None` when the report has no entry for `path`.
    pub fn from_patch_plan(plan: &model::PatchPlanContent, path: &str) -> Option<Self> {
        let content = plan
            .content
            .iter()
            .find(|e| e.path == path)
            .map(|e| match &e.content {
                model::PathContent::Text { text } => FileContentView::Text { text: text.clone() },
                model::PathContent::Binary { blob_id, size } => FileContentView::Binary {
                    blob_id: blob_id.clone(),
                    size: *size,
                },
                model::PathContent::Opaque { size } => FileContentView::Opaque { size: *size },
            });
        Some(Self {
            ref_name: plan.ref_name.clone(),
            path: path.to_owned(),
            content: content?,
        })
    }
}

/// A directory listing at a point (from `tree`). prikk emits leaf paths only; a caller builds the
/// directory structure from path segments.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TreeView {
    pub point: String,
    pub prefix: Option<String>,
    pub entries: Vec<TreeEntryView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TreeEntryView {
    pub path: String,
    /// `file` | `symlink`.
    pub kind: String,
    /// `text` | `binary` (absent where it does not apply).
    pub encoding: Option<String>,
    pub mode: u32,
    pub size: u64,
    /// Present for binary entries only (a cache hint, not a retrieval key).
    pub content_id: Option<String>,
}

impl TreeView {
    pub fn from_tree(t: &model::TreeListing) -> Self {
        Self {
            point: t.point.clone(),
            prefix: t.prefix.clone(),
            entries: t
                .entries
                .iter()
                .map(|e| TreeEntryView {
                    path: e.path.clone(),
                    kind: e.kind.clone(),
                    encoding: e.encoding.clone(),
                    mode: e.mode,
                    size: e.size,
                    content_id: e.content_id.clone(),
                })
                .collect(),
        }
    }
}

/// The raw bytes of a file at a point (from `cat`), plus the metadata needed to serve them. The bytes
/// are **raw repository content** — the web layer serves them from the isolated content origin as a
/// download, never rendered inline (RFC 003 T4). Not serialized as JSON: it is a byte payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFile {
    pub path: String,
    /// `text` | `binary` (from the metadata), if known.
    pub encoding: Option<String>,
    pub size: u64,
    pub bytes: Vec<u8>,
}

/// What [`ReadService::raw_file`] reads: the file `path` at `point` (the current branch when `None`),
/// refused if larger than `max_bytes`.
#[derive(Debug, Clone, Copy)]
pub struct RawTarget<'a> {
    pub point: Option<&'a str>,
    pub path: &'a str,
    pub max_bytes: Option<u64>,
}

/// Refs/branches/tags (from `branch` + `tag`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RefsView {
    pub current_branch: Option<String>,
    pub branches: Vec<BranchView>,
    pub tags: Vec<TagView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BranchView {
    pub ref_name: String,
    pub ref_state_id: String,
    pub closed: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VerifyView {
    pub ok: bool,
    pub failed_conditions: Vec<FailedConditionView>,
    pub stages: Vec<StageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FailedConditionView {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StageView {
    pub stage: String,
    /// `evaluated` | `failed` | `not_evaluated` | `halted`.
    pub status: String,
    pub detail: Option<String>,
    /// For a `not_evaluated` stage: the id of the stage that blocked it, as data (RFC 012 D-4). Absent from
    /// the JSON otherwise. `detail` keeps its English text unchanged for existing API readers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    /// For a `halted` stage: the id of the stage it stopped after, as data (the same pattern as
    /// `blocked_by`). Absent from the JSON otherwise; `detail` is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

impl VerifyView {
    pub fn from_verify(v: &model::VerifyReport) -> Self {
        Self {
            ok: v.verdict.ok,
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
    ///
    /// The honesty invariant (IS-39): `PrikkVerified` only when prikk's verdict is `ok` **and** every stage
    /// was evaluated. prikk never reports `ok` with an unfinished stage; the check is here so that no future
    /// input can turn one into the "verified" badge.
    pub fn assurance(&self) -> Assurance {
        if self.ok && self.stages.iter().all(|s| s.status == "evaluated") {
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
            blocked_by: None,
            after: None,
        },
        model::Stage::Failed { stage, message } => StageView {
            stage: stage.clone(),
            status: "failed".into(),
            detail: Some(message.clone()),
            blocked_by: None,
            after: None,
        },
        model::Stage::NotEvaluated { stage, blocked_by } => StageView {
            stage: stage.clone(),
            status: "not_evaluated".into(),
            detail: Some(format!("blocked by {blocked_by}")),
            blocked_by: Some(blocked_by.clone()),
            after: None,
        },
        model::Stage::Halted { stage, after } => StageView {
            stage: stage.clone(),
            status: "halted".into(),
            detail: Some(format!("after {after}")),
            blocked_by: None,
            after: Some(after.clone()),
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

    /// The record of a repository the principal may **read** (the one `authorize(_, Read, _)` gate), for a
    /// surface that serves it by other means than a view (the transport). A denied or absent repository is
    /// [`ReadError::NotFound`], the same for both (T-7).
    pub fn readable_record(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
    ) -> Result<RepositoryRecord> {
        self.authorize_read(principal, owner, name)
    }

    /// The record of a repository the principal may **propose** a change to (RFC 004 part 3b, T2), and whether they
    /// hold `write` or above. The one read gate comes first: a repository the principal may not read, or that does
    /// not exist, is [`ProposeAccess::NotFound`], the same for both (T-7). Only a principal who **can read** the
    /// repository learns that they may not propose ([`ProposeAccess::NotPermitted`]), which tells them nothing
    /// they did not know.
    pub fn propose_access(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
    ) -> std::result::Result<(RepositoryRecord, bool), ProposeAccess> {
        let record = match self.authorize_read(principal, owner, name) {
            Ok(r) => r,
            Err(ReadError::NotFound | ReadError::NoSuchPath) => {
                return Err(ProposeAccess::NotFound);
            }
            Err(e) => return Err(ProposeAccess::Backend(e.to_string())),
        };
        let ctx = self
            .access_context(principal, &record)
            .map_err(|e| ProposeAccess::Backend(e.to_string()))?;
        let resource = Resource::Repo(RepoResource {
            repo_id: record.repo_id.clone(),
            owner: record.owner.clone(),
            visibility: record.visibility,
            proposals: record.proposals,
        });
        if !authorize(principal, Action::Propose, &resource, &ctx).is_allowed() {
            return Err(ProposeAccess::NotPermitted);
        }
        let writer = ctx
            .repo_role
            .is_some_and(|r| r >= planeter_store::Role::Write);
        Ok((record, writer))
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
            proposals: record.proposals,
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

    fn open(
        &self,
        record: &RepositoryRecord,
        limits: CallLimits,
    ) -> Result<planeter_prikk::CliPrikkRepo> {
        self.hosting
            .open_record_with(record, limits)
            .map_err(|e| ReadError::Unavailable(e.to_string()))
    }

    /// The repository's store size **S** for admission (RFC 011 D-1, T2) — after the one read gate. A
    /// repository the principal may not read, or that does not exist, is [`ReadError::NotFound`] (so
    /// neither the cost nor `too_large` can reveal a hidden repository); a walk that fails is
    /// [`ReadError::Unavailable`] with one log line, never a guess.
    pub fn store_bytes(&self, principal: &Principal, owner: &Owner, name: &str) -> Result<u64> {
        let record = self.authorize_read(principal, owner, name)?;
        crate::admission::store_size(&record.path).map_err(|e| {
            eprintln!("planeter: a read was not admitted: {e}");
            ReadError::Unavailable(e.to_string())
        })
    }

    /// History of a ref (`log`), bounded by `limit`.
    pub fn history(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        ref_name: Option<&str>,
        limit: Option<usize>,
        limits: CallLimits,
    ) -> Result<HistoryView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        let log = driver.log(ref_name, limit).map_err(unavailable)?;
        Ok(HistoryView::from_log(&log))
    }

    /// A change's content/effect (`show`).
    pub fn change(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        target: &str,
        limits: CallLimits,
    ) -> Result<ChangeView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        let show = driver.show(target).map_err(unavailable)?;
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
        limits: CallLimits,
    ) -> Result<FileView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        // RFC 011 D-3: learn the entry's size from the tree listing and refuse a file above the shared
        // bound *before* any content is read (the patch-plan read holds ≈ 3.1 × the file, in prikk and
        // again here). A path that is not an entry is `NoSuchPath`: the repository was authorized above.
        let listing = driver.tree(ref_name, Some(path)).map_err(unavailable)?;
        let entry = listing
            .entries
            .iter()
            .find(|e| e.path == path)
            .ok_or(ReadError::NoSuchPath)?;
        if entry.size > CONTENT_MAX_BYTES {
            return Err(ReadError::TooLarge);
        }
        let plan = driver
            .patch_plan_content(ref_name, &[path])
            .map_err(unavailable)?;
        // A path that is not in the repository is `NoSuchPath` (never a successful view that says "not found").
        FileView::from_patch_plan(&plan, path).ok_or(ReadError::NoSuchPath)
    }

    /// Refs/branches/tags (`branch` + `tag`).
    pub fn refs(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        include_closed: bool,
        limits: CallLimits,
    ) -> Result<RefsView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        let branches = driver.branches(include_closed).map_err(unavailable)?;
        let tags = driver.tags().map_err(unavailable)?;
        Ok(RefsView::from_lists(&branches, &tags))
    }

    /// A directory listing at a point (`tree`). `prefix` filters by whole path components. A prefix with no
    /// entries is an empty listing, as the API has always answered it (see [`Self::tree_existing`]).
    pub fn tree(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        point: Option<&str>,
        prefix: Option<&str>,
        limits: CallLimits,
    ) -> Result<TreeView> {
        self.tree_inner(principal, owner, name, point, prefix, limits)
            .map(|(view, _)| view)
    }

    /// [`Self::tree`] for the page that names the folder it shows: a **non-empty** `prefix` that has no entry
    /// under it, at a point prikk has **sealed** (`target_block_id` present), is [`ReadError::NoSuchPath`].
    /// prikk stores files, not directories (measured on 0.53.0: an empty directory in the worktree leaves no
    /// trace, and a prefix with no file below it lists nothing), so an empty listing at a sealed point means
    /// the path is not there. Without a sealed block (a repository whose only content is queued work) nothing
    /// is said about the path: the listing is empty, as it is for the API. The API's `tree` does not use this
    /// (its status for a missing prefix is part of the contract and stays `200` with no entries).
    pub fn tree_existing(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        point: Option<&str>,
        prefix: Option<&str>,
        limits: CallLimits,
    ) -> Result<TreeView> {
        let (view, sealed) = self.tree_inner(principal, owner, name, point, prefix, limits)?;
        let asked_for_a_path = prefix.is_some_and(|p| !p.is_empty());
        if asked_for_a_path && sealed && view.entries.is_empty() {
            return Err(ReadError::NoSuchPath);
        }
        Ok(view)
    }

    /// The listing and whether prikk resolved it to a sealed block.
    fn tree_inner(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        point: Option<&str>,
        prefix: Option<&str>,
        limits: CallLimits,
    ) -> Result<(TreeView, bool)> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        let listing = driver.tree(point, prefix).map_err(unavailable)?;
        let sealed = listing.target_block_id.is_some();
        Ok((TreeView::from_tree(&listing), sealed))
    }

    /// The raw bytes of a file at a point (`cat`), for the download / isolated-origin view. `max_bytes`,
    /// if set, refuses (returns `Unavailable`) when the content exceeds it — prikk writes nothing.
    /// A missing path is [`ReadError::NoSuchPath`].
    pub fn raw_file(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        target: RawTarget<'_>,
        limits: CallLimits,
    ) -> Result<RawFile> {
        let RawTarget {
            point,
            path,
            max_bytes,
        } = target;
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        // Metadata first: a missing path becomes NotFound (not a generic unavailable).
        let meta = match driver.path_content_meta(point, path) {
            Ok(m) => m,
            // Only prikk's own "no such path" is `NotFound`. A timeout (status 124), a ceiling hit or any
            // other failure says nothing about the path: `Unavailable` (RFC 011 handoff 1, deviation 4).
            Err(e) if is_missing_path(&e) => return Err(ReadError::NoSuchPath),
            Err(e) => return Err(unavailable(e)),
        };
        // RFC 011 D-4: a file above the bound is `TooLarge` (a retry cannot help), decided from the
        // metadata before `cat` runs. `cat --max-bytes` below stays as the backstop.
        if max_bytes.is_some_and(|max| meta.size > max) {
            return Err(ReadError::TooLarge);
        }
        let bytes = match driver.cat_bytes(point, path, max_bytes) {
            Ok(b) => b,
            // The file grew between the two reads: the same answer as above.
            Err(planeter_prikk::PrikkError::Command { stderr, .. })
                if stderr.contains("above the --max-bytes bound") =>
            {
                return Err(ReadError::TooLarge);
            }
            Err(e) => return Err(unavailable(e)),
        };
        Ok(RawFile {
            path: path.to_owned(),
            encoding: meta.encoding,
            size: meta.size,
            bytes,
        })
    }

    /// The repository verify status (`verify`) — always re-derived, never cached (T5 honesty).
    pub fn verify(
        &self,
        principal: &Principal,
        owner: &Owner,
        name: &str,
        limits: CallLimits,
    ) -> Result<VerifyView> {
        let record = self.authorize_read(principal, owner, name)?;
        let driver = self.open(&record, limits)?;
        let v = driver.verify().map_err(unavailable)?;
        Ok(VerifyView::from_verify(&v))
    }
}

/// Whether a failed `cat` is prikk's own "the path does not exist at this point" (exit 1, "path … does
/// not exist at …"), as opposed to a timeout, a ceiling hit or any other failure.
fn is_missing_path(e: &planeter_prikk::PrikkError) -> bool {
    matches!(
        e,
        planeter_prikk::PrikkError::Command { code: Some(1), stderr } if stderr.contains("does not exist")
    )
}

/// A failed prikk call as the read service reports it: `Unavailable` — and, when the cause is the call's
/// own memory ceiling (RFC 011 D-2), one log line naming the verb and the ceiling (no path, no tool
/// output). A ceiling hit says nothing about the repository, so it is never mapped to a verdict.
fn unavailable(e: planeter_prikk::PrikkError) -> ReadError {
    if let planeter_prikk::PrikkError::CeilingExceeded {
        verb,
        ceiling_bytes,
    } = &e
    {
        eprintln!(
            "planeter: a read hit its memory ceiling: verb={verb} ceiling={ceiling_bytes} bytes"
        );
    }
    ReadError::Unavailable(e.to_string())
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
    fn a_change_view_never_says_verified_because_no_verification_ran_for_it() {
        // IS-44: a sealed patch is `Sealed` (signed in, nothing more is known), a queued one is
        // `ForgeApprovedUnsealed`; no patch, in any mix, is ever `PrikkVerified`.
        let show: model::ShowReport = serde_json::from_str(
            r#"{"schema_version":"show-report-v1","patches":[
                {"patch_id":"p1","queued":true,"operations":[]},
                {"patch_id":"p2","queued":false,"operations":[]},
                {"patch_id":"p3","queued":false,"operations":[]},
                {"patch_id":"p4","queued":true,"operations":[]}]}"#,
        )
        .unwrap();
        let v = ChangeView::from_show(&show);
        let got: Vec<Assurance> = v.patches.iter().map(|p| p.assurance).collect();
        assert_eq!(
            got,
            [
                Assurance::ForgeApprovedUnsealed,
                Assurance::Sealed,
                Assurance::Sealed,
                Assurance::ForgeApprovedUnsealed
            ]
        );
        assert!(!got.contains(&Assurance::PrikkVerified));
        // The JSON tells the truth too.
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["patches"][1]["assurance"], "sealed");
        assert_eq!(json["patches"][0]["assurance"], "forge-approved-unsealed");
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

    #[test]
    fn only_a_blocked_or_halted_stage_carries_its_field_and_the_english_detail_is_unchanged() {
        let report: model::VerifyReport = serde_json::from_str(
            r#"{"schema_version":"verify-report-v1","verdict":{"ok":false,
                "failed_conditions":[]},
                "active_sessions":{"count":0,"verified_count":0},"stages":[
                {"stage":"a","status":"evaluated"},
                {"stage":"b","status":"failed","message":"boom"},
                {"stage":"c","status":"not_evaluated","blocked_by":"b"},
                {"stage":"d","status":"halted","after":"b"}]}"#,
        )
        .unwrap();
        let v = VerifyView::from_verify(&report);
        let blockers: Vec<Option<&str>> =
            v.stages.iter().map(|s| s.blocked_by.as_deref()).collect();
        assert_eq!(blockers, [None, None, Some("b"), None]);
        assert_eq!(v.stages[2].detail.as_deref(), Some("blocked by b"));
        assert_eq!(v.stages[3].detail.as_deref(), Some("after b"));
        let afters: Vec<Option<&str>> = v.stages.iter().map(|s| s.after.as_deref()).collect();
        assert_eq!(afters, [None, None, None, Some("b")]);
        // In the JSON each field is absent, not null, when it does not apply.
        let json = serde_json::to_value(&v).unwrap();
        let stages = json["stages"].as_array().unwrap();
        for (i, (blocked, halted)) in [(false, false), (false, false), (true, false), (false, true)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                stages[i].get("blocked_by").is_some(),
                blocked,
                "{}",
                stages[i]
            );
            assert_eq!(stages[i].get("after").is_some(), halted, "{}", stages[i]);
            assert!(stages[i].get("detail").is_some(), "detail is always there");
        }
        assert_eq!(stages[2]["blocked_by"], "b");
        assert_eq!(stages[3]["after"], "b");
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
                proposals: Default::default(),
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
        let denied = svc.history(
            &Principal::Anonymous,
            &owner,
            "app",
            None,
            None,
            CallLimits::NONE,
        );
        assert!(matches!(denied, Err(ReadError::NotFound)));
        // A genuinely missing repo: also NotFound. The two are indistinguishable (T-7).
        let missing = svc.history(
            &Principal::Anonymous,
            &owner,
            "does-not-exist",
            None,
            None,
            CallLimits::NONE,
        );
        assert!(matches!(missing, Err(ReadError::NotFound)));
    }

    #[test]
    fn anonymous_read_of_internal_is_denied() {
        let (svc, owner) = service_with_repo(Visibility::Internal);
        assert!(matches!(
            svc.history(
                &Principal::Anonymous,
                &owner,
                "app",
                None,
                None,
                CallLimits::NONE
            ),
            Err(ReadError::NotFound)
        ));
    }
}
