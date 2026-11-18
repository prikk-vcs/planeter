//! The delivery routes (RFC 008 handoff 2): **landing by delivery** over HTTP.
//!
//! A maintainer sends an archive of their repository; the server checks it in a scratch repository that trusts only the keys this
//! repository already trusts, says what would happen (`POST …/deliveries`), and carries it out when asked (`POST …/{id}/confirm`).
//! Bearer token only, like propose (a session cookie is what another site's request carries); the action is `MergeSeal`; a hidden
//! repository and an unknown delivery are one `404`. The engine is [`planeter_transport::delivery`]; this module is its door.
//!
//! * The upload has **the shape and the edge limit of repository creation's import** (the magic, the metadata length, the metadata —
//!   here an empty object — and the archive). Everything that can be decided before the first byte of the archive (who, the
//!   repository's state, a delivery in flight, the principal's limits, the size) is decided before it.
//! * The stage and the rule run inside the request, **detached from it** (a dropped connection must not abort anything); the route
//!   waits up to the service's `request_wait` (25 s) and answers `202` with the delivery as it stands when the work takes longer, and the client polls `GET`.
//! * A refused report is **not** an HTTP error: the maintainer asked a question and got an answer (`201`, `report.verdict: refused`).

use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde::Serialize;

use planeter_core::Principal;
use planeter_store::{DeliveryState, RepositoryRecord, StartRefusal, UserId};
use planeter_transport::creation::cost_model;
use planeter_transport::delivery::{DeliveryError, DeliveryService, DeliveryUpload, Report};
use planeter_transport::{DeliveryView, FOLLOWED_REF};

use crate::api::{
    AppState, Failure, code_response, code_response_with_keys, error_response,
    with_security_headers,
};
use crate::catalog::{Key, T, negotiate_from_headers};
use crate::creation::{MAGIC, METADATA_MAX, PREFIX_LEN, bad_framing, detail, retry_after};
use crate::errors::ErrorCode;
use crate::gate::{AdmitError, offload};
use crate::transport::{TransportRuntime, UPLOAD_TOTAL, limits, next_chunk};

// ------------------------------------------------------------------------------------------------ the view

