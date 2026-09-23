//! The egress guard (RFC 001 D-6 seam, filled 2026-09-23; threat model T-8 / C-8, LAY-5).
//!
//! Any outbound fetch the forge makes on behalf of a caller — webhooks, OIDC JWKS/discovery, repository
//! mirrors, import sources, avatar/URL previews — is an SSRF vector: a caller-supplied URL can point at
//! loopback, link-local (cloud metadata), or private ranges. Every outbound path clears its URL through
//! **one** [`EgressGuard`] before connecting, and connects only to the **pinned** addresses the guard
//! resolved, so DNS cannot rebind between check and connect.
//!
//! The rules ([`StdEgressGuard`]):
//! - scheme must be `https` (or `http` only if the policy allows plain HTTP);
//! - the host — a literal IP or a DNS name — must resolve, and **every** resolved address must be a
//!   public unicast address: loopback, link-local (incl. `169.254.169.254`), private (`10/8`,
//!   `172.16/12`, `192.168/16`, CGNAT `100.64/10`), unspecified, multicast, reserved, and their IPv6
//!   counterparts (`::1`, `fe80::/10`, `fc00::/7`, `ff00::/8`, IPv4-mapped forms, NAT64) are refused —
//!   unless the policy explicitly allows private targets (a self-hosted webhook receiver, say);
//! - userinfo in the URL is refused (a classic confusion vector);
//! - **redirects are re-checked**: a caller following `3xx` runs the new location through the guard
//!   again before connecting.
//!
//! Resolution uses the OS resolver synchronously; call it off the async runtime (`spawn_blocking`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

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
    /// The host did not resolve to any address.
    Unresolvable(String),
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
            EgressError::Unresolvable(h) => write!(f, "outbound host did not resolve: {h}"),
            EgressError::NotConfigured => {
                f.write_str("no egress policy configured; outbound denied")
            }
        }
    }
}

impl std::error::Error for EgressError {}

/// An admitted destination: the URL as parsed and the **pinned** addresses the caller must connect to
/// (never re-resolving the name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub url: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub addrs: Vec<SocketAddr>,
}

/// The single outbound-fetch admission point. A caller passes a candidate URL; the guard returns the
/// pinned target only if the SSRF policy permits it. It authorizes the *destination* — it does not
/// itself perform the fetch (the caller does, to `addrs`, and re-checks every redirect).
pub trait EgressGuard: Send + Sync {
    fn check(&self, url: &str) -> Result<ResolvedTarget, EgressError>;
}

/// The A0 default: **refuse every outbound URL** (no policy configured). Correct failure direction —
/// the forge makes no outbound request until a real guard is installed.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllEgress;

impl EgressGuard for DenyAllEgress {
    fn check(&self, _url: &str) -> Result<ResolvedTarget, EgressError> {
        Err(EgressError::NotConfigured)
    }
}

/// Egress policy knobs. Defaults are the safe ones: HTTPS only, public addresses only.
#[derive(Debug, Clone, Default)]
pub struct EgressPolicy {
    /// Permit plain `http://` targets (default: refused).
    pub allow_plain_http: bool,
    /// Permit private/loopback/link-local targets (default: refused). For deployments whose webhook
    /// receivers live on an internal network — an explicit, logged opt-in, never a default.
    pub allow_private_targets: bool,
}

/// The production guard: parse, policy, resolve, classify every address, pin.
#[derive(Debug, Clone, Default)]
pub struct StdEgressGuard {
    policy: EgressPolicy,
}

impl StdEgressGuard {
    pub fn new(policy: EgressPolicy) -> Self {
        Self { policy }
    }

    /// Resolve `host` (literal IP or DNS name) to socket addresses via the OS resolver.
    fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, EgressError> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let addrs: Vec<SocketAddr> = (host, port)
            .to_socket_addrs()
            .map_err(|_| EgressError::Unresolvable(host.to_owned()))?
            .collect();
        if addrs.is_empty() {
            return Err(EgressError::Unresolvable(host.to_owned()));
        }
        Ok(addrs)
    }
}

