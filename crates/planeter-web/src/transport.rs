//! The transport's read side over HTTP (RFC 004 D-2, D-11, D-12): `GET …/transport/advertise` and
//! `GET …/transport/fetch?kind=archive|bundle[&ref=…]`.
//!
//! Both routes are `Authorized(Read)` through the route registry, answer a hidden and an absent repository
//! identically (`404`), and sit behind the rate limits of [`RateLimiter`], which are applied **before** the
//! repository is looked at and keyed by the caller only, so a rate-limit answer says nothing about any repository.
//! A fetch is a whole artifact exported once and **served from the spool file** by a chunked file stream: prikk's output
//! is never streamed to a client, so a slow client holds no lease and no process.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use futures_core::Stream;
use serde::Serialize;

use planeter_core::Principal;
use planeter_transport::{
    CacheStatus, ExportService, FetchError, FetchRecord, Kind, Offer, Outcome, ProposeError,
    ProposeMeta, ProposeService, RefClass, Submitter,
};

use crate::api::{
    AppState, Failure, code_response, code_response_detailed, code_response_with_keys,
    error_response, respond, try_both_owners, with_security_headers,
};
use crate::errors::ErrorCode;
use crate::gate::offload;

/// The transport's runtime: the export service and the edge limits. `None` in the application state means the
/// operator configured no export spool, and an authorized reader is told so (`transport_off`).
pub struct TransportRuntime {
    pub export: Arc<ExportService>,
    /// The write side (propose), when the operator also configured the artifact store; `None` otherwise.
    pub propose: Option<Arc<ProposeService>>,
    /// Recovery (RFC 004 part 4), when the write side is on; `None` otherwise.
    pub recovery: Option<Arc<planeter_transport::RecoveryService>>,
    /// What a person sees of the proposals (RFC 005 part 5: the list, the detail, closing, the download), when the write
    /// side is on; `None` otherwise.
    pub proposals: Option<Arc<planeter_transport::ProposalService>>,
    pub ip_limit: RateLimiter,
    pub principal_limit: RateLimiter,
}

/// A sliding-window counter per key: at most `max` requests in any `window`. Bounded in memory: when many keys have
/// been seen, the expired ones are dropped.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<u64>>>,
}

const MAX_KEYS: usize = 50_000;

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        RateLimiter {
            max,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Count one request of `key` at `now_unix` seconds. `Err(retry_after_secs)` when over the limit (the request is
    /// not counted then).
    pub fn check(&self, key: &str, now_unix: u64) -> Result<(), u64> {
        let window = self.window.as_secs().max(1);
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if hits.len() > MAX_KEYS {
            hits.retain(|_, q| {
                q.back()
                    .is_some_and(|t| now_unix.saturating_sub(*t) < window)
            });
        }
        let q = hits.entry(key.to_owned()).or_default();
        while q
            .front()
            .is_some_and(|t| now_unix.saturating_sub(*t) >= window)
        {
            q.pop_front();
        }
        if q.len() >= self.max as usize {
            let oldest = q.front().copied().unwrap_or(now_unix);
            return Err((oldest + window).saturating_sub(now_unix).max(1));
        }
        q.push_back(now_unix);
        Ok(())
    }
}

// ------------------------------------------------------------------------------------------------ views

#[derive(Serialize)]
pub struct OfferView {
    kind: &'static str,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    size: Option<u64>,
    etag: String,
}

#[derive(Serialize)]
pub struct RefView {
    name: String,
    /// `published` (`heads/…`, `tags/…`) or `received` (`remotes/…`).
    state: &'static str,
    /// `branch`, `tag` or `received`.
    kind: &'static str,
    closed: Option<bool>,
    tip_block: Option<String>,
    ref_state_id: Option<String>,
    fetch: Vec<OfferView>,
}

#[derive(Serialize)]
pub struct AdvertiseView {
    fingerprint: String,
    refs: Vec<RefView>,
    archive: OfferView,
}

fn offer_view(o: &Offer) -> OfferView {
    OfferView {
        kind: o.kind.as_str(),
        ref_name: o.ref_name.clone(),
        size: o.size,
        etag: o.etag.clone(),
    }
}

// ------------------------------------------------------------------------------------------------ helpers

pub(crate) fn client_key(
    state: &AppState,
    peer: crate::client_ip::PeerAddr,
    headers: &HeaderMap,
) -> String {
    crate::client_ip::client_ip(peer.0.map(|a| a.ip()), headers, &state.trusted_proxies)
        .map_or_else(|| "ip:unknown".to_owned(), |ip| format!("ip:{ip}"))
}

pub(crate) fn rate_limited(
    state: &AppState,
    retry_after: u64,
    export: Option<&ExportService>,
) -> Response {
    if let Some(e) = export {
        e.metrics().refusal("rate_limited");
    }
    let mut resp = code_response(state, ErrorCode::RateLimited);
    if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

/// Apply the per-IP and (for a signed-in caller) per-principal limits. `None` = within the limits.
pub(crate) fn limits(
    state: &AppState,
    rt: &TransportRuntime,
    peer: crate::client_ip::PeerAddr,
    headers: &HeaderMap,
    principal: &Principal,
) -> Option<Response> {
    let now = (state.now_unix)();
    if let Err(after) = rt.ip_limit.check(&client_key(state, peer, headers), now) {
        return Some(rate_limited(state, after, Some(&rt.export)));
    }
    if let Some(user) = principal.user()
        && let Err(after) = rt
            .principal_limit
            .check(&format!("user:{}", user.as_str()), now)
    {
        return Some(rate_limited(state, after, Some(&rt.export)));
    }
    None
}

/// The record of a repository the caller may read: the one read gate, existence-neutral.
pub(crate) async fn readable(
    state: &AppState,
    principal: &Principal,
    owner: &str,
    name: &str,
) -> Result<planeter_store::RepositoryRecord, Failure> {
    let (read, principal, owner, name) = (
        Arc::clone(&state.read),
        principal.clone(),
        owner.to_owned(),
        name.to_owned(),
    );
    offload(move || try_both_owners(&owner, |o| read.readable_record(&principal, o, &name)))
        .await
        .map_err(|_| Failure::Internal)?
        .map_err(Failure::Read)
}

/// A refusal of the export service, as a response: the named code, the hint that is true for the cause, and
/// `Retry-After` for `busy`. Counts the refusal.
fn fetch_failure(state: &AppState, export: &ExportService, e: &FetchError) -> Response {
    export.metrics().refusal(e.reason());
    match e {
        FetchError::Busy { retry_after } => {
            let mut resp = code_response(state, ErrorCode::Busy);
            if let Ok(v) = HeaderValue::from_str(&retry_after.as_secs().max(1).to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            resp
        }
        FetchError::RefRequired => {
            code_response_with_keys(state, ErrorCode::InvalidRequest, ErrorCode::REF_REQUIRED)
        }
        FetchError::UnknownRef => code_response(state, ErrorCode::UnknownRef),
        FetchError::BundleOfReceivedRef => code_response(state, ErrorCode::BundleOfReceivedRef),
        FetchError::NotVerified => code_response(state, ErrorCode::NotVerified),
        FetchError::Deadline => {
            code_response_with_keys(state, ErrorCode::ExportFailed, ErrorCode::EXPORT_DEADLINE)
        }
        FetchError::Ceiling => {
            code_response_with_keys(state, ErrorCode::ExportFailed, ErrorCode::EXPORT_CEILING)
        }
        FetchError::TooLargeForSpool => {
            code_response_with_keys(state, ErrorCode::ExportFailed, ErrorCode::SPOOL_TOO_SMALL)
        }
        FetchError::ExportFailed => code_response(state, ErrorCode::ExportFailed),
        FetchError::Unavailable => code_response(state, ErrorCode::Unavailable),
    }
}

// ------------------------------------------------------------------------------------------------ advertise

pub(crate) async fn advertise_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let runtime = state.transport.clone();
    if let Some(rt) = &runtime
        && let Some(refused) = limits(&state, rt, peer, &headers, &principal)
    {
        return refused;
    }
    let record = match readable(&state, &principal, &owner, &name).await {
        Ok(r) => r,
        Err(e) => return error_response(&state, e),
    };
    let Some(rt) = runtime else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let export = Arc::clone(&rt.export);
    let result = state.gate.run(move || export.advertise(&record)).await;
    let ad = match result {
        Ok(Ok(ad)) => ad,
        Ok(Err(e)) => return fetch_failure(&state, &rt.export, &e),
        Err(_) => return error_response(&state, Failure::Busy),
    };
    let view = AdvertiseView {
        fingerprint: ad.fingerprint,
        archive: offer_view(&ad.archive),
        refs: ad
            .refs
            .iter()
            .map(|(r, offers)| RefView {
                name: r.name.clone(),
                state: if r.class == RefClass::Received {
                    "received"
                } else {
                    "published"
                },
                kind: match r.class {
                    RefClass::Branch => "branch",
                    RefClass::Tag => "tag",
                    RefClass::Received => "received",
                },
                closed: (r.class == RefClass::Branch).then_some(r.closed),
                tip_block: r.tip_block.clone(),
                ref_state_id: r.ref_state_id.clone(),
                fetch: offers.iter().map(offer_view).collect(),
            })
            .collect(),
    };
    respond(&state, &headers, Ok::<_, Failure>(view))
}

// ------------------------------------------------------------------------------------------------ fetch

#[derive(Debug, serde::Deserialize)]
pub struct FetchQuery {
    kind: Option<String>,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
}

/// Does `If-None-Match` name `etag`? (`*`, a list, weak validators compared weakly.)
pub(crate) fn none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(str::trim)
                .any(|t| t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag)
        })
}

