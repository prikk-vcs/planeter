//! The browse web UI (RFC 003 WEB-01/05/06): server-rendered pages over the same `ReadService` and the
//! same `authorize(_, Read, _)` gate as the API. Sign-in is a password → cookie session (WEB-05:
//! `HttpOnly`/`SameSite=Strict`/`Secure` cookies, CSRF double-submit on every POST). Every page carries
//! the strict CSP from [`crate::security`] — no inline script or style; the one stylesheet is served
//! from `/static/app.css` on the app origin; raw bytes link to the isolated content origin.
//!
//! Honesty on the page (WEB-06): the repository badge is prikk's re-derived `verify` verdict, and a
//! queued patch is labelled *Held here, not signed in* — never *Verified* (the words are the catalog's;
//! the API keeps its stable `Assurance` values).

use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::{FormRejection, QueryRejection};
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};

use crate::routes::{Action, Class, Routes};
use planeter_auth::{
    OidcError, PENDING_TTL_SECS, SESSION_TTL_SECS, SessionId, random_token, tokens_match,
};
use planeter_core::admission::{CONTENT_MAX_BYTES, ReadVerb};
use planeter_core::{Assurance, Principal, RawTarget, ReadError, TreeEntryView, VerifyView};

use crate::api::{
    AppState, Failure, Subject, read_blocking, retry_after_secs, try_both_owners,
    with_security_headers,
};
use crate::authn;
use crate::catalog::{Key, Lang, LanguageVaries, T, lang_cookie, negotiate_from_headers};
use crate::client_ip::{PeerAddr, client_ip};
use crate::cookies::{
    CSRF_COOKIE, OIDC_STATE_COOKIE, SESSION_COOKIE, clear_cookie, get_cookie, set_cookie,
};
use crate::gate::GateError;
use crate::principal::principal_from_request;
use crate::sanitize::render_markdown;

mod creation_pages;
mod proposal_pages;
mod review_pages;

const HISTORY_PAGE: usize = 50;
const CSRF_TTL_SECS: u64 = 30 * 24 * 3600;

/// The browse UI's routes, with their authorization classes (IS-14). This is the only place UI routes are
/// registered.
pub fn routes() -> Routes {
    // Every page that shows repository data reads through `ReadService`: `authorize(_, Read, _)` first.
    let read = Class::Authorized(Action::Read);
    let close = Class::Authorized(Action::Close);
    // Review (RFC 005 part 6): the forms of the proposal page.
    let (comment_class, decision_class, admin_class) = (
        Class::Authorized(Action::Comment),
        Class::Authorized(Action::Review),
        Class::Authorized(Action::RepoAdmin),
    );
    // Creating a repository (RFC 004 part 7): forms with a CSRF token; the handlers decide who may create where
    // (`creation_target`) before anything is read or written, and a stage is its stager's alone.
    let create = Class::public(
        "creates a repository for the signed-in person: an anonymous visitor is shown the sign-in sentence, every refusal for \"not there\" is one sentence, and no repository data is read or served",
    );
    let (new_get, import_post) = (
        get(creation_pages::new_page).post(creation_pages::new_empty_submit),
        post(creation_pages::new_import_submit),
    );
    let stage_both = get(creation_pages::stage_page).post(creation_pages::confirm_submit);
    Routes::new()
        .route(
            "/",
            Class::public("the home page: a form to type an address; it lists nothing"),
            &["GET"],
            get(home),
        )
        .route(
            "/go",
            Class::public("parses an address and redirects to its page, the same for every repository; the page authorizes"),
            &["GET"],
            get(go),
        )
        .route(
            "/static/app.css",
            Class::public("the one stylesheet, embedded in the binary"),
            &["GET"],
            get(stylesheet),
        )
        .route(
            "/login",
            Class::public("sign-in: it authenticates the caller (throttled, CSRF-protected) and serves no repository data"),
            &["GET", "POST"],
            get(login_form).post(login_submit),
        )
        .route(
            "/login/oidc",
            Class::public("begins single sign-on: a redirect to the provider; no repository data"),
            &["GET"],
            get(oidc_begin),
        )
        .route(
            "/login/oidc/callback",
            Class::public("completes single sign-on: it authenticates the caller; no repository data"),
            &["GET"],
            get(oidc_callback),
        )
        .route(
            "/lang",
            Class::public("sets the visitor's own language cookie, then returns to the page they were on (CSRF-protected); it names no repository and serves no repository data"),
            &["POST"],
            post(set_language),
        )
        .route(
            "/logout",
            Class::public("ends the caller's own session (CSRF-protected); no repository data"),
            &["POST"],
            post(logout),
        )
        .route("/new", create, &["GET", "POST"], new_get)
        .route("/new/import", create, &["POST"], import_post)
        .route("/new/import/{stage}", create, &["GET", "POST"], stage_both)
        .route("/{owner}/{name}", read, &["GET"], get(repo_home))
        .route("/{owner}/{name}/tree/{*path}", read, &["GET"], get(tree_page))
        .route("/{owner}/{name}/blob/{*path}", read, &["GET"], get(file_page))
        .route("/{owner}/{name}/history", read, &["GET"], get(history_page))
        .route("/{owner}/{name}/change/{target}", read, &["GET"], get(change_page))
        .route("/{owner}/{name}/refs", read, &["GET"], get(refs_page))
        .route("/{owner}/{name}/verify", read, &["GET"], get(verify_page))
        .route("/{owner}/{name}/proposals", read, &["GET"], get(proposal_pages::list_page))
        .route("/{owner}/{name}/proposals/{id}", read, &["GET"], get(proposal_pages::detail_page))
        .route("/{owner}/{name}/proposals/{id}/close", close, &["POST"], post(proposal_pages::close_submit))
        .route("/{owner}/{name}/proposals/{id}/comments", comment_class, &["POST"], post(review_pages::comment_submit))
        .route("/{owner}/{name}/proposals/{id}/decisions", decision_class, &["POST"], post(review_pages::decision_submit))
        .route("/{owner}/{name}/proposals/{id}/comments/{cid}/hide", admin_class, &["POST"], post(review_pages::hide_submit))
        .route("/{owner}/{name}/proposals/{id}/comments/{cid}/unhide", admin_class, &["POST"], post(review_pages::unhide_submit))
}

pub fn router(state: AppState) -> Router {
    routes().build(state)
}

// -- request context: principal + CSRF token (minted on first sight, carried in a cookie) --

/// The signed-in person as the page header shows them: the account id, and the display name when there is
/// one (RFC 012 D-6a).
#[derive(Clone, Debug)]
pub(crate) struct Who {
    id: String,
    name: Option<String>,
}

