//! RFC 014 D-12: **the interoperability test: planeter's provider code against a real sui-id built from a tagged release.**
//!
//! **`#[ignore]` by default and never run by the gates or CI**: it needs an external provider that someone started. It is measurement and
//! evidence, not a gate. Every check is a named test (`c01_…` to `c09_…`; run them one at a time and in order, `--test-threads=1`), and each
//! one **records what it measured** (a line per fact, `CONFIRMED`, `CORRECTED` or `NOT SHOWN`, against what the ledger said) in
//! `$PLANETER_SUI_ID_EVIDENCE/<test>.txt` and on stdout. A fact that differs from the ledger is a finding, not a failure of the test: the
//! test fails only where a thing planeter itself promises is broken (it signed someone in when it should have refused, it hung).
//!
//! Method: the same as `oidc_private_provider.rs`. The real [`OidcProvider`], the real `curl` (through a wrapper that trusts the throw-away CA:
//! the product has no private CA, RFC 014 open question 4), the egress guard with the one-host allowance for the issuer's authority, and
//! no bubblewrap. A test user's browser step is plain HTTP calls with `curl` and a cookie jar to sui-id's own endpoints: `GET
//! /oauth2/authorize` (redirects to the login form), `POST /admin/login` (the form fields `username`, `password`, `next`: sui-id has one
//! login form for the console and for the applications), and the redirect back to the authorize URL, which answers with the redirect to the
//! callback carrying `code` and `state`. The administrator's actions (rotate the signing key, disable a user) are done by a small driver
//! (`$PLANETER_SUI_ID_ADMIN`) that uses sui-id's own admin console the way an administrator's browser does.
//!
//! The whole procedure (build the tag, the throw-away CA, the TLS stand-in, the users and clients, these tests, and the removal of everything it
//! made) is `tests/fixtures/sui-id-interop/run.sh`; the stand-in and the admin driver are in the same directory.
//!
//! Settings (environment): `PLANETER_SUI_ID_URL` (the issuer, https), `PLANETER_SUI_ID_CA` (the throw-away CA file),
//! `PLANETER_SUI_ID_PUBLIC_CLIENT`, `PLANETER_SUI_ID_CONFIDENTIAL_CLIENT`, `PLANETER_SUI_ID_CONFIDENTIAL_SECRET`,
//! `PLANETER_SUI_ID_REDIRECT_URI`, `PLANETER_SUI_ID_USER` / `_PASSWORD` / `_USER_ID` (a user with an email),
//! `PLANETER_SUI_ID_NOEMAIL_USER` / `_NOEMAIL_PASSWORD` (a user without one), `PLANETER_SUI_ID_CONTROL_DIR` (the TLS stand-in's control files
//! and request log), `PLANETER_SUI_ID_ADMIN` (the admin driver's command), `PLANETER_SUI_ID_EVIDENCE` (where the records go).
//! Without them every test says so and passes.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64ct::{Base64UrlUnpadded, Encoding as _};
use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, InsecureStubHasher, OidcConfig, OidcError, OidcProvider, SessionStore,
    VerifiedIdentity,
};
use planeter_core::{
    CurlFetcher, EgressError, EgressPolicy, FetchError, FetchRequest, HttpsFetcher, Principal,
    StdEgressGuard,
};
use planeter_store::UserId;
use serde_json::Value;
use sha2::Digest as _;

// ------------------------------------------------------------------------------------------------ settings

struct Cfg {
    issuer: String,
    ca: String,
    public_client: String,
    confidential_client: String,
    confidential_secret: String,
    redirect_uri: String,
    user: String,
    password: String,
    user_id: String,
    noemail_user: String,
    noemail_password: String,
    ctl: PathBuf,
    admin: Vec<String>,
    evidence: PathBuf,
}

fn cfg() -> Option<Cfg> {
    let get = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
    let evidence = PathBuf::from(
        get("PLANETER_SUI_ID_EVIDENCE").unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").to_owned()),
    );
    std::fs::create_dir_all(&evidence).ok()?;
    Some(Cfg {
        issuer: get("PLANETER_SUI_ID_URL")?.trim_end_matches('/').to_owned(),
        ca: get("PLANETER_SUI_ID_CA")?,
        public_client: get("PLANETER_SUI_ID_PUBLIC_CLIENT")?,
        confidential_client: get("PLANETER_SUI_ID_CONFIDENTIAL_CLIENT")?,
        confidential_secret: get("PLANETER_SUI_ID_CONFIDENTIAL_SECRET")?,
        redirect_uri: get("PLANETER_SUI_ID_REDIRECT_URI")?,
        user: get("PLANETER_SUI_ID_USER")?,
        password: get("PLANETER_SUI_ID_PASSWORD")?,
        user_id: get("PLANETER_SUI_ID_USER_ID")?,
        noemail_user: get("PLANETER_SUI_ID_NOEMAIL_USER")?,
        noemail_password: get("PLANETER_SUI_ID_NOEMAIL_PASSWORD")?,
        ctl: PathBuf::from(get("PLANETER_SUI_ID_CONTROL_DIR")?),
        admin: get("PLANETER_SUI_ID_ADMIN")?
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        evidence,
    })
}

macro_rules! need {
    ($c:ident) => {
        let Some($c) = cfg() else {
            eprintln!(
                "SKIPPED: the sui-id interoperability test needs a running, tagged sui-id behind a TLS stand-in and the \
                 PLANETER_SUI_ID_* settings (see the head of this file); none is set, so nothing was measured"
            );
            return;
        };
    };
}

// ------------------------------------------------------------------------------------------------ the evidence

/// What a check measured: one line per fact. Never panics: a difference from the ledger is a finding.
struct Report {
    name: &'static str,
    text: String,
}

impl Report {
    fn new(name: &'static str) -> Report {
        Report {
            name,
            text: String::new(),
        }
    }
    fn note(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.text, "{}", line.as_ref());
    }
    /// `expected` is what the ledger (or the handoff) said; `measured` is what was seen.
    fn check(&mut self, id: &str, expected: &str, measured: impl AsRef<str>, ok: bool) {
        let verdict = if ok { "CONFIRMED" } else { "CORRECTED" };
        let _ = writeln!(
            self.text,
            "[{verdict}] {id}\n    expected: {expected}\n    measured: {}",
            measured.as_ref()
        );
    }
    fn not_shown(&mut self, id: &str, why: &str) {
        let _ = writeln!(self.text, "[NOT SHOWN] {id}\n    why: {why}");
    }
    fn save(self, c: &Cfg) {
        println!("===== {} =====\n{}", self.name, self.text);
        let _ = std::fs::write(c.evidence.join(format!("{}.txt", self.name)), &self.text);
    }
}

// ------------------------------------------------------------------------------------------------ plain helpers

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz");
                match u8::from_str_radix(h, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A query parameter of `url`, percent-decoded.
fn query(url: &str, name: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    let q = q.split('#').next().unwrap_or(q);
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| pct_decode(v))
    })
}

fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            o.push(b as char);
        } else {
            let _ = write!(o, "%{b:02X}");
        }
    }
    o
}

/// The two JSON parts of a JWT (header, payload).
fn jwt(token: &str) -> (Value, Value) {
    let mut p = token.split('.');
    let part = |s: Option<&str>| -> Value {
        serde_json::from_slice(&Base64UrlUnpadded::decode_vec(s.unwrap_or("")).unwrap_or_default())
            .unwrap_or(Value::Null)
    };
    (part(p.next()), part(p.next()))
}

fn keys_of(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    k.sort();
    k
}

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut buf)
        .unwrap();
    Base64UrlUnpadded::encode_string(&buf)
}

