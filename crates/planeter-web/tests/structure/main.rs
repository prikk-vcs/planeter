//! RFC 012 D-2/D-5 (structure and semantics): every page and state the UI can produce is rendered through the
//! real router and parsed with an HTML5 parser (`html5ever`, a dev-dependency already in the lock through
//! `ammonia`); structural rules run over each. Each rule has a planted failure proving it can fail.
//!
//! Two subsets: the **stand-in** states need no prikk (no repository is read: the pages around the repositories,
//! the sign-in and its failures, the refusals) and always run; the **real-prikk** states build the baseline's
//! fixtures with a real `prikk` and self-skip, printing why, without it. `STRUCTURE_REPORT=<file>` writes the
//! per-rule table (and every violation) for the review request.

mod dom;
mod rules;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, header};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, InsecureStubHasher, OidcConfig, OidcProvider, PasswordHasher as _,
};
use planeter_core::{
    FetchError, FetchRequest, HostingService, HttpsFetcher, NullAuditSink, ReadService,
};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, UserId, Visibility,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

use rules::{RULES, Violation};

// ---------------------------------------------------------------------------------------------------------
// States

/// One rendered state. `identity` says which page it is: two states with different identities must have
/// different titles; two with the same identity are the same page (and must have the same title).
struct State {
    id: String,
    identity: String,
    status: u16,
    html: String,
}

/// What a request produced that is not a page (a redirect, a bare body): counted, never silently dropped.
#[derive(Default)]
struct Skipped(Vec<String>);

struct Out {
    states: Vec<State>,
    skipped: Skipped,
}

impl Out {
    fn new() -> Self {
        Out {
            states: Vec::new(),
            skipped: Skipped::default(),
        }
    }
    async fn take(&mut self, id: &str, identity: &str, resp: axum::response::Response) {
        let status = resp.status().as_u16();
        let is_html = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("text/html"));
        let body = axum::body::to_bytes(resp.into_body(), 16 << 20)
            .await
            .unwrap();
        let html = String::from_utf8_lossy(&body).into_owned();
        // A page is an HTML document; a bare body (an older fragment, an empty 404) is not one.
        if is_html && html.contains("<html") {
            self.states.push(State {
                id: id.to_owned(),
                identity: identity.to_owned(),
                status,
                html,
            });
        } else {
            self.skipped.0.push(format!(
                "{id}: status {status}, {} bytes, {}",
                body.len(),
                if is_html {
                    "html without a document"
                } else {
                    "not html"
                }
            ));
        }
    }
}

/// `STRUCTURE_LANG=qaa` renders every state in the test language (right to left, every message wrapped in
/// `‹ ›`), for looking at a page with `dir="rtl"` (with `STRUCTURE_DUMP`); the rules that read English text
/// (the title pattern) are expected to complain then, so it is never set in a normal run.
fn test_language() -> Option<&'static str> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let lang = std::env::var("STRUCTURE_LANG").ok()?;
    if lang != "qaa" {
        return None;
    }
    ONCE.call_once(|| {
        use planeter_web::catalog::{Dir, Entry, LangDef, Plural, register_test_language, shipped};
        fn other_only(_: u64) -> Plural {
            Plural::Other
        }
        let entries: Vec<Entry> = shipped()[0]
            .entries
            .iter()
            .map(|e| {
                let text: &'static str = Box::leak(format!("‹{}›", e.other).into_boxed_str());
                Entry {
                    key: e.key,
                    one: text,
                    other: text,
                }
            })
            .collect();
        register_test_language(Box::leak(Box::new(LangDef {
            tag: "qaa",
            endonym: "Test language",
            dir: Dir::Rtl,
            plural: other_only,
            group_separator: ',',
            decimal_separator: '.',
            entries: Box::leak(entries.into_boxed_slice()),
        })));
    });
    Some("qaa")
}

fn req(method: &str, uri: &str, cookie: Option<&str>, form: Option<String>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "forge.example");
    if let Some(lang) = test_language() {
        b = b.header(header::ACCEPT_LANGUAGE, lang);
    }
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    if form.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    b.body(Body::from(form.unwrap_or_default())).unwrap()
}

async fn call(
    r: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    form: Option<String>,
) -> axum::response::Response {
    r.clone()
        .oneshot(req(method, uri, cookie, form))
        .await
        .unwrap()
}

async fn visit(
    out: &mut Out,
    r: &Router,
    id: &str,
    identity: &str,
    uri: &str,
    cookie: Option<&str>,
) {
    let resp = call(r, "GET", uri, cookie, None).await;
    out.take(id, identity, resp).await;
}

fn cookie_value(resp: &axum::response::Response, name: &str) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| {
            c.strip_prefix(&format!("{name}="))
                .map(|v| v.split(';').next().unwrap_or("").to_owned())
        })
}

// ---------------------------------------------------------------------------------------------------------
// The application under test

