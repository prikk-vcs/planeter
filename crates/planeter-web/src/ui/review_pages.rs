//! Review on the proposal page (RFC 005 part 6, T4): the review line of the default view, and — under *Details* — the
//! decisions, the thread of comments and the forms the viewer may use; and the four form handlers.
//!
//! The same rules as the API's, through the same service. **A hidden comment's text reaches a page in one place only**
//! (`comment_row`), and only for a viewer who administers the repository (`crate::reviews::visible_body`).
//! No script is needed: every form is an ordinary form with a CSRF token, and a refusal is a page with a way back.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use planeter_auth::tokens_match;
use planeter_core::review::{CommentRefusal, DecisionRefusal, HideRefusal, ReviewState, Verdict};
use planeter_core::{Action, ReadError};
use planeter_store::{ChangeState, CommentRecord, RepositoryRecord};
use planeter_transport::{
    Detail, Limited, NewComment, ReviewError, ReviewService, THREADS_LIMIT_DEFAULT, Thread,
};

use super::{Ctx, ctx, error_page, pct, redirect, render, short};
use crate::api::{AppState, Failure};
use crate::catalog::{Key, T};
use crate::cookies::{CSRF_COOKIE, get_cookie};
use crate::gate::offload;
use crate::timefmt;
use crate::transport::readable;

// ------------------------------------------------------------------------------------------------ what the template shows

pub(super) struct DecisionRow {
    pub text: String,
    pub note: Option<String>,
}

pub(super) struct CommentRow {
    pub id: String,
    pub meta: String,
    pub anchor: Option<String>,
    /// Sanitized HTML; `None` for a hidden comment the viewer may not read.
    pub body_html: Option<String>,
    /// "Hidden by a maintainer: …" (or the administrator's longer sentence).
    pub hidden_line: Option<String>,
    pub hidden: bool,
}

pub(super) struct ThreadRow {
    pub top: CommentRow,
    pub replies: Vec<CommentRow>,
}

/// Everything the review part of the proposal page needs.
pub(super) struct ReviewBlock {
    /// The default view's review line and its meaning (open proposals only).
    pub line: Option<String>,
    pub means: Option<String>,
    pub earlier_note: Option<String>,
    pub decisions_heading: String,
    pub decisions: Vec<DecisionRow>,
    pub earlier: Vec<DecisionRow>,
    pub comments_count: String,
    pub threads: Vec<ThreadRow>,
    pub more: Option<String>,
    pub can_comment: bool,
    pub can_decide: bool,
    pub can_hide: bool,
    /// The viewer sent part of this proposal: they cannot approve it.
    pub is_submitter: bool,
    /// The latest revision (the form says which revision a decision is for).
    pub latest: u32,
    pub decision_heading: String,
    pub decision_consequence: String,
    pub comment_action: String,
    pub decision_action: String,
}

// ------------------------------------------------------------------------------------------------ building it

