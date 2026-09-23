//! Client IP behind a TLS-terminating reverse proxy (owner ruling 2026-09-23: TLS is proxy-terminated),
//! and the bind rule that follows from it.
//!
//! - [`TrustedProxies`] is the operator's list of proxy addresses/CIDRs (`PLANETER_TRUSTED_PROXIES`).
//! - [`client_ip`] derives the real client address: if the TCP peer is a trusted proxy, walk
//!   `X-Forwarded-For` **from the right**, skipping trusted hops, and take the first untrusted address
//!   (the one the outermost trusted proxy saw). If the peer is *not* trusted, `X-Forwarded-For` is
//!   **ignored** — anyone can send that header. With no trusted proxies configured, the peer is the client.
//! - [`bind_allowed`] refuses a **non-loopback** bind unless trusted proxies are configured: planeter
//!   speaks plain HTTP, so exposing it directly would expose plaintext and cookies marked `Secure`
//!   would never be sent. Loopback binds (behind a proxy on the same host) are always allowed.
//!
//! The derived address feeds per-IP login throttling (threat model C-2b / RR-7) and, later, transport.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;

/// An address or CIDR block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Net {
    addr: IpAddr,
    prefix: u8,
}

impl Net {
    /// Parse `ip` or `ip/prefix`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, Some(p.parse::<u8>().ok()?)),
            None => (s, None),
        };
        let addr: IpAddr = ip.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(Self { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => {
                let (n, a) = (u32::from(n), u32::from(a));
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - self.prefix)
                };
                n & mask == a & mask
            }
            (IpAddr::V6(n), IpAddr::V6(a)) => {
                let (n, a) = (u128::from(n), u128::from(a));
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - self.prefix)
                };
                n & mask == a & mask
            }
            (IpAddr::V4(_), IpAddr::V6(a)) => a
                .to_ipv4_mapped()
                .is_some_and(|a4| self.contains(IpAddr::V4(a4))),
            (IpAddr::V6(_), IpAddr::V4(_)) => false,
        }
    }
}

/// The operator's trusted reverse proxies.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    nets: Vec<Net>,
}

impl TrustedProxies {
    /// Parse a comma-separated list of addresses/CIDRs. `Err` names the first bad entry.
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut nets = Vec::new();
        for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            nets.push(Net::parse(item).ok_or_else(|| format!("bad trusted proxy entry {item:?}"))?);
        }
        Ok(Self { nets })
    }

    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        self.nets.iter().any(|n| n.contains(ip))
    }
}

/// The real client address for this request, per the module rules. `None` when the peer is unknown
/// (e.g. a test driving the router without a socket) and no trusted proxy chain applies.
pub fn client_ip(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    trusted: &TrustedProxies,
) -> Option<IpAddr> {
    let peer = peer.map(normalize);
    let p = peer?;
    if !trusted.is_trusted(p) {
        return Some(p);
    }
    // Walk X-Forwarded-For from the right: the rightmost entry was added by the outermost proxy.
    let mut chain: Vec<IpAddr> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(','))
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .map(normalize)
        .collect();
    while let Some(ip) = chain.pop() {
        if !trusted.is_trusted(ip) {
            return Some(ip);
        }
    }
    Some(p)
}

fn normalize(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// The bind rule: loopback is always fine; a non-loopback bind needs trusted proxies configured.
pub fn bind_allowed(addr: SocketAddr, trusted: &TrustedProxies) -> Result<(), String> {
    let ip = normalize(addr.ip());
    let loopback = match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6 == Ipv6Addr::LOCALHOST,
    };
    if loopback || !trusted.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "refusing to bind {addr}: planeter speaks plain HTTP behind a TLS-terminating proxy. Bind a \
             loopback address, or set PLANETER_TRUSTED_PROXIES to the proxy's address(es)/CIDRs."
        ))
    }
}

/// The TCP peer address, if the server was started with connect-info (tests driving the router
/// directly have none).
#[derive(Debug, Clone, Copy)]
pub struct PeerAddr(pub Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerAddr {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(PeerAddr(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    fn hdr(xff: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", xff.parse().unwrap());
        h
    }

    #[test]
    fn cidr_matching() {
        let n = Net::parse("10.0.0.0/8").unwrap();
        assert!(n.contains(ip("10.9.8.7")));
        assert!(!n.contains(ip("11.0.0.1")));
        assert!(
            Net::parse("192.168.1.5")
                .unwrap()
                .contains(ip("192.168.1.5"))
        );
        assert!(
            !Net::parse("192.168.1.5")
                .unwrap()
                .contains(ip("192.168.1.6"))
        );
        assert!(Net::parse("fd00::/8").unwrap().contains(ip("fd12::1")));
        assert!(
            Net::parse("10.0.0.0/8")
                .unwrap()
                .contains(ip("::ffff:10.1.1.1"))
        );
        assert!(Net::parse("10.0.0.0/33").is_none());
        assert!(Net::parse("nope").is_none());
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_for() {
        let t = TrustedProxies::parse("10.0.0.1").unwrap();
        let h = hdr("203.0.113.9");
        assert_eq!(
            client_ip(Some(ip("198.51.100.4")), &h, &t),
            Some(ip("198.51.100.4"))
        );
    }

    #[test]
    fn trusted_peer_walks_forwarded_for_from_the_right_past_trusted_hops() {
        let t = TrustedProxies::parse("10.0.0.0/8, 172.16.0.1").unwrap();
        // client → 172.16.0.1 (trusted) → 10.0.0.5 (trusted, the peer)
        let h = hdr("203.0.113.9, 172.16.0.1");
        assert_eq!(
            client_ip(Some(ip("10.0.0.5")), &h, &t),
            Some(ip("203.0.113.9"))
        );
        // A spoofed inner entry is irrelevant: the rightmost untrusted wins.
        let h = hdr("1.1.1.1, 203.0.113.9, 172.16.0.1");
        assert_eq!(
            client_ip(Some(ip("10.0.0.5")), &h, &t),
            Some(ip("203.0.113.9"))
        );
        // No header at all from a trusted peer → the peer.
        assert_eq!(
            client_ip(Some(ip("10.0.0.5")), &HeaderMap::new(), &t),
            Some(ip("10.0.0.5"))
        );
        // Unknown peer → unknown client (never trust the header alone).
        assert_eq!(client_ip(None, &h, &t), None);
    }

    #[test]
    fn bind_rule() {
        let none = TrustedProxies::default();
        assert!(bind_allowed("127.0.0.1:8080".parse().unwrap(), &none).is_ok());
        assert!(bind_allowed("[::1]:8080".parse().unwrap(), &none).is_ok());
        assert!(bind_allowed("0.0.0.0:8080".parse().unwrap(), &none).is_err());
        assert!(bind_allowed("192.168.1.10:8080".parse().unwrap(), &none).is_err());
        let some = TrustedProxies::parse("10.0.0.1").unwrap();
        assert!(bind_allowed("0.0.0.0:8080".parse().unwrap(), &some).is_ok());
    }
}