/// What a `Range` header asks of an artifact of `size` bytes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RangeAsk {
    /// No range, an unsupported unit, several ranges or a malformed one: serve the whole artifact.
    Whole,
    /// A single satisfiable range: first and last byte, inclusive.
    Part(u64, u64),
    /// A single range that starts at or after the end.
    Unsatisfiable,
}

pub(crate) fn parse_range(value: Option<&str>, size: u64) -> RangeAsk {
    let Some(v) = value.and_then(|v| v.trim().strip_prefix("bytes=")) else {
        return RangeAsk::Whole;
    };
    if v.contains(',') {
        return RangeAsk::Whole;
    }
    let Some((a, b)) = v.trim().split_once('-') else {
        return RangeAsk::Whole;
    };
    let (a, b) = (a.trim(), b.trim());
    match (a.is_empty(), b.is_empty()) {
        // `-n`: the last n bytes.
        (true, false) => match b.parse::<u64>() {
            Ok(0) => RangeAsk::Unsatisfiable,
            Ok(n) if size == 0 => {
                let _ = n;
                RangeAsk::Unsatisfiable
            }
            Ok(n) => RangeAsk::Part(size.saturating_sub(n), size - 1),
            Err(_) => RangeAsk::Whole,
        },
        // `a-` and `a-b`.
        (false, empty_b) => {
            let Ok(first) = a.parse::<u64>() else {
                return RangeAsk::Whole;
            };
            let last = if empty_b {
                u64::MAX
            } else {
                match b.parse::<u64>() {
                    Ok(l) if l >= first => l,
                    _ => return RangeAsk::Whole,
                }
            };
            if first >= size {
                RangeAsk::Unsatisfiable
            } else {
                RangeAsk::Part(first, last.min(size - 1))
            }
        }
        (true, true) => RangeAsk::Whole,
    }
}

const CHUNK: usize = 64 * 1024;

/// The result of one chunk read on the blocking pool: the file handed back, and the bytes read.
type ChunkRead = std::io::Result<(File, Vec<u8>)>;

/// A response body that reads `remaining` bytes of a spool file in chunks, each read on the blocking pool, so
/// the async workers never block on the disk and a slow client simply stops pulling.
pub(crate) struct FileStream {
    file: Option<File>,
    remaining: u64,
    pending: Option<tokio::task::JoinHandle<ChunkRead>>,
}

impl FileStream {
    pub(crate) fn new(file: File, remaining: u64) -> Self {
        FileStream {
            file: Some(file),
            remaining,
            pending: None,
        }
    }
}