/// A PKCE pair of the test's own (S256).
fn pkce() -> (String, String) {
    let verifier = random_b64(32);
    let challenge = Base64UrlUnpadded::encode_string(&sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

// ------------------------------------------------------------------------------------------------ planeter's side

fn resolver() -> planeter_core::Resolver {
    Arc::new(
        |host: &str, port: u16| -> Result<Vec<SocketAddr>, EgressError> {
            match host {
                "idp.test" => Ok(vec![SocketAddr::new("127.0.0.1".parse().unwrap(), port)]),
                _ => Err(EgressError::Unresolvable(host.to_owned())),
            }
        },
    )
}

fn guard(issuer: &str) -> StdEgressGuard {
    StdEgressGuard::new(EgressPolicy {
        allow_private_for_authority: Some(
            planeter_core::url_authority(issuer).expect("an issuer").1,
        ),
        ..Default::default()
    })
    .with_resolver(resolver())
}

/// The `curl` of these tests: the real one, told to trust the throw-away CA. With `strip_no_cache` it also **removes the
/// `Cache-Control: no-cache` and `Pragma: no-cache` request headers** (the stand-in for planeter before the ID-7 fix, as `jwks_refresh.rs` does).
fn wrapper(dir: &Path, ca: &str, strip_no_cache: bool) -> PathBuf {
    let script = dir.join(if strip_no_cache {
        "curl-trusting-and-stripping-no-cache"
    } else {
        "curl-trusting-the-throwaway-ca"
    });
    let body = if strip_no_cache {
        format!(
            "#!/usr/bin/env python3\nimport os, sys\na = sys.argv[1:]\nout = []\ni = 0\nwhile i < len(a):\n    if a[i] == '-H' and i + 1 < len(a) and a[i + 1].lower() in ('cache-control: no-cache', 'pragma: no-cache'):\n        i += 2\n        continue\n    out.append(a[i])\n    i += 1\nos.execvp('curl', ['curl', '--cacert', {ca:?}] + out)\n"
        )
    } else {
        format!("#!/bin/sh\nexec curl --cacert {ca} \"$@\"\n")
    };
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..200 {
        match Command::new(&script).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(10)),
            _ => break,
        }
    }
    script
}

/// One fetch planeter made: what it asked and what came back.
#[derive(Clone, Debug)]
struct Exchange {
    url: String,
    form: Option<String>,
    basic: bool,
    no_cache: bool,
    body: Result<Vec<u8>, String>,
    ms: u128,
}

/// The real fetcher, with a log of every request planeter makes.
struct Recording {
    inner: CurlFetcher,
    log: Mutex<Vec<Exchange>>,
}

impl HttpsFetcher for Recording {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        let t = Instant::now();
        let r = self.inner.fetch(req);
        self.log.lock().unwrap().push(Exchange {
            url: req.url.clone(),
            form: req.form.clone(),
            basic: req.authorization.is_some(),
            no_cache: req.no_cache,
            body: match &r {
                Ok(b) => Ok(b.clone()),
                Err(e) => Err(e.to_string()),
            },
            ms: t.elapsed().as_millis(),
        });
        r
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Public,
    Confidential,
}

struct Rig {
    provider: OidcProvider,
    rec: Arc<Recording>,
    client: String,
}

impl Rig {
    fn exchanges(&self) -> Vec<Exchange> {
        self.rec.log.lock().unwrap().clone()
    }
}

fn rig(c: &Cfg, work: &Path, kind: Kind, strip_no_cache: bool) -> Rig {
    let curl = wrapper(work, &c.ca, strip_no_cache);
    let rec = Arc::new(Recording {
        inner: CurlFetcher::unconfined(Box::new(guard(&c.issuer)))
            .with_curl_binary(curl.as_os_str()),
        log: Mutex::new(Vec::new()),
    });
    let client = match kind {
        Kind::Public => c.public_client.clone(),
        Kind::Confidential => c.confidential_client.clone(),
    };
    let provider = OidcProvider::new(
        OidcConfig {
            issuer: c.issuer.clone(),
            client_id: client.clone(),
            client_secret: (kind == Kind::Confidential).then(|| c.confidential_secret.clone()),
            redirect_uri: c.redirect_uri.clone(),
        },
        rec.clone() as Arc<dyn HttpsFetcher>,
    )
    .with_endpoints_on_issuer_host();
    Rig {
        provider,
        rec,
        client,
    }
}

