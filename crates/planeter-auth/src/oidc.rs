//! OIDC / OAuth 2.0 sign-in (RFC 002 T2, `STD-2`; owner-ruled 2026-09-24): the **authorization-code flow
//! with PKCE**, ID tokens verified against the provider's **JWKS** with `jsonwebtoken` (RustCrypto
//! backend — RS256/ES256/EdDSA, never `none`), every outbound request (discovery, JWKS, token exchange)
//! made through the [`HttpsFetcher`] — i.e. the egress guard + confined `curl`. Nothing here is
//! home-grown crypto.
//!
//! Flow: [`OidcProvider::begin_login`] mints `state`, `nonce` and a PKCE verifier, remembers them
//! **server-side** ([`PendingLogins`], ten minutes, one-shot) and returns the authorization URL;
//! [`OidcProvider::finish_login`] takes the callback's `code` + `state`, exchanges the code (with the
//! verifier; HTTP Basic client auth when a secret is configured), verifies the ID token's signature by
//! `kid` (refreshing the JWKS once on an unknown `kid`), issuer, audience, expiry and **nonce**, and
//! returns the [`VerifiedIdentity`]. Mapping a verified `(issuer, subject)` to a local account is the
//! account store's link table — **no auto-provisioning**: an unlinked subject is refused.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use planeter_core::{FetchRequest, HttpsFetcher};
use sha2::Digest as _;

use crate::session::random_token;

/// The identity a verified OIDC ID token asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdentity {
    pub issuer: String,
    /// The provider's stable subject identifier.
    pub subject: String,
    pub email: Option<String>,
}

/// Why OIDC verification failed. Coarse on purpose: a caller shows "sign-in failed", the log gets detail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OidcError {
    /// No OIDC provider is configured.
    NotConfigured,
    /// The token failed signature or claim validation (issuer, audience, expiry, nonce, kid).
    Invalid,
    /// The callback's `state` is unknown, expired, or already used.
    State,
    /// Provider discovery / JWKS / token endpoint could not be fetched or parsed.
    Provider(String),
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OidcError::NotConfigured => f.write_str("no OIDC provider configured"),
            OidcError::Invalid => f.write_str("invalid OIDC token"),
            OidcError::State => f.write_str("unknown or expired OIDC login state"),
            OidcError::Provider(e) => write!(f, "OIDC provider error: {e}"),
        }
    }
}

impl std::error::Error for OidcError {}

/// Verifies an OIDC ID token to a [`VerifiedIdentity`] (no nonce check — for tokens presented directly).
pub trait OidcVerifier: Send + Sync {
    fn verify(&self, id_token: &str) -> Result<VerifiedIdentity, OidcError>;
}

/// The default: no provider configured, so every token is refused.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredOidc;

impl OidcVerifier for UnconfiguredOidc {
    fn verify(&self, _id_token: &str) -> Result<VerifiedIdentity, OidcError> {
        Err(OidcError::NotConfigured)
    }
}

/// Provider configuration (from the operator's environment).
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// The issuer URL; discovery is at `<issuer>/.well-known/openid-configuration`.
    pub issuer: String,
    pub client_id: String,
    /// `None` for a public client (PKCE only).
    pub client_secret: Option<String>,
    /// planeter's callback URL as registered at the provider.
    pub redirect_uri: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Debug, serde::Deserialize)]
struct IdClaims {
    iss: String,
    sub: String,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// Pending logins keyed by `state` (server-side; the browser holds only the opaque `state`).
#[derive(Default)]
pub struct PendingLogins {
    inner: Mutex<HashMap<String, Pending>>,
}

#[derive(Debug, Clone)]
struct Pending {
    nonce: String,
    verifier: String,
    created_at: u64,
}

/// Pending-login lifetime.
pub const PENDING_TTL_SECS: u64 = 600;
const MAX_PENDING: usize = 10_000;

impl PendingLogins {
    fn insert(&self, state: String, p: Pending, now: u64) {
        if let Ok(mut m) = self.inner.lock() {
            m.retain(|_, v| now.saturating_sub(v.created_at) < PENDING_TTL_SECS);
            if m.len() < MAX_PENDING {
                m.insert(state, p);
            }
        }
    }
    /// One-shot: a state can be redeemed once.
    fn take(&self, state: &str, now: u64) -> Option<Pending> {
        let mut m = self.inner.lock().ok()?;
        let p = m.remove(state)?;
        (now.saturating_sub(p.created_at) < PENDING_TTL_SECS).then_some(p)
    }
}

/// The result of [`OidcProvider::begin_login`].
#[derive(Debug, Clone)]
pub struct BeginLogin {
    /// Where to send the browser.
    pub authorization_url: String,
    /// The opaque state the browser carries back (also set as a cookie by the web layer).
    pub state: String,
}

/// A configured OpenID Connect provider.
pub struct OidcProvider {
    cfg: OidcConfig,
    fetcher: Arc<dyn HttpsFetcher>,
    discovery: Mutex<Option<Discovery>>,
    jwks: Mutex<Option<JwkSet>>,
    pending: PendingLogins,
}

impl OidcProvider {
    pub fn new(cfg: OidcConfig, fetcher: Arc<dyn HttpsFetcher>) -> Self {
        Self {
            cfg,
            fetcher,
            discovery: Mutex::new(None),
            jwks: Mutex::new(None),
            pending: PendingLogins::default(),
        }
    }

