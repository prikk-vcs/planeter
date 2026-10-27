//! Creating a repository on the pages (RFC 004 part 7, T5): `/new` with its two plain choices, the screen that shows what an
//! uploaded archive contains, and the result. No script is needed: every step is an ordinary form with a CSRF token.
//!
//! **The fingerprint fields are empty on every rendering, and stay empty after a refusal.** Nothing is ever prefilled: the
//! creator types what the maintainer told them, or pastes it. The carried fingerprints are shown *beside* the fields, for
//! comparison; the page cannot make comparing them easier by filling them in, because then there would be nothing to compare.

use std::collections::BTreeMap;
use std::time::Instant;

use askama::Template;
use axum::body::Body;
use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use planeter_auth::tokens_match;
use planeter_core::creation::{AdoptionError, AdoptionRequest, NewRepo, group_fingerprint};
use planeter_transport::{Confirmed, CreateError, StageMeta, StageSummary};

use super::{Ctx, MessageTpl, Who, ctx, error_page, pct, redirect, render};
use crate::api::{AppState, Failure};
use crate::catalog::{Key, T};
use crate::cookies::{CSRF_COOKIE, get_cookie};
use crate::creation::{
    CreationRuntime, Fail, StageWriter, admit_new, confirm, create_empty, finish_stage,
    is_reserved, request_error_key, stage_of,
};
use crate::multipart::{Event, Parser};
use crate::timefmt;
use crate::transport::{UPLOAD_TOTAL, next_chunk};

// ------------------------------------------------------------------------------------------------ what the templates show

/// The values of the creation form as it was last sent (kept on a refusal, except the file).
pub(super) struct Values {
    pub owner: String,
    pub name: String,
    pub visibility: String,
    pub proposals: String,
}

#[derive(Template)]
#[template(path = "new.html")]
struct NewTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    v: Values,
    error: Option<String>,
    import_enabled: bool,
    max_note: String,
}

pub(super) struct KeyRow {
    pub index: usize,
    pub title: String,
    pub fingerprint_label: String,
    pub grouped: String,
}

#[derive(Template)]
#[template(path = "stage.html")]
struct StageTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    repo: String,
    stage_id: String,
    error: Option<String>,
    error_detail: Option<String>,
    size_line: String,
    digest_line: String,
    objects_line: String,
    expires_line: String,
    refs: Vec<String>,
    refs_more: Option<String>,
    verifies_line: String,
    keys: Vec<KeyRow>,
    confirm_note: String,
    none_note: String,
}

#[derive(Template)]
#[template(path = "created.html")]
struct CreatedTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    owner: String,
    name: String,
    repo: String,
    verified_line: String,
    landings: Vec<String>,
    landings_more: Option<String>,
    trusted_line: String,
}

/// `12.5 MiB`-style text for a byte count. Units are not translated (they are the symbols everyone reads the same).
fn size_text(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else if b < KIB * KIB * KIB {
        format!("{:.1} MiB", b / KIB / KIB)
    } else {
        format!("{:.1} GiB", b / KIB / KIB / KIB)
    }
}

fn default_values(c: &Ctx) -> Values {
    Values {
        owner: c.user.as_ref().map(|u| u.id.clone()).unwrap_or_default(),
        name: String::new(),
        visibility: "private".into(),
        proposals: "members".into(),
    }
}

fn max_note(c: &Ctx, rt: &CreationRuntime) -> String {
    c.t.msg(Key::NewImportMax)
        .with("size", size_text(rt.service.config().max_archive_bytes))
        .text()
}

fn new_page_with(
    state: &AppState,
    c: &Ctx,
    status: StatusCode,
    v: Values,
    error: Option<String>,
) -> Response {
    let rt = state.creation.as_ref();
    let tpl = NewTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        v,
        error,
        import_enabled: rt.is_some_and(|r| r.service.import_enabled()),
        max_note: rt.map(|r| max_note(c, r)).unwrap_or_default(),
    };
    render(state, status, &tpl, c)
}

