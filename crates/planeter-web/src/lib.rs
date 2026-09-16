#![forbid(unsafe_code)]
//! `planeter-web` — planeter upper (surfaces) layer. See RFC 001/003.
//!
//! The web/API surface over the read path (RFC 003). This crate holds the **rendering-safety core**
//! ([`sanitize`] — strict HTML/Markdown sanitization; [`security`] — the strict CSP, the security
//! headers, and the isolated content origin for raw bytes), and (next) the HTTP/OpenAPI read API that
//! calls `planeter_core::ReadService` behind the one `authorize(_, Read, _)` gate.

pub mod sanitize;
pub mod security;

pub use sanitize::{render_markdown, sanitize_html};
pub use security::{ContentOrigin, app_csp, app_security_headers, raw_content_headers};
