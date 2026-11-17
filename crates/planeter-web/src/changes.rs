//! The file-by-file view of a revision, the JSON side (RFC 005 part 6, handoff 1 of 2): the list of changed files and one file's diff.
//!
//! * **Authorization** is the proposal's own: the read class `GET …/proposals/{id}` uses, then the one read gate first, so a hidden repository,
//!   an unknown proposal and an unknown revision are all the same `404`, byte for byte.
//! * **Cost.** A view rebuilds two trees with prikk (`extract`), so it is admitted by the existing `extract` cost (RFC 011) from the **heavy** pool, with
//!   no new setting; both extractions run under the one admission. Concurrent requests for one revision take turns *before* asking for memory, so the second
//!   finds the first's answer in the cache instead of extracting again. A cached answer needs no admission at all.
//! * Nothing here signs, selects, merges or changes a repository, and no file name or content is ever logged or counted.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::Response;
use serde::{Deserialize, Serialize};

use planeter_core::admission::ceiling_for;
use planeter_store::RevisionRecord;
use planeter_transport::changes::diff::{CONTEXT, MAX_DIFF_LINES, MAX_EDIT_DISTANCE};
use planeter_transport::changes::model::{ChangeSet, Compared, FileDiff, FileEntry, Totals};
use planeter_transport::changes::tree::TEXT_MAX;
use planeter_transport::{ChangesError, ChangesReason, ChangesService};

use crate::api::{
    AppState, Failure, code_response_detailed, code_response_with_keys, error_response,
};
use crate::catalog::Key;
use crate::errors::{ErrorCode, ErrorDetail, ErrorKeys};
use crate::gate::{AdmitError, offload};
use crate::proposals::{failure, invalid_with_reason, json_ok, start};

/// Files per page of the list: the default and the most.
pub const LIST_DEFAULT: usize = 100;
pub const LIST_MAX: usize = 200;

#[derive(Debug, Deserialize)]
pub struct ChangesQuery {
    after: Option<String>,
    limit: Option<usize>,
}

type QueryResult = Result<Query<ChangesQuery>, axum::extract::rejection::QueryRejection>;

/// The effective limits, so a reader knows what a missing diff means.
#[derive(Serialize)]
struct LimitsView {
    /// A side of a text diff is read up to this many bytes; a larger file has no diff (`too_large`).
    text_bytes: u64,
    /// Lines returned per file, context included.
    diff_lines: usize,
    context_lines: usize,
    /// The edit distance explored before a file is shown as a whole replacement (`approximate`).
    edit_distance: usize,
    /// Changed files listed; beyond it the view is `tree_too_large`.
    files: u64,
    /// Files per page of the list: the default and the most.
    page_default: usize,
    page_max: usize,
    /// The bytes of one revision's files that are looked at, and the time one view may take.
    tree_bytes: u64,
    wall_seconds: u64,
}

#[derive(Serialize)]
struct RevisionRef {
    seq: u32,
    id: String,
}

#[derive(Serialize)]
struct ListView {
    revision: RevisionRef,
    compared: Compared,
    rebuilt: bool,
    provisional: bool,
    chain_signature: Option<String>,
    totals: Totals,
    limits: LimitsView,
    files: Vec<FileEntry>,
    next: Option<String>,
}

#[derive(Serialize)]
struct FileView {
    #[serde(flatten)]
    entry: FileEntry,
    hunks: Vec<planeter_transport::changes::model::DiffHunk>,
    truncated: Option<planeter_transport::changes::model::Truncated>,
    approximate: bool,
}

fn changes_of(rt: &Arc<crate::transport::TransportRuntime>) -> Option<Arc<ChangesService>> {
    rt.changes.clone()
}

fn limits(svc: &ChangesService) -> LimitsView {
    let c = svc.config();
    LimitsView {
        text_bytes: TEXT_MAX,
        diff_lines: MAX_DIFF_LINES,
        context_lines: CONTEXT,
        edit_distance: MAX_EDIT_DISTANCE,
        files: c.max_files,
        page_default: LIST_DEFAULT,
        page_max: LIST_MAX,
        tree_bytes: c.max_bytes,
        wall_seconds: c.wall.as_secs(),
    }
}