fn both(t: &T, message: Key, hint: Key, params: &[(&'static str, String)]) -> String {
    let say = |key: Key| {
        let mut m = t.msg(key);
        for (k, v) in params {
            m = m.with(k, v);
        }
        m.text()
    };
    format!("{} {}", say(message), say(hint))
}

/// A refusal as the sentences a page says, with its status. `owner` is the namespace named (for the quota sentence).
fn fail_text(t: &T, fail: &Fail, owner: &str) -> (StatusCode, String, Option<String>) {
    use planeter_store::StageRefusal as S;
    let unprocessable = StatusCode::UNPROCESSABLE_ENTITY;
    let e = match fail {
        Fail::Rate(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                both(
                    t,
                    Key::ErrRateCreationsMessage,
                    Key::ErrRateCreationsHint,
                    &[],
                ),
                None,
            );
        }
        Fail::OverBudget => {
            return (
                unprocessable,
                both(
                    t,
                    Key::ErrCreateTooBigMessage,
                    Key::ErrCreateTooBigHint,
                    &[],
                ),
                None,
            );
        }
        Fail::Busy => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                both(t, Key::ErrBusyMessage, Key::ErrBusyHint, &[]),
                None,
            );
        }
        Fail::Internal => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                both(t, Key::ErrInternalMessage, Key::ErrInternalHint, &[]),
                None,
            );
        }
        Fail::Create(e) => e,
    };
    match e {
        CreateError::NotSignedIn => (
            StatusCode::FORBIDDEN,
            both(
                t,
                Key::ErrForbiddenCreateSigninMessage,
                Key::ErrForbiddenCreateSigninHint,
                &[],
            ),
            None,
        ),
        CreateError::Refused => (
            StatusCode::FORBIDDEN,
            both(
                t,
                Key::ErrForbiddenCreateMessage,
                Key::ErrForbiddenCreateHint,
                &[],
            ),
            None,
        ),
        CreateError::Quota { limit, owned } => (
            unprocessable,
            both(
                t,
                Key::ErrRepositoryQuotaMessage,
                Key::ErrRepositoryQuotaHint,
                &[
                    ("owner", owner.to_owned()),
                    ("used", owned.to_string()),
                    ("limit", limit.to_string()),
                ],
            ),
            None,
        ),
        CreateError::NameTaken => (
            StatusCode::CONFLICT,
            both(t, Key::ErrNameTakenMessage, Key::ErrNameTakenHint, &[]),
            None,
        ),
        CreateError::ImportOff => (
            StatusCode::SERVICE_UNAVAILABLE,
            t.get(Key::NewImportOff),
            None,
        ),
        CreateError::Busy { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            both(t, Key::ErrBusyMessage, Key::ErrBusyHint, &[]),
            None,
        ),
        CreateError::TooLarge { .. } => (
            StatusCode::PAYLOAD_TOO_LARGE,
            both(
                t,
                Key::ErrImportTooLargeMessage,
                Key::ErrImportTooLargeHint,
                &[],
            ),
            None,
        ),
        CreateError::StageLimit(S::TooManyForPrincipal { limit }) => (
            unprocessable,
            both(
                t,
                Key::ErrStageLimitMessage,
                Key::ErrStageLimitHint,
                &[("limit", limit.to_string())],
            ),
            None,
        ),
        CreateError::StageLimit(_) => (
            unprocessable,
            both(t, Key::ErrStageBudgetMessage, Key::ErrStageBudgetHint, &[]),
            None,
        ),
        CreateError::ArchiveRefused { text } => (
            unprocessable,
            both(
                t,
                Key::ErrArchiveRefusedMessage,
                Key::ErrArchiveRefusedHint,
                &[],
            ),
            Some(t.msg(Key::StagePrikk).with("detail", text).text()),
        ),
        CreateError::Keys { .. } => (
            unprocessable,
            both(t, Key::ErrArchiveKeysMessage, Key::ErrArchiveKeysHint, &[]),
            None,
        ),
        CreateError::StageNotFound => (StatusCode::NOT_FOUND, t.get(Key::StageGone), None),
        CreateError::StageExpired => (
            StatusCode::GONE,
            both(
                t,
                Key::ErrStageExpiredMessage,
                Key::ErrStageExpiredHint,
                &[],
            ),
            None,
        ),
        CreateError::StageInUse => (
            StatusCode::CONFLICT,
            both(t, Key::ErrStageInUseMessage, Key::ErrStageInUseHint, &[]),
            None,
        ),
        CreateError::Adoption(a) => (unprocessable, adoption_text(t, a), None),
        CreateError::VerifyFailed { conditions } => (
            unprocessable,
            both(
                t,
                Key::ErrCreationVerifyMessage,
                Key::ErrCreationVerifyHint,
                &[],
            ),
            Some(
                t.msg(Key::StageErrorVerify)
                    .with("conditions", conditions.join(", "))
                    .text(),
            ),
        ),
        CreateError::TimedOut => (
            StatusCode::SERVICE_UNAVAILABLE,
            both(
                t,
                Key::ErrCreateTimedOutMessage,
                Key::ErrCreateTimedOutHint,
                &[],
            ),
            None,
        ),
        CreateError::CeilingExceeded => (
            unprocessable,
            both(
                t,
                Key::ErrCreateTooBigMessage,
                Key::ErrCreateTooBigHint,
                &[],
            ),
            None,
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            both(t, Key::ErrInternalMessage, Key::ErrInternalHint, &[]),
            None,
        ),
    }
}

