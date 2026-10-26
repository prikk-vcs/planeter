//! The pages of proposals (RFC 005 part 5, T4): the list, one proposal, and the form that closes one.
//!
//! The pages read through the same service as the JSON side (`crate::proposals`): the one read gate first, then the service.
//! **No prikk call is made per item** — the landing observation is one `sync pending` per repository per state fingerprint.
//!
//! The default view of one proposal (D-11) shows the title, who proposed it, the state, one sentence of what it is, a
//! one-line summary and the actions the viewer may take. Everything else sits under one `<details>`. The review-state and
//! checks lines of D-11 are not shown: they would state things that do not exist yet.

use std::sync::Arc;

use askama::Template;
use axum::extract::rejection::FormRejection;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;

use planeter_core::Action;
use planeter_store::{ChangeState, RepositoryRecord};
use planeter_transport::{
    DETAIL_PATCH_LIMIT, Detail, LIST_LIMIT_DEFAULT, ProposalError, ProposalService, Retention,
    StateFilter, Summary,
};

use super::{
    Ctx, ExplainRow, Who, assurance, ctx, error_page, explanation, pct, redirect, render, short,
};
use crate::api::{AppState, Failure};
use crate::catalog::{Key, T};
use crate::cookies::{CSRF_COOKIE, get_cookie};
use crate::gate::offload;
use crate::timefmt;
use crate::transport::readable;
use planeter_auth::tokens_match;

// ------------------------------------------------------------------------------------------------ what a page needs

/// The proposals service, when this server has one.
fn service_of(state: &AppState) -> Option<Arc<ProposalService>> {
    state.transport.as_ref().and_then(|rt| rt.proposals.clone())
}

/// Whether the repository navigation shows *Proposals* for this viewer, and how many proposals are open: the link shows when
/// the repository has at least one proposal or the viewer may propose. Called only after the page's own read succeeded, so
/// the repository is readable. A failure to answer shows no link (a page is never refused for the sake of its navigation).
pub(super) async fn nav_for(
    state: &AppState,
    c: &Ctx,
    owner: &str,
    name: &str,
) -> (bool, Option<u64>) {
    let Some(service) = service_of(state) else {
        return (false, None);
    };
    let Ok(record) = readable(state, &c.principal, owner, name).await else {
        return (false, None);
    };
    let (read, principal) = (Arc::clone(&state.read), c.principal.clone());
    let answer = offload(move || {
        let total = service.count(&record.repo_id).ok()?;
        let open = service.count_open(&record.repo_id).ok()?;
        let may_propose = total > 0 || read.allowed(&principal, &record, Action::Propose).ok()?;
        Some((may_propose, open))
    })
    .await;
    match answer {
        Ok(Some((show, open))) => (show, (show && open > 0).then_some(open)),
        _ => (false, None),
    }
}

/// What a page answers when this server keeps no proposals (no artifact store is set up): a plain sentence, with a way back
/// to the repository. It is shown only after the read gate, so it says nothing about a repository the viewer cannot read.
fn off_page(state: &AppState, c: &Ctx, owner: &str, name: &str) -> Response {
    let tpl = super::MessageTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        title: c
            .t
            .msg(Key::TitleProposals)
            .with("repo", format!("{owner}/{name}"))
            .product()
            .text(),
        heading: c.t.get(Key::ProposalsHeading),
        body: c.t.get(Key::ErrProposeOffMessage),
        link_label: c.t.get(Key::NavCode),
        link_href: format!("/{owner}/{name}"),
    };
    render(state, StatusCode::SERVICE_UNAVAILABLE, &tpl, c)
}

// ------------------------------------------------------------------------------------------------ the list

struct Row {
    id: String,
    title: String,
    by: String,
    state_label: Key,
    summary: String,
    when: String,
}

fn state_label(s: ChangeState) -> Key {
    match s {
        ChangeState::Open => Key::ProposalStateOpen,
        ChangeState::Landed => Key::ProposalStateLanded,
        ChangeState::Closed => Key::ProposalStateClosed,
    }
}

