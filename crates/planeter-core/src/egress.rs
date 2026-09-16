//! The egress-guard seam (RFC 001 D-6, threat model LAY-5).
//!
//! Any outbound fetch the forge makes on behalf of a caller — webhooks (RFC 004/005), remote mirror
//! pulls, avatar/URL previews — is an SSRF risk: a caller-supplied URL could point at internal
//! metadata endpoints, loopback, or link-local addresses. A0 ships the *chokepoint*: one
//! [`EgressGuard`] trait every future outbound path must clear a URL through, plus a **deny-by-default**
//! stub. No callers exist yet; the guard is here so RFC 004/webhook code has exactly one place to route
//! through rather than each surface reinventing the filter.
//!
//! The contract (filled by a later RFC): resolve the host, and **refuse** loopback (`127.0.0.0/8`,
//! `::1`), link-local (`169.254.0.0/16`, `fe80::/10`, including the cloud metadata address
//! `169.254.169.254`), and private ranges (`10/8`, `172.16/12`, `192.168/16`, `fc00::/7`) unless an
//! explicit allowlist opts them in; re-check after redirects; and pin the resolved address so DNS cannot
//! rebind between check and connect.

/// Why an outbound URL was refused (or could not be evaluated).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EgressError {
    /// The scheme is not permitted (only `https`, and `http` where explicitly allowed).
    DisallowedScheme(String),
    /// The URL could not be parsed into a host to check.
    Unparsable(String),
    /// The target resolves to a blocked address (loopback/link-local/private) and is not allowlisted.
    BlockedAddress(String),
    /// No egress policy is configured, so the guard refuses (the A0 default).
    NotConfigured,
}

impl std::fmt::Display for EgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EgressError::DisallowedScheme(s) => write!(f, "disallowed URL scheme: {s}"),
            EgressError::Unparsable(u) => write!(f, "unparsable outbound URL: {u}"),
            EgressError::BlockedAddress(a) => {
                write!(f, "outbound address blocked (SSRF guard): {a}")
            }
            EgressError::NotConfigured => {
                f.write_str("no egress policy configured; outbound denied")
            }
        }
    }
}

impl std::error::Error for EgressError {}

/// The single outbound-fetch admission point. A caller passes a candidate URL; the guard returns `Ok`
/// only for a target the SSRF policy permits. It authorizes the *destination* — it does not itself
/// perform the fetch (the caller does, after clearing the guard, honouring the pinned address).
pub trait EgressGuard: Send + Sync {
    /// Admit or refuse an outbound URL. `Ok(())` means the destination is permitted.
    fn check(&self, url: &str) -> Result<(), EgressError>;
}

/// The A0 default: **refuse every outbound URL** (no policy configured). Correct failure direction —
/// the forge makes no outbound request until a real guard is installed (RFC 004). Never ship this in a
/// deployment that needs webhooks/mirrors.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllEgress;

impl EgressGuard for DenyAllEgress {
    fn check(&self, _url: &str) -> Result<(), EgressError> {
        Err(EgressError::NotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_guard_refuses_all_egress() {
        let guard = DenyAllEgress;
        assert_eq!(
            guard.check("https://example.com/hook"),
            Err(EgressError::NotConfigured)
        );
        assert_eq!(
            guard.check("http://169.254.169.254/latest/meta-data/"),
            Err(EgressError::NotConfigured)
        );
    }
}
