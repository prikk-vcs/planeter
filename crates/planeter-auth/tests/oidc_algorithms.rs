//! The OIDC verifier's algorithm matrix (D-50): every ID-token algorithm the verifier names, signed with a
//! key of that kind and verified through the real `OidcProvider::verify_id_token`, each with a tampered
//! payload and a tampered signature; a key of the wrong kind; the algorithms it must refuse (HS*, PS*,
//! `none`); and the claims (audience, expiry). 30 rows.
//!
//! The test pins **behaviour, not the backend**: it passes on `jsonwebtoken`'s `rust_crypto` and its
//! `aws_lc_rs` backend alike, so the result of any later change of backend is judged by the same rows.
//! The keys in `tests/fixtures/oidc-alg-keys.json` are throwaway test keys (see the file's `_comment`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64ct::{Base64UrlUnpadded, Encoding};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use planeter_auth::{OidcConfig, OidcError, OidcProvider};
use planeter_core::{FetchError, FetchRequest, HttpsFetcher};

const ISSUER: &str = "https://idp.example";
const CLIENT: &str = "planeter-client";

#[derive(Default)]
struct Mock(Mutex<HashMap<String, Vec<u8>>>);
impl HttpsFetcher for Mock {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        self.0
            .lock()
            .unwrap()
            .get(&req.url)
            .cloned()
            .ok_or_else(|| FetchError::Transport(format!("no route {}", req.url)))
    }
}

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

fn provider(keys: &serde_json::Value) -> OidcProvider {
    let f = Arc::new(Mock::default());
    f.0.lock().unwrap().insert(
        format!("{ISSUER}/.well-known/openid-configuration"),
        format!(r#"{{"issuer":"{ISSUER}","authorization_endpoint":"{ISSUER}/auth","token_endpoint":"{ISSUER}/token","jwks_uri":"{ISSUER}/jwks"}}"#).into_bytes(),
    );
    let mut set = Vec::new();
    for (kid, v) in keys
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| !k.starts_with('_'))
    {
        let mut jwk = v["jwk"].clone();
        jwk["kid"] = serde_json::json!(kid);
        jwk["use"] = serde_json::json!("sig");
        set.push(jwk);
    }
    f.0.lock().unwrap().insert(
        format!("{ISSUER}/jwks"),
        serde_json::json!({"keys": set}).to_string().into_bytes(),
    );
    OidcProvider::new(
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: CLIENT.into(),
            client_secret: None,
            redirect_uri: "https://forge.example/cb".into(),
        },
        f,
    )
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn claims() -> serde_json::Value {
    serde_json::json!({"iss": ISSUER, "sub": "user-42", "aud": CLIENT, "exp": now() + 300, "iat": now(), "email": "u@example"})
}

fn sign(alg: Algorithm, kid: &str, key: &EncodingKey) -> String {
    let mut h = Header::new(alg);
    h.kid = Some(kid.into());
    encode(&h, &claims(), key).unwrap()
}

fn tamper_payload(tok: &str) -> String {
    let mut p: Vec<String> = tok.split('.').map(str::to_owned).collect();
    let mut c: serde_json::Value =
        serde_json::from_slice(&Base64UrlUnpadded::decode_vec(&p[1]).unwrap()).unwrap();
    c["sub"] = serde_json::json!("someone-else");
    p[1] = Base64UrlUnpadded::encode_string(c.to_string().as_bytes());
    p.join(".")
}

fn tamper_signature(tok: &str) -> String {
    let mut p: Vec<String> = tok.split('.').map(str::to_owned).collect();
    let mut sig = Base64UrlUnpadded::decode_vec(&p[2]).unwrap();
    let mid = sig.len() / 2;
    sig[mid] ^= 0x01;
    p[2] = Base64UrlUnpadded::encode_string(&sig);
    p.join(".")
}