    pub fn issuer(&self) -> &str {
        &self.cfg.issuer
    }

    fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        req: &FetchRequest,
    ) -> Result<T, OidcError> {
        let bytes = self
            .fetcher
            .fetch(req)
            .map_err(|e| OidcError::Provider(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| OidcError::Provider(format!("bad JSON: {e}")))
    }

    fn discovery(&self) -> Result<Discovery, OidcError> {
        if let Ok(g) = self.discovery.lock()
            && let Some(d) = g.as_ref()
        {
            return Ok(d.clone());
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.cfg.issuer.trim_end_matches('/')
        );
        let d: Discovery = self.fetch_json(&FetchRequest::get(url))?;
        // The document must describe the issuer we were configured with (RFC 8414 §3.3).
        if d.issuer.trim_end_matches('/') != self.cfg.issuer.trim_end_matches('/') {
            return Err(OidcError::Provider("discovery issuer mismatch".into()));
        }
        if let Ok(mut g) = self.discovery.lock() {
            *g = Some(d.clone());
        }
        Ok(d)
    }

    fn jwks(&self, refresh: bool) -> Result<JwkSet, OidcError> {
        if !refresh
            && let Ok(g) = self.jwks.lock()
            && let Some(k) = g.as_ref()
        {
            return Ok(k.clone());
        }
        let d = self.discovery()?;
        let set: JwkSet = self.fetch_json(&FetchRequest::get(d.jwks_uri))?;
        if let Ok(mut g) = self.jwks.lock() {
            *g = Some(set.clone());
        }
        Ok(set)
    }

    /// Start a login: mint state/nonce/PKCE, remember them, return the authorization URL.
    pub fn begin_login(&self, now_unix: u64) -> Result<BeginLogin, OidcError> {
        let d = self.discovery()?;
        let state = random_token();
        let nonce = random_token();
        let verifier = random_token();
        let challenge = {
            use base64ct::{Base64UrlUnpadded, Encoding as _};
            Base64UrlUnpadded::encode_string(&sha2::Sha256::digest(verifier.as_bytes()))
        };
        self.pending.insert(
            state.clone(),
            Pending {
                nonce: nonce.clone(),
                verifier,
                created_at: now_unix,
            },
            now_unix,
        );
        let sep = if d.authorization_endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        let authorization_url = format!(
            "{}{sep}response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
            d.authorization_endpoint,
            pct(&self.cfg.client_id),
            pct(&self.cfg.redirect_uri),
            pct("openid email profile"),
            pct(&state),
            pct(&nonce),
            pct(&challenge),
        );
        Ok(BeginLogin {
            authorization_url,
            state,
        })
    }

    /// Finish a login from the callback: redeem the state, exchange the code, verify the ID token.
    pub fn finish_login(
        &self,
        state: &str,
        code: &str,
        now_unix: u64,
    ) -> Result<VerifiedIdentity, OidcError> {
        let pending = self.pending.take(state, now_unix).ok_or(OidcError::State)?;
        let d = self.discovery()?;
        let mut form = format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            pct(code),
            pct(&self.cfg.redirect_uri),
            pct(&self.cfg.client_id),
            pct(&pending.verifier),
        );
        let authorization = match &self.cfg.client_secret {
            Some(secret) => {
                use base64ct::{Base64, Encoding as _};
                Some(format!(
                    "Basic {}",
                    Base64::encode_string(
                        format!("{}:{}", pct(&self.cfg.client_id), pct(secret)).as_bytes()
                    )
                ))
            }
            None => {
                form.push_str("");
                None
            }
        };
        let tok: TokenResponse = self.fetch_json(&FetchRequest {
            url: d.token_endpoint,
            form: Some(form),
            authorization,
        })?;
        self.verify_id_token(&tok.id_token, Some(&pending.nonce))
    }