fn anchor_text(t: &T, c: &CommentRecord) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = &c.patch_id {
        parts.push(
            t.msg(Key::CommentAnchorPatch)
                .code("patch", short(p))
                .text(),
        );
    }
    if let Some(path) = &c.path {
        parts.push(match (c.line_from, c.line_to) {
            (Some(f), Some(l)) if f == l => t
                .msg(Key::CommentAnchorLine)
                .with("path", path)
                .with("line", f)
                .text(),
            (Some(f), Some(l)) => t
                .msg(Key::CommentAnchorLines)
                .with("path", path)
                .with("from", f)
                .with("to", l)
                .text(),
            _ => t.msg(Key::CommentAnchorPath).with("path", path).text(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// One comment as the page shows it. **The only place the text of a comment is turned into page content.**
fn comment_row(t: &T, c: &CommentRecord, can_see_hidden: bool) -> CommentRow {
    let body_html =
        crate::reviews::visible_body(c, can_see_hidden).map(crate::sanitize::render_markdown);
    let hidden_line = c.hidden.as_ref().map(|h| {
        if can_see_hidden {
            t.msg(Key::CommentHiddenAdmin)
                .with("who", &h.by)
                .with("when", timefmt::minute(h.at))
                .with("reason", &h.reason)
                .text()
        } else {
            t.msg(Key::CommentHidden).with("reason", &h.reason).text()
        }
    });
    CommentRow {
        id: c.comment_id.clone(),
        meta: t
            .msg(Key::CommentMeta)
            .with("who", &c.author)
            .with("when", timefmt::minute(c.created_at))
            .with("revision", c.revision_seq)
            .text(),
        anchor: anchor_text(t, c),
        body_html,
        hidden_line,
        hidden: c.hidden.is_some(),
    }
}

fn decision_row(t: &T, d: &planeter_transport::DecisionView) -> DecisionRow {
    let key = match d.verdict {
        Verdict::Approve => Key::ReviewDecisionApprove,
        Verdict::RequestChanges => Key::ReviewDecisionRequest,
    };
    DecisionRow {
        text: t
            .msg(key)
            .with("who", &d.reviewer)
            .with("revision", d.revision)
            .text(),
        note: d
            .note
            .as_ref()
            .map(|n| t.msg(Key::ReviewDecisionNote).with("note", n).text()),
    }
}

fn thread_rows(t: &T, threads: &[Thread], can_see_hidden: bool) -> Vec<ThreadRow> {
    threads
        .iter()
        .map(|th| ThreadRow {
            top: comment_row(t, &th.comment, can_see_hidden),
            replies: th
                .replies
                .iter()
                .map(|r| comment_row(t, r, can_see_hidden))
                .collect(),
        })
        .collect()
}

/// The review part of the detail page, or `None` when review is not set up on this server (or the records could not be read:
/// a page never fails for want of its comments).
pub(super) async fn review_block(
    state: &AppState,
    c: &Ctx,
    record: &RepositoryRecord,
    owner: &str,
    name: &str,
    d: &Detail,
    after: Option<&str>,
) -> Option<ReviewBlock> {
    let service: Arc<ReviewService> = state.transport.as_ref()?.reviews.clone()?;
    let (change, revisions) = (
        d.proposal.clone(),
        d.revisions
            .iter()
            .map(|r| r.record.clone())
            .collect::<Vec<_>>(),
    );
    let after_owned = after.map(str::to_owned);
    let parts = {
        let svc = Arc::clone(&service);
        match offload(move || {
            svc.detail_parts(
                &change,
                &revisions,
                THREADS_LIMIT_DEFAULT,
                after_owned.as_deref(),
            )
        })
        .await
        {
            Ok(Ok(p)) => p,
            _ => return None,
        }
    };
    let (review, page) = parts;
    let (read, principal, rec) = (Arc::clone(&state.read), c.principal.clone(), record.clone());
    let (can_comment, can_decide, can_hide) = offload(move || {
        (
            read.allowed(&principal, &rec, Action::Comment)
                .unwrap_or(false),
            read.allowed(&principal, &rec, Action::Review)
                .unwrap_or(false),
            read.allowed(&principal, &rec, Action::RepoAdmin)
                .unwrap_or(false),
        )
    })
    .await
    .unwrap_or((false, false, false));
    let t = c.t;
    let p = &d.proposal;
    let latest = review.latest;
    let open = p.state == ChangeState::Open;
    let st = &review.standing;
    let (line, means) = if open {
        let (line, key) = match st.state {
            ReviewState::AwaitingReview => {
                (t.get(Key::ReviewLineAwaiting), Key::ReviewMeansAwaiting)
            }
            ReviewState::Approved => (
                t.msg(Key::ReviewLineApproved)
                    .n(st.approved_by.len() as u64)
                    .text(),
                Key::ReviewMeansApproved,
            ),
            ReviewState::ChangesRequested => (
                t.msg(Key::ReviewLineChanges)
                    .n(st.changes_requested_by.len() as u64)
                    .text(),
                Key::ReviewMeansChanges,
            ),
        };
        (Some(line), Some(t.msg(key).with("revision", latest).text()))
    } else {
        (None, None)
    };
    let earlier_note = (open
        && st.state == ReviewState::AwaitingReview
        && !st.approved_on.is_empty())
    .then(|| {
        t.msg(Key::ReviewEarlierNote)
            .with("revision", latest)
            .text()
    });
    let viewer = c.principal.user().map(|u| u.as_str().to_owned());
    let is_submitter = viewer
        .as_ref()
        .is_some_and(|v| d.revisions.iter().any(|r| &r.record.submitter == v));
    Some(ReviewBlock {
        line,
        means,
        earlier_note,
        decisions_heading: t
            .msg(Key::ReviewDecisionsHeading)
            .with("revision", latest)
            .text(),
        decisions: review.current.iter().map(|x| decision_row(&t, x)).collect(),
        earlier: review.earlier.iter().map(|x| decision_row(&t, x)).collect(),
        comments_count: t.msg(Key::CommentsCount).n(page.total).text(),
        threads: thread_rows(&t, &page.threads, can_hide),
        more: page.next.as_ref().map(|cursor| {
            format!(
                "/{}/{}/proposals/{}?after={}",
                owner,
                name,
                pct(&p.change_id),
                pct(cursor)
            )
        }),
        can_comment: can_comment
            && planeter_core::review::check_comment_window(
                p.state,
                p.retain_until,
                (state.now_unix)(),
            )
            .is_ok(),
        can_decide: can_decide && open,
        can_hide,
        is_submitter,
        latest,
        decision_heading: t
            .msg(Key::DecisionFormHeading)
            .with("revision", latest)
            .text(),
        decision_consequence: t
            .msg(Key::DecisionFormConsequence)
            .with("revision", latest)
            .text(),
        comment_action: format!(
            "/{}/{}/proposals/{}/comments",
            owner,
            name,
            pct(&p.change_id)
        ),
        decision_action: format!(
            "/{}/{}/proposals/{}/decisions",
            owner,
            name,
            pct(&p.change_id)
        ),
    })
}

/// The review word of each open proposal of a list: the `Key` of its sentence. A proposal that is not open shows none.
pub(super) async fn review_words(
    state: &AppState,
    items: &[(String, u32)],
) -> BTreeMap<String, Key> {
    let Some(service) = state.transport.as_ref().and_then(|rt| rt.reviews.clone()) else {
        return BTreeMap::new();
    };
    let items = items.to_vec();
    let words = offload(move || service.review_words(&items))
        .await
        .unwrap_or_default();
    words
        .into_iter()
        .map(|(id, st)| {
            (
                id,
                match st {
                    ReviewState::AwaitingReview => Key::ReviewLineAwaiting,
                    ReviewState::Approved => Key::ReviewWordApproved,
                    ReviewState::ChangesRequested => Key::ReviewWordChanges,
                },
            )
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------ the forms

#[derive(serde::Deserialize)]
pub(super) struct CommentForm {
    csrf: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    patch: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    line_from: String,
    #[serde(default)]
    line_to: String,
    #[serde(default)]
    parent: String,
}

#[derive(serde::Deserialize)]
pub(super) struct DecisionForm {
    csrf: String,
    #[serde(default)]
    decision: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    revision: String,
}

#[derive(serde::Deserialize)]
pub(super) struct HideForm {
    csrf: String,
    #[serde(default)]
    reason: String,
}

#[derive(serde::Deserialize)]
pub(super) struct CsrfOnly {
    csrf: String,
}

fn back_to(owner: &str, name: &str, id: &str) -> String {
    format!("/{owner}/{name}/proposals/{}", pct(id))
}

/// A refusal of a form, as one page: the heading of the form, the sentences, a way back to the proposal.
fn refusal_page(
    state: &AppState,
    c: &Ctx,
    status: StatusCode,
    heading: Key,
    body: String,
    back: &str,
) -> Response {
    let tpl = super::MessageTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        title: c.t.msg(heading).product().text(),
        heading: c.t.get(heading),
        body,
        link_label: c.t.get(Key::ProposalBack),
        link_href: back.to_owned(),
    };
    render(state, status, &tpl, c)
}

fn both(t: &T, message: Key, hint: Key) -> String {
    format!("{} {}", t.get(message), t.get(hint))
}

fn comment_refusal_key(r: CommentRefusal) -> Key {
    match r {
        CommentRefusal::BodyEmpty => Key::CommentRefusalBodyEmpty,
        CommentRefusal::BodyTooLong => Key::CommentRefusalBodyTooLong,
        CommentRefusal::PatchIdInvalid | CommentRefusal::PatchNotInSet => Key::CommentRefusalPatch,
        CommentRefusal::PathEmpty | CommentRefusal::PathTooLong | CommentRefusal::PathInvalid => {
            Key::CommentRefusalPath
        }
        CommentRefusal::LinesNeedAPath => Key::CommentRefusalLinesNeedPath,
        CommentRefusal::LinesIncomplete => Key::CommentRefusalLinesIncomplete,
        CommentRefusal::LineZero => Key::CommentRefusalLineZero,
        CommentRefusal::LineOrder => Key::CommentRefusalLineOrder,
        CommentRefusal::ParentUnknown
        | CommentRefusal::ParentOtherProposal
        | CommentRefusal::ParentNotTopLevel => Key::CommentRefusalParent,
    }
}

/// A review error as a page. `heading` is the form's.
fn error_response_page(
    state: &AppState,
    c: &Ctx,
    at: &str,
    back: &str,
    heading: Key,
    e: &ReviewError,
) -> Response {
    let t = &c.t;
    let unprocessable = StatusCode::UNPROCESSABLE_ENTITY;
    match e {
        ReviewError::NotFound | ReviewError::BadCursor => {
            error_page(state, c, at, Failure::Read(ReadError::NotFound))
        }
        ReviewError::Comment(r) => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            t.get(comment_refusal_key(*r)),
            back,
        ),
        ReviewError::CommentsClosed(cl) => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            format!(
                "{} {}",
                t.msg(Key::ErrCommentsClosedMessage)
                    .with("date", timefmt::date(cl.until))
                    .text(),
                t.get(Key::ErrCommentsClosedHint)
            ),
            back,
        ),
        ReviewError::Decision(DecisionRefusal::NotOpen(_)) => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            both(
                t,
                Key::ErrDecisionsClosedMessage,
                Key::ErrDecisionsClosedHint,
            ),
            back,
        ),
        ReviewError::Decision(DecisionRefusal::RevisionNotCurrent { current }) => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            format!(
                "{} {}",
                t.msg(Key::ErrRevisionNotCurrentMessage)
                    .with("current", current)
                    .text(),
                t.get(Key::ErrRevisionNotCurrentHint)
            ),
            back,
        ),
        ReviewError::Decision(DecisionRefusal::SelfApproval(_)) => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            both(t, Key::ErrSelfApprovalMessage, Key::ErrSelfApprovalHint),
            back,
        ),
        ReviewError::Decision(DecisionRefusal::NoteTooLong) => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            t.get(Key::DecisionRefusalNoteTooLong),
            back,
        ),
        ReviewError::HideReason(HideRefusal::ReasonRequired | HideRefusal::ReasonTooLong) => {
            refusal_page(
                state,
                c,
                unprocessable,
                heading,
                t.get(Key::HideRefusalReason),
                back,
            )
        }
        ReviewError::CommentState { .. } => refusal_page(
            state,
            c,
            unprocessable,
            heading,
            both(t, Key::ErrCommentStateMessage, Key::ErrCommentStateHint),
            back,
        ),
        ReviewError::RateLimited { what, .. } => {
            let (m, h) = match what {
                Limited::Comments => (Key::ErrRateCommentsMessage, Key::ErrRateCommentsHint),
                Limited::Decisions => (Key::ErrRateDecisionsMessage, Key::ErrRateDecisionsHint),
            };
            refusal_page(
                state,
                c,
                StatusCode::TOO_MANY_REQUESTS,
                heading,
                both(t, m, h),
                back,
            )
        }
        ReviewError::Unavailable(_) => error_page(state, c, at, Failure::Internal),
    }
}