impl EgressGuard for StdEgressGuard {
    fn check(&self, url: &str) -> Result<ResolvedTarget, EgressError> {
        let parsed = url::Url::parse(url).map_err(|_| EgressError::Unparsable(url.to_owned()))?;
        let scheme = parsed.scheme().to_ascii_lowercase();
        match scheme.as_str() {
            "https" => {}
            "http" if self.policy.allow_plain_http => {}
            other => return Err(EgressError::DisallowedScheme(other.to_owned())),
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(EgressError::Unparsable("userinfo in URL".to_owned()));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| EgressError::Unparsable("no host".to_owned()))?
            .trim_matches(|c| c == '[' || c == ']')
            .to_owned();
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| EgressError::Unparsable("no port".to_owned()))?;
        let addrs = Self::resolve(&host, port)?;
        if !self.policy.allow_private_targets {
            // All-or-nothing: one blocked address among the answers refuses the whole target.
            if let Some(bad) = addrs.iter().find(|a| !is_public_unicast(a.ip())) {
                return Err(EgressError::BlockedAddress(format!(
                    "{host} -> {}",
                    bad.ip()
                )));
            }
        }
        Ok(ResolvedTarget {
            url: url.to_owned(),
            scheme,
            host,
            port,
            addrs,
        })
    }
}

/// Is `ip` a globally routable unicast address? Conservative: everything not clearly public is refused.
pub fn is_public_unicast(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            if v6 != Ipv6Addr::UNSPECIFIED
                && v6 != Ipv6Addr::LOCALHOST
                && let Some(v4) = v6.to_ipv4()
            {
                // Deprecated IPv4-compatible form (::a.b.c.d): judge the embedded v4.
                return is_public_v4(v4);
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xffc0) == 0xfe80
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfec0
                || (seg[0] == 0x2001 && seg[1] == 0x0db8)
                || (seg[0] == 0x0064 && seg[1] == 0xff9b))
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    !(v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
        || o[0] >= 240)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> StdEgressGuard {
        StdEgressGuard::default()
    }

    #[test]
    fn default_guard_refuses_all_egress() {
        assert_eq!(
            DenyAllEgress.check("https://example.com/hook").unwrap_err(),
            EgressError::NotConfigured
        );
    }

    #[test]
    fn scheme_policy() {
        assert!(matches!(
            guard().check("http://93.184.216.34/"),
            Err(EgressError::DisallowedScheme(_))
        ));
        assert!(matches!(
            guard().check("ftp://93.184.216.34/"),
            Err(EgressError::DisallowedScheme(_))
        ));
        let g = StdEgressGuard::new(EgressPolicy {
            allow_plain_http: true,
            ..Default::default()
        });
        assert!(g.check("http://93.184.216.34/").is_ok());
    }

    #[test]
    fn blocked_literal_addresses() {
        for u in [
            "https://127.0.0.1/",
            "https://127.1.2.3/",
            "https://10.0.0.5/",
            "https://172.16.9.9/",
            "https://192.168.1.1/",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.1.1/",
            "https://0.0.0.0/",
            "https://[::1]/",
            "https://[fe80::1]/",
            "https://[fc00::1]/",
            "https://[fd12::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[::ffff:10.0.0.1]/",
            "https://[64:ff9b::7f00:1]/",
            "https://224.0.0.1/",
        ] {
            assert!(
                matches!(guard().check(u), Err(EgressError::BlockedAddress(_))),
                "should block {u}"
            );
        }
    }

    #[test]
    fn public_literal_is_pinned() {
        let t = guard().check("https://93.184.216.34:8443/x?y=1").unwrap();
        assert_eq!(t.host, "93.184.216.34");
        assert_eq!(t.port, 8443);
        assert_eq!(t.addrs, vec!["93.184.216.34:8443".parse().unwrap()]);
        let t6 = guard().check("https://[2606:4700::1111]/").unwrap();
        assert_eq!(t6.port, 443);
        assert_eq!(t6.addrs.len(), 1);
    }

    #[test]
    fn localhost_name_resolves_to_loopback_and_is_blocked() {
        assert!(matches!(
            guard().check("https://localhost/"),
            Err(EgressError::BlockedAddress(_))
        ));
    }

    #[test]
    fn userinfo_and_garbage_are_unparsable() {
        assert!(matches!(
            guard().check("https://user:pw@93.184.216.34/"),
            Err(EgressError::Unparsable(_))
        ));
        assert!(matches!(
            guard().check("not a url"),
            Err(EgressError::Unparsable(_))
        ));
    }

    #[test]
    fn private_opt_in_allows_internal_targets() {
        let g = StdEgressGuard::new(EgressPolicy {
            allow_private_targets: true,
            ..Default::default()
        });
        assert!(g.check("https://10.0.0.5/hook").is_ok());
    }
}
