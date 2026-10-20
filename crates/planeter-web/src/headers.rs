//! The response policy: cache directives, `Vary`, `Permissions-Policy` and HSTS, applied to every
//! response in one place (RFC 003 follow-on, IS-29; forge-commons *Transport Security and the Web*).
//!
//! - **Dynamic responses** (UI and API, errors included) are `Cache-Control: private, no-cache`: a
//!   cache may keep a copy but must revalidate and a shared cache must not serve it, so a private
//!   repository's page is never handed to someone else and a conditional `GET` still saves the body.
//!   They carry `Vary: Cookie, Authorization` because the representation depends on who asks.
//! - **Credential flows** — any response that sets or clears a cookie, and the sign-in and sign-out
//!   responses — are `Cache-Control: no-store`.
//! - **The stylesheet** keeps its own `public, max-age=3600` (a handler-set `Cache-Control` is never
//!   overwritten) and, being static, is not `Vary`.
//! - **Language** (RFC 012 D-3): a response whose text depends on the caller's language — every HTML page
//!   ([`crate::catalog::LanguageVaries`]) and every API error body ([`crate::errors::ErrorBody`]) — gets
//!   `Vary: Accept-Language` and `Content-Language`; an error body is written here, in the language
//!   negotiated from the `planeter_lang` cookie and `Accept-Language`, from the code and the catalog keys the
//!   handler chose, so that no handler needs the request's headers.
//! - `Permissions-Policy` switches off browser features the app never uses.
//! - `Strict-Transport-Security: max-age=15552000` (180 days, no `includeSubDomains`, no `preload`) is
//!   sent **only when trusted proxies are configured**: that is the declaration that the app sits behind
//!   TLS. A development bind on plain loopback must not tell a browser to insist on HTTPS.
//!
//! A reverse proxy may rewrite these; the app's directives are a default, the proxy's policy wins.

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::api::AppState;

/// `Permissions-Policy`: features the app never uses, switched off.
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=()";

/// The HSTS value (see the module doc).
pub const HSTS: &str = "max-age=15552000";

/// The stylesheet's path (static: shared-cacheable, not `Vary`).
const STYLESHEET: &str = "/static/app.css";

pub async fn response_policy(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let signs_in_or_out = (path == "/login" && req.method() == Method::POST) || path == "/logout";
    let lang = crate::catalog::negotiate_from_headers(req.headers());
    let mut resp = next.run(req).await;
    let varies_by_language = resp
        .extensions()
        .get::<crate::catalog::LanguageVaries>()
        .is_some();
    if let Some(eb) = resp.extensions().get::<crate::errors::ErrorBody>().copied() {
        let body = eb.code.body_for(eb.keys, lang);
        *resp.body_mut() = axum::body::Body::from(body);
        resp.headers_mut().remove(header::CONTENT_LENGTH);
    }
    let language_varies = varies_by_language
        || resp
            .extensions()
            .get::<crate::errors::ErrorBody>()
            .is_some();
    let headers = resp.headers_mut();
    if language_varies && let Ok(v) = HeaderValue::from_str(lang.tag()) {
        headers.insert(header::CONTENT_LANGUAGE, v);
    }

    let sets_cookie = headers.contains_key(header::SET_COOKIE);
    if !headers.contains_key(header::CACHE_CONTROL) {
        let value = if sets_cookie || signs_in_or_out {
            "no-store"
        } else {
            "private, no-cache"
        };
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    }

    if path != STYLESHEET {
        // Merge with any `Vary` a handler already set.
        let mut vary: Vec<String> = headers
            .get_all(header::VARY)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        let mut needed_tokens = vec!["Cookie", "Authorization"];
        if language_varies {
            needed_tokens.push("Accept-Language");
        }
        for needed in needed_tokens {
            if !vary.iter().any(|t| t.eq_ignore_ascii_case(needed)) {
                vary.push(needed.to_owned());
            }
        }
        if let Ok(v) = HeaderValue::from_str(&vary.join(", ")) {
            headers.insert(header::VARY, v);
        }
    }

    if !headers.contains_key("permissions-policy") {
        headers.insert(
            "permissions-policy",
            HeaderValue::from_static(PERMISSIONS_POLICY),
        );
    }
    if !state.trusted_proxies.is_empty() && !headers.contains_key(header::STRICT_TRANSPORT_SECURITY)
    {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    }
    resp
}
