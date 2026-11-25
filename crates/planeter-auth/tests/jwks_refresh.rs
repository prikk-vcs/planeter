//! RFC 014 ID-7: **the JWKS refresh is `no-cache`**. A provider may publish a new key only when it becomes the signer, and its key set is
//! served `public, max-age=300`: a shared cache in front of it could answer planeter's one refresh (after an unknown `kid`) with the stale
//! copy, and a sign-in right after a rotation would fail. The refresh therefore asks every cache for a fresh answer; the first, ordinary
//! fetch does not. Shown here with a stand-in provider that rotates and a caching stand-in in front of it, **with the fix (the sign-in
//! succeeds) and without it (it fails)**.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use planeter_auth::{OidcConfig, OidcError, OidcProvider};
use planeter_core::{FetchError, FetchRequest, HttpsFetcher};

const ISSUER: &str = "https://idp.example";
const CLIENT: &str = "planeter-client";

fn keys() -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oidc-alg-keys.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

fn jwk(keys: &serde_json::Value, name: &str, kid: &str) -> serde_json::Value {
    let mut j = keys[name]["jwk"].clone();
    j["kid"] = serde_json::json!(kid);
    j["use"] = serde_json::json!("sig");
    j
}

/// The provider: what it publishes now (it changes at the rotation) and a log of what it was asked, with the `no_cache` flag of each request.
struct Provider {
    jwks: Mutex<String>,
    asked: Mutex<Vec<(String, bool)>>,
}

/// A shared cache in front of the provider: it keeps the first answer of an URL and gives it back to every request that does not say
/// `no-cache` (the provider's `max-age=300`, which never runs out within this test); a request that does say it goes to the provider and
/// replaces the copy.
struct Cache {
    provider: Arc<Provider>,
    copies: Mutex<HashMap<String, Vec<u8>>>,
    /// Stand-in for the code before this fix: the flag is not passed on.
    strip_no_cache: bool,
}

impl HttpsFetcher for Cache {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        let no_cache = req.no_cache && !self.strip_no_cache;
        if !no_cache && let Some(copy) = self.copies.lock().unwrap().get(&req.url) {
            return Ok(copy.clone());
        }
        // Reaches the provider.
        self.provider
            .asked
            .lock()
            .unwrap()
            .push((req.url.clone(), req.no_cache));
        let body = if req.url.ends_with("/.well-known/openid-configuration") {
            format!(r#"{{"issuer":"{ISSUER}","authorization_endpoint":"{ISSUER}/auth","token_endpoint":"{ISSUER}/token","jwks_uri":"{ISSUER}/jwks"}}"#)
        } else if req.url == format!("{ISSUER}/jwks") {
            self.provider.jwks.lock().unwrap().clone()
        } else {
            return Err(FetchError::Transport(format!("no route {}", req.url)));
        }
        .into_bytes();
        self.copies
            .lock()
            .unwrap()
            .insert(req.url.clone(), body.clone());
        Ok(body)
    }
}

fn token(alg: Algorithm, kid: &str, key: &EncodingKey) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut h = Header::new(alg);
    h.kid = Some(kid.into());
    encode(&h, &serde_json::json!({"iss": ISSUER, "sub": "user-42", "aud": CLIENT, "exp": now + 300, "iat": now}), key).unwrap()
}

/// The rotation: sign in with the old key (the provider's JWKS is read and cached), the provider then starts signing with a new key and
/// publishes it, and the next sign-in arrives with a token under the new `kid`.
fn rotation(strip_no_cache: bool) -> (Result<(), OidcError>, Vec<(String, bool)>) {
    let k = keys();
    let old = EncodingKey::from_ec_pem(k["p256"]["pem"].as_str().unwrap().as_bytes()).unwrap();
    let new = EncodingKey::from_ed_pem(k["ed"]["pem"].as_str().unwrap().as_bytes()).unwrap();
    let provider = Arc::new(Provider {
        jwks: Mutex::new(serde_json::json!({"keys": [jwk(&k, "p256", "k-old")]}).to_string()),
        asked: Mutex::new(Vec::new()),
    });
    let cache = Arc::new(Cache {
        provider: provider.clone(),
        copies: Mutex::new(HashMap::new()),
        strip_no_cache,
    });
    let p = OidcProvider::new(
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: CLIENT.into(),
            client_secret: None,
            redirect_uri: "https://forge.example/cb".into(),
        },
        cache,
    );
    p.verify_id_token(&token(Algorithm::ES256, "k-old", &old), None)
        .expect("the old key works before the rotation");
    // The rotation: only now does the provider publish the new key.
    *provider.jwks.lock().unwrap() =
        serde_json::json!({"keys": [jwk(&k, "p256", "k-old"), jwk(&k, "ed", "k-new")]}).to_string();
    let after = p
        .verify_id_token(&token(Algorithm::EdDSA, "k-new", &new), None)
        .map(|_| ());
    let asked = provider.asked.lock().unwrap().clone();
    (after, asked)
}

#[test]
fn a_sign_in_right_after_a_rotation_succeeds_because_the_refresh_asks_for_a_fresh_key_set() {
    let (after, asked) = rotation(false);
    assert_eq!(after, Ok(()), "the new key is found");
    // What reached the provider: discovery and the first key-set fetch (neither says no-cache), then the one refresh (which does).
    let jwks: Vec<bool> = asked
        .iter()
        .filter(|(u, _)| u.ends_with("/jwks"))
        .map(|(_, n)| *n)
        .collect();
    assert_eq!(jwks, vec![false, true], "{asked:?}");
    assert!(
        asked
            .iter()
            .filter(|(u, _)| !u.ends_with("/jwks"))
            .all(|(_, n)| !*n),
        "discovery is an ordinary fetch: {asked:?}"
    );
}

#[test]
fn without_the_header_the_same_rotation_fails_the_sign_in() {
    // The code as it was before the fix (the refresh is a plain fetch): the shared cache answers it with the stale set and the new `kid` is unknown.
    let (after, asked) = rotation(true);
    assert_eq!(
        after,
        Err(OidcError::Invalid),
        "the stale key set has no k-new"
    );
    let jwks_reaching_the_provider = asked.iter().filter(|(u, _)| u.ends_with("/jwks")).count();
    assert_eq!(
        jwks_reaching_the_provider, 1,
        "the refresh never reached the provider: {asked:?}"
    );
}

#[test]
fn an_ordinary_sign_in_with_a_known_key_makes_no_refresh() {
    let k = keys();
    let old = EncodingKey::from_ec_pem(k["p256"]["pem"].as_str().unwrap().as_bytes()).unwrap();
    let provider = Arc::new(Provider {
        jwks: Mutex::new(serde_json::json!({"keys": [jwk(&k, "p256", "k-old")]}).to_string()),
        asked: Mutex::new(Vec::new()),
    });
    let cache = Arc::new(Cache {
        provider: provider.clone(),
        copies: Mutex::new(HashMap::new()),
        strip_no_cache: false,
    });
    let p = OidcProvider::new(
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: CLIENT.into(),
            client_secret: None,
            redirect_uri: "https://forge.example/cb".into(),
        },
        cache,
    );
    for _ in 0..3 {
        p.verify_id_token(&token(Algorithm::ES256, "k-old", &old), None)
            .unwrap();
    }
    assert!(
        provider
            .asked
            .lock()
            .unwrap()
            .iter()
            .all(|(_, no_cache)| !*no_cache),
        "no request asked for no-cache"
    );
}