fn what_sentence(s: ChangeState) -> Key {
    match s {
        ChangeState::Open => Key::ProposalWhatOpen,
        ChangeState::Landed => Key::ProposalWhatLanded,
        ChangeState::Closed => Key::ProposalWhatClosed,
    }
}

/// "3 patches in 2 revisions".
fn summary_line(t: &T, patches: usize, revisions: u32) -> String {
    let revs = t
        .msg(Key::ProposalRevisionsCount)
        .n(u64::from(revisions))
        .text();
    t.msg(Key::ProposalSummary)
        .n(patches as u64)
        .with("revisions", revs)
        .text()
}

fn row(t: &T, s: &Summary) -> Row {
    Row {
        id: s.proposal.change_id.clone(),
        title: s.proposal.title.clone(),
        by: s.proposal.opened_by.clone(),
        state_label: state_label(s.proposal.state),
        summary: summary_line(t, s.patches, s.revisions),
        when: timefmt::minute(s.proposal.updated_at),
    }
}

struct FilterLink {
    label: Key,
    href: String,
    current: bool,
}

#[derive(Template)]
#[template(path = "proposals.html")]
struct ListTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    owner: String,
    name: String,
    repo: String,
    proposals_nav: bool,
    filters: Vec<FilterLink>,
    rows: Vec<Row>,
    /// The address of the next page of the same list.
    more: Option<String>,
}

#[derive(serde::Deserialize)]
pub(super) struct ListQuery {
    state: Option<String>,
    after: Option<String>,
}

fn filter_name(f: StateFilter) -> &'static str {
    match f {
        StateFilter::Open => "open",
        StateFilter::Landed => "landed",
        StateFilter::Closed => "closed",
        StateFilter::All => "all",
    }
}

pub(super) async fn list_page(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let at = format!("/{owner}/{name}/proposals");
    let record = match readable(&state, &c.principal, &owner, &name).await {
        Ok(r) => r,
        Err(e) => return error_page(&state, &c, &at, e),
    };
    let Some(service) = service_of(&state) else {
        return off_page(&state, &c, &owner, &name);
    };
    let filter = q
        .state
        .as_deref()
        .and_then(StateFilter::parse)
        .unwrap_or(StateFilter::Open);
    let after = q.after;
    let page = {
        let (service, record) = (Arc::clone(&service), record.clone());
        state
            .gate
            .run(move || service.list(&record, filter, LIST_LIMIT_DEFAULT, after.as_deref()))
            .await
    };
    let page = match page {
        Ok(Ok(p)) => p,
        // A cursor this service did not give is the same as no cursor: the first page.
        Ok(Err(ProposalError::BadCursor)) => {
            let (service, record) = (Arc::clone(&service), record.clone());
            match state
                .gate
                .run(move || service.list(&record, filter, LIST_LIMIT_DEFAULT, None))
                .await
            {
                Ok(Ok(p)) => p,
                _ => return error_page(&state, &c, &at, Failure::Internal),
            }
        }
        Ok(Err(_)) => return error_page(&state, &c, &at, Failure::Internal),
        Err(_) => return error_page(&state, &c, &at, Failure::Busy),
    };
    let (proposals_nav, _) = nav_for(&state, &c, &owner, &name).await;
    let filters = [
        (StateFilter::Open, Key::ProposalStateOpen),
        (StateFilter::Landed, Key::ProposalStateLanded),
        (StateFilter::Closed, Key::ProposalStateClosed),
        (StateFilter::All, Key::ProposalsFilterAll),
    ]
    .into_iter()
    .map(|(f, label)| FilterLink {
        label,
        href: format!("{at}?state={}", filter_name(f)),
        current: f == filter,
    })
    .collect();
    let more = page
        .next
        .as_deref()
        .map(|cursor| format!("{at}?state={}&after={}", filter_name(filter), pct(cursor)));
    let rows = page.items.iter().map(|s| row(&c.t, s)).collect();
    render(
        &state,
        StatusCode::OK,
        &ListTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner,
            name,
            proposals_nav,
            filters,
            rows,
            more,
        },
        &c,
    )
}

// ------------------------------------------------------------------------------------------------ one proposal