#[derive(Serialize)]
struct TitledId {
    id: String,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyJson {
    pub key_id: String,
    pub fingerprint: String,
}

#[derive(Serialize)]
struct ReportJson {
    verdict: &'static str,
    reason: Option<String>,
    /// The branch a `branch_missing` or `not_a_successor` refusal is about.
    reason_ref: Option<String>,
    old_tip: Option<String>,
    new_tip: Option<String>,
    new_blocks: u64,
    refs_added: Vec<String>,
    proposals_stay: Vec<TitledId>,
    proposals_land: Vec<TitledId>,
    proposals_cannot_be_kept: Vec<TitledId>,
    trusted_keys: Vec<KeyJson>,
    archive_bytes: u64,
}

#[derive(Serialize)]
struct FailureJson {
    reason: String,
}

/// A delivery as its owner sees it (the OpenAPI schema `DeliveryView`).
#[derive(Serialize)]
pub struct DeliveryJson {
    id: String,
    state: &'static str,
    started_at: u64,
    updated_at: u64,
    report: Option<ReportJson>,
    failure: Option<FailureJson>,
    needs_recovery: bool,
    /// What the state means, in the caller's language, in plain words.
    message: String,
    /// What to do next.
    hint: String,
}

/// The two catalog keys of a refusal or a failure reason, or `None` for a word this server does not know.
fn reason_keys(reason: &str) -> Option<(Key, Key)> {
    Some(match reason {
        "no_main" => (
            Key::DeliveryReasonNoMainMessage,
            Key::DeliveryReasonNoMainHint,
        ),
        "unadopted_key" => (
            Key::DeliveryReasonUnadoptedKeyMessage,
            Key::DeliveryReasonUnadoptedKeyHint,
        ),
        "not_a_successor" => (
            Key::DeliveryReasonNotASuccessorMessage,
            Key::DeliveryReasonNotASuccessorHint,
        ),
        "older_or_diverged" => (
            Key::DeliveryReasonOlderOrDivergedMessage,
            Key::DeliveryReasonOlderOrDivergedHint,
        ),
        "unrelated" => (
            Key::DeliveryReasonUnrelatedMessage,
            Key::DeliveryReasonUnrelatedHint,
        ),
        "branch_missing" => (
            Key::DeliveryReasonBranchMissingMessage,
            Key::DeliveryReasonBranchMissingHint,
        ),
        "nothing_to_deliver" => (
            Key::DeliveryReasonNothingToDeliverMessage,
            Key::DeliveryReasonNothingToDeliverHint,
        ),
        "damaged_archive" => (
            Key::DeliveryReasonDamagedArchiveMessage,
            Key::DeliveryReasonDamagedArchiveHint,
        ),
        "moved" => (
            Key::DeliveryReasonMovedMessage,
            Key::DeliveryReasonMovedHint,
        ),
        "exchange_failed" => (
            Key::DeliveryReasonExchangeFailedMessage,
            Key::DeliveryReasonExchangeFailedHint,
        ),
        "verify_failed" => (
            Key::DeliveryReasonVerifyFailedMessage,
            Key::DeliveryReasonVerifyFailedHint,
        ),
        "publication_trust" => (
            Key::DeliveryReasonPublicationTrustMessage,
            Key::DeliveryReasonPublicationTrustHint,
        ),
        "timed_out" => (
            Key::DeliveryReasonTimedOutMessage,
            Key::DeliveryReasonTimedOutHint,
        ),
        "ceiling" => (
            Key::DeliveryReasonCeilingMessage,
            Key::DeliveryReasonCeilingHint,
        ),
        "unavailable" => (
            Key::DeliveryReasonUnavailableMessage,
            Key::DeliveryReasonUnavailableHint,
        ),
        "repository_gone" => (
            Key::DeliveryReasonRepositoryGoneMessage,
            Key::DeliveryReasonRepositoryGoneHint,
        ),
        _ => return None,
    })
}

fn state_keys(state: DeliveryState) -> (Key, Key) {
    match state {
        DeliveryState::Staged => (
            Key::DeliveryStateStagedMessage,
            Key::DeliveryStateStagedHint,
        ),
        DeliveryState::Reported => (Key::DeliveryAcceptedMessage, Key::DeliveryAcceptedHint),
        DeliveryState::Confirmed => (
            Key::DeliveryStateConfirmedMessage,
            Key::DeliveryStateConfirmedHint,
        ),
        DeliveryState::Exchanged => (
            Key::DeliveryStateExchangedMessage,
            Key::DeliveryStateExchangedHint,
        ),
        DeliveryState::Reaccepting => (
            Key::DeliveryStateReacceptingMessage,
            Key::DeliveryStateReacceptingHint,
        ),
        DeliveryState::Done => (Key::DeliveryStateDoneMessage, Key::DeliveryStateDoneHint),
        DeliveryState::Failed => (
            Key::DeliveryStateFailedMessage,
            Key::DeliveryStateFailedHint,
        ),
        DeliveryState::Abandoned => (
            Key::DeliveryStateAbandonedMessage,
            Key::DeliveryStateAbandonedHint,
        ),
        DeliveryState::Expired => (
            Key::DeliveryStateExpiredMessage,
            Key::DeliveryStateExpiredHint,
        ),
    }
}

/// The message and the hint of a delivery, in `t`'s language. Public so that a test can hold every one of them to the word lists.
pub fn texts(
    t: &T,
    state: DeliveryState,
    reason: Option<&str>,
    reason_ref: Option<&str>,
    needs_recovery: bool,
) -> (String, String) {
    let branch = reason_ref.unwrap_or(FOLLOWED_REF);
    let (m, h) = if needs_recovery {
        (
            Key::DeliveryNeedsRecoveryMessage,
            Key::DeliveryNeedsRecoveryHint,
        )
    } else if state == DeliveryState::Failed {
        reason
            .and_then(reason_keys)
            .unwrap_or_else(|| state_keys(state))
    } else {
        state_keys(state)
    };
    (
        t.msg(m).with("ref", branch).text(),
        t.msg(h).with("ref", branch).text(),
    )
}

fn titled(titles: &[(String, String)], ids: &[String]) -> Vec<TitledId> {
    ids.iter()
        .map(|id| TitledId {
            id: id.clone(),
            title: titles
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, t)| t.clone())
                .unwrap_or_default(),
        })
        .collect()
}

