//! Creating a repository over HTTP (RFC 004 part 7): `POST /api/v1/repos`, `POST /api/v1/repos/import`,
//! `GET /api/v1/repos/import/{stage}` and `POST /api/v1/repos/import/{stage}/confirm`, and the helpers the pages share.
//!
//! **Bearer only.** A session cookie is what a browser attaches to a request another site makes, so these routes do not look
//! at one (the page forms carry a CSRF token instead). The order of every handler is the order a person can act on it:
//! who they are → the rate limit → whether the values are valid → whether they may create *there* and whether there is room →
//! only then the body of an archive is read.
//!
//! Who may create where is [`planeter_core::creation::creation_target`]; every refusal for "not there" is one answer, so it
//! tells a stranger nothing about which organizations exist. A taken name is said only after that check has passed.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::{Deserialize, Serialize};

use planeter_core::Principal;
use planeter_core::creation::{
    AdoptionError, AdoptionRequest, NameError, NewRepo, RequestError, group_fingerprint,
    visibility_word,
};
use planeter_store::{Owner, RepositoryRecord, UserId};
use planeter_transport::creation::cost_model;
use planeter_transport::{
    Confirmed, CreateError, CreationService, FoundKey, StageMeta, StageSummary, StageUpload,
    StageUploaded,
};

use crate::api::{
    AppState, Failure, code_response, code_response_detailed, code_response_with_keys,
    error_response, with_security_headers,
};
use crate::catalog::{Key, T, negotiate_from_headers};
use crate::errors::{ErrorCode, ErrorDetail, ErrorKeys};
use crate::gate::{AdmitError, offload};
use crate::transport::{RateLimiter, UPLOAD_TOTAL, next_chunk};

/// What the web layer holds of repository creation.
pub struct CreationRuntime {
    pub service: Arc<CreationService>,
    /// Creations and stagings one principal may start in an hour.
    pub rate: RateLimiter,
}

// ------------------------------------------------------------------------------------------------ reserved names

/// The words a name could be confused with, from the paths a server answers on (`patterns` as axum writes them, like
/// `/{owner}/{name}/tree/{*path}`): **the first segment of every route that has a static one** (`api`, `login`, `static`,
/// `new`, …), and any static segment that stands where another route has a parameter after the same path (`import` in
/// `/api/v1/repos/import`, beside `/api/v1/repos/{owner}/{name}/…`). Lowercase. Derived, never listed by hand: a route
/// added tomorrow reserves its word tomorrow, and `tests/creation_http.rs` fails if a name that a route owns is creatable.
pub fn reserved_from(patterns: &[&str]) -> BTreeSet<String> {
    let segs: Vec<Vec<&str>> = patterns
        .iter()
        .map(|p| p.split('/').filter(|s| !s.is_empty()).collect())
        .collect();
    let is_param = |s: &str| s.starts_with('{');
    // `q`'s prefix matches `r`'s when each of `q`'s segments is the same word or a parameter (which takes any word of `r`'s). A
    // parameter in `r`'s prefix takes no part: the words it could stand for that collide with `q` are `q`'s own first segments,
    // and those are reserved already.
    let same_prefix =
        |q: &[&str], r: &[&str]| q.iter().zip(r.iter()).all(|(x, y)| x == y || is_param(x));
    let mut out = BTreeSet::new();
    for r in &segs {
        for (i, seg) in r.iter().enumerate() {
            if is_param(seg) {
                continue;
            }
            let first = i == 0;
            let beside_param = segs
                .iter()
                .any(|q| q.len() > i && is_param(q[i]) && same_prefix(&q[..i], &r[..i]));
            if first || beside_param {
                out.insert(seg.to_ascii_lowercase());
            }
        }
    }
    out
}

/// The reserved words of this server (see [`reserved_from`]), computed once from the route tables.
pub fn reserved_words() -> &'static BTreeSet<String> {
    static WORDS: OnceLock<BTreeSet<String>> = OnceLock::new();
    WORDS.get_or_init(|| {
        let api = crate::api::routes();
        let ui = crate::ui::routes();
        let patterns: Vec<&str> = api
            .table()
            .iter()
            .chain(ui.table().iter())
            .map(|e| e.path)
            .collect();
        reserved_from(&patterns)
    })
}

