//! The language switcher's plumbing (RFC 012 D-3b, handoff `japanese-and-switcher-handoff-v1` T4).
//!
//! Every page carries, in its footer, one `POST /lang` form per available language (no script). The form must
//! say where to come back to, but the 15 page handlers and their templates know nothing about the request's
//! address, and the `Referrer-Policy` is `no-referrer` (so the browser sends no `Referer`). A small middleware
//! therefore records, for the duration of the request, **the path to return to** in a task-local; the footer
//! reads it when it renders ([`back`]). The path is only ever a local path ([`safe_back`]); a request that is
//! not a plain page view (a POST, the sign-in callback with its one-time `code`) returns to `/login` or `/`,
//! never to an address that carries a secret.

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;

tokio::task_local! {
    static BACK: String;
}

/// The longest address the switcher will carry back (anything longer returns to `/`).
const MAX_BACK: usize = 1024;

/// The path to return to for the page being rendered: set by [`remember_path`]; `/` outside a request.
pub fn back() -> String {
    BACK.try_with(Clone::clone)
        .unwrap_or_else(|_| "/".to_owned())
}

/// A return address that is safe to redirect to: a local path (`/…`), not protocol-relative (`//host`), no
/// backslash, no control character or space, within the length limit. Anything else is `None`.
pub fn safe_back(s: &str) -> Option<&str> {
    let ok = s.starts_with('/')
        && !s.starts_with("//")
        && !s.starts_with("/\\")
        && s.len() <= MAX_BACK
        && !s
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\');
    ok.then_some(s)
}

/// Where a visitor viewing `method uri` should be sent back to after choosing a language.
pub fn back_for(method: &Method, uri: &axum::http::Uri) -> String {
    let path = uri.path();
    // The sign-in's own callbacks and every non-GET request return to a page that can be shown again.
    let target = if path == "/login" {
        "/login".to_owned()
    } else if path.starts_with("/login/") || path == "/logout" || path == "/lang" {
        "/".to_owned()
    } else if *method == Method::GET {
        match uri.query() {
            Some(q) => format!("{path}?{q}"),
            None => path.to_owned(),
        }
    } else {
        "/".to_owned()
    };
    safe_back(&target).map_or_else(|| "/".to_owned(), str::to_owned)
}

/// The middleware: run the rest of the request with [`back`] answering for this request.
pub async fn remember_path(req: Request, next: Next) -> Response {
    let back = back_for(req.method(), req.uri());
    BACK.scope(back, next.run(req)).await
}
