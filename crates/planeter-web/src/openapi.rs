//! The read API's OpenAPI 3.0 document (RFC 003 D-2, STD-3), built by hand (no new crate).
//!
//! It is built from [`crate::api::paths`], the same constants the router registers its routes with, so a
//! route cannot be added to one and not the other without a test (`tests/api_contract.rs`) noticing:
//! the test compares the document's paths with its own expected list, probes every path for
//! "registered" (a wrong method answers `405`, an unknown path `404`), and checks that a sample of every
//! view serializes to exactly the fields declared here.

use serde_json::{Value, json};

use crate::api::paths;
use crate::errors::ErrorCode;

fn param(name: &str, location: &str, required: bool, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "in": location,
        "required": required,
        "description": description,
        "schema": schema,
    })
}

fn owner_name() -> Vec<Value> {
    vec![
        param(
            "owner",
            "path",
            true,
            "The repository owner (a user or an organization handle).",
            json!({"type": "string"}),
        ),
        param(
            "name",
            "path",
            true,
            "The repository name.",
            json!({"type": "string"}),
        ),
    ]
}

fn ref_param() -> Value {
    param(
        "ref",
        "query",
        false,
        "A branch or tag ref (for example `heads/main`); the repository's current branch when omitted.",
        json!({"type": "string"}),
    )
}

fn error_ref() -> Value {
    json!({"$ref": "#/components/schemas/Error"})
}

/// The error responses every operation can return, all with the one error schema.
fn errors(extra: &[ErrorCode]) -> Value {
    let mut out = serde_json::Map::new();
    for code in [
        ErrorCode::InvalidRequest,
        ErrorCode::MethodNotAllowed,
        ErrorCode::NotFound,
        ErrorCode::Internal,
        ErrorCode::Unavailable,
        ErrorCode::TooLarge,
    ]
    .iter()
    .chain(extra)
    {
        let entry = out.entry(code.status().as_u16().to_string()).or_insert_with(|| {
            json!({"description": "", "content": {"application/json": {"schema": error_ref()}}})
        });
        let existing = entry["description"].as_str().unwrap_or("").to_owned();
        let add = format!("`{}`: {}", code.code(), code.description());
        entry["description"] = json!(if existing.is_empty() {
            add
        } else {
            format!("{existing}; {add}")
        });
    }
    if let Some(r) = out.get_mut("405") {
        r["headers"] = json!({"Allow": {
            "description": "The methods this route supports: always `GET`.",
            "schema": {"type": "string"}
        }});
    }
    // The 503 from the gate also sets Retry-After.
    if let Some(r) = out.get_mut("503") {
        r["headers"] = json!({"Retry-After": {
            "description": "Seconds to wait before retrying (present when the code is `busy`).",
            "schema": {"type": "integer"}
        }});
    }
    Value::Object(out)
}

fn json_get(id: &str, summary: &str, params: Vec<Value>, schema: &str) -> Value {
    json!({"get": {
        "operationId": id,
        "summary": summary,
        "parameters": params,
        "security": [{"bearerToken": []}, {"sessionCookie": []}, {}],
        "responses": {
            "200": {
                "description": "The view.",
                "headers": {"ETag": {"description": "A strong validator over the canonical JSON.", "schema": {"type": "string"}}},
                "content": {"application/json": {"schema": {"$ref": format!("#/components/schemas/{schema}")}}}
            },
            "304": {"description": "Not modified (the request's `If-None-Match` matched `ETag`)."}
        },
    }})
}

fn with_errors(mut op: Value, extra: &[ErrorCode]) -> Value {
    let errs = errors(extra);
    let method = op
        .as_object()
        .and_then(|o| o.keys().next().cloned())
        .expect("an operation has a method");
    let responses = op[method.as_str()]["responses"]
        .as_object_mut()
        .expect("responses");
    for (k, v) in errs.as_object().expect("errors") {
        responses.insert(k.clone(), v.clone());
    }
    if method == "post" {
        responses["405"]["headers"]["Allow"]["description"] =
            json!("The methods this route supports: always `POST`.");
    }
    op
}

fn string_prop() -> Value {
    json!({"type": "string"})
}

fn obj(required: &[&str], props: Value) -> Value {
    json!({"type": "object", "required": required, "properties": props})
}

fn array_of(schema: &str) -> Value {
    json!({"type": "array", "items": {"$ref": format!("#/components/schemas/{schema}")}})
}

fn nullable_string() -> Value {
    json!({"type": "string", "nullable": true})
}