fn work_dir(c: &Cfg, name: &str) -> PathBuf {
    let d = c.evidence.join(format!("work-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

// ------------------------------------------------------------------------------------------------ the browser's side (plain HTTP with curl)

#[derive(Debug)]
struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// What the redirect to the application's callback carried.
#[derive(Debug)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    /// Whether the provider asked for a sign-in on the way (the form was posted).
    asked_to_sign_in: bool,
    /// The last response, when it was not a redirect to the callback.
    last: Option<(u16, String)>,
}

struct Browser<'a> {
    c: &'a Cfg,
    jar: PathBuf,
}

impl<'a> Browser<'a> {
    fn new(c: &'a Cfg, dir: &Path, name: &str) -> Browser<'a> {
        let jar = dir.join(format!("jar-{name}.txt"));
        let _ = std::fs::remove_file(&jar);
        Browser { c, jar }
    }

    fn port(&self) -> u16 {
        planeter_core::url_authority(&self.c.issuer).unwrap().1.port
    }

    fn abs(&self, loc: &str) -> String {
        if loc.starts_with('/') {
            format!("{}{loc}", self.c.issuer)
        } else {
            loc.to_owned()
        }
    }

    fn curl(&self, args: &[String]) -> Resp {
        let out = Command::new("curl")
            .args(["-s", "-i", "--max-time", "60", "-A", "browser-step"])
            .args(["--cacert", &self.c.ca])
            .args(["--resolve", &format!("idp.test:{}:127.0.0.1", self.port())])
            .args([
                "-b",
                self.jar.to_str().unwrap(),
                "-c",
                self.jar.to_str().unwrap(),
            ])
            .args(args)
            .output()
            .expect("run curl");
        let raw = String::from_utf8_lossy(&out.stdout).into_owned();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        Resp {
            status,
            headers,
            body: body.to_owned(),
        }
    }

    fn get(&self, url: &str) -> Resp {
        self.curl(&[self.abs(url)])
    }

    fn post(&self, url: &str, fields: &[(&str, &str)], basic: Option<(&str, &str)>) -> Resp {
        let mut a = Vec::new();
        for (k, v) in fields {
            a.push("--data-urlencode".to_owned());
            a.push(format!("{k}={v}"));
        }
        if let Some((u, p)) = basic {
            a.push("-u".to_owned());
            a.push(format!("{u}:{p}"));
        }
        a.push(self.abs(url));
        self.curl(&a)
    }

    /// Follow an authorization URL to the redirect to the callback, signing in at the provider's form if it asks.
    fn authorize(&self, url: &str, user: &str, password: &str) -> Callback {
        let mut asked = false;
        let mut r = self.get(url);
        for _ in 0..6 {
            let loc = match (r.status, r.header("location")) {
                (301..=303 | 307 | 308, Some(l)) => l.to_owned(),
                _ => {
                    return Callback {
                        code: None,
                        state: None,
                        asked_to_sign_in: asked,
                        last: Some((r.status, r.body.chars().take(300).collect())),
                    };
                }
            };
            if loc.starts_with(&self.c.redirect_uri) {
                return Callback {
                    code: query(&loc, "code"),
                    state: query(&loc, "state"),
                    asked_to_sign_in: asked,
                    last: None,
                };
            }
            if loc.starts_with("/admin/login") {
                asked = true;
                let next = query(&loc, "next").unwrap_or_default();
                // The form is fetched first, as a browser does (cookies), then posted.
                let _ = self.get(&loc);
                r = self.post(
                    "/admin/login",
                    &[("username", user), ("password", password), ("next", &next)],
                    None,
                );
                // sui-id limits `POST /admin/login` to 10 a minute per client address (`runtime/ratelimit.rs`: `Limiter::new(10, 60)`), and
                // every request of this test comes from 127.0.0.1: when it answers 429 the test waits for the window to pass.
                for _ in 0..3 {
                    if r.status != 429 {
                        break;
                    }
                    eprintln!(
                        "(the provider answered 429 to the login form: waiting 61 s for its 10-a-minute window)"
                    );
                    std::thread::sleep(Duration::from_secs(61));
                    r = self.post(
                        "/admin/login",
                        &[("username", user), ("password", password), ("next", &next)],
                        None,
                    );
                }
                if r.status == 200 {
                    // The form came back: the sign-in was refused.
                    return Callback {
                        code: None,
                        state: None,
                        asked_to_sign_in: true,
                        last: Some((r.status, "the login form was shown again".to_owned())),
                    };
                }
                continue;
            }
            r = self.get(&loc);
        }
        panic!("too many redirects from {url}");
    }

    /// The provider's token endpoint, called directly (the test's own PKCE; `basic` for the confidential client).
    fn token(&self, fields: &[(&str, &str)], basic: Option<(&str, &str)>) -> Resp {
        self.post("/oauth2/token", fields, basic)
    }
}

/// An authorization request of the test's own (a hand-made URL), with `extra` parameters.
fn own_url(
    c: &Cfg,
    client: &str,
    challenge: &str,
    nonce: &str,
    state: &str,
    extra: &str,
) -> String {
    format!(
        "{}/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256{extra}",
        c.issuer,
        pct_encode(client),
        pct_encode(&c.redirect_uri),
        pct_encode("openid email profile"),
        pct_encode(state),
        pct_encode(nonce),
        pct_encode(challenge)
    )
}

/// The provider's request log as the TLS stand-in wrote it (`ctl/requests.log`), from `since` (unix seconds as a float) on.
fn proxy_log(c: &Cfg, since: f64) -> Vec<Value> {
    std::fs::read_to_string(c.ctl.join("requests.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["t"].as_f64().unwrap_or(0.0) >= since)
        .collect()
}

fn t_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn admin(c: &Cfg, args: &[&str]) -> String {
    let (prog, rest) = c.admin.split_first().expect("the admin driver");
    let out = Command::new(prog)
        .args(rest)
        .args(args)
        .output()
        .expect("run the admin driver");
    assert!(
        out.status.success(),
        "admin {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The driver prints one JSON line; only what the administrator's action answered is kept (no page text, no secret).
    let text = String::from_utf8_lossy(&out.stdout);
    let status = text.split("\"status\": ").nth(1).map(|r| {
        r.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
    });
    match status {
        Some(st) => format!("HTTP {st}"),
        None => text.chars().take(60).collect(),
    }
}

fn ctl_set(c: &Cfg, name: &str, body: &str) {
    std::fs::write(c.ctl.join(name), body).unwrap();
}

fn ctl_clear(c: &Cfg, name: &str) {
    let _ = std::fs::remove_file(c.ctl.join(name));
}

/// Removes control files (and resumes the provider) on every way out of a test, including a panic.
struct Restore<'a>(&'a Cfg, &'static [&'static str], bool);

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        for n in self.1 {
            ctl_clear(self.0, n);
        }
        if self.2 {
            let _ = Command::new(&self.0.admin[0])
                .args(&self.0.admin[1..])
                .args(["signal-provider", "CONT"])
                .output();
        }
    }
}

/// One whole sign-in through planeter's provider code: begin, the browser step, finish. Returns what planeter got and the ID token the provider
/// issued (read from planeter's own fetch log, not by a second request).
struct SignIn {
    who: Result<VerifiedIdentity, OidcError>,
    id_token: Option<String>,
    token_response_bytes: usize,
    begin_url: String,
    callback: Callback,
    planeter_ms: u128,
    browser_ms: u128,
}

fn sign_in(c: &Cfg, rig: &Rig, b: &Browser, user: &str, password: &str) -> SignIn {
    let t = Instant::now();
    let begin = rig.provider.begin_login(now()).expect("begin_login");
    let mut planeter_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let cb = b.authorize(&begin.authorization_url, user, password);
    let browser_ms = t.elapsed().as_millis();
    let _ = c;
    let code = cb.code.clone().expect("the provider returned no code");
    assert_eq!(cb.state.as_deref(), Some(begin.state.as_str()));
    let t = Instant::now();
    let who = rig.provider.finish_login(&begin.state, &code, now());
    planeter_ms += t.elapsed().as_millis();
    let (id_token, bytes) = rig
        .exchanges()
        .iter()
        .rev()
        .find(|e| {
            e.form
                .as_deref()
                .is_some_and(|f| f.contains("grant_type=authorization_code"))
        })
        .and_then(|e| e.body.as_ref().ok())
        .map(|b| {
            let v: Value = serde_json::from_slice(b).unwrap_or(Value::Null);
            (v["id_token"].as_str().map(str::to_owned), b.len())
        })
        .unwrap_or((None, 0));
    SignIn {
        who,
        id_token,
        token_response_bytes: bytes,
        begin_url: begin.authorization_url,
        callback: cb,
        planeter_ms,
        browser_ms,
    }
}

// ================================================================================================ the checks

/// 1. Discovery parses, the issuer is exact, the endpoints are on the issuer's authority, the algorithms (ID-2).
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c01_discovery_and_jwks() {
    need!(c);
    let mut r = Report::new("c01_discovery_and_jwks");
    let work = work_dir(&c, "c01");
    let rig = rig(&c, &work, Kind::Public, false);
    let b = Browser::new(&c, &work, "c01");
    rig.provider
        .begin_login(now())
        .expect("planeter's discovery over the one-host rule");
    let ex = rig.exchanges();
    let disc_body = ex[0].body.as_ref().unwrap();
    let d: Value = serde_json::from_slice(disc_body).unwrap();
    r.check(
        "discovery parses, `issuer` equals the configured issuer exactly (ID-2/D-12)",
        "issuer == configured issuer, byte for byte",
        format!("issuer = {:?}, configured = {:?}", d["issuer"], c.issuer),
        d["issuer"] == c.issuer.as_str(),
    );
    let auth = planeter_core::url_authority(&c.issuer);
    let mut all_on = true;
    for k in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
        let on = planeter_core::url_authority(d[k].as_str().unwrap_or("")) == auth;
        all_on &= on;
        r.note(format!(
            "    {k} = {} (on the issuer's authority: {on})",
            d[k]
        ));
    }
    r.check(
        "the endpoints are on the issuer's authority (the one-host rule holds; planeter's `with_endpoints_on_issuer_host` accepted it)",
        "authorization, token and jwks endpoints on the issuer's host and port",
        format!("all on the issuer's authority: {all_on}"),
        all_on,
    );
    r.check(
        "the algorithm the document lists (ID-2: EdDSA only)",
        "id_token_signing_alg_values_supported == [\"EdDSA\"]",
        format!("{}", d["id_token_signing_alg_values_supported"]),
        d["id_token_signing_alg_values_supported"] == serde_json::json!(["EdDSA"]),
    );
    r.note(format!("    discovery keys: {:?}", keys_of(&d)));
    for k in [
        "token_endpoint_auth_methods_supported",
        "code_challenge_methods_supported",
        "grant_types_supported",
        "scopes_supported",
        "response_types_supported",
        "subject_types_supported",
    ] {
        r.note(format!("    {k} = {}", d[k]));
    }
    r.check(
        "the token endpoint lists `basic`, `post` and `none` (the sui-id reply, ID-8 context)",
        "client_secret_basic, client_secret_post, none",
        format!("{}", d["token_endpoint_auth_methods_supported"]),
        d["token_endpoint_auth_methods_supported"]
            == serde_json::json!(["client_secret_basic", "client_secret_post", "none"]),
    );
    r.check(
        "PKCE: S256 only",
        "code_challenge_methods_supported == [\"S256\"]",
        format!("{}", d["code_challenge_methods_supported"]),
        d["code_challenge_methods_supported"] == serde_json::json!(["S256"]),
    );
    // The key set.
    let jwks = b.get(d["jwks_uri"].as_str().unwrap());
    let set: Value = serde_json::from_str(&jwks.body).unwrap();
    r.note(format!(
        "    JWKS: HTTP {}, {} bytes, Cache-Control: {:?}",
        jwks.status,
        jwks.body.len(),
        jwks.header("cache-control")
    ));
    for k in set["keys"].as_array().unwrap() {
        r.note(format!(
            "    key kid={} kty={} crv={} alg={} use={}",
            k["kid"], k["kty"], k["crv"], k["alg"], k["use"]
        ));
    }
    let all_ed = set["keys"]
        .as_array()
        .unwrap()
        .iter()
        .all(|k| k["kty"] == "OKP" && k["crv"] == "Ed25519" && k["alg"] == "EdDSA");
    r.check(
        "the key set holds Ed25519 (OKP) keys only (ID-2)",
        "every key OKP / Ed25519 / EdDSA",
        format!("all: {all_ed}"),
        all_ed,
    );
    r.save(&c);
}

