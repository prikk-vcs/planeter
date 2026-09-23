//! The browse web UI (RFC 003 WEB-01/05/06): server-rendered pages over the same `ReadService` and the
//! same `authorize(_, Read, _)` gate as the API. Sign-in is a password → cookie session (WEB-05:
//! `HttpOnly`/`SameSite=Strict`/`Secure` cookies, CSRF double-submit on every POST). Every page carries
//! the strict CSP from [`crate::security`] — no inline script or style; the one stylesheet is served
//! from `/static/app.css` on the app origin; raw bytes link to the isolated content origin.
//!
//! Honesty on the page (WEB-06): the repository badge is prikk's re-derived `verify` verdict, and a
//! queued patch is labelled *forge-approved, unsealed* — never *prikk-verified*.

use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};

use planeter_auth::{SESSION_TTL_SECS, SessionId, random_token, tokens_match};
use planeter_core::{Assurance, Principal, ReadError, TreeEntryView};

use crate::api::{AppState, try_both_owners, with_security_headers};
use crate::cookies::{CSRF_COOKIE, SESSION_COOKIE, clear_cookie, get_cookie, set_cookie};
use crate::principal::principal_from_request;
use crate::sanitize::render_markdown;

const HISTORY_PAGE: usize = 50;
const CSRF_TTL_SECS: u64 = 30 * 24 * 3600;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/go", get(go))
        .route("/static/app.css", get(stylesheet))
        .route("/login", get(login_form).post(login_submit))
        .route("/logout", post(logout))
        .route("/{owner}/{name}", get(repo_home))
        .route("/{owner}/{name}/tree/{*path}", get(tree_page))
        .route("/{owner}/{name}/blob/{*path}", get(file_page))
        .route("/{owner}/{name}/history", get(history_page))
        .route("/{owner}/{name}/change/{target}", get(change_page))
        .route("/{owner}/{name}/refs", get(refs_page))
        .route("/{owner}/{name}/verify", get(verify_page))
        .with_state(state)
}

// -- request context: principal + CSRF token (minted on first sight, carried in a cookie) --

struct Ctx {
    principal: Principal,
    user: Option<String>,
    csrf: String,
    /// A `Set-Cookie` to attach if the CSRF cookie was just minted.
    set_csrf: Option<String>,
}

fn ctx(headers: &HeaderMap, state: &AppState) -> Ctx {
    let principal = principal_from_request(headers, state);
    let user = principal.user().map(|u| u.as_str().to_owned());
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
        set_csrf,
    }
}

fn html(state: &AppState, status: StatusCode, body: String, cookies: Vec<String>) -> Response {
    let mut b = with_security_headers(
        state,
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8"),
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
        Err(_) => html(
            state,
            StatusCode::INTERNAL_SERVER_ERROR,
            "template error".into(),
            vec![],
        ),
    }
}

fn error_page(state: &AppState, c: &Ctx, e: ReadError) -> Response {
    match e {
        ReadError::NotFound => render(
            state,
            StatusCode::NOT_FOUND,
            &NotFoundTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
            },
            c,
        ),
        _ => render(
            state,
            StatusCode::SERVICE_UNAVAILABLE,
            &UnavailableTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
            },
            c,
        ),
    }
}

fn assurance(a: Assurance) -> (&'static str, &'static str) {
    match a {
        Assurance::PrikkVerified => ("ok", "prikk-verified"),
        Assurance::ForgeApprovedUnsealed => ("warn", "forge-approved, unsealed"),
        Assurance::Unverified => ("bad", "unverified"),
    }
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

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTpl {
    user: Option<String>,
    csrf: String,
}
#[derive(Template)]
#[template(path = "login.html")]
struct LoginTpl {
    user: Option<String>,
    csrf: String,
    failed: bool,
}
#[derive(Template)]
#[template(path = "notfound.html")]
struct NotFoundTpl {
    user: Option<String>,
    csrf: String,
}
#[derive(Template)]
#[template(path = "unavailable.html")]
struct UnavailableTpl {
    user: Option<String>,
    csrf: String,
}
#[derive(Template)]
#[template(path = "repo.html")]
struct RepoTpl {
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    point: String,
    current_branch: Option<String>,
    assurance_class: &'static str,
    assurance_label: &'static str,
    dirs: Vec<DirRow>,
    files: Vec<FileRow>,
    ref_query: String,
    readme_html: Option<String>,
}
#[derive(Template)]
#[template(path = "tree.html")]
struct TreeTpl {
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    point: String,
    prefix: String,
    dirs: Vec<DirRow>,
    files: Vec<FileRow>,
    ref_query: String,
}
#[derive(Template)]
#[template(path = "file.html")]
struct FileTpl {
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    path: String,
    size: u64,
    encoding: String,
    raw_url: String,
    rendered_html: Option<String>,
    text: Option<String>,
}
struct HistoryRow {
    block_id: String,
    short_id: String,
    kind: String,
    rollback_block: bool,
    patch_count: u64,
    messages: Vec<String>,
}
#[derive(Template)]
#[template(path = "history.html")]
struct HistoryTpl {
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    ref_name: String,
    assurance_class: &'static str,
    assurance_label: &'static str,
    entries: Vec<HistoryRow>,
}
struct OpRow {
    kind: String,
    paths: Vec<String>,
}
struct PatchRow {
    short_id: String,
    assurance_class: &'static str,
    assurance_label: &'static str,
    operations: Vec<OpRow>,
}
#[derive(Template)]
#[template(path = "change.html")]
struct ChangeTpl {
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    target: String,
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
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    branches: Vec<BranchRow>,
    tags: Vec<TagRow>,
}
struct StageRow {
    stage: String,
    status: String,
    detail: String,
}
#[derive(Template)]
#[template(path = "verify.html")]
struct VerifyTpl {
    user: Option<String>,
    csrf: String,
    owner: String,
    name: String,
    assurance_class: &'static str,
    assurance_label: &'static str,
    verified_sessions: u64,
    total_sessions: u64,
    failed: Vec<String>,
    stages: Vec<StageRow>,
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
    let c = ctx(&headers, &state);
    render(
        &state,
        StatusCode::OK,
        &HomeTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
        },
        &c,
    )
}

