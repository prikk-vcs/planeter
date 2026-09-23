//! Outbound HTTPS fetches (owner ruling 2026-09-24): planeter drives **`curl` as a confined subprocess**
//! rather than linking an HTTPS client — the prikk/bubblewrap pattern, zero crypto crates in the tree.
//!
//! Every fetch goes through the [`crate::egress::EgressGuard`] first and connects only to the address
//! the guard pinned (`--resolve host:port:ip`), HTTPS only (`--proto =https`), with a byte ceiling
//! (`--max-filesize`), a wall-time bound, and **no redirects** (`--max-redirs 0` — a redirect would
//! need re-checking; the OIDC endpoints this serves never redirect). curl runs under bubblewrap with a
//! read-only system view, the CA bundle bound, network shared (that is the point), and no environment.
//! The [`HttpsFetcher`] trait lets callers be tested with a mock and keeps the subprocess in one place.

use std::ffi::OsString;
use std::process::Command;

use crate::egress::{EgressError, EgressGuard, ResolvedTarget};

/// A fetch failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError {
    /// The egress guard refused the destination.
    Egress(EgressError),
    /// curl could not be spawned or failed (exit status / stderr).
    Transport(String),
    /// The response exceeded the byte ceiling.
    TooLarge,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Egress(e) => write!(f, "egress refused: {e}"),
            FetchError::Transport(e) => write!(f, "fetch failed: {e}"),
            FetchError::TooLarge => f.write_str("fetch refused: response over the byte ceiling"),
        }
    }
}

impl std::error::Error for FetchError {}

/// An outbound HTTPS request. `form` is an `application/x-www-form-urlencoded` POST body; `None` = GET.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub url: String,
    pub form: Option<String>,
    /// Extra `Authorization` header value (e.g. `Basic …` for a confidential OIDC client), if any.
    pub authorization: Option<String>,
}

impl FetchRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            form: None,
            authorization: None,
        }
    }
}

/// Fetches an HTTPS resource and returns the body bytes. Implementations must route the destination
/// through the egress guard; callers treat the bytes as untrusted data.
pub trait HttpsFetcher: Send + Sync {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError>;
}

/// Bounds for one fetch.
#[derive(Debug, Clone, Copy)]
pub struct FetchLimits {
    pub max_bytes: u64,
    pub max_secs: u64,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            max_bytes: 1 << 20,
            max_secs: 15,
        }
    }
}

/// The production fetcher: egress guard → confined `curl`.
pub struct CurlFetcher {
    guard: Box<dyn EgressGuard>,
    limits: FetchLimits,
    curl: OsString,
    bwrap: Option<OsString>,
}

impl CurlFetcher {
    /// Confined by bubblewrap (production).
    pub fn new(guard: Box<dyn EgressGuard>) -> Self {
        Self {
            guard,
            limits: FetchLimits::default(),
            curl: OsString::from("curl"),
            bwrap: Some(OsString::from("bwrap")),
        }
    }

    /// **Dev only:** run curl directly, unconfined.
    pub fn unconfined(guard: Box<dyn EgressGuard>) -> Self {
        Self {
            bwrap: None,
            ..Self::new(guard)
        }
    }