fn adoption_text(t: &T, a: &AdoptionError) -> String {
    match a {
        AdoptionError::Mismatch { key_id } => both(
            t,
            Key::ErrAdoptMismatchMessage,
            Key::ErrAdoptMismatchHint,
            &[("key", key_id.clone())],
        ),
        AdoptionError::Malformed { key_id } => both(
            t,
            Key::ErrAdoptMalformedMessage,
            Key::ErrAdoptMalformedHint,
            &[("key", key_id.clone())],
        ),
        AdoptionError::Unaddressed { key_ids } => both(
            t,
            Key::ErrAdoptUnaddressedMessage,
            Key::ErrAdoptUnaddressedHint,
            &[("keys", key_ids.join(", "))],
        ),
        AdoptionError::Duplicate { key_id } => {
            t.msg(Key::StageErrorBoth).with("key", key_id).text()
        }
        _ => both(t, Key::ErrAdoptOtherMessage, Key::ErrAdoptOtherHint, &[]),
    }
}

// ------------------------------------------------------------------------------------------------ small checks

fn csrf_ok(headers: &HeaderMap, token: &str) -> bool {
    let cookie = get_cookie(headers, CSRF_COOKIE).unwrap_or_default();
    !cookie.is_empty() && tokens_match(&cookie, token)
}

fn csrf_failed(state: &AppState, c: &Ctx) -> Response {
    super::notice(
        state,
        c,
        StatusCode::FORBIDDEN,
        super::Notice::CsrfFailed,
        super::Next::Home,
        vec![],
    )
}

/// The page for a request that needs an account and has none.
fn sign_in_page(state: &AppState, c: &Ctx) -> Response {
    new_page_with(state, c, StatusCode::FORBIDDEN, default_values(c), None)
}

fn unavailable_page(state: &AppState, c: &Ctx, at: &str) -> Response {
    error_page(state, c, at, Failure::Internal)
}

/// A refusal of the creation form: the form again, with what was sent and the sentences.
fn form_refused(state: &AppState, c: &Ctx, v: Values, fail: &Fail, owner: &str) -> Response {
    let (status, text, detail) = fail_text(&c.t, fail, owner);
    // prikk's own words, when it refused an archive, follow our sentence.
    let text = detail.map_or(text.clone(), |d| format!("{text} {d}"));
    new_page_with(state, c, status, v, Some(text))
}

/// A name the form's four values do not pass: the form again, with the field's sentence.
fn invalid_values(
    state: &AppState,
    c: &Ctx,
    v: Values,
    e: planeter_core::creation::RequestError,
) -> Response {
    let text = c.t.get(request_error_key(e));
    new_page_with(state, c, StatusCode::UNPROCESSABLE_ENTITY, v, Some(text))
}

// ------------------------------------------------------------------------------------------------ /new

pub(super) async fn new_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let c = ctx(&headers, &state).await;
    let status = if c.user.is_some() {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    new_page_with(&state, &c, status, default_values(&c), None)
}

