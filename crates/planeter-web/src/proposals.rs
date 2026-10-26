//! Proposals over HTTP, the JSON side (RFC 005 part 5; D-2, D-8, D-10, D-11): the list, the detail, closing, and the
//! download of an accepted artifact. (A *proposal* is the code's *change*: see `docs/proposals.md`.)
//!
//! * The list and the detail are `Authorized(Read)`: the one read gate first, so a hidden and an absent repository, and an id
//!   of another repository and an unknown one, are all the same `404`. Both are rate limited like the fetch routes, before
//!   the repository is looked at. **No prikk call is made per item**: the landing observation is one `sync pending` per
//!   repository per state fingerprint (and `log` only when some proposal's patches have all left pending).
//! * Closing is bearer-only (a cookie is not looked at, like `propose`), for the proposal's opener or whoever `authorize(Close)`
//!   allows; the reason is required.
//! * The download is for whoever holds `review` or `merge-seal`; it is served from the artifact store with the fetch route's
//!   machinery (`Range`, `ETag` = the digest) and **never under a repository lease**: the file is immutable.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Serialize;

use planeter_core::{Action, Assurance, Principal};
use planeter_store::{ChangeRecord, ChangeState, RepositoryRecord};
use planeter_transport::{
    Detail, Page, ProposalError, ProposalService, Retention, RevisionView, StateFilter, Summary,
};

use crate::api::{
    AppState, Failure, code_response, code_response_detailed, code_response_with_keys,
    error_response, with_security_headers,
};
use crate::errors::{ErrorCode, ErrorDetail};
use crate::gate::offload;
use crate::transport::{
    FileStream, RangeAsk, TransportRuntime, limits, none_match, parse_range, readable,
};

// ------------------------------------------------------------------------------------------------ views

/// What is left of the files of a proposal that left `open`.
#[derive(Serialize)]
pub struct RetentionView {
    /// `kept` (until `until`), `partly_removed` or `removed`. Absent for an open proposal.
    state: &'static str,
    /// Unix seconds: when the grace ends (the files are kept until then, and a sweep may remove them after).
    until: u64,
}

fn retention_view(r: Retention) -> Option<RetentionView> {
    match r {
        Retention::Open => None,
        Retention::Kept { until } => Some(RetentionView {
            state: "kept",
            until,
        }),
        Retention::Partly { until } => Some(RetentionView {
            state: "partly_removed",
            until,
        }),
        Retention::Removed { until } => Some(RetentionView {
            state: "removed",
            until,
        }),
    }
}

#[derive(Serialize)]
pub struct ProposalItem {
    id: String,
    title: String,
    /// `open`, `landed` or `closed`.
    state: &'static str,
    opened_by: String,
    opened_at: u64,
    updated_at: u64,
    revision_count: u32,
    /// How many patches the current set holds.
    patch_count: usize,
    /// Whether the latest revision says what it was built on.
    built_on: bool,
    retention: Option<RetentionView>,
}

fn item(s: &Summary) -> ProposalItem {
    ProposalItem {
        id: s.proposal.change_id.clone(),
        title: s.proposal.title.clone(),
        state: s.proposal.state.as_str(),
        opened_by: s.proposal.opened_by.clone(),
        opened_at: s.proposal.opened_at,
        updated_at: s.proposal.updated_at,
        revision_count: s.revisions,
        patch_count: s.patches,
        built_on: s.built_on,
        retention: retention_view(s.retention),
    }
}

#[derive(Serialize)]
pub struct ListView {
    proposals: Vec<ProposalItem>,
    /// The cursor to pass as `after` for the next page; absent on the last page.
    next: Option<String>,
}

