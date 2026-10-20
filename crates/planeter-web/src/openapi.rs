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
    let responses = op["get"]["responses"].as_object_mut().expect("responses");
    for (k, v) in errs.as_object().expect("errors") {
        responses.insert(k.clone(), v.clone());
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
    json!({
        "Error": obj(&["code", "message", "hint"], json!({
            "code": {"type": "string", "enum": ErrorCode::ALL.iter().map(|c| c.code()).collect::<Vec<_>>(),
                     "description": "A stable, machine-readable code."},
            "message": {"type": "string", "description": "One plain sentence, in the language negotiated from `Accept-Language` (English by default). Never a path, a repository's existence, or tool output."},
            "hint": {"type": "string", "description": "What the caller can do next, in the same language."}
        })),
        "Assurance": {"type": "string", "enum": ["prikk-verified", "sealed", "forge-approved-unsealed", "unverified"], "description": "`prikk-verified` only where the repository's verification ran and passed; a sealed patch on the change endpoint is `sealed` (signed into the history; nothing was verified for that view); a queued one is `forge-approved-unsealed`."},
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
        "FailedCondition": obj(&["id", "message"], json!({"id": string_prop(), "message": string_prop()})),
        "Stage": obj(&["stage", "status", "detail"], json!({
            "stage": string_prop(), "status": string_prop(), "detail": nullable_string(),
            "blocked_by": {"type": "string", "description": "Only on a `not_evaluated` stage: the id of the stage that blocked it. Absent otherwise."},
            "after": {"type": "string", "description": "Only on a `halted` stage: the id of the stage it stopped after. Absent otherwise."}
        })),
        "VerifyView": obj(&["ok", "verified_sessions", "total_sessions", "failed_conditions", "stages"], json!({
            "ok": {"type": "boolean"},
            "verified_sessions": {"type": "integer", "minimum": 0},
            "total_sessions": {"type": "integer", "minimum": 0},
            "failed_conditions": array_of("FailedCondition"),
            "stages": array_of("Stage")
        })),
    })
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
