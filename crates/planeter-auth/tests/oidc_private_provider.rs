//! RFC 014 D-11: **a sign-in provider on a private address**, against a real HTTPS stand-in provider and the real `curl`.
//!
//! The stand-in (`tests/fixtures/provider-standin/standin.py`) listens on loopback with a self-made certificate. The product has no way to
//! trust such a certificate (RFC 014 open question 4: a private CA is a later handoff), so **the test-only mechanism is a `curl` wrapper that
//! adds `--cacert <that certificate>`** (`CurlFetcher::with_curl_binary`); nothing else is changed in the fetcher or the guard, and no
//! bubblewrap is used (the guard, the pinning and the arguments are what is under test). Tests skip, saying so, without python3 or curl.
//!
//! Shown: allowance **off**, a provider on loopback is refused before any connection; **on**, the whole authorization-code flow with PKCE, a
//! nonce and an EdDSA ID token completes and the linked person is signed in; a **second private host named in discovery** is refused, a
//! **redirect to another private address** is not followed, an **`http` issuer** is refused, a **name that resolves to a private and a public
//! address** is refused when it is not the issuer's host, and **the address pinned is the address connected to** (a resolver that answers
//! differently the second time).

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use base64ct::{Base64UrlUnpadded, Encoding as _};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InsecureStubHasher, OidcConfig, OidcError, OidcProvider,
};
use planeter_core::{
    CurlFetcher, EgressError, EgressGuard, EgressPolicy, Principal, StdEgressGuard,
};
use planeter_store::UserId;
use sha2::Digest as _;

const CLIENT: &str = "planeter-client";
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/provider-standin"
);

macro_rules! need_tools {
    () => {
        if !tools_available() {
            eprintln!("skipping: needs python3 (with ssl) and curl");
            return;
        }
    };
}