impl Stream for FileStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(handle) = self.pending.as_mut() {
                return match Pin::new(handle).poll(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(joined) => {
                        self.pending = None;
                        match joined {
                            Ok(Ok((file, buf))) => {
                                if buf.is_empty() {
                                    self.remaining = 0;
                                    return Poll::Ready(None);
                                }
                                self.remaining -= buf.len() as u64;
                                self.file = Some(file);
                                Poll::Ready(Some(Ok(Bytes::from(buf))))
                            }
                            Ok(Err(e)) => {
                                self.remaining = 0;
                                Poll::Ready(Some(Err(e)))
                            }
                            Err(_) => {
                                self.remaining = 0;
                                Poll::Ready(Some(Err(std::io::Error::other("the read task ended"))))
                            }
                        }
                    }
                };
            }
            if self.remaining == 0 {
                return Poll::Ready(None);
            }
            let Some(mut file) = self.file.take() else {
                return Poll::Ready(None);
            };
            let want = (self.remaining.min(CHUNK as u64)) as usize;
            self.pending = Some(tokio::task::spawn_blocking(move || {
                let mut buf = vec![0u8; want];
                let mut got = 0;
                while got < want {
                    let n = file.read(&mut buf[got..])?;
                    if n == 0 {
                        break;
                    }
                    got += n;
                }
                buf.truncate(got);
                Ok((file, buf))
            }));
        }
    }
}

fn artifact_name(owner: &str, name: &str, kind: Kind, ref_name: Option<&str>) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    match (kind, ref_name) {
        (Kind::Archive, _) => format!("{}-{}.archive", clean(owner), clean(name)),
        (Kind::Bundle, r) => format!(
            "{}-{}-{}.bundle",
            clean(owner),
            clean(name),
            clean(r.unwrap_or("ref"))
        ),
    }
}

pub(crate) async fn fetch_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    q: Result<Query<FetchQuery>, QueryRejection>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let started = Instant::now();
    let principal = crate::principal::principal_from_request(&headers, &state).await;
    let runtime = state.transport.clone();
    if let Some(rt) = &runtime
        && let Some(refused) = limits(&state, rt, peer, &headers, &principal)
    {
        return refused;
    }
    // The one read gate first: a hidden and an absent repository are both `404`, before the query, the kind or
    // the configuration is looked at.
    let record = match readable(&state, &principal, &owner, &name).await {
        Ok(r) => r,
        Err(e) => return error_response(&state, e),
    };
    let Some(rt) = runtime else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let export = Arc::clone(&rt.export);
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let kind = match q.kind.as_deref() {
        None => return error_response(&state, Failure::Invalid),
        Some(k) => match Kind::parse(k) {
            Some(k) => k,
            None => {
                export.metrics().refusal("unknown_kind");
                return code_response(&state, ErrorCode::UnknownKind);
            }
        },
    };
    // The metrics and the log line name the repository by its opaque id (no owner or name beyond the access log).
    let repo_label = record.repo_id.as_str().to_owned();
    // The planned request: validated ref and the ETag of the current state, without any export.
    let plan = {
        let (export, record, ref_name) = (Arc::clone(&export), record.clone(), q.ref_name.clone());
        state
            .gate
            .run(move || export.plan(&record, kind, ref_name.as_deref()))
            .await
    };
    let (ref_name, etag) = match plan {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return fetch_failure(&state, &export, &e),
        Err(_) => {
            export.metrics().refusal("busy");
            return error_response(&state, Failure::Busy);
        }
    };
    // A client whose copy is current: no export, no bytes.
    if none_match(&headers, &etag) {
        export.metrics().fetch(
            &repo_label,
            &FetchRecord {
                kind: kind.as_str(),
                outcome: Outcome::Unchanged,
                bytes: 0,
                wall: started.elapsed(),
                lease_wait: Duration::ZERO,
                cache: CacheStatus::None,
            },
        );
        return with_security_headers(&state, Response::builder().status(StatusCode::NOT_MODIFIED))
            .header(header::ETAG, etag)
            .body(Body::empty())
            .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal));
    }
    // Export if needed (its own bound on concurrent exports, not the read gate's slots, so reads are never
    // starved by exports), then serve from the spool.
    let artifact = {
        let (export, record, ref_name) = (Arc::clone(&export), record.clone(), ref_name.clone());
        offload(move || export.artifact(&record, kind, ref_name.as_deref())).await
    };
    let artifact = match artifact {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => return fetch_failure(&state, &export, &e),
        Err(_) => return error_response(&state, Failure::Internal),
    };
    let size = artifact.size;
    let range = {
        // `If-Range`: a range applies only if the validator is current.
        let honour = headers
            .get(header::IF_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| v.trim() == artifact.etag);
        if honour {
            parse_range(
                headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
                size,
            )
        } else {
            RangeAsk::Whole
        }
    };
    if range == RangeAsk::Unsatisfiable {
        export.metrics().refusal("range_not_satisfiable");
        let mut resp = code_response(&state, ErrorCode::RangeNotSatisfiable);
        if let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}")) {
            resp.headers_mut().insert(header::CONTENT_RANGE, v);
        }
        return resp;
    }
    let (status, first, len) = match range {
        RangeAsk::Part(a, b) => (StatusCode::PARTIAL_CONTENT, a, b - a + 1),
        _ => (StatusCode::OK, 0, size),
    };
    let Some(mut file) = export.open(&artifact.digest) else {
        // Evicted between the export and now (a very small spool): the next request exports again.
        export.metrics().refusal("unavailable");
        return code_response(&state, ErrorCode::Unavailable);
    };
    if first > 0 && file.seek(SeekFrom::Start(first)).is_err() {
        return code_response(&state, ErrorCode::Unavailable);
    }
    export.metrics().fetch(
        &repo_label,
        &FetchRecord {
            kind: kind.as_str(),
            outcome: Outcome::Changed,
            bytes: len,
            wall: started.elapsed(),
            lease_wait: artifact.lease_wait,
            cache: artifact.cache,
        },
    );
    let filename = artifact_name(&owner, &name, kind, ref_name.as_deref());
    let mut builder = with_security_headers(&state, Response::builder().status(status))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .header(header::ETAG, artifact.etag)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len.to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {first}-{}/{size}", first + len - 1),
        );
    }
    let body = Body::from_stream(FileStream {
        file: Some(file),
        remaining: len,
        pending: None,
    });
    builder
        .body(body)
        .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal))
}