/// 2. The authorization-code flow with PKCE S256, the public client and the confidential client (`client_secret_basic`).
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c02_code_flow_public_and_confidential() {
    need!(c);
    let mut r = Report::new("c02_code_flow_public_and_confidential");
    let work = work_dir(&c, "c02");
    for (kind, label) in [
        (Kind::Public, "public"),
        (Kind::Confidential, "confidential"),
    ] {
        let rig = rig(&c, &work, kind, false);
        let b = Browser::new(&c, &work, label);
        let s = sign_in(&c, &rig, &b, &c.user, &c.password);
        let who = s.who.as_ref().expect("the sign-in completes");
        r.note(format!(
            "  [{label}] sign-in completed: issuer={:?} sub={:?} (the provider asked for a sign-in on the way: {})",
            who.issuer, who.subject, s.callback.asked_to_sign_in
        ));
        let tok = s
            .id_token
            .clone()
            .expect("an ID token in the token response");
        let (h, p) = jwt(&tok);
        r.check(
            &format!("[{label}] the ID token verifies against the JWKS with EdDSA (planeter's verification passed)"),
            "alg == EdDSA, kid in the JWKS",
            format!("alg={} kid={}", h["alg"], h["kid"]),
            h["alg"] == "EdDSA" && h["kid"].is_string(),
        );
        let nonce = query(&s.begin_url, "nonce").unwrap();
        let aud_ok = p["aud"] == rig.client.as_str()
            || p["aud"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| *x == rig.client.as_str()));
        r.check(
            &format!("[{label}] iss, aud, exp and the nonce"),
            "iss == issuer, aud holds the client id, exp in the future, nonce == the one planeter sent",
            format!(
                "iss={} aud={} exp-now={}s nonce-equal={}",
                p["iss"],
                p["aud"],
                p["exp"].as_i64().unwrap_or(0) - now() as i64,
                p["nonce"] == nonce.as_str()
            ),
            p["iss"] == c.issuer.as_str()
                && aud_ok
                && p["exp"].as_u64().unwrap_or(0) > now()
                && p["nonce"] == nonce.as_str(),
        );
        let tokreq = rig
            .exchanges()
            .into_iter()
            .find(|e| e.form.is_some())
            .unwrap();
        let form = tokreq.form.unwrap();
        let has_secret_in_body = form.contains("client_secret");
        match kind {
            Kind::Confidential => r.check(
                "[confidential] the secret goes by `client_secret_basic` (the Authorization header), not in the body",
                "Authorization: Basic present; no client_secret in the form",
                format!("basic={} secret-in-body={has_secret_in_body}", tokreq.basic),
                tokreq.basic && !has_secret_in_body,
            ),
            Kind::Public => r.check(
                "[public] PKCE only: no secret, no Authorization header",
                "no Authorization header; the form carries client_id and code_verifier",
                format!(
                    "basic={} client_id-in-body={} code_verifier-in-body={}",
                    tokreq.basic,
                    form.contains("client_id="),
                    form.contains("code_verifier=")
                ),
                !tokreq.basic && form.contains("client_id=") && form.contains("code_verifier="),
            ),
        }
        assert!(
            query(&s.begin_url, "code_challenge_method").as_deref() == Some("S256"),
            "planeter sends S256"
        );
    }
    r.save(&c);
}

/// 3. The claims of the ID token; `sub`; `email_verified`; no `name`; no userinfo request.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c03_claims_sub_email_and_no_userinfo() {
    need!(c);
    let mut r = Report::new("c03_claims_sub_email_and_no_userinfo");
    let work = work_dir(&c, "c03");
    let t0 = t_now();
    let rig1 = rig(&c, &work, Kind::Public, false);
    let b = Browser::new(&c, &work, "c03");
    let s1 = sign_in(&c, &rig1, &b, &c.user, &c.password);
    let (h1, p1) = jwt(s1.id_token.as_ref().unwrap());
    let claims = keys_of(&p1);
    r.note(format!("    header: {h1}"));
    r.note(format!(
        "    claim names of the ID token (user with an email): {claims:?}"
    ));
    let expected = [
        "acr",
        "amr",
        "aud",
        "email",
        "email_verified",
        "exp",
        "iat",
        "iss",
        "jti",
        "nonce",
        "sub",
    ];
    r.check(
        "the exact claim set (ID-2): iss, sub, aud, iat, exp, nonce, jti, acr, amr, and email/email_verified under the email scope",
        &format!("{expected:?}"),
        format!("{claims:?}"),
        claims == expected,
    );
    for k in ["acr", "amr", "jti", "email", "email_verified"] {
        r.note(format!("    {k} = {}", p1[k]));
    }
    r.check(
        "no `name` and no `preferred_username` in the ID token (ID-2: they live at userinfo)",
        "neither claim present (the scope `profile` was requested by planeter)",
        format!(
            "name={} preferred_username={}",
            p1["name"], p1["preferred_username"]
        ),
        p1["name"].is_null() && p1["preferred_username"].is_null(),
    );
    r.check(
        "`email_verified` is false (ID-1)",
        "email_verified == false",
        format!("{}", p1["email_verified"]),
        p1["email_verified"] == false,
    );
    let sub = p1["sub"].as_str().unwrap_or("").to_owned();
    let uuid_ok = sub.len() == 36
        && sub.chars().enumerate().all(|(i, ch)| {
            if [8, 13, 18, 23].contains(&i) {
                ch == '-'
            } else {
                ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()
            }
        });
    r.check(
        "`sub` is 36 characters, the hyphenated lowercase UUID form (ID-6)",
        "36 chars, 8-4-4-4-12 lowercase hex",
        format!(
            "{sub:?} (len {}), version nibble {:?}",
            sub.len(),
            sub.chars().nth(14)
        ),
        uuid_ok,
    );
    // A second sign-in by the same user, in a new browser and a new provider object.
    let rig2 = rig(&c, &work, Kind::Public, false);
    let b2 = Browser::new(&c, &work, "c03b");
    let s2 = sign_in(&c, &rig2, &b2, &c.user, &c.password);
    let (_, p2) = jwt(s2.id_token.as_ref().unwrap());
    r.check(
        "`sub` is the same on a second sign-in by the same user (ID-6)",
        "equal",
        format!("first={} second={}", p1["sub"], p2["sub"]),
        p1["sub"] == p2["sub"],
    );
    r.check(
        "`jti` differs between the two ID tokens (an identifier of this token)",
        "different",
        format!("{} / {}", p1["jti"], p2["jti"]),
        p1["jti"] != p2["jti"],
    );
    // planeter stores no email from it: the account row is unchanged by an OIDC sign-in.
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account::local("alice", "Alice", None))
        .unwrap();
    accounts
        .link_oidc(&c.issuer, &sub, UserId::new("alice"))
        .unwrap();
    let before = format!("{:?}", accounts.get(&UserId::new("alice")).unwrap());
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Authenticator::new(
        accounts.clone(),
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    );
    let who = s1.who.as_ref().unwrap();
    let principal = auth.authenticate_oidc(who);
    let after = format!("{:?}", accounts.get(&UserId::new("alice")).unwrap());
    r.note(format!(
        "    what planeter's provider code returned to the authenticator: issuer={:?} subject={:?} email={:?}",
        who.issuer, who.subject, who.email
    ));
    r.check(
        "planeter stores no email from the ID token: the account row is unchanged by the sign-in (ID-1)",
        "the linked account (no email) is signed in and its row is byte-identical afterwards",
        format!("principal={principal:?}; row unchanged={}", before == after),
        matches!(principal, Ok(Principal::User(_))) && before == after,
    );
    // The user with no email: which claims.
    let rig3 = rig(&c, &work, Kind::Public, false);
    let b3 = Browser::new(&c, &work, "c03c");
    let s3 = sign_in(&c, &rig3, &b3, &c.noemail_user, &c.noemail_password);
    let (_, p3) = jwt(s3.id_token.as_ref().unwrap());
    r.note(format!(
        "    claim names of the ID token (user WITHOUT an email): {:?}",
        keys_of(&p3)
    ));
    // The provider's own count: no userinfo request by planeter.
    let all: Vec<Exchange> = [&rig1, &rig2, &rig3]
        .iter()
        .flat_map(|g| g.exchanges())
        .collect();
    let userinfo = all.iter().filter(|e| e.url.contains("userinfo")).count();
    let seen = proxy_log(&c, t0)
        .into_iter()
        .filter(|v| v["ua"] == "planeter")
        .collect::<Vec<_>>();
    let paths: Vec<String> = seen
        .iter()
        .map(|v| format!("{} {}", v["method"], v["path"]))
        .collect();
    r.note(format!(
        "    the provider's log of requests with User-Agent `planeter` (3 sign-ins): {paths:?}"
    ));
    r.check(
        "planeter makes no userinfo request (ID-2); three requests per sign-in",
        "0 userinfo requests; discovery, token, JWKS per sign-in",
        format!(
            "userinfo requests by planeter: {userinfo} (its own log), {} at the provider; total requests 3 sign-ins: {}",
            seen.iter().filter(|v| v["path"].as_str().unwrap_or("").contains("userinfo")).count(),
            seen.len()
        ),
        userinfo == 0
            && seen.iter().all(|v| !v["path"].as_str().unwrap_or("").contains("userinfo"))
            && seen.len() == 9,
    );
    r.save(&c);
}