/// Whether a (lowercase) word belongs to the server.
pub fn is_reserved(word: &str) -> bool {
    reserved_words().contains(word)
}

// ------------------------------------------------------------------------------------------------ wire

/// `POST /api/v1/repos` and the metadata of `POST /api/v1/repos/import`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewWire {
    owner: String,
    name: String,
    visibility: String,
    #[serde(default)]
    proposals: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedKey {
    key_id: String,
    fingerprint: String,
}

/// The body of a confirmation.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmWire {
    #[serde(default)]
    adopt: Vec<TypedKey>,
    #[serde(default)]
    leave_out: Vec<String>,
    #[serde(default)]
    no_adopt: bool,
}

impl From<ConfirmWire> for AdoptionRequest {
    fn from(w: ConfirmWire) -> Self {
        AdoptionRequest {
            adopt: w
                .adopt
                .into_iter()
                .map(|k| (k.key_id, k.fingerprint))
                .collect(),
            leave_out: w.leave_out,
            no_adopt: w.no_adopt,
        }
    }
}

#[derive(Serialize)]
pub struct CreatedView {
    owner: String,
    name: String,
    visibility: &'static str,
    proposals: &'static str,
    url: String,
    created_at: u64,
}

#[derive(Serialize)]
pub struct KeyView {
    key_id: String,
    /// The whole public key, 64 hexadecimal characters: what a maintainer reads off their own machine with
    /// `prikk key public`. The same text is `public_key` (two names: the first is what a person compares, the second what
    /// `trust maintainer add` takes).
    fingerprint: String,
    /// The same, in groups of eight for reading aloud and copying.
    fingerprint_grouped: String,
    public_key: String,
}

fn key_view(k: &FoundKey) -> KeyView {
    KeyView {
        key_id: k.key_id.clone(),
        fingerprint: k.public_key.clone(),
        fingerprint_grouped: group_fingerprint(&k.public_key),
        public_key: k.public_key.clone(),
    }
}

#[derive(Serialize)]
pub struct StageRefView {
    ref_name: String,
    /// With every carried key trusted, it would be a published branch.
    publishable: bool,
    landed_as: String,
    /// prikk's reason when it would not be published.
    reason: Option<String>,
}

#[derive(Serialize)]
pub struct StageView {
    stage_id: String,
    owner: String,
    name: String,
    visibility: &'static str,
    proposals: &'static str,
    archive_bytes: u64,
    archive_sha256: String,
    created_at: u64,
    expires_at: u64,
    /// prikk found nothing wrong inside the archive. Always true for a kept stage; **not** a statement of trust.
    internally_consistent: bool,
    keys: Vec<KeyView>,
    refs: Vec<StageRefView>,
    refs_total: u64,
    objects: std::collections::BTreeMap<String, u64>,
    object_total: u64,
    verifies_with_all_keys: bool,
    verify_failures: Vec<String>,
    note: Option<String>,
    /// One plain sentence in the caller's language: nothing is trusted yet.
    message: String,
}

pub(crate) fn stage_view(s: &StageSummary, t: &T) -> StageView {
    StageView {
        stage_id: s.stage_id.clone(),
        owner: s.owner.handle().to_owned(),
        name: s.name.clone(),
        visibility: visibility_word(s.visibility),
        proposals: s.proposals.as_str(),
        archive_bytes: s.archive_bytes,
        archive_sha256: s.archive_digest.clone(),
        created_at: s.created_at,
        expires_at: s.expires_at,
        internally_consistent: true,
        keys: s.findings.keys.iter().map(key_view).collect(),
        refs: s
            .findings
            .refs
            .iter()
            .map(|r| StageRefView {
                ref_name: r.ref_name.clone(),
                publishable: r.publishable,
                landed_as: r.landed_as.clone(),
                reason: r.reason.clone(),
            })
            .collect(),
        refs_total: s.findings.refs_total,
        objects: s.findings.objects.clone(),
        object_total: s.findings.object_total,
        verifies_with_all_keys: s.findings.verifies_with_all_keys,
        verify_failures: s.findings.verify_failures.clone(),
        note: s.findings.note.clone(),
        message: t.get(Key::StageIntro),
    }
}

