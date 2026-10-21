#![forbid(unsafe_code)]
//! `planeter-web` — planeter upper (surfaces) layer. See RFC 001/003.
//!
//! The web/API surface over the read path (RFC 003): the **rendering-safety core** ([`sanitize`] —
//! strict HTML/Markdown sanitization; [`security`] — the strict CSP, the security headers, and the
//! isolated content origin for raw bytes), the HTTP/OpenAPI read [`api`] and the server-rendered browse
//! [`ui`], both calling `planeter_core::ReadService` behind the one `authorize(_, Read, _)` gate; the
//! [`principal`] extractor (session cookie or bearer token), the [`cookies`] helpers (CSRF double-submit,
//! `HttpOnly`/`SameSite=Strict`/`Secure`), sign-in (local password or OIDC) with the login throttles, and
//! the trusted-proxy [`client_ip`](mod@client_ip) rules. Every read-path prikk call runs through the
//! [`gate`]: off the async workers, under a bounded number of concurrent invocations (IS-4).

pub mod api;
pub(crate) mod authn;
pub mod budget;
pub mod catalog;
pub mod client_ip;
pub mod cookies;
pub mod errors;
pub mod gate;
pub mod headers;
pub mod lang_switch;
pub mod openapi;
pub mod principal;
pub mod routes;
pub mod sanitize;
pub mod security;
pub mod ui;

pub use api::AppState;
pub use client_ip::{TrustedProxies, bind_allowed, client_ip};
pub use gate::{AuthGate, BlockingGate, PrikkGate};
pub use routes::{Class, Entry, Routes, Surface};

/// Every route the web surface serves, with its authorization class (IS-14): the API's and the UI's. The router
/// is built from this and from nothing else, so the table is complete by construction.
pub fn registry() -> Routes {
    api::routes().merge(ui::routes())
}

/// The full application router: the JSON read API under `/api/v1/…` merged with the browse UI.
///
/// Around the routes: unknown `/api/` paths and wrong methods on them answer in the API's one error shape
/// (other unknown paths keep the bare `404` of the UI), and every response passes through the
/// response-policy layer ([`headers`]): cache directives, `Vary`, `Permissions-Policy` and, behind
/// declared trusted proxies, HSTS.
pub fn router(state: AppState) -> axum::Router {
    let for_fallback = state.clone();
    let for_405 = state.clone();
    let for_layer = state.clone();
    registry()
        .build(state)
        .fallback(move |uri: axum::http::Uri| {
            let state = for_fallback.clone();
            async move {
                if uri.path().starts_with("/api/") {
                    api::code_response(&state, errors::ErrorCode::NotFound)
                } else {
                    axum::http::Response::builder()
                        .status(axum::http::StatusCode::NOT_FOUND)
                        .body(axum::body::Body::empty())
                        .expect("empty 404")
                }
            }
        })
        .method_not_allowed_fallback(move |uri: axum::http::Uri| {
            let state = for_405.clone();
            async move {
                if uri.path().starts_with("/api/") {
                    return api::code_response(&state, errors::ErrorCode::MethodNotAllowed);
                }
                axum::http::Response::builder()
                    .status(axum::http::StatusCode::METHOD_NOT_ALLOWED)
                    .body(axum::body::Body::empty())
                    .expect("empty 405")
            }
        })
        .layer(axum::middleware::from_fn_with_state(
            for_layer,
            headers::response_policy,
        ))
        .layer(axum::middleware::from_fn(lang_switch::remember_path))
}
pub use sanitize::{render_markdown, sanitize_html};
pub use security::{
    ContentOrigin, ContentOriginCheck, REFUSE_SAME_HOST, WARN_SAME_HOST, app_csp,
    app_security_headers, check_content_origin, raw_content_headers,
};

/// Serve the read API on `addr` until the process is stopped.
pub async fn serve(state: AppState, addr: std::net::SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Connect-info gives handlers the TCP peer, which the trusted-proxy rules turn into the client IP.
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
}
