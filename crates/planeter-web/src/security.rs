//! Response security headers and the isolated content origin (RFC 003 T4 / `STD-6`).
//!
//! Two defenses live here, as plain header sets so they are testable without the HTTP layer and applied
//! uniformly by the router (T3):
//! - **A strict Content-Security-Policy** for app pages: no inline script, tight sources, `object-src
//!   'none'`, `frame-ancestors 'none'` — plus `nosniff` and `Referrer-Policy`. Even if sanitization
//!   (`crate::sanitize`) ever missed something, the CSP denies it a way to execute.
//! - **An isolated content origin.** Raw repository bytes (raw file view, downloads, hosted content) are
//!   served from a **distinct domain** from the app, with `nosniff`, `Content-Disposition: attachment`,
//!   and a `sandbox` CSP — so hostile bytes render as an inert download on an origin that shares no
//!   cookies or DOM with the app, never as active content in the app's origin.

/// A response header as (name, value).
pub type Header = (&'static str, String);

/// The isolated origin (a distinct scheme+host) that raw repository bytes are served from. Configured
/// per deployment; distinct from the app origin (`T4`).
///
/// It also knows the **application's own host** (the host the server is bound to), when the binary
/// supplies it. The raw route answers only on the content origin's host (IS-16); when the content
/// origin's host *is* the application's host there is no isolation to enforce, and the route answers
/// on that host (the loopback development shape; production refuses to start in that state).
#[derive(Debug, Clone)]
pub struct ContentOrigin {
    origin: String,
    app_host: Option<String>,
}

impl ContentOrigin {
    /// An origin with the application's host unknown: the raw route is **gated** on the content host.
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            app_host: None,
        }
    }

    /// Record the application's own host (the bound address). Used for the same-host decision only.
    pub fn with_app_host(mut self, host: impl AsRef<str>) -> Self {
        self.app_host = Some(normalize_host(host.as_ref()));
        self
    }

    pub fn as_str(&self) -> &str {
        &self.origin
    }

    /// The content origin's host, lower-case, without port or brackets. `None` when the configured
    /// string has no recognizable host.
    pub fn host(&self) -> Option<String> {
        origin_host(&self.origin)
    }

    /// True when the content origin's host is the application's own host (a port-only difference is
    /// *not* isolation: cookies are host-scoped and ignore the port).
    pub fn same_host_as_app(&self) -> bool {
        match (&self.app_host, self.host()) {
            (Some(app), Some(content)) => *app == content,
            _ => false,
        }
    }

    /// Whether `effective_host` may be served the raw route. Same-host deployments are ungated (there
    /// is nothing to isolate); otherwise the request's host must be the content origin's host.
    pub fn serves_raw_on(&self, effective_host: Option<&str>) -> bool {
        if self.same_host_as_app() {
            return true;
        }
        match (self.host(), effective_host) {
            (Some(content), Some(h)) => normalize_host(h) == content,
            _ => false,
        }
    }
}

/// The host of an origin string `scheme://[userinfo@]host[:port][/...]`, normalized.
pub(crate) fn origin_host(origin: &str) -> Option<String> {
    let rest = origin.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let h = normalize_host(host);
    (!h.is_empty()).then_some(h)
}

