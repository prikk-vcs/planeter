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

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;

use planeter_auth::Authenticator;
use planeter_core::{RawFile, ReadError, ReadService};
use planeter_store::Owner;

use crate::errors::ErrorCode;
use crate::gate::{GateError, PrikkGate};
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
    /// The bound on concurrent prikk invocations and the place blocking prikk work runs (IS-4).
    pub gate: Arc<PrikkGate>,
    /// The bound on concurrent password verifications (IS-24); a separate gate from `gate`.
    pub auth_gate: Arc<crate::gate::AuthGate>,
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

/// The API's route paths. The router and the OpenAPI document both take their paths from here, and
/// `tests/api_contract.rs` holds an independent list, so they cannot drift unnoticed.
pub mod paths {
    pub const HISTORY: &str = "/api/v1/repos/{owner}/{name}/history";
    pub const CHANGE: &str = "/api/v1/repos/{owner}/{name}/change/{target}";
    pub const FILE: &str = "/api/v1/repos/{owner}/{name}/file";
    pub const TREE: &str = "/api/v1/repos/{owner}/{name}/tree";
    pub const RAW: &str = "/api/v1/repos/{owner}/{name}/raw";
    pub const REFS: &str = "/api/v1/repos/{owner}/{name}/refs";
    pub const VERIFY: &str = "/api/v1/repos/{owner}/{name}/verify";
    pub const OPENAPI: &str = "/api/v1/openapi.json";
    /// Every route, in document order.
    pub const ALL: [&str; 8] = [HISTORY, CHANGE, FILE, TREE, RAW, REFS, VERIFY, OPENAPI];
}

/// Build the read-API router. This is the only place API routes are registered.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(paths::HISTORY, get(history_handler))
        .route(paths::CHANGE, get(change_handler))
        .route(paths::FILE, get(file_handler))
        .route(paths::TREE, get(tree_handler))
        .route(paths::RAW, get(raw_handler))
        .route(paths::REFS, get(refs_handler))
        .route(paths::VERIFY, get(verify_handler))
        .route(paths::OPENAPI, get(openapi_handler))
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
    q: Result<Query<HistoryQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| {
            read.history(&principal, o, &name, q.ref_name.as_deref(), Some(limit))
        })
    })
    .await;
    respond(&state, &headers, result)
}

async fn change_handler(
    State(state): State<AppState>,
    Path((owner, name, target)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| read.change(&principal, o, &name, &target))
    })
    .await;
    respond(&state, &headers, result)
}

async fn file_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    q: Result<Query<FileQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| {
            read.file(&principal, o, &name, q.ref_name.as_deref(), &q.path)
        })
    })
    .await;
    respond(&state, &headers, result)
}

async fn tree_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    q: Result<Query<TreeQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| {
            read.tree(
                &principal,
                o,
                &name,
                q.ref_name.as_deref(),
                q.prefix.as_deref(),
            )
        })
    })
    .await;
    respond(&state, &headers, result)
}

async fn raw_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    q: Result<Query<RawQuery>, QueryRejection>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    // IS-16: raw bytes are served only on the content origin's host. Decided first, before the query
    // or the principal is looked at, and answered exactly as an unknown route is.
    let host =
        crate::client_ip::effective_host(peer.0.map(|a| a.ip()), &headers, &state.trusted_proxies);
    if !state.content_origin.serves_raw_on(host.as_deref()) {
        return code_response(&state, ErrorCode::NotFound);
    }
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| {
            read.raw_file(
                &principal,
                o,
                &name,
                q.ref_name.as_deref(),
                &q.path,
                Some(RAW_MAX_BYTES),
            )
        })
    })
    .await;
    match result {
        Ok(raw) => build_raw(&state, &raw),
        Err(e) => error_response(&state, e),
    }
}

async fn refs_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    q: Result<Query<RefsQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| read.refs(&principal, o, &name, q.all))
    })
    .await;
    respond(&state, &headers, result)
}

async fn verify_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let result = read_blocking(&state, move |read| {
        try_both_owners(&owner, |o| read.verify(&principal, o, &name))
    })
    .await;
    respond(&state, &headers, result)
}