struct RevRow {
    seq: u32,
    by: String,
    sent: String,
    added: usize,
    built_on: Option<String>,
    declared: Key,
    claims: Vec<(String, String)>,
    key_ids: Vec<String>,
    digest: String,
    /// The files are still in the store.
    available: bool,
    /// The download address, when the files are still kept.
    download: Option<String>,
}

struct PatchRow {
    short_id: String,
    /// The badge, when the patch is held or sealed; `None` = not listed, and nothing is claimed.
    badge: Option<(&'static str, Key)>,
}

#[derive(Template)]
#[template(path = "proposal.html")]
struct DetailTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    owner: String,
    name: String,
    repo: String,
    proposals_nav: bool,
    title: String,
    by: String,
    when: String,
    state_label: Key,
    what: Key,
    summary: String,
    closed_reason: Option<String>,
    /// The one line of the default view: "Closed: <reason>", the reason cut at 200 characters (the full text is under Details).
    closed_line: Option<String>,
    /// The received refs that hold a patch of this open proposal (Details only; information, never a state).
    sightings: Vec<String>,
    description_html: Option<String>,
    revisions: Vec<RevRow>,
    patches: Vec<PatchRow>,
    patches_note: Option<String>,
    pending_note: Option<String>,
    retention_note: Option<String>,
    explain: Vec<ExplainRow>,
    /// The latest revision's download, when the viewer may download and the files are kept.
    latest_download: Option<(u32, String)>,
    may_close: bool,
    /// Where the close form posts.
    close_action: String,
}

fn artifact_href(owner: &str, name: &str, id: &str, seq: u32) -> String {
    format!(
        "/api/v1/repos/{owner}/{name}/proposals/{}/revisions/{seq}/artifact",
        pct(id)
    )
}

/// How many characters of a closing reason the default view shows.
const REASON_SHOWN: usize = 200;

/// "Closed: <reason>", the reason cut at [`REASON_SHOWN`] characters with an ellipsis (the template escapes it).
fn closed_line(t: &T, reason: &str) -> String {
    let shown: String = if reason.chars().count() > REASON_SHOWN {
        reason.chars().take(REASON_SHOWN).collect::<String>() + "…"
    } else {
        reason.to_owned()
    };
    t.msg(Key::ProposalClosedLine).with("reason", shown).text()
}

fn retention_note(t: &T, r: Retention) -> Option<String> {
    let (key, until) = match r {
        Retention::Open => return None,
        Retention::Kept { until } => (Key::ProposalRetentionKept, until),
        Retention::Partly { until } => (Key::ProposalRetentionPartly, until),
        Retention::Removed { until } => (Key::ProposalRetentionRemoved, until),
    };
    Some(t.msg(key).with("date", timefmt::date(until)).text())
}

/// Who may download (review or merge-seal) and who may close (the opener, or whoever `authorize(Close)` allows).
async fn permissions(
    state: &AppState,
    c: &Ctx,
    record: &RepositoryRecord,
    opener: &str,
) -> (bool, bool) {
    let (read, principal, record, opener) = (
        Arc::clone(&state.read),
        c.principal.clone(),
        record.clone(),
        opener.to_owned(),
    );
    let answer = offload(move || {
        let download = read.allowed(&principal, &record, Action::Review)?
            || read.allowed(&principal, &record, Action::MergeSeal)?;
        let close = may_close(&read, &principal, &record, &opener)?;
        Ok::<(bool, bool), planeter_core::ReadError>((download, close))
    })
    .await;
    match answer {
        Ok(Ok(both)) => both,
        // Not knowing is not allowing.
        _ => (false, false),
    }
}

fn may_close(
    read: &planeter_core::ReadService,
    principal: &planeter_core::Principal,
    record: &RepositoryRecord,
    opener: &str,
) -> Result<bool, planeter_core::ReadError> {
    if read.allowed(principal, record, Action::Close)? {
        return Ok(true);
    }
    let is_opener = principal.user().is_some_and(|u| u.as_str() == opener);
    Ok(is_opener && read.credential_permits(principal, record, Action::Close))
}