    /// Verify an ID token: signature by `kid` against the JWKS (one refresh on an unknown `kid`),
    /// issuer, audience (= client id), expiry, and — when given — the nonce.
    pub fn verify_id_token(
        &self,
        token: &str,
        expected_nonce: Option<&str>,
    ) -> Result<VerifiedIdentity, OidcError> {
        let header = decode_header(token).map_err(|_| OidcError::Invalid)?;
        let alg = match header.alg {
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA => header.alg,
            _ => return Err(OidcError::Invalid),
        };
        let kid = header.kid.ok_or(OidcError::Invalid)?;
        let jwk = match self.jwks(false)?.find(&kid) {
            Some(j) => j.clone(),
            None => self
                .jwks(true)?
                .find(&kid)
                .cloned()
                .ok_or(OidcError::Invalid)?,
        };
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| OidcError::Invalid)?;
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[self.cfg.issuer.trim_end_matches('/'), &self.cfg.issuer]);
        validation.set_audience(&[self.cfg.client_id.as_str()]);
        validation.validate_exp = true;
        validation.leeway = 60;
        let data = decode::<IdClaims>(token, &key, &validation).map_err(|_| OidcError::Invalid)?;
        if let Some(expected) = expected_nonce {
            match data.claims.nonce.as_deref() {
                Some(n) if crate::session::tokens_match(n, expected) => {}
                _ => return Err(OidcError::Invalid),
            }
        }
        Ok(VerifiedIdentity {
            issuer: data.claims.iss,
            subject: data.claims.sub,
            email: data.claims.email,
        })
    }

    #[cfg(test)]
    fn peek_pending(&self, state: &str) -> Option<(String, String)> {
        self.pending
            .inner
            .lock()
            .ok()?
            .get(state)
            .map(|p| (p.nonce.clone(), p.verifier.clone()))
    }
}

impl OidcVerifier for OidcProvider {
    fn verify(&self, id_token: &str) -> Result<VerifiedIdentity, OidcError> {
        self.verify_id_token(id_token, None)
    }
}

/// Percent-encode a URL query value (unreserved characters pass through).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use planeter_core::FetchError;

    const ISSUER: &str = "https://idp.example";
    const CLIENT: &str = "planeter-client";
    // A throwaway P-256 key generated for these tests (PKCS#8) and its JWK coordinates.
    const TEST_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgopZeFiRGZXWvz9h4
G2WRp6WFSymfcUW7Kavsub8xSMehRANCAATQ+QQEj43jJBGpA4RYiR3l6Pnckdxw
BXaTdqRaPA7mOrNmL/VGO0eeJS9V/25SRXPdRAdaGWs75OxYcjsHqSZV
-----END PRIVATE KEY-----";
    const JWK_X: &str = "0PkEBI-N4yQRqQOEWIkd5ej53JHccAV2k3akWjwO5jo";
    const JWK_Y: &str = "s2Yv9UY7R54lL1X_blJFc91EB1oZazvk7FhyOwepJlU";

    /// A fetcher answering from a map; unknown URLs fail like the guard would.
    #[derive(Default)]
    struct MockFetcher {
        routes: Mutex<HashMap<String, Vec<u8>>>,
    }
    impl MockFetcher {
        fn set(&self, url: &str, body: impl Into<Vec<u8>>) {
            self.routes
                .lock()
                .unwrap()
                .insert(url.to_owned(), body.into());
        }
    }
    impl HttpsFetcher for MockFetcher {
        fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
            self.routes
                .lock()
                .unwrap()
                .get(&req.url)
                .cloned()
                .ok_or_else(|| FetchError::Transport(format!("no route for {}", req.url)))
        }
    }

    fn jwks_json(kid: &str) -> String {
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"{JWK_X}","y":"{JWK_Y}","kid":"{kid}","alg":"ES256","use":"sig"}}]}}"#
        )
    }

    fn provider(secret: Option<&str>) -> (OidcProvider, Arc<MockFetcher>) {
        let f = Arc::new(MockFetcher::default());
        f.set(
            "https://idp.example/.well-known/openid-configuration",
            format!(
                r#"{{"issuer":"{ISSUER}","authorization_endpoint":"https://idp.example/auth","token_endpoint":"https://idp.example/token","jwks_uri":"https://idp.example/jwks"}}"#
            ),
        );
        f.set("https://idp.example/jwks", jwks_json("k1"));
        let p = OidcProvider::new(
            OidcConfig {
                issuer: ISSUER.into(),
                client_id: CLIENT.into(),
                client_secret: secret.map(str::to_owned),
                redirect_uri: "https://forge.example/login/oidc/callback".into(),
            },
            f.clone(),
        );
        (p, f)
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn sign(kid: &str, claims: serde_json::Value) -> String {
        let mut h = Header::new(Algorithm::ES256);
        h.kid = Some(kid.into());
        encode(
            &h,
            &claims,
            &EncodingKey::from_ec_pem(TEST_KEY_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn claims(nonce: &str) -> serde_json::Value {
        serde_json::json!({
            "iss": ISSUER, "sub": "user-42", "aud": CLIENT,
            "exp": now() + 300, "iat": now(), "nonce": nonce, "email": "u@example"
        })
    }

    #[test]
    fn begin_login_builds_a_pkce_authorization_url_and_remembers_state() {
        let (p, _) = provider(None);
        let b = p.begin_login(1000).unwrap();
        assert!(
            b.authorization_url
                .starts_with("https://idp.example/auth?response_type=code")
        );
        assert!(b.authorization_url.contains("code_challenge_method=S256"));
        assert!(b.authorization_url.contains(&format!("state={}", b.state)));
        let (nonce, verifier) = p.peek_pending(&b.state).unwrap();
        assert!(b.authorization_url.contains(&format!("nonce={nonce}")));
        assert!(
            !b.authorization_url.contains(&verifier),
            "verifier never leaves the server"
        );
    }

    #[test]
    fn full_code_flow_verifies_the_id_token_and_nonce() {
        let (p, f) = provider(Some("s3cret"));
        let b = p.begin_login(1000).unwrap();
        let (nonce, _) = p.peek_pending(&b.state).unwrap();
        f.set(
            "https://idp.example/token",
            format!(
                r#"{{"id_token":"{}","access_token":"x"}}"#,
                sign("k1", claims(&nonce))
            ),
        );
        let id = p.finish_login(&b.state, "the-code", 1001).unwrap();
        assert_eq!(id.subject, "user-42");
        assert_eq!(id.issuer, ISSUER);
        assert_eq!(id.email.as_deref(), Some("u@example"));
        // State is one-shot.
        assert_eq!(
            p.finish_login(&b.state, "the-code", 1002).unwrap_err(),
            OidcError::State
        );
    }

    #[test]
    fn wrong_nonce_audience_issuer_or_expiry_is_refused() {
        let (p, f) = provider(None);
        let b = p.begin_login(1000).unwrap();
        let (nonce, _) = p.peek_pending(&b.state).unwrap();
        let mut bad_nonce = claims("other-nonce");
        bad_nonce["nonce"] = serde_json::Value::String("other".into());
        f.set(
            "https://idp.example/token",
            format!(r#"{{"id_token":"{}"}}"#, sign("k1", bad_nonce)),
        );
        assert_eq!(
            p.finish_login(&b.state, "c", 1001).unwrap_err(),
            OidcError::Invalid
        );

        let mut bad_aud = claims(&nonce);
        bad_aud["aud"] = serde_json::Value::String("someone-else".into());
        assert_eq!(
            p.verify_id_token(&sign("k1", bad_aud), None).unwrap_err(),
            OidcError::Invalid
        );
        let mut bad_iss = claims(&nonce);
        bad_iss["iss"] = serde_json::Value::String("https://evil.example".into());
        assert_eq!(
            p.verify_id_token(&sign("k1", bad_iss), None).unwrap_err(),
            OidcError::Invalid
        );
        let mut expired = claims(&nonce);
        expired["exp"] = serde_json::Value::from(now() - 3600);
        assert_eq!(
            p.verify_id_token(&sign("k1", expired), None).unwrap_err(),
            OidcError::Invalid
        );
    }

    #[test]
    fn unknown_kid_refreshes_jwks_once_then_refuses() {
        let (p, f) = provider(None);
        let (_, nonce) = ("", "n");
        // First JWKS has k1; the token is signed under k2 → refresh; still absent → Invalid.
        assert_eq!(
            p.verify_id_token(&sign("k2", claims(nonce)), None)
                .unwrap_err(),
            OidcError::Invalid
        );
        // Now the provider rotates to k2: the next verification refreshes and succeeds.
        f.set("https://idp.example/jwks", jwks_json("k2"));
        assert!(p.verify_id_token(&sign("k2", claims(nonce)), None).is_ok());
    }

    #[test]
    fn discovery_issuer_mismatch_is_refused() {
        let f = Arc::new(MockFetcher::default());
        f.set(
            "https://idp.example/.well-known/openid-configuration",
            r#"{"issuer":"https://other.example","authorization_endpoint":"a","token_endpoint":"t","jwks_uri":"j"}"#,
        );
        let p = OidcProvider::new(
            OidcConfig {
                issuer: ISSUER.into(),
                client_id: CLIENT.into(),
                client_secret: None,
                redirect_uri: "https://forge.example/cb".into(),
            },
            f,
        );
        assert!(matches!(
            p.begin_login(1).unwrap_err(),
            OidcError::Provider(_)
        ));
    }
}