#[derive(serde::Deserialize)]
pub(super) struct NewForm {
    csrf: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    proposals: String,
}

pub(super) async fn new_empty_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<NewForm>, FormRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    let Ok(Form(f)) = form else {
        return super::notice(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            super::Notice::FormIncomplete,
            super::Next::Home,
            vec![],
        );
    };
    if !csrf_ok(&headers, &f.csrf) {
        return csrf_failed(&state, &c);
    }
    let (Some(rt), Some(user)) = (state.creation.clone(), c.principal.user().cloned()) else {
        return sign_in_page(&state, &c);
    };
    let values = || Values {
        owner: f.owner.clone(),
        name: f.name.clone(),
        visibility: f.visibility.clone(),
        proposals: f.proposals.clone(),
    };
    let req = match NewRepo::parse(
        &f.owner,
        &f.name,
        &f.visibility,
        Some(&f.proposals),
        &is_reserved,
    ) {
        Ok(r) => r,
        Err(e) => return invalid_values(&state, &c, values(), e),
    };
    let owner_handle = req.owner.clone();
    let owner = match admit_new(&state, &rt, &c.principal, &user, &req).await {
        Ok(o) => o,
        Err(fail) => return form_refused(&state, &c, values(), &fail, &owner_handle),
    };
    match create_empty(&state, &rt, &user, owner, req).await {
        Ok(record) => redirect(
            &state,
            &format!("/{}/{}", pct(record.owner.handle()), pct(&record.name)),
            c.set_csrf.iter().cloned().collect(),
        ),
        Err(fail) => form_refused(&state, &c, values(), &fail, &owner_handle),
    }
}

// ------------------------------------------------------------------------------------------------ /new/import

/// The form fields the upload may carry, in the order the form sends them.
const FIELDS: [&str; 5] = ["csrf", "owner", "name", "visibility", "proposals"];