struct Ctx {
    principal: Principal,
    user: Option<Who>,
    csrf: String,
    /// The page language and its messages (RFC 012 D-3).
    t: T,
    /// A `Set-Cookie` to attach if the CSRF cookie was just minted.
    set_csrf: Option<String>,
}

async fn ctx(headers: &HeaderMap, state: &AppState) -> Ctx {
    let principal = principal_from_request(headers, state).await;
    let user = match principal.user() {
        Some(u) => {
            let id = u.as_str().to_owned();
            let account = u.clone();
            // The name is looked up for the header only; a lookup that fails shows the id alone.
            let name = authn::blocking(state, move |s| s.auth.display_name(&account))
                .await
                .ok()
                .flatten();
            Some(Who { id, name })
        }
        None => None,
    };
    let (csrf, set_csrf) = match get_cookie(headers, CSRF_COOKIE) {
        Some(t) if !t.is_empty() => (t, None),
        _ => {
            let t = random_token();
            let set = set_cookie(CSRF_COOKIE, &t, CSRF_TTL_SECS);
            (t, Some(set))
        }
    };
    Ctx {
        principal,
        user,
        csrf,
        t: T::new(negotiate_from_headers(headers)),
        set_csrf,
    }
}

fn html(state: &AppState, status: StatusCode, body: String, cookies: Vec<String>) -> Response {
    let mut b = with_security_headers(
        state,
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .extension(LanguageVaries),
    );
    for c in cookies {
        b = b.header(header::SET_COOKIE, c);
    }
    b.body(Body::from(body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("empty response")
    })
}

fn redirect(state: &AppState, to: &str, cookies: Vec<String>) -> Response {
    let mut b = with_security_headers(
        state,
        Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, to),
    );
    for c in cookies {
        b = b.header(header::SET_COOKIE, c);
    }
    b.body(Body::empty()).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("empty response")
    })
}

fn render<T: Template>(state: &AppState, status: StatusCode, tpl: &T, c: &Ctx) -> Response {
    match tpl.render() {
        Ok(body) => html(state, status, body, c.set_csrf.iter().cloned().collect()),
        // A template that cannot render cannot be a page: the one sentence, as the body.
        Err(_) => html(
            state,
            StatusCode::INTERNAL_SERVER_ERROR,
            c.t.get(Key::PageTemplateError),
            vec![],
        ),
    }
}

fn error_page(state: &AppState, c: &Ctx, at: &str, e: Failure) -> Response {
    match e {
        // No prikk slot within the bounded wait (IS-4): the same existence-neutral page, plus
        // `Retry-After`.
        Failure::Busy => busy_page(state, c, at, retry_after_secs(state)),
        // RFC 011 D-4: the plain page for "too large" (the same message as the API's `too_large`).
        Failure::Read(ReadError::TooLarge) | Failure::OverBudget => render(
            state,
            StatusCode::UNPROCESSABLE_ENTITY,
            &TooLargeTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
                t: c.t,
                at: at.to_owned(),
            },
            c,
        ),
        // `NoSuchPath` is shown by the pages that know their repository (`path_error_page`); anywhere else it is
        // the plain not-found page.
        Failure::Read(ReadError::NotFound | ReadError::NoSuchPath) => render(
            state,
            StatusCode::NOT_FOUND,
            &NotFoundTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
                t: c.t,
                at: at.to_owned(),
            },
            c,
        ),
        _ => render(
            state,
            StatusCode::SERVICE_UNAVAILABLE,
            &UnavailableTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
                t: c.t,
                at: at.to_owned(),
            },
            c,
        ),
    }
}

/// The error page of a page that names a path inside a repository (a file or a folder): a path the reader may
/// look for but that is not there says so (IS-45), with a link back to the repository's files. Every other failure
/// is `error_page`'s, unchanged. `NoSuchPath` exists only for a reader who may read the repository (the service
/// raises it after authorization), so this page is never shown to a stranger of a private one.
fn path_error_page(
    state: &AppState,
    c: &Ctx,
    owner: &str,
    name: &str,
    ref_name: Option<&str>,
    at: &str,
    e: Failure,
) -> Response {
    if !matches!(e, Failure::Read(ReadError::NoSuchPath)) {
        return error_page(state, c, at, e);
    }
    let tpl = MessageTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        title: c
            .t
            .msg(Key::TitleNotFound)
            .with("path", at)
            .product()
            .text(),
        heading: c.t.get(Key::NotFoundHeading),
        body: c.t.get(Key::NoSuchPathBody),
        link_label: c.t.get(Key::NoSuchPathLink),
        link_href: format!("/{owner}/{name}{}", ref_query(ref_name)),
    };
    render(state, StatusCode::NOT_FOUND, &tpl, c)
}

/// The "busy" page with `Retry-After` (a refused request, IS-4 / IS-24): the same page whether or not the
/// repository or account exists.
fn busy_page(state: &AppState, c: &Ctx, at: &str, retry_after: u64) -> Response {
    let mut resp = render(
        state,
        StatusCode::SERVICE_UNAVAILABLE,
        &BusyTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            at: at.to_owned(),
        },
        c,
    );
    if let Ok(v) = retry_after.to_string().parse() {
        resp.headers_mut()
            .insert(axum::http::header::RETRY_AFTER, v);
    }
    resp
}

/// The pages that used to be bare strings (RFC 012 T7): each is a heading, one sentence and one link.
#[derive(Clone, Copy)]
enum Notice {
    CsrfFailed,
    FormIncomplete,
    SsoOff,
    SsoDown,
    SsoIncomplete,
    SsoMismatch,
    SsoExpired,
    SsoRefused,
    SsoUnlinked,
    ServerError,
}

impl Notice {
    /// (title, heading, body) keys.
    fn keys(self) -> (Key, Key, Key) {
        match self {
            Notice::CsrfFailed => (Key::PageCsrfTitle, Key::PageCsrfHeading, Key::PageCsrfBody),
            Notice::FormIncomplete => (Key::PageFormTitle, Key::PageFormHeading, Key::PageFormBody),
            Notice::SsoOff => (
                Key::PageSsoOffTitle,
                Key::PageSsoOffHeading,
                Key::PageSsoOffBody,
            ),
            Notice::SsoDown => (
                Key::PageSsoDownTitle,
                Key::PageSsoDownHeading,
                Key::PageSsoDownBody,
            ),
            Notice::SsoIncomplete => (
                Key::PageSsoIncompleteTitle,
                Key::PageSsoIncompleteHeading,
                Key::PageSsoIncompleteBody,
            ),
            Notice::SsoMismatch => (
                Key::PageSsoMismatchTitle,
                Key::PageSsoMismatchHeading,
                Key::PageSsoMismatchBody,
            ),
            Notice::SsoExpired => (
                Key::PageSsoExpiredTitle,
                Key::PageSsoExpiredHeading,
                Key::PageSsoExpiredBody,
            ),
            Notice::SsoRefused => (
                Key::PageSsoRefusedTitle,
                Key::PageSsoRefusedHeading,
                Key::PageSsoRefusedBody,
            ),
            Notice::SsoUnlinked => (
                Key::PageSsoUnlinkedTitle,
                Key::PageSsoUnlinkedHeading,
                Key::PageSsoUnlinkedBody,
            ),
            Notice::ServerError => (
                Key::PageServerErrorTitle,
                Key::PageServerErrorHeading,
                Key::PageServerErrorBody,
            ),
        }
    }
}

