//! The read API (RFC 003 T3 / `STD-3`): GET-only HTTP endpoints over `planeter_core::ReadService`,
//! behind the one `authorize(_, Read, _)` gate. Every response carries the strict security headers
//! ([`crate::security`]); the honest view-models are emitted as JSON.
//!
//! Conventions established here (later RFCs extend the API): bounded page size, `ETag`/`If-None-Match`
//! → `304`, and existence-neutral errors — a denied private repo and a missing one both return `404`
//! (T-7). Authentication is by `Authorization: Bearer <token>` (else anonymous); interactive session/
//! OIDC sign-in is the web-UI's path, not the API's.
//!
//! Cursor/`Link`-header pagination and enforced rate-limit headers are reserved follow-ons (documented in
//! the review package) — this module ships the bounded-`limit` + `ETag` conventions honestly rather than
//! emitting rate-limit numbers it does not enforce.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;

use planeter_auth::Authenticator;
use planeter_core::{RawFile, ReadError, ReadService};
use planeter_store::Owner;

use crate::security::{ContentOrigin, app_security_headers, raw_content_headers};

/// Default and maximum history page size (bounded — `STD-3`).
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
/// The cap on raw file bytes served in one response (`cat --max-bytes`). Bounds the *response*; hostile
/// input is bounded upstream, at the accept edge (PK-24).
const RAW_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Shared application state (cheap to clone — all `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub read: Arc<ReadService>,
    pub auth: Arc<Authenticator>,
    pub content_origin: ContentOrigin,
    /// Browser sessions (the UI's sign-in); the API accepts the session cookie too.
    pub sessions: Arc<dyn planeter_auth::SessionStore>,
    /// Injected clock (tests pin it).
    pub now_unix: fn() -> u64,
    /// Per-account sign-in throttle (threat model C-2b).
    pub login_throttle: Arc<planeter_auth::LoginThrottle>,
    /// Per-client-IP sign-in throttle (C-2b; needs a trusted proxy to know the client).
    pub ip_throttle: Arc<planeter_auth::LoginThrottle>,
    /// The operator's trusted reverse proxies (client IP derivation + the bind rule).
    pub trusted_proxies: Arc<crate::client_ip::TrustedProxies>,
    /// The OIDC provider, when the operator configured one (SSO sign-in).
    pub oidc: Option<Arc<planeter_auth::OidcProvider>>,
}

/// Build the read-API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/repos/{owner}/{name}/history", get(history_handler))
        .route(
            "/api/v1/repos/{owner}/{name}/change/{target}",
            get(change_handler),
        )
        .route("/api/v1/repos/{owner}/{name}/file", get(file_handler))
        .route("/api/v1/repos/{owner}/{name}/tree", get(tree_handler))
        .route("/api/v1/repos/{owner}/{name}/raw", get(raw_handler))
        .route("/api/v1/repos/{owner}/{name}/refs", get(refs_handler))
        .route("/api/v1/repos/{owner}/{name}/verify", get(verify_handler))
        .route("/api/v1/openapi.json", get(openapi_handler))
        .with_state(state)
}

// -- request query models --

#[derive(Debug, serde::Deserialize)]
pub struct HistoryQuery {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
pub struct FileQuery {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    path: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct RefsQuery {
    #[serde(default)]
    all: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct TreeQuery {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    prefix: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RawQuery {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    path: String,
}

// -- handlers --

async fn history_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let result = try_both_owners(&owner, |o| {
        state
            .read
            .history(&principal, o, &name, q.ref_name.as_deref(), Some(limit))
    });
    respond(&state, &headers, result)
}

async fn change_handler(
    State(state): State<AppState>,
    Path((owner, name, target)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let result = try_both_owners(&owner, |o| state.read.change(&principal, o, &name, &target));
    respond(&state, &headers, result)
}

async fn file_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<FileQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let result = try_both_owners(&owner, |o| {
        state
            .read
            .file(&principal, o, &name, q.ref_name.as_deref(), &q.path)
    });
    respond(&state, &headers, result)
}

async fn tree_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<TreeQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let result = try_both_owners(&owner, |o| {
        state.read.tree(
            &principal,
            o,
            &name,
            q.ref_name.as_deref(),
            q.prefix.as_deref(),
        )
    });
    respond(&state, &headers, result)
}

async fn raw_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<RawQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let result = try_both_owners(&owner, |o| {
        state.read.raw_file(
            &principal,
            o,
            &name,
            q.ref_name.as_deref(),
            &q.path,
            Some(RAW_MAX_BYTES),
        )
    });
    match result {
        Ok(raw) => build_raw(&state, &raw),
        Err(e) => error_response(&state, e),
    }
}

async fn refs_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<RefsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let result = try_both_owners(&owner, |o| state.read.refs(&principal, o, &name, q.all));
    respond(&state, &headers, result)
}

async fn verify_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state);
    let result = try_both_owners(&owner, |o| state.read.verify(&principal, o, &name));
    respond(&state, &headers, result)
}

async fn openapi_handler(State(state): State<AppState>) -> Response {
    let doc = openapi_doc();
    let body = serde_json::to_string(&doc).unwrap_or_default();
    build_json(&state, &body, None)
}

// -- helpers --

/// Try the handle as a user, then as an org. A `NotFound` for the user is retried as an org (the two are
/// a shared namespace); any other outcome is returned as-is.
pub(crate) fn try_both_owners<T>(
    handle: &str,
    f: impl Fn(&Owner) -> Result<T, ReadError>,
) -> Result<T, ReadError> {
    match f(&Owner::User(handle.to_owned())) {
        Err(ReadError::NotFound) => f(&Owner::Org(handle.to_owned())),
        other => other,
    }
}

/// Turn a read result into a response: JSON + security headers + ETag/304 on success; a status-only
/// (existence-neutral) error otherwise.
fn respond<T: Serialize>(
    state: &AppState,
    req_headers: &HeaderMap,
    result: Result<T, ReadError>,
) -> Response {
    match result {
        Ok(view) => {
            let body = serde_json::to_string(&view).unwrap_or_default();
            let etag = etag_of(&body);
            // Conditional GET: unchanged → 304, no body.
            if req_headers
                .get(header::IF_NONE_MATCH)
                .and_then(|h| h.to_str().ok())
                .is_some_and(|m| m == etag)
            {
                return with_security_headers(
                    state,
                    Response::builder().status(StatusCode::NOT_MODIFIED),
                )
                .header(header::ETAG, etag)
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response_fallback());
            }
            build_json(state, &body, Some(etag))
        }
        Err(e) => error_response(state, e),
    }
}

fn build_json(state: &AppState, body: &str, etag: Option<String>) -> Response {
    let mut builder = with_security_headers(
        state,
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json"),
    );
    if let Some(tag) = etag {
        builder = builder.header(header::ETAG, tag);
    }
    builder
        .body(Body::from(body.to_owned()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response_fallback())
}

/// Serve raw repository bytes as an inert download (isolated-content-origin headers, T4). The bytes are
/// never rendered; `Content-Disposition: attachment` + `nosniff` + a `sandbox` CSP force a download.
fn build_raw(_state: &AppState, raw: &RawFile) -> Response {
    let filename = raw.path.rsplit('/').next().unwrap_or(&raw.path);
    let mut builder = Response::builder().status(StatusCode::OK);
    for (name, value) in raw_content_headers(filename) {
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from(raw.bytes.clone()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response_fallback())
}

fn error_response(state: &AppState, e: ReadError) -> Response {
    let status = match e {
        ReadError::NotFound => StatusCode::NOT_FOUND,
        ReadError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        // Backend and any future variant map to 500 (existence-neutral).
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    with_security_headers(state, Response::builder().status(status))
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response_fallback())
}

pub(crate) fn with_security_headers(
    state: &AppState,
    mut builder: axum::http::response::Builder,
) -> axum::http::response::Builder {
    for (name, value) in app_security_headers(&state.content_origin) {
        builder = builder.header(name, value);
    }
    builder
}

fn etag_of(body: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    format!("\"{:x}\"", h.finish())
}

/// A minimal OpenAPI 3.0 description of the read API.
fn openapi_doc() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "planeter read API", "version": "0.1.0"},
        "paths": {
            "/api/v1/repos/{owner}/{name}/history": {"get": {"summary": "Repository history"}},
            "/api/v1/repos/{owner}/{name}/change/{target}": {"get": {"summary": "A change's content"}},
            "/api/v1/repos/{owner}/{name}/file": {"get": {"summary": "A file's content at a ref"}},
            "/api/v1/repos/{owner}/{name}/tree": {"get": {"summary": "Directory listing at a ref"}},
            "/api/v1/repos/{owner}/{name}/raw": {"get": {"summary": "Raw file bytes (download)"}},
            "/api/v1/repos/{owner}/{name}/refs": {"get": {"summary": "Branches and tags"}},
            "/api/v1/repos/{owner}/{name}/verify": {"get": {"summary": "Verify status"}}
        }
    })
}

/// Small shim so the `unwrap_or_else` fallbacks above type-check to `Response`.
trait IntoResponseFallback {
    fn into_response_fallback(self) -> Response;
}
impl IntoResponseFallback for StatusCode {
    fn into_response_fallback(self) -> Response {
        Response::builder()
            .status(self)
            .body(Body::empty())
            .expect("status-only response is always valid")
    }
}