    #[must_use]
    pub fn with_limits(mut self, limits: FetchLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The curl argument vector for a pinned target (pure; unit-tested).
    pub fn curl_args(
        target: &ResolvedTarget,
        req: &FetchRequest,
        limits: FetchLimits,
    ) -> Vec<String> {
        let pin = format!(
            "{}:{}:{}",
            target.host,
            target.port,
            target
                .addrs
                .iter()
                .map(|a| a.ip().to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut a = vec![
            "--silent".to_owned(),
            "--show-error".to_owned(),
            "--fail".to_owned(),
            "--proto".to_owned(),
            "=https".to_owned(),
            "--max-redirs".to_owned(),
            "0".to_owned(),
            "--max-time".to_owned(),
            limits.max_secs.to_string(),
            "--max-filesize".to_owned(),
            limits.max_bytes.to_string(),
            "--resolve".to_owned(),
            pin,
            "-H".to_owned(),
            "Accept: application/json".to_owned(),
            "-H".to_owned(),
            "User-Agent: planeter".to_owned(),
        ];
        if let Some(auth) = &req.authorization {
            a.push("-H".to_owned());
            a.push(format!("Authorization: {auth}"));
        }
        if let Some(form) = &req.form {
            a.push("-H".to_owned());
            a.push("Content-Type: application/x-www-form-urlencoded".to_owned());
            a.push("--data-binary".to_owned());
            a.push(form.clone());
        }
        a.push("--".to_owned());
        a.push(req.url.clone());
        a
    }

    fn command(&self, args: &[String]) -> Command {
        match &self.bwrap {
            None => {
                let mut c = Command::new(&self.curl);
                c.args(args);
                c
            }
            Some(bwrap) => {
                // A read-only system view, the CA bundle, network shared, nothing else.
                let mut c = Command::new(bwrap);
                c.arg("--ro-bind").arg("/usr").arg("/usr");
                for (target, link) in [
                    ("usr/lib", "/lib"),
                    ("usr/lib", "/lib64"),
                    ("usr/bin", "/bin"),
                    ("usr/bin", "/sbin"),
                ] {
                    c.arg("--symlink").arg(target).arg(link);
                }
                for ca in [
                    "/etc/ssl",
                    "/etc/ca-certificates",
                    "/etc/pki",
                    "/etc/resolv.conf",
                ] {
                    if std::path::Path::new(ca).exists() {
                        c.arg("--ro-bind").arg(ca).arg(ca);
                    }
                }
                c.arg("--proc").arg("/proc");
                c.arg("--dev").arg("/dev");
                c.arg("--tmpfs").arg("/tmp");
                c.arg("--unshare-all").arg("--share-net");
                c.arg("--die-with-parent").arg("--new-session");
                c.arg("--clearenv")
                    .arg("--setenv")
                    .arg("PATH")
                    .arg("/usr/bin");
                c.arg("--").arg(&self.curl).args(args);
                c
            }
        }
    }
}

impl HttpsFetcher for CurlFetcher {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        let target = self.guard.check(&req.url).map_err(FetchError::Egress)?;
        let args = Self::curl_args(&target, req, self.limits);
        let out = self
            .command(&args)
            .output()
            .map_err(|e| FetchError::Transport(format!("spawn: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).into_owned();
            // curl exit 63 = --max-filesize exceeded.
            if out.status.code() == Some(63) {
                return Err(FetchError::TooLarge);
            }
            return Err(FetchError::Transport(format!(
                "curl exit {:?}: {}",
                out.status.code(),
                err.trim()
            )));
        }
        if out.stdout.len() as u64 > self.limits.max_bytes {
            return Err(FetchError::TooLarge);
        }
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ResolvedTarget {
        ResolvedTarget {
            url: "https://idp.example/.well-known/openid-configuration".into(),
            scheme: "https".into(),
            host: "idp.example".into(),
            port: 443,
            addrs: vec!["93.184.216.34:443".parse().unwrap()],
        }
    }

    #[test]
    fn get_args_pin_https_no_redirects_bounded() {
        let a = CurlFetcher::curl_args(
            &target(),
            &FetchRequest::get("https://idp.example/.well-known/openid-configuration"),
            FetchLimits {
                max_bytes: 4096,
                max_secs: 7,
            },
        );
        let s = a.join(" ");
        assert!(s.contains("--proto =https"));
        assert!(s.contains("--max-redirs 0"));
        assert!(s.contains("--max-time 7"));
        assert!(s.contains("--max-filesize 4096"));
        assert!(s.contains("--resolve idp.example:443:93.184.216.34"));
        assert!(!s.contains("--data-binary"));
        assert_eq!(
            a.last().unwrap(),
            "https://idp.example/.well-known/openid-configuration"
        );
    }

    #[test]
    fn form_post_and_authorization_are_included() {
        let a = CurlFetcher::curl_args(
            &target(),
            &FetchRequest {
                url: "https://idp.example/token".into(),
                form: Some("grant_type=authorization_code&code=x".into()),
                authorization: Some("Basic abc".into()),
            },
            FetchLimits::default(),
        );
        let s = a.join(" ");
        assert!(s.contains("Content-Type: application/x-www-form-urlencoded"));
        assert!(s.contains("--data-binary grant_type=authorization_code&code=x"));
        assert!(s.contains("Authorization: Basic abc"));
    }

    #[test]
    fn egress_refusal_is_a_fetch_error_before_any_spawn() {
        let f = CurlFetcher::unconfined(Box::new(crate::egress::DenyAllEgress));
        assert!(matches!(
            f.fetch(&FetchRequest::get("https://idp.example/x")),
            Err(FetchError::Egress(_))
        ));
    }
}