#[derive(Default)]
struct MockFetcher {
    routes: Mutex<HashMap<String, Vec<u8>>>,
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

struct Opts {
    /// (name, visibility, path of the repository on disk)
    repos: Vec<(&'static str, Visibility, PathBuf)>,
    binary: Option<String>,
    gate: Arc<PrikkGate>,
    /// `Some(true)`: SSO on and discovery works; `Some(false)`: SSO on, the provider is down.
    sso: Option<bool>,
    tag: &'static str,
}

fn scratch(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("structure-{name}"))
}

fn build(o: Opts) -> Router {
    let root = scratch(&format!("hosting-{}", o.tag));
    let _ = std::fs::remove_dir_all(&root);
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (i, (name, visibility, path)) in o.repos.into_iter().enumerate() {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(format!("r{i}")),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility,
                created_at: 0,
                prikk_format_version: None,
                path,
            })
            .unwrap();
    }
    let mut hosting = HostingService::new(root, store as Arc<dyn RepositoryStore>)
        .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    if let Some(b) = o.binary {
        hosting = hosting.with_binary(b);
    }
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        Arc::new(hosting),
        membership,
        Arc::new(NullAuditSink),
    ));
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account {
            user: UserId::new("alice"),
            display_name: "Alice".into(),
            email: None,
            password: Some(InsecureStubHasher.hash("pw")),
        })
        .unwrap();
    accounts
        .link_oidc("https://idp.example", "user-42", UserId::new("alice"))
        .unwrap();
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Arc::new(Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    ));
    let oidc = o.sso.map(|discovery| {
        let f = Arc::new(MockFetcher::default());
        if discovery {
            f.routes.lock().unwrap().insert(
                "https://idp.example/.well-known/openid-configuration".into(),
                br#"{"issuer":"https://idp.example","authorization_endpoint":"https://idp.example/auth","token_endpoint":"https://idp.example/token","jwks_uri":"https://idp.example/jwks"}"#.to_vec(),
            );
        }
        Arc::new(OidcProvider::new(
            OidcConfig {
                issuer: "https://idp.example".into(),
                client_id: "c".into(),
                client_secret: None,
                redirect_uri: "https://forge.example/login/oidc/callback".into(),
            },
            f,
        ))
    });
    router(AppState {
        read,
        gate: o.gate,
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::default()),
        oidc,
    })
}

fn gate() -> Arc<PrikkGate> {
    Arc::new(PrikkGate::default().with_budget(2048 << 20))
}

/// A repository that exists in the store with a directory on disk but nothing prikk can read in it (an empty
/// store): admission can size it, and the program is then missing or busy.
fn nowhere() -> PathBuf {
    let d = scratch("empty-repository-dir");
    std::fs::create_dir_all(d.join(".prikk")).unwrap();
    d
}

const CSRF: &str = "tok123";

// ---------------------------------------------------------------------------------------------------------
// The stand-in subset: no repository is read