/// 4. A replayed code, an expired code, a wrong verifier, a wrong state, a wrong nonce.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c04_replay_expiry_verifier_state_nonce() {
    need!(c);
    let mut r = Report::new("c04_replay_expiry_verifier_state_nonce");
    let work = work_dir(&c, "c04");
    let b = Browser::new(&c, &work, "c04");
    let (ver, chal) = pkce();
    let basic = Some((
        c.confidential_client.as_str(),
        c.confidential_secret.as_str(),
    ));
    let redirect = c.redirect_uri.as_str();
    // The provider: a replayed code.
    let u = own_url(
        &c,
        &c.confidential_client,
        &chal,
        "n-replay",
        "s-replay",
        "",
    );
    let cb = b.authorize(&u, &c.user, &c.password);
    let code = cb.code.clone().expect("a code");
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect),
        ("code_verifier", ver.as_str()),
    ];
    let first = b.token(&form, basic);
    let second = b.token(&form, basic);
    r.check(
        "the provider refuses a replayed code (the second exchange fails)",
        "first 200, second 400 (invalid_grant)",
        format!(
            "first={} second={} body={:?}",
            first.status,
            second.status,
            second.body.chars().take(120).collect::<String>()
        ),
        first.status == 200 && second.status >= 400,
    );
    // A wrong code_verifier.
    let (ver2, chal2) = pkce();
    let u = own_url(&c, &c.confidential_client, &chal2, "n-ver", "s-ver", "");
    let code2 = b.authorize(&u, &c.user, &c.password).code.expect("a code");
    let wrong = b.token(
        &[
            ("grant_type", "authorization_code"),
            ("code", code2.as_str()),
            ("redirect_uri", redirect),
            (
                "code_verifier",
                "not-the-verifier-not-the-verifier-not-the-verifier",
            ),
        ],
        basic,
    );
    r.check(
        "the provider refuses a wrong code_verifier",
        "400 invalid_grant",
        format!(
            "{} {:?}",
            wrong.status,
            wrong.body.chars().take(120).collect::<String>()
        ),
        wrong.status >= 400,
    );
    // ...and the code is spent by the failed attempt or still good with the right verifier: record which.
    let right_after = b.token(
        &[
            ("grant_type", "authorization_code"),
            ("code", code2.as_str()),
            ("redirect_uri", redirect),
            ("code_verifier", ver2.as_str()),
        ],
        basic,
    );
    r.note(format!(
        "    (after the wrong verifier, the right one on the same code: HTTP {}: the failed attempt {} the code)",
        right_after.status,
        if right_after.status >= 400 { "spent" } else { "did not spend" }
    ));
    // An expired code (the provider's lifetime is 60 s in its source, `AUTH_CODE_LIFETIME_SECS`: waited out).
    let (ver3, chal3) = pkce();
    let u = own_url(&c, &c.confidential_client, &chal3, "n-exp", "s-exp", "");
    let code3 = b.authorize(&u, &c.user, &c.password).code.expect("a code");
    std::thread::sleep(Duration::from_secs(65));
    let late = b.token(
        &[
            ("grant_type", "authorization_code"),
            ("code", code3.as_str()),
            ("redirect_uri", redirect),
            ("code_verifier", ver3.as_str()),
        ],
        basic,
    );
    r.check(
        "the provider refuses an expired code (waited 65 s; the provider's lifetime is 60 s in its source)",
        "400 invalid_grant after 60 s",
        format!("{} {:?}", late.status, late.body.chars().take(120).collect::<String>()),
        late.status >= 400,
    );
    // planeter's side.
    let rig = rig(&c, &work, Kind::Confidential, false);
    let begin = rig.provider.begin_login(now()).unwrap();
    let cb = b.authorize(&begin.authorization_url, &c.user, &c.password);
    let good_code = cb.code.clone().unwrap();
    let n_before = rig.exchanges().len();
    let wrong_state = rig
        .provider
        .finish_login("not-a-state-planeter-issued", &good_code, now());
    r.check(
        "planeter refuses a wrong `state` (before any request to the provider)",
        "Err(State), and no request made",
        format!(
            "{wrong_state:?}; requests made by the refusal: {}",
            rig.exchanges().len() - n_before
        ),
        matches!(wrong_state, Err(OidcError::State)) && rig.exchanges().len() == n_before,
    );
    let ok = rig.provider.finish_login(&begin.state, &good_code, now());
    let replay = rig.provider.finish_login(&begin.state, &good_code, now());
    r.check(
        "planeter refuses a replayed state/code (the second finish)",
        "first Ok, second Err(State)",
        format!("first ok={} second={replay:?}", ok.is_ok()),
        ok.is_ok() && matches!(replay, Err(OidcError::State)),
    );
    // A code from another flow under a fresh state: the provider refuses (the code is bound to the other challenge).
    let begin2 = rig.provider.begin_login(now()).unwrap();
    let u1 = rig.provider.begin_login(now()).unwrap();
    let foreign = b
        .authorize(&u1.authorization_url, &c.user, &c.password)
        .code
        .unwrap();
    let crossed = rig.provider.finish_login(&begin2.state, &foreign, now());
    r.check(
        "a code issued to another flow (another PKCE challenge) is refused by the provider and planeter signs nobody in",
        "Err(Provider(..))",
        format!("{crossed:?}"),
        crossed.is_err(),
    );
    // A wrong nonce: the authorization URL is edited so that the provider puts another nonce in the ID token.
    let rig = rig_again(&c, &work);
    let begin = rig.provider.begin_login(now()).unwrap();
    let real_nonce = query(&begin.authorization_url, "nonce").unwrap();
    let edited = begin.authorization_url.replace(
        &format!("nonce={}", pct_encode(&real_nonce)),
        "nonce=another-nonce",
    );
    assert_ne!(edited, begin.authorization_url);
    let cb = b.authorize(&edited, &c.user, &c.password);
    let wrong_nonce = rig
        .provider
        .finish_login(&begin.state, &cb.code.unwrap(), now());
    r.check(
        "planeter refuses an ID token whose nonce is not the one it sent",
        "Err(Invalid)",
        format!("{wrong_nonce:?}"),
        matches!(wrong_nonce, Err(OidcError::Invalid)),
    );
    r.save(&c);
}

fn rig_again(c: &Cfg, work: &Path) -> Rig {
    rig(c, work, Kind::Public, false)
}