// ------------------------------------------------------------------------------------------------ propose

/// The first bytes of a proposal's body (RFC 004 part 3b T1).
const MAGIC: &[u8] = b"PLANETER-PROPOSE-1\n";
/// The magic and the 8-byte big-endian metadata length.
const PREFIX_LEN: u64 = MAGIC.len() as u64 + 8;
/// How long a client may be silent in the middle of an upload before it is cut off.
const UPLOAD_IDLE: Duration = Duration::from_secs(30);
/// The longest an upload may take in all.
const UPLOAD_TOTAL: Duration = Duration::from_secs(15 * 60);

/// The metadata of a proposal as it arrives. Unknown fields are refused.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MetaWire {
    proposal: Option<String>,
    title: Option<String>,
    description: Option<String>,
    target_ref: Option<String>,
    base: Option<String>,
    declared_patches: Option<Vec<String>>,
    client: Option<String>,
}

impl From<MetaWire> for ProposeMeta {
    fn from(w: MetaWire) -> Self {
        ProposeMeta {
            proposal: w.proposal,
            title: w.title,
            description: w.description,
            target_ref: w.target_ref,
            base: w.base,
            declared_patches: w.declared_patches,
            client: w.client,
        }
    }
}

type BodyStream = axum::body::BodyDataStream;

/// The next chunk of the body, or `None` at its end; an idle client is cut off.
async fn next_chunk(stream: &mut BodyStream, deadline: Instant) -> Result<Option<Bytes>, ()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let wait = UPLOAD_IDLE.min(remaining);
    if wait.is_zero() {
        return Err(());
    }
    let next = std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx));
    match tokio::time::timeout(wait, next).await {
        Ok(Some(Ok(b))) => Ok(Some(b)),
        Ok(Some(Err(_))) => Err(()),
        Ok(None) => Ok(None),
        Err(_) => Err(()),
    }
}

fn bad_framing(state: &AppState, export: &ExportService, reason: &'static str) -> Response {
    export.metrics().refusal("bad_framing");
    detailed(
        state,
        ErrorCode::InvalidRequest,
        ErrorCode::BAD_FRAMING,
        Vec::new(),
        vec![("reason", reason.into())],
    )
}