pub(super) async fn new_import_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let c = ctx(&headers, &state).await;
    let (Some(rt), Some(user)) = (state.creation.clone(), c.principal.user().cloned()) else {
        return sign_in_page(&state, &c);
    };
    let again = |status: StatusCode, v: Values, text: String| {
        new_page_with(&state, &c, status, v, Some(text))
    };
    let upload_error = |v: Values| {
        again(
            StatusCode::UNPROCESSABLE_ENTITY,
            v,
            c.t.get(Key::NewErrorUpload),
        )
    };
    if !rt.service.import_enabled() {
        return again(
            StatusCode::SERVICE_UNAVAILABLE,
            default_values(&c),
            c.t.get(Key::NewImportOff),
        );
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let Ok(mut parser) = Parser::new(&content_type) else {
        return upload_error(default_values(&c));
    };
    let max = rt.service.config().max_archive_bytes;
    let Some(total) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
    else {
        return upload_error(default_values(&c));
    };
    // The archive plus the form's own bytes (the fields and the part headers are far below this).
    if total > max + 64 * 1024 {
        let (status, text, _) = fail_text(
            &c.t,
            &Fail::Create(CreateError::TooLarge { limit: max }),
            "",
        );
        return again(status, default_values(&c), text);
    }

    let deadline = Instant::now() + UPLOAD_TOTAL;
    let mut stream = body.into_data_stream();
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut writer: Option<StageWriter> = None;
    let mut meta: Option<StageMeta> = None;
    let mut owner_handle = String::new();
    let mut uploaded = None;
    'body: loop {
        let chunk = match next_chunk(&mut stream, deadline).await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(()) => return upload_error(values_of(&fields, &c)),
        };
        let events = match parser.push(&chunk) {
            Ok(e) => e,
            Err(_) => return upload_error(values_of(&fields, &c)),
        };
        for ev in events {
            match ev {
                Event::Text { name, value } => {
                    if !FIELDS.contains(&name.as_str()) || fields.insert(name, value).is_some() {
                        return upload_error(values_of(&fields, &c));
                    }
                }
                Event::FileStart => {
                    // Before one byte of the file is read: the token, the values, who may create there, the limits.
                    if !csrf_ok(&headers, fields.get("csrf").map_or("", String::as_str)) {
                        return csrf_failed(&state, &c);
                    }
                    let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
                    let req = match NewRepo::parse(
                        &get("owner"),
                        &get("name"),
                        &get("visibility"),
                        Some(&get("proposals")),
                        &is_reserved,
                    ) {
                        Ok(r) => r,
                        Err(e) => return invalid_values(&state, &c, values_of(&fields, &c), e),
                    };
                    owner_handle = req.owner.clone();
                    let owner = match admit_new(&state, &rt, &c.principal, &user, &req).await {
                        Ok(o) => o,
                        Err(fail) => {
                            return form_refused(
                                &state,
                                &c,
                                values_of(&fields, &c),
                                &fail,
                                &owner_handle,
                            );
                        }
                    };
                    meta = Some(StageMeta {
                        owner,
                        name: req.name.clone(),
                        visibility: req.visibility,
                        proposals: req.proposals,
                    });
                    writer = match StageWriter::begin(&rt, &user, total.min(max)).await {
                        Ok(w) => Some(w),
                        Err(fail) => {
                            return form_refused(
                                &state,
                                &c,
                                values_of(&fields, &c),
                                &fail,
                                &owner_handle,
                            );
                        }
                    };
                }
                Event::FileData(d) => {
                    let Some(w) = writer.as_mut() else {
                        return upload_error(values_of(&fields, &c));
                    };
                    if let Err(fail) = w.write(d).await {
                        return form_refused(
                            &state,
                            &c,
                            values_of(&fields, &c),
                            &fail,
                            &owner_handle,
                        );
                    }
                }
                Event::FileEnd => {
                    let Some(w) = writer.take() else {
                        return upload_error(values_of(&fields, &c));
                    };
                    uploaded = match w.finish().await {
                        Ok(u) => Some(u),
                        Err(fail) => {
                            return form_refused(
                                &state,
                                &c,
                                values_of(&fields, &c),
                                &fail,
                                &owner_handle,
                            );
                        }
                    };
                }
                Event::End => break 'body,
            }
        }
    }
    if parser.finish().is_err() {
        return upload_error(values_of(&fields, &c));
    }
    let (Some(uploaded), Some(meta)) = (uploaded, meta) else {
        // No file part: a form sent without a file.
        return again(
            StatusCode::UNPROCESSABLE_ENTITY,
            values_of(&fields, &c),
            c.t.get(Key::NewErrorFile),
        );
    };
    match finish_stage(&state, &rt, uploaded, meta, &user).await {
        Ok(summary) => redirect(
            &state,
            &format!("/new/import/{}", summary.stage_id),
            c.set_csrf.iter().cloned().collect(),
        ),
        Err(fail) => form_refused(&state, &c, values_of(&fields, &c), &fail, &owner_handle),
    }
}

fn values_of(fields: &BTreeMap<String, String>, c: &Ctx) -> Values {
    let d = default_values(c);
    let get = |k: &str, default: String| fields.get(k).cloned().unwrap_or(default);
    Values {
        owner: get("owner", d.owner),
        name: get("name", d.name),
        visibility: get("visibility", d.visibility),
        proposals: get("proposals", d.proposals),
    }
}

// ------------------------------------------------------------------------------------------------ the stage screen

