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
#[derive(Debug, Clone)]
pub struct ContentOrigin(pub String);

impl ContentOrigin {
    pub fn new(origin: impl Into<String>) -> Self {
        Self(origin.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
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
