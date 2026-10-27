//! Review over HTTP, the JSON side (RFC 005 part 6): comments, decisions, hiding, and the `review` and `comments` parts of the
//! proposal's detail. (A *proposal* is the code's *change*; see `docs/proposals.md`.)
//!
//! * The writes are **bearer-only** (a cookie is not looked at, like `propose` and `close`): the read gate first (a stranger
//!   is `404`, whatever else is or is not set up), then the caller's right to do this (`403` with the hint for it), then the
//!   service, which applies the rules of `planeter_core::review`.
//! * **A hidden comment's text leaves this module in one place only** — `visible_body` — and only for a caller who holds
//!   `RepoAdmin`. Everyone else gets `hidden: true`, the reason, and no `body` (HTML or source). The text is never logged.
//! * Existence-neutral: an unknown proposal, another repository's proposal, an unknown comment and another proposal's
//!   comment are all the same `404`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::Response;
use serde::Serialize;

use planeter_core::review::Verdict;
use planeter_core::{Action, Principal};
use planeter_store::{CommentRecord, RepositoryRecord};
use planeter_transport::{
    DecisionView, Limited, NewComment, ReviewError, ReviewService, ReviewView, ThreadsPage,
};

use crate::api::{
    AppState, Failure, code_response, code_response_detailed, code_response_with_keys,
    error_response,
};
use crate::errors::{ErrorCode, ErrorDetail};
use crate::gate::offload;
use crate::proposals::{json_ok, start_with};
use crate::transport::TransportRuntime;

/// The largest request body a comment takes: 16 KiB of text can need six times that once JSON has escaped it.
const COMMENT_BODY_MAX: usize = 128 * 1024;
/// The largest body of a decision or a hide.
const SMALL_BODY_MAX: usize = 16 * 1024;

fn reviews_of(rt: &Arc<TransportRuntime>) -> Option<Arc<ReviewService>> {
    rt.reviews.clone()
}

// ------------------------------------------------------------------------------------------------ views

#[derive(Serialize)]
pub struct BodyJson {
    /// The Markdown as written.
    source: String,
    /// The same, rendered through the sanitizer.
    html: String,
}

/// **The only place a comment's text is released.** A hidden comment's body goes to `can_see_hidden` callers and nobody else.
pub(crate) fn visible_body(c: &CommentRecord, can_see_hidden: bool) -> Option<&str> {
    if c.hidden.is_some() && !can_see_hidden {
        None
    } else {
        Some(&c.body)
    }
}

#[derive(Serialize)]
pub struct CommentJson {
    id: String,
    author: String,
    /// The revision that was the latest when it was written.
    revision: u32,
    created_at: u64,
    patch_id: Option<String>,
    path: Option<String>,
    line_from: Option<u32>,
    line_to: Option<u32>,
    /// Absent for a hidden comment, unless the caller administers the repository.
    body: Option<BodyJson>,
    hidden: bool,
    /// Why it was hidden: said to everyone.
    hidden_reason: Option<String>,
    /// Who hid it and when: said to people who administer the repository only.
    hidden_by: Option<String>,
    hidden_at: Option<u64>,
    replies: Vec<CommentJson>,
}

pub(crate) fn comment_json(
    c: &CommentRecord,
    replies: &[CommentRecord],
    can_see_hidden: bool,
) -> CommentJson {
    CommentJson {
        id: c.comment_id.clone(),
        author: c.author.clone(),
        revision: c.revision_seq,
        created_at: c.created_at,
        patch_id: c.patch_id.clone(),
        path: c.path.clone(),
        line_from: c.line_from,
        line_to: c.line_to,
        body: visible_body(c, can_see_hidden).map(|b| BodyJson {
            source: b.to_owned(),
            html: crate::sanitize::render_markdown(b),
        }),
        hidden: c.hidden.is_some(),
        hidden_reason: c.hidden.as_ref().map(|h| h.reason.clone()),
        hidden_by: c
            .hidden
            .as_ref()
            .filter(|_| can_see_hidden)
            .map(|h| h.by.clone()),
        hidden_at: c.hidden.as_ref().filter(|_| can_see_hidden).map(|h| h.at),
        replies: replies
            .iter()
            .map(|r| comment_json(r, &[], can_see_hidden))
            .collect(),
    }
}

#[derive(Serialize)]
pub struct CommentsJson {
    /// All the proposal's comments, replies included.
    total: u64,
    items: Vec<CommentJson>,
    /// The cursor to pass as `after` for the next page; absent on the last page.
    next: Option<String>,
}