async fn go(State(state): State<AppState>, Query(q): Query<GoQuery>) -> Response {
    let ok = q
        .repo
        .split_once('/')
        .is_some_and(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'));
    if ok {
        redirect(&state, &format!("/{}", pct(&q.repo)), vec![])
    } else {
        redirect(&state, "/", vec![])
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
    let c = ctx(&headers, &state);
    render(
        &state,
        StatusCode::OK,
        &LoginTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            failed: false,
        },
        &c,
    )
}

async fn login_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let c = ctx(&headers, &state);
    let cookie_csrf = get_cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, &form.csrf) {
        return html(
            &state,
            StatusCode::FORBIDDEN,
            "CSRF check failed".into(),
            vec![],
        );
    }
    let now = (state.now_unix)();
    // C-2b: a locked account is refused before the password is even checked.
    if state.login_throttle.check(&form.username, now).is_err() {
        return render(
            &state,
            StatusCode::TOO_MANY_REQUESTS,
            &LoginTpl {
                user: None,
                csrf: c.csrf.clone(),
                failed: true,
            },
            &c,
        );
    }
    match state
        .auth
        .authenticate_password(&form.username, &form.password)
    {
        Ok(Principal::User(user)) => {
            state.login_throttle.record_success(&form.username);
            match state.sessions.create(user, now, now + SESSION_TTL_SECS) {
                Ok(session) => redirect(
                    &state,
                    "/",
                    vec![set_cookie(SESSION_COOKIE, &session.id.0, SESSION_TTL_SECS)],
                ),
                Err(_) => html(
                    &state,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session error".into(),
                    vec![],
                ),
            }
        }
        _ => {
            state.login_throttle.record_failure(&form.username, now);
            render(
                &state,
                StatusCode::UNAUTHORIZED,
                &LoginTpl {
                    user: None,
                    csrf: c.csrf.clone(),
                    failed: true,
                },
                &c,
            )
        }
    }
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    let cookie_csrf = get_cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if cookie_csrf.is_empty() || !tokens_match(&cookie_csrf, &form.csrf) {
        return html(
            &state,
            StatusCode::FORBIDDEN,
            "CSRF check failed".into(),
            vec![],
        );
    }
    if let Some(id) = get_cookie(&headers, SESSION_COOKIE) {
        let _ = state.sessions.delete(&SessionId(id));
    }
    redirect(&state, "/", vec![clear_cookie(SESSION_COOKIE)])
}