/// 5. `prompt=login` and `max_age=0` with a live provider session (ID-4): what the provider does, and that planeter's claim is unchanged.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c05_prompt_login_is_not_read_as_a_recent_sign_in() {
    need!(c);
    let mut r = Report::new("c05_prompt_login_is_not_read_as_a_recent_sign_in");
    let work = work_dir(&c, "c05");
    let b = Browser::new(&c, &work, "c05");
    // A live provider session: sign in once.
    let (_, chal) = pkce();
    let first = b.authorize(
        &own_url(&c, &c.public_client, &chal, "n1", "s1", ""),
        &c.user,
        &c.password,
    );
    r.note(format!(
        "    first request (no session): asked to sign in = {}, code = {}",
        first.asked_to_sign_in,
        first.code.is_some()
    ));
    // Without prompt: a live session answers at once.
    let (_, chal) = pkce();
    let plain = b.get(&own_url(&c, &c.public_client, &chal, "n2", "s2", ""));
    r.note(format!(
        "    second request, live session, no prompt: HTTP {} -> {:?}",
        plain.status,
        plain
            .header("location")
            .map(|l| l.chars().take(70).collect::<String>())
    ));
    // With prompt=login & max_age=0.
    let (ver, chal) = pkce();
    let url = own_url(
        &c,
        &c.public_client,
        &chal,
        "n3",
        "s3",
        "&prompt=login&max_age=0",
    );
    let resp = b.get(&url);
    let loc = resp.header("location").unwrap_or("").to_owned();
    let returned_code = loc.starts_with(&c.redirect_uri) && query(&loc, "code").is_some();
    let asked = loc.starts_with("/admin/login");
    r.check(
        "ID-4: with a live provider session, `prompt=login` + `max_age=0` get a code with no new sign-in (the request silently ignores them)",
        "a redirect to the callback with a code, no login form",
        format!("HTTP {} -> {:?}; returned a code at once: {returned_code}; asked for a new sign-in: {asked}", resp.status, loc.chars().take(90).collect::<String>()),
        returned_code && !asked,
    );
    if let Some(code) = query(&loc, "code") {
        let t = b.token(
            &[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", c.redirect_uri.as_str()),
                ("code_verifier", ver.as_str()),
                ("client_id", c.public_client.as_str()),
            ],
            None,
        );
        let tok: Value = serde_json::from_str(&t.body).unwrap_or(Value::Null);
        let (_, p) = jwt(tok["id_token"].as_str().unwrap_or(""));
        r.check(
            "what the ID token carries: no `auth_time` (ID-4)",
            "no auth_time claim",
            format!("claims = {:?}", keys_of(&p)),
            p.get("auth_time").is_none(),
        );
    }
    // planeter sends neither parameter, and its claim is its own session's age.
    let rig = rig(&c, &work, Kind::Public, false);
    let begin = rig.provider.begin_login(now()).unwrap();
    r.check(
        "planeter's own authorization URL carries neither `prompt` nor `max_age` (ID-4)",
        "neither present",
        format!(
            "prompt={:?} max_age={:?}",
            query(&begin.authorization_url, "prompt"),
            query(&begin.authorization_url, "max_age")
        ),
        query(&begin.authorization_url, "prompt").is_none()
            && query(&begin.authorization_url, "max_age").is_none(),
    );
    // A planeter sign-in whose authorization request carried them (hand-made): what planeter's session says about it.
    let hand = format!("{}&prompt=login&max_age=0", begin.authorization_url);
    let cb = b.authorize(&hand, &c.user, &c.password);
    let who = rig
        .provider
        .finish_login(&begin.state, &cb.code.clone().unwrap(), now())
        .expect("planeter completes the sign-in");
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account::local("alice", "Alice", None))
        .unwrap();
    accounts
        .link_oidc(&who.issuer, &who.subject, UserId::new("alice"))
        .unwrap();
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    );
    let sessions = InMemorySessionStore::new();
    let t_made = now();
    let principal = auth.authenticate_oidc(&who).unwrap();
    let Principal::User(u) = principal else {
        panic!("a user")
    };
    let s = sessions
        .create(u, t_made, t_made + sessions.ttl_secs())
        .unwrap();
    r.check(
        "planeter's claim (\"a sign-in to planeter within 15 minutes\") is unchanged: the session's age is planeter's own clock, and the provider's answer carries nothing it could be read from",
        "Session.created_at is the moment planeter made it; VerifiedIdentity holds issuer, subject and email only",
        format!(
            "created_at={} (planeter's clock {t_made}); identity fields: issuer, subject, email; the returned code came {} a new sign-in at the provider",
            s.created_at,
            if cb.asked_to_sign_in { "after" } else { "without" }
        ),
        s.created_at == t_made,
    );
    r.save(&c);
}

