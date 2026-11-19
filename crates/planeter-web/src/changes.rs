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
use planeter_store::{RepositoryRecord, RevisionRecord};
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
        ChangesReason::PartlyLanded => (
            Key::ErrChangesPartlyLandedMessage,
            Key::ErrChangesPartlyLandedHint,
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

/// Why a view of a revision is not there: what the JSON and the pages each turn into their own answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fail {
    /// The engine's closed list of reasons.
    Reason(ChangesReason),
    /// The revision has no changed file with this id.
    NoSuchFile,
    /// The cost is over the memory budget of the heavy pool.
    OverBudget,
    Internal,
}

fn fail_of(e: ChangesError) -> Fail {
    match e {
        ChangesError::NoSuchFile => Fail::NoSuchFile,
        ChangesError::Unavailable(r) => Fail::Reason(r),
    }
}

/// Run `job` under the view's admission: the heavy pool at the `extract` cost, held for the whole job (both extractions and the comparison).
async fn admitted<T, F>(state: &AppState, svc: &Arc<ChangesService>, job: F) -> Result<T, Fail>
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
        Ok(r) => r.map_err(fail_of),
        Err(AdmitError::Busy) => {
            svc.busy();
            Err(Fail::Reason(ChangesReason::Busy))
        }
        Err(AdmitError::TooLarge { .. }) => Err(Fail::OverBudget),
        Err(AdmitError::Aborted) => Err(Fail::Internal),
    }
}

/// Take our turn on this revision. More than eight waiting already: refused at once, as busy.
async fn take_turn(
    svc: &Arc<ChangesService>,
    rev: &RevisionRecord,
) -> Result<planeter_transport::Flight, Fail> {
    let (flier, rid) = (Arc::clone(svc), rev.revision_id.clone());
    match offload(move || flier.flight(&rid)).await {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(_)) => {
            svc.busy();
            Err(Fail::Reason(ChangesReason::Busy))
        }
        Err(_) => Err(Fail::Internal),
    }
}

/// The change set of a revision: from the cache, or computed under the admission. The caller holds the turn.
async fn list_under_turn(
    state: &AppState,
    svc: &Arc<ChangesService>,
    record: &RepositoryRecord,
    rev: &RevisionRecord,
) -> Result<ChangeSet, Fail> {
    let cached = {
        let (svc, record, rev) = (Arc::clone(svc), record.clone(), rev.clone());
        offload(move || svc.cached_list(&record, &rev))
            .await
            .map_err(|_| Fail::Internal)?
    };
    match cached {
        Some(r) => r.map_err(fail_of),
        None => {
            let (svc2, record, rev2) = (Arc::clone(svc), record.clone(), rev.clone());
            admitted(state, svc, move |ceiling| {
                svc2.list(&record, &rev2, ceiling)
            })
            .await
        }
    }
}

/// One file's diff: from the cache, or computed under the admission. The caller holds the turn.
async fn file_under_turn(
    state: &AppState,
    svc: &Arc<ChangesService>,
    record: &RepositoryRecord,
    rev: &RevisionRecord,
    file_id: &str,
) -> Result<FileDiff, Fail> {
    let cached = {
        let (svc, record, rev, fid) = (
            Arc::clone(svc),
            record.clone(),
            rev.clone(),
            file_id.to_owned(),
        );
        offload(move || svc.cached_file(&record, &rev, &fid))
            .await
            .map_err(|_| Fail::Internal)?
    };
    match cached {
        Some(r) => r.map_err(fail_of),
        None => {
            let (svc2, record, rev2, fid) = (
                Arc::clone(svc),
                record.clone(),
                rev.clone(),
                file_id.to_owned(),
            );
            admitted(state, svc, move |ceiling| {
                svc2.file(&record, &rev2, &fid, ceiling)
            })
            .await
        }
    }
}

/// The change set of a revision (the list page and the JSON list).
pub(crate) async fn fetch_list(
    state: &AppState,
    svc: &Arc<ChangesService>,
    record: &RepositoryRecord,
    rev: &RevisionRecord,
) -> Result<ChangeSet, Fail> {
    let _turn = take_turn(svc, rev).await?;
    list_under_turn(state, svc, record, rev).await
}

/// One file's diff (the JSON file answer).
pub(crate) async fn fetch_file(
    state: &AppState,
    svc: &Arc<ChangesService>,
    record: &RepositoryRecord,
    rev: &RevisionRecord,
    file_id: &str,
) -> Result<FileDiff, Fail> {
    let _turn = take_turn(svc, rev).await?;
    file_under_turn(state, svc, record, rev, file_id).await
}

/// One file's diff and the whole list (the file page: the neighbours come from the list). The list is read first, so that a list that is
/// not cached is computed once, with every diff written to the cache, and the file then comes from the cache.
pub(crate) async fn fetch_file_and_list(
    state: &AppState,
    svc: &Arc<ChangesService>,
    record: &RepositoryRecord,
    rev: &RevisionRecord,
    file_id: &str,
) -> Result<(FileDiff, ChangeSet), Fail> {
    let _turn = take_turn(svc, rev).await?;
    let set = list_under_turn(state, svc, record, rev).await?;
    if !set.files.iter().any(|f| f.file_id == file_id) {
        return Err(Fail::NoSuchFile);
    }
    let diff = file_under_turn(state, svc, record, rev, file_id).await?;
    Ok((diff, set))
}

/// The JSON answer for a failure.
fn json_fail(state: &AppState, f: Fail) -> Response {
    match f {
        Fail::Reason(r) => unavailable(state, r),
        Fail::NoSuchFile => not_found(state),
        Fail::OverBudget => error_response(state, Failure::OverBudget),
        Fail::Internal => error_response(state, Failure::Internal),
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
    let set = match fetch_list(&state, &svc, &record, &rev).await {
        Ok(s) => s,
        Err(f) => return json_fail(&state, f),
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
    match fetch_file(&state, &svc, &record, &rev, &file_id).await {
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
        Err(f) => json_fail(&state, f),
    }
}