fn detailed(
    state: &AppState,
    code: ErrorCode,
    keys: crate::errors::ErrorKeys,
    params: Vec<(&'static str, String)>,
    fields: Vec<(&'static str, serde_json::Value)>,
) -> Response {
    code_response_detailed(
        state,
        code,
        keys,
        crate::errors::ErrorDetail { params, fields },
    )
}

/// A refusal of the propose service, as a response: the named code, the hint that is true for the cause, and
/// `Retry-After` for `busy`. Refusals that came out of the accept were counted by the service; the ones from the
/// checks are counted here.
fn propose_failure(state: &AppState, e: &ProposeError) -> Response {
    use planeter_core::change::DeriveError;
    let code = |c: ErrorCode| code_response(state, c);
    match e {
        ProposeError::Busy { retry_after } => {
            let mut resp = code_response(state, ErrorCode::Busy);
            if let Ok(v) = HeaderValue::from_str(&retry_after.as_secs().max(1).to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            resp
        }
        ProposeError::NeedsRecovery { .. } => code(ErrorCode::NeedsRecovery),
        ProposeError::Meta(m) => detailed(
            state,
            ErrorCode::InvalidRequest,
            ErrorCode::BAD_METADATA,
            Vec::new(),
            vec![("reason", m.reason().into())],
        ),
        ProposeError::UnknownProposal => code(ErrorCode::UnknownProposal),
        ProposeError::ProposalNotOpen(s) => detailed(
            state,
            ErrorCode::ProposalNotOpen,
            ErrorCode::ProposalNotOpen.keys(),
            Vec::new(),
            vec![("state", s.as_str().into())],
        ),
        ProposeError::NotYourProposal => code(ErrorCode::NotYourProposal),
        ProposeError::Quota(r) => {
            let (limit, used) = r.figures();
            let (message, hint) = crate::quota::keys(r);
            detailed(
                state,
                ErrorCode::QuotaExceeded,
                crate::errors::ErrorKeys { message, hint },
                vec![("limit", limit.to_string()), ("used", used.to_string())],
                vec![
                    ("quota", r.quota.name().into()),
                    ("limit", limit.into()),
                    ("used", used.into()),
                    (
                        "unit",
                        match r.quota {
                            planeter_core::change::Quota::OpenChanges
                            | planeter_core::change::Quota::RevisionsPerChange => "count",
                            _ => "mebibytes",
                        }
                        .into(),
                    ),
                ],
            )
        }
        ProposeError::BaseUnknown => code(ErrorCode::BaseUnknown),
        ProposeError::IntentHeldByAnother => code(ErrorCode::IntentPending),
        ProposeError::NoNewContent => code(ErrorCode::NoNewContent),
        ProposeError::Declaration(DeriveError::DeclarationMissesAdded { missing }) => detailed(
            state,
            ErrorCode::DeclarationMissesAdded,
            ErrorCode::DeclarationMissesAdded.keys(),
            Vec::new(),
            vec![
                ("count", missing.len().into()),
                (
                    "patch_ids",
                    missing.iter().take(100).cloned().collect::<Vec<_>>().into(),
                ),
            ],
        ),
        ProposeError::Declaration(DeriveError::DeclaredNotHeld { ids }) => detailed(
            state,
            ErrorCode::DeclaredNotHeld,
            ErrorCode::DeclaredNotHeld.keys(),
            Vec::new(),
            vec![
                ("count", ids.len().into()),
                (
                    "patch_ids",
                    ids.iter().take(100).cloned().collect::<Vec<_>>().into(),
                ),
            ],
        ),
        ProposeError::Declaration(_) => code(ErrorCode::Internal),
        ProposeError::Refused { text } => detailed(
            state,
            ErrorCode::AcceptRefused,
            ErrorCode::AcceptRefused.keys(),
            Vec::new(),
            vec![(
                "detail",
                text.trim().chars().take(2000).collect::<String>().into(),
            )],
        ),
        ProposeError::Incomplete { intent_id, cause } => detailed(
            state,
            ErrorCode::Incomplete,
            ErrorCode::Incomplete.keys(),
            Vec::new(),
            vec![
                ("intent", intent_id.clone().into()),
                ("cause", cause.reason().into()),
            ],
        ),
        ProposeError::Unavailable => code(ErrorCode::Unavailable),
    }
}

#[derive(Serialize)]
struct RevisionView {
    id: String,
    seq: u32,
}

#[derive(Serialize)]
struct ClaimView {
    claim_id: String,
    outcome: String,
}

#[derive(Serialize)]
struct PatchAssuranceView {
    patch_id: String,
    assurance: planeter_core::Assurance,
}

#[derive(Serialize)]
pub struct ProposedView {
    proposal_id: String,
    revision: RevisionView,
    created_proposal: bool,
    already_recorded: bool,
    claims: Vec<ClaimView>,
    added_count: u64,
    /// The first patches the push added, each classified from facts (`sync pending`, `log`), never from `queued`.
    patches: Vec<PatchAssuranceView>,
    declared: bool,
    artifact_bytes: u64,
    /// What this work is on the server: held, not sealed, not verified.
    assurance: planeter_core::Assurance,
    /// One plain sentence, in the caller's language: the work is held for review and no branch has changed.
    message: String,
}

/// [`planeter_core::ReadService::propose_access`] for a handle that may name a user or an organization: the two are
/// one namespace, so a `NotFound` for the user is retried as an org (as the reads do).
fn propose_access_both(
    read: &planeter_core::ReadService,
    principal: &Principal,
    handle: &str,
    name: &str,
) -> Result<(planeter_store::RepositoryRecord, bool), planeter_core::ProposeAccess> {
    match read.propose_access(
        principal,
        &planeter_store::Owner::User(handle.to_owned()),
        name,
    ) {
        Err(planeter_core::ProposeAccess::NotFound) => read.propose_access(
            principal,
            &planeter_store::Owner::Org(handle.to_owned()),
            name,
        ),
        other => other,
    }
}

pub(crate) async fn propose_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Body,
) -> Response {
    // A proposal is authenticated by a bearer token only. A session cookie is what a browser attaches to a request
    // another site makes, so it is not looked at here (the same door as everywhere else, without the cookie).
    let mut bearer_only = headers.clone();
    bearer_only.remove(header::COOKIE);
    let principal = crate::principal::principal_from_request(&bearer_only, &state).await;
    let runtime = state.transport.clone();
    if let Some(rt) = &runtime
        && let Some(refused) = limits(&state, rt, peer, &headers, &principal)
    {
        return refused;
    }
    // The one read gate and the one propose check: a hidden repository is `404` to a stranger, before the body, the
    // framing or the configuration is looked at; a reader who may not propose learns only that (`403`).
    let access = {
        let (read, principal, owner, name) = (
            Arc::clone(&state.read),
            principal.clone(),
            owner.clone(),
            name.clone(),
        );
        offload(move || propose_access_both(&read, &principal, &owner, &name)).await
    };
    let (record, can_write) = match access {
        Err(_) => return error_response(&state, Failure::Internal),
        Ok(Err(planeter_core::ProposeAccess::NotFound)) => {
            return error_response(&state, Failure::Read(planeter_core::ReadError::NotFound));
        }
        Ok(Err(planeter_core::ProposeAccess::NotPermitted)) => {
            if let Some(rt) = &runtime {
                rt.export.metrics().refusal("forbidden");
            }
            return code_response(&state, ErrorCode::Forbidden);
        }
        Ok(Err(_)) => return error_response(&state, Failure::Internal),
        Ok(Ok(found)) => found,
    };
    let Some(rt) = runtime else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let Some(propose) = rt.propose.clone() else {
        return code_response_with_keys(&state, ErrorCode::TransportOff, ErrorCode::PROPOSE_OFF);
    };
    let export = Arc::clone(&rt.export);
    let Some(user) = principal.user() else {
        return code_response(&state, ErrorCode::Forbidden);
    };
    let who = Submitter {
        id: user.as_str().to_owned(),
        can_write,
    };

    // A repository waiting for recovery takes no writes: said before the body is looked at (reads are unaffected).
    let writable = {
        let (propose, record) = (Arc::clone(&propose), record.clone());
        offload(move || propose.ensure_writable(&record)).await
    };
    match writable {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            export.metrics().refusal(e.reason());
            return propose_failure(&state, &e);
        }
        Err(_) => return error_response(&state, Failure::Internal),
    }

    // The framing, from the headers: the length is required and bounded before a byte of the body is read.
    if let Some(ct) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        && !ct.trim().eq_ignore_ascii_case("application/octet-stream")
    {
        return bad_framing(&state, &export, "content_type");
    }
    let Some(total) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
    else {
        export.metrics().refusal("length_required");
        return code_response(&state, ErrorCode::LengthRequired);
    };
    let cfg = propose.config();
    let too_large = |export: &ExportService| {
        export.metrics().refusal("payload_too_large");
        code_response(&state, ErrorCode::PayloadTooLarge)
    };
    if total > cfg.max_artifact_bytes + planeter_transport::propose::METADATA_MAX_BYTES + PREFIX_LEN
    {
        return too_large(&export);
    }
    if total < PREFIX_LEN {
        return bad_framing(&state, &export, "too_short");
    }

    // The prefix and the metadata, read as they arrive. Whatever follows the metadata in the last chunk read is the
    // start of the artifact (`carry`); the metadata itself is capped, so this buffer is too.
    let deadline = Instant::now() + UPLOAD_TOTAL;
    let mut stream = body.into_data_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut meta_len: Option<u64> = None;
    loop {
        let needed = PREFIX_LEN + meta_len.unwrap_or(0);
        if buf.len() as u64 >= needed {
            if meta_len.is_some() {
                break;
            }
            if &buf[..MAGIC.len()] != MAGIC {
                return bad_framing(&state, &export, "magic");
            }
            let mut n = [0u8; 8];
            n.copy_from_slice(&buf[MAGIC.len()..PREFIX_LEN as usize]);
            let m = u64::from_be_bytes(n);
            if m > planeter_transport::propose::METADATA_MAX_BYTES {
                return too_large(&export);
            }
            if PREFIX_LEN + m >= total {
                return bad_framing(&state, &export, "no_artifact");
            }
            meta_len = Some(m);
            continue;
        }
        match next_chunk(&mut stream, deadline).await {
            Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
            Ok(None) | Err(()) => return bad_framing(&state, &export, "truncated"),
        }
    }
    let carry = buf.split_off((PREFIX_LEN + meta_len.unwrap_or(0)) as usize);
    let m = meta_len.unwrap_or(0);
    let artifact_len = total - PREFIX_LEN - m;
    if artifact_len > cfg.max_artifact_bytes {
        return too_large(&export);
    }
    let wire: MetaWire = match serde_json::from_slice(&buf[PREFIX_LEN as usize..]) {
        Ok(w) => w,
        Err(e) => {
            let reason = if e.to_string().contains("unknown field") {
                "unknown_field"
            } else {
                "not_json"
            };
            export.metrics().refusal("bad_metadata");
            return detailed(
                &state,
                ErrorCode::InvalidRequest,
                ErrorCode::BAD_METADATA,
                Vec::new(),
                vec![("reason", reason.into())],
            );
        }
    };
    let meta: ProposeMeta = wire.into();

    // The checks that need no artifact byte.
    let plan = {
        let (propose, record, who, meta) = (
            Arc::clone(&propose),
            record.clone(),
            who.clone(),
            meta.clone(),
        );
        state
            .gate
            .run(move || propose.check(&record, &who, &meta, artifact_len))
            .await
    };
    let plan = match plan {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            export.metrics().refusal(e.reason());
            return propose_failure(&state, &e);
        }
        Err(_) => {
            export.metrics().refusal("busy");
            return error_response(&state, Failure::Busy);
        }
    };

    // The upload: streamed to disk and hashed, never held in memory.
    let started = {
        let propose = Arc::clone(&propose);
        offload(move || propose.start_upload(artifact_len)).await
    };
    let mut upload = match started {
        Ok(Ok(u)) => u,
        Ok(Err(e)) => {
            export.metrics().refusal(e.reason());
            return propose_failure(&state, &e);
        }
        Err(_) => return error_response(&state, Failure::Internal),
    };
    let mut pending_bytes = carry;
    loop {
        if !pending_bytes.is_empty() {
            let chunk = std::mem::take(&mut pending_bytes);
            let wrote = tokio::task::spawn_blocking(move || {
                let r = upload.write(&chunk);
                (upload, r)
            })
            .await;
            match wrote {
                Ok((u, Ok(()))) => upload = u,
                Ok((_, Err(_))) => return bad_framing(&state, &export, "length"),
                Err(_) => return error_response(&state, Failure::Internal),
            }
        }
        if upload.written() >= artifact_len {
            break;
        }
        match next_chunk(&mut stream, deadline).await {
            Ok(Some(chunk)) => pending_bytes = chunk.to_vec(),
            Ok(None) | Err(()) => return bad_framing(&state, &export, "truncated"),
        }
    }
    if upload.written() != artifact_len {
        return bad_framing(&state, &export, "length");
    }
    let uploaded = match offload(move || upload.finish()).await {
        Ok(Ok(u)) => u,
        _ => return error_response(&state, Failure::Internal),
    };

    // The accept: admitted against the same byte budget as reads (a write is never light), under the write lease.
    let cost = propose.config().accept_cost(artifact_len);
    let result = {
        let (propose, record, who, meta, plan) = (
            Arc::clone(&propose),
            record.clone(),
            who.clone(),
            meta.clone(),
            plan,
        );
        state
            .gate
            .run_weighted_heavy(cost, "propose", move || {
                propose.accept(&record, &who, &meta, &plan, uploaded)
            })
            .await
    };
    let proposed = match result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return propose_failure(&state, &e),
        Err(crate::gate::AdmitError::TooLarge { .. }) => {
            export.metrics().refusal("over_budget");
            return error_response(&state, Failure::OverBudget);
        }
        Err(crate::gate::AdmitError::Busy) => {
            export.metrics().refusal("busy");
            return error_response(&state, Failure::Busy);
        }
        Err(crate::gate::AdmitError::Aborted) => return error_response(&state, Failure::Internal),
    };
    let lang = crate::catalog::negotiate_from_headers(&headers);
    let key = if proposed.already_recorded {
        crate::catalog::Key::ProposeAlreadyRecorded
    } else {
        crate::catalog::Key::ProposeHeld
    };
    let view = ProposedView {
        proposal_id: proposed.proposal_id,
        revision: RevisionView {
            id: proposed.revision_id,
            seq: proposed.seq,
        },
        created_proposal: proposed.created_proposal,
        already_recorded: proposed.already_recorded,
        claims: proposed
            .claims
            .into_iter()
            .map(|c| ClaimView {
                claim_id: c.claim_id,
                outcome: c.outcome,
            })
            .collect(),
        added_count: proposed.added_count,
        patches: proposed
            .patches
            .into_iter()
            .map(|(patch_id, assurance)| PatchAssuranceView {
                patch_id,
                assurance,
            })
            .collect(),
        declared: proposed.declared,
        artifact_bytes: proposed.artifact_bytes,
        assurance: planeter_core::Assurance::ForgeApprovedUnsealed,
        message: crate::catalog::T::new(lang).msg(key).text(),
    };
    let status = if proposed.already_recorded || !proposed.created_proposal && proposed.seq > 1 {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let json = serde_json::to_vec(&view).unwrap_or_default();
    let mut resp = with_security_headers(&state, Response::builder().status(status))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(json))
        .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal));
    resp.extensions_mut().insert(crate::catalog::LanguageVaries);
    resp
}