/// Lower-case, strip a `:port`, strip IPv6 brackets and one trailing dot. Accepts a bare host, a
/// `host:port`, or a `Host`/`X-Forwarded-Host` header value.
pub(crate) fn normalize_host(raw: &str) -> String {
    let raw = raw.trim();
    let host = if let Some(rest) = raw.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if raw.matches(':').count() == 1 {
        raw.split(':').next().unwrap_or("")
    } else {
        // A bare IPv6 address (several colons, no brackets) or a plain name.
        raw
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// What the start-up check found about the configured content origin.
#[derive(Debug, PartialEq, Eq)]
pub enum ContentOriginCheck {
    /// A distinct host: the raw route is gated and the app host refuses it.
    Isolated,
    /// The same host as the application, on a loopback bind (development): one warning.
    SameHostLoopback,
}

/// The message printed when planeter refuses to start (IS-16 T2).
pub const REFUSE_SAME_HOST: &str = "planeter: refusing to start: raw repository content would be \
served from the application's own host, which breaks the isolation the documentation promises. Set \
PLANETER_CONTENT_ORIGIN to a different host (for example https://raw.example.org).";

/// The one-line warning for a same-host content origin on a loopback bind.
pub const WARN_SAME_HOST: &str = "planeter: warning: raw repository content is served from the \
application's own host (PLANETER_CONTENT_ORIGIN is not a different host); acceptable for local \
development only.";

/// Decide, before serving, whether the content origin is acceptable for this bind address. `Err` is
/// the full message to print (exit 1): either the refusal, or a malformed origin.
pub fn check_content_origin(
    content_origin: &ContentOrigin,
    bind: std::net::SocketAddr,
) -> Result<ContentOriginCheck, String> {
    if content_origin.host().is_none() {
        return Err(format!(
            "planeter: PLANETER_CONTENT_ORIGIN is not an origin with a host (expected \
             scheme://host[:port], got {:?})",
            content_origin.as_str()
        ));
    }
    if !content_origin.same_host_as_app() {
        return Ok(ContentOriginCheck::Isolated);
    }
    if crate::client_ip::is_loopback(bind.ip()) {
        Ok(ContentOriginCheck::SameHostLoopback)
    } else {
        Err(REFUSE_SAME_HOST.to_owned())
    }
}

/// The strict Content-Security-Policy for **app** pages. `img-src` also allows the content origin (for
/// avatars/inline images served there); everything else is `'self'` or denied. No inline/`unsafe-*`
/// script.
pub fn app_csp(content_origin: &ContentOrigin) -> String {
    format!(
        "default-src 'self'; \
         script-src 'self'; \
         style-src 'self'; \
         img-src 'self' {origin}; \
         font-src 'self'; \
         connect-src 'self'; \
         object-src 'none'; \
         base-uri 'none'; \
         frame-ancestors 'none'; \
         form-action 'self'",
        origin = content_origin.as_str()
    )
}

/// The full header set for an app (HTML/API) response.
pub fn app_security_headers(content_origin: &ContentOrigin) -> Vec<Header> {
    vec![
        ("Content-Security-Policy", app_csp(content_origin)),
        ("X-Content-Type-Options", "nosniff".to_owned()),
        ("X-Frame-Options", "DENY".to_owned()),
        ("Referrer-Policy", "no-referrer".to_owned()),
    ]
}

/// The header set for serving **raw repository bytes** from the content origin: force a download, forbid
/// sniffing, and sandbox any rendering so the bytes cannot act as content.
pub fn raw_content_headers(filename: &str) -> Vec<Header> {
    vec![
        ("X-Content-Type-Options", "nosniff".to_owned()),
        (
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", sanitize_filename(filename)),
        ),
        (
            "Content-Security-Policy",
            "default-src 'none'; sandbox".to_owned(),
        ),
        ("X-Frame-Options", "DENY".to_owned()),
        ("Referrer-Policy", "no-referrer".to_owned()),
    ]
}

/// Strip characters that could break out of the `Content-Disposition` filename quoting or inject a path.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' | '/' => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> ContentOrigin {
        ContentOrigin::new("https://raw.planeter.example")
    }

    #[test]
    fn app_csp_forbids_inline_script_and_framing() {
        let csp = app_csp(&origin());
        assert!(csp.contains("script-src 'self'"));
        assert!(!csp.contains("unsafe-inline"));
        assert!(!csp.contains("unsafe-eval"));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        // the content origin is allowed only for images
        assert!(csp.contains("img-src 'self' https://raw.planeter.example"));
    }

    #[test]
    fn app_headers_include_the_non_negotiables() {
        let headers = app_security_headers(&origin());
        let names: Vec<_> = headers.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"Content-Security-Policy"));
        assert!(names.contains(&"X-Content-Type-Options"));
        assert!(names.contains(&"X-Frame-Options"));
        assert!(names.contains(&"Referrer-Policy"));
    }

    #[test]
    fn raw_content_forces_download_and_no_sniff() {
        let headers = raw_content_headers("README.md");
        let map: std::collections::HashMap<_, _> = headers.into_iter().collect();
        assert_eq!(
            map.get("X-Content-Type-Options").map(String::as_str),
            Some("nosniff")
        );
        assert!(map["Content-Disposition"].starts_with("attachment;"));
        assert!(map["Content-Security-Policy"].contains("sandbox"));
    }

    #[test]
    fn raw_content_filename_is_sanitized() {
        let headers = raw_content_headers("evil\"; drop\r\n/name");
        let map: std::collections::HashMap<_, _> = headers.into_iter().collect();
        let disp = &map["Content-Disposition"];
        assert!(!disp.contains('"') || disp.matches('"').count() == 2); // only the wrapping quotes
        assert!(!disp.contains('\r') && !disp.contains('\n'));
        assert!(!disp.contains('/'));
    }
}