/// What every form handler does first, in the same order: the read gate (a stranger is told nothing), the service, the
/// form (incomplete → the form page), the CSRF token, and whether the viewer may do this at all.
struct Gate {
    c: Ctx,
    record: RepositoryRecord,
    service: Arc<ReviewService>,
    user: String,
    back: String,
    at: String,
}

/// What a form handler needs the viewer to be allowed to do, and the sentences to say when they are not.
struct Need {
    action: Action,
    forbidden: Key,
    forbidden_hint: Key,
    heading: Key,
}

async fn gate<F>(
    state: &AppState,
    headers: &HeaderMap,
    (owner, name, id): (&str, &str, &str),
    form: Result<Form<F>, FormRejection>,
    csrf_of: fn(&F) -> &str,
    need: Need,
) -> Result<(Gate, F), Box<Response>> {
    let Need {
        action,
        forbidden,
        forbidden_hint,
        heading,
    } = need;
    let c = ctx(headers, state).await;
    let at = format!("/{owner}/{name}/proposals/{}", short(id));
    let back = back_to(owner, name, id);
    let record = match readable(state, &c.principal, owner, name).await {
        Ok(r) => r,
        Err(e) => return Err(Box::new(error_page(state, &c, &at, e))),
    };
    let Some(service) = state.transport.as_ref().and_then(|rt| rt.reviews.clone()) else {
        return Err(Box::new(super::proposal_pages::off_page(
            state, &c, owner, name,
        )));
    };
    let Ok(Form(form)) = form else {
        return Err(Box::new(super::notice(
            state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            super::Notice::FormIncomplete,
            super::Next::Home,
            vec![],
        )));
    };
    let cookie_csrf = get_cookie(headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, csrf_of(&form)) {
        return Err(Box::new(super::notice(
            state,
            &c,
            StatusCode::FORBIDDEN,
            super::Notice::CsrfFailed,
            super::Next::Home,
            vec![],
        )));
    }
    let (read, principal, rec) = (Arc::clone(&state.read), c.principal.clone(), record.clone());
    let allowed = offload(move || read.allowed(&principal, &rec, action).unwrap_or(false))
        .await
        .unwrap_or(false);
    let user = c.principal.user().map(|u| u.as_str().to_owned());
    let (true, Some(user)) = (allowed, user) else {
        return Err(Box::new(refusal_page(
            state,
            &c,
            StatusCode::FORBIDDEN,
            heading,
            both(&c.t, forbidden, forbidden_hint),
            &back,
        )));
    };
    Ok((
        Gate {
            c,
            record,
            service,
            user,
            back,
            at,
        },
        form,
    ))
}