#[derive(Serialize)]
pub struct LandingView {
    ref_name: String,
    landed_as: String,
    /// A published branch of the new repository.
    published: bool,
    /// prikk's reason when it is not.
    reason: Option<String>,
}

#[derive(Serialize)]
pub struct ConfirmedView {
    owner: String,
    name: String,
    url: String,
    /// The repository passed `verify`. False only when keys were left out: the branches they sign are under `remotes/`.
    verified: bool,
    adopted: Vec<KeyView>,
    left_out: Vec<KeyView>,
    landings: Vec<LandingView>,
    landings_total: u64,
    objects: u64,
    note: Option<String>,
    message: String,
}

pub(crate) fn confirmed_view(c: &Confirmed, t: &T) -> ConfirmedView {
    ConfirmedView {
        owner: c.record.owner.handle().to_owned(),
        name: c.record.name.clone(),
        url: format!("/{}/{}", c.record.owner.handle(), c.record.name),
        verified: c.verified,
        adopted: c.adopted.iter().map(key_view).collect(),
        left_out: c.left_out.iter().map(key_view).collect(),
        landings: c
            .landings
            .iter()
            .map(|l| LandingView {
                ref_name: l.ref_name.clone(),
                landed_as: l.landed_as.clone(),
                published: l.published,
                reason: l.reason.clone(),
            })
            .collect(),
        landings_total: c.landings_total,
        objects: c.objects,
        note: c.note.clone(),
        message: t.get(if c.verified {
            Key::CreatedVerified
        } else {
            Key::CreatedUnverified
        }),
    }
}

// ------------------------------------------------------------------------------------------------ refusals

/// Why a creation step did not happen, before it is turned into a response or a page.
pub(crate) enum Fail {
    Create(CreateError),
    /// The per-principal rate limit; seconds to wait.
    Rate(u64),
    /// The admission cost is above the memory budget.
    OverBudget,
    /// No slot in the wait.
    Busy,
    Internal,
}

impl From<CreateError> for Fail {
    fn from(e: CreateError) -> Self {
        Fail::Create(e)
    }
}

fn admit_fail(e: AdmitError) -> Fail {
    match e {
        AdmitError::TooLarge { .. } => Fail::OverBudget,
        AdmitError::Busy => Fail::Busy,
        AdmitError::Aborted => Fail::Internal,
    }
}