/// Where a notice page sends the reader next.
#[derive(Clone, Copy)]
enum Next {
    SignIn,
    Home,
}

/// A notice page with its status code (the codes the bare strings had) and, optionally, cookies.
fn notice(
    state: &AppState,
    c: &Ctx,
    status: StatusCode,
    which: Notice,
    next: Next,
    cookies: Vec<String>,
) -> Response {
    let (title, heading, body) = which.keys();
    let (link_label, link_href) = match next {
        Next::SignIn => (Key::PageLinkSignIn, "/login"),
        Next::Home => (Key::PageLinkHome, "/"),
    };
    let tpl = MessageTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t: c.t,
        title: c.t.msg(title).product().text(),
        heading: c.t.get(heading),
        body: c.t.get(body),
        link_label: c.t.get(link_label),
        link_href: link_href.to_owned(),
    };
    let mut resp = render(state, status, &tpl, c);
    for ck in cookies {
        if let Ok(v) = ck.parse() {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    resp
}

/// The assurance badge: its CSS class and its label's catalog key.
fn assurance(a: Assurance) -> (&'static str, Key) {
    match a {
        Assurance::PrikkVerified => ("ok", Key::AssuranceVerified),
        // Signed in, nothing verified: the plain badge, never the green one (IS-44).
        Assurance::Sealed => ("neutral", Key::AssuranceSealed),
        Assurance::ForgeApprovedUnsealed => ("warn", Key::AssuranceQueued),
        Assurance::Unverified => ("bad", Key::AssuranceUnverified),
    }
}

/// One paragraph of the in-place explanation (RFC 012 D-4): the sentence, and the label it explains when the
/// page can show several labels (the change page, one per patch).
struct ExplainRow {
    label: Option<Key>,
    text: Key,
}

/// What a badge's label means, as the catalog key of its explanation.
fn explanation(label: Key) -> Key {
    match label {
        Key::AssuranceVerified => Key::ExplainVerified,
        Key::AssuranceSealed => Key::ExplainSigned,
        Key::AssuranceQueued => Key::ExplainQueued,
        Key::AssuranceUnverified => Key::ExplainFailed,
        _ => Key::ExplainUnavailable,
    }
}

/// The explanation of the one badge a repository-level page shows.
fn explain_badge(label: Key) -> Vec<ExplainRow> {
    vec![ExplainRow {
        label: None,
        text: explanation(label),
    }]
}

/// The same on the Verification page, where the findings are on the page itself: the failed sentence says
/// "listed below" instead of pointing at the page the reader is on.
fn explain_badge_here(label: Key) -> Vec<ExplainRow> {
    vec![ExplainRow {
        label: None,
        text: match label {
            Key::AssuranceUnverified => Key::ExplainFailedHere,
            other => explanation(other),
        },
    }]
}

fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Percent-encode a query value (RFC 3986 unreserved characters pass through).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn ref_query(r: Option<&str>) -> String {
    r.map(|r| format!("?ref={}", pct(r))).unwrap_or_default()
}

// -- listing: prikk emits leaf paths; the page builds the directory level --

struct DirRow {
    name: String,
    path: String,
}
struct FileRow {
    name: String,
    path: String,
    size: u64,
    encoding: String,
}

/// The immediate children of `prefix` (`""` = root; otherwise `dir/` with a trailing slash).
fn listing(entries: &[TreeEntryView], prefix: &str) -> (Vec<DirRow>, Vec<FileRow>) {
    let mut dirs: Vec<DirRow> = Vec::new();
    let mut files = Vec::new();
    for e in entries {
        let Some(rest) = e.path.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            Some((d, _)) => {
                if !dirs.iter().any(|x| x.name == d) {
                    dirs.push(DirRow {
                        name: d.to_owned(),
                        path: format!("{prefix}{d}"),
                    });
                }
            }
            None => files.push(FileRow {
                name: rest.to_owned(),
                path: e.path.clone(),
                size: e.size,
                encoding: e.encoding.clone().unwrap_or_default(),
            }),
        }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    (dirs, files)
}

// -- templates --
//
// Every template carries `t`, the page's language (RFC 012 D-3): all words come from the catalog.

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    /// What was typed, shown again beside the error.
    value: String,
    error: bool,
}
#[derive(Template)]
#[template(path = "login.html")]
struct LoginTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    /// Why the form is shown again: the one message for wrong credentials, or the lock-out's own.
    error: Option<Key>,
    /// The page title for this state.
    title: Key,
    sso: bool,
}
#[derive(Template)]
#[template(path = "notfound.html")]
struct NotFoundTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    /// The path that was asked for: the title's context.
    at: String,
}
#[derive(Template)]
#[template(path = "unavailable.html")]
struct UnavailableTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    /// The path that was asked for: the title's context.
    at: String,
}
#[derive(Template)]
#[template(path = "busy.html")]
struct BusyTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    at: String,
}
#[derive(Template)]
#[template(path = "toolarge.html")]
struct TooLargeTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    /// The path that was asked for: the title's context.
    at: String,
}
#[derive(Template)]
#[template(path = "message.html")]
struct MessageTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    title: String,
    heading: String,
    body: String,
    link_label: String,
    link_href: String,
}
#[derive(Template)]
#[template(path = "repo.html")]
struct RepoTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    open_proposals: Option<String>,
    owner: String,
    name: String,
    /// `owner/name`, the repository's address.
    repo: String,
    point: String,
    current_branch: Option<String>,
    assurance_class: &'static str,
    assurance_label: Key,
    explain: Vec<ExplainRow>,
    dirs: Vec<DirRow>,
    files: Vec<FileRow>,
    ref_query: String,
    readme_html: Option<String>,
}
#[derive(Template)]
#[template(path = "tree.html")]
struct TreeTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    owner: String,
    name: String,
    repo: String,
    point: String,
    prefix: String,
    dirs: Vec<DirRow>,
    files: Vec<FileRow>,
    ref_query: String,
}
#[derive(Template)]
#[template(path = "file.html")]
struct FileTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    owner: String,
    name: String,
    repo: String,
    path: String,
    size: u64,
    encoding: String,
    raw_url: String,
    /// The last path segment: what the download link names.
    file_name: String,
    rendered_html: Option<String>,
    text: Option<String>,
}
struct HistoryRow {
    block_id: String,
    short_id: String,
    kind: String,
    rollback_block: bool,
    patch_count: u64,
    /// The link text: the change's first message line, or "Change {id}" when it has none.
    summary: String,
    /// The rest of the messages (and the rest of the first one), shown under the summary.
    more: Vec<String>,
}
#[derive(Template)]
#[template(path = "history.html")]
struct HistoryTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    owner: String,
    name: String,
    repo: String,
    ref_name: String,
    assurance_class: &'static str,
    assurance_label: Key,
    explain: Vec<ExplainRow>,
    entries: Vec<HistoryRow>,
}
struct OpRow {
    kind: String,
    paths: Vec<String>,
}
struct PatchRow {
    short_id: String,
    assurance_class: &'static str,
    assurance_label: Key,
    operations: Vec<OpRow>,
}
#[derive(Template)]
#[template(path = "change.html")]
struct ChangeTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    owner: String,
    name: String,
    repo: String,
    target: String,
    /// The target shortened for the title.
    target_short: String,
    /// What each badge on the page means, and the two terms the page uses (patch, rollback).
    explain: Vec<ExplainRow>,
    patches: Vec<PatchRow>,
}
struct BranchRow {
    ref_name: String,
    short_state: String,
    current: bool,
    closed: bool,
}
struct TagRow {
    ref_name: String,
    short_target: String,
}
#[derive(Template)]
#[template(path = "refs.html")]
struct RefsTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    owner: String,
    name: String,
    repo: String,
    branches: Vec<BranchRow>,
    tags: Vec<TagRow>,
}
struct StageRow {
    stage: String,
    /// The status in the page's language (an unknown token from prikk is shown as it came). For a check that
    /// was not run it names the check that blocked it.
    status_label: String,
    /// prikk's own message for a failed check. Empty when the status says it all.
    detail: String,
}
/// One finding of a failed check: prikk's id and message, shown as prikk wrote them.
struct FindingRow {
    id: String,
    message: String,
}
#[derive(Template)]
#[template(path = "verify.html")]
struct VerifyTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    proposals_nav: bool,
    owner: String,
    name: String,
    repo: String,
    assurance_class: &'static str,
    assurance_label: Key,
    explain: Vec<ExplainRow>,
    failed: Vec<FindingRow>,
    stages: Vec<StageRow>,
}