fn tools_available() -> bool {
    Command::new("python3")
        .args(["-c", "import ssl, http.server"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("curl")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// A stand-in provider process on `bind` (a loopback address) with its own directory of canned answers and its log of requests.
struct Standin {
    child: Child,
    dir: PathBuf,
    port: u16,
}

impl Standin {
    fn start(name: &str, bind: &str) -> Standin {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("oidc-private-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let child = Command::new("python3")
            .arg(format!("{FIXTURES}/standin.py"))
            .arg(bind)
            .arg(&dir)
            .arg(format!("{FIXTURES}/cert.pem"))
            .arg(format!("{FIXTURES}/key.pem"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the stand-in provider");
        let end = Instant::now() + Duration::from_secs(30);
        let port = loop {
            if let Ok(p) = std::fs::read_to_string(dir.join("port")) {
                break p.trim().parse().unwrap();
            }
            assert!(
                Instant::now() < end,
                "the stand-in provider did not start within 30 s"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        Standin { child, dir, port }
    }

    fn put(&self, file: &str, body: impl AsRef<[u8]>) {
        std::fs::write(self.dir.join(file), body).unwrap();
    }

    /// What reached it, one JSON value per request.
    fn requests(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.dir.join("requests.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

impl Drop for Standin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// The `curl` of these tests: the real one, told to trust the stand-in's certificate. **Test only** (see the module's documentation).
fn trusting_curl(dir: &Path) -> PathBuf {
    let script = dir.join("curl-trusting-the-standin");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nexec curl --cacert {FIXTURES}/cert.pem \"$@\"\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A script that was written a moment ago can be refused with "Text file busy" (ETXTBSY) while another test's thread has forked and not yet
    // executed: the child holds a copy of the write descriptor until its `exec`. It is a race of the test and it goes away by itself, so
    // the script is tried until it runs, before any fetch depends on it.
    for _ in 0..200 {
        match Command::new(&script).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(10)),
            _ => break,
        }
    }
    script
}

/// Names the tests use: `idp.test` and `other.test` are private, `mixed.test` has a private and a public address.
fn resolver() -> planeter_core::Resolver {
    Arc::new(
        |host: &str, port: u16| -> Result<Vec<SocketAddr>, EgressError> {
            let ips: &[&str] = match host {
                "idp.test" => &["127.0.0.1"],
                "other.test" => &["127.0.0.2"],
                "mixed.test" => &["127.0.0.1", "93.184.216.34"],
                _ => return Err(EgressError::Unresolvable(host.to_owned())),
            };
            Ok(ips
                .iter()
                .map(|ip| SocketAddr::new(ip.parse().unwrap(), port))
                .collect())
        },
    )
}

/// The guard of the sign-in provider: with `Some(issuer URL)` the **authority** of that URL (its host and effective port) may be private.
fn guard(allow_for_issuer: Option<&str>) -> StdEgressGuard {
    StdEgressGuard::new(EgressPolicy {
        allow_private_for_authority: allow_for_issuer
            .map(|u| planeter_core::url_authority(u).expect("an issuer URL").1),
        ..Default::default()
    })
    .with_resolver(resolver())
}

fn provider(
    issuer: &str,
    guard: impl EgressGuard + 'static,
    work: &Path,
    strict: bool,
) -> OidcProvider {
    let fetcher = Arc::new(
        CurlFetcher::unconfined(Box::new(guard)).with_curl_binary(trusting_curl(work).as_os_str()),
    );
    let p = OidcProvider::new(
        OidcConfig {
            issuer: issuer.to_owned(),
            client_id: CLIENT.into(),
            client_secret: None,
            redirect_uri: "https://forge.example/login/oidc/callback".into(),
        },
        fetcher,
    );
    if strict {
        p.with_endpoints_on_issuer_host()
    } else {
        p
    }
}

fn discovery(issuer: &str, token_host_port: &str) -> String {
    format!(
        r#"{{"issuer":"{issuer}","authorization_endpoint":"{issuer}/auth","token_endpoint":"https://{token_host_port}/token","jwks_uri":"{issuer}/jwks"}}"#
    )
}

fn query(url: &str, name: &str) -> String {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|kv| {
            kv.split_once('=')
                .filter(|(k, _)| *k == name)
                .map(|(_, v)| v.to_owned())
        })
        .unwrap()
}

fn id_token(issuer: &str, nonce: &str) -> String {
    let k = keys();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut h = Header::new(Algorithm::EdDSA);
    h.kid = Some("k-ed".into());
    encode(
        &h,
        &serde_json::json!({"iss": issuer, "sub": "user-42", "aud": CLIENT, "exp": now + 300, "iat": now, "nonce": nonce}),
        &EncodingKey::from_ed_pem(k["ed"]["pem"].as_str().unwrap().as_bytes()).unwrap(),
    )
    .unwrap()
}

fn jwks() -> String {
    let k = keys();
    let mut j = k["ed"]["jwk"].clone();
    j["kid"] = serde_json::json!("k-ed");
    j["use"] = serde_json::json!("sig");
    serde_json::json!({"keys": [j]}).to_string()
}

fn publish(s: &Standin, issuer: &str) {
    s.put(
        "discovery.json",
        discovery(issuer, &format!("idp.test:{}", s.port)),
    );
    s.put("jwks.json", jwks());
}

#[test]
fn with_the_allowance_off_a_provider_on_loopback_is_refused_before_any_connection() {
    need_tools!();
    let s = Standin::start("off", "127.0.0.1");
    let issuer = format!("https://idp.test:{}", s.port);
    publish(&s, &issuer);
    let p = provider(&issuer, guard(None), &s.dir, false);
    match p.begin_login(1_000) {
        Err(OidcError::Provider(why)) => assert!(
            why.contains("blocked") && why.contains("127.0.0.1"),
            "{why}"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // A literal loopback issuer too (the same as before this change).
    let lit = format!("https://127.0.0.1:{}", s.port);
    assert!(
        provider(&lit, guard(None), &s.dir, false)
            .begin_login(1_000)
            .is_err()
    );
    assert!(
        s.requests().is_empty(),
        "nothing reached the provider: {:?}",
        s.requests()
    );
}

#[test]
fn with_the_allowance_on_the_whole_flow_completes_and_the_linked_person_is_signed_in() {
    need_tools!();
    let s = Standin::start("on", "127.0.0.1");
    let issuer = format!("https://idp.test:{}", s.port);
    publish(&s, &issuer);
    let p = provider(&issuer, guard(Some(&issuer)), &s.dir, true);
    let begin = p
        .begin_login(1_000)
        .expect("discovery over the private address");
    // The browser is sent to the provider with PKCE and a nonce.
    assert!(
        begin
            .authorization_url
            .starts_with(&format!("{issuer}/auth?"))
    );
    assert_eq!(
        query(&begin.authorization_url, "code_challenge_method"),
        "S256"
    );
    let nonce = query(&begin.authorization_url, "nonce");
    s.put(
        "token.json",
        format!(r#"{{"id_token":"{}"}}"#, id_token(&issuer, &nonce)),
    );
    let who = p
        .finish_login(&begin.state, "the-code", 1_001)
        .expect("the ID token is verified");
    assert_eq!(
        (who.issuer.as_str(), who.subject.as_str()),
        (issuer.as_str(), "user-42")
    );
    // The provider received discovery, the token exchange (with the PKCE verifier that matches the challenge) and the key set: nothing else.
    let seen = s.requests();
    let paths: Vec<(&str, &str)> = seen
        .iter()
        .map(|r| (r["method"].as_str().unwrap(), r["path"].as_str().unwrap()))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("GET", "/.well-known/openid-configuration"),
            ("POST", "/token"),
            ("GET", "/jwks")
        ]
    );
    let form = seen[1]["body"].as_str().unwrap();
    let verifier = form
        .split('&')
        .find_map(|kv| kv.strip_prefix("code_verifier="))
        .expect("a PKCE verifier");
    assert_eq!(
        Base64UrlUnpadded::encode_string(&sha2::Sha256::digest(verifier.as_bytes())),
        query(&begin.authorization_url, "code_challenge"),
        "the verifier is the one the challenge was made from"
    );
    assert!(form.contains("grant_type=authorization_code") && form.contains("code=the-code"));
    assert_eq!(
        seen[0]["host"],
        format!("idp.test:{}", s.port),
        "the name is sent, the address was pinned"
    );
    // The person linked to that identity is signed in.
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account::local("alice", "Alice", None))
        .unwrap();
    accounts
        .link_oidc(&issuer, "user-42", UserId::new("alice"))
        .unwrap();
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    );
    assert!(
        matches!(auth.authenticate_oidc(&who), Ok(Principal::User(u)) if u == UserId::new("alice"))
    );
}

#[test]
fn a_second_private_host_named_in_discovery_is_refused_and_never_asked() {
    need_tools!();
    let issuer_server = Standin::start("second-a", "127.0.0.1");
    let other = Standin::start("second-b", "127.0.0.2");
    let issuer = format!("https://idp.test:{}", issuer_server.port);
    // The provider's own discovery document points the token endpoint at another private host.
    issuer_server.put(
        "discovery.json",
        discovery(&issuer, &format!("other.test:{}", other.port)),
    );
    issuer_server.put("jwks.json", jwks());
    let p = provider(&issuer, guard(Some(&issuer)), &issuer_server.dir, true);
    match p.begin_login(1_000) {
        Err(OidcError::Provider(why)) => {
            assert_eq!(
                why,
                "the token endpoint is not on the issuer's host and port"
            )
        }
        other => panic!("expected the refusal naming the token endpoint, got {other:?}"),
    }
    assert!(
        other.requests().is_empty(),
        "the other private host was never asked"
    );
    // Even without the endpoint comparison (the guard alone) that host is refused: it is private and it is not the allowed one.
    let g = guard(Some(&issuer));
    assert!(matches!(
        g.check(&format!("https://other.test:{}/token", other.port)),
        Err(EgressError::BlockedAddress(_))
    ));
    // Each of the three endpoints is compared, and named.
    for (field, name) in [
        ("authorization_endpoint", "authorization"),
        ("token_endpoint", "token"),
        ("jwks_uri", "jwks"),
    ] {
        let mut d: serde_json::Value = serde_json::from_str(&discovery(
            &issuer,
            &format!("idp.test:{}", issuer_server.port),
        ))
        .unwrap();
        d[field] = serde_json::json!(format!("https://mixed.test:{}/x", other.port));
        issuer_server.put("discovery.json", d.to_string());
        let p = provider(&issuer, guard(Some(&issuer)), &issuer_server.dir, true);
        match p.begin_login(1_000) {
            Err(OidcError::Provider(why)) => assert_eq!(
                why,
                format!("the {name} endpoint is not on the issuer's host and port")
            ),
            other => panic!("{field}: {other:?}"),
        }
    }
    assert!(other.requests().is_empty());
}

#[test]
fn another_port_of_the_issuers_host_is_another_service_and_is_refused_and_never_asked() {
    need_tools!();
    let issuer_server = Standin::start("port-a", "127.0.0.1");
    // Another service on the **same host** (the same loopback address, the same name): what an operator did not allow.
    let admin = Standin::start("port-b", "127.0.0.1");
    assert_ne!(issuer_server.port, admin.port);
    let issuer = format!("https://idp.test:{}", issuer_server.port);
    issuer_server.put("jwks.json", jwks());
    // Each of the three endpoints, one at a time, on the other port of the issuer's host: refused with that endpoint named.
    for (field, name) in [
        ("authorization_endpoint", "authorization"),
        ("token_endpoint", "token"),
        ("jwks_uri", "jwks"),
    ] {
        let mut d: serde_json::Value = serde_json::from_str(&discovery(
            &issuer,
            &format!("idp.test:{}", issuer_server.port),
        ))
        .unwrap();
        d[field] = serde_json::json!(format!("https://idp.test:{}/x", admin.port));
        issuer_server.put("discovery.json", d.to_string());
        let p = provider(&issuer, guard(Some(&issuer)), &issuer_server.dir, true);
        match p.begin_login(1_000) {
            Err(OidcError::Provider(why)) => assert_eq!(
                why,
                format!("the {name} endpoint is not on the issuer's host and port"),
                "{field}"
            ),
            other => panic!("{field}: {other:?}"),
        }
    }
    // An endpoint without a port (443) is another authority too, and so is the same port over `http`.
    for endpoint in [
        "https://idp.test/token",
        &format!("http://idp.test:{}/token", issuer_server.port),
    ] {
        let mut d: serde_json::Value = serde_json::from_str(&discovery(
            &issuer,
            &format!("idp.test:{}", issuer_server.port),
        ))
        .unwrap();
        d["token_endpoint"] = serde_json::json!(endpoint);
        issuer_server.put("discovery.json", d.to_string());
        let p = provider(&issuer, guard(Some(&issuer)), &issuer_server.dir, true);
        assert!(
            matches!(p.begin_login(1_000), Err(OidcError::Provider(_))),
            "{endpoint}"
        );
    }
    assert!(
        admin.requests().is_empty(),
        "the other port received nothing: {:?}",
        admin.requests()
    );
    // The guard alone refuses the other port (private, and not the allowed authority) and admits the allowed one.
    let g = guard(Some(&issuer));
    assert!(matches!(
        g.check(&format!("https://idp.test:{}/x", admin.port)),
        Err(EgressError::BlockedAddress(_))
    ));
    assert!(g.check(&format!("{issuer}/x")).is_ok());
    // The issuer's own stand-in was asked for the discovery document (once per attempt) and for nothing else.
    let asked: Vec<String> = issuer_server
        .requests()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(asked.len(), 5, "{asked:?}");
    assert!(
        asked
            .iter()
            .all(|p| p == "/.well-known/openid-configuration"),
        "{asked:?}"
    );
}

#[test]
fn a_redirect_to_another_private_address_is_not_followed() {
    need_tools!();
    let s = Standin::start("redirect-a", "127.0.0.1");
    let other = Standin::start("redirect-b", "127.0.0.2");
    let issuer = format!("https://idp.test:{}", s.port);
    publish(&s, &issuer);
    other.put(
        "discovery.json",
        discovery(&issuer, &format!("idp.test:{}", s.port)),
    );
    // Every GET on the issuer's host is answered with a redirect to the other private host (which would answer with a document).
    s.put(
        "redirect",
        format!(
            "https://other.test:{}/.well-known/openid-configuration",
            other.port
        ),
    );
    let p = provider(&issuer, guard(Some(&issuer)), &s.dir, true);
    assert!(
        matches!(p.begin_login(1_000), Err(OidcError::Provider(_))),
        "a redirect answer is not a discovery document"
    );
    assert!(
        other.requests().is_empty(),
        "the redirect target was never connected to: {:?}",
        other.requests()
    );
    // The same for a redirect to a private literal address and to a public address: curl follows no redirect (`--max-redirs 0`, no `-L`).
    for target in [
        "https://127.0.0.2/x".to_owned(),
        "https://93.184.216.34/x".to_owned(),
    ] {
        s.put("redirect", target);
        let p = provider(&issuer, guard(Some(&issuer)), &s.dir, true);
        assert!(p.begin_login(1_000).is_err());
    }
    assert!(other.requests().is_empty());
    let asked: Vec<_> = s
        .requests()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        asked.len(),
        3,
        "one request each, never a second one: {asked:?}"
    );
}

#[test]
fn an_http_issuer_is_refused_even_on_the_allowed_authority() {
    need_tools!();
    let s = Standin::start("http", "127.0.0.1");
    let issuer = format!("http://idp.test:{}", s.port);
    let p = provider(&issuer, guard(Some(&issuer)), &s.dir, true);
    match p.begin_login(1_000) {
        Err(OidcError::Provider(why)) => {
            assert!(why.contains("disallowed URL scheme: http"), "{why}")
        }
        other => panic!("{other:?}"),
    }
    assert!(s.requests().is_empty());
}

#[test]
fn a_name_with_a_private_and_a_public_address_is_refused_unless_it_is_the_allowed_authority() {
    // (No stand-in needed: the guard alone, with the resolver that gives `mixed.test` both kinds.)
    let g = guard(Some("https://idp.test:8443"));
    assert!(matches!(
        g.check("https://mixed.test:8443/"),
        Err(EgressError::BlockedAddress(_))
    ));
    // The allowed host itself is admitted whatever it resolves to (the operator named it), and everything it resolved is pinned.
    let only_mixed = guard(Some("https://mixed.test:8443"));
    let t = only_mixed.check("https://mixed.test:8443/").unwrap();
    assert_eq!(t.addrs.len(), 2);
}

#[test]
fn the_pinned_address_is_the_address_connected_to_even_if_the_name_resolves_elsewhere_later() {
    need_tools!();
    let s = Standin::start("pinned", "127.0.0.1");
    let issuer = format!("https://idp.test:{}", s.port);
    publish(&s, &issuer);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    // The first answer is the stand-in's loopback address; every later answer for the name is an unreachable public one (DNS rebinding). If
    // the fetch resolved the name again after the check, or connected by name, it would go there (and wait for its time out) or fail.
    let flapping: planeter_core::Resolver = Arc::new(move |host: &str, port: u16| {
        assert_eq!(host, "idp.test");
        let ip = if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            "127.0.0.1"
        } else {
            "203.0.113.9"
        };
        Ok(vec![SocketAddr::new(ip.parse().unwrap(), port)])
    });
    let g = StdEgressGuard::new(EgressPolicy {
        allow_private_for_authority: Some(planeter_core::Authority::new("idp.test", s.port)),
        ..Default::default()
    })
    .with_resolver(flapping);
    let fetcher =
        CurlFetcher::unconfined(Box::new(g)).with_curl_binary(trusting_curl(&s.dir).as_os_str());
    let url = format!("{issuer}/.well-known/openid-configuration");
    let body =
        planeter_core::HttpsFetcher::fetch(&fetcher, &planeter_core::FetchRequest::get(&url))
            .expect("the fetch goes to the pinned address");
    assert!(String::from_utf8_lossy(&body).contains(&issuer));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the name was resolved once for the fetch, and not again to connect"
    );
    assert_eq!(
        s.requests().len(),
        1,
        "the stand-in on the pinned address received it"
    );
    // The arguments say so: the connection is pinned to the checked address (`--resolve name:port:address`), never left to the name.
    let target = {
        let again = guard(Some(&issuer));
        again.check(&url).unwrap()
    };
    let args = CurlFetcher::curl_args(
        &target,
        &planeter_core::FetchRequest::get(&url),
        Default::default(),
    );
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--resolve" && w[1] == format!("idp.test:{}:127.0.0.1", s.port)),
        "{args:?}"
    );
}