/// Component schemas for every view type the API returns.
fn schemas() -> Value {
    let mut all = json!({
        "Error": obj(&["code", "message", "hint"], json!({
            "code": {"type": "string", "enum": ErrorCode::ALL.iter().map(|c| c.code()).collect::<Vec<_>>(),
                     "description": "A stable, machine-readable code."},
            "message": {"type": "string", "description": "One plain sentence, in the language negotiated from `Accept-Language` (English by default). Never a path, a repository's existence, or tool output."},
            "hint": {"type": "string", "description": "What the caller can do next, in the same language."},
            "reason": {"type": "string", "description": "Only on `invalid_request` for a proposal: which part of the framing or the metadata was refused (for example `magic`, `truncated`, `title_required`, `unknown_field`)."},
            "quota": {"type": "string", "enum": ["open_proposals", "accepted_bytes", "revisions_per_proposal", "repository_capacity"], "description": "Only on `quota_exceeded`: which limit."},
            "limit": {"type": "integer", "minimum": 0, "description": "Only on `quota_exceeded`: the limit, in `unit`."},
            "used": {"type": "integer", "minimum": 0, "description": "Only on `quota_exceeded`: the current use, in `unit`."},
            "unit": {"type": "string", "enum": ["count", "mebibytes"], "description": "Only on `quota_exceeded`: the unit of `limit` and `used`."},
            "state": {"type": "string", "enum": ["landed", "closed"], "description": "Only on `proposal_not_open`."},
            "count": {"type": "integer", "minimum": 0, "description": "Only on `declaration_misses_added` and `declared_not_held`: how many patches."},
            "patch_ids": {"type": "array", "items": {"type": "string"}, "description": "Only on `declaration_misses_added` and `declared_not_held`: the first patch ids concerned."},
            "detail": {"type": "string", "description": "Only on `accept_refused`: prikk's own text, verbatim (English, not translated)."},
            "intent": {"type": "string", "description": "Only on `incomplete`: the id of the intent left open; the same artifact, sent again, resumes it."},
            "cause": {"type": "string", "description": "Only on `incomplete`: `accept_deadline`, `accept_ceiling`, `accept_failed`, `accept_unreadable` or `accept_not_recorded`."}
        })),
        "Assurance": {"type": "string", "enum": ["prikk-verified", "sealed", "forge-approved-unsealed", "unverified"], "description": "`prikk-verified` only where the repository's verification ran and passed; a sealed patch on the change endpoint is `sealed` (signed into the history; nothing was verified for that view); a queued one is `forge-approved-unsealed`. **In plain words, `forge-approved-unsealed` means: accepted and held by this server, not yet signed into the history; nobody has approved it for merging.** (The word *approved* is part of the machine value, which is in the 0.4.0 contract and does not change; it is not a review decision.)"},
        "PatchMessage": obj(&["patch_id", "message"], json!({"patch_id": string_prop(), "message": string_prop()})),
        "HistoryEntry": obj(
            &["block_id", "ref_state_id", "kind", "rollback_block", "patch_count", "messages"],
            json!({
                "block_id": string_prop(), "ref_state_id": string_prop(), "kind": string_prop(),
                "rollback_block": {"type": "boolean"}, "patch_count": {"type": "integer", "minimum": 0},
                "messages": array_of("PatchMessage")
            })),
        "HistoryView": obj(&["ref_name", "current_branch", "entries"], json!({
            "ref_name": string_prop(), "current_branch": nullable_string(), "entries": array_of("HistoryEntry")
        })),
        "Operation": obj(&["kind", "paths"], json!({
            "kind": string_prop(), "paths": {"type": "array", "items": string_prop()}
        })),
        "ChangePatch": obj(&["patch_id", "assurance", "operations"], json!({
            "patch_id": string_prop(), "assurance": {"$ref": "#/components/schemas/Assurance"},
            "operations": array_of("Operation")
        })),
        "ChangeView": obj(&["patches"], json!({"patches": array_of("ChangePatch")})),
        "FileContentText": obj(&["kind", "text"], json!({
            "kind": {"type": "string", "enum": ["text"]}, "text": string_prop()
        })),
        "FileContentBinary": obj(&["kind", "blob_id", "size"], json!({
            "kind": {"type": "string", "enum": ["binary"]}, "blob_id": string_prop(),
            "size": {"type": "integer", "minimum": 0}
        })),
        "FileContentOpaque": obj(&["kind", "size"], json!({
            "kind": {"type": "string", "enum": ["opaque"]}, "size": {"type": "integer", "minimum": 0}
        })),
        "FileContent": {
            "oneOf": [
                {"$ref": "#/components/schemas/FileContentText"},
                {"$ref": "#/components/schemas/FileContentBinary"},
                {"$ref": "#/components/schemas/FileContentOpaque"}
            ],
            "discriminator": {"propertyName": "kind", "mapping": {
                "text": "#/components/schemas/FileContentText",
                "binary": "#/components/schemas/FileContentBinary",
                "opaque": "#/components/schemas/FileContentOpaque"
            }}
        },
        "FileView": obj(&["ref_name", "path", "content"], json!({
            "ref_name": string_prop(), "path": string_prop(), "content": {"$ref": "#/components/schemas/FileContent"}
        })),
        "TreeEntry": obj(&["path", "kind", "encoding", "mode", "size", "content_id"], json!({
            "path": string_prop(), "kind": string_prop(), "encoding": nullable_string(),
            "mode": {"type": "integer", "minimum": 0}, "size": {"type": "integer", "minimum": 0},
            "content_id": nullable_string()
        })),
        "TreeView": obj(&["point", "prefix", "entries"], json!({
            "point": string_prop(), "prefix": nullable_string(), "entries": array_of("TreeEntry")
        })),
        "Branch": obj(&["ref_name", "ref_state_id", "closed", "current"], json!({
            "ref_name": string_prop(), "ref_state_id": string_prop(),
            "closed": {"type": "boolean"}, "current": {"type": "boolean"}
        })),
        "Tag": obj(&["ref_name", "target_block_id"], json!({"ref_name": string_prop(), "target_block_id": string_prop()})),
        "RefsView": obj(&["current_branch", "branches", "tags"], json!({
            "current_branch": nullable_string(), "branches": array_of("Branch"), "tags": array_of("Tag")
        })),
        "Offer": obj(&["kind", "ref", "size", "etag"], json!({
            "kind": {"type": "string", "enum": ["bundle", "archive"]},
            "ref": nullable_string(),
            "size": {"type": "integer", "minimum": 0, "nullable": true, "description": "The last known size of an export of it; null until the first export (it may be stale)."},
            "etag": {"type": "string", "description": "The `ETag` a fetch of it would return now."}
        })),
        "AdvertisedRef": obj(&["name", "state", "kind", "closed", "tip_block", "ref_state_id", "fetch"], json!({
            "name": string_prop(),
            "state": {"type": "string", "enum": ["published", "received"]},
            "kind": {"type": "string", "enum": ["branch", "tag", "received"]},
            "closed": {"type": "boolean", "nullable": true},
            "tip_block": nullable_string(), "ref_state_id": nullable_string(),
            "fetch": array_of("Offer")
        })),
        "Advertisement": obj(&["fingerprint", "refs", "archive"], json!({
            "fingerprint": {"type": "string", "description": "The repository's state fingerprint."},
            "refs": array_of("AdvertisedRef"),
            "archive": {"$ref": "#/components/schemas/Offer"}
        })),
        "ProposedRevision": obj(&["id", "seq"], json!({"id": string_prop(), "seq": {"type": "integer", "minimum": 1}})),
        "ProposedClaim": obj(&["claim_id", "outcome"], json!({
            "claim_id": string_prop(),
            "outcome": {"type": "string", "description": "What accept said of the claim, as it said it (stored, not re-asserted)."}
        })),
        "ProposedPatch": obj(&["patch_id", "assurance"], json!({
            "patch_id": string_prop(), "assurance": {"$ref": "#/components/schemas/Assurance"}
        })),
        "Proposed": obj(&["proposal_id", "revision", "created_proposal", "already_recorded", "claims", "added_count", "patches", "declared", "artifact_bytes", "assurance", "message"], json!({
            "proposal_id": string_prop(),
            "revision": {"$ref": "#/components/schemas/ProposedRevision"},
            "created_proposal": {"type": "boolean", "description": "This push opened the proposal."},
            "already_recorded": {"type": "boolean", "description": "The artifact was already a revision; nothing was recorded."},
            "claims": array_of("ProposedClaim"),
            "added_count": {"type": "integer", "minimum": 0, "description": "How many patches the push added."},
            "patches": {"type": "array", "items": {"$ref": "#/components/schemas/ProposedPatch"}, "description": "The first patches the push added (at most 1,000), each classified from facts (`sync pending`, `log`), never from `queued`."},
            "declared": {"type": "boolean", "description": "Whether the revision carries the submitter's declared patch set."},
            "artifact_bytes": {"type": "integer", "minimum": 0},
            "assurance": {"$ref": "#/components/schemas/Assurance", "description": "For a proposal always `forge-approved-unsealed`: accepted and held by this server, not yet signed into the history; nobody has approved it for merging."},
            "message": {"type": "string", "description": "One plain sentence, in the language negotiated from `Accept-Language`: the work is held for review and no branch has changed."}
        })),
        "FailedCondition": obj(&["id", "message"], json!({"id": string_prop(), "message": string_prop()})),
        "Stage": obj(&["stage", "status", "detail"], json!({
            "stage": string_prop(), "status": string_prop(), "detail": nullable_string(),
            "blocked_by": {"type": "string", "description": "Only on a `not_evaluated` stage: the id of the stage that blocked it. Absent otherwise."},
            "after": {"type": "string", "description": "Only on a `halted` stage: the id of the stage it stopped after. Absent otherwise."}
        })),
        "VerifyView": obj(&["ok", "failed_conditions", "stages"], json!({
            "ok": {"type": "boolean"},
            "failed_conditions": array_of("FailedCondition"),
            "stages": array_of("Stage")
        })),
    });
    if let (Some(all), Some(more)) = (all.as_object_mut(), recovery_schemas().as_object()) {
        all.extend(more.clone());
    }
    if let (Some(all), Some(more)) = (all.as_object_mut(), proposal_schemas().as_object()) {
        all.extend(more.clone());
    }
    all
}