// ------------------------------------------------------------------------------------------------ recovery

#[derive(Serialize)]
struct CauseView {
    code: &'static str,
    text: String,
}

#[derive(Serialize)]
struct HolderJson {
    holder_id: String,
    class: String,
    operation: String,
    since_unix: u64,
    dead: bool,
}

#[derive(Serialize)]
struct ProcJson {
    pid: u32,
    command: String,
}

#[derive(Serialize)]
struct IntentJson {
    intent_id: String,
    principal: String,
    started_at: u64,
    attempts: u32,
    artifact_bytes: u64,
    expired: bool,
}

#[derive(Serialize)]
struct LockJson {
    path: String,
    kind: String,
    recorded_pid: u32,
    liveness: String,
    expected: bool,
}

#[derive(Serialize)]
struct DoctorJson {
    errors: u32,
    warnings: u32,
    info: u32,
    clean: bool,
    repairs_named: Vec<String>,
    repairs_allowed: Vec<String>,
    repair_refusal: Option<String>,
}

#[derive(Serialize)]
struct BlockerJson {
    code: &'static str,
    text: String,
    detail: String,
    overridable: bool,
}

#[derive(Serialize)]
pub struct PlanJson {
    flagged: bool,
    flag_reason: Option<String>,
    flag_since: Option<u64>,
    cause: CauseView,
    holders: Vec<HolderJson>,
    processes: Vec<ProcJson>,
    processes_uninspected: usize,
    intents: Vec<IntentJson>,
    locks: Vec<LockJson>,
    doctor: Option<DoctorJson>,
    blockers: Vec<BlockerJson>,
    commands: Vec<String>,
    may_run_unattended: bool,
    steps: Vec<&'static str>,
}