fn report_json(svc: &DeliveryService, repo: &planeter_store::RepoId, r: &Report) -> ReportJson {
    let all: Vec<String> = r
        .proposals_stay
        .iter()
        .chain(&r.proposals_land)
        .chain(&r.proposals_cannot_be_kept)
        .cloned()
        .collect();
    let titles = svc.titles(repo, &all);
    ReportJson {
        verdict: if r.is_acceptable() {
            "acceptable"
        } else {
            "refused"
        },
        reason: r.reason.clone(),
        reason_ref: r.reason_ref.clone(),
        old_tip: r.old_tip.clone(),
        new_tip: r.new_tip.clone(),
        new_blocks: r.new_blocks,
        refs_added: r.refs_added.clone(),
        proposals_stay: titled(&titles, &r.proposals_stay),
        proposals_land: titled(&titles, &r.proposals_land),
        proposals_cannot_be_kept: titled(&titles, &r.proposals_cannot_be_kept),
        trusted_keys: r
            .trusted_keys
            .iter()
            .map(|k| KeyJson {
                key_id: k.key_id.clone(),
                fingerprint: k.fingerprint.clone(),
            })
            .collect(),
        archive_bytes: r.archive_bytes,
    }
}

/// The delivery as the API shows it.
pub fn delivery_json(svc: &DeliveryService, v: &DeliveryView, t: &T) -> DeliveryJson {
    let reason = v
        .reason
        .clone()
        .or_else(|| v.report.as_ref().and_then(|r| r.reason.clone()));
    let reason_ref = v.report.as_ref().and_then(|r| r.reason_ref.clone());
    let (message, hint) = texts(
        t,
        v.state,
        reason.as_deref(),
        reason_ref.as_deref(),
        v.needs_recovery,
    );
    DeliveryJson {
        id: v.delivery_id.clone(),
        state: v.state.as_str(),
        started_at: v.created_at,
        updated_at: v.updated_at,
        report: v.report.as_ref().map(|r| report_json(svc, &v.repo_id, r)),
        failure: (v.state == DeliveryState::Failed).then(|| FailureJson {
            reason: reason.unwrap_or_else(|| "unavailable".to_owned()),
        }),
        needs_recovery: v.needs_recovery,
        message,
        hint,
    }
}

fn json(state: &AppState, status: StatusCode, body: Vec<u8>) -> Response {
    let mut resp = with_security_headers(state, Response::builder().status(status))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| code_response(state, ErrorCode::Internal));
    resp.extensions_mut().insert(crate::catalog::LanguageVaries);
    resp
}

fn view_response(
    state: &AppState,
    status: StatusCode,
    svc: &DeliveryService,
    v: &DeliveryView,
    headers: &HeaderMap,
) -> Response {
    let t = T::new(negotiate_from_headers(headers));
    json(
        state,
        status,
        serde_json::to_vec(&delivery_json(svc, v, &t)).unwrap_or_default(),
    )
}

// ------------------------------------------------------------------------------------------------ refusals

/// An engine refusal as the API's error body.
fn failure_response(state: &AppState, e: &DeliveryError, max_archive: u64) -> Response {
    match e {
        DeliveryError::Busy { retry_after: wait } => retry_after(
            code_response(state, ErrorCode::DeliveryBusy),
            wait.as_secs().max(1),
        ),
        DeliveryError::TooLarge { limit } => detail(
            state,
            ErrorCode::PayloadTooLarge,
            ErrorCode::IMPORT_TOO_LARGE,
            vec![],
            vec![("limit", (*limit).into())],
        ),
        DeliveryError::StageLimit(StartRefusal::TooManyForPrincipal { limit }) => detail(
            state,
            ErrorCode::StageLimit,
            ErrorCode::StageLimit.keys(),
            vec![("limit", limit.to_string())],
            vec![("limit", (*limit).into())],
        ),
        DeliveryError::StageLimit(StartRefusal::BudgetExceeded { limit }) => detail(
            state,
            ErrorCode::StageLimit,
            ErrorCode::STAGE_BUDGET,
            vec![],
            vec![("limit_bytes", (*limit).into())],
        ),
        DeliveryError::StageLimit(StartRefusal::RepoBusy) => {
            code_response(state, ErrorCode::DeliveryBusy)
        }
        DeliveryError::StageLimit(StartRefusal::IdTaken) => {
            code_response(state, ErrorCode::Internal)
        }
        DeliveryError::NeedsRecovery { .. } => code_response(state, ErrorCode::NeedsRecovery),
        DeliveryError::NotFound => {
            error_response(state, Failure::Read(planeter_core::ReadError::NotFound))
        }
        DeliveryError::Expired => code_response(state, ErrorCode::StageExpired),
        DeliveryError::NotConfirmable(_) => code_response(state, ErrorCode::DeliveryNotConfirmable),
        DeliveryError::TimedOut | DeliveryError::CeilingExceeded | DeliveryError::Unavailable => {
            retry_after(code_response(state, ErrorCode::Unavailable), 30)
        }
        // not an HTTP error: the caller shows the delivery (state `failed`) instead
        DeliveryError::Failed(_) | DeliveryError::Incomplete { .. } => {
            let _ = max_archive;
            code_response(state, ErrorCode::Internal)
        }
    }
}