async fn openapi_handler(State(state): State<AppState>) -> Response {
    let doc = crate::openapi::document();
    let body = serde_json::to_string(&doc).unwrap_or_default();
    build_json(&state, &body, None)
}

// -- helpers --

/// Why a read produced no value: the read service's own error, or the gate refusing for lack of a slot.
#[derive(Debug)]
pub(crate) enum Failure {
    Read(ReadError),
    /// No prikk slot became free within the bounded wait (IS-4): `503` with `Retry-After`.
    Busy,
    /// A malformed query (a missing or unparsable parameter): `400`, `invalid_request`.
    Invalid,
}

impl From<ReadError> for Failure {
    fn from(e: ReadError) -> Self {
        Failure::Read(e)
    }
}

/// Run a blocking read-service job through the gate (RFC 003 D-8, IS-4). **Every** handler reaches the
/// read service through this one function, so that none can forget `spawn_blocking` or the cap.
///
/// A page that needs several prikk calls does them all inside one job, under one permit: it either
/// gets its slot or is refused as a whole, never half-rendered.
pub(crate) async fn read_blocking<T, F>(state: &AppState, job: F) -> Result<T, Failure>
where
    F: FnOnce(&ReadService) -> Result<T, ReadError> + Send + 'static,
    T: Send + 'static,
{
    let read = Arc::clone(&state.read);
    match state.gate.run(move || job(&read)).await {
        Ok(result) => result.map_err(Failure::Read),
        Err(GateError::Busy) => Err(Failure::Busy),
        // A panicking job or a shutting-down runtime: an internal fault, not a missing repository.
        Err(GateError::Aborted) => Err(Failure::Read(ReadError::Backend(
            "the read task did not complete".to_owned(),
        ))),
    }
}

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
    result: Result<T, Failure>,
) -> Response {
    match result {
        Ok(view) => {
            let value = serde_json::to_value(&view).unwrap_or_default();
            let body = serde_json::to_string(&view).unwrap_or_default();
            let etag = etag_of_json(&value);
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

/// The API error response: the one JSON error shape, the security headers, and `Retry-After` for
/// `busy`. Status and body come from [`ErrorCode`], so nothing here can leak a path or tool output.
pub(crate) fn error_response(state: &AppState, e: Failure) -> Response {
    let code = match e {
        Failure::Busy => ErrorCode::Busy,
        Failure::Invalid => ErrorCode::InvalidRequest,
        Failure::Read(ReadError::NotFound) => ErrorCode::NotFound,
        Failure::Read(ReadError::Unavailable(_)) => ErrorCode::Unavailable,
        // Backend and any future variant: an internal fault (existence-neutral).
        Failure::Read(_) => ErrorCode::Internal,
    };
    code_response(state, code)
}

/// A response for an [`ErrorCode`] (used by the router fallbacks too).
pub(crate) fn code_response(state: &AppState, code: ErrorCode) -> Response {
    let mut builder = with_security_headers(state, Response::builder().status(code.status()))
        .header(header::CONTENT_TYPE, "application/json");
    if code == ErrorCode::Busy {
        builder = builder.header(header::RETRY_AFTER, retry_after_secs(state).to_string());
    }
    builder
        .body(Body::from(code.body()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response_fallback())
}

/// The `Retry-After` value for a refused request: the configured wait, at least one second.
pub(crate) fn retry_after_secs(state: &AppState) -> u64 {
    state.gate.wait().as_secs().max(1)
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

/// A strong validator over the **canonical** form of a JSON value: object keys sorted, no whitespace.
/// SHA-256 truncated to 128 bits, hex, quoted. The output is specified (unlike `DefaultHasher`, which
/// is not stable across compilers or releases) and does not depend on key order.
pub fn etag_of_json(value: &serde_json::Value) -> String {
    let mut canonical = String::new();
    write_canonical(value, &mut canonical);
    let digest = planeter_auth::sha256_hex(canonical.as_bytes());
    format!("\"{}\"", &digest[..32])
}

fn write_canonical(v: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
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
