//! RFC 014 D-11 / ID-7: **the arguments the confined `curl` is given** for each of the three kinds of OIDC call (discovery, the key set and
//! its refresh, the token exchange), recorded by a stand-in `curl` that the real fetcher runs (`CurlFetcher::with_curl_binary`, no
//! bubblewrap: the arguments are what is under test). The same run shows the one-host allowance admit the provider's private host and pin
//! it, the `--max-redirs 0` and `--proto =https` of every call, and that only the refresh carries `no-cache`. Writes the transcript to
//! `target/tmp/oidc-curl-transcript.txt`.

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use planeter_auth::{OidcConfig, OidcProvider};
use planeter_core::{CurlFetcher, EgressError, EgressPolicy, StdEgressGuard};

const ISSUER: &str = "https://idp.test:8443";
const CLIENT: &str = "planeter-client";

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("oidc-curl-arguments");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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

fn jwk(k: &serde_json::Value, name: &str, kid: &str) -> serde_json::Value {
    let mut j = k[name]["jwk"].clone();
    j["kid"] = serde_json::json!(kid);
    j["use"] = serde_json::json!("sig");
    j
}

#[test]
fn the_three_kinds_of_call_and_what_curl_is_given_for_each() {
    let d = dir();
    // The stand-in curl: records every argument (one per line, a line `---` between calls) and answers with the file named after the URL.
    let script = d.join("curl");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> {log}; done\nprintf -- '---\\n' >> {log}\nfor last; do :; done\ncase \"$last\" in\n  */openid-configuration) cat {d}/discovery.json;;\n  */jwks) cat {d}/jwks.json;;\n  */token) cat {d}/token.json;;\n  *) exit 22;;\nesac\n",
            log = d.join("calls.log").display(),
            d = d.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let k = keys();
    std::fs::write(
        d.join("discovery.json"),
        format!(r#"{{"issuer":"{ISSUER}","authorization_endpoint":"{ISSUER}/auth","token_endpoint":"{ISSUER}/token","jwks_uri":"{ISSUER}/jwks"}}"#),
    )
    .unwrap();
    std::fs::write(
        d.join("jwks.json"),
        serde_json::json!({"keys": [jwk(&k, "p256", "k-old")]}).to_string(),
    )
    .unwrap();

    // The guard of the sign-in provider with the allowance for its own authority: the name resolves to a private address.
    let guard = StdEgressGuard::new(EgressPolicy {
        allow_private_for_authority: Some(planeter_core::Authority::new("idp.test", 8443)),
        ..Default::default()
    })
    .with_resolver(Arc::new(
        |host: &str, port: u16| -> Result<Vec<SocketAddr>, EgressError> {
            assert_eq!(host, "idp.test");
            Ok(vec![SocketAddr::new("10.1.2.3".parse().unwrap(), port)])
        },
    ));
    let fetcher =
        Arc::new(CurlFetcher::unconfined(Box::new(guard)).with_curl_binary(script.as_os_str()));
    let provider = OidcProvider::new(
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: CLIENT.into(),
            client_secret: Some("s3cret".into()),
            redirect_uri: "https://forge.example/cb".into(),
        },
        fetcher,
    )
    .with_endpoints_on_issuer_host();

    let begin = provider.begin_login(1_000).unwrap();
    let query = |name: &str| {
        begin
            .authorization_url
            .split_once('?')
            .unwrap()
            .1
            .split('&')
            .find_map(|kv| {
                kv.split_once('=')
                    .filter(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_owned())
            })
            .unwrap()
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sign = |alg: Algorithm, kid: &str, key: &EncodingKey, nonce: Option<&str>| {
        let mut h = Header::new(alg);
        h.kid = Some(kid.into());
        let mut c = serde_json::json!({"iss": ISSUER, "sub": "user-42", "aud": CLIENT, "exp": now + 300, "iat": now});
        if let Some(n) = nonce {
            c["nonce"] = serde_json::json!(n);
        }
        encode(&h, &c, key).unwrap()
    };
    let p256 = EncodingKey::from_ec_pem(k["p256"]["pem"].as_str().unwrap().as_bytes()).unwrap();
    std::fs::write(
        d.join("token.json"),
        format!(
            r#"{{"id_token":"{}"}}"#,
            sign(Algorithm::ES256, "k-old", &p256, Some(&query("nonce")))
        ),
    )
    .unwrap();
    let who = provider
        .finish_login(&begin.state, "the-code", 1_001)
        .expect("the sign-in completes against the private host");
    assert_eq!(who.subject, "user-42");
    // The rotation: a token under an unknown kid makes the provider refresh the key set once.
    std::fs::write(
        d.join("jwks.json"),
        serde_json::json!({"keys": [jwk(&k, "p256", "k-old"), jwk(&k, "ed", "k-new")]}).to_string(),
    )
    .unwrap();
    let ed = EncodingKey::from_ed_pem(k["ed"]["pem"].as_str().unwrap().as_bytes()).unwrap();
    provider
        .verify_id_token(&sign(Algorithm::EdDSA, "k-new", &ed, None), None)
        .expect("the new key is found after the refresh");

    // The transcript: one call after another.
    let log = std::fs::read_to_string(d.join("calls.log")).unwrap();
    let calls: Vec<Vec<&str>> = log
        .split("---\n")
        .filter(|c| !c.is_empty())
        .map(|c| c.lines().collect())
        .collect();
    let kinds = [
        "discovery (GET)",
        "token exchange (POST)",
        "key set, first fetch (GET)",
        "key set, refresh after an unknown kid (GET)",
    ];
    assert_eq!(calls.len(), kinds.len(), "{log}");
    let mut transcript = String::new();
    for (kind, c) in kinds.iter().zip(&calls) {
        // The client secret is a Basic header: never printed in the transcript.
        let shown: Vec<String> = c
            .iter()
            .map(|a| {
                if a.starts_with("Authorization: Basic ") {
                    "Authorization: Basic <the client's credentials>".to_owned()
                } else {
                    (*a).to_owned()
                }
            })
            .collect();
        transcript += &format!("{kind}\n  curl {}\n", shown.join(" "));
    }
    std::fs::write(d.join("..").join("oidc-curl-transcript.txt"), &transcript).unwrap();

    for (i, c) in calls.iter().enumerate() {
        let s = c.join(" ");
        // Every call: https only, no redirect followed, bounded, **pinned to the address the guard checked** (never the name), nothing else.
        for must in [
            "--proto =https",
            "--max-redirs 0",
            "--max-time 15",
            "--max-filesize 1048576",
            "--resolve idp.test:8443:10.1.2.3",
        ] {
            assert!(s.contains(must), "call {i}: `{must}` is missing in: {s}");
        }
        assert!(
            !s.contains(" -L") && !s.contains("--location"),
            "call {i} may follow redirects: {s}"
        );
        assert_eq!(
            c.last().copied(),
            Some(
                [
                    "https://idp.test:8443/.well-known/openid-configuration",
                    "https://idp.test:8443/token",
                    "https://idp.test:8443/jwks",
                    "https://idp.test:8443/jwks"
                ][i]
            )
        );
        // Only the refresh asks the caches for a fresh answer.
        assert_eq!(
            s.contains("Cache-Control: no-cache") && s.contains("Pragma: no-cache"),
            i == 3,
            "call {i}: {s}"
        );
        assert_eq!(s.contains("--data-binary"), i == 1, "call {i}");
    }
}