pub(crate) fn comments_json(page: &ThreadsPage, can_see_hidden: bool) -> CommentsJson {
    CommentsJson {
        total: page.total,
        items: page
            .threads
            .iter()
            .map(|t| comment_json(&t.comment, &t.replies, can_see_hidden))
            .collect(),
        next: page.next.clone(),
    }
}

#[derive(Serialize)]
pub struct DecisionJson {
    reviewer: String,
    revision: u32,
    /// `approve` or `request-changes`.
    decision: &'static str,
    note: Option<String>,
    created_at: u64,
}

fn decision_json(d: &DecisionView) -> DecisionJson {
    DecisionJson {
        reviewer: d.reviewer.clone(),
        revision: d.revision,
        decision: d.verdict.as_str(),
        note: d.note.clone(),
        created_at: d.created_at,
    }
}

#[derive(Serialize)]
pub struct ReviewJson {
    /// `awaiting_review`, `approved` or `changes_requested`: a function of the current decisions on the latest revision.
    state: &'static str,
    /// The latest revision: the one the state is about.
    revision: u32,
    approved_count: usize,
    changes_requested_count: usize,
    /// The current decisions on the latest revision.
    decisions: Vec<DecisionJson>,
    /// Earlier revisions that hold an approval: history, counted for nothing now.
    approved_on: Vec<u32>,
    /// Earlier revisions that hold a request for changes.
    requested_changes_on: Vec<u32>,
    /// The current decisions on earlier revisions, with reviewers and notes.
    earlier_decisions: Vec<DecisionJson>,
}

pub(crate) fn review_json(v: &ReviewView) -> ReviewJson {
    ReviewJson {
        state: v.standing.state.as_str(),
        revision: v.latest,
        approved_count: v.standing.approved_by.len(),
        changes_requested_count: v.standing.changes_requested_by.len(),
        decisions: v.current.iter().map(decision_json).collect(),
        approved_on: v.standing.approved_on.clone(),
        requested_changes_on: v.standing.requested_changes_on.clone(),
        earlier_decisions: v.earlier.iter().map(decision_json).collect(),
    }
}

// ------------------------------------------------------------------------------------------------ refusals