fn retry_after(mut resp: Response, secs: u64) -> Response {
    if let Ok(v) = HeaderValue::from_str(&secs.max(1).to_string()) {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

fn detail(
    state: &AppState,
    code: ErrorCode,
    keys: ErrorKeys,
    params: Vec<(&'static str, String)>,
    fields: Vec<(&'static str, serde_json::Value)>,
) -> Response {
    code_response_detailed(state, code, keys, ErrorDetail { params, fields })
}

/// A refusal as the API's error body. `owner` is the namespace the request named (for the quota sentence).
pub(crate) fn fail_response(state: &AppState, fail: &Fail, owner: &str) -> Response {
    use planeter_store::StageRefusal as S;
    let e = match fail {
        Fail::Rate(secs) => {
            return retry_after(
                code_response_with_keys(state, ErrorCode::RateLimited, ErrorCode::RATE_CREATIONS),
                *secs,
            );
        }
        Fail::OverBudget => return error_response(state, Failure::OverBudget),
        Fail::Busy => return error_response(state, Failure::Busy),
        Fail::Internal => return code_response(state, ErrorCode::Internal),
        Fail::Create(e) => e,
    };
    match e {
        CreateError::NotSignedIn => code_response_with_keys(
            state,
            ErrorCode::Forbidden,
            ErrorCode::FORBIDDEN_CREATE_SIGNIN,
        ),
        CreateError::Refused => {
            code_response_with_keys(state, ErrorCode::Forbidden, ErrorCode::FORBIDDEN_CREATE)
        }
        CreateError::Quota { limit, owned } => detail(
            state,
            ErrorCode::RepositoryQuota,
            ErrorCode::RepositoryQuota.keys(),
            vec![
                ("owner", owner.to_owned()),
                ("used", owned.to_string()),
                ("limit", limit.to_string()),
            ],
            vec![("limit", (*limit).into()), ("used", (*owned).into())],
        ),
        CreateError::NameTaken => code_response(state, ErrorCode::NameTaken),
        CreateError::ImportOff => {
            code_response_with_keys(state, ErrorCode::TransportOff, ErrorCode::IMPORT_OFF)
        }
        CreateError::Busy { retry_after: wait } => {
            retry_after(code_response(state, ErrorCode::Busy), wait.as_secs().max(1))
        }
        CreateError::TooLarge { limit } => detail(
            state,
            ErrorCode::PayloadTooLarge,
            ErrorCode::IMPORT_TOO_LARGE,
            vec![],
            vec![("limit", (*limit).into())],
        ),
        CreateError::StageLimit(S::TooManyForPrincipal { limit }) => detail(
            state,
            ErrorCode::StageLimit,
            ErrorCode::StageLimit.keys(),
            vec![("limit", limit.to_string())],
            vec![("limit", (*limit).into())],
        ),
        CreateError::StageLimit(S::BudgetExceeded { limit }) => detail(
            state,
            ErrorCode::StageLimit,
            ErrorCode::STAGE_BUDGET,
            vec![],
            vec![("limit_bytes", (*limit).into())],
        ),
        CreateError::StageLimit(S::IdTaken) => code_response(state, ErrorCode::Internal),
        CreateError::ArchiveRefused { text } => detail(
            state,
            ErrorCode::ArchiveRefused,
            ErrorCode::ArchiveRefused.keys(),
            vec![],
            vec![("detail", text.clone().into())],
        ),
        CreateError::Keys { carried } => detail(
            state,
            ErrorCode::ArchiveRefused,
            ErrorCode::ARCHIVE_KEYS,
            vec![],
            vec![("carried", (*carried).into())],
        ),
        CreateError::StageNotFound => code_response(state, ErrorCode::NotFound),
        CreateError::StageExpired => code_response(state, ErrorCode::StageExpired),
        CreateError::StageInUse => code_response(state, ErrorCode::StageInUse),
        CreateError::Adoption(a) => adoption_response(state, a),
        CreateError::VerifyFailed { conditions } => detail(
            state,
            ErrorCode::CreationVerifyFailed,
            ErrorCode::CreationVerifyFailed.keys(),
            vec![],
            vec![("conditions", conditions.clone().into())],
        ),
        CreateError::TimedOut => {
            code_response_with_keys(state, ErrorCode::Unavailable, ErrorCode::CREATE_TIMED_OUT)
        }
        CreateError::CeilingExceeded => {
            code_response_with_keys(state, ErrorCode::TooLarge, ErrorCode::CREATE_TOO_BIG)
        }
        _ => code_response(state, ErrorCode::Internal),
    }
}

fn adoption_response(state: &AppState, a: &AdoptionError) -> Response {
    let code = ErrorCode::AdoptionRefused;
    match a {
        AdoptionError::Mismatch { key_id } => detail(
            state,
            code,
            ErrorCode::ADOPT_MISMATCH,
            vec![("key", key_id.clone())],
            vec![
                ("reason", "mismatch".into()),
                ("key", key_id.clone().into()),
            ],
        ),
        AdoptionError::Malformed { key_id } => detail(
            state,
            code,
            ErrorCode::ADOPT_MALFORMED,
            vec![("key", key_id.clone())],
            vec![
                ("reason", "malformed".into()),
                ("key", key_id.clone().into()),
            ],
        ),
        AdoptionError::Unaddressed { key_ids } => detail(
            state,
            code,
            ErrorCode::ADOPT_UNADDRESSED,
            vec![("keys", key_ids.join(", "))],
            vec![
                ("reason", "unaddressed".into()),
                ("keys", key_ids.clone().into()),
            ],
        ),
        AdoptionError::Conflicting => detail(
            state,
            code,
            code.keys(),
            vec![],
            vec![("reason", "conflicting".into())],
        ),
        AdoptionError::UnknownKey { key_id } => detail(
            state,
            code,
            code.keys(),
            vec![],
            vec![
                ("reason", "unknown_key".into()),
                ("key", key_id.clone().into()),
            ],
        ),
        AdoptionError::Duplicate { key_id } => detail(
            state,
            code,
            code.keys(),
            vec![],
            vec![
                ("reason", "duplicate".into()),
                ("key", key_id.clone().into()),
            ],
        ),
    }
}

/// A field of a request that is not allowed: the `field` and the `reason` say which and why.
fn bad_value(state: &AppState, field: &'static str, reason: &'static str) -> Response {
    detail(
        state,
        ErrorCode::InvalidRequest,
        ErrorCode::BAD_NAME,
        vec![],
        vec![("field", field.into()), ("reason", reason.into())],
    )
}

fn request_fail(state: &AppState, e: RequestError) -> Response {
    match e {
        RequestError::Owner(n) => bad_value(state, "owner", n.reason()),
        RequestError::Name(n) => bad_value(state, "name", n.reason()),
        RequestError::Visibility => bad_value(state, "visibility", "unknown"),
        RequestError::Proposals => bad_value(state, "proposals", "unknown"),
    }
}

/// What a form says about a refused value, as the page keys: the field's error sentence.
pub(crate) fn request_error_key(e: RequestError) -> Key {
    match e {
        RequestError::Owner(_) => Key::NewErrorOwner,
        RequestError::Name(n) => match n {
            NameError::Empty => Key::NewErrorNameEmpty,
            NameError::TooLong => Key::NewErrorNameTooLong,
            NameError::Uppercase => Key::NewErrorNameUppercase,
            NameError::BadCharacter => Key::NewErrorNameBadCharacter,
            NameError::BadStart => Key::NewErrorNameBadStart,
            NameError::Dotted => Key::NewErrorNameDotted,
            NameError::Reserved => Key::NewErrorNameReserved,
        },
        RequestError::Visibility => Key::NewErrorVisibility,
        RequestError::Proposals => Key::NewErrorProposals,
    }
}

// ------------------------------------------------------------------------------------------------ shared steps

fn json_response(
    state: &AppState,
    status: StatusCode,
    body: Vec<u8>,
    headers: &HeaderMap,
) -> Response {
    let _ = headers;
    let mut resp = with_security_headers(state, Response::builder().status(status))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| code_response(state, ErrorCode::Internal));
    resp.extensions_mut().insert(crate::catalog::LanguageVaries);
    resp
}

/// The principal of a request that may not use a session cookie.
async fn bearer_principal(state: &AppState, headers: &HeaderMap) -> Principal {
    let mut bearer_only = headers.clone();
    bearer_only.remove(header::COOKIE);
    crate::principal::principal_from_request(&bearer_only, state).await
}

fn rate(state: &AppState, rt: &CreationRuntime, user: &UserId) -> Result<(), Fail> {
    rt.rate
        .check(&format!("user:{}", user.as_str()), (state.now_unix)())
        .map_err(Fail::Rate)
}

/// Everything that is decided before an archive's body is read, or an empty repository is made: the rate limit, who may
/// create there, and whether there is room. Returns the namespace.
pub(crate) async fn admit_new(
    state: &AppState,
    rt: &Arc<CreationRuntime>,
    principal: &Principal,
    user: &UserId,
    req: &NewRepo,
) -> Result<Owner, Fail> {
    rate(state, rt, user)?;
    let (svc, p, handle, name) = (
        Arc::clone(&rt.service),
        principal.clone(),
        req.owner.clone(),
        req.name.clone(),
    );
    offload(move || {
        let owner = svc.target(&p, &handle)?;
        svc.check_room(&owner, &name)?;
        Ok::<Owner, CreateError>(owner)
    })
    .await
    .map_err(|_| Fail::Internal)?
    .map_err(Fail::Create)
}

/// Create an empty repository (after [`admit_new`]).
pub(crate) async fn create_empty(
    state: &AppState,
    rt: &Arc<CreationRuntime>,
    user: &UserId,
    owner: Owner,
    req: NewRepo,
) -> Result<RepositoryRecord, Fail> {
    let (svc, creator) = (Arc::clone(&rt.service), user.clone());
    state
        .gate
        .run(move || svc.create_empty(owner, req.name, req.visibility, req.proposals, &creator))
        .await
        .map_err(|e| match e {
            crate::gate::GateError::Busy => Fail::Busy,
            _ => Fail::Internal,
        })?
        .map_err(Fail::Create)
}

/// An archive being written to a stage (the body arrives in pieces; each piece is written on the blocking pool).
pub(crate) struct StageWriter {
    upload: Option<StageUpload>,
}

impl StageWriter {
    /// Begin: the principal's limits are checked against `expected` before a byte is read.
    pub(crate) async fn begin(
        rt: &Arc<CreationRuntime>,
        user: &UserId,
        expected: u64,
    ) -> Result<StageWriter, Fail> {
        let (svc, user) = (Arc::clone(&rt.service), user.clone());
        let upload = offload(move || svc.start_stage(&user, expected))
            .await
            .map_err(|_| Fail::Internal)?
            .map_err(Fail::Create)?;
        Ok(StageWriter {
            upload: Some(upload),
        })
    }

    pub(crate) async fn write(&mut self, chunk: Vec<u8>) -> Result<(), Fail> {
        let mut upload = self.upload.take().ok_or(Fail::Internal)?;
        let (upload, result) = tokio::task::spawn_blocking(move || {
            let r = upload.write(&chunk);
            (upload, r)
        })
        .await
        .map_err(|_| Fail::Internal)?;
        match result {
            Ok(()) => {
                self.upload = Some(upload);
                Ok(())
            }
            // The upload (and its directory) is dropped here.
            Err(e) => Err(Fail::Create(e)),
        }
    }

    pub(crate) fn written(&self) -> u64 {
        self.upload.as_ref().map_or(0, StageUpload::written)
    }

    pub(crate) async fn finish(mut self) -> Result<StageUploaded, Fail> {
        let upload = self.upload.take().ok_or(Fail::Internal)?;
        offload(move || upload.finish())
            .await
            .map_err(|_| Fail::Internal)?
            .map_err(Fail::Create)
    }
}

/// Verify the uploaded archive and keep it as a stage: admitted against the memory budget as a write.
pub(crate) async fn finish_stage(
    state: &AppState,
    rt: &Arc<CreationRuntime>,
    uploaded: StageUploaded,
    meta: StageMeta,
    user: &UserId,
) -> Result<StageSummary, Fail> {
    let cost = cost_model::admission_cost(uploaded.size);
    let (svc, user) = (Arc::clone(&rt.service), user.clone());
    state
        .gate
        .run_weighted_heavy(cost, "import-stage", move || {
            svc.finish_stage(uploaded, meta, &user)
        })
        .await
        .map_err(admit_fail)?
        .map_err(Fail::Create)
}

/// The stage of this user, if there is one.
pub(crate) async fn stage_of(
    rt: &Arc<CreationRuntime>,
    user: &UserId,
    id: &str,
) -> Result<StageSummary, Fail> {
    let (svc, user, id) = (Arc::clone(&rt.service), user.clone(), id.to_owned());
    offload(move || svc.stage(&user, &id))
        .await
        .map_err(|_| Fail::Internal)?
        .map_err(Fail::Create)
}

/// Confirm a stage: admitted as a write by the stage's archive size.
pub(crate) async fn confirm(
    state: &AppState,
    rt: &Arc<CreationRuntime>,
    principal: &Principal,
    user: &UserId,
    id: &str,
    req: AdoptionRequest,
) -> Result<Confirmed, Fail> {
    rate(state, rt, user)?;
    // The size comes from the stage record; a stage that is not this user's, or is gone, is the one neutral answer.
    let summary = stage_of(rt, user, id).await?;
    let cost = cost_model::admission_cost(summary.archive_bytes);
    let (svc, p, u, id) = (
        Arc::clone(&rt.service),
        principal.clone(),
        user.clone(),
        id.to_owned(),
    );
    state
        .gate
        .run_weighted_heavy(cost, "import-confirm", move || {
            svc.confirm(&p, &u, &id, &req)
        })
        .await
        .map_err(admit_fail)?
        .map_err(Fail::Create)
}

// ------------------------------------------------------------------------------------------------ handlers

/// The bytes of a small JSON body, or a refusal.
async fn small_body(state: &AppState, body: Body, limit: usize) -> Result<Bytes, Response> {
    axum::body::to_bytes(body, limit)
        .await
        .map_err(|_| bad_value(state, "body", "too_large_or_unreadable"))
}

/// The common start of every API handler here: the signed-in user, or the refusal.
async fn signed_in(state: &AppState, headers: &HeaderMap) -> Result<(Principal, UserId), Response> {
    let principal = bearer_principal(state, headers).await;
    match principal.user().cloned() {
        Some(user) => Ok((principal, user)),
        None => Err(fail_response(
            state,
            &Fail::Create(CreateError::NotSignedIn),
            "",
        )),
    }
}

pub(crate) async fn create_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(rt) = state.creation.clone() else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let (principal, user) = match signed_in(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let bytes = match small_body(&state, body, 16 * 1024).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let wire: NewWire = match serde_json::from_slice(&bytes) {
        Ok(w) => w,
        Err(_) => return bad_value(&state, "body", "not_json_or_unknown_field"),
    };
    let req = match NewRepo::parse(
        &wire.owner,
        &wire.name,
        &wire.visibility,
        wire.proposals.as_deref(),
        &is_reserved,
    ) {
        Ok(r) => r,
        Err(e) => return request_fail(&state, e),
    };
    let owner_handle = req.owner.clone();
    let owner = match admit_new(&state, &rt, &principal, &user, &req).await {
        Ok(o) => o,
        Err(f) => return fail_response(&state, &f, &owner_handle),
    };
    match create_empty(&state, &rt, &user, owner, req).await {
        Ok(record) => {
            let view = CreatedView {
                owner: record.owner.handle().to_owned(),
                name: record.name.clone(),
                visibility: visibility_word(record.visibility),
                proposals: record.proposals.as_str(),
                url: format!("/{}/{}", record.owner.handle(), record.name),
                created_at: record.created_at,
            };
            json_response(
                &state,
                StatusCode::CREATED,
                serde_json::to_vec(&view).unwrap_or_default(),
                &headers,
            )
        }
        Err(f) => fail_response(&state, &f, &owner_handle),
    }
}

/// The first bytes of an archive creation's body.
const MAGIC: &[u8] = b"PLANETER-IMPORT-1\n";
/// The magic and the 8-byte big-endian metadata length.
const PREFIX_LEN: u64 = MAGIC.len() as u64 + 8;
/// The most metadata an archive creation carries.
const METADATA_MAX: u64 = 64 * 1024;

fn bad_framing(state: &AppState, reason: &'static str) -> Response {
    detail(
        state,
        ErrorCode::InvalidRequest,
        ErrorCode::BAD_IMPORT_FRAMING,
        vec![],
        vec![("reason", reason.into())],
    )
}

pub(crate) async fn import_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(rt) = state.creation.clone() else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let (principal, user) = match signed_in(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if !rt.service.import_enabled() {
        return fail_response(&state, &Fail::Create(CreateError::ImportOff), "");
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
    let max = rt.service.config().max_archive_bytes;
    let too_large = |state: &AppState| {
        detail(
            state,
            ErrorCode::PayloadTooLarge,
            ErrorCode::IMPORT_TOO_LARGE,
            vec![],
            vec![("limit", max.into())],
        )
    };
    if total > max + METADATA_MAX + PREFIX_LEN {
        return too_large(&state);
    }
    if total < PREFIX_LEN {
        return bad_framing(&state, "too_short");
    }

    // The prefix and the metadata, read as they arrive; what follows the metadata in the last chunk read is the start of
    // the archive (`carry`).
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
    let carry = buf.split_off((PREFIX_LEN + meta_len.unwrap_or(0)) as usize);
    let archive_len = total - PREFIX_LEN - meta_len.unwrap_or(0);
    if archive_len > max {
        return too_large(&state);
    }
    let wire: NewWire = match serde_json::from_slice(&buf[PREFIX_LEN as usize..]) {
        Ok(w) => w,
        Err(_) => return bad_framing(&state, "metadata"),
    };
    let req = match NewRepo::parse(
        &wire.owner,
        &wire.name,
        &wire.visibility,
        wire.proposals.as_deref(),
        &is_reserved,
    ) {
        Ok(r) => r,
        Err(e) => return request_fail(&state, e),
    };
    // Who may create there and whether there is room: before one byte of the archive is read.
    let owner_handle = req.owner.clone();
    let owner = match admit_new(&state, &rt, &principal, &user, &req).await {
        Ok(o) => o,
        Err(f) => return fail_response(&state, &f, &owner_handle),
    };
    let meta = StageMeta {
        owner,
        name: req.name.clone(),
        visibility: req.visibility,
        proposals: req.proposals,
    };
    let mut writer = match StageWriter::begin(&rt, &user, archive_len).await {
        Ok(w) => w,
        Err(f) => return fail_response(&state, &f, &owner_handle),
    };
    let mut pending = carry;
    loop {
        if !pending.is_empty() {
            let chunk = std::mem::take(&mut pending);
            if let Err(f) = writer.write(chunk).await {
                return match f {
                    Fail::Create(CreateError::TooLarge { .. }) => bad_framing(&state, "length"),
                    other => fail_response(&state, &other, &owner_handle),
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
    let uploaded = match writer.finish().await {
        Ok(u) => u,
        Err(f) => return fail_response(&state, &f, &owner_handle),
    };
    match finish_stage(&state, &rt, uploaded, meta, &user).await {
        Ok(summary) => {
            let t = T::new(negotiate_from_headers(&headers));
            json_response(
                &state,
                StatusCode::CREATED,
                serde_json::to_vec(&stage_view(&summary, &t)).unwrap_or_default(),
                &headers,
            )
        }
        Err(f) => fail_response(&state, &f, &owner_handle),
    }
}

pub(crate) async fn stage_handler(
    State(state): State<AppState>,
    Path(stage): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(rt) = state.creation.clone() else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let (_, user) = match signed_in(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match stage_of(&rt, &user, &stage).await {
        Ok(summary) => {
            let t = T::new(negotiate_from_headers(&headers));
            json_response(
                &state,
                StatusCode::OK,
                serde_json::to_vec(&stage_view(&summary, &t)).unwrap_or_default(),
                &headers,
            )
        }
        Err(f) => fail_response(&state, &f, ""),
    }
}

pub(crate) async fn confirm_handler(
    State(state): State<AppState>,
    Path(stage): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(rt) = state.creation.clone() else {
        return code_response(&state, ErrorCode::TransportOff);
    };
    let (principal, user) = match signed_in(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let bytes = match small_body(&state, body, 64 * 1024).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let wire: ConfirmWire = match serde_json::from_slice(&bytes) {
        Ok(w) => w,
        Err(_) => return bad_value(&state, "body", "not_json_or_unknown_field"),
    };
    // The namespace for the quota sentence: the stage's own.
    let owner_handle = match stage_of(&rt, &user, &stage).await {
        Ok(s) => s.owner.handle().to_owned(),
        Err(f) => return fail_response(&state, &f, ""),
    };
    match confirm(&state, &rt, &principal, &user, &stage, wire.into()).await {
        Ok(done) => {
            let t = T::new(negotiate_from_headers(&headers));
            json_response(
                &state,
                StatusCode::CREATED,
                serde_json::to_vec(&confirmed_view(&done, &t)).unwrap_or_default(),
                &headers,
            )
        }
        Err(f) => fail_response(&state, &f, &owner_handle),
    }
}