// ------------------------------------------------------------------------------------------------ the door

struct Door {
    record: RepositoryRecord,
    user: UserId,
    rt: Arc<TransportRuntime>,
    svc: Arc<DeliveryService>,
}

/// Who, where, and whether deliveries are on: the rate limits, the one read gate and the `MergeSeal` check (a stranger and a hidden
/// repository are one `404`; a reader who may not deliver learns only that: `403`), then the configuration.
// The refusal is a whole response, as in the other doors of this crate (`admin_gate`).
#[allow(clippy::result_large_err)]
async fn door(
    state: &AppState,
    peer: crate::client_ip::PeerAddr,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<Door, Response> {
    // A session cookie is what a browser attaches to a request another site makes: not looked at here.
    let mut bearer_only = headers.clone();
    bearer_only.remove(header::COOKIE);
    let principal: Principal = crate::principal::principal_from_request(&bearer_only, state).await;
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
            match read.merge_access(
                &principal,
                &planeter_store::Owner::User(owner.clone()),
                &name,
            ) {
                Err(planeter_core::ProposeAccess::NotFound) => {
                    read.merge_access(&principal, &planeter_store::Owner::Org(owner), &name)
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
            if let Some(rt) = &runtime {
                rt.export.metrics().refusal("forbidden");
            }
            return Err(code_response_with_keys(
                state,
                ErrorCode::Forbidden,
                ErrorCode::FORBIDDEN_DELIVER,
            ));
        }
        Ok(Ok(r)) => r,
    };
    let Some(rt) = runtime else {
        return Err(code_response(state, ErrorCode::TransportOff));
    };
    let Some(svc) = rt.deliveries.clone() else {
        return Err(code_response_with_keys(
            state,
            ErrorCode::TransportOff,
            ErrorCode::PROPOSE_OFF,
        ));
    };
    let Some(user) = principal.user().cloned() else {
        return Err(code_response_with_keys(
            state,
            ErrorCode::Forbidden,
            ErrorCode::FORBIDDEN_DELIVER,
        ));
    };
    Ok(Door {
        record,
        user,
        rt,
        svc,
    })
}

fn admit_fail(state: &AppState, e: AdmitError) -> Response {
    match e {
        AdmitError::TooLarge { .. } => {
            code_response_with_keys(state, ErrorCode::TooLarge, ErrorCode::CREATE_TOO_BIG)
        }
        AdmitError::Busy => error_response(state, Failure::Busy),
        AdmitError::Aborted => code_response(state, ErrorCode::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ the upload

/// An archive being written into a delivery's directory (each piece on the blocking pool).
struct Writer {
    upload: Option<DeliveryUpload>,
}

impl Writer {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), DeliveryError> {
        let mut upload = self.upload.take().ok_or(DeliveryError::Unavailable)?;
        let (upload, result) = tokio::task::spawn_blocking(move || {
            let r = upload.write(&chunk);
            (upload, r)
        })
        .await
        .map_err(|_| DeliveryError::Unavailable)?;
        match result {
            Ok(()) => {
                self.upload = Some(upload);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn written(&self) -> u64 {
        self.upload.as_ref().map_or(0, DeliveryUpload::written)
    }
}

/// The metadata of a delivery: an object with no field (it exists so that the framing is the import's, and a field can be added).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MetaWire {}

// ------------------------------------------------------------------------------------------------ the handlers

/// `POST …/deliveries`: the archive in, the delivery (staged and reported) out.
pub(crate) async fn start_handler(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let door = match door(&state, peer, &headers, &owner, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Door {
        record,
        user,
        rt,
        svc,
    } = door;
    let max = svc.config().max_archive_bytes;
    // Before a byte of the body: the repository's state and a delivery in flight.
    {
        let (svc, record) = (Arc::clone(&svc), record.clone());
        match offload(move || svc.precheck(&record)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                rt.export.metrics().refusal(e.reason());
                return failure_response(&state, &e, max);
            }
            Err(_) => return error_response(&state, Failure::Internal),
        }
    }
    // The framing, from the headers: the length is required and bounded before a byte of the body is read.
    if let Some(ct) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        && !ct.trim().eq_ignore_ascii_case("application/octet-stream")
    {
        return bad_framing(&state, "content_type");
    }
    let Some(total) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
    else {
        return detail(
            &state,
            ErrorCode::LengthRequired,
            ErrorCode::IMPORT_LENGTH,
            vec![],
            vec![],
        );
    };
    let too_large = || {
        detail(
            &state,
            ErrorCode::PayloadTooLarge,
            ErrorCode::IMPORT_TOO_LARGE,
            vec![],
            vec![("limit", max.into())],
        )
    };
    if total > max + METADATA_MAX + PREFIX_LEN {
        return too_large();
    }
    if total < PREFIX_LEN {
        return bad_framing(&state, "too_short");
    }
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
                return bad_framing(&state, "magic");
            }
            let mut n = [0u8; 8];
            n.copy_from_slice(&buf[MAGIC.len()..PREFIX_LEN as usize]);
            let m = u64::from_be_bytes(n);
            if m > METADATA_MAX {
                return bad_framing(&state, "metadata_too_long");
            }
            if PREFIX_LEN + m >= total {
                return bad_framing(&state, "no_archive");
            }
            meta_len = Some(m);
            continue;
        }
        match next_chunk(&mut stream, deadline).await {
            Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
            Ok(None) | Err(()) => return bad_framing(&state, "truncated"),
        }
    }
    let m = meta_len.unwrap_or(0);
    let carry = buf.split_off((PREFIX_LEN + m) as usize);
    let archive_len = total - PREFIX_LEN - m;
    if archive_len > max {
        return too_large();
    }
    let meta = &buf[PREFIX_LEN as usize..];
    if !meta.iter().all(u8::is_ascii_whitespace)
        && serde_json::from_slice::<MetaWire>(meta).is_err()
    {
        return bad_framing(&state, "metadata");
    }

    // The delivery begins: the one-in-flight rule, the principal's and the host's limits, the size, all before the archive.
    let started = {
        let (svc, user, record) = (Arc::clone(&svc), user.clone(), record.clone());
        offload(move || svc.start(&user, &record, archive_len)).await
    };
    let upload = match started {
        Ok(Ok(u)) => u,
        Ok(Err(e)) => {
            rt.export.metrics().refusal(e.reason());
            return failure_response(&state, &e, max);
        }
        Err(_) => return error_response(&state, Failure::Internal),
    };
    let id = upload.delivery_id().to_owned();
    let mut writer = Writer {
        upload: Some(upload),
    };
    let mut pending = carry;
    loop {
        if !pending.is_empty() {
            let chunk = std::mem::take(&mut pending);
            if let Err(e) = writer.write(chunk).await {
                return match e {
                    DeliveryError::TooLarge { .. } => bad_framing(&state, "length"),
                    other => failure_response(&state, &other, max),
                };
            }
        }
        if writer.written() >= archive_len {
            break;
        }
        match next_chunk(&mut stream, deadline).await {
            Ok(Some(chunk)) => pending = chunk.to_vec(),
            Ok(None) | Err(()) => return bad_framing(&state, "truncated"),
        }
    }
    if writer.written() != archive_len {
        return bad_framing(&state, "length");
    }
    let upload = writer.upload.take().expect("the upload is back");
    let uploaded = {
        let svc = Arc::clone(&svc);
        offload(move || upload.finish(&svc)).await
    };
    let uploaded = match uploaded {
        Ok(Ok(u)) => u,
        Ok(Err(e)) => return failure_response(&state, &e, max),
        Err(_) => return error_response(&state, Failure::Internal),
    };

    // The stage and the rule, **detached from this request**: the memory budget admits them as a write of the archive's size
    // (creation's cost model); the request waits `request_wait` and then answers with the delivery as it stands.
    let cost = cost_model::admission_cost(archive_len);
    let work = {
        let (state, svc, user, record) = (
            state.clone(),
            Arc::clone(&svc),
            user.clone(),
            record.clone(),
        );
        tokio::spawn(async move {
            state
                .gate
                .run_weighted_heavy(cost, "delivery-stage", move || {
                    svc.finish(&user, &record, uploaded)
                })
                .await
        })
    };
    let finished = tokio::time::timeout(svc.config().request_wait, work).await;
    let view = {
        let (svc, user, id) = (Arc::clone(&svc), user.clone(), id.clone());
        offload(move || svc.view(&user, &id)).await
    };
    let status = match &finished {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::ACCEPTED,
    };
    if let Ok(joined) = finished {
        match joined {
            Ok(Ok(Ok(_report))) => {}
            Ok(Ok(Err(e))) => return failure_response(&state, &e, max),
            Ok(Err(e)) => return admit_fail(&state, e),
            Err(_) => return error_response(&state, Failure::Internal),
        }
    }
    match view {
        Ok(Ok(v)) => view_response(&state, status, &svc, &v, &headers),
        Ok(Err(e)) => failure_response(&state, &e, max),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

/// `GET …/deliveries/{id}`.
pub(crate) async fn view_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let Door {
        record, user, svc, ..
    } = match door(&state, peer, &headers, &owner, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let max = svc.config().max_archive_bytes;
    let viewed = {
        let (svc, user, id) = (Arc::clone(&svc), user.clone(), id);
        offload(move || svc.view(&user, &id)).await
    };
    match viewed {
        // a delivery of another repository is not this one's: the same `404`
        Ok(Ok(v)) if v.repo_id != record.repo_id => {
            failure_response(&state, &DeliveryError::NotFound, max)
        }
        Ok(Ok(v)) => view_response(&state, StatusCode::OK, &svc, &v, &headers),
        Ok(Err(e)) => failure_response(&state, &e, max),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

/// `POST …/deliveries/{id}/confirm`: carry it out. **Detached**: a dropped connection does not abort an install.
pub(crate) async fn confirm_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let Door {
        record,
        user,
        rt,
        svc,
    } = match door(&state, peer, &headers, &owner, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let max = svc.config().max_archive_bytes;
    // What it is, before anything is started: not found, or not in a state that can be confirmed.
    let current = {
        let (svc, user, id) = (Arc::clone(&svc), user.clone(), id.clone());
        offload(move || svc.view(&user, &id)).await
    };
    let current = match current {
        Ok(Ok(v)) if v.repo_id == record.repo_id => v,
        Ok(Ok(_)) => return failure_response(&state, &DeliveryError::NotFound, max),
        Ok(Err(e)) => return failure_response(&state, &e, max),
        Err(_) => return error_response(&state, Failure::Internal),
    };
    if current.state != DeliveryState::Reported
        || !current.report.as_ref().is_some_and(Report::is_acceptable)
    {
        return failure_response(&state, &DeliveryError::NotConfirmable(current.state), max);
    }
    let bytes = current.report.as_ref().map_or(0, |r| r.archive_bytes);
    let cost = cost_model::admission_cost(bytes);
    let work = {
        let (state, svc, user, record, id) = (
            state.clone(),
            Arc::clone(&svc),
            user.clone(),
            record.clone(),
            id.clone(),
        );
        tokio::spawn(async move {
            state
                .gate
                .run_weighted_heavy(cost, "delivery-confirm", move || {
                    svc.confirm(&user, &record, &id)
                })
                .await
        })
    };
    let finished = tokio::time::timeout(svc.config().request_wait, work).await;
    let mut status = match &finished {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::ACCEPTED,
    };
    if let Ok(joined) = finished {
        match joined {
            Ok(Ok(Ok(_installed))) => {}
            // the request worked; the delivery did not (its view says so), or it stopped after its exchange and will be finished
            Ok(Ok(Err(DeliveryError::Failed(_)))) => {}
            Ok(Ok(Err(DeliveryError::Incomplete { .. }))) => status = StatusCode::ACCEPTED,
            Ok(Ok(Err(e))) => {
                rt.export.metrics().refusal(e.reason());
                return failure_response(&state, &e, max);
            }
            Ok(Err(e)) => return admit_fail(&state, e),
            Err(_) => return error_response(&state, Failure::Internal),
        }
    }
    let view = {
        let (svc, user, id) = (Arc::clone(&svc), user.clone(), id);
        offload(move || svc.view(&user, &id)).await
    };
    match view {
        Ok(Ok(v)) => view_response(&state, status, &svc, &v, &headers),
        Ok(Err(e)) => failure_response(&state, &e, max),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

/// `DELETE …/deliveries/{id}`: give up a delivery that was checked and not carried out.
pub(crate) async fn cancel_handler(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    peer: crate::client_ip::PeerAddr,
    headers: HeaderMap,
) -> Response {
    let Door {
        record, user, svc, ..
    } = match door(&state, peer, &headers, &owner, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let max = svc.config().max_archive_bytes;
    let done = {
        let (svc, user, repo) = (Arc::clone(&svc), user.clone(), record.repo_id.clone());
        offload(move || match svc.view(&user, &id) {
            Ok(v) if v.repo_id == repo => svc.cancel(&user, &id),
            Ok(_) => Err(DeliveryError::NotFound),
            Err(e) => Err(e),
        })
        .await
    };
    match done {
        Ok(Ok(())) => {
            let mut resp =
                with_security_headers(&state, Response::builder().status(StatusCode::NO_CONTENT))
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(Body::empty())
                    .unwrap_or_else(|_| code_response(&state, ErrorCode::Internal));
            resp.extensions_mut().insert(crate::catalog::LanguageVaries);
            resp
        }
        Ok(Err(e)) => failure_response(&state, &e, max),
        Err(_) => error_response(&state, Failure::Internal),
    }
}

// ------------------------------------------------------------------------------------------------ the last delivery

/// The last finished delivery of a repository, as the pages and the refs answer show it.
#[derive(Debug, Clone, Serialize)]
pub struct LastDelivery {
    /// Unix seconds.
    pub delivered_at: u64,
    /// The display name of whoever delivered (the account id when they have none).
    pub delivered_by: String,
    pub old_tip: Option<String>,
    pub new_tip: Option<String>,
    /// The keys this repository trusted when the delivery was checked (every block of what was delivered is signed by one of them).
    pub trusted_keys: Vec<KeyJson>,
}

/// The last `done` delivery of `repo`, when deliveries are on and there is one. A lookup that fails shows none: the line is an
/// addition to a page, never a reason for it to fail.
pub(crate) async fn last_delivery(
    state: &AppState,
    repo: &planeter_store::RepoId,
) -> Option<LastDelivery> {
    let svc = state.transport.as_ref()?.deliveries.clone()?;
    let repo = repo.clone();
    let row = offload(move || svc.last_done(&repo)).await.ok()?.ok()??;
    let account = UserId::new(row.principal.clone());
    let name = crate::authn::blocking(state, move |s| s.auth.display_name(&account))
        .await
        .ok()
        .flatten();
    Some(LastDelivery {
        delivered_at: row.finished_at.unwrap_or(row.updated_at),
        delivered_by: name.unwrap_or(row.principal),
        old_tip: row.old_tip,
        new_tip: row.new_tip,
        trusted_keys: row
            .report_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Report>(j).ok())
            .map(|r| {
                r.trusted_keys
                    .into_iter()
                    .map(|k| KeyJson {
                        key_id: k.key_id,
                        fingerprint: k.fingerprint,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// What a repository page adds when the repository has had a delivery (RFC 008 D-4): the sentence, the project's branch, and the keys.
pub struct DeliveredBlock {
    pub line: String,
    pub branch_line: String,
    pub keys_heading: String,
    pub keys: Vec<KeyJson>,
}

/// The block for the pages of `owner/name`, in `t`'s language; `None` when the repository has had no delivery (a page then shows
/// nothing new), or deliveries are off.
pub(crate) async fn delivered_block(
    state: &AppState,
    principal: &Principal,
    owner: &str,
    name: &str,
    t: &T,
) -> Option<DeliveredBlock> {
    state.transport.as_ref()?.deliveries.as_ref()?;
    let record = crate::transport::readable(state, principal, owner, name)
        .await
        .ok()?;
    let last = last_delivery(state, &record.repo_id).await?;
    Some(DeliveredBlock {
        line: t
            .msg(Key::RepoDeliveredBy)
            .with("name", &last.delivered_by)
            .with("date", crate::timefmt::date(last.delivered_at))
            .text(),
        branch_line: t
            .msg(Key::RepoProjectBranch)
            .with("ref", FOLLOWED_REF)
            .text(),
        keys_heading: t.get(Key::RepoTrustedKeys).to_owned(),
        keys: last.trusted_keys,
    })
}