fn detail_tpl(
    c: &Ctx,
    d: &Detail,
    owner: &str,
    name: &str,
    proposals_nav: bool,
    may_download: bool,
    may_close: bool,
) -> DetailTpl {
    let p = &d.proposal;
    let t = c.t;
    let revisions: Vec<RevRow> = d
        .revisions
        .iter()
        .map(|r| RevRow {
            seq: r.record.seq,
            by: r.record.submitter.clone(),
            sent: timefmt::minute(r.record.created_at),
            added: r.record.added_patch_ids.len(),
            built_on: r.record.declared_base.as_deref().map(short),
            declared: if r.record.declared_patch_ids.is_some() {
                Key::ProposalRevDeclared
            } else {
                Key::ProposalRevUndeclared
            },
            claims: r
                .record
                .claim_outcomes
                .iter()
                .map(|x| (x.claim_id.clone(), x.outcome.clone()))
                .collect(),
            key_ids: r.record.author_key_ids.clone(),
            digest: r.record.artifact_digest.clone(),
            available: r.artifact_available,
            download: (may_download && r.artifact_available)
                .then(|| artifact_href(owner, name, &p.change_id, r.record.seq)),
        })
        .collect();
    let latest_download = d
        .revisions
        .iter()
        .max_by_key(|r| r.record.seq)
        .filter(|r| may_download && r.artifact_available)
        .map(|r| {
            (
                r.record.seq,
                artifact_href(owner, name, &p.change_id, r.record.seq),
            )
        });
    let patches: Vec<PatchRow> = d
        .patches
        .iter()
        .map(|x| PatchRow {
            short_id: short(&x.patch_id),
            badge: x.assurance.map(assurance),
        })
        .collect();
    // One explanation per distinct badge on the page, then the sentence for a patch that is not listed.
    let mut explain: Vec<ExplainRow> = Vec::new();
    for row in &patches {
        if let Some((_, label)) = row.badge
            && !explain.iter().any(|r| r.label == Some(label))
        {
            explain.push(ExplainRow {
                label: Some(label),
                text: explanation(label),
            });
        }
    }
    if patches.iter().any(|r| r.badge.is_none()) {
        explain.push(ExplainRow {
            label: Some(Key::ProposalPatchNotListed),
            text: Key::ProposalPatchNotListedExplain,
        });
    }
    DetailTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t,
        owner: owner.to_owned(),
        name: name.to_owned(),
        repo: format!("{owner}/{name}"),
        proposals_nav,
        title: p.title.clone(),
        by: p.opened_by.clone(),
        when: timefmt::minute(p.updated_at),
        state_label: state_label(p.state),
        what: what_sentence(p.state),
        summary: summary_line(&t, d.patch_total, d.revisions.len() as u32),
        closed_reason: p.closed_reason.clone(),
        closed_line: p.closed_reason.as_deref().map(|r| closed_line(&t, r)),
        sightings: d.sightings.clone(),
        description_html: (!p.description.trim().is_empty())
            .then(|| crate::sanitize::render_markdown(&p.description)),
        revisions,
        patches,
        patches_note: d.truncated.then(|| {
            t.msg(Key::ProposalPatchesTruncated)
                .with("limit", DETAIL_PATCH_LIMIT)
                .text()
        }),
        pending_note: (p.state == ChangeState::Open && d.still_pending > 0).then(|| {
            t.msg(Key::ProposalPatchesPending)
                .n(d.still_pending as u64)
                .text()
        }),
        retention_note: retention_note(&t, d.retention),
        explain,
        latest_download,
        may_close: may_close && p.state == ChangeState::Open,
        close_action: format!("/{owner}/{name}/proposals/{}/close", pct(&p.change_id)),
    }
}