impl From<&Page> for ListView {
    fn from(p: &Page) -> Self {
        ListView {
            proposals: p.items.iter().map(item).collect(),
            next: p.next.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct ClaimItem {
    claim_id: String,
    /// What `sync accept` reported for the claim, as prikk said it (stored, not re-asserted).
    outcome: String,
}

#[derive(Serialize)]
pub struct RevisionItem {
    seq: u32,
    created_at: u64,
    submitter: String,
    claims: Vec<ClaimItem>,
    author_key_ids: Vec<String>,
    /// The block the sender says the work was built on (the declared base); absent when none was given.
    built_on: Option<String>,
    /// Whether the sender declared the patches the revision contains (a plain push by prikk does not).
    declared: bool,
    /// How many patches this accept added.
    added_count: usize,
    artifact_digest: String,
    artifact_bytes: u64,
    /// The files are still in the store (derived each time).
    artifact_available: bool,
}

fn revision_item(r: &RevisionView) -> RevisionItem {
    RevisionItem {
        seq: r.record.seq,
        created_at: r.record.created_at,
        submitter: r.record.submitter.clone(),
        claims: r
            .record
            .claim_outcomes
            .iter()
            .map(|c| ClaimItem {
                claim_id: c.claim_id.clone(),
                outcome: c.outcome.clone(),
            })
            .collect(),
        author_key_ids: r.record.author_key_ids.clone(),
        built_on: r.record.declared_base.clone(),
        declared: r.record.declared_patch_ids.is_some(),
        added_count: r.record.added_patch_ids.len(),
        artifact_digest: r.record.artifact_digest.clone(),
        artifact_bytes: r.record.artifact_bytes,
        artifact_available: r.artifact_available,
    }
}

#[derive(Serialize)]
pub struct PatchItem {
    patch_id: String,
    /// `forge-approved-unsealed` (held by this server, nobody has signed it in), `sealed` (in a block the repository lists)
    /// or `null` (neither: nothing is claimed).
    assurance: Option<Assurance>,
}

#[derive(Serialize)]
pub struct SightingItem {
    #[serde(rename = "ref")]
    ref_name: String,
}

#[derive(Serialize)]
pub struct DescriptionView {
    /// The text as sent.
    source: String,
    /// The same, rendered as Markdown through the sanitizer.
    html: String,
}

#[derive(Serialize)]
pub struct DetailView {
    id: String,
    title: String,
    description: DescriptionView,
    state: &'static str,
    /// Why it was closed (a closed proposal).
    closed_reason: Option<String>,
    /// When the landing was observed (a landed proposal), unix seconds.
    landed_at: Option<u64>,
    /// The ref the sender hoped it would reach (advisory).
    target_ref: Option<String>,
    opened_by: String,
    opened_at: u64,
    updated_at: u64,
    retention: Option<RetentionView>,
    revisions: Vec<RevisionItem>,
    /// The patches of the current set, at most 1,000.
    patches: Vec<PatchItem>,
    patch_total: usize,
    truncated: bool,
    /// How many patches of the whole current set are still held by this server, not yet signed in.
    still_pending: usize,
    /// For an open proposal: the received refs whose logs hold any of its patches. Information only; no state is derived
    /// from a received ref (it is replaceable and unverified).
    received_sightings: Vec<SightingItem>,
    /// One plain sentence that is true under every answer to what happens after a maintainer signs the work.
    note: String,
}

pub(crate) fn detail_view(d: &Detail, t: &crate::catalog::T) -> DetailView {
    let p: &ChangeRecord = &d.proposal;
    DetailView {
        id: p.change_id.clone(),
        title: p.title.clone(),
        description: DescriptionView {
            source: p.description.clone(),
            html: crate::sanitize::render_markdown(&p.description),
        },
        state: p.state.as_str(),
        closed_reason: p.closed_reason.clone(),
        landed_at: (p.state == ChangeState::Landed).then_some(p.updated_at),
        target_ref: p.target_ref.clone(),
        opened_by: p.opened_by.clone(),
        opened_at: p.opened_at,
        updated_at: p.updated_at,
        retention: retention_view(d.retention),
        revisions: d.revisions.iter().map(revision_item).collect(),
        patches: d
            .patches
            .iter()
            .map(|x| PatchItem {
                patch_id: x.patch_id.clone(),
                assurance: x.assurance,
            })
            .collect(),
        patch_total: d.patch_total,
        truncated: d.truncated,
        still_pending: d.still_pending,
        received_sightings: d
            .sightings
            .iter()
            .map(|r| SightingItem {
                ref_name: r.clone(),
            })
            .collect(),
        note: t.get(crate::catalog::Key::ProposalsNoMerge),
    }
}

// ------------------------------------------------------------------------------------------------ the gate

/// What every handler of this module starts with, in the same order: the principal, the edge limits, the one read gate.
/// A cookie is looked at only when `cookie_ok`.
async fn start(
    state: &AppState,
    peer: crate::client_ip::PeerAddr,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
    cookie_ok: bool,
) -> Result<(Principal, RepositoryRecord, Arc<ProposalService>), Box<Response>> {
    let mut seen = headers.clone();
    if !cookie_ok {
        seen.remove(header::COOKIE);
    }
    let principal = crate::principal::principal_from_request(&seen, state).await;
    let runtime = state.transport.clone();
    if let Some(rt) = &runtime
        && let Some(refused) = limits(state, rt, peer, headers, &principal)
    {
        return Err(Box::new(refused));
    }
    // The one read gate first: a stranger is `404` whatever else is or is not set up.
    let record = match readable(state, &principal, owner, name).await {
        Ok(r) => r,
        Err(e) => return Err(Box::new(error_response(state, e))),
    };
    let Some(service) = runtime.as_ref().and_then(proposals_of) else {
        return Err(Box::new(code_response_with_keys(
            state,
            ErrorCode::TransportOff,
            ErrorCode::PROPOSE_OFF,
        )));
    };
    Ok((principal, record, service))
}

fn proposals_of(rt: &Arc<TransportRuntime>) -> Option<Arc<ProposalService>> {
    rt.proposals.clone()
}

fn failure(state: &AppState, e: &ProposalError) -> Response {
    match e {
        ProposalError::NotFound => {
            error_response(state, Failure::Read(planeter_core::ReadError::NotFound))
        }
        ProposalError::BadCursor => error_response(state, Failure::Invalid),
        ProposalError::ReasonRequired => code_response(state, ErrorCode::ReasonRequired),
        ProposalError::ReasonTooLong => invalid_with_reason(state, "reason_too_long"),
        ProposalError::AlreadyFinal(s) => code_response_detailed(
            state,
            ErrorCode::ProposalFinal,
            ErrorCode::ProposalFinal.keys(),
            ErrorDetail {
                params: Vec::new(),
                fields: vec![("state", s.as_str().into())],
            },
        ),
        ProposalError::Unavailable(_) => code_response(state, ErrorCode::Unavailable),
    }
}

fn invalid_with_reason(state: &AppState, reason: &'static str) -> Response {
    code_response_detailed(
        state,
        ErrorCode::InvalidRequest,
        ErrorCode::InvalidRequest.keys(),
        ErrorDetail {
            params: Vec::new(),
            fields: vec![("reason", reason.into())],
        },
    )
}

fn json_ok<T: Serialize>(state: &AppState, view: &T) -> Response {
    match serde_json::to_string(view) {
        Ok(body) => with_security_headers(
            state,
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json"),
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| code_response(state, ErrorCode::Internal)),
        Err(_) => code_response(state, ErrorCode::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ the list

#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    state: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
}

pub(crate) async fn list_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    q: Result<Query<ListQuery>, QueryRejection>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let (_principal, record, service) =
        match start(&state, peer, &headers, &owner, &name, true).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    let filter = match q.state.as_deref() {
        None => StateFilter::Open,
        Some(s) => match StateFilter::parse(s) {
            Some(f) => f,
            None => return invalid_with_reason(&state, "unknown_state"),
        },
    };
    let limit = match q.limit {
        None => planeter_transport::LIST_LIMIT_DEFAULT,
        Some(0) => return invalid_with_reason(&state, "limit_zero"),
        Some(n) => n.min(planeter_transport::LIST_LIMIT_MAX),
    };
    let after = q.after;
    let result = state
        .gate
        .run(move || service.list(&record, filter, limit, after.as_deref()))
        .await;
    match result {
        Ok(Ok(page)) => json_ok(&state, &ListView::from(&page)),
        Ok(Err(e)) => failure(&state, &e),
        Err(_) => error_response(&state, Failure::Busy),
    }
}

// ------------------------------------------------------------------------------------------------ the detail

pub(crate) async fn detail_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let (principal, record, service) =
        match start(&state, peer, &headers, &owner, &name, true).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    let _ = principal;
    let result = state.gate.run(move || service.detail(&record, &id)).await;
    match result {
        Ok(Ok(d)) => {
            // The note is the catalog's, in the caller's language; the response layer does not rewrite a 200 body, so
            // the language is negotiated here.
            let t = crate::catalog::T::new(crate::catalog::negotiate_from_headers(&headers));
            json_ok(&state, &detail_view(&d, &t))
        }
        Ok(Err(e)) => failure(&state, &e),
        Err(_) => error_response(&state, Failure::Busy),
    }
}

// ------------------------------------------------------------------------------------------------ closing

/// What a close answers: the proposal as it now stands.
#[derive(Serialize)]
pub struct ClosedView {
    id: String,
    state: &'static str,
    closed_reason: Option<String>,
    updated_at: u64,
    /// The files are kept at least until then (unix seconds).
    retain_until: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseBody {
    reason: Option<String>,
}

/// The largest request body a close takes (a reason is at most 1,000 characters).
const CLOSE_BODY_MAX: usize = 16 * 1024;

/// May this principal close this proposal? The opener, with a credential that permits closing; or whoever
/// `authorize(Close)` allows (`write` and above).
async fn may_close(
    state: &AppState,
    principal: &Principal,
    record: &RepositoryRecord,
    opener: &str,
) -> Result<bool, Box<Response>> {
    let (read, p, r, opener) = (
        Arc::clone(&state.read),
        principal.clone(),
        record.clone(),
        opener.to_owned(),
    );
    let verdict = offload(move || {
        if read.allowed(&p, &r, Action::Close)? {
            return Ok(true);
        }
        let is_opener = p.user().is_some_and(|u| u.as_str() == opener);
        Ok::<bool, planeter_core::ReadError>(
            is_opener && read.credential_permits(&p, &r, Action::Close),
        )
    })
    .await;
    match verdict {
        Ok(Ok(b)) => Ok(b),
        _ => Err(Box::new(error_response(state, Failure::Internal))),
    }
}

pub(crate) async fn close_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (principal, record, service) =
        match start(&state, peer, &headers, &owner, &name, false).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    // The proposal must exist here before anything is said about permission to close it.
    let opener = {
        let (service, record, id) = (Arc::clone(&service), record.clone(), id.clone());
        match offload(move || service.opener(&record, &id)).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return failure(&state, &e),
            Err(_) => return error_response(&state, Failure::Internal),
        }
    };
    match may_close(&state, &principal, &record, &opener).await {
        Ok(true) => {}
        Ok(false) => {
            return code_response_with_keys(
                &state,
                ErrorCode::Forbidden,
                ErrorCode::FORBIDDEN_CLOSE,
            );
        }
        Err(r) => return *r,
    }
    if body.len() > CLOSE_BODY_MAX {
        return invalid_with_reason(&state, "too_large");
    }
    let reason = match serde_json::from_slice::<CloseBody>(&body) {
        Ok(b) => b.reason.unwrap_or_default(),
        Err(e) if e.to_string().contains("unknown field") => {
            return invalid_with_reason(&state, "unknown_field");
        }
        Err(_) => return invalid_with_reason(&state, "not_json"),
    };
    let result = {
        let (service, record, id) = (Arc::clone(&service), record.clone(), id.clone());
        offload(move || service.close(&record, &id, &reason)).await
    };
    match result {
        Ok(Ok(closed)) => json_ok(
            &state,
            &ClosedView {
                id: closed.change_id,
                state: closed.state.as_str(),
                closed_reason: closed.closed_reason,
                updated_at: closed.updated_at,
                retain_until: closed.retain_until,
            },
        ),
        Ok(Err(e)) => failure(&state, &e),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ the download

/// A file name made of safe characters only.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) async fn artifact_handler(
    State(state): State<AppState>,
    Path((owner, name, id, seq)): Path<(String, String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let (principal, record, service) =
        match start(&state, peer, &headers, &owner, &name, true).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    // Who may download: whoever holds `review` or `merge-seal` (either). A reader who holds neither is told so.
    let allowed = {
        let (read, p, r) = (Arc::clone(&state.read), principal.clone(), record.clone());
        match offload(move || {
            Ok::<bool, planeter_core::ReadError>(
                read.allowed(&p, &r, Action::Review)? || read.allowed(&p, &r, Action::MergeSeal)?,
            )
        })
        .await
        {
            Ok(Ok(b)) => b,
            _ => return error_response(&state, Failure::Internal),
        }
    };
    if !allowed {
        return code_response_with_keys(&state, ErrorCode::Forbidden, ErrorCode::FORBIDDEN_REVIEW);
    }
    let Ok(seq) = seq.parse::<u32>() else {
        return error_response(&state, Failure::Read(planeter_core::ReadError::NotFound));
    };
    let found = {
        let (service, record, id) = (Arc::clone(&service), record.clone(), id.clone());
        offload(move || service.revision(&record, &id, seq)).await
    };
    let (proposal, revision) = match found {
        Ok(Ok((p, Some(r)))) => (p, r),
        Ok(Ok((_, None))) => {
            return error_response(&state, Failure::Read(planeter_core::ReadError::NotFound));
        }
        Ok(Err(e)) => return failure(&state, &e),
        Err(_) => return error_response(&state, Failure::Internal),
    };
    let digest = match planeter_transport::record_digest(&revision.artifact_digest) {
        Ok(d) => d,
        Err(_) => return code_response(&state, ErrorCode::Internal),
    };
    let Some(mut file) = service.artifacts().get(&digest) else {
        // Removed by the retention sweep (or never kept): the plain sentence and the date it was kept until.
        let until = proposal.retain_until.unwrap_or(proposal.updated_at);
        return code_response_detailed(
            &state,
            ErrorCode::ArtifactRemoved,
            ErrorCode::ArtifactRemoved.keys(),
            ErrorDetail {
                params: vec![("date", crate::timefmt::date(until))],
                fields: vec![("kept_until", until.into())],
            },
        );
    };
    let size = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return code_response(&state, ErrorCode::Unavailable),
    };
    let etag = format!("\"{}\"", digest.as_str());
    if none_match(&headers, &etag) {
        return with_security_headers(&state, Response::builder().status(StatusCode::NOT_MODIFIED))
            .header(header::ETAG, etag)
            .body(Body::empty())
            .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal));
    }
    let range = {
        let honour = headers
            .get(header::IF_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| v.trim() == etag);
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
    {
        use std::io::{Seek, SeekFrom};
        if first > 0 && file.seek(SeekFrom::Start(first)).is_err() {
            return code_response(&state, ErrorCode::Unavailable);
        }
    }
    let filename = format!(
        "{}-proposal-{}-r{}.prikk-sync",
        clean(&name),
        clean(&proposal.change_id),
        revision.seq
    );
    let mut builder = with_security_headers(&state, Response::builder().status(status))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .header(header::ETAG, etag)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len.to_string());
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {first}-{}/{size}", first + len - 1),
        );
    }
    builder
        .body(Body::from_stream(FileStream::new(file, len)))
        .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal))
}
