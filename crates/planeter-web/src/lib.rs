#![forbid(unsafe_code)]
//! `planeter-web` — planeter upper (surfaces) layer. See RFC 001/003.
//!
//! The web/API surface over the read path (RFC 003). This crate holds the **rendering-safety core**
//! ([`sanitize`] — strict HTML/Markdown sanitization; [`security`] — the strict CSP, the security
//! headers, and the isolated content origin for raw bytes), and (next) the HTTP/OpenAPI read API that
//! calls `planeter_core::ReadService` behind the one `authorize(_, Read, _)` gate.

pub mod api;
pub mod sanitize;
pub mod security;

pub use api::{AppState, router};
pub use sanitize::{render_markdown, sanitize_html};
pub use security::{ContentOrigin, app_csp, app_security_headers, raw_content_headers};

/// Serve the read API on `addr` until the process is stopped.
pub async fn serve(state: AppState, addr: std::net::SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await
}