/// The answer for a reason that makes the view unavailable: the code's one shape, the reason's own sentence, its own status.
pub(crate) fn unavailable(state: &AppState, reason: ChangesReason) -> Response {
    let (message, hint, status) = match reason {
        ChangesReason::ClaimNotHeld => (
            Key::ErrChangesClaimNotHeldMessage,
            Key::ErrChangesClaimNotHeldHint,
            axum::http::StatusCode::CONFLICT,
        ),
        ChangesReason::NotOneChange => (
            Key::ErrChangesNotOneChangeMessage,
            Key::ErrChangesNotOneChangeHint,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ChangesReason::TreeTooLarge => (
            Key::ErrChangesTreeTooLargeMessage,
            Key::ErrChangesTreeTooLargeHint,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ChangesReason::Busy => (
            Key::ErrChangesBusyMessage,
            Key::ErrChangesBusyHint,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
        ),
        ChangesReason::TimedOut => (
            Key::ErrChangesTimedOutMessage,
            Key::ErrChangesTimedOutHint,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
        ),
        ChangesReason::Failed => (
            Key::ErrChangesFailedMessage,
            Key::ErrChangesFailedHint,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
        ),
    };
    let mut resp = code_response_detailed(
        state,
        ErrorCode::ChangesUnavailable,
        ErrorKeys { message, hint },
        ErrorDetail {
            params: Vec::new(),
            fields: vec![("reason", reason.code().into())],
        },
    );
    *resp.status_mut() = status;
    if reason == ChangesReason::Busy
        && let Ok(v) = HeaderValue::from_str(&crate::api::retry_after_secs(state).to_string())
    {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

fn not_found(state: &AppState) -> Response {
    error_response(state, Failure::Read(planeter_core::ReadError::NotFound))
}

fn fail(state: &AppState, svc: &ChangesService, e: ChangesError) -> Response {
    match e {
        ChangesError::NoSuchFile => not_found(state),
        ChangesError::Unavailable(r) => {
            if r == ChangesReason::Busy {
                svc.busy();
            }
            unavailable(state, r)
        }
    }
}

/// Everything before the work: the principal, the edge limits, the one read gate, the service, the proposal and its revision.
async fn open(
    state: &AppState,
    peer: crate::client_ip::PeerAddr,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
    id: &str,
    seq: &str,
) -> Result<
    (
        planeter_store::RepositoryRecord,
        Arc<ChangesService>,
        RevisionRecord,
    ),
    Box<Response>,
> {
    let (_principal, record, proposals) = start(state, peer, headers, owner, name, true).await?;
    let Some(svc) = state.transport.as_ref().and_then(changes_of) else {
        return Err(Box::new(code_response_with_keys(
            state,
            ErrorCode::TransportOff,
            ErrorCode::PROPOSE_OFF,
        )));
    };
    let Ok(seq) = seq.parse::<u32>() else {
        return Err(Box::new(not_found(state)));
    };
    let found = {
        let (proposals, record, id) = (Arc::clone(&proposals), record.clone(), id.to_owned());
        offload(move || proposals.revision(&record, &id, seq)).await
    };
    match found {
        Ok(Ok((_, Some(rev)))) => Ok((record, svc, rev)),
        Ok(Ok((_, None))) => Err(Box::new(not_found(state))),
        Ok(Err(e)) => Err(Box::new(failure(state, &e))),
        Err(_) => Err(Box::new(error_response(state, Failure::Internal))),
    }
}

/// Run `job` under the view's admission: the heavy pool at the `extract` cost, held for the whole job (both extractions and the comparison).
async fn admitted<T, F>(
    state: &AppState,
    svc: &Arc<ChangesService>,
    job: F,
) -> Result<Result<T, ChangesError>, Box<Response>>
where
    F: FnOnce(Option<u64>) -> Result<T, ChangesError> + Send + 'static,
    T: Send + 'static,
{
    let cost = svc.cost();
    let ceiling = Some(ceiling_for(cost, state.gate.budget_bytes()));
    match state
        .gate
        .run_weighted_heavy(cost, "changes", move || job(ceiling))
        .await
    {
        Ok(r) => Ok(r),
        Err(AdmitError::Busy) => {
            svc.busy();
            Err(Box::new(unavailable(state, ChangesReason::Busy)))
        }
        Err(AdmitError::TooLarge { .. }) => {
            Err(Box::new(error_response(state, Failure::OverBudget)))
        }
        Err(AdmitError::Aborted) => Err(Box::new(error_response(state, Failure::Internal))),
    }
}

pub(crate) async fn list_handler(
    State(state): State<AppState>,
    Path((owner, name, id, seq)): Path<(String, String, String, String)>,
    q: QueryResult,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let (record, svc, rev) = match open(&state, peer, &headers, &owner, &name, &id, &seq).await {
        Ok(t) => t,
        Err(r) => return *r,
    };
    let Ok(Query(q)) = q else {
        return error_response(&state, Failure::Invalid);
    };
    if q.limit == Some(0) {
        return invalid_with_reason(&state, "limit_zero");
    }
    let limit = q.limit.unwrap_or(LIST_DEFAULT).min(LIST_MAX);
    // take our turn on this revision, then look in the cache before asking for any memory
    let turn = {
        let (flier, rid) = (Arc::clone(&svc), rev.revision_id.clone());
        match offload(move || flier.flight(&rid)).await {
            Ok(Ok(t)) => t,
            // more than eight already wait for this revision: refused at once, in words
            Ok(Err(_)) => {
                return fail(&state, &svc, ChangesError::Unavailable(ChangesReason::Busy));
            }
            Err(_) => return error_response(&state, Failure::Internal),
        }
    };
    let cached = {
        let (svc, rev) = (Arc::clone(&svc), rev.clone());
        match offload(move || svc.cached_list(&rev)).await {
            Ok(c) => c,
            Err(_) => return error_response(&state, Failure::Internal),
        }
    };
    let result = match cached {
        Some(r) => r,
        None => {
            let (svc2, record, rev2) = (Arc::clone(&svc), record.clone(), rev.clone());
            match admitted(&state, &svc, move |ceiling| {
                svc2.list(&record, &rev2, ceiling)
            })
            .await
            {
                Ok(r) => r,
                Err(resp) => return *resp,
            }
        }
    };
    drop(turn);
    let set = match result {
        Ok(s) => s,
        Err(e) => return fail(&state, &svc, e),
    };
    match page(&svc, &rev, set, q.after.as_deref(), limit) {
        Paged::View(v) => json_ok(&state, &*v),
        Paged::BadCursor => invalid_with_reason(&state, "after_unknown"),
    }
}

/// A page of the list as the response, or the answer to a cursor that names nothing.
fn page(
    svc: &ChangesService,
    rev: &RevisionRecord,
    set: ChangeSet,
    after: Option<&str>,
    limit: usize,
) -> Paged {
    let start = match after {
        None => 0,
        Some(a) => match set.files.iter().position(|f| f.file_id == a) {
            Some(i) => i + 1,
            None => return Paged::BadCursor,
        },
    };
    let end = (start + limit).min(set.files.len());
    let next = (end < set.files.len()).then(|| set.files[end - 1].file_id.clone());
    Paged::View(Box::new(ListView {
        revision: RevisionRef {
            seq: rev.seq,
            id: rev.revision_id.clone(),
        },
        compared: set.compared,
        rebuilt: set.rebuilt,
        provisional: set.provisional,
        chain_signature: set.chain_signature,
        totals: set.totals,
        limits: limits(svc),
        files: set.files[start..end].to_vec(),
        next,
    }))
}

enum Paged {
    View(Box<ListView>),
    BadCursor,
}

/// One file's diff.
pub(crate) async fn file_handler(
    State(state): State<AppState>,
    Path((owner, name, id, seq, file_id)): Path<(String, String, String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let (record, svc, rev) = match open(&state, peer, &headers, &owner, &name, &id, &seq).await {
        Ok(t) => t,
        Err(r) => return *r,
    };
    // a file id is 64 lowercase hexadecimal characters; anything else names nothing
    if file_id.len() != 64
        || !file_id
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return not_found(&state);
    }
    let turn = {
        let (flier, rid) = (Arc::clone(&svc), rev.revision_id.clone());
        match offload(move || flier.flight(&rid)).await {
            Ok(Ok(t)) => t,
            // more than eight already wait for this revision: refused at once, in words
            Ok(Err(_)) => {
                return fail(&state, &svc, ChangesError::Unavailable(ChangesReason::Busy));
            }
            Err(_) => return error_response(&state, Failure::Internal),
        }
    };
    let cached = {
        let (svc, rev, fid) = (Arc::clone(&svc), rev.clone(), file_id.clone());
        match offload(move || svc.cached_file(&rev, &fid)).await {
            Ok(c) => c,
            Err(_) => return error_response(&state, Failure::Internal),
        }
    };
    let result = match cached {
        Some(r) => r,
        None => {
            let (svc2, record, rev2, fid) = (
                Arc::clone(&svc),
                record.clone(),
                rev.clone(),
                file_id.clone(),
            );
            match admitted(&state, &svc, move |ceiling| {
                svc2.file(&record, &rev2, &fid, ceiling)
            })
            .await
            {
                Ok(r) => r,
                Err(resp) => return *resp,
            }
        }
    };
    drop(turn);
    match result {
        Ok(FileDiff {
            entry,
            hunks,
            truncated,
            approximate,
        }) => json_ok(
            &state,
            &FileView {
                entry,
                hunks,
                truncated,
                approximate,
            },
        ),
        Err(e) => fail(&state, &svc, e),
    }
}