#[derive(Serialize)]
struct StepJson {
    seq: u32,
    name: String,
    started_at: u64,
    ended_at: Option<u64>,
    outcome: Option<String>,
    detail: String,
}

#[derive(Serialize)]
struct RecordJson {
    recovery_id: String,
    actor: String,
    started_at: u64,
    ended_at: Option<u64>,
    outcome: Option<String>,
    steps: Vec<StepJson>,
}

#[derive(Serialize)]
pub struct RecoveryStateJson {
    plan: PlanJson,
    recoveries: Vec<RecordJson>,
}

#[derive(Serialize)]
pub struct RecoveryRunJson {
    outcome: &'static str,
    outcome_text: String,
    recovery_id: Option<String>,
    stopped: Option<String>,
    plan: PlanJson,
}

fn cause_key(code: &str) -> crate::catalog::Key {
    use crate::catalog::Key;
    match code {
        "accept_deadline" => Key::RecoveryCauseAcceptDeadline,
        "accept_ceiling" => Key::RecoveryCauseAcceptCeiling,
        "accept_failed" => Key::RecoveryCauseAcceptFailed,
        "accept_unreadable" => Key::RecoveryCauseAcceptUnreadable,
        "accept_not_recorded" => Key::RecoveryCauseAcceptNotRecorded,
        "server_stopped" => Key::RecoveryCauseServerStopped,
        _ => Key::RecoveryCauseOther,
    }
}

fn blocker_key(code: &str) -> crate::catalog::Key {
    use crate::catalog::Key;
    match code {
        "cause_not_ours" => Key::RecoveryBlockerCauseNotOurs,
        "lease_held" => Key::RecoveryBlockerLeaseHeld,
        "foreign_process" => Key::RecoveryBlockerForeignProcess,
        "doctor_problems" => Key::RecoveryBlockerDoctorProblems,
        "unexpected_lock" => Key::RecoveryBlockerUnexpectedLock,
        "too_many_open_intents" => Key::RecoveryBlockerTooManyOpenIntents,
        "processes_uninspected" => Key::RecoveryBlockerProcessesUninspected,
        _ => Key::RecoveryBlockerOutputUnreadable,
    }
}

fn outcome_key(o: planeter_transport::RecoveryOutcome) -> crate::catalog::Key {
    use crate::catalog::Key;
    use planeter_transport::RecoveryOutcome as O;
    match o {
        O::Recovered => Key::RecoveryOutcomeRecovered,
        O::Stopped => Key::RecoveryOutcomeStopped,
        O::NotNow => Key::RecoveryOutcomeNotNow,
        O::NothingToDo => Key::RecoveryOutcomeNothingToDo,
    }
}

fn plan_json(p: &planeter_transport::RecoveryPlan, lang: crate::catalog::Lang) -> PlanJson {
    let t = crate::catalog::T::new(lang);
    PlanJson {
        flagged: p.flagged,
        flag_reason: p.flag_reason.clone(),
        flag_since: p.flag_since,
        cause: CauseView {
            code: p.cause.code(),
            text: t.msg(cause_key(p.cause.code())).text(),
        },
        holders: p
            .holders
            .iter()
            .map(|h| HolderJson {
                holder_id: h.holder_id.clone(),
                class: h.class.clone(),
                operation: h.operation.clone(),
                since_unix: h.since_unix,
                dead: h.dead,
            })
            .collect(),
        processes: p
            .processes
            .iter()
            .map(|x| ProcJson {
                pid: x.pid,
                command: x.command.clone(),
            })
            .collect(),
        processes_uninspected: p.processes_uninspected,
        intents: p
            .intents
            .iter()
            .map(|i| IntentJson {
                intent_id: i.intent_id.clone(),
                principal: i.principal.clone(),
                started_at: i.started_at,
                attempts: i.attempts,
                artifact_bytes: i.artifact_bytes,
                expired: i.expired,
            })
            .collect(),
        locks: p
            .locks
            .iter()
            .map(|l| LockJson {
                path: l.path.clone(),
                kind: l.kind.clone(),
                recorded_pid: l.recorded_pid,
                liveness: l.liveness.clone(),
                expected: l.expected,
            })
            .collect(),
        doctor: p.doctor.as_ref().map(|d| DoctorJson {
            errors: d.errors,
            warnings: d.warnings,
            info: d.info,
            clean: d.clean,
            repairs_named: d.repairs_named.clone(),
            repairs_allowed: d.repairs_allowed.clone(),
            repair_refusal: d.repair_refusal.clone(),
        }),
        blockers: p
            .blockers
            .iter()
            .map(|b| BlockerJson {
                code: b.code,
                text: t.msg(blocker_key(b.code)).text(),
                detail: b.detail.chars().take(1000).collect(),
                overridable: b.overridable,
            })
            .collect(),
        commands: p.commands.clone(),
        may_run_unattended: p.may_run_unattended,
        steps: p.steps.clone(),
    }
}

fn record_json(r: &planeter_store::RecoveryRecord) -> RecordJson {
    RecordJson {
        recovery_id: r.recovery_id.clone(),
        actor: r.actor.clone(),
        started_at: r.started_at,
        ended_at: r.ended_at,
        outcome: r.outcome.clone(),
        steps: r
            .steps
            .iter()
            .map(|s| StepJson {
                seq: s.seq,
                name: s.name.clone(),
                started_at: s.started_at,
                ended_at: s.ended_at,
                outcome: s.outcome.clone(),
                detail: s.detail.clone(),
            })
            .collect(),
    }
}