fn rate_limited(state: &AppState, what: Limited, retry_after: u64) -> Response {
    let keys = match what {
        Limited::Comments => ErrorCode::RATE_COMMENTS,
        Limited::Decisions => ErrorCode::RATE_DECISIONS,
    };
    let mut resp = code_response_with_keys(state, ErrorCode::RateLimited, keys);
    if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

pub(crate) fn invalid_with_keys(
    state: &AppState,
    reason: &'static str,
    keys: crate::errors::ErrorKeys,
) -> Response {
    code_response_detailed(
        state,
        ErrorCode::InvalidRequest,
        keys,
        ErrorDetail {
            params: Vec::new(),
            fields: vec![("reason", reason.into())],
        },
    )
}

/// A review refusal as a response. `keys` are the `invalid_request` hint for the kind of request.
fn failure(state: &AppState, e: &ReviewError, keys: crate::errors::ErrorKeys) -> Response {
    use planeter_core::review::DecisionRefusal as D;
    match e {
        ReviewError::NotFound => {
            error_response(state, Failure::Read(planeter_core::ReadError::NotFound))
        }
        ReviewError::BadCursor => error_response(state, Failure::Invalid),
        ReviewError::Comment(r) => invalid_with_keys(state, r.reason(), keys),
        ReviewError::CommentsClosed(c) => code_response_detailed(
            state,
            ErrorCode::CommentsClosed,
            ErrorCode::CommentsClosed.keys(),
            ErrorDetail {
                params: vec![("date", crate::timefmt::date(c.until))],
                fields: vec![
                    ("state", c.state.as_str().into()),
                    ("kept_until", c.until.into()),
                ],
            },
        ),
        ReviewError::Decision(D::NotOpen(s)) => code_response_detailed(
            state,
            ErrorCode::DecisionsClosed,
            ErrorCode::DecisionsClosed.keys(),
            ErrorDetail {
                params: Vec::new(),
                fields: vec![("state", s.as_str().into())],
            },
        ),
        ReviewError::Decision(D::RevisionNotCurrent { current }) => code_response_detailed(
            state,
            ErrorCode::RevisionNotCurrent,
            ErrorCode::RevisionNotCurrent.keys(),
            ErrorDetail {
                params: vec![("current", current.to_string())],
                fields: vec![("current", (*current).into())],
            },
        ),
        ReviewError::Decision(D::SelfApproval(k)) => code_response_detailed(
            state,
            ErrorCode::SelfApproval,
            ErrorCode::SelfApproval.keys(),
            ErrorDetail {
                params: Vec::new(),
                fields: vec![("reason", k.reason().into())],
            },
        ),
        ReviewError::Decision(D::NoteTooLong) => {
            invalid_with_keys(state, "note_too_long", ErrorCode::BAD_DECISION)
        }
        ReviewError::HideReason(r) => invalid_with_keys(
            state,
            match r {
                planeter_core::review::HideRefusal::ReasonRequired => "reason_required",
                planeter_core::review::HideRefusal::ReasonTooLong => "reason_too_long",
            },
            ErrorCode::BAD_HIDE,
        ),
        ReviewError::CommentState { hidden } => code_response_detailed(
            state,
            ErrorCode::CommentState,
            ErrorCode::CommentState.keys(),
            ErrorDetail {
                params: Vec::new(),
                fields: vec![("hidden", (*hidden).into())],
            },
        ),
        ReviewError::RateLimited { what, retry_after } => rate_limited(state, *what, *retry_after),
        ReviewError::Unavailable(_) => code_response(state, ErrorCode::Unavailable),
    }
}

/// May this principal do `action` here? A failure to find out is "no".
async fn may(
    state: &AppState,
    principal: &Principal,
    record: &RepositoryRecord,
    action: Action,
) -> Result<bool, Box<Response>> {
    let (read, p, r) = (Arc::clone(&state.read), principal.clone(), record.clone());
    match offload(move || read.allowed(&p, &r, action)).await {
        Ok(Ok(b)) => Ok(b),
        _ => Err(Box::new(error_response(state, Failure::Internal))),
    }
}

/// Whether the caller may read the text of hidden comments (`RepoAdmin`); asked only when a page holds one.
pub(crate) async fn can_see_hidden(
    state: &AppState,
    principal: &Principal,
    record: &RepositoryRecord,
) -> bool {
    may(state, principal, record, Action::RepoAdmin)
        .await
        .unwrap_or(false)
}

// ------------------------------------------------------------------------------------------------ comment

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentBody {
    body: Option<String>,
    patch_id: Option<String>,
    path: Option<String>,
    line_from: Option<u32>,
    line_to: Option<u32>,
    parent: Option<String>,
}

fn parse<T: serde::de::DeserializeOwned>(
    state: &AppState,
    body: &[u8],
    keys: crate::errors::ErrorKeys,
) -> Result<T, Box<Response>> {
    serde_json::from_slice::<T>(body).map_err(|e| {
        let reason = if e.to_string().contains("unknown field") {
            "unknown_field"
        } else {
            "not_json"
        };
        Box::new(invalid_with_keys(state, reason, keys))
    })
}

pub(crate) async fn comment_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (principal, record, service) =
        match start_with(&state, peer, &headers, &owner, &name, false, reviews_of).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    match may(&state, &principal, &record, Action::Comment).await {
        Ok(true) => {}
        Ok(false) => {
            return code_response_with_keys(
                &state,
                ErrorCode::Forbidden,
                ErrorCode::FORBIDDEN_COMMENT,
            );
        }
        Err(r) => return *r,
    }
    let Some(author) = principal.user().map(|u| u.as_str().to_owned()) else {
        return code_response_with_keys(&state, ErrorCode::Forbidden, ErrorCode::FORBIDDEN_COMMENT);
    };
    if body.len() > COMMENT_BODY_MAX {
        return invalid_with_keys(&state, "too_large", ErrorCode::BAD_COMMENT);
    }
    let b: CommentBody = match parse(&state, &body, ErrorCode::BAD_COMMENT) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let new = NewComment {
        body: b.body.unwrap_or_default(),
        patch_id: b.patch_id,
        path: b.path,
        line_from: b.line_from,
        line_to: b.line_to,
        parent: b.parent,
    };
    let result = {
        let (record, id) = (record.clone(), id.clone());
        offload(move || service.add_comment(&record, &id, &author, &new)).await
    };
    match result {
        // The author sees what they wrote; it is not hidden.
        Ok(Ok(c)) => {
            let mut resp = json_ok(&state, &comment_json(&c, &[], false));
            *resp.status_mut() = axum::http::StatusCode::CREATED;
            resp
        }
        Ok(Err(e)) => failure(&state, &e, ErrorCode::BAD_COMMENT),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ decision

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionBody {
    decision: Option<String>,
    note: Option<String>,
    revision: Option<u32>,
}

pub(crate) async fn decision_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (principal, record, service) =
        match start_with(&state, peer, &headers, &owner, &name, false, reviews_of).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    match may(&state, &principal, &record, Action::Review).await {
        Ok(true) => {}
        Ok(false) => {
            return code_response_with_keys(
                &state,
                ErrorCode::Forbidden,
                ErrorCode::FORBIDDEN_DECISION,
            );
        }
        Err(r) => return *r,
    }
    let Some(reviewer) = principal.user().map(|u| u.as_str().to_owned()) else {
        return code_response_with_keys(
            &state,
            ErrorCode::Forbidden,
            ErrorCode::FORBIDDEN_DECISION,
        );
    };
    if body.len() > SMALL_BODY_MAX {
        return invalid_with_keys(&state, "too_large", ErrorCode::BAD_DECISION);
    }
    let b: DecisionBody = match parse(&state, &body, ErrorCode::BAD_DECISION) {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(verdict) = b.decision.as_deref().and_then(Verdict::parse) else {
        return invalid_with_keys(&state, "decision_unknown", ErrorCode::BAD_DECISION);
    };
    let result = {
        let (record, id) = (record.clone(), id.clone());
        let service = Arc::clone(&service);
        offload(move || {
            let d = service.decide(
                &record,
                &id,
                &reviewer,
                verdict,
                b.revision,
                b.note.as_deref(),
            )?;
            let review = service.review(&record, &id)?;
            Ok::<_, ReviewError>((d, review))
        })
        .await
    };
    match result {
        Ok(Ok((d, review))) => {
            #[derive(Serialize)]
            struct Out {
                decision: DecisionJson,
                review: ReviewJson,
            }
            json_ok(
                &state,
                &Out {
                    decision: DecisionJson {
                        reviewer: d.reviewer,
                        revision: d.revision_seq,
                        decision: verdict.as_str(),
                        note: d.note,
                        created_at: d.created_at,
                    },
                    review: review_json(&review),
                },
            )
        }
        Ok(Err(e)) => failure(&state, &e, ErrorCode::BAD_DECISION),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ hide and unhide

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HideBody {
    reason: Option<String>,
}

pub(crate) async fn hide_handler(
    State(state): State<AppState>,
    Path((owner, name, id, cid)): Path<(String, String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    moderate(state, owner, name, id, cid, peer, headers, Some(body)).await
}

pub(crate) async fn unhide_handler(
    State(state): State<AppState>,
    Path((owner, name, id, cid)): Path<(String, String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    moderate(state, owner, name, id, cid, peer, headers, None).await
}

#[allow(clippy::too_many_arguments)]
async fn moderate(
    state: AppState,
    owner: String,
    name: String,
    id: String,
    cid: String,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    hide_body: Option<Bytes>,
) -> Response {
    let (principal, record, service) =
        match start_with(&state, peer, &headers, &owner, &name, false, reviews_of).await {
            Ok(t) => t,
            Err(r) => return *r,
        };
    match may(&state, &principal, &record, Action::RepoAdmin).await {
        Ok(true) => {}
        Ok(false) => {
            return code_response_with_keys(
                &state,
                ErrorCode::Forbidden,
                ErrorCode::FORBIDDEN_HIDE,
            );
        }
        Err(r) => return *r,
    }
    let Some(by) = principal.user().map(|u| u.as_str().to_owned()) else {
        return code_response_with_keys(&state, ErrorCode::Forbidden, ErrorCode::FORBIDDEN_HIDE);
    };
    let reason = match &hide_body {
        None => None,
        Some(b) => {
            if b.len() > SMALL_BODY_MAX {
                return invalid_with_keys(&state, "too_large", ErrorCode::BAD_HIDE);
            }
            match parse::<HideBody>(&state, b, ErrorCode::BAD_HIDE) {
                Ok(h) => Some(h.reason.unwrap_or_default()),
                Err(r) => return *r,
            }
        }
    };
    let result = {
        let (record, id, cid) = (record.clone(), id.clone(), cid.clone());
        offload(move || match reason {
            Some(r) => service.hide(&record, &id, &cid, &by, &r),
            None => service.unhide(&record, &id, &cid, &by),
        })
        .await
    };
    match result {
        // The caller administers the repository: they may read what they just hid or showed.
        Ok(Ok(c)) => json_ok(&state, &comment_json(&c, &[], true)),
        Ok(Err(e)) => failure(&state, &e, ErrorCode::BAD_HIDE),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ the detail's query

/// The paging of the comments in the detail.
#[derive(Debug, serde::Deserialize)]
pub struct DetailQuery {
    pub(crate) limit: Option<usize>,
    pub(crate) after: Option<String>,
}

pub(crate) type DetailQueryResult = Result<Query<DetailQuery>, QueryRejection>;