async fn repo_home(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<RefQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state);
    let r = q.ref_name.as_deref();
    let tree = try_both_owners(&owner, |o| state.read.tree(&c.principal, o, &name, r, None));
    let tree = match tree {
        Ok(t) => t,
        Err(e) => return error_page(&state, &c, e),
    };
    let refs = try_both_owners(&owner, |o| state.read.refs(&c.principal, o, &name, false)).ok();
    let verify = try_both_owners(&owner, |o| state.read.verify(&c.principal, o, &name));
    let (ac, al) = match &verify {
        Ok(v) => assurance(v.assurance()),
        Err(_) => ("bad", "verify unavailable"),
    };
    let readme_html = tree
        .entries
        .iter()
        .find(|e| e.path.eq_ignore_ascii_case("README.md"))
        .and_then(|e| {
            try_both_owners(&owner, |o| {
                state
                    .read
                    .raw_file(&c.principal, o, &name, r, &e.path, Some(1 << 20))
            })
            .ok()
        })
        .map(|raw| render_markdown(&String::from_utf8_lossy(&raw.bytes)));
    let (dirs, files) = listing(&tree.entries, "");
    render(
        &state,
        StatusCode::OK,
        &RepoTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            owner: owner.clone(),
            name: name.clone(),
            point: tree.point.clone(),
            current_branch: refs.and_then(|r| r.current_branch),
            assurance_class: ac,
            assurance_label: al,
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
    let c = ctx(&headers, &state);
    let r = q.ref_name.as_deref();
    let prefix = path.trim_matches('/').to_owned();
    let tree = try_both_owners(&owner, |o| {
        state
            .read
            .tree(&c.principal, o, &name, r, Some(prefix.as_str()))
    });
    let tree = match tree {
        Ok(t) => t,
        Err(e) => return error_page(&state, &c, e),
    };
    let (dirs, files) = listing(&tree.entries, &format!("{prefix}/"));
    render(
        &state,
        StatusCode::OK,
        &TreeTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
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
    let c = ctx(&headers, &state);
    let r = q.ref_name.as_deref();
    let raw = try_both_owners(&owner, |o| {
        state
            .read
            .raw_file(&c.principal, o, &name, r, &path, Some(4 << 20))
    });
    let raw = match raw {
        Ok(f) => f,
        Err(e) => return error_page(&state, &c, e),
    };
    let raw_url = format!(
        "{}/api/v1/repos/{}/{}/raw?path={}{}",
        state.content_origin.as_str(),
        pct(&owner),
        pct(&name),
        pct(&path),
        r.map(|r| format!("&ref={}", pct(r))).unwrap_or_default()
    );
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
    render(
        &state,
        StatusCode::OK,
        &FileTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            owner,
            name,
            path,
            size: raw.size,
            encoding: raw.encoding.unwrap_or_default(),
            raw_url,
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
    let c = ctx(&headers, &state);
    let r = q.ref_name.as_deref();
    let h = try_both_owners(&owner, |o| {
        state
            .read
            .history(&c.principal, o, &name, r, Some(HISTORY_PAGE))
    });
    let h = match h {
        Ok(h) => h,
        Err(e) => return error_page(&state, &c, e),
    };
    let verify = try_both_owners(&owner, |o| state.read.verify(&c.principal, o, &name));
    let (ac, al) = match &verify {
        Ok(v) => assurance(v.assurance()),
        Err(_) => ("bad", "verify unavailable"),
    };
    let entries = h
        .entries
        .iter()
        .map(|e| HistoryRow {
            block_id: e.block_id.clone(),
            short_id: short(&e.block_id),
            kind: e.kind.clone(),
            rollback_block: e.rollback_block,
            patch_count: e.patch_count,
            messages: e.messages.iter().map(|m| m.message.clone()).collect(),
        })
        .collect();
    render(
        &state,
        StatusCode::OK,
        &HistoryTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            owner,
            name,
            ref_name: h.ref_name,
            assurance_class: ac,
            assurance_label: al,
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
    let c = ctx(&headers, &state);
    let ch = try_both_owners(&owner, |o| {
        state.read.change(&c.principal, o, &name, &target)
    });
    let ch = match ch {
        Ok(v) => v,
        Err(e) => return error_page(&state, &c, e),
    };
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
    render(
        &state,
        StatusCode::OK,
        &ChangeTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            owner,
            name,
            target,
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
    let c = ctx(&headers, &state);
    let v = try_both_owners(&owner, |o| state.read.refs(&c.principal, o, &name, true));
    let v = match v {
        Ok(v) => v,
        Err(e) => return error_page(&state, &c, e),
    };
    render(
        &state,
        StatusCode::OK,
        &RefsTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
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
    let c = ctx(&headers, &state);
    let v = try_both_owners(&owner, |o| state.read.verify(&c.principal, o, &name));
    let v = match v {
        Ok(v) => v,
        Err(e) => return error_page(&state, &c, e),
    };
    let (ac, al) = assurance(v.assurance());
    render(
        &state,
        StatusCode::OK,
        &VerifyTpl {
            user: c.user.clone(),
            csrf: c.csrf.clone(),
            owner,
            name,
            assurance_class: ac,
            assurance_label: al,
            verified_sessions: v.verified_sessions,
            total_sessions: v.total_sessions,
            failed: v
                .failed_conditions
                .iter()
                .map(|f| format!("{}: {}", f.id, f.message))
                .collect(),
            stages: v
                .stages
                .iter()
                .map(|s| StageRow {
                    stage: s.stage.clone(),
                    status: s.status.clone(),
                    detail: s.detail.clone().unwrap_or_default(),
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

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(pct("heads/main"), "heads/main");
        assert_eq!(pct("a b&c"), "a%20b%26c");
        assert_eq!(ref_query(None), "");
    }
}