#[test]
fn every_algorithm_row_behaves_as_required() {
    let k = keys();
    let p = provider(&k);
    let pem = |n: &str| k[n]["pem"].as_str().unwrap().as_bytes().to_vec();
    let rsa = EncodingKey::from_rsa_pem(&pem("rsa")).unwrap();
    let p256 = EncodingKey::from_ec_pem(&pem("p256")).unwrap();
    let p384 = EncodingKey::from_ec_pem(&pem("p384")).unwrap();
    let ed = EncodingKey::from_ed_pem(&pem("ed")).unwrap();
    let mut failures = Vec::new();
    let mut rows = 0;
    let mut line = |name: &str, what: &str, got: Result<(), OidcError>, want_ok: bool| {
        let ok = got.is_ok() == want_ok;
        println!(
            "{:<8} {:<26} {:<18} {}",
            name,
            what,
            if got.is_ok() { "accepted" } else { "refused" },
            if ok { "as required" } else { "WRONG" }
        );
        rows += 1;
        if !ok {
            failures.push(format!("{name} {what}"));
        }
    };
    let accepted: [(Algorithm, &str, &str, &EncodingKey); 6] = [
        (Algorithm::RS256, "RS256", "rsa", &rsa),
        (Algorithm::RS384, "RS384", "rsa", &rsa),
        (Algorithm::RS512, "RS512", "rsa", &rsa),
        (Algorithm::ES256, "ES256", "p256", &p256),
        (Algorithm::ES384, "ES384", "p384", &p384),
        (Algorithm::EdDSA, "EdDSA", "ed", &ed),
    ];
    for (alg, name, kid, key) in accepted {
        let tok = sign(alg, kid, key);
        line(
            name,
            "valid token",
            p.verify_id_token(&tok, None).map(|_| ()),
            true,
        );
        line(
            name,
            "tampered payload",
            p.verify_id_token(&tamper_payload(&tok), None).map(|_| ()),
            false,
        );
        line(
            name,
            "tampered signature",
            p.verify_id_token(&tamper_signature(&tok), None).map(|_| ()),
            false,
        );
    }
    // A key of the wrong kind for the header's algorithm, and a header that names another algorithm than the key's.
    line(
        "ES256",
        "kid of a P-384 key",
        p.verify_id_token(&sign(Algorithm::ES256, "p384", &p256), None)
            .map(|_| ()),
        false,
    );
    line(
        "RS256",
        "kid of an EC key",
        p.verify_id_token(&sign(Algorithm::RS256, "p256", &rsa), None)
            .map(|_| ()),
        false,
    );
    line(
        "EdDSA",
        "kid of an RSA key",
        p.verify_id_token(&sign(Algorithm::EdDSA, "rsa", &ed), None)
            .map(|_| ()),
        false,
    );
    // Algorithms planeter refuses: whatever the signature.
    let hs = EncodingKey::from_secret(b"a shared secret that is long enough to be a key");
    for (alg, name) in [
        (Algorithm::HS256, "HS256"),
        (Algorithm::HS384, "HS384"),
        (Algorithm::HS512, "HS512"),
    ] {
        line(
            name,
            "valid HMAC token",
            p.verify_id_token(&sign(alg, "rsa", &hs), None).map(|_| ()),
            false,
        );
    }
    for (alg, name) in [
        (Algorithm::PS256, "PS256"),
        (Algorithm::PS384, "PS384"),
        (Algorithm::PS512, "PS512"),
    ] {
        line(
            name,
            "valid RSA-PSS token",
            p.verify_id_token(&sign(alg, "rsa", &rsa), None).map(|_| ()),
            false,
        );
    }
    let none = format!(
        "{}.{}.",
        Base64UrlUnpadded::encode_string(br#"{"alg":"none","kid":"rsa"}"#),
        Base64UrlUnpadded::encode_string(claims().to_string().as_bytes())
    );
    line(
        "none",
        "unsigned token",
        p.verify_id_token(&none, None).map(|_| ()),
        false,
    );
    // The claims are still checked.
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some("p256".into());
    let mut c = claims();
    c["aud"] = serde_json::json!("someone-else");
    line(
        "ES256",
        "wrong audience",
        p.verify_id_token(&encode(&h, &c, &p256).unwrap(), None)
            .map(|_| ()),
        false,
    );
    let mut c = claims();
    c["exp"] = serde_json::json!(now() - 3600);
    line(
        "ES256",
        "expired",
        p.verify_id_token(&encode(&h, &c, &p256).unwrap(), None)
            .map(|_| ()),
        false,
    );
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        rows, 30,
        "the matrix has 30 rows; a row was added or dropped"
    );
}