pub(super) async fn detail_page(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let at = format!("/{owner}/{name}/proposals/{}", short(&id));
    let record = match readable(&state, &c.principal, &owner, &name).await {
        Ok(r) => r,
        Err(e) => return error_page(&state, &c, &at, e),
    };
    let Some(service) = service_of(&state) else {
        return off_page(&state, &c, &owner, &name);
    };
    let found = {
        let (service, record, id) = (Arc::clone(&service), record.clone(), id.clone());
        state.gate.run(move || service.detail(&record, &id)).await
    };
    let d = match found {
        Ok(Ok(d)) => d,
        Ok(Err(ProposalError::NotFound)) => {
            return error_page(
                &state,
                &c,
                &at,
                Failure::Read(planeter_core::ReadError::NotFound),
            );
        }
        Ok(Err(_)) => return error_page(&state, &c, &at, Failure::Internal),
        Err(_) => return error_page(&state, &c, &at, Failure::Busy),
    };
    let (may_download, may_close) = permissions(&state, &c, &record, &d.proposal.opened_by).await;
    let (proposals_nav, _) = nav_for(&state, &c, &owner, &name).await;
    let tpl = detail_tpl(
        &c,
        &d,
        &owner,
        &name,
        proposals_nav,
        may_download,
        may_close,
    );
    render(&state, StatusCode::OK, &tpl, &c)
}

// ------------------------------------------------------------------------------------------------ closing

#[derive(serde::Deserialize)]
pub(super) struct CloseForm {
    csrf: String,
    #[serde(default)]
    reason: String,
}

/// A refusal of the form, as one page: the heading of the form, the sentence, a way back to the proposal.
fn refusal(state: &AppState, c: &Ctx, status: StatusCode, body: Key, back: &str) -> Response {
    let tpl = super::MessageTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        title: c.t.msg(Key::ProposalCloseHeading).product().text(),
        heading: c.t.get(Key::ProposalCloseHeading),
        body: c.t.get(body),
        link_label: c.t.get(Key::ProposalBack),
        link_href: back.to_owned(),
    };
    render(state, status, &tpl, c)
}

pub(super) async fn close_submit(
    State(state): State<AppState>,
    Path((owner, name, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    form: Result<Form<CloseForm>, FormRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    let back = format!("/{owner}/{name}/proposals/{}", pct(&id));
    let at = format!("/{owner}/{name}/proposals/{}", short(&id));
    // The read gate first: a stranger is told nothing, whatever else is or is not so.
    let record = match readable(&state, &c.principal, &owner, &name).await {
        Ok(r) => r,
        Err(e) => return error_page(&state, &c, &at, e),
    };
    let Some(service) = service_of(&state) else {
        return off_page(&state, &c, &owner, &name);
    };
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
    let cookie_csrf = get_cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, &form.csrf) {
        return super::notice(
            &state,
            &c,
            StatusCode::FORBIDDEN,
            super::Notice::CsrfFailed,
            super::Next::Home,
            vec![],
        );
    }
    let opener = {
        let (service, record, id) = (Arc::clone(&service), record.clone(), id.clone());
        match offload(move || service.opener(&record, &id)).await {
            Ok(Ok(o)) => o,
            Ok(Err(ProposalError::NotFound)) => {
                return error_page(
                    &state,
                    &c,
                    &at,
                    Failure::Read(planeter_core::ReadError::NotFound),
                );
            }
            _ => return error_page(&state, &c, &at, Failure::Internal),
        }
    };
    let (_, allowed) = permissions(&state, &c, &record, &opener).await;
    if !allowed {
        return refusal(
            &state,
            &c,
            StatusCode::FORBIDDEN,
            Key::ErrForbiddenCloseMessage,
            &back,
        );
    }
    let result = {
        let (service, record, id, reason) = (
            Arc::clone(&service),
            record.clone(),
            id.clone(),
            form.reason,
        );
        offload(move || service.close(&record, &id, &reason)).await
    };
    match result {
        Ok(Ok(_)) => redirect(&state, &back, vec![]),
        Ok(Err(ProposalError::ReasonRequired)) => refusal(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Key::ErrReasonRequiredMessage,
            &back,
        ),
        Ok(Err(ProposalError::ReasonTooLong)) => refusal(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Key::ProposalCloseTooLong,
            &back,
        ),
        Ok(Err(ProposalError::AlreadyFinal(_))) => refusal(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Key::ErrProposalFinalMessage,
            &back,
        ),
        Ok(Err(ProposalError::NotFound)) => error_page(
            &state,
            &c,
            &at,
            Failure::Read(planeter_core::ReadError::NotFound),
        ),
        _ => error_page(&state, &c, &at, Failure::Internal),
    }
}
