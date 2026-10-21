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
use planeter_core::admission::{ReadVerb, ceiling_for};
use planeter_core::{CallLimits, Principal, RawFile, RawTarget, ReadError, ReadService};
use planeter_store::Owner;

use crate::errors::{ErrorBody, ErrorCode};
use crate::gate::{AdmitError, PrikkGate, offload};
use crate::routes::{Action, Class, Routes};
use crate::security::{ContentOrigin, app_security_headers, raw_content_headers};

/// Default and maximum history page size (bounded — `STD-3`).
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
/// The cap on raw file bytes served in one response (`cat --max-bytes`). Bounds the *response*; hostile
/// input is bounded upstream, at the accept edge (PK-24). Unchanged since before 0.3.0: the raw route is
/// not reduced to [`CONTENT_MAX_BYTES`], which bounds the `file` view (RFC 011 refinement R-3).
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

/// The read API's routes, with their authorization classes (IS-14). This is the only place API routes are
/// registered.
pub fn routes() -> Routes {
    // Every repository read goes through `ReadService`, whose first line is `authorize(_, Read, _)`.
    let read = Class::Authorized(Action::Read);
    let description = Class::public(
        "the API's own description: it names no repository and serves no repository data",
    );
    Routes::new()
        .route(paths::HISTORY, read, &["GET"], get(history_handler))
        .route(paths::CHANGE, read, &["GET"], get(change_handler))
        .route(paths::FILE, read, &["GET"], get(file_handler))
        .route(paths::TREE, read, &["GET"], get(tree_handler))
        .route(paths::RAW, read, &["GET"], get(raw_handler))
        .route(paths::REFS, read, &["GET"], get(refs_handler))
        .route(paths::VERIFY, read, &["GET"], get(verify_handler))
        .route(paths::OPENAPI, description, &["GET"], get(openapi_handler))
}

/// Build the read-API router.
pub fn router(state: AppState) -> Router {
    routes().build(state)
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
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::Log]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.history(
                &principal,
                o,
                &name,
                q.ref_name.as_deref(),
                Some(limit),
                adm.limits(ReadVerb::Log),
            )
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
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::Show]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.change(&principal, o, &name, &target, adm.limits(ReadVerb::Show))
        })
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
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::FileContent]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.file(
                &principal,
                o,
                &name,
                q.ref_name.as_deref(),
                &q.path,
                adm.limits(ReadVerb::FileContent),
            )
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
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::Tree]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.tree(
                &principal,
                o,
                &name,
                q.ref_name.as_deref(),
                q.prefix.as_deref(),
                adm.limits(ReadVerb::Tree),
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
        // Decided from the host alone: the same for a hidden, an absent and a public repository.
        return code_response(&state, ErrorCode::RawElsewhere);
    }
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::Cat]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.raw_file(
                &principal,
                o,
                &name,
                RawTarget {
                    point: q.ref_name.as_deref(),
                    path: &q.path,
                    max_bytes: Some(RAW_MAX_BYTES),
                },
                adm.limits(ReadVerb::Cat),
            )
        })
    })
    .await;
    match result {
        Ok(raw) => build_raw(&state, &raw),
        // A file above the raw cap: `too_large` with the hint that is true for this route (the content
        // origin is where the bytes would be served from, so "download it there" cannot help).
        Err(Failure::Read(ReadError::TooLarge)) => error_response(&state, Failure::RawTooLarge),
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
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::Refs]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.refs(&principal, o, &name, q.all, adm.limits(ReadVerb::Refs))
        })
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
    let subject = Subject::new(&principal, &owner, &name, &[ReadVerb::Verify]);
    let result = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&owner, |o| {
            read.verify(&principal, o, &name, adm.limits(ReadVerb::Verify))
        })
    })
    .await;
    respond(&state, &headers, result)
}

async fn openapi_handler(State(state): State<AppState>) -> Response {
    let doc = crate::openapi::document(state.content_origin.as_str());
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
    /// A view that could not be serialized: `500`, `internal`.
    Internal,
    /// The repository's estimated read memory is above its pool of the budget (RFC 011 D-1): `422`,
    /// `too_large`, with the hint "raise the budget". Distinct from `Read(TooLarge)`, which is a *file*
    /// above a bound.
    OverBudget,
    /// A raw file above the raw cap: `422`, `too_large`, with the raw route's own hint.
    RawTooLarge,
}

impl From<ReadError> for Failure {
    fn from(e: ReadError) -> Self {
        Failure::Read(e)
    }
}

/// Who is reading what — the inputs of admission (RFC 011 D-1).
pub(crate) struct Subject {
    principal: Principal,
    owner: String,
    name: String,
    /// Every verb the job will run; the admission cost is the largest of their costs (the calls run one
    /// after another inside the job, so only one child is alive at a time).
    verbs: &'static [ReadVerb],
}

impl Subject {
    pub(crate) fn new(
        principal: &Principal,
        owner: &str,
        name: &str,
        verbs: &'static [ReadVerb],
    ) -> Self {
        Self {
            principal: principal.clone(),
            owner: owner.to_owned(),
            name: name.to_owned(),
            verbs,
        }
    }
}

/// What the gate admitted: the repository's store size and the budget, from which each call's memory
/// ceiling follows (`cost × 1.5`, never above the budget — RFC 011 D-2).
pub(crate) struct Admitted {
    store_bytes: u64,
    budget: Option<u64>,
}

