//! Typed models of prikk's `--format json` output (RFC 001 D-2 / PKI-4).
//!
//! Modeled on prikk 0.43.0's hand-written JSON (verified against prikk source). Every document is a
//! top-level object whose first field is a `schema_version` string; the driver (T3) checks it. Unknown
//! *fields* are ignored (serde default — forward-compatible); unknown enum *variants* error and are
//! caught by the version pin (RFC 001 D-3 / PK-18: pre-1.0 JSON may drift, so planeter pins the prikk
//! version it drives).

use serde::Deserialize;

// ----------------------------------------------------------------------------
// Shared unions
// ----------------------------------------------------------------------------

/// A path in an operation: either resolved to a path, or an unresolved node id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PathRef {
    Resolved { path: String },
    Unresolved { unresolved_node_id: String },
}

/// Blob content as `show` reports it (tagged by inner `kind`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BlobContent {
    Text { text: String },
    Binary { blob_id: String, size: u64 },
    Unavailable { blob_id: String },
}

// ----------------------------------------------------------------------------
// `log` — schema_version "log-report-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LogReport {
    pub schema_version: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub current_branch: Option<String>,
    pub blocks: Vec<LogBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LogBlock {
    pub block_id: String,
    pub ref_state_id: String,
    pub update_seq: u64,
    /// Debug rendering of prikk's block kind: `Root` | `Normal` | `Merge` | `Repair` | `Import`.
    /// Kept as a string (open vocabulary; robust to prikk adding kinds pre-1.0).
    pub kind: String,
    pub rollback_block: bool,
    pub parent_count: u64,
    pub patch_count: u64,
    pub rollback_patch_count: u64,
    pub required_attestation_count: u64,
    pub patch_messages: Vec<PatchMessage>,
    pub previous_ref_state_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PatchMessage {
    pub patch_id: String,
    pub message: String,
}

// ----------------------------------------------------------------------------
// `show` — schema_version "show-report-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ShowReport {
    pub schema_version: String,
    pub patches: Vec<ShowPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ShowPatch {
    pub patch_id: String,
    /// `true` = committed but unsealed (queued).
    pub queued: bool,
    pub operations: Vec<Operation>,
}

/// A `show` operation. `kind` is the top-level operation kind (same kebab vocabulary as `content`'s
/// tag); `content` is the tagged detail.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Operation {
    pub kind: String,
    pub paths: Vec<PathRef>,
    pub content: OperationContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OperationContent {
    CreateFile {
        content: BlobContent,
        mode: u32,
    },
    DeleteNode {
        preimage: DeletePreimage,
    },
    EditText {
        old_span_text: String,
        replacement_text: String,
    },
    ReplaceBinary {
        old: BlobContent,
        new: BlobContent,
    },
    RenamePath {
        author_key_id: String,
    },
    ChangePerm {
        old_mode: u32,
        new_mode: u32,
    },
    CreateSymlink {
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DeletePreimage {
    File { content: BlobContent },
    Symlink { old_target: String },
}

// ----------------------------------------------------------------------------
// `verify` — schema_version "verify-report-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VerifyReport {
    pub schema_version: String,
    pub verdict: Verdict,
    pub active_sessions: ActiveSessions,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Verdict {
    pub ok: bool,
    pub failed_conditions: Vec<FailedCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FailedCondition {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ActiveSessions {
    pub count: u64,
    pub verified_count: u64,
}

/// One verification stage, tagged by `status`; `stage` is the stage label (see prikk's 14-stage order).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Stage {
    Evaluated { stage: String },
    Failed { stage: String, message: String },
    NotEvaluated { stage: String, blocked_by: String },
    Halted { stage: String, after: String },
}

// ----------------------------------------------------------------------------
// `branch` — schema_version "branch-list-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BranchList {
    pub schema_version: String,
    pub branches: Vec<Branch>,
    pub received: Vec<ReceivedRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Branch {
    pub ref_name: String,
    pub ref_state_id: String,
    pub closed: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReceivedRef {
    pub ref_name: String,
    pub ref_state_id: String,
}

// ----------------------------------------------------------------------------
// `tag` — schema_version "tag-list-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TagList {
    pub schema_version: String,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tag {
    pub ref_name: String,
    pub target_block_id: String,
}

// ----------------------------------------------------------------------------
// `status` — schema_version "status-report-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusReport {
    pub schema_version: String,
    pub repository: String,
    pub active_wal_records: u64,
    pub trailing_partial_wal_bytes: u64,
    pub heads_main_ref_state: Option<String>,
    pub current_branch: Option<String>,
    pub provisional_worktree: Option<ProvisionalWorktree>,
    pub interrupted_materialization: Option<InterruptedMaterialization>,
    pub queue: Queue,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProvisionalWorktree {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub block_id: String,
    pub replay_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InterruptedMaterialization {
    pub routes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Queue {
    pub count: u64,
    pub target_ref: Option<String>,
    pub target_ref_status: Option<String>,
    pub threshold_status: Option<String>,
    pub warn_threshold: Option<u64>,
    pub hard_limit: Option<u64>,
    pub patches: Vec<QueuedPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct QueuedPatch {
    pub patch_id: String,
    pub message: Option<String>,
    pub operations: Vec<QueuedOperation>,
}

/// A queued operation (leaner than `show`'s `Operation`: no `content`). `author_key_id` is present
/// only when `kind == "rename-path"`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct QueuedOperation {
    pub kind: String,
    pub paths: Vec<PathRef>,
    #[serde(default)]
    pub author_key_id: Option<String>,
}

// ----------------------------------------------------------------------------
// `worktree-status` — schema_version "worktree-status-report-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorktreeStatusReport {
    pub schema_version: String,
    pub repository: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub current_branch: Option<String>,
    pub tracked_files: u64,
    pub unchanged_files: u64,
    pub clean: bool,
    pub refused_count: u64,
    pub refused_declaration_count: u64,
    pub queued_elsewhere: Option<String>,
    pub changes: Vec<WorktreeChange>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorktreeChange {
    pub path: String,
    /// `missing` | `modified` | `untracked` | `unsupported-path`.
    pub kind: String,
    pub detail: String,
    /// `authored` | `refused`.
    pub authoring: String,
    pub refusal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Declaration {
    pub old_path: String,
    pub new_path: String,
    /// `rename` | `deletion` | `deletion-ignored` | `never-tracked` | `refused`.
    pub resolution: String,
    pub refusal: Option<String>,
    /// Non-null only for `resolution == "rename"`.
    pub content_changed: Option<bool>,
    pub mode_changed: Option<bool>,
}

// ----------------------------------------------------------------------------
// `checkout --patch-plan --content-path` — schema_version "patch-plan-content-v1"
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PatchPlanContent {
    pub schema_version: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub target_block_id: String,
    pub coverage: Coverage,
    pub content: Vec<PathContentEntry>,
    pub not_found: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Coverage {
    pub applied_operation_kinds: Vec<String>,
    pub walk: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PathContentEntry {
    pub path: String,
    pub mode: u32,
    pub content: PathContent,
}

/// Content in a patch-plan entry (like [`BlobContent`] but with `opaque`, no `unavailable`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PathContent {
    Text { text: String },
    Binary { blob_id: String, size: u64 },
    Opaque { size: u64 },
}

// ----------------------------------------------------------------------------
// `key status` / `trust maintainer list` / `trust maintainer check`
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeyStatus {
    pub schema_version: String,
    pub roles: Vec<KeyRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeyRole {
    /// `author` | `maintainer`.
    pub role: String,
    pub source: String,
    pub path: String,
    pub usable: bool,
    pub reason: Option<String>,
    pub key_id: String,
    /// `environment` | `default`.
    pub key_id_source: String,
    pub public_key: Option<String>,
    /// `unrecorded` | `matches` | `mismatch` | `not-adopted`, or null.
    pub binding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrustList {
    pub schema_version: String,
    pub keys: Vec<TrustedKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrustedKey {
    pub key_id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TrustCheck {
    pub schema_version: String,
    pub key_id: String,
    pub trusted: bool,
    pub public_key: Option<String>,
}

// ----------------------------------------------------------------------------
// Tests: parse representative prikk JSON to lock the serde attributes (tagging,
// renaming, nullability) against the documented 0.43.0 shapes.
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_report_with_root_block() {
        let json = r#"{
            "schema_version": "log-report-v1",
            "repository": "/r/.prikk",
            "ref": "heads/main",
            "current_branch": null,
            "blocks": [{
                "block_id": "b1", "ref_state_id": "r1", "update_seq": 0,
                "kind": "Root", "rollback_block": false, "parent_count": 0,
                "patch_count": 1, "rollback_patch_count": 0, "required_attestation_count": 0,
                "patch_messages": [{"patch_id": "p1", "message": "init"}],
                "previous_ref_state_id": null
            }]
        }"#;
        let r: LogReport = serde_json::from_str(json).unwrap();
        assert_eq!(r.schema_version, "log-report-v1");
        assert_eq!(r.ref_name, "heads/main");
        assert_eq!(r.current_branch, None);
        assert_eq!(r.blocks[0].kind, "Root");
        assert_eq!(r.blocks[0].patch_messages[0].message, "init");
        assert_eq!(r.blocks[0].previous_ref_state_id, None);
    }

    #[test]
    fn parses_verify_report_with_tagged_stage_variants() {
        let json = r#"{
            "schema_version": "verify-report-v1",
            "verdict": {"ok": false, "failed_conditions": [{"id": "stage-failure", "message": "boom"}]},
            "active_sessions": {"count": 0, "verified_count": 1},
            "stages": [
                {"stage": "objects", "status": "evaluated"},
                {"stage": "refs", "status": "failed", "message": "bad"},
                {"stage": "wal-replay", "status": "not_evaluated", "blocked_by": "refs"},
                {"stage": "commit-index", "status": "halted", "after": "refs"}
            ]
        }"#;
        let r: VerifyReport = serde_json::from_str(json).unwrap();
        assert!(!r.verdict.ok);
        assert_eq!(r.verdict.failed_conditions[0].id, "stage-failure");
        assert_eq!(
            r.stages[0],
            Stage::Evaluated {
                stage: "objects".into()
            }
        );
        assert_eq!(
            r.stages[2],
            Stage::NotEvaluated {
                stage: "wal-replay".into(),
                blocked_by: "refs".into()
            }
        );
    }

    #[test]
    fn parses_show_operation_content_union() {
        let json = r#"{
            "schema_version": "show-report-v1",
            "patches": [{
                "patch_id": "p1", "queued": true,
                "operations": [
                    {"kind": "edit-text", "paths": [{"path": "a.txt"}],
                     "content": {"kind": "edit-text", "old_span_text": "x", "replacement_text": "y"}},
                    {"kind": "create-file", "paths": [{"path": "b.txt"}],
                     "content": {"kind": "create-file", "content": {"kind": "text", "text": "hi"}, "mode": 420}},
                    {"kind": "delete-node", "paths": [{"unresolved_node_id": "n9"}],
                     "content": {"kind": "delete-node", "preimage": {"kind": "symlink", "old_target": "t"}}}
                ]
            }]
        }"#;
        let r: ShowReport = serde_json::from_str(json).unwrap();
        let ops = &r.patches[0].operations;
        assert!(matches!(ops[0].content, OperationContent::EditText { .. }));
        assert_eq!(
            ops[1].paths[0],
            PathRef::Resolved {
                path: "b.txt".into()
            }
        );
        assert_eq!(
            ops[2].paths[0],
            PathRef::Unresolved {
                unresolved_node_id: "n9".into()
            }
        );
        match &ops[2].content {
            OperationContent::DeleteNode { preimage } => {
                assert_eq!(
                    *preimage,
                    DeletePreimage::Symlink {
                        old_target: "t".into()
                    }
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_status_queue_nullable_and_optional_author_key() {
        let json = r#"{
            "schema_version": "status-report-v1",
            "repository": "/r/.prikk",
            "active_wal_records": 0, "trailing_partial_wal_bytes": 0,
            "heads_main_ref_state": null, "current_branch": "main",
            "provisional_worktree": null, "interrupted_materialization": null,
            "queue": {
                "count": 2, "target_ref": "heads/main", "target_ref_status": null,
                "threshold_status": null, "warn_threshold": null, "hard_limit": null,
                "patches": [{
                    "patch_id": "p1", "message": null,
                    "operations": [
                        {"kind": "rename-path", "paths": [{"path": "a"}, {"path": "b"}], "author_key_id": "k1"},
                        {"kind": "edit-text", "paths": [{"path": "a"}]}
                    ]
                }]
            }
        }"#;
        let r: StatusReport = serde_json::from_str(json).unwrap();
        assert_eq!(r.heads_main_ref_state, None);
        assert_eq!(r.queue.count, 2);
        let ops = &r.queue.patches[0].operations;
        assert_eq!(ops[0].author_key_id.as_deref(), Some("k1"));
        assert_eq!(ops[1].author_key_id, None); // absent, not null
    }

    #[test]
    fn parses_patch_plan_content_union_with_opaque_and_not_found() {
        let json = r#"{
            "schema_version": "patch-plan-content-v1",
            "ref": "heads/main", "target_block_id": "b1",
            "coverage": {"applied_operation_kinds": ["edit-text"], "walk": "single-parent"},
            "content": [
                {"path": "a.txt", "mode": 420, "content": {"kind": "text", "text": "hi"}},
                {"path": "big.bin", "mode": 420, "content": {"kind": "opaque", "size": 99}}
            ],
            "not_found": ["missing"]
        }"#;
        let r: PatchPlanContent = serde_json::from_str(json).unwrap();
        assert_eq!(r.ref_name, "heads/main");
        assert_eq!(r.content[1].content, PathContent::Opaque { size: 99 });
        assert_eq!(r.not_found, vec!["missing".to_owned()]);
    }

    #[test]
    fn parses_branch_tag_key_trust_shapes() {
        let b: BranchList = serde_json::from_str(
            r#"{"schema_version":"branch-list-v1",
                "branches":[{"ref_name":"heads/main","ref_state_id":"r1","closed":false,"current":true}],
                "received":[{"ref_name":"remotes/o/main","ref_state_id":"r2"}]}"#,
        )
        .unwrap();
        assert!(b.branches[0].current);
        assert_eq!(b.received[0].ref_name, "remotes/o/main");

        let k: KeyStatus = serde_json::from_str(
            r#"{"schema_version":"key-status-v1","roles":[
                {"role":"author","source":"seed","path":"/s","usable":true,"reason":null,
                 "key_id":"k","key_id_source":"default","public_key":"ab","binding":null}]}"#,
        )
        .unwrap();
        assert_eq!(k.roles[0].binding, None);

        let c: TrustCheck = serde_json::from_str(
            r#"{"schema_version":"trust-check-v1","key_id":"k","trusted":false,"public_key":null}"#,
        )
        .unwrap();
        assert!(!c.trusted);
        assert_eq!(c.public_key, None);
    }
}