/// The repository's record if the caller may administer it, else the response to give: `404` for a stranger (the
/// read gate, identical for a hidden and an absent repository), `403` for a reader who may not administer. The
/// request is authenticated by bearer token only, like every request that changes something.
// The error is the response to send, which is large by nature and sent once.
#[allow(clippy::result_large_err)]
async fn admin_gate(
    state: &AppState,
    peer: crate::client_ip::PeerAddr,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<
    (
        planeter_store::RepositoryRecord,
        Principal,
        Arc<planeter_transport::RecoveryService>,
    ),
    Response,
> {
    let mut bearer_only = headers.clone();
    bearer_only.remove(header::COOKIE);
    let principal = crate::principal::principal_from_request(&bearer_only, state).await;
    let runtime = state.transport.clone();
    if let Some(rt) = &runtime
        && let Some(refused) = limits(state, rt, peer, headers, &principal)
    {
        return Err(refused);
    }
    let access = {
        let (read, principal, owner, name) = (
            Arc::clone(&state.read),
            principal.clone(),
            owner.to_owned(),
            name.to_owned(),
        );
        offload(move || {
            match read.admin_access(
                &principal,
                &planeter_store::Owner::User(owner.clone()),
                &name,
            ) {
                Err(planeter_core::ProposeAccess::NotFound) => {
                    read.admin_access(&principal, &planeter_store::Owner::Org(owner), &name)
                }
                other => other,
            }
        })
        .await
    };
    let record = match access {
        Err(_) | Ok(Err(planeter_core::ProposeAccess::Backend(_))) => {
            return Err(error_response(state, Failure::Internal));
        }
        Ok(Err(planeter_core::ProposeAccess::NotFound)) => {
            return Err(error_response(
                state,
                Failure::Read(planeter_core::ReadError::NotFound),
            ));
        }
        Ok(Err(_)) => {
            return Err(code_response_with_keys(
                state,
                ErrorCode::Forbidden,
                ErrorCode::FORBIDDEN_ADMIN,
            ));
        }
        Ok(Ok(r)) => r,
    };
    let Some(rt) = runtime else {
        return Err(code_response(state, ErrorCode::TransportOff));
    };
    let Some(recovery) = rt.recovery.clone() else {
        return Err(code_response_with_keys(
            state,
            ErrorCode::TransportOff,
            ErrorCode::PROPOSE_OFF,
        ));
    };
    Ok((record, principal, recovery))
}

pub(crate) async fn recovery_plan_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let (record, _principal, recovery) =
        match admin_gate(&state, peer, &headers, &owner, &name).await {
            Ok(v) => v,
            Err(r) => return r,
        };
    let lang = crate::catalog::negotiate_from_headers(&headers);
    let made = {
        let (recovery, record) = (Arc::clone(&recovery), record.clone());
        state
            .gate
            .run(move || {
                let plan = recovery.plan(&record)?;
                Ok::<_, planeter_transport::RecoveryError>((
                    plan,
                    recovery.recent(&record.repo_id, 10),
                ))
            })
            .await
    };
    let (plan, recent) = match made {
        Ok(Ok(v)) => v,
        Ok(Err(_)) => return code_response(&state, ErrorCode::Unavailable),
        Err(_) => return error_response(&state, Failure::Busy),
    };
    let view = RecoveryStateJson {
        plan: plan_json(&plan, lang),
        recoveries: recent.iter().map(record_json).collect(),
    };
    let mut resp = respond(&state, &headers, Ok::<_, Failure>(view));
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp.extensions_mut().insert(crate::catalog::LanguageVaries);
    resp
}

/// The body of a run: `{ "abandon_intents": ["…"] }`, every field optional; at most 64 KiB.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RunBody {
    #[serde(default)]
    abandon_intents: Vec<String>,
}

pub(crate) async fn recovery_run_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (record, principal, recovery) =
        match admin_gate(&state, peer, &headers, &owner, &name).await {
            Ok(v) => v,
            Err(r) => return r,
        };
    let Ok(bytes) = axum::body::to_bytes(body, 64 * 1024).await else {
        return error_response(&state, Failure::Invalid);
    };
    let parsed: RunBody = if bytes.iter().all(u8::is_ascii_whitespace) {
        RunBody::default()
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(b) => b,
            Err(_) => return error_response(&state, Failure::Invalid),
        }
    };
    if parsed.abandon_intents.len() > 1000 {
        return error_response(&state, Failure::Invalid);
    }
    let Some(actor) = principal.user().map(|u| u.as_str().to_owned()) else {
        return code_response_with_keys(&state, ErrorCode::Forbidden, ErrorCode::FORBIDDEN_ADMIN);
    };
    let lang = crate::catalog::negotiate_from_headers(&headers);
    let ran = {
        let (recovery, record) = (Arc::clone(&recovery), record.clone());
        state
            .gate
            .run(move || recovery.recover(&record, &actor, &parsed.abandon_intents))
            .await
    };
    let result = match ran {
        Ok(Ok(r)) => r,
        Ok(Err(planeter_transport::RecoveryError::UnknownIntent(_))) => {
            return code_response(&state, ErrorCode::UnknownIntent);
        }
        Ok(Err(planeter_transport::RecoveryError::Unavailable)) => {
            return code_response(&state, ErrorCode::Unavailable);
        }
        Err(_) => return error_response(&state, Failure::Busy),
    };
    let view = RecoveryRunJson {
        outcome: result.outcome.code(),
        outcome_text: crate::catalog::T::new(lang)
            .msg(outcome_key(result.outcome))
            .text(),
        recovery_id: result.recovery_id,
        stopped: result.stopped,
        plan: plan_json(&result.plan, lang),
    };
    let json = serde_json::to_vec(&view).unwrap_or_default();
    let mut resp = with_security_headers(&state, Response::builder().status(StatusCode::OK))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(json))
        .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal));
    resp.extensions_mut().insert(crate::catalog::LanguageVaries);
    resp
}