/// A history row: the link text is the change's first message line (data from prikk, escaped by the template),
/// with the short id as plain secondary text; a change with no message is "Change {id}".
fn history_row(t: &T, e: &planeter_core::read::HistoryEntry) -> HistoryRow {
    let short_id = short(&e.block_id);
    let mut messages = e
        .messages
        .iter()
        .map(|m| m.message.trim())
        .filter(|m| !m.is_empty());
    let (summary, mut more) = match messages.next() {
        Some(first) => {
            let (line, rest) = first.split_once('\n').unwrap_or((first, ""));
            let rest = rest.trim();
            (
                line.trim().to_owned(),
                if rest.is_empty() {
                    vec![]
                } else {
                    vec![rest.to_owned()]
                },
            )
        }
        None => (
            t.msg(Key::HistoryChangeFallback)
                .with("id", &short_id)
                .text(),
            vec![],
        ),
    };
    more.extend(messages.map(str::to_owned));
    HistoryRow {
        block_id: e.block_id.clone(),
        short_id,
        kind: e.kind.clone(),
        rollback_block: e.rollback_block,
        patch_count: e.patch_count,
        summary,
        more,
    }
}

/// The catalog label of a verify stage status token (`evaluated`, `failed`, `not_evaluated`, `halted`). A
/// `not_evaluated` stage names its blocker from the view's `blocked_by` field and a `halted` one the stage it
/// stopped after from `after`, not from the English `detail`.
fn stage_status_label(t: &T, stage: &planeter_core::read::StageView) -> String {
    match stage.status.as_str() {
        "evaluated" => t.get(Key::VerifyStatusEvaluated),
        "failed" => t.get(Key::VerifyStatusFailed),
        "not_evaluated" => match stage.blocked_by.as_deref().filter(|b| !b.is_empty()) {
            Some(check) => t
                .msg(Key::VerifyStatusNotEvaluated)
                .with("check", check)
                .text(),
            None => t.get(Key::VerifyStatusNotRun),
        },
        "halted" => match stage.after.as_deref().filter(|a| !a.is_empty()) {
            Some(check) => t
                .msg(Key::VerifyStatusHaltedAfter)
                .with("check", check)
                .text(),
            None => t.get(Key::VerifyStatusHalted),
        },
        other => other.to_owned(),
    }
}

/// The detail cell of a verify stage: prikk's message for a failure. A check that was not run or stopped early
/// has none: its status already names the other check (and the English `detail` would not translate).
fn stage_detail(stage: &planeter_core::read::StageView) -> String {
    if matches!(stage.status.as_str(), "not_evaluated" | "halted") {
        return String::new();
    }
    stage.detail.clone().unwrap_or_default()
}

// -- query models --

#[derive(serde::Deserialize)]
struct RefQuery {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
}
#[derive(serde::Deserialize)]
struct GoQuery {
    repo: String,
}
#[derive(serde::Deserialize)]
struct LoginForm {
    csrf: String,
    username: String,
    password: String,
}
#[derive(serde::Deserialize)]
struct CsrfForm {
    csrf: String,
}

// -- handlers --

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let c = ctx(&headers, &state).await;
    render(
        &state,
        StatusCode::OK,
        &HomeTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            value: String::new(),
            error: false,
        },
        &c,
    )
}

/// The home page again with the address field in error (RFC 012 D-2): the message sits beside the field,
/// which is marked invalid, and the status is 400.
fn address_error(state: &AppState, c: &Ctx, value: &str) -> Response {
    render(
        state,
        StatusCode::BAD_REQUEST,
        &HomeTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            value: value.to_owned(),
            error: true,
        },
        c,
    )
}