fn number(s: &str) -> Result<Option<u32>, ()> {
    let s = s.trim();
    if s.is_empty() {
        Ok(None)
    } else {
        s.parse::<u32>().map(Some).map_err(|_| ())
    }
}

pub(super) async fn comment_submit(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    form: Result<Form<CommentForm>, FormRejection>,
) -> Response {
    let (g, f) = match gate(
        &state,
        &headers,
        (&owner, &name, &id),
        form,
        |f| &f.csrf,
        Need {
            action: Action::Comment,
            forbidden: Key::ErrForbiddenCommentMessage,
            forbidden_hint: Key::ErrForbiddenCommentHint,
            heading: Key::CommentRefusedHeading,
        },
    )
    .await
    {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let refuse = |r: CommentRefusal| {
        refusal_page(
            &state,
            &g.c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Key::CommentRefusedHeading,
            g.c.t.get(comment_refusal_key(r)),
            &g.back,
        )
    };
    let (Ok(line_from), Ok(line_to)) = (number(&f.line_from), number(&f.line_to)) else {
        return refuse(CommentRefusal::LineZero);
    };
    // The patch is named by the first characters of its id; the page resolves it to the one patch of the proposal that starts
    // so, and refuses when none or more than one does.
    let patch_id = match f.patch.trim() {
        "" => None,
        prefix => {
            let (svc, rec, pid, prefix) = (
                Arc::clone(&g.service),
                g.record.clone(),
                id.clone(),
                prefix.to_owned(),
            );
            match offload(move || svc.patch_by_prefix(&rec, &pid, &prefix)).await {
                Ok(Ok(Some(p))) => Some(p),
                Ok(Ok(None)) => return refuse(CommentRefusal::PatchNotInSet),
                Ok(Err(e)) => {
                    return error_response_page(
                        &state,
                        &g.c,
                        &g.at,
                        &g.back,
                        Key::CommentRefusedHeading,
                        &e,
                    );
                }
                Err(_) => return error_page(&state, &g.c, &g.at, Failure::Internal),
            }
        }
    };
    let new = NewComment {
        body: f.body,
        patch_id,
        path: Some(f.path.trim().to_owned()).filter(|p| !p.is_empty()),
        line_from,
        line_to,
        parent: Some(f.parent.trim().to_owned()).filter(|p| !p.is_empty()),
    };
    let (svc, rec, pid, user) = (
        Arc::clone(&g.service),
        g.record.clone(),
        id.clone(),
        g.user.clone(),
    );
    match offload(move || svc.add_comment(&rec, &pid, &user, &new)).await {
        Ok(Ok(comment)) => redirect(
            &state,
            &format!("{}#comment-{}", g.back, comment.comment_id),
            vec![],
        ),
        Ok(Err(e)) => {
            error_response_page(&state, &g.c, &g.at, &g.back, Key::CommentRefusedHeading, &e)
        }
        Err(_) => error_page(&state, &g.c, &g.at, Failure::Internal),
    }
}

pub(super) async fn decision_submit(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    form: Result<Form<DecisionForm>, FormRejection>,
) -> Response {
    let (g, f) = match gate(
        &state,
        &headers,
        (&owner, &name, &id),
        form,
        |f| &f.csrf,
        Need {
            action: Action::Review,
            forbidden: Key::ErrForbiddenDecisionMessage,
            forbidden_hint: Key::ErrForbiddenDecisionHint,
            heading: Key::DecisionRefusedHeading,
        },
    )
    .await
    {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let Some(verdict) = Verdict::parse(f.decision.trim()) else {
        return super::notice(
            &state,
            &g.c,
            StatusCode::UNPROCESSABLE_ENTITY,
            super::Notice::FormIncomplete,
            super::Next::Home,
            vec![],
        );
    };
    let Ok(revision) = number(&f.revision) else {
        return super::notice(
            &state,
            &g.c,
            StatusCode::UNPROCESSABLE_ENTITY,
            super::Notice::FormIncomplete,
            super::Next::Home,
            vec![],
        );
    };
    let (svc, rec, pid, user) = (
        Arc::clone(&g.service),
        g.record.clone(),
        id.clone(),
        g.user.clone(),
    );
    match offload(move || svc.decide(&rec, &pid, &user, verdict, revision, Some(&f.note))).await {
        Ok(Ok(_)) => redirect(&state, &g.back, vec![]),
        Ok(Err(e)) => error_response_page(
            &state,
            &g.c,
            &g.at,
            &g.back,
            Key::DecisionRefusedHeading,
            &e,
        ),
        Err(_) => error_page(&state, &g.c, &g.at, Failure::Internal),
    }
}

pub(super) async fn hide_submit(
    State(state): State<AppState>,
    Path((owner, name, id, cid)): Path<(String, String, String, String)>,
    headers: HeaderMap,
    form: Result<Form<HideForm>, FormRejection>,
) -> Response {
    let (g, f) = match gate(
        &state,
        &headers,
        (&owner, &name, &id),
        form,
        |f| &f.csrf,
        Need {
            action: Action::RepoAdmin,
            forbidden: Key::ErrForbiddenHideMessage,
            forbidden_hint: Key::ErrForbiddenHideHint,
            heading: Key::HideRefusedHeading,
        },
    )
    .await
    {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let (svc, rec, pid, user, c2) = (
        Arc::clone(&g.service),
        g.record.clone(),
        id.clone(),
        g.user.clone(),
        cid.clone(),
    );
    match offload(move || svc.hide(&rec, &pid, &c2, &user, &f.reason)).await {
        Ok(Ok(_)) => redirect(&state, &format!("{}#comment-{cid}", g.back), vec![]),
        Ok(Err(e)) => {
            error_response_page(&state, &g.c, &g.at, &g.back, Key::HideRefusedHeading, &e)
        }
        Err(_) => error_page(&state, &g.c, &g.at, Failure::Internal),
    }
}

pub(super) async fn unhide_submit(
    State(state): State<AppState>,
    Path((owner, name, id, cid)): Path<(String, String, String, String)>,
    headers: HeaderMap,
    form: Result<Form<CsrfOnly>, FormRejection>,
) -> Response {
    let (g, _) = match gate(
        &state,
        &headers,
        (&owner, &name, &id),
        form,
        |f| &f.csrf,
        Need {
            action: Action::RepoAdmin,
            forbidden: Key::ErrForbiddenHideMessage,
            forbidden_hint: Key::ErrForbiddenHideHint,
            heading: Key::HideRefusedHeading,
        },
    )
    .await
    {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let (svc, rec, pid, user, c2) = (
        Arc::clone(&g.service),
        g.record.clone(),
        id.clone(),
        g.user.clone(),
        cid.clone(),
    );
    match offload(move || svc.unhide(&rec, &pid, &c2, &user)).await {
        Ok(Ok(_)) => redirect(&state, &format!("{}#comment-{cid}", g.back), vec![]),
        Ok(Err(e)) => {
            error_response_page(&state, &g.c, &g.at, &g.back, Key::HideRefusedHeading, &e)
        }
        Err(_) => error_page(&state, &g.c, &g.at, Failure::Internal),
    }
}