fn stage_page_with(
    state: &AppState,
    c: &Ctx,
    status: StatusCode,
    s: &StageSummary,
    error: Option<(String, Option<String>)>,
) -> Response {
    let t = &c.t;
    let repo = format!("{}/{}", s.owner.handle(), s.name);
    let f = &s.findings;
    let refs: Vec<String> = f
        .refs
        .iter()
        .map(|r| {
            if r.ref_name.starts_with("remotes/") {
                t.msg(Key::StageRefReceived).with("ref", &r.ref_name).text()
            } else if r.publishable {
                t.msg(Key::StageRefPublished)
                    .with("ref", &r.ref_name)
                    .text()
            } else {
                let reason = r.reason.clone().unwrap_or_default();
                t.msg(Key::StageRefKept)
                    .with("ref", &r.ref_name)
                    .with("reason", reason)
                    .text()
            }
        })
        .collect();
    let more = f.refs_total.saturating_sub(f.refs.len() as u64);
    let confirm_note = t
        .msg(Key::StageConfirmConsequence)
        .with("repo", &repo)
        .text();
    let none_note = t.msg(Key::StageNoneConsequence).with("repo", &repo).text();
    let tpl = StageTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        repo: repo.clone(),
        stage_id: s.stage_id.clone(),
        error: error.as_ref().map(|(e, _)| e.clone()),
        error_detail: error.and_then(|(_, d)| d),
        size_line: t
            .msg(Key::StageSize)
            .with("size", size_text(s.archive_bytes))
            .text(),
        digest_line: t
            .msg(Key::StageDigest)
            .with("digest", &s.archive_digest)
            .text(),
        objects_line: t.msg(Key::StageObjects).n(f.object_total).text(),
        expires_line: t
            .msg(Key::StageExpires)
            .with("when", timefmt::minute(s.expires_at))
            .text(),
        refs,
        refs_more: (more > 0).then(|| t.msg(Key::StageRefMore).n(more).text()),
        verifies_line: if f.verifies_with_all_keys {
            t.get(Key::StageVerifiesYes)
        } else {
            t.msg(Key::StageVerifiesNo)
                .with("problems", f.verify_failures.join(", "))
                .text()
        },
        keys: f
            .keys
            .iter()
            .enumerate()
            .map(|(index, k)| KeyRow {
                index,
                title: t.msg(Key::StageKeyId).with("key", &k.key_id).text(),
                fingerprint_label: t.get(Key::StageKeyFingerprint),
                grouped: group_fingerprint(&k.public_key),
            })
            .collect(),
        confirm_note,
        none_note,
    };
    render(state, status, &tpl, c)
}

/// A refusal to show a stage at all (not found, expired, …) or an outage.
fn stage_unreachable(state: &AppState, c: &Ctx, at: &str, fail: &Fail) -> Response {
    match fail {
        Fail::Create(CreateError::StageNotFound) => {
            let tpl = MessageTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
                t: c.t,
                title: c.t.msg(Key::TitleStage).product().text(),
                heading: c.t.get(Key::NewRefusedHeading),
                body: c.t.get(Key::StageGone),
                link_label: c.t.get(Key::StageGoneLink),
                link_href: "/new".to_owned(),
            };
            render(state, StatusCode::NOT_FOUND, &tpl, c)
        }
        Fail::Create(
            e @ (CreateError::StageExpired | CreateError::NotSignedIn | CreateError::ImportOff),
        ) => {
            let (status, text, _) = fail_text(&c.t, &Fail::Create(clone_simple(e)), "");
            let tpl = MessageTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
                t: c.t,
                title: c.t.msg(Key::TitleStage).product().text(),
                heading: c.t.get(Key::NewRefusedHeading),
                body: text,
                link_label: c.t.get(Key::StageGoneLink),
                link_href: "/new".to_owned(),
            };
            render(state, status, &tpl, c)
        }
        _ => unavailable_page(state, c, at),
    }
}

fn clone_simple(e: &CreateError) -> CreateError {
    match e {
        CreateError::StageExpired => CreateError::StageExpired,
        CreateError::NotSignedIn => CreateError::NotSignedIn,
        _ => CreateError::ImportOff,
    }
}

pub(super) async fn stage_page(
    State(state): State<AppState>,
    Path(stage): Path<String>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let at = format!("/new/import/{stage}");
    let (Some(rt), Some(user)) = (state.creation.clone(), c.principal.user().cloned()) else {
        return stage_unreachable(&state, &c, &at, &Fail::Create(CreateError::NotSignedIn));
    };
    match stage_of(&rt, &user, &stage).await {
        Ok(s) => stage_page_with(&state, &c, StatusCode::OK, &s, None),
        Err(fail) => stage_unreachable(&state, &c, &at, &fail),
    }
}