async fn go(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<GoQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(q)) = q else {
        let c = ctx(&headers, &state).await;
        return address_error(&state, &c, "");
    };
    let ok = q
        .repo
        .split_once('/')
        .is_some_and(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'));
    if ok {
        redirect(&state, &format!("/{}", pct(&q.repo)), vec![])
    } else {
        let c = ctx(&headers, &state).await;
        address_error(&state, &c, &q.repo)
    }
}

async fn stylesheet(State(state): State<AppState>) -> Response {
    with_security_headers(
        &state,
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
            .header(header::CACHE_CONTROL, "public, max-age=3600"),
    )
    .body(Body::from(include_str!("../static/app.css")))
    .unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("empty response")
    })
}

async fn login_form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let c = ctx(&headers, &state).await;
    render(
        &state,
        StatusCode::OK,
        &LoginTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            error: None,
            title: Key::TitleLogin,
            sso: state.oidc.is_some(),
        },
        &c,
    )
}

async fn login_submit(
    State(state): State<AppState>,
    PeerAddr(peer): PeerAddr,
    headers: HeaderMap,
    form: Result<Form<LoginForm>, FormRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    let Ok(Form(form)) = form else {
        return notice(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Notice::FormIncomplete,
            Next::SignIn,
            vec![],
        );
    };
    let cookie_csrf = get_cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, &form.csrf) {
        return notice(
            &state,
            &c,
            StatusCode::FORBIDDEN,
            Notice::CsrfFailed,
            Next::SignIn,
            vec![],
        );
    }
    let now = (state.now_unix)();
    // C-2b: a locked account — or a locked client address — is refused before the password is checked.
    let ip_key =
        client_ip(peer.map(|p| p.ip()), &headers, &state.trusted_proxies).map(|ip| ip.to_string());
    let ip_locked = ip_key
        .as_deref()
        .is_some_and(|k| state.ip_throttle.check(k, now).is_err());
    if ip_locked || state.login_throttle.check(&form.username, now).is_err() {
        return render(
            &state,
            StatusCode::TOO_MANY_REQUESTS,
            &LoginTpl {
                user: None,
                csrf: c.csrf.clone(),
                t: c.t,
                error: Some(Key::LoginLocked),
                title: Key::TitleLoginLocked,
                sso: state.oidc.is_some(),
            },
            &c,
        );
    }
    // Password verification takes a permit from the auth gate and runs off the async workers. Every
    // verification does: an unknown account's dummy one too, so a refusal under load says nothing about
    // whether the account exists (IS-24, IS-31). A refused request is not a failed attempt.
    let (user, password) = (form.username.clone(), form.password.clone());
    let verdict = authn::blocking_gated(&state, move |s| {
        s.auth.authenticate_password(&user, &password)
    })
    .await;
    let verdict = match verdict {
        Ok(v) => v,
        Err(GateError::Busy) => {
            let wait = state.auth_gate.wait().as_secs().max(1);
            return busy_page(&state, &c, "/login", wait);
        }
        Err(GateError::Aborted) => {
            return notice(
                &state,
                &c,
                StatusCode::INTERNAL_SERVER_ERROR,
                Notice::ServerError,
                Next::SignIn,
                vec![],
            );
        }
    };
    match verdict {
        Ok(Principal::User(user)) => {
            state.login_throttle.record_success(&form.username);
            if let Some(k) = &ip_key {
                state.ip_throttle.record_success(k);
            }
            let created = authn::blocking(&state, move |s| {
                s.sessions.create(user, now, now + SESSION_TTL_SECS)
            })
            .await;
            match created {
                Ok(Ok(session)) => redirect(
                    &state,
                    "/",
                    vec![set_cookie(SESSION_COOKIE, &session.id.0, SESSION_TTL_SECS)],
                ),
                _ => notice(
                    &state,
                    &c,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Notice::ServerError,
                    Next::SignIn,
                    vec![],
                ),
            }
        }
        _ => {
            state.login_throttle.record_failure(&form.username, now);
            if let Some(k) = &ip_key {
                state.ip_throttle.record_failure(k, now);
            }
            render(
                &state,
                StatusCode::UNAUTHORIZED,
                &LoginTpl {
                    user: None,
                    csrf: c.csrf.clone(),
                    t: c.t,
                    error: Some(Key::LoginFailed),
                    title: Key::TitleLoginFailed,
                    sso: state.oidc.is_some(),
                },
                &c,
            )
        }
    }
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<CsrfForm>, FormRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    let Ok(Form(form)) = form else {
        return notice(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Notice::FormIncomplete,
            Next::Home,
            vec![],
        );
    };
    let cookie_csrf = get_cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, &form.csrf) {
        return notice(
            &state,
            &c,
            StatusCode::FORBIDDEN,
            Notice::CsrfFailed,
            Next::Home,
            vec![],
        );
    }
    if let Some(id) = get_cookie(&headers, SESSION_COOKIE) {
        let _ = authn::blocking(&state, move |s| s.sessions.delete(&SessionId(id))).await;
    }
    redirect(&state, "/", vec![clear_cookie(SESSION_COOKIE)])
}

/// The language switcher's form: the CSRF token, the language chosen, and the page to return to.
#[derive(serde::Deserialize)]
struct LangForm {
    csrf: String,
    lang: String,
    #[serde(default)]
    next: String,
}

/// `POST /lang`: remember the visitor's language in a cookie (a language code and nothing else) and send them
/// back to the page they were on. CSRF double-submit like every other form; an unknown language is a refused
/// form, not a silent fallback; the return address must be a local path.
async fn set_language(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<LangForm>, FormRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    let Ok(Form(form)) = form else {
        return notice(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Notice::FormIncomplete,
            Next::Home,
            vec![],
        );
    };
    let cookie_csrf = get_cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, &form.csrf) {
        return notice(
            &state,
            &c,
            StatusCode::FORBIDDEN,
            Notice::CsrfFailed,
            Next::Home,
            vec![],
        );
    }
    let Some(lang) = Lang::from_tag(&form.lang) else {
        return notice(
            &state,
            &c,
            StatusCode::UNPROCESSABLE_ENTITY,
            Notice::FormIncomplete,
            Next::Home,
            vec![],
        );
    };
    let to = crate::lang_switch::safe_back(&form.next).unwrap_or("/");
    redirect(&state, to, vec![lang_cookie(lang.tag())])
}

#[derive(serde::Deserialize)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