/// 6. Key rotation (ID-7): the JWKS header, the previous key, and the sign-in around the rotation.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c06_key_rotation_and_the_no_cache_refresh() {
    need!(c);
    let mut r = Report::new("c06_key_rotation_and_the_no_cache_refresh");
    let work = work_dir(&c, "c06");
    let _restore = Restore(&c, &["jwks-stale.json", "jwks-ignore-nocache"], false);
    let b = Browser::new(&c, &work, "c06");
    let jwks_url = format!("{}/.well-known/jwks.json", c.issuer);
    let kids = |body: &str| -> Vec<String> {
        let v: Value = serde_json::from_str(body).unwrap();
        v["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["kid"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = b.get(&jwks_url);
    let kids_before = kids(&before.body);
    r.check(
        "the key set is served `Cache-Control: public, max-age=300` (ID-7)",
        "public, max-age=300",
        format!("{:?}", before.header("cache-control")),
        before.header("cache-control") == Some("public, max-age=300"),
    );
    // (i) planeter's own cache holds the old set; the provider rotates; the next sign-in needs one refresh.
    let rig1 = rig(&c, &work, Kind::Public, false);
    let s0 = sign_in(&c, &rig1, &b, &c.user, &c.password);
    assert!(s0.who.is_ok());
    let n_before = rig1.exchanges().len();
    r.note(format!(
        "  rotation (i): administrator rotates the signing key: {}",
        admin(&c, &["rotate-key"])
            .chars()
            .take(60)
            .collect::<String>()
    ));
    let after = b.get(&jwks_url);
    let kids_after = kids(&after.body);
    r.note(format!(
        "    kids before: {kids_before:?}\n    kids after:  {kids_after:?}"
    ));
    r.check(
        "the previous key stays in the set after a rotation, and the new key is published at once (ID-7)",
        "every earlier kid still present; one new kid",
        format!("earlier kids kept: {}; new kids: {}", kids_before.iter().all(|k| kids_after.contains(k)), kids_after.iter().filter(|k| !kids_before.contains(k)).count()),
        kids_before.iter().all(|k| kids_after.contains(k)) && kids_after.iter().filter(|k| !kids_before.contains(k)).count() == 1,
    );
    let s1 = sign_in(&c, &rig1, &b, &c.user, &c.password);
    let ex = rig1.exchanges();
    let jwks_fetches: Vec<(bool, bool)> = ex[n_before..]
        .iter()
        .filter(|e| e.url.contains("jwks"))
        .map(|e| (e.no_cache, e.body.is_ok()))
        .collect();
    r.check(
        "(i) planeter's cache holds the old set: a sign-in whose ID token is signed by the NEW key succeeds through the one `no-cache` refresh",
        "Ok; exactly one JWKS fetch, with no_cache = true",
        format!("{:?}; JWKS fetches (no_cache, ok): {jwks_fetches:?}", s1.who.as_ref().map(|w| w.subject.clone())),
        s1.who.is_ok() && jwks_fetches == vec![(true, true)],
    );
    // (ii) a shared cache in front of the provider serves the old set to a request without no-cache.
    let stale = b.get(&jwks_url).body;
    ctl_set(&c, "jwks-stale.json", &stale);
    r.note(format!("  rotation (ii): a caching stand-in now serves the old set ({} keys) to requests without `no-cache`; administrator rotates again: {}", kids(&stale).len(), admin(&c, &["rotate-key"]).chars().take(40).collect::<String>()));
    let t0 = t_now();
    let rig2 = rig(&c, &work, Kind::Public, false);
    let s2 = sign_in(&c, &rig2, &b, &c.user, &c.password);
    let served: Vec<String> = proxy_log(&c, t0)
        .into_iter()
        .filter(|v| v["ua"] == "planeter" && v["path"].as_str().unwrap_or("").contains("jwks"))
        .map(|v| format!("nocache={} served={}", v["nocache"], v["served"]))
        .collect();
    r.check(
        "(ii) WITH the refresh: the stale set is served to the first fetch, the `no-cache` refresh goes through to the provider, and the sign-in succeeds",
        "Ok; JWKS requests: [stale, then upstream with no-cache]",
        format!("{:?}; the provider's log of JWKS requests by planeter: {served:?}", s2.who.as_ref().map(|w| w.subject.clone())),
        s2.who.is_ok() && served.len() == 2 && served[0].contains("stale") && served[1].contains("nocache=true") && served[1].contains("upstream"),
    );
    ctl_clear(&c, "jwks-stale.json");
    // (iii) the same, with the no-cache header removed by the wrapper (planeter before the fix).
    let stale = b.get(&jwks_url).body;
    ctl_set(&c, "jwks-stale.json", &stale);
    admin(&c, &["rotate-key"]);
    let t0 = t_now();
    let rig3 = rig(&c, &work, Kind::Public, true);
    let begin = rig3.provider.begin_login(now()).unwrap();
    let cb = b.authorize(&begin.authorization_url, &c.user, &c.password);
    let res = rig3
        .provider
        .finish_login(&begin.state, &cb.code.unwrap(), now());
    let served: Vec<String> = proxy_log(&c, t0)
        .into_iter()
        .filter(|v| v["ua"] == "planeter" && v["path"].as_str().unwrap_or("").contains("jwks"))
        .map(|v| format!("nocache={} served={}", v["nocache"], v["served"]))
        .collect();
    r.check(
        "(iii) WITHOUT the header (the wrapper strips it): both fetches get the stale set, the new key is never found, and planeter refuses the sign-in",
        "Err(Invalid); JWKS requests: [stale, stale]",
        format!("{res:?}; the provider's log: {served:?}"),
        matches!(res, Err(OidcError::Invalid)) && served.len() == 2 && served.iter().all(|s| s.contains("stale")),
    );
    ctl_clear(&c, "jwks-stale.json");
    r.save(&c);
}

/// 7. The provider slow, down, or answering garbage: planeter refuses the sign-in and says so; never a hang.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c07_a_provider_that_is_slow_down_or_garbage_never_signs_anyone_in_and_never_hangs() {
    need!(c);
    let mut r = Report::new(
        "c07_a_provider_that_is_slow_down_or_garbage_never_signs_anyone_in_and_never_hangs",
    );
    let work = work_dir(&c, "c07");
    let _restore = Restore(
        &c,
        &["down", "token-delay-secs", "discovery-override"],
        true,
    );
    let b = Browser::new(&c, &work, "c07");
    let bound = Duration::from_secs(25);
    // (a) the token endpoint slower than planeter's 15 s bound.
    let rig_a = rig(&c, &work, Kind::Public, false);
    let begin = rig_a.provider.begin_login(now()).unwrap();
    let code = b
        .authorize(&begin.authorization_url, &c.user, &c.password)
        .code
        .unwrap();
    ctl_set(&c, "token-delay-secs", "20");
    let t = Instant::now();
    let res = rig_a.provider.finish_login(&begin.state, &code, now());
    let took = t.elapsed();
    r.check(
        "(a) a token endpoint slower than planeter's 15-second bound: the sign-in is refused, with the error said",
        "Err(Provider(..)) after about 15 s, never a hang",
        format!("{res:?} after {} ms", took.as_millis()),
        matches!(res, Err(OidcError::Provider(_))) && took < bound,
    );
    assert!(
        res.is_err() && took < bound,
        "never a hang, never signed in"
    );
    ctl_clear(&c, "token-delay-secs");
    // (b) the provider goes down between the redirect and the token exchange.
    let rig_b = rig(&c, &work, Kind::Public, false);
    let begin = rig_b.provider.begin_login(now()).unwrap();
    let code = b
        .authorize(&begin.authorization_url, &c.user, &c.password)
        .code
        .unwrap();
    ctl_set(&c, "down", "");
    let t = Instant::now();
    let res = rig_b.provider.finish_login(&begin.state, &code, now());
    let took = t.elapsed();
    r.check(
        "(b) the provider goes down mid-flow (after discovery, before the token exchange): the sign-in is refused",
        "Err(Provider(..)) at once",
        format!("{res:?} after {} ms", took.as_millis()),
        matches!(res, Err(OidcError::Provider(_))) && took < bound,
    );
    assert!(res.is_err() && took < bound);
    // (c) the provider is down at the start.
    let rig_c = rig(&c, &work, Kind::Public, false);
    let t = Instant::now();
    let res = rig_c.provider.begin_login(now());
    let took = t.elapsed();
    r.check(
        "(c) the provider is down when the sign-in starts (no discovery): the sign-in is refused",
        "Err(Provider(..)) at once",
        format!("{res:?} after {} ms", took.as_millis()),
        matches!(res, Err(OidcError::Provider(_))) && took < bound,
    );
    assert!(res.is_err() && took < bound);
    ctl_clear(&c, "down");
    // (d) discovery that is not JSON; (e) discovery of another issuer.
    for (label, body, expect) in [
        (
            "(d) a discovery document that is not JSON",
            "<html>this is not json</html>".to_owned(),
            "bad JSON",
        ),
        (
            "(e) a discovery document that names another issuer",
            format!(
                "{{\"issuer\":\"https://evil.example\",\"authorization_endpoint\":\"{0}/a\",\"token_endpoint\":\"{0}/t\",\"jwks_uri\":\"{0}/j\"}}",
                c.issuer
            ),
            "issuer mismatch",
        ),
    ] {
        ctl_set(&c, "discovery-override", &body);
        let rig_d = rig(&c, &work, Kind::Public, false);
        let t = Instant::now();
        let res = rig_d.provider.begin_login(now());
        r.check(
            &format!("{label}: the sign-in is refused, with the error said"),
            &format!("Err(Provider(\"…{expect}…\"))"),
            format!("{res:?} after {} ms", t.elapsed().as_millis()),
            matches!(&res, Err(OidcError::Provider(m)) if m.contains(expect)),
        );
        assert!(res.is_err());
        ctl_clear(&c, "discovery-override");
    }
    // (f) the real provider process stopped (SIGSTOP): it accepts the connection and never answers.
    let rig_f = rig(&c, &work, Kind::Public, false);
    admin(&c, &["signal-provider", "STOP"]);
    let t = Instant::now();
    let res = rig_f.provider.begin_login(now());
    let took = t.elapsed();
    admin(&c, &["signal-provider", "CONT"]);
    r.check(
        "(f) the provider process itself stopped (SIGSTOP; the connection is accepted, nothing is answered): the sign-in is refused after the bound",
        "Err(Provider(..)) after about 15 s, never a hang",
        format!("{res:?} after {} ms", took.as_millis()),
        matches!(res, Err(OidcError::Provider(_))) && took < bound,
    );
    assert!(res.is_err() && took < bound);
    r.note("    in every case above no principal was produced: `finish_login`/`begin_login` returned an error and the authenticator was never reached");
    r.save(&c);
}