/// The schemas of the proposals (RFC 005 part 5): a *proposal* is what the code calls a *change*.
fn proposal_schemas() -> Value {
    json!({
        "ProposalRetention": obj(&["state", "until"], json!({
            "state": {"type": "string", "enum": ["kept", "partly_removed", "removed"], "description": "What is left of the files of a proposal that landed or was closed."},
            "until": {"type": "integer", "minimum": 0, "description": "Unix seconds: the files are kept until then; after it a sweep may remove them (the records stay)."}
        })),
        "ProposalItem": obj(&["id", "title", "state", "opened_by", "opened_at", "updated_at", "revision_count", "patch_count", "built_on", "retention"], json!({
            "id": string_prop(), "title": string_prop(),
            "state": {"type": "string", "enum": ["open", "landed", "closed"]},
            "opened_by": string_prop(),
            "opened_at": {"type": "integer", "minimum": 0}, "updated_at": {"type": "integer", "minimum": 0},
            "revision_count": {"type": "integer", "minimum": 0},
            "patch_count": {"type": "integer", "minimum": 0, "description": "The patches of the current set: the latest declared revision's declared set plus what every later revision added."},
            "built_on": {"type": "boolean", "description": "Whether the latest revision says what it was built on."},
            "retention": {"allOf": [{"$ref": "#/components/schemas/ProposalRetention"}], "nullable": true, "description": "Absent (null) while the proposal is open."}
        })),
        "ProposalList": obj(&["proposals", "next"], json!({
            "proposals": array_of("ProposalItem"),
            "next": {"type": "string", "nullable": true, "description": "The `after` cursor of the next page; null on the last page."}
        })),
        "ProposalClaim": obj(&["claim_id", "outcome"], json!({
            "claim_id": string_prop(),
            "outcome": {"type": "string", "description": "What `sync accept` reported for the claim, as prikk said it (stored, not re-asserted; English, not translated)."}
        })),
        "ProposalRevision": obj(&["seq", "created_at", "submitter", "claims", "author_key_ids", "built_on", "declared", "added_count", "artifact_digest", "artifact_bytes", "artifact_available"], json!({
            "seq": {"type": "integer", "minimum": 1},
            "created_at": {"type": "integer", "minimum": 0},
            "submitter": string_prop(),
            "claims": array_of("ProposalClaim"),
            "author_key_ids": {"type": "array", "items": string_prop()},
            "built_on": {"type": "string", "nullable": true, "description": "The block the sender says the work was built on (the declared base); null when none was given."},
            "declared": {"type": "boolean", "description": "Whether the sender declared the patches the revision contains. A plain push by prikk does not, and then the list of patches may be incomplete."},
            "added_count": {"type": "integer", "minimum": 0, "description": "How many patches this accept added."},
            "artifact_digest": string_prop(),
            "artifact_bytes": {"type": "integer", "minimum": 0},
            "artifact_available": {"type": "boolean", "description": "The files are still in the store (derived each time; a retention sweep removes them after the grace)."}
        })),
        "ProposalPatch": obj(&["patch_id", "assurance"], json!({
            "patch_id": string_prop(),
            "assurance": {"allOf": [{"$ref": "#/components/schemas/Assurance"}], "nullable": true, "description": "`forge-approved-unsealed`: listed by `sync pending` (held by this server, nobody has signed it in). `sealed`: in a block `log` lists for a ref the repository holds. `null`: in neither; nothing is claimed. Never read from `queued`."}
        })),
        "ProposalDescription": obj(&["source", "html"], json!({
            "source": {"type": "string", "description": "The text as sent."},
            "html": {"type": "string", "description": "The same, rendered as Markdown through the sanitizer."}
        })),
        "ProposalDetail": obj(&["id", "title", "description", "state", "closed_reason", "landed_at", "target_ref", "opened_by", "opened_at", "updated_at", "retention", "revisions", "patches", "patch_total", "truncated", "still_pending", "note"], json!({
            "id": string_prop(), "title": string_prop(),
            "description": {"$ref": "#/components/schemas/ProposalDescription"},
            "state": {"type": "string", "enum": ["open", "landed", "closed"]},
            "closed_reason": nullable_string(),
            "landed_at": {"type": "integer", "nullable": true, "description": "When the landing was observed, unix seconds (a landed proposal)."},
            "target_ref": {"type": "string", "nullable": true, "description": "The ref the sender hoped it would reach (advisory)."},
            "opened_by": string_prop(),
            "opened_at": {"type": "integer", "minimum": 0}, "updated_at": {"type": "integer", "minimum": 0},
            "retention": {"allOf": [{"$ref": "#/components/schemas/ProposalRetention"}], "nullable": true},
            "revisions": array_of("ProposalRevision"),
            "patches": {"type": "array", "items": {"$ref": "#/components/schemas/ProposalPatch"}, "description": "The patches of the current set, at most 1,000, in id order."},
            "patch_total": {"type": "integer", "minimum": 0}, "truncated": {"type": "boolean"},
            "still_pending": {"type": "integer", "minimum": 0, "description": "How many patches of the whole current set are still held by this server, not yet signed in."},
            "note": {"type": "string", "description": "One plain sentence, in the language negotiated from `Accept-Language`, that is true under every answer to what happens after a maintainer signs the work."}
        })),
        "ProposalClosed": obj(&["id", "state", "closed_reason", "updated_at", "retain_until"], json!({
            "id": string_prop(), "state": {"type": "string", "enum": ["closed"]},
            "closed_reason": string_prop(), "updated_at": {"type": "integer", "minimum": 0},
            "retain_until": {"type": "integer", "nullable": true, "description": "The files are kept at least until then (unix seconds)."}
        })),
    })
}

