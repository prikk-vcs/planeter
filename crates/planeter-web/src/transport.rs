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
    CacheStatus, ExportService, FetchError, FetchRecord, Kind, Offer, Outcome, RefClass,
};

use crate::api::{
    AppState, Failure, code_response, code_response_with_keys, error_response, respond,
    try_both_owners, with_security_headers,
};
use crate::errors::ErrorCode;
use crate::gate::offload;

/// The transport's runtime: the export service and the edge limits. `None` in the application state means the
/// operator configured no export spool, and an authorized reader is told so (`transport_off`).
pub struct TransportRuntime {
    pub export: Arc<ExportService>,
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

fn client_key(state: &AppState, peer: crate::client_ip::PeerAddr, headers: &HeaderMap) -> String {
    crate::client_ip::client_ip(peer.0.map(|a| a.ip()), headers, &state.trusted_proxies)
        .map_or_else(|| "ip:unknown".to_owned(), |ip| format!("ip:{ip}"))
}

fn rate_limited(state: &AppState, retry_after: u64, export: Option<&ExportService>) -> Response {
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
fn limits(
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
async fn readable(
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
fn none_match(headers: &HeaderMap, etag: &str) -> bool {
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
enum RangeAsk {
    /// No range, an unsupported unit, several ranges or a malformed one: serve the whole artifact.
    Whole,
    /// A single satisfiable range: first and last byte, inclusive.
    Part(u64, u64),
    /// A single range that starts at or after the end.
    Unsatisfiable,
}

fn parse_range(value: Option<&str>, size: u64) -> RangeAsk {
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
struct FileStream {
    file: Option<File>,
    remaining: u64,
    pending: Option<tokio::task::JoinHandle<ChunkRead>>,
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