/// Start an SSO sign-in: 303 to the provider; the opaque `state` rides in a short-lived cookie.
async fn oidc_begin(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let c = ctx(&headers, &state).await;
    if state.oidc.is_none() {
        return notice(
            &state,
            &c,
            StatusCode::NOT_FOUND,
            Notice::SsoOff,
            Next::SignIn,
            vec![],
        );
    }
    // Provider discovery may run a confined `curl`: off the async workers.
    let now = (state.now_unix)();
    let begun = authn::blocking(&state, move |s| s.oidc.map(|o| o.begin_login(now))).await;
    let begun = match begun {
        Ok(Some(r)) => r,
        _ => {
            return notice(
                &state,
                &c,
                StatusCode::BAD_GATEWAY,
                Notice::SsoDown,
                Next::SignIn,
                vec![],
            );
        }
    };
    match begun {
        Ok(b) => redirect(
            &state,
            &b.authorization_url,
            vec![set_cookie(OIDC_STATE_COOKIE, &b.state, PENDING_TTL_SECS)],
        ),
        Err(e) => {
            // The provider's error is for the operator's log, never for the page.
            eprintln!("planeter: SSO provider unavailable: {e}");
            notice(
                &state,
                &c,
                StatusCode::BAD_GATEWAY,
                Notice::SsoDown,
                Next::SignIn,
                vec![],
            )
        }
    }
}

/// The provider's callback: state (cookie == query, one-shot server-side) → code exchange → ID token
/// verification → the linked account → a session. Unlinked subjects are refused (no auto-provisioning).
async fn oidc_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    q: Result<Query<OidcCallbackQuery>, QueryRejection>,
) -> Response {
    let c = ctx(&headers, &state).await;
    if state.oidc.is_none() {
        return notice(
            &state,
            &c,
            StatusCode::NOT_FOUND,
            Notice::SsoOff,
            Next::SignIn,
            vec![],
        );
    }
    let (Some(code), Some(st)) = q.map(|Query(q)| (q.code, q.state)).unwrap_or((None, None)) else {
        return notice(
            &state,
            &c,
            StatusCode::BAD_REQUEST,
            Notice::SsoIncomplete,
            Next::SignIn,
            vec![],
        );
    };
    let cookie_state = get_cookie(&headers, OIDC_STATE_COOKIE).unwrap_or_default();
    if cookie_state.is_empty() || !tokens_match(&cookie_state, &st) {
        return notice(
            &state,
            &c,
            StatusCode::BAD_REQUEST,
            Notice::SsoMismatch,
            Next::SignIn,
            vec![],
        );
    }
    let now = (state.now_unix)();
    // Code exchange and ID-token verification run a confined `curl`; the account lookup and the session
    // write are SQLite. All of it is blocking, so all of it runs off the async workers.
    let completed = authn::blocking(&state, move |s| {
        let oidc = s.oidc?;
        let identity = match oidc.finish_login(&st, &code, now) {
            Ok(id) => id,
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(match s.auth.authenticate_oidc(&identity) {
            Ok(Principal::User(user)) => s
                .sessions
                .create(user, now, now + SESSION_TTL_SECS)
                .map(Some)
                .map_err(|_| ()),
            _ => Ok(None),
        }))
    })
    .await;
    match completed {
        Ok(Some(Err(OidcError::State))) => notice(
            &state,
            &c,
            StatusCode::BAD_REQUEST,
            Notice::SsoExpired,
            Next::SignIn,
            vec![],
        ),
        Ok(Some(Err(_))) => notice(
            &state,
            &c,
            StatusCode::UNAUTHORIZED,
            Notice::SsoRefused,
            Next::SignIn,
            vec![],
        ),
        Ok(Some(Ok(Ok(Some(session))))) => redirect(
            &state,
            "/",
            vec![
                set_cookie(SESSION_COOKIE, &session.id.0, SESSION_TTL_SECS),
                clear_cookie(OIDC_STATE_COOKIE),
            ],
        ),
        Ok(Some(Ok(Ok(None)))) => notice(
            &state,
            &c,
            StatusCode::UNAUTHORIZED,
            Notice::SsoUnlinked,
            Next::SignIn,
            vec![clear_cookie(OIDC_STATE_COOKIE)],
        ),
        _ => notice(
            &state,
            &c,
            StatusCode::INTERNAL_SERVER_ERROR,
            Notice::ServerError,
            Next::SignIn,
            vec![],
        ),
    }
}

/// The repository badge's `verify`, admitted on its own: it is the page's most expensive call
/// (4.1 × the store) and optional — the page shows "Could not be checked now" when it is refused or fails — so
/// it must not decide whether the rest of the page can be served.
async fn badge_verify(
    state: &AppState,
    principal: &Principal,
    owner: &str,
    name: &str,
) -> Result<VerifyView, Failure> {
    let (p, o, n) = (principal.clone(), owner.to_owned(), name.to_owned());
    let subject = Subject::new(principal, owner, name, &[ReadVerb::Verify]);
    read_blocking(state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.verify(&p, ow, &n, adm.limits(ReadVerb::Verify))
        })
    })
    .await
}

async fn repo_home(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<RefQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let r = q.ref_name.as_deref();
    // The page's four prikk calls run in one gated job, under one permit (RFC 003 D-8, IS-4).
    let (principal, o, n, rr) = (
        c.principal.clone(),
        owner.clone(),
        name.clone(),
        r.map(str::to_owned),
    );
    // The badge's `verify` is admitted on its own (below): it costs 4.1 × the store, twice a `tree`, and
    // the page renders "Could not be checked now" without it — so a repository whose `verify` does not fit the
    // budget still gets its page instead of `too_large`.
    let subject = Subject::new(
        &c.principal,
        &owner,
        &name,
        &[ReadVerb::Tree, ReadVerb::Refs, ReadVerb::Cat],
    );
    let page = read_blocking(&state, subject, move |read, adm| {
        let r = rr.as_deref();
        let tree = try_both_owners(&o, |ow| {
            read.tree(&principal, ow, &n, r, None, adm.limits(ReadVerb::Tree))
        })?;
        let refs = try_both_owners(&o, |ow| {
            read.refs(&principal, ow, &n, false, adm.limits(ReadVerb::Refs))
        })
        .ok();
        let readme = tree
            .entries
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case("README.md"))
            .and_then(|e| {
                try_both_owners(&o, |ow| {
                    read.raw_file(
                        &principal,
                        ow,
                        &n,
                        RawTarget {
                            point: r,
                            path: &e.path,
                            max_bytes: Some(1 << 20),
                        },
                        adm.limits(ReadVerb::Cat),
                    )
                })
                .ok()
            });
        Ok((tree, refs, readme))
    })
    .await;
    let (tree, refs, readme) = match page {
        Ok(p) => p,
        Err(e) => return error_page(&state, &c, &format!("/{owner}/{name}"), e),
    };
    let verify = badge_verify(&state, &c.principal, &owner, &name).await;
    let (ac, al) = match &verify {
        Ok(v) => assurance(v.assurance()),
        Err(_) => ("bad", Key::AssuranceVerifyUnavailable),
    };
    let readme_html = readme.map(|raw| render_markdown(&String::from_utf8_lossy(&raw.bytes)));
    let (dirs, files) = listing(&tree.entries, "");
    let (proposals_nav, open_proposals) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &RepoTpl {
            proposals_nav,
            open_proposals: open_proposals.map(|n| c.t.msg(Key::RepoOpenProposals).n(n).text()),
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner: owner.clone(),
            name: name.clone(),
            point: tree.point.clone(),
            current_branch: refs.and_then(|r| r.current_branch),
            assurance_class: ac,
            assurance_label: al,
            explain: explain_badge(al),
            dirs,
            files,
            ref_query: ref_query(r),
            readme_html,
        },
        &c,
    )
}