async fn build_standin_states() -> Out {
    let mut out = Out::new();
    let anon = format!("planeter_csrf={CSRF}");
    let public = || {
        vec![
            ("ghost", Visibility::Public, nowhere()),
            ("secret", Visibility::Private, nowhere()),
        ]
    };

    let r = build(Opts {
        repos: public(),
        binary: Some("/nonexistent/prikk".into()),
        gate: gate(),
        sso: None,
        tag: "standin",
    });
    visit(&mut out, &r, "home-anon", "home", "/", None).await;
    visit(&mut out, &r, "login-fresh-sso-off", "login", "/login", None).await;
    for (id, uri) in [
        ("go-bad-noslash", "/go?repo=alicefull"),
        ("go-bad-empty", "/go?repo=alice/"),
        ("go-bad-extra", "/go?repo=a/b/c"),
        ("go-missing-param", "/go"),
        ("go-spaces", "/go?repo=%20alice/full"),
    ] {
        visit(&mut out, &r, id, "go-error", uri, None).await;
    }
    visit(
        &mut out,
        &r,
        "go-good",
        "go-redirect",
        "/go?repo=alice/ghost",
        None,
    )
    .await;

    // Sign-in outcomes.
    let post = |uri: &str, cookie: Option<&str>, form: String| req("POST", uri, cookie, Some(form));
    let resp = r
        .clone()
        .oneshot(post(
            "/login",
            Some(&anon),
            "csrf=bogus&username=alice&password=pw".into(),
        ))
        .await
        .unwrap();
    out.take("login-csrf-failure", "notice-csrf", resp).await;
    let resp = r
        .clone()
        .oneshot(post("/login", Some(&anon), format!("csrf={CSRF}")))
        .await
        .unwrap();
    out.take("login-form-rejection", "notice-form", resp).await;
    let resp = r
        .clone()
        .oneshot(post("/logout", Some(&anon), String::new()))
        .await
        .unwrap();
    out.take("logout-form-rejection", "notice-form", resp).await;
    let resp = r
        .clone()
        .oneshot(post(
            "/login",
            Some(&anon),
            format!("csrf={CSRF}&username=alice&password=wrong"),
        ))
        .await
        .unwrap();
    out.take("login-failed", "login-failed", resp).await;
    let resp = r
        .clone()
        .oneshot(post(
            "/login",
            Some(&anon),
            format!("csrf={CSRF}&username=alice&password=pw"),
        ))
        .await
        .unwrap();
    let session = cookie_value(&resp, "planeter_session").expect("a session cookie");
    let signed = format!("planeter_csrf={CSRF}; planeter_session={session}");
    visit(&mut out, &r, "home-signed-in", "home", "/", Some(&signed)).await;
    visit(
        &mut out,
        &r,
        "login-page-signed-in",
        "login",
        "/login",
        Some(&signed),
    )
    .await;
    let resp = r
        .clone()
        .oneshot(post("/logout", Some(&anon), "csrf=bogus".into()))
        .await
        .unwrap();
    out.take("logout-csrf-failure", "notice-csrf", resp).await;
    for _ in 0..12 {
        let resp = r
            .clone()
            .oneshot(post(
                "/login",
                Some(&anon),
                format!("csrf={CSRF}&username=mallory&password=x"),
            ))
            .await
            .unwrap();
        if resp.status().as_u16() == 429 {
            out.take("login-locked", "login-locked", resp).await;
            break;
        }
    }

    // Pages that do not exist, or that this reader may not know exist.
    visit(
        &mut out,
        &r,
        "notfound-private-anon",
        "notfound:/alice/secret",
        "/alice/secret",
        None,
    )
    .await;
    visit(
        &mut out,
        &r,
        "notfound-path",
        "notfound-bare",
        "/definitely/not/a/page/at/all/x",
        None,
    )
    .await;
    let resp = r
        .clone()
        .oneshot(req("POST", "/", None, Some("x=1".into())))
        .await
        .unwrap();
    out.take("method-405-ui", "method-405", resp).await;
    visit(
        &mut out,
        &r,
        "oidc-begin-not-configured",
        "sso-off",
        "/login/oidc",
        None,
    )
    .await;
    visit(
        &mut out,
        &r,
        "oidc-callback-not-configured",
        "sso-off",
        "/login/oidc/callback?code=x&state=y",
        None,
    )
    .await;

    // A repository that exists but cannot be read (the program is not there).
    visit(
        &mut out,
        &r,
        "unavailable-repo",
        "unavailable:/alice/ghost",
        "/alice/ghost",
        None,
    )
    .await;
    visit(
        &mut out,
        &r,
        "unavailable-history",
        "unavailable:/alice/ghost/history",
        "/alice/ghost/history",
        None,
    )
    .await;
    visit(
        &mut out,
        &r,
        "unavailable-change",
        "unavailable:/alice/ghost/change/0000",
        "/alice/ghost/change/0000",
        None,
    )
    .await;

    // SSO.
    let rs = build(Opts {
        repos: public(),
        binary: Some("/nonexistent/prikk".into()),
        gate: gate(),
        sso: Some(true),
        tag: "sso",
    });
    visit(&mut out, &rs, "login-fresh-sso-on", "login", "/login", None).await;
    visit(
        &mut out,
        &rs,
        "oidc-callback-missing",
        "sso-incomplete",
        "/login/oidc/callback",
        None,
    )
    .await;
    visit(
        &mut out,
        &rs,
        "oidc-callback-state-mismatch",
        "sso-mismatch",
        "/login/oidc/callback?code=c&state=s",
        Some("planeter_oidc_state=other"),
    )
    .await;
    let rs2 = build(Opts {
        repos: public(),
        binary: Some("/nonexistent/prikk".into()),
        gate: gate(),
        sso: Some(false),
        tag: "sso2",
    });
    visit(
        &mut out,
        &rs2,
        "oidc-begin-provider-down",
        "sso-down",
        "/login/oidc",
        None,
    )
    .await;

    // Busy: the light pool is held and the wait is short.
    let busy_gate = Arc::new(PrikkGate::new(8, Duration::from_millis(300)).with_budget(64 << 20));
    let rb = build(Opts {
        repos: public(),
        binary: Some("/nonexistent/prikk".into()),
        gate: busy_gate.clone(),
        sso: None,
        tag: "busy",
    });
    let holders: Vec<_> = (0..2)
        .map(|_| {
            let g = busy_gate.clone();
            tokio::spawn(async move {
                g.run_weighted(16 << 20, "log", || {
                    std::thread::sleep(Duration::from_millis(1500))
                })
                .await
            })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    visit(
        &mut out,
        &rb,
        "busy-history",
        "busy:/alice/ghost/history",
        "/alice/ghost/history",
        None,
    )
    .await;
    for h in holders {
        let _ = h.await;
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// The real-prikk subset: the baseline's fixtures

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Fixtures {
    root: PathBuf,
    keys: PathBuf,
}

impl Fixtures {
    fn prikk(&self, repo: &Path, args: &[&str]) -> String {
        let out = Command::new("prikk")
            .args(args)
            .current_dir(repo)
            .env("HOME", &self.keys)
            .env("XDG_CONFIG_HOME", &self.keys)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "prikk {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
    fn dir(&self, name: &str) -> PathBuf {
        let d = self.root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

fn fixtures() -> Fixtures {
    let root = scratch("fixtures");
    let _ = std::fs::remove_dir_all(&root);
    let keys = root.join("keys");
    std::fs::create_dir_all(&keys).unwrap();
    let f = Fixtures { root, keys };

    // full: README, text, binary, non-UTF-8, a long name, a file above the 4 MiB bound, two commits, a branch,
    // a tag, a closed branch
    let full = f.dir("full");
    f.prikk(&full, &["setup"]);
    std::fs::write(
        full.join("README.md"),
        "# Full fixture\n\nA short introduction with a [link to the guide](docs/guide.md) and `inline code`.\n\n## Contents\n\n- one\n- two\n\n```\nfenced code\n```\n\n![a picture](https://example.invalid/pic.png)\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
    )
    .unwrap();
    std::fs::create_dir_all(full.join("src")).unwrap();
    std::fs::create_dir_all(full.join("docs")).unwrap();
    std::fs::write(
        full.join("src/main.rs"),
        b"fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    std::fs::write(full.join("docs/guide.md"), b"# Guide\n\nSome text.\n").unwrap();
    std::fs::write(full.join("data.bin"), vec![0xA5u8; 2048]).unwrap();
    std::fs::write(full.join("latin1.txt"), b"\xff\xfeinvalid utf8 \x80\x81\n").unwrap();
    std::fs::write(
        full.join("a-very-long-file-name-that-keeps-going-and-going-and-going-and-going-and-going-and-going-and-going-until-it-is-longer-than-a-narrow-screen.txt"),
        b"x\n",
    )
    .unwrap();
    std::fs::write(full.join("huge.bin"), vec![0x5Au8; 5 * 1024 * 1024]).unwrap();
    f.prikk(
        &full,
        &["commit", "--from-worktree", "-m", "Add the first files"],
    );
    std::fs::write(
        full.join("src/main.rs"),
        b"fn main() {\n    println!(\"hello\");\n}\nsecond line\n",
    )
    .unwrap();
    f.prikk(&full, &["commit", "--from-worktree", "-m", "Second change: a multi-word message with a very long unbroken token aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
    f.prikk(&full, &["seal", "--allow-no-audit"]);
    f.prikk(&full, &["branch", "create", "heads/dev"]);
    f.prikk(&full, &["branch", "create", "heads/old"]);
    f.prikk(&full, &["branch", "close", "heads/old"]);
    f.prikk(
        &full,
        &[
            "tag",
            "create",
            "tags/v1",
            "--target",
            "heads/main",
            "-m",
            "first",
        ],
    );

    let noreadme = f.dir("noreadme");
    f.prikk(&noreadme, &["setup"]);
    std::fs::write(noreadme.join("a.txt"), b"hello\n").unwrap();
    f.prikk(
        &noreadme,
        &["commit", "--from-worktree", "-m", "only a text file"],
    );
    f.prikk(&noreadme, &["seal", "--allow-no-audit"]);

    let empty = f.dir("empty");
    f.prikk(&empty, &["setup"]);

    let unsealed = f.dir("unsealed");
    f.prikk(&unsealed, &["setup"]);
    std::fs::write(unsealed.join("a.txt"), b"hello\n").unwrap();
    f.prikk(
        &unsealed,
        &["commit", "--from-worktree", "-m", "first sealed"],
    );
    f.prikk(&unsealed, &["seal", "--allow-no-audit"]);
    std::fs::write(unsealed.join("a.txt"), b"hello\nmore\n").unwrap();
    f.prikk(
        &unsealed,
        &["commit", "--from-worktree", "-m", "queued, not sealed"],
    );
    f
}

fn first_after(page: &str, prefix: &str) -> String {
    let at = page
        .find(prefix)
        .unwrap_or_else(|| panic!("no {prefix} in the page"))
        + prefix.len();
    page[at..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect()
}

async fn build_real_states() -> Option<Out> {
    if !have("prikk") {
        eprintln!(
            "structure: the real-prikk states are skipped: `prikk` is not on PATH (the stand-in states still run)"
        );
        return None;
    }
    let f = fixtures();
    let repos = || {
        vec![
            ("full", Visibility::Public, f.root.join("full")),
            ("noreadme", Visibility::Public, f.root.join("noreadme")),
            ("unsealed", Visibility::Public, f.root.join("unsealed")),
            ("empty", Visibility::Public, f.root.join("empty")),
            ("secret", Visibility::Private, f.root.join("noreadme")),
        ]
    };
    let mut out = Out::new();
    let r = build(Opts {
        repos: repos(),
        binary: None,
        gate: gate(),
        sso: None,
        tag: "real",
    });
    let anon = format!("planeter_csrf={CSRF}");
    let resp = call(
        &r,
        "POST",
        "/login",
        Some(&anon),
        Some(format!("csrf={CSRF}&username=alice&password=pw")),
    )
    .await;
    let session = cookie_value(&resp, "planeter_session").expect("a session cookie");
    let signed = format!("planeter_csrf={CSRF}; planeter_session={session}");

    let long = "a-very-long-file-name-that-keeps-going-and-going-and-going-and-going-and-going-and-going-and-going-until-it-is-longer-than-a-narrow-screen.txt";
    let pages: Vec<(&str, String, String)> = vec![
        ("repo-full", "repo:alice/full".into(), "/alice/full".into()),
        (
            "repo-full-ref-dev",
            "repo:alice/full".into(),
            "/alice/full?ref=heads/dev".into(),
        ),
        (
            "repo-full-ref-bad",
            "unavailable:/alice/full".into(),
            "/alice/full?ref=heads/nope".into(),
        ),
        (
            "repo-noreadme",
            "repo:alice/noreadme".into(),
            "/alice/noreadme".into(),
        ),
        (
            "repo-unsealed",
            "repo:alice/unsealed".into(),
            "/alice/unsealed".into(),
        ),
        (
            "repo-empty",
            "repo:alice/empty".into(),
            "/alice/empty".into(),
        ),
        (
            "tree-src",
            "tree:alice/full:src".into(),
            "/alice/full/tree/src".into(),
        ),
        (
            "tree-docs",
            "tree:alice/full:docs".into(),
            "/alice/full/tree/docs".into(),
        ),
        (
            "tree-missing",
            "tree:alice/full:nope".into(),
            "/alice/full/tree/nope".into(),
        ),
        (
            "file-text",
            "file:alice/full:src/main.rs".into(),
            "/alice/full/blob/src/main.rs".into(),
        ),
        (
            "file-markdown",
            "file:alice/full:docs/guide.md".into(),
            "/alice/full/blob/docs/guide.md".into(),
        ),
        (
            "file-readme-md",
            "file:alice/full:README.md".into(),
            "/alice/full/blob/README.md".into(),
        ),
        (
            "file-binary",
            "file:alice/full:data.bin".into(),
            "/alice/full/blob/data.bin".into(),
        ),
        (
            "file-nonutf8",
            "file:alice/full:latin1.txt".into(),
            "/alice/full/blob/latin1.txt".into(),
        ),
        (
            "file-longname",
            format!("file:alice/full:{long}"),
            format!("/alice/full/blob/{long}"),
        ),
        (
            "file-toolarge",
            "toolarge:/alice/full/blob/huge.bin".into(),
            "/alice/full/blob/huge.bin".into(),
        ),
        (
            "file-missing",
            "notfound:/alice/full/blob/nope.txt".into(),
            "/alice/full/blob/nope.txt".into(),
        ),
        (
            "history-full",
            "history:alice/full".into(),
            "/alice/full/history".into(),
        ),
        (
            "history-dev",
            "history:alice/full".into(),
            "/alice/full/history?ref=heads/dev".into(),
        ),
        (
            "history-noreadme",
            "history:alice/noreadme".into(),
            "/alice/noreadme/history".into(),
        ),
        (
            "history-empty",
            "history:alice/empty".into(),
            "/alice/empty/history".into(),
        ),
        (
            "refs-full",
            "refs:alice/full".into(),
            "/alice/full/refs".into(),
        ),
        (
            "refs-noreadme",
            "refs:alice/noreadme".into(),
            "/alice/noreadme/refs".into(),
        ),
        (
            "verify-full",
            "verify:alice/full".into(),
            "/alice/full/verify".into(),
        ),
        (
            "verify-unsealed",
            "verify:alice/unsealed".into(),
            "/alice/unsealed/verify".into(),
        ),
        (
            "notfound-repo",
            "notfound:/alice/nope".into(),
            "/alice/nope".into(),
        ),
        (
            "change-bad-id",
            "unavailable:/alice/full/change/0000".into(),
            "/alice/full/change/0000".into(),
        ),
    ];
    for (id, identity, uri) in &pages {
        visit(&mut out, &r, id, identity, uri, None).await;
    }
    visit(
        &mut out,
        &r,
        "repo-full-signed-in",
        "repo:alice/full",
        "/alice/full",
        Some(&signed),
    )
    .await;

    // Change pages: a sealed block and a patch in a sealed block, and a queued patch.
    let history = {
        let resp = call(&r, "GET", "/alice/full/history", None, None).await;
        let b = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        String::from_utf8_lossy(&b).into_owned()
    };
    // The history links carry the whole block id; the page shows a short one.
    let block = first_after(&history, "/alice/full/change/");
    visit(
        &mut out,
        &r,
        "change-sealed",
        &format!("change:alice/full:{block}"),
        &format!("/alice/full/change/{block}"),
        None,
    )
    .await;
    let status: serde_json::Value =
        serde_json::from_str(&f.prikk(&f.root.join("unsealed"), &["status", "--format", "json"]))
            .unwrap();
    let queued = status["queue"]["patches"][0]["patch_id"]
        .as_str()
        .unwrap()
        .to_owned();
    visit(
        &mut out,
        &r,
        "change-queued-patch",
        &format!("change:alice/unsealed:{queued}"),
        &format!("/alice/unsealed/change/{queued}"),
        None,
    )
    .await;

    // The refusals that need a repository to measure: too large for the read budget.
    let rt = build(Opts {
        repos: repos(),
        binary: None,
        gate: Arc::new(PrikkGate::default().with_budget(24 << 20)),
        sso: None,
        tag: "toolarge",
    });
    visit(
        &mut out,
        &rt,
        "toolarge-repo",
        "toolarge:/alice/full",
        "/alice/full",
        None,
    )
    .await;
    let rt2 = build(Opts {
        repos: repos(),
        binary: None,
        gate: Arc::new(PrikkGate::default().with_budget(32 << 20)),
        sso: None,
        tag: "toolarge2",
    });
    visit(
        &mut out,
        &rt2,
        "toolarge-verify-page",
        "toolarge:/alice/full/verify",
        "/alice/full/verify",
        None,
    )
    .await;
    visit(
        &mut out,
        &rt2,
        "repo-verify-unavailable",
        "repo:alice/full",
        "/alice/full",
        None,
    )
    .await;

    // `verify` answers "not ok": the real failing stdout with exit status 0, from a stand-in program.
    let script = scratch("prikk-verify-bad.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *verify*) cat '{}'; exit 0 ;;\n  *) exec prikk \"$@\" ;;\nesac\n",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/structure/canned-verify-bad.json").display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let rv = build(Opts {
        repos: repos(),
        binary: Some(script.display().to_string()),
        gate: gate(),
        sso: None,
        tag: "verifybad",
    });
    visit(
        &mut out,
        &rv,
        "verify-unverified",
        "verify:alice/full",
        "/alice/full/verify",
        None,
    )
    .await;
    visit(
        &mut out,
        &rv,
        "repo-unverified",
        "repo:alice/full",
        "/alice/full",
        None,
    )
    .await;
    visit(
        &mut out,
        &rv,
        "history-unverified",
        "history:alice/full",
        "/alice/full/history",
        None,
    )
    .await;
    Some(out)
}

/// Each subset is rendered once per test process and shared by the tests (they write into the same scratch
/// directories, so they must not each build their own).
async fn standin_states() -> &'static Out {
    static CELL: tokio::sync::OnceCell<Out> = tokio::sync::OnceCell::const_new();
    CELL.get_or_init(build_standin_states).await
}

async fn real_states() -> Option<&'static Out> {
    static CELL: tokio::sync::OnceCell<Option<Out>> = tokio::sync::OnceCell::const_new();
    CELL.get_or_init(build_real_states).await.as_ref()
}

// ---------------------------------------------------------------------------------------------------------
// Running the rules

struct Report {
    /// rule → (state id → details)
    by_rule: BTreeMap<&'static str, BTreeMap<String, Vec<String>>>,
    checked: usize,
    /// (state id, status, identity, title)
    listing: Vec<(String, u16, String, String)>,
}

/// Titles: a page has one, and two states that are different pages do not share it.
fn title_violations(states: &[&State], titles: &[Option<String>]) -> Vec<(String, Violation)> {
    let mut out = Vec::new();
    let mut by_title: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let mut by_identity: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (s, t) in states.iter().zip(titles) {
        if let Some(t) = t {
            by_title.entry(t.clone()).or_default().insert(&s.identity);
            by_identity
                .entry(&s.identity)
                .or_default()
                .insert(t.clone());
        }
    }
    for (title, identities) in &by_title {
        if identities.len() > 1 {
            let ids: Vec<&str> = states
                .iter()
                .filter(|s| identities.contains(s.identity.as_str()))
                .map(|s| s.id.as_str())
                .collect();
            out.push((
                ids.join(", "),
                rules::v_title(format!(
                    "{title:?} is the title of different pages ({})",
                    identities.iter().cloned().collect::<Vec<_>>().join(" | ")
                )),
            ));
        }
    }
    for (identity, titles) in &by_identity {
        if titles.len() > 1 {
            out.push((
                identity.to_string(),
                rules::v_title(format!("the same page has different titles: {titles:?}")),
            ));
        }
    }
    out
}

fn run(states: &[&State]) -> Report {
    let mut by_rule: BTreeMap<&'static str, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut titles = Vec::new();
    let mut listing = Vec::new();
    for s in states {
        let dom = dom::parse(&s.html);
        let title = rules::page_title(&dom);
        listing.push((
            s.id.clone(),
            s.status,
            s.identity.clone(),
            title.clone().unwrap_or_default(),
        ));
        titles.push(title);
        for v in rules::check(&dom) {
            by_rule
                .entry(v.rule)
                .or_default()
                .entry(s.id.clone())
                .or_default()
                .push(v.detail);
        }
    }
    for (ids, v) in title_violations(states, &titles) {
        by_rule
            .entry(v.rule)
            .or_default()
            .entry(ids)
            .or_default()
            .push(v.detail);
    }
    Report {
        by_rule,
        checked: states.len(),
        listing,
    }
}

fn print(label: &str, rep: &Report, skipped: &Skipped) -> String {
    let mut s = format!(
        "== {label}: {} states checked, {} not pages\n",
        rep.checked,
        skipped.0.len()
    );
    for rule in RULES {
        let found = rep.by_rule.get(rule);
        let states = found.map_or(0, |m| m.len());
        let items: usize = found.map_or(0, |m| m.values().map(Vec::len).sum());
        s.push_str(&format!(
            "{rule:<28} {states:>3} states  {items:>4} findings\n"
        ));
    }
    s.push_str("-- not pages (redirects, bare bodies):\n");
    for x in &skipped.0 {
        s.push_str(&format!("   {x}\n"));
    }
    s.push_str("-- states (id, status, title):\n");
    for (id, status, _, title) in &rep.listing {
        s.push_str(&format!("   {id:<32} {status}  {title}\n"));
    }
    s.push_str("-- findings:\n");
    for (rule, m) in &rep.by_rule {
        for (id, details) in m {
            let mut seen = BTreeSet::new();
            for d in details {
                if seen.insert(d) {
                    let n = details.iter().filter(|x| *x == d).count();
                    s.push_str(&format!(
                        "   [{rule}] {id}: {d}{}\n",
                        if n > 1 {
                            format!(" (x{n})")
                        } else {
                            String::new()
                        }
                    ));
                }
            }
        }
    }
    s
}

fn finish(label: &str, out: &Out) {
    let all: Vec<&State> = out.states.iter().collect();
    let rep = run(&all);
    let text = print(label, &rep, &out.skipped);
    eprintln!("{text}");
    // `STRUCTURE_DUMP=<dir>` saves every state's page as `<dir>/<id>.html` (the browser measurements of the
    // presentation checks read them), with its status in `<id>.status`.
    if let Ok(dir) = std::env::var("STRUCTURE_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        for s in &out.states {
            std::fs::write(Path::new(&dir).join(format!("{}.html", s.id)), &s.html).unwrap();
            std::fs::write(
                Path::new(&dir).join(format!("{}.status", s.id)),
                s.status.to_string(),
            )
            .unwrap();
        }
    }
    if let Ok(path) = std::env::var("STRUCTURE_OUTLINES") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for s in &out.states {
            writeln!(
                f,
                "### {} ({})\n{}",
                s.id,
                s.status,
                rules::outline(&dom::parse(&s.html))
            )
            .unwrap();
        }
    }
    if let Ok(path) = std::env::var("STRUCTURE_REPORT") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{text}").unwrap();
    }
    assert!(
        rep.by_rule.is_empty(),
        "{label}: structural findings (the table is above)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_standin_state_passes_every_rule() {
    let out = standin_states().await;
    // The subset must stay a subset worth having: a state that stops being a page is a change to look at.
    finish("stand-in states", out);
    assert!(
        out.states.len() >= 25,
        "only {} stand-in states are pages",
        out.states.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_real_prikk_state_passes_every_rule() {
    let Some(out) = real_states().await else {
        return;
    };
    finish("real-prikk states", out);
    assert!(
        out.states.len() >= 36,
        "only {} real-prikk states are pages",
        out.states.len()
    );
}

/// The two subsets together: no two different pages share a title across them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn titles_are_distinct_across_both_subsets() {
    let Some(real) = real_states().await else {
        return;
    };
    let standin = standin_states().await;
    let all: Vec<&State> = standin.states.iter().chain(real.states.iter()).collect();
    let rep = run(&all);
    let found = rep.by_rule.get("title");
    assert!(
        found.is_none(),
        "title findings across both subsets: {found:#?}"
    );
}

// ---------------------------------------------------------------------------------------------------------
// Each rule can fail

/// A page that passes every rule; each planted failure below breaks exactly one thing in it.
const GOOD: &str = r##"<!doctype html><html lang="en" dir="ltr"><head><meta charset="utf-8"><title>Page · planeter</title></head><body>
<a class="skip" href="#main">Skip to content</a>
<header><a href="/">planeter</a><nav aria-label="Repository"><a href="/x">Code</a></nav></header>
<main id="main"><h1>Heading</h1><h2>Sub</h2>
<form method="get" action="/go"><label for="q">Address</label><input id="q" name="q"><button type="submit">Open</button></form>
<table><caption>Files</caption><thead><tr><th scope="col">Name</th></tr></thead><tbody><tr><th scope="row">a</th></tr></tbody></table>
<section class="readme"><h2>Readme title</h2><table><tr><td>an author's table</td></tr></table></section>
<p><a href="/a">Download main.rs</a> <img src="/i" alt="A picture"></p>
</main><footer>planeter 0.1</footer></body></html>"##;

fn rules_found(html: &str) -> BTreeSet<&'static str> {
    rules::check(&dom::parse(html))
        .into_iter()
        .map(|v| v.rule)
        .collect()
}

#[test]
fn the_good_document_passes_every_rule() {
    let found = rules::check(&dom::parse(GOOD));
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn every_rule_has_a_planted_failure_and_each_plant_trips_its_rule() {
    // (rule, what to replace, with what)
    let plants: &[(&str, &str, &str)] = &[
        ("lang-dir", r#" lang="en" dir="ltr""#, ""),
        ("lang-dir", r#" dir="ltr""#, ""),
        ("one-h1", "<h1>Heading</h1>", "<h1>Heading</h1><h1>Two</h1>"),
        ("one-h1", "<h1>Heading</h1><h2>Sub</h2>", "<h2>Sub</h2>"),
        (
            "no-h1-in-rendered-content",
            "<h2>Readme title</h2>",
            "<h1>Readme title</h1>",
        ),
        ("heading-order", "<h2>Sub</h2>", "<h3>Sub</h3>"),
        (
            "skip-link",
            r##"<a class="skip" href="#main">Skip to content</a>"##,
            "",
        ),
        ("skip-link", r##"href="#main""##, r##"href="#elsewhere""##),
        ("landmarks", "<footer>planeter 0.1</footer>", ""),
        (
            "landmarks",
            "<header><a href=\"/\">planeter</a>",
            "<div><a href=\"/\">planeter</a>",
        ),
        ("landmarks", r#"<main id="main">"#, r#"<div id="main">"#),
        ("nav-names", r#"<nav aria-label="Repository">"#, "<nav>"),
        (
            "nav-names",
            "</nav></header>",
            r#"</nav><nav aria-label="Repository"><a href="/y">Y</a></nav></header>"#,
        ),
        ("form-labels", r#"<label for="q">Address</label>"#, ""),
        (
            "form-labels",
            r#"<label for="q">Address</label><input id="q" name="q">"#,
            r#"<input name="q" placeholder="Address">"#,
        ),
        (
            "form-labels",
            r#"<label for="q">Address</label><input id="q" name="q">"#,
            r#"<input name="q" aria-label="Address">"#,
        ),
        ("button-text", ">Open</button>", "></button>"),
        ("tables", "<caption>Files</caption>", ""),
        ("tables", r#"<th scope="col">"#, "<th>"),
        (
            "tables",
            r#"<thead><tr><th scope="col">Name</th></tr></thead><tbody><tr><th scope="row">a</th></tr></tbody>"#,
            "<tr><td>a</td></tr>",
        ),
        ("images", r#" alt="A picture""#, ""),
        ("no-icon-glyphs", "<p>", "<p>📁 "),
        (
            "aria-hidden-focusable",
            "<p>",
            r#"<p><a aria-hidden="true" href="/z">z</a> "#,
        ),
        ("link-text", "Download main.rs", "click here"),
        ("link-text", "Download main.rs", "0123456789ab"),
        ("link-text", "Download main.rs", ""),
        ("title", "<title>Page · planeter</title>", ""),
        ("title", "<title>Page · planeter</title>", "<title></title>"),
        (
            "title",
            "<title>Page · planeter</title>",
            "<title>Page</title>",
        ),
        (
            "title",
            "<title>Page · planeter</title>",
            "<title>change 39fd67ec44546355daa76edb6405d475724d2fc685eb97a5ce928f0c003a1b36 · planeter</title>",
        ),
        (
            "form-in-phrasing",
            "</main>",
            "<span><form></form></span></main>",
        ),
        (
            "no-inline-script-or-style",
            "</main>",
            "<script>x()</script></main>",
        ),
        (
            "no-inline-script-or-style",
            "<p>",
            r#"<p style="color:red">"#,
        ),
        ("no-inline-script-or-style", "<p>", r#"<p onclick="x()">"#),
    ];
    for (rule, from, to) in plants {
        assert!(
            GOOD.contains(from),
            "the plant for {rule} does not apply: {from:?}"
        );
        let bad = GOOD.replacen(from, to, 1);
        let found = rules_found(&bad);
        assert!(
            found.contains(rule),
            "planted failure for {rule} ({from:?} -> {to:?}) was not found; found {found:?}"
        );
    }
    let planted: BTreeSet<&str> = plants.iter().map(|p| p.0).collect();
    for rule in RULES {
        if planted.contains(rule) {
            continue;
        }
        panic!("rule {rule} has no planted failure");
    }
}

#[test]
fn the_vague_link_list_matches_whole_link_text_only() {
    // "Download main.rs" is not vague; "Download" alone is.
    let bad = GOOD.replacen("Download main.rs", "Download", 1);
    assert!(rules_found(&bad).contains("link-text"));
    assert!(rules_found(GOOD).is_empty());
}

#[test]
fn titles_shared_by_different_pages_are_found_and_the_same_page_may_repeat() {
    let mk = |id: &str, identity: &str| State {
        id: id.into(),
        identity: identity.into(),
        status: 200,
        html: String::new(),
    };
    let owned = [mk("a", "page-a"), mk("b", "page-b"), mk("a2", "page-a")];
    let states: Vec<&State> = owned.iter().collect();
    let same = vec![
        Some("T".to_owned()),
        Some("T".to_owned()),
        Some("T".to_owned()),
    ];
    assert_eq!(
        title_violations(&states, &same).len(),
        1,
        "two different pages with one title"
    );
    let ok = vec![
        Some("A".to_owned()),
        Some("B".to_owned()),
        Some("A".to_owned()),
    ];
    assert!(
        title_violations(&states, &ok).is_empty(),
        "the same page, the same title"
    );
    let drift = vec![
        Some("A".to_owned()),
        Some("B".to_owned()),
        Some("A2".to_owned()),
    ];
    assert_eq!(
        title_violations(&states, &drift).len(),
        1,
        "one page, two titles"
    );
}
