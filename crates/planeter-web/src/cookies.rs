//! Minimal cookie handling (no crate): read one cookie from a `Cookie` header, and format `Set-Cookie`
//! values that are `HttpOnly`, `SameSite=Strict`, `Secure`, `Path=/` — the browser-session posture
//! WEB-05/`STD-6` requires. TLS is proxy-terminated (owner ruling 2026-09-23), so `Secure` is always set;
//! browsers treat `localhost` as a secure context for development.

use axum::http::HeaderMap;

pub const SESSION_COOKIE: &str = "planeter_session";
pub const CSRF_COOKIE: &str = "planeter_csrf";

/// The value of cookie `name` from the request's `Cookie` header(s), if present.
pub fn get_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name).then(|| v.to_owned())
        })
        .next()
}

/// A `Set-Cookie` value for a session-style cookie with a lifetime in seconds.
pub fn set_cookie(name: &str, value: &str, max_age_secs: u64) -> String {
    format!("{name}={value}; Path=/; Max-Age={max_age_secs}; HttpOnly; Secure; SameSite=Strict")
}

/// A `Set-Cookie` value that clears `name`.
pub fn clear_cookie(name: &str) -> String {
    format!("{name}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::COOKIE;

    #[test]
    fn reads_the_named_cookie_among_several() {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, "a=1; planeter_session=abc; b=2".parse().unwrap());
        assert_eq!(get_cookie(&h, SESSION_COOKIE).as_deref(), Some("abc"));
        assert_eq!(get_cookie(&h, "zzz"), None);
    }

    #[test]
    fn set_cookie_is_httponly_secure_strict() {
        let c = set_cookie(SESSION_COOKIE, "x", 60);
        assert!(c.contains("HttpOnly") && c.contains("Secure") && c.contains("SameSite=Strict"));
        assert!(clear_cookie(SESSION_COOKIE).contains("Max-Age=0"));
    }
}