/// 8. Offboarding (ID-5): disable the user at the provider and measure what planeter does.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c08_offboarding_a_disabled_user() {
    need!(c);
    let mut r = Report::new("c08_offboarding_a_disabled_user");
    let work = work_dir(&c, "c08");
    let b = Browser::new(&c, &work, "c08");
    let rig = rig(&c, &work, Kind::Confidential, false);
    // Sign in through the provider into a planeter session, as the real handlers do (in process).
    let s = sign_in(&c, &rig, &b, &c.user, &c.password);
    let who = s.who.as_ref().expect("a sign-in").clone();
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account::local("interop", "Interop", None))
        .unwrap();
    accounts
        .link_oidc(&who.issuer, &who.subject, UserId::new("interop"))
        .unwrap();
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    );
    let sessions = InMemorySessionStore::new();
    let t_made = now();
    let Principal::User(u) = auth.authenticate_oidc(&who).unwrap() else {
        panic!("a user")
    };
    let session = sessions
        .create(u, t_made, t_made + sessions.ttl_secs())
        .unwrap();
    r.note(format!("    planeter session made at {t_made}, lifetime {} h (the store's `ttl_secs`: PLANETER_SESSION_HOURS)", sessions.ttl_secs() / 3600));
    // The provider side before: a pending code, a refresh token, the provider's own session.
    let (ver, chal) = pkce();
    let basic = Some((
        c.confidential_client.as_str(),
        c.confidential_secret.as_str(),
    ));
    let pending = b
        .authorize(
            &own_url(&c, &c.confidential_client, &chal, "n-off", "s-off", ""),
            &c.user,
            &c.password,
        )
        .code
        .expect("a pending code");
    let (rver, rchal) = pkce();
    let rcode = b
        .authorize(
            &own_url(
                &c,
                &c.confidential_client,
                &rchal,
                "n-ref",
                "s-ref",
                "&access_type=offline",
            )
            .replace(
                "openid%20email%20profile",
                "openid%20email%20profile%20offline_access",
            ),
            &c.user,
            &c.password,
        )
        .code
        .expect("a code with offline_access");
    let tok = b.token(
        &[
            ("grant_type", "authorization_code"),
            ("code", rcode.as_str()),
            ("redirect_uri", c.redirect_uri.as_str()),
            ("code_verifier", rver.as_str()),
        ],
        basic,
    );
    let tokv: Value = serde_json::from_str(&tok.body).unwrap_or(Value::Null);
    let refresh = tokv["refresh_token"].as_str().map(str::to_owned);
    r.note(format!("    a refresh token was issued: {} (HTTP {}); this is a measurement of the provider's claim: planeter does not use refresh tokens", refresh.is_some(), tok.status));
    let ref_before = refresh.as_ref().map(|t| {
        b.token(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", t.as_str()),
            ],
            basic,
        )
    });
    if let Some(rb) = &ref_before {
        r.note(format!(
            "    refresh token before the disable: HTTP {}",
            rb.status
        ));
    }
    // The refresh token was rotated by the use above; take a new one if the response gave it.
    let refresh2 = ref_before
        .as_ref()
        .and_then(|x| serde_json::from_str::<Value>(&x.body).ok())
        .and_then(|v| v["refresh_token"].as_str().map(str::to_owned))
        .or(refresh);
    // Disable the user at the provider.
    r.note(format!(
        "    the administrator disables the user at sui-id: {}",
        admin(&c, &["disable-user", &c.user_id])
            .chars()
            .take(80)
            .collect::<String>()
    ));
    let still = sessions.get(&session.id, now() + 60).unwrap();
    r.check(
        "ID-5: the planeter session stays valid after the user is disabled at the provider, until its lifetime ends (planeter has no refresh check: RFC 014 open question 7, the owner's)",
        "the session is still found after the disable",
        format!("session found: {}; expires_at - now = {} h", still.is_some(), (session.expires_at.saturating_sub(now())) / 3600),
        still.is_some(),
    );
    // A new sign-in through the provider fails at the provider.
    let b2 = Browser::new(&c, &work, "c08-new");
    let t0 = t_now();
    let begin = rig.provider.begin_login(now()).unwrap();
    let cb = b2.authorize(&begin.authorization_url, &c.user, &c.password);
    r.check(
        "ID-5: a NEW sign-in through the provider fails at the provider (the login is refused, no code)",
        "no code; the login form shown again",
        format!("code: {:?}; last: {:?}", cb.code, cb.last),
        cb.code.is_none(),
    );
    let _ = t0;
    // The provider session that existed before the disable.
    let (_, chal2) = pkce();
    let old = b.get(&own_url(&c, &c.public_client, &chal2, "n-old", "s-old", ""));
    let old_loc = old.header("location").unwrap_or("").to_owned();
    r.check(
        "ID-5: the user's own provider session is revoked in the same transaction (the provider's claim)",
        "an authorization request with the cookie jar of the earlier sign-in is sent to the login form, not answered with a code",
        format!(
            "HTTP {} -> {:?}",
            old.status,
            old_loc.chars().take(40).collect::<String>()
        ),
        old_loc.starts_with("/admin/login"),
    );
    // The pending code.
    let pend = b.token(
        &[
            ("grant_type", "authorization_code"),
            ("code", pending.as_str()),
            ("redirect_uri", c.redirect_uri.as_str()),
            ("code_verifier", ver.as_str()),
        ],
        basic,
    );
    r.check(
        "ID-5: a pending code is revoked in the same transaction (the provider's claim)",
        "the exchange of a code issued before the disable fails",
        format!(
            "HTTP {} {:?}",
            pend.status,
            pend.body.chars().take(100).collect::<String>()
        ),
        pend.status >= 400,
    );
    match refresh2 {
        Some(t) => {
            let after = b.token(
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", t.as_str()),
                ],
                basic,
            );
            r.check(
                "ID-5: the refresh token is revoked in the same transaction (the provider's claim; planeter does not use refresh tokens)",
                "the use of a refresh token after the disable fails",
                format!("before: {:?}; after: HTTP {} {:?}", ref_before.as_ref().map(|x| x.status), after.status, after.body.chars().take(100).collect::<String>()),
                after.status >= 400,
            );
        }
        None => r.not_shown(
            "the refresh token is revoked on disable (ID-5)",
            "no refresh token could be obtained from the provider in this setup",
        ),
    }
    r.note("    planeter has no refresh check: nothing in planeter asks the provider again after the sign-in (RFC 014 open question 7, the owner's).");
    // Put the user back.
    r.note(format!(
        "    the administrator enables the user again: {}",
        admin(&c, &["enable-user", &c.user_id])
            .chars()
            .take(60)
            .collect::<String>()
    ));
    let b3 = Browser::new(&c, &work, "c08-back");
    let (_, chal3) = pkce();
    let back = b3.authorize(
        &own_url(&c, &c.public_client, &chal3, "n-back", "s-back", ""),
        &c.user,
        &c.password,
    );
    r.note(format!(
        "    after enabling again, a sign-in at once gets a code: {} (the refused login while disabled may have counted toward the provider's lockout)",
        back.code.is_some()
    ));
    if back.code.is_none() {
        r.note(format!(
            "    the administrator unlocks the user (`sui-id admin unlock-user`, offline): {}",
            admin(&c, &["unlock-user", &c.user])
        ));
        let (_, chal4) = pkce();
        let again = b3.authorize(
            &own_url(&c, &c.public_client, &chal4, "n-back2", "s-back2", ""),
            &c.user,
            &c.password,
        );
        r.note(format!(
            "    after the unlock, a sign-in gets a code: {}",
            again.code.is_some()
        ));
    }
    r.save(&c);
}

/// 9. Cost and size: the documents, the requests per sign-in, the time.
#[test]
#[ignore = "needs a running sui-id (see the head of the file)"]
fn c09_cost_and_size() {
    need!(c);
    let mut r = Report::new("c09_cost_and_size");
    let work = work_dir(&c, "c09");
    let b = Browser::new(&c, &work, "c09");
    let rig = rig(&c, &work, Kind::Public, false);
    let t0 = t_now();
    let s = sign_in(&c, &rig, &b, &c.user, &c.password);
    assert!(s.who.is_ok());
    let ex = rig.exchanges();
    let disc = ex
        .iter()
        .find(|e| e.url.contains("openid-configuration"))
        .unwrap();
    let jw = ex.iter().find(|e| e.url.contains("jwks")).unwrap();
    let tokr = ex.iter().find(|e| e.form.is_some()).unwrap();
    let idt = s.id_token.clone().unwrap();
    r.note(format!(
        "    discovery document: {} bytes ({} ms)",
        disc.body.as_ref().map(Vec::len).unwrap_or(0),
        disc.ms
    ));
    r.note(format!(
        "    JWKS: {} bytes ({} ms; {} keys now)",
        jw.body.as_ref().map(Vec::len).unwrap_or(0),
        jw.ms,
        serde_json::from_slice::<Value>(jw.body.as_ref().unwrap())
            .map(|v| v["keys"].as_array().map(Vec::len).unwrap_or(0))
            .unwrap_or(0)
    ));
    r.note(format!(
        "    token response: {} bytes ({} ms); the ID token itself: {} bytes",
        s.token_response_bytes,
        tokr.ms,
        idt.len()
    ));
    let seen: Vec<String> = proxy_log(&c, t0)
        .into_iter()
        .filter(|v| v["ua"] == "planeter")
        .map(|v| {
            format!(
                "{} {} ({} B, {} ms)",
                v["method"], v["path"], v["bytes"], v["ms"]
            )
        })
        .collect();
    r.check(
        "the number of requests planeter makes per sign-in (expected: discovery, token, JWKS: three)",
        "3",
        format!("{} at planeter; {} at the provider: {seen:?}", ex.len(), seen.len()),
        ex.len() == 3 && seen.len() == 3,
    );
    r.note(format!(
        "    time of one sign-in against the local provider: planeter's part (begin + finish: discovery, token exchange, key fetch, verification) {} ms; the browser step (authorize, login form, redirects, with curl) {} ms",
        s.planeter_ms, s.browser_ms
    ));
    // A second sign-in with the caches warm (the same provider object).
    let s2 = sign_in(&c, &rig, &b, &c.user, &c.password);
    let n = rig.exchanges().len() - ex.len();
    r.note(format!(
        "    a second sign-in with the provider object's caches warm: {n} request(s) by planeter (the token exchange only), planeter's part {} ms",
        s2.planeter_ms
    ));
    r.save(&c);
}