/// What the confirmation answers for the typed fingerprints: built from the form, with the stage's own key order.
fn adoption_from_form(
    form: &BTreeMap<String, String>,
    keys: &[String],
) -> Result<AdoptionRequest, AdoptionError> {
    if form.get("mode").is_some_and(|m| m == "none") {
        return Ok(AdoptionRequest {
            no_adopt: true,
            ..AdoptionRequest::default()
        });
    }
    let mut req = AdoptionRequest::default();
    for (i, key_id) in keys.iter().enumerate() {
        let typed = form.get(&format!("fp_{i}")).map(|s| s.trim()).unwrap_or("");
        let skip = form.contains_key(&format!("skip_{i}"));
        match (typed.is_empty(), skip) {
            (true, true) => req.leave_out.push(key_id.clone()),
            (false, true) => {
                return Err(AdoptionError::Duplicate {
                    key_id: key_id.clone(),
                });
            }
            (false, false) => req.adopt.push((key_id.clone(), typed.to_owned())),
            // Neither: the service says which keys were not answered.
            (true, false) => {}
        }
    }
    Ok(req)
}

pub(super) async fn confirm_submit(
    State(state): State<AppState>,
    Path(stage): Path<String>,
    headers: HeaderMap,
    form: Result<Form<BTreeMap<String, String>>, FormRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    let at = format!("/new/import/{stage}");
    let Ok(Form(form)) = form else {
        return super::notice(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            super::Notice::FormIncomplete,
            super::Next::Home,
            vec![],
        );
    };
    if !csrf_ok(&headers, form.get("csrf").map_or("", String::as_str)) {
        return csrf_failed(&state, &c);
    }
    let (Some(rt), Some(user)) = (state.creation.clone(), c.principal.user().cloned()) else {
        return stage_unreachable(&state, &c, &at, &Fail::Create(CreateError::NotSignedIn));
    };
    let summary = match stage_of(&rt, &user, &stage).await {
        Ok(s) => s,
        Err(fail) => return stage_unreachable(&state, &c, &at, &fail),
    };
    let key_ids: Vec<String> = summary
        .findings
        .keys
        .iter()
        .map(|k| k.key_id.clone())
        .collect();
    let refused = |fail: Fail| {
        let (status, text, detail) = fail_text(&c.t, &fail, summary.owner.handle());
        stage_page_with(&state, &c, status, &summary, Some((text, detail)))
    };
    let req = match adoption_from_form(&form, &key_ids) {
        Ok(r) => r,
        Err(a) => return refused(Fail::Create(CreateError::Adoption(a))),
    };
    match confirm(&state, &rt, &c.principal, &user, &stage, req).await {
        Ok(done) => created_page(&state, &c, &done),
        Err(
            fail @ Fail::Create(
                CreateError::StageNotFound | CreateError::StageExpired | CreateError::NotSignedIn,
            ),
        ) => stage_unreachable(&state, &c, &at, &fail),
        Err(fail) => refused(fail),
    }
}

fn created_page(state: &AppState, c: &Ctx, done: &Confirmed) -> Response {
    let t = &c.t;
    let repo = format!("{}/{}", done.record.owner.handle(), done.record.name);
    let landings: Vec<String> = done
        .landings
        .iter()
        .map(|l| {
            if l.published {
                t.msg(Key::CreatedBranchPublished)
                    .with("ref", &l.ref_name)
                    .text()
            } else if let Some(reason) = &l.reason {
                t.msg(Key::CreatedBranchKept)
                    .with("ref", &l.ref_name)
                    .with("landed", &l.landed_as)
                    .with("reason", reason)
                    .text()
            } else {
                t.msg(Key::CreatedBranchKeptPlain)
                    .with("ref", &l.ref_name)
                    .with("landed", &l.landed_as)
                    .text()
            }
        })
        .collect();
    let more = done
        .landings_total
        .saturating_sub(done.landings.len() as u64);
    let tpl = CreatedTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        owner: pct(done.record.owner.handle()),
        name: pct(&done.record.name),
        repo,
        verified_line: t.get(if done.verified {
            Key::CreatedVerified
        } else {
            Key::CreatedUnverified
        }),
        landings,
        landings_more: (more > 0).then(|| t.msg(Key::StageRefMore).n(more).text()),
        trusted_line: if done.adopted.is_empty() {
            t.get(Key::CreatedNoneTrusted)
        } else {
            t.msg(Key::CreatedTrusted)
                .with(
                    "keys",
                    done.adopted
                        .iter()
                        .map(|k| k.key_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
                .text()
        },
    };
    render(state, StatusCode::OK, &tpl, c)
}