impl Admitted {
    /// The per-call limits for one prikk call of `verb` made inside the admitted job.
    pub(crate) fn limits(&self, verb: ReadVerb) -> CallLimits {
        CallLimits::data_ceiling(ceiling_for(verb.cost(self.store_bytes), self.budget))
    }
}

/// Run a blocking read-service job through the gate (RFC 003 D-8, IS-4; RFC 011 D-1). **Every** handler
/// reaches the read service through this one function, so that none can forget `spawn_blocking`, the cap
/// or the budget.
///
/// Two phases, in this order, so that neither `busy` nor `too_large` can reveal a hidden repository:
///
/// 1. **authorize and measure** (ungated, on the blocking pool): the one read gate, then a fresh walk of
///    the repository's store. A repository the principal may not read, or that does not exist, is
///    `NotFound` here — before any budget is looked at;
/// 2. **admit** by estimated memory ([`PrikkGate::run_weighted`]): the budget and a slot within one
///    bounded wait; a cost above its own pool of the budget is `too_large` at once.
///
/// A page that needs several prikk calls does them all inside one job, under one admission: it either
/// gets its budget and slot or is refused as a whole, never half-rendered.
pub(crate) async fn read_blocking<T, F>(
    state: &AppState,
    subject: Subject,
    job: F,
) -> Result<T, Failure>
where
    F: FnOnce(&ReadService, &Admitted) -> Result<T, ReadError> + Send + 'static,
    T: Send + 'static,
{
    let read = Arc::clone(&state.read);

    // Phase 1.
    let store_bytes = {
        let (read, principal, owner, name) = (
            Arc::clone(&read),
            subject.principal.clone(),
            subject.owner.clone(),
            subject.name.clone(),
        );
        offload(move || try_both_owners(&owner, |o| read.store_bytes(&principal, o, &name)))
            .await
            .map_err(|_| {
                Failure::Read(ReadError::Backend(
                    "the read task did not complete".to_owned(),
                ))
            })?
            .map_err(Failure::Read)?
    };

    // Phase 2.
    let cost = subject
        .verbs
        .iter()
        .map(|v| v.cost(store_bytes))
        .max()
        .unwrap_or(0);
    let what = subject
        .verbs
        .iter()
        .map(|v| v.name())
        .collect::<Vec<_>>()
        .join("+");
    let admitted = Admitted {
        store_bytes,
        budget: state.gate.budget_bytes(),
    };
    match state
        .gate
        .run_weighted(cost, &what, move || job(&read, &admitted))
        .await
    {
        Ok(result) => result.map_err(Failure::Read),
        Err(AdmitError::Busy) => Err(Failure::Busy),
        Err(AdmitError::TooLarge { .. }) => Err(Failure::OverBudget),
        // A panicking job or a shutting-down runtime: an internal fault, not a missing repository.
        Err(AdmitError::Aborted) => Err(Failure::Read(ReadError::Backend(
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
            // A view that cannot be serialized is an internal error, never an ETag over `null`.
            let (Ok(value), Ok(body)) = (serde_json::to_value(&view), serde_json::to_string(&view))
            else {
                return error_response(state, Failure::Internal);
            };
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

/// `respond` for a view alone (no request, no read), so a test can hand it a value whose
/// serialization fails.
#[doc(hidden)]
pub fn respond_for_test<T: Serialize>(state: &AppState, view: T) -> Response {
    respond(state, &HeaderMap::new(), Ok::<T, Failure>(view))
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
        Failure::Internal => ErrorCode::Internal,
        // IS-45: a missing path in a repository the reader may read is answered exactly as a missing
        // repository: the same status, headers and body, so the API's shape tells no one anything new.
        Failure::Read(ReadError::NotFound | ReadError::NoSuchPath) => ErrorCode::NotFound,
        Failure::Read(ReadError::Unavailable(_)) => ErrorCode::Unavailable,
        Failure::Read(ReadError::TooLarge) | Failure::OverBudget => ErrorCode::TooLarge,
        Failure::RawTooLarge => {
            return code_response_with_keys(state, ErrorCode::TooLarge, ErrorCode::TOO_LARGE_RAW);
        }
        // Backend and any future variant: an internal fault (existence-neutral).
        Failure::Read(_) => ErrorCode::Internal,
    };
    code_response(state, code)
}

/// A response for an [`ErrorCode`] (used by the router fallbacks too).
pub(crate) fn code_response(state: &AppState, code: ErrorCode) -> Response {
    code_response_with_keys(state, code, code.keys())
}

/// [`code_response`] with the message and hint keys chosen for the cause (see
/// [`ErrorCode::TOO_LARGE_RAW`]). **The one place an error body is built.** The body written here is the
/// English one; the response carries an [`ErrorBody`] extension, and the response-policy layer
/// ([`crate::headers`]), which sees the request, writes it again in the caller's language
/// (`Accept-Language`) and adds `Vary: Accept-Language` — so no handler needs the request's headers.
pub(crate) fn code_response_with_keys(
    state: &AppState,
    code: ErrorCode,
    keys: crate::errors::ErrorKeys,
) -> Response {
    let mut builder = with_security_headers(state, Response::builder().status(code.status()))
        .header(header::CONTENT_TYPE, "application/json")
        .extension(ErrorBody { code, keys });
    if code == ErrorCode::Busy {
        builder = builder.header(header::RETRY_AFTER, retry_after_secs(state).to_string());
    }
    if code == ErrorCode::MethodNotAllowed {
        builder = builder.header(header::ALLOW, "GET");
    }
    builder
        .body(Body::from(code.body_for(keys, crate::catalog::EN)))
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