async fn tree_page(
    State(state): State<AppState>,
    Path((owner, name, path)): Path<(String, String, String)>,
    Query(q): Query<RefQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let r = q.ref_name.as_deref();
    let prefix = path.trim_matches('/').to_owned();
    let (principal, o, n, rr, pf) = (
        c.principal.clone(),
        owner.clone(),
        name.clone(),
        r.map(str::to_owned),
        prefix.clone(),
    );
    let subject = Subject::new(&c.principal, &owner, &name, &[ReadVerb::Tree]);
    let tree = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.tree_existing(
                &principal,
                ow,
                &n,
                rr.as_deref(),
                Some(pf.as_str()),
                adm.limits(ReadVerb::Tree),
            )
        })
    })
    .await;
    let tree = match tree {
        Ok(t) => t,
        Err(e) => {
            return path_error_page(
                &state,
                &c,
                &owner,
                &name,
                r,
                &format!("/{owner}/{name}/tree/{prefix}"),
                e,
            );
        }
    };
    let (dirs, files) = listing(&tree.entries, &format!("{prefix}/"));
    let (proposals_nav, _) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &TreeTpl {
            proposals_nav,
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner: owner.clone(),
            name: name.clone(),
            point: tree.point.clone(),
            prefix,
            dirs,
            files,
            ref_query: ref_query(r),
        },
        &c,
    )
}

async fn file_page(
    State(state): State<AppState>,
    Path((owner, name, path)): Path<(String, String, String)>,
    Query(q): Query<RefQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let r = q.ref_name.as_deref();
    let (principal, o, n, rr, p) = (
        c.principal.clone(),
        owner.clone(),
        name.clone(),
        r.map(str::to_owned),
        path.clone(),
    );
    let subject = Subject::new(&c.principal, &owner, &name, &[ReadVerb::Cat]);
    let raw = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.raw_file(
                &principal,
                ow,
                &n,
                RawTarget {
                    point: rr.as_deref(),
                    path: &p,
                    max_bytes: Some(CONTENT_MAX_BYTES),
                },
                adm.limits(ReadVerb::Cat),
            )
        })
    })
    .await;
    let raw = match raw {
        Ok(f) => f,
        Err(e) => {
            return path_error_page(
                &state,
                &c,
                &owner,
                &name,
                r,
                &format!("/{owner}/{name}/blob/{path}"),
                e,
            );
        }
    };
    let raw_url = format!(
        "{}/api/v1/repos/{}/{}/raw?path={}{}",
        state.content_origin.as_str(),
        pct(&owner),
        pct(&name),
        pct(&path),
        r.map(|r| format!("&ref={}", pct(r))).unwrap_or_default()
    );
    let file_name = path.rsplit('/').next().unwrap_or(&path).to_owned();
    let is_text = raw.encoding.as_deref() == Some("text");
    let (rendered_html, text) = if is_text {
        let t = String::from_utf8_lossy(&raw.bytes).into_owned();
        if path.to_ascii_lowercase().ends_with(".md") {
            (Some(render_markdown(&t)), None)
        } else {
            (None, Some(t))
        }
    } else {
        (None, None)
    };
    let (proposals_nav, _) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &FileTpl {
            proposals_nav,
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner,
            name,
            path,
            size: raw.size,
            encoding: raw.encoding.unwrap_or_default(),
            raw_url,
            file_name,
            rendered_html,
            text,
        },
        &c,
    )
}

async fn history_page(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<RefQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let r = q.ref_name.as_deref();
    let (principal, o, n, rr) = (
        c.principal.clone(),
        owner.clone(),
        name.clone(),
        r.map(str::to_owned),
    );
    let subject = Subject::new(&c.principal, &owner, &name, &[ReadVerb::Log]);
    let page = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.history(
                &principal,
                ow,
                &n,
                rr.as_deref(),
                Some(HISTORY_PAGE),
                adm.limits(ReadVerb::Log),
            )
        })
    })
    .await;
    let h = match page {
        Ok(h) => h,
        Err(e) => return error_page(&state, &c, &format!("/{owner}/{name}/history"), e),
    };
    let verify = badge_verify(&state, &c.principal, &owner, &name).await;
    let (ac, al) = match &verify {
        Ok(v) => assurance(v.assurance()),
        Err(_) => ("bad", Key::AssuranceVerifyUnavailable),
    };
    let entries: Vec<HistoryRow> = h.entries.iter().map(|e| history_row(&c.t, e)).collect();
    // The rollback tag is explained once, and only on a page that shows one.
    let mut explain = explain_badge(al);
    if entries.iter().any(|e| e.rollback_block) {
        explain.push(ExplainRow {
            label: None,
            text: Key::ExplainRollback,
        });
    }
    let (proposals_nav, _) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &HistoryTpl {
            proposals_nav,
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner,
            name,
            ref_name: h.ref_name,
            assurance_class: ac,
            assurance_label: al,
            explain,
            entries,
        },
        &c,
    )
}