/// The proposal paths (RFC 005 part 5).
fn proposal_paths(paths_obj: &mut serde_json::Map<String, Value>) {
    let id_param = param(
        "id",
        "path",
        true,
        "The proposal's id (from the list, or `proposal_id` of a propose answer).",
        json!({"type": "string"}),
    );
    let mut with_id = owner_name();
    with_id.push(id_param.clone());
    let read_security = json!([{"bearerToken": []}, {"sessionCookie": []}, {}]);
    paths_obj.insert(
        paths::PROPOSALS.into(),
        with_errors(
            json!({"get": {
                "operationId": "getProposals",
                "summary": "The proposals of a repository, newest-updated first",
                "description": "`Authorized(Read)`: a hidden and an absent repository are both `404`. Rate limited like the fetch routes. **No prikk call is made per item**: the landing of an open proposal is observed once per repository per state fingerprint, from `sync pending` and (only when some proposal's patches have all left the pending list) `log`; a proposal is `landed` only when every patch of its current set is absent from the pending list **and** present in a block `log` lists. This server does not merge anything itself; a maintainer signs the work on their own computer.",
                "parameters": owner_name().into_iter().chain([
                    param("state", "query", false, "`open` (the default), `landed`, `closed` or `all`.", json!({"type": "string", "enum": ["open", "landed", "closed", "all"]})),
                    param("limit", "query", false, "At most 100; 50 when omitted.", json!({"type": "integer", "minimum": 1, "maximum": 100})),
                    param("after", "query", false, "The `next` cursor of the previous page.", json!({"type": "string"})),
                ]).collect::<Vec<_>>(),
                "security": read_security.clone(),
                "responses": {"200": {"description": "A page of proposals.", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ProposalList"}}}}},
            }}),
            &[ErrorCode::TransportOff, ErrorCode::RateLimited],
        ),
    );
    paths_obj.insert(
        paths::PROPOSAL.into(),
        with_errors(
            json!({"get": {
                "operationId": "getProposal",
                "summary": "One proposal, in full",
                "description": "`Authorized(Read)`; an id of another repository is the same `404` as an unknown one. The description is rendered Markdown through the sanitizer (and given as sent); the revisions are in order, each with its claims **as accept reported them (stored, not re-asserted)**, its key ids and what it was built on; the patches are those of the **current** set (at most 1,000, `truncated` says when there are more), each classified from the facts of one observation, never from `queued`.",
                "parameters": with_id.clone(),
                "security": read_security.clone(),
                "responses": {"200": {"description": "The proposal.", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ProposalDetail"}}}}},
            }}),
            &[ErrorCode::TransportOff, ErrorCode::RateLimited],
        ),
    );
    paths_obj.insert(
        paths::PROPOSAL_CLOSE.into(),
        with_errors(
            json!({"post": {
                "operationId": "postProposalClose",
                "summary": "Close a proposal with a reason",
                "description": "For the proposal's **opener** (with a credential that permits closing) and for whoever `authorize(Close)` allows (`write` and above); a stranger is `404`, a reader who may not close it `403`. **Bearer token only** (a cookie is not looked at), like `propose`. Closing keeps the record and the files (for the grace period, 30 days by default); nobody can reopen it. A landed or closed proposal is `proposal_final`.",
                "parameters": with_id.clone(),
                "security": [{"bearerToken": []}],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "required": ["reason"], "properties": {"reason": {"type": "string", "maxLength": 1000}}}}}},
                "responses": {"200": {"description": "The proposal, closed.", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ProposalClosed"}}}}},
            }}),
            &[ErrorCode::TransportOff, ErrorCode::RateLimited, ErrorCode::Forbidden, ErrorCode::ReasonRequired, ErrorCode::ProposalFinal],
        ),
    );
    let mut download_params = with_id;
    download_params.push(param(
        "seq",
        "path",
        true,
        "The revision's number (1, 2, …).",
        json!({"type": "integer", "minimum": 1}),
    ));
    paths_obj.insert(
        paths::PROPOSAL_ARTIFACT.into(),
        with_errors(
            json!({"get": {
                "operationId": "getProposalArtifact",
                "summary": "The artifact of an accepted revision, byte for byte",
                "description": "For whoever holds `review` or `merge-seal` (either); a stranger is `404`, a reader who holds neither `403`. Served from the artifact store as the fetch route is (`Range`, `If-Range`, `ETag` = the digest, `Accept-Ranges`), never under a repository lease: the file is immutable. After the retention sweep it may be gone: `410` `artifact_removed`, with the date it was kept until.",
                "parameters": download_params,
                "security": read_security,
                "responses": {
                    "200": {"description": "The artifact.", "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}},
                    "206": {"description": "A byte range of it."},
                    "304": {"description": "Not modified."}
                },
            }}),
            &[ErrorCode::TransportOff, ErrorCode::RateLimited, ErrorCode::Forbidden, ErrorCode::RangeNotSatisfiable, ErrorCode::ArtifactRemoved],
        ),
    );
}

/// The recovery schemas (RFC 004 part 4), in two parts so that no one `json!` is too deep for the compiler.
fn recovery_schemas() -> Value {
    let mut a = json!({
        "RecoveryCause": obj(&["code", "text"], json!({
            "code": {"type": "string", "enum": ["accept_deadline", "accept_ceiling", "accept_failed", "accept_unreadable", "accept_not_recorded", "server_stopped", "other"]},
            "text": string_prop()
        })),
        "RecoveryHolder": obj(&["holder_id", "class", "operation", "since_unix", "dead"], json!({
            "holder_id": string_prop(), "class": {"type": "string", "enum": ["W", "X"]}, "operation": string_prop(),
            "since_unix": {"type": "integer", "minimum": 0},
            "dead": {"type": "boolean", "description": "Written by another boot of the server: that process is gone."}
        })),
        "RecoveryProcess": obj(&["pid", "command"], json!({"pid": {"type": "integer", "minimum": 0}, "command": string_prop()})),
        "RecoveryIntent": obj(&["intent_id", "principal", "started_at", "attempts", "artifact_bytes", "expired"], json!({
            "intent_id": string_prop(), "principal": string_prop(), "started_at": {"type": "integer", "minimum": 0},
            "attempts": {"type": "integer", "minimum": 1}, "artifact_bytes": {"type": "integer", "minimum": 0},
            "expired": {"type": "boolean", "description": "Older than the retention period (30 days): an administrator's run may abandon it."}
        })),
        "RecoveryLock": obj(&["path", "kind", "recorded_pid", "liveness", "expected"], json!({
            "path": string_prop(), "kind": string_prop(), "recorded_pid": {"type": "integer", "minimum": 0},
            "liveness": {"type": "string", "description": "prikk's own words, kept for the record and never trusted (inside our PID namespace they are wrong)."},
            "expected": {"type": "boolean", "description": "Of a kind an accept leaves, inside the repository's own `.prikk` directory."}
        })),
        "RecoveryDoctor": obj(&["errors", "warnings", "info", "clean", "repairs_named", "repairs_allowed", "repair_refusal"], json!({
            "errors": {"type": "integer", "minimum": 0}, "warnings": {"type": "integer", "minimum": 0}, "info": {"type": "integer", "minimum": 0},
            "clean": {"type": "boolean"},
            "repairs_named": {"type": "array", "items": {"type": "string"}, "description": "Flags the report's recommendations name."},
            "repairs_allowed": {"type": "array", "items": {"type": "string"}, "description": "The repairs an administrator's run would run: only when **every** problem is covered by a repair the report names that is on the idempotent allow-list (`--repair-wal-tail`, `--repair-index`, `--repair-pointer-index-tail`, `--repair-tails`)."},
            "repair_refusal": {"type": "string", "nullable": true, "description": "Why planeter repairs nothing here, when the report has problems that are not all covered that way (a problem with no named repair, or one that names a repair off the list): a person decides."}
        })),
        "RecoveryBlocker": obj(&["code", "text", "detail", "overridable"], json!({
            "code": {"type": "string", "enum": ["cause_not_ours", "lease_held", "foreign_process", "processes_uninspected", "doctor_problems", "output_unreadable", "unexpected_lock", "too_many_open_intents"]},
            "text": string_prop(),
            "detail": {"type": "string", "description": "What was seen, verbatim where it is prikk's text (English)."},
            "overridable": {"type": "boolean", "description": "An administrator's run goes on despite it; the others always stop."}
        })),
    });
    let b = json!({
        "RecoveryPlan": obj(&["flagged", "flag_reason", "flag_since", "cause", "holders", "processes", "processes_uninspected", "intents", "locks", "doctor", "blockers", "commands", "may_run_unattended", "steps"], json!({
            "flagged": {"type": "boolean"}, "flag_reason": nullable_string(),
            "flag_since": {"type": "integer", "minimum": 0, "nullable": true},
            "cause": {"$ref": "#/components/schemas/RecoveryCause"},
            "holders": array_of("RecoveryHolder"), "processes": array_of("RecoveryProcess"),
            "processes_uninspected": {"type": "integer", "minimum": 0, "description": "How many `prikk` or `bwrap` processes (or processes that could not be identified) could not be inspected: another user's, or hidden. A warning that stops an unattended run and not an administrator's."}, "intents": array_of("RecoveryIntent"),
            "locks": array_of("RecoveryLock"),
            "doctor": {"allOf": [{"$ref": "#/components/schemas/RecoveryDoctor"}], "nullable": true},
            "blockers": array_of("RecoveryBlocker"),
            "commands": {"type": "array", "items": {"type": "string"}, "description": "The exact commands for an operator when the procedure stays stopped; run them in the shell of the server's host."},
            "may_run_unattended": {"type": "boolean", "description": "Every witness agrees and the cause is ours: the server may run it by itself."},
            "steps": {"type": "array", "items": {"type": "string"}}
        })),
        "RecoveryStep": obj(&["seq", "name", "started_at", "ended_at", "outcome", "detail"], json!({
            "seq": {"type": "integer", "minimum": 1}, "name": string_prop(), "started_at": {"type": "integer", "minimum": 0},
            "ended_at": {"type": "integer", "minimum": 0, "nullable": true}, "outcome": nullable_string(),
            "detail": {"type": "string", "description": "What was done and what prikk said, verbatim (bounded)."}
        })),
        "RecoveryRecord": obj(&["recovery_id", "actor", "started_at", "ended_at", "outcome", "steps"], json!({
            "recovery_id": string_prop(), "actor": {"type": "string", "description": "A user id, or `automatic`."},
            "started_at": {"type": "integer", "minimum": 0}, "ended_at": {"type": "integer", "minimum": 0, "nullable": true},
            "outcome": nullable_string(), "steps": array_of("RecoveryStep")
        })),
        "RecoveryState": obj(&["plan", "recoveries"], json!({
            "plan": {"$ref": "#/components/schemas/RecoveryPlan"}, "recoveries": array_of("RecoveryRecord")
        })),
        "RecoveryRun": obj(&["outcome", "outcome_text", "recovery_id", "stopped", "plan"], json!({
            "outcome": {"type": "string", "enum": ["recovered", "stopped", "not_now", "nothing_to_do"]},
            "outcome_text": string_prop(), "recovery_id": nullable_string(),
            "stopped": {"type": "string", "nullable": true, "description": "Why a stopped run stopped, in a line."},
            "plan": {"$ref": "#/components/schemas/RecoveryPlan"}
        })),
    });
    if let (Some(a), Some(b)) = (a.as_object_mut(), b.as_object()) {
        a.extend(b.clone());
    }
    a
}

/// The full document.
pub fn document(content_origin: &str) -> Value {
    let mut history_params = owner_name();
    history_params.push(ref_param());
    history_params.push(param(
        "limit",
        "query",
        false,
        "The most entries to return; values above 200 are clamped to 200.",
        json!({"type": "integer", "default": 50, "maximum": 200}),
    ));
    let mut change_params = owner_name();
    change_params.push(param(
        "target",
        "path",
        true,
        "A block id or a patch id.",
        json!({"type": "string"}),
    ));
    let mut file_params = owner_name();
    file_params.push(ref_param());
    file_params.push(param(
        "path",
        "query",
        true,
        "A repository-relative file path.",
        json!({"type": "string"}),
    ));
    let mut tree_params = owner_name();
    tree_params.push(ref_param());
    tree_params.push(param(
        "prefix",
        "query",
        false,
        "Only paths at or under this directory prefix.",
        json!({"type": "string"}),
    ));
    let mut raw_params = owner_name();
    raw_params.push(ref_param());
    raw_params.push(param(
        "path",
        "query",
        true,
        "A repository-relative file path.",
        json!({"type": "string"}),
    ));
    let mut refs_params = owner_name();
    refs_params.push(param(
        "all",
        "query",
        false,
        "Include closed branches.",
        json!({"type": "boolean", "default": false}),
    ));

    let raw = {
        let mut op = json!({"get": {
            "operationId": "getRaw",
            "summary": "Raw file bytes, as an inert download",
            "description": "Served with `Content-Disposition: attachment`, `nosniff` and a sandbox CSP; never rendered. Larger than the response cap is refused. **Raw bytes are served only on the content origin named in this path's `servers` entry**; any other host answers `404` with the code `raw_elsewhere`.",
            "parameters": raw_params,
            "security": [{"bearerToken": []}, {"sessionCookie": []}, {}],
            "responses": {
                "200": {
                    "description": "The file's bytes.",
                    "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}
                }
            },
        }});
        let errs = errors(&[ErrorCode::Busy, ErrorCode::RawElsewhere]);
        let responses = op["get"]["responses"].as_object_mut().expect("responses");
        for (k, v) in errs.as_object().expect("errors") {
            responses.insert(k.clone(), v.clone());
        }
        op
    };

    let mut paths_obj = serde_json::Map::new();
    let busy = [ErrorCode::Busy];
    paths_obj.insert(
        paths::HISTORY.into(),
        with_errors(
            json_get(
                "getHistory",
                "Repository history",
                history_params,
                "HistoryView",
            ),
            &busy,
        ),
    );
    paths_obj.insert(
        paths::CHANGE.into(),
        with_errors(
            json_get(
                "getChange",
                "A change's content",
                change_params,
                "ChangeView",
            ),
            &busy,
        ),
    );
    paths_obj.insert(
        paths::FILE.into(),
        with_errors(
            json_get(
                "getFile",
                "A file's content at a ref",
                file_params,
                "FileView",
            ),
            &busy,
        ),
    );
    paths_obj.insert(
        paths::TREE.into(),
        with_errors(
            json_get(
                "getTree",
                "Directory listing at a ref",
                tree_params,
                "TreeView",
            ),
            &busy,
        ),
    );
    let mut raw = raw;
    raw["servers"] = json!([{
        "url": content_origin.trim_end_matches('/'),
        "description": "The content origin: the only host that serves raw file bytes."
    }]);
    paths_obj.insert(paths::RAW.into(), raw);
    paths_obj.insert(
        paths::REFS.into(),
        with_errors(
            json_get("getRefs", "Branches and tags", refs_params, "RefsView"),
            &busy,
        ),
    );
    paths_obj.insert(
        paths::VERIFY.into(),
        with_errors(
            json_get("getVerify", "Verify status", owner_name(), "VerifyView"),
            &busy,
        ),
    );
    // RFC 004 part 2: the transport's read side.
    let transport_errors = [
        ErrorCode::Busy,
        ErrorCode::TransportOff,
        ErrorCode::RateLimited,
    ];
    paths_obj.insert(
        paths::TRANSPORT_ADVERTISE.into(),
        with_errors(
            {
                let mut op = json_get(
                    "getTransportAdvertise",
                    "What the repository holds and can be fetched",
                    owner_name(),
                    "Advertisement",
                );
                op["get"]["description"] = json!("The refs the repository holds (published `heads/…` and `tags/…`, received `remotes/…`), the repository's **state fingerprint**, and the fetches on offer with the last known size of each (absent until the first export) and the `ETag` a fetch would return now. No filesystem path appears. Subject to the per-client and per-user rate limit.");
                op
            },
            &transport_errors,
        ),
    );
    paths_obj.insert(
        paths::TRANSPORT_FETCH.into(),
        with_errors(
            {
                let mut params = owner_name();
                params.push(param(
                    "kind",
                    "query",
                    true,
                    "`archive` (the whole repository, the only way to carry a received ref) or `bundle` (one published ref).",
                    json!({"type": "string", "enum": ["archive", "bundle"]}),
                ));
                params.push(param(
                    "ref",
                    "query",
                    false,
                    "The ref to bundle (required for `kind=bundle`; ignored for `archive`); take it from the advertisement.",
                    json!({"type": "string"}),
                ));
                json!({"get": {
                    "operationId": "getTransportFetch",
                    "summary": "Fetch the repository as one verified artifact",
                    "description": "A **whole artifact**, exported once under the repository's export lease, verified (`archive verify` / `bundle verify`; one that does not verify is never served), spooled and cached by the repository's state fingerprint, and served from the spool file: a slow client holds no lock. `304` when `If-None-Match` matches the `ETag`; `Range` is honoured within the one current artifact (`206`, `416`). The cost of a changed-state fetch is the artifact's size; the mitigations are the cache, single flight (concurrent requests share one export), a bound on concurrent exports and the rate limits.",
                    "parameters": params,
                    "security": [{"bearerToken": []}, {"sessionCookie": []}, {}],
                    "responses": {
                        "200": {
                            "description": "The artifact.",
                            "headers": {
                                "ETag": {"description": "Derived from the repository, the kind, the ref and the state fingerprint: equal for equal states.", "schema": {"type": "string"}},
                                "Accept-Ranges": {"description": "`bytes`.", "schema": {"type": "string"}},
                                "Content-Disposition": {"description": "`attachment` with a file name.", "schema": {"type": "string"}}
                            },
                            "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}
                        },
                        "206": {
                            "description": "A byte range of the one current artifact.",
                            "headers": {"Content-Range": {"description": "`bytes first-last/size`.", "schema": {"type": "string"}}},
                            "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}
                        },
                        "304": {"description": "Not modified (the request's `If-None-Match` matched `ETag`)."}
                    },
                }})
            },
            &[
                ErrorCode::Busy,
                ErrorCode::TransportOff,
                ErrorCode::RateLimited,
                ErrorCode::UnknownRef,
                ErrorCode::UnknownKind,
                ErrorCode::BundleOfReceivedRef,
                ErrorCode::NotVerified,
                ErrorCode::ExportFailed,
                ErrorCode::RangeNotSatisfiable,
            ],
        ),
    );
    paths_obj.insert(
        paths::TRANSPORT_PROPOSE.into(),
        with_errors(
            json!({"post": {
                "operationId": "postTransportPropose",
                "summary": "Propose: put an artifact on the forge as a revision of a proposal",
                "description": "Authorized for **propose** (`write` and above; any signed-in reader if the repository's `proposals` setting is `any-authenticated`; never a CI job) and authenticated by a **bearer token only** (a session cookie is not looked at, so another site cannot make this request on a browser's behalf). The body is framed, without multipart: `PLANETER-PROPOSE-1\\n`, an 8-byte big-endian length `M`, `M` bytes of JSON metadata, then the artifact (`sync build` output, `PEXCH002`) to the end of the body. `Content-Length` is required; `M` is at most 4 MiB and the artifact at most `PRIKK_EXCHANGE_MAX_BYTES` (default 100 MiB; `413` before the body is read) and at most `PRIKK_EXCHANGE_MAX_OBJECTS` (default 5,000) patches and as many blobs, which prikk checks and refuses with `422 accept_refused` (nothing is accepted); the artifact is streamed to disk and hashed, never held in memory. Metadata fields: `change` (an existing change id to add a revision to; absent opens a change), `title` (required for a new change, at most 200 characters), `description` (at most 64 KiB), `target_ref` (advisory), `base` (the block id the artifact was built on; it must be listed by `log` of a ref the repository holds), `declared_patches` (the 64-hex patch ids the artifact contains; absent for a push by plain prikk), `client` (name and version). Unknown fields are refused. The checks run in this order, all before any write: rate limit, read gate, propose authorization, `needs_recovery`, framing and size, quota (a refusal is held back only until the artifact's digest shows it is a repeat of one already recorded), metadata, the change named (open; opened by the caller or the caller holds `write`), the declared base. Then the artifact is accepted under the repository's write lease, with an intent journaled first. **Nothing here seals, merges or publishes**: the work is held for review and no branch changes. If the accept does not finish, the answer is `503 incomplete` with the intent's id, and sending the same artifact again once the repository has recovered resumes it and records exactly one revision. A repeat of an artifact already recorded answers `200` with its revision.",
                "parameters": owner_name(),
                "security": [{"bearerToken": []}],
                "requestBody": {
                    "required": true,
                    "description": "`PLANETER-PROPOSE-1\\n` + 8-byte big-endian metadata length + metadata JSON + artifact bytes.",
                    "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}
                },
                "responses": {
                    "201": {
                        "description": "A proposal was opened (or revision 1 of it recorded).",
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Proposed"}}}
                    },
                    "200": {
                        "description": "A revision was added to an existing proposal, or the artifact was already recorded.",
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Proposed"}}}
                    }
                },
            }}),
            &[
                ErrorCode::Busy,
                ErrorCode::TransportOff,
                ErrorCode::RateLimited,
                ErrorCode::Forbidden,
                ErrorCode::LengthRequired,
                ErrorCode::PayloadTooLarge,
                ErrorCode::NeedsRecovery,
                ErrorCode::QuotaExceeded,
                ErrorCode::BaseUnknown,
                ErrorCode::DeclarationMissesAdded,
                ErrorCode::DeclaredNotHeld,
                ErrorCode::NoNewContent,
                ErrorCode::UnknownProposal,
                ErrorCode::ProposalNotOpen,
                ErrorCode::NotYourProposal,
                ErrorCode::AcceptRefused,
                ErrorCode::IntentPending,
                ErrorCode::Incomplete,
            ],
        ),
    );
    // RFC 004 part 4: recovery. One path, two operations: the plan (GET) and the run (POST).
    let recovery_errors = [
        ErrorCode::Busy,
        ErrorCode::TransportOff,
        ErrorCode::RateLimited,
        ErrorCode::Forbidden,
    ];
    let mut recovery = with_errors(
        json!({"get": {
            "operationId": "getRecovery",
            "summary": "What recovery would do for a repository, and what it has done",
            "description": "For whoever may **administer** the repository (`admin`; a stranger is told `404` exactly as for a hidden repository, a reader who may not administer `403`); bearer token only. **Read-only**: it runs `doctor` and the listing of `unlock` and changes nothing. The plan names the flag and its cause, the lease holders (`dead` = written by another boot of the server), the processes that are not the server's and name the repository, the open proposals (intents), the locks `unlock` lists, `doctor`'s verdict, the blockers (what disagrees and whether an administrator's run may go on), the exact commands for an operator, and whether the server could run it by itself. `recoveries` are the last runs with every step and the text of each prikk call.",
            "parameters": owner_name(),
            "security": [{"bearerToken": []}],
            "responses": {"200": {"description": "The plan and the last runs.", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/RecoveryState"}}}}},
        }}),
        &recovery_errors,
    );
    let run = with_errors(
        json!({"post": {
            "operationId": "postRecovery",
            "summary": "Run recovery for a repository",
            "description": "Runs the procedure of `docs/propose.md` for a repository that proposals flagged: the witnesses (the flag's cause, only dead lease holders, no process that is not the server's, `doctor` clean, `unlock` listing only locks of the kinds an accept leaves) are read, the repository's write lease is taken, the witnesses are read again under it, each listed lock is cleared (`unlock --lock … --force`; prikk's liveness line is never used), `doctor`'s named idempotent repairs are run when this request is an administrator's, a fresh `verify` must pass, the dead lease records are removed, the flag is cleared and the state fingerprint moves. Every step is written to a recovery record before it is taken. The body is optional: `{\"abandon_intents\": [\"…\"]}` abandons the named open proposals (their accepted patches stay in the repository; their artifacts leave the retained set). A run that stops leaves the flag set and changes nothing more; the answer is `200` with the `outcome` (`recovered`, `stopped`, `not_now`, `nothing_to_do`) and the plan as it then stood.",
            "parameters": owner_name(),
            "security": [{"bearerToken": []}],
            "requestBody": {
                "required": false,
                "content": {"application/json": {"schema": {"type": "object", "properties": {"abandon_intents": {"type": "array", "items": {"type": "string"}}}}}}
            },
            "responses": {"200": {"description": "What the run did.", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/RecoveryRun"}}}}},
        }}),
        &[
            ErrorCode::Busy,
            ErrorCode::TransportOff,
            ErrorCode::RateLimited,
            ErrorCode::Forbidden,
            ErrorCode::UnknownIntent,
        ],
    );
    recovery["get"]["responses"]["405"]["headers"]["Allow"]["description"] =
        json!("The methods this route supports: `GET` and `POST`.");
    if let (Some(item), Some(post)) = (recovery.as_object_mut(), run.get("post")) {
        item.insert("post".to_owned(), post.clone());
    }
    paths_obj.insert(paths::RECOVERY.into(), recovery);
    proposal_paths(&mut paths_obj);
    paths_obj.insert(
        paths::OPENAPI.into(),
        json!({"get": {
            "operationId": "getOpenApi",
            "summary": "This document",
            "security": [],
            "responses": {"200": {
                "description": "The OpenAPI document.",
                "content": {"application/json": {"schema": {"type": "object"}}}
            }}
        }}),
    );

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "planeter read API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Read-only JSON over a planeter repository. Every error has the one `Error` shape. Hidden and absent repositories are indistinguishable (`not_found`). Raw file bytes are served only on the content origin named on the raw operation. This document is in English; the `message` and `hint` of an error follow `Accept-Language` (the `code` and the shape never change)."
        },
        "paths": Value::Object(paths_obj),
        "components": {
            "securitySchemes": {
                "bearerToken": {"type": "http", "scheme": "bearer", "description": "A scoped personal access token."},
                "sessionCookie": {"type": "apiKey", "in": "cookie", "name": crate::cookies::SESSION_COOKIE,
                                  "description": "The browser session cookie set by sign-in."}
            },
            "schemas": schemas()
        }
    })
}