async fn change_page(
    State(state): State<AppState>,
    Path((owner, name, target)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let (principal, o, n, t) = (
        c.principal.clone(),
        owner.clone(),
        name.clone(),
        target.clone(),
    );
    let subject = Subject::new(&c.principal, &owner, &name, &[ReadVerb::Show]);
    let ch = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.change(&principal, ow, &n, &t, adm.limits(ReadVerb::Show))
        })
    })
    .await;
    let ch = match ch {
        Ok(v) => v,
        Err(e) => {
            return error_page(
                &state,
                &c,
                &format!("/{owner}/{name}/change/{}", short(&target)),
                e,
            );
        }
    };
    // One explanation per distinct badge on the page, then the two terms the page uses.
    let mut explain: Vec<ExplainRow> = Vec::new();
    for p in &ch.patches {
        let label = assurance(p.assurance).1;
        if !explain.iter().any(|r| r.label == Some(label)) {
            explain.push(ExplainRow {
                label: Some(label),
                text: explanation(label),
            });
        }
    }
    for text in [Key::ExplainPatch, Key::ExplainRollback] {
        explain.push(ExplainRow { label: None, text });
    }
    let patches = ch
        .patches
        .iter()
        .map(|p| {
            let (ac, al) = assurance(p.assurance);
            PatchRow {
                short_id: short(&p.patch_id),
                assurance_class: ac,
                assurance_label: al,
                operations: p
                    .operations
                    .iter()
                    .map(|op| OpRow {
                        kind: op.kind.clone(),
                        paths: op.paths.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    let (proposals_nav, _) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &ChangeTpl {
            proposals_nav,
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner,
            name,
            target_short: short(&target),
            target,
            explain,
            patches,
        },
        &c,
    )
}

async fn refs_page(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let (principal, o, n) = (c.principal.clone(), owner.clone(), name.clone());
    let subject = Subject::new(&c.principal, &owner, &name, &[ReadVerb::Refs]);
    let v = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.refs(&principal, ow, &n, true, adm.limits(ReadVerb::Refs))
        })
    })
    .await;
    let v = match v {
        Ok(v) => v,
        Err(e) => return error_page(&state, &c, &format!("/{owner}/{name}/refs"), e),
    };
    let (proposals_nav, _) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &RefsTpl {
            proposals_nav,
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner,
            name,
            branches: v
                .branches
                .iter()
                .map(|b| BranchRow {
                    ref_name: b.ref_name.clone(),
                    short_state: short(&b.ref_state_id),
                    current: b.current,
                    closed: b.closed,
                })
                .collect(),
            tags: v
                .tags
                .iter()
                .map(|t| TagRow {
                    ref_name: t.ref_name.clone(),
                    short_target: short(&t.target_block_id),
                })
                .collect(),
        },
        &c,
    )
}

async fn verify_page(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let (principal, o, n) = (c.principal.clone(), owner.clone(), name.clone());
    let subject = Subject::new(&c.principal, &owner, &name, &[ReadVerb::Verify]);
    let v = read_blocking(&state, subject, move |read, adm| {
        try_both_owners(&o, |ow| {
            read.verify(&principal, ow, &n, adm.limits(ReadVerb::Verify))
        })
    })
    .await;
    let v = match v {
        Ok(v) => v,
        Err(e) => return error_page(&state, &c, &format!("/{owner}/{name}/verify"), e),
    };
    let (ac, al) = assurance(v.assurance());
    let (proposals_nav, _) = proposal_pages::nav_for(&state, &c, &owner, &name).await;
    render(
        &state,
        StatusCode::OK,
        &VerifyTpl {
            proposals_nav,
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            t: c.t,
            repo: format!("{owner}/{name}"),
            owner,
            name,
            assurance_class: ac,
            assurance_label: al,
            explain: explain_badge_here(al),
            failed: v
                .failed_conditions
                .iter()
                .map(|f| FindingRow {
                    id: f.id.clone(),
                    message: f.message.clone(),
                })
                .collect(),
            stages: v
                .stages
                .iter()
                .map(|s| StageRow {
                    stage: s.stage.clone(),
                    status_label: stage_status_label(&c.t, s),
                    detail: stage_detail(s),
                })
                .collect(),
        },
        &c,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> TreeEntryView {
        TreeEntryView {
            path: path.into(),
            kind: "file".into(),
            encoding: Some("text".into()),
            mode: 33188,
            size: 1,
            content_id: None,
        }
    }

    #[test]
    fn listing_builds_one_directory_level_from_leaf_paths() {
        let e = [
            entry("README.md"),
            entry("src/main.rs"),
            entry("src/lib/a.rs"),
            entry("docs/x.md"),
        ];
        let (dirs, files) = listing(&e, "");
        assert_eq!(
            dirs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(),
            ["docs", "src"]
        );
        assert_eq!(files.len(), 1);
        let (dirs, files) = listing(&e, "src/");
        assert_eq!(dirs[0].path, "src/lib");
        assert_eq!(files[0].path, "src/main.rs");
    }

    fn stage(
        status: &str,
        detail: Option<&str>,
        blocked_by: Option<&str>,
    ) -> planeter_core::read::StageView {
        planeter_core::read::StageView {
            stage: "s".into(),
            status: status.into(),
            detail: detail.map(str::to_owned),
            blocked_by: blocked_by.map(str::to_owned),
            after: None,
        }
    }

    #[test]
    fn a_check_that_was_not_run_names_its_blocker_from_the_field_not_from_the_detail() {
        let t = T::new(crate::catalog::EN);
        // The detail says something else on purpose: the sentence is built from `blocked_by`.
        let s = stage("not_evaluated", Some("blocked by x"), Some("refs"));
        assert_eq!(stage_status_label(&t, &s), "not run (blocked by refs)");
        assert_eq!(stage_detail(&s), "", "the status already says it");
        // No blocker, or an empty one: the plain status, never "blocked by " and nothing.
        for blocked_by in [None, Some("")] {
            let s = stage("not_evaluated", Some("blocked by x"), blocked_by);
            assert_eq!(stage_status_label(&t, &s), "not run");
        }
        // The other statuses are the plain words; an unknown token is shown as it came.
        for (token, word) in [
            ("evaluated", "completed"),
            ("failed", "problem found"),
            ("halted", "stopped early"),
            ("surprise", "surprise"),
        ] {
            assert_eq!(stage_status_label(&t, &stage(token, None, None)), word);
        }
        assert_eq!(
            stage_detail(&stage("failed", Some("prikk says"), None)),
            "prikk says"
        );
        // A stopped check has no detail cell either: its status names the check it stopped after.
        assert_eq!(stage_detail(&stage("halted", Some("after refs"), None)), "");
    }

    #[test]
    fn a_stopped_check_names_what_it_stopped_after_from_the_field_not_from_the_detail() {
        let t = T::new(crate::catalog::EN);
        let mut s = stage("halted", Some("after x"), None);
        s.after = Some("refs".into());
        assert_eq!(stage_status_label(&t, &s), "stopped early (after refs)");
        for after in [None, Some("".to_owned())] {
            s.after = after;
            assert_eq!(stage_status_label(&t, &s), "stopped early");
        }
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(pct("heads/main"), "heads/main");
        assert_eq!(pct("a b&c"), "a%20b%26c");
        assert_eq!(ref_query(None), "");
    }
}
