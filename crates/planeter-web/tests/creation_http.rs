//! RFC 004 part 7 over HTTP, through the real router with a real prikk (the sandbox is off; the sequence's kill-safety is held
//! by `planeter-transport`'s tests and by the binary's SIGKILL test): creating an empty repository, staging an archive, confirming
//! it with the typed fingerprints — through the API and through the pages, in two languages — and the refusals around them.
//!
//! The tests that need prikk print why and skip without one. `CREATION_TRANSCRIPT=<file>` appends every JSON exchange to a file
//! (the transcript in the review request); `CREATION_HTML_DUMP=<dir>` writes the pages.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;
#[allow(dead_code)]
#[path = "structure/dom.rs"]
mod dom;
#[allow(dead_code)]
#[path = "structure/rules.rs"]
mod rules;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher,
    SessionStore as _, TokenHasher, TokenRecord,
};
use planeter_core::{
    HostingService, LeaseManager, NullAuditSink, ReadService, RepoScope, Scope, ScopeAccess,
    new_boot_id,
};
use planeter_store::{
    InMemoryLeaseRecords, InMemoryMembershipStore, InMemoryRepositoryStore, InMemoryStageStore,
    MembershipStore, OrgId, OrgRole, RepositoryStore, StageStore, UserId,
};
use planeter_transport::{CreationConfig, CreationService};
use planeter_web::creation::{CreationRuntime, is_reserved, reserved_from, reserved_words};
use planeter_web::{AppState, ContentOrigin, RateLimiter, router};

use archives::{KeyInfo, prikk_available, simple_archive, tampered, two_key_archive};

macro_rules! need_prikk {
    () => {
        if !prikk_available() {
            eprintln!("skipping: prikk is not on PATH");
            return;
        }
    };
}

static NOW: AtomicU64 = AtomicU64::new(1_000_000);
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}
const BASE: u64 = 1_000_000;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct World {
    router: Router,
    base: PathBuf,
    sessions: Arc<planeter_auth::InMemorySessionStore>,
    repos: Arc<InMemoryRepositoryStore>,
    stages: Arc<InMemoryStageStore>,
}

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("creation-http-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// alice belongs to the organization `acme`; mallory, bob and carol belong to nothing. Tokens are named `tok-<user>`
/// (write scope) and `tok-<user>-ro` (read only).
fn world(name: &str, tweak: impl FnOnce(&mut CreationConfig, &mut u32)) -> World {
    NOW.store(BASE, Ordering::Relaxed);
    let base = dir(name);
    let repos = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(
        HostingService::new(
            base.join("repos"),
            repos.clone() as Arc<dyn RepositoryStore>,
        )
        .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    let members = Arc::new(InMemoryMembershipStore::new());
    members
        .set_org_role(UserId::new("alice"), OrgId::new("acme"), OrgRole::Member)
        .unwrap();
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        members.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user, access) in [
        ("tok-alice", "alice", ScopeAccess::Write),
        ("tok-alice-ro", "alice", ScopeAccess::ReadOnly),
        ("tok-mallory", "mallory", ScopeAccess::Write),
        ("tok-bob", "bob", ScopeAccess::Write),
    ] {
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&*hasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let auth = Arc::new(Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        creds,
        hasher.clone(),
        hasher,
    ));
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        now,
        Duration::from_secs(1),
    ));
    let stages = Arc::new(InMemoryStageStore::new());
    let mut cfg = CreationConfig {
        deadline: Duration::from_secs(120),
        ..CreationConfig::default()
    };
    let mut per_hour = 10u32;
    tweak(&mut cfg, &mut per_hour);
    let service = Arc::new(
        CreationService::new(
            hosting,
            repos.clone() as Arc<dyn RepositoryStore>,
            members as Arc<dyn MembershipStore>,
            stages.clone() as Arc<dyn StageStore>,
            leases,
            Some(base.join("stages")),
            cfg,
        )
        .unwrap()
        .with_clock(now),
    );
    let sessions = Arc::new(planeter_auth::InMemorySessionStore::new());
    let router = router(AppState {
        read,
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: sessions.clone(),
        now_unix: now,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport: None,
        creation: Some(Arc::new(CreationRuntime {
            service,
            rate: RateLimiter::new(per_hour, Duration::from_secs(3600)),
        })),
    });
    World {
        router,
        base,
        sessions,
        repos,
        stages,
    }
}

impl World {
    fn session(&self, user: &str) -> String {
        let s = self
            .sessions
            .create(UserId::new(user), now(), now() + 3600)
            .unwrap();
        format!("planeter_csrf=tok123; planeter_session={}", s.id.0)
    }

    fn stage_dirs(&self) -> usize {
        std::fs::read_dir(self.base.join("stages"))
            .map(|d| d.flatten().count())
            .unwrap_or(0)
    }

    fn repo_dirs(&self) -> usize {
        std::fs::read_dir(self.base.join("repos"))
            .map(|d| {
                d.flatten()
                    .map(|s| {
                        std::fs::read_dir(s.path())
                            .map(|c| c.flatten().count())
                            .unwrap_or(0)
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.text()))
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or("").to_owned()
    }
    fn h(&self, n: &str) -> &str {
        self.headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let asked = std::env::var("CREATION_TRANSCRIPT").is_ok().then(|| {
        let mut line = format!("{} {}", req.method(), req.uri());
        if let Some(a) = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
        {
            line += &format!("\n    authorization: {a}");
        }
        line
    });
    let resp = r.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = axum::body::to_bytes(resp.into_body(), 1 << 26)
        .await
        .unwrap()
        .to_vec();
    if let (Some(asked), Ok(path)) = (asked, std::env::var("CREATION_TRANSCRIPT")) {
        use std::io::Write as _;
        let is_json = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("application/json"));
        if is_json {
            let shown: String = String::from_utf8_lossy(&body).chars().take(1400).collect();
            let retry = headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(|v| format!("\n    retry-after: {v}"))
                .unwrap_or_default();
            let test = std::thread::current().name().unwrap_or("?").to_owned();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            let _ = writeln!(f, "[{test}]\n> {asked}\n< {status}{retry}\n    {shown}\n");
        }
    }
    Reply {
        status,
        headers,
        body,
    }
}

async fn api_post(w: &World, uri: &str, tok: Option<&str>, body: &str) -> Reply {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = tok {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    send(&w.router, req.body(Body::from(body.to_owned())).unwrap()).await
}

async fn api_get(w: &World, uri: &str, tok: Option<&str>) -> Reply {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(t) = tok {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    send(&w.router, req.body(Body::empty()).unwrap()).await
}

fn framed(meta: &str, archive: &[u8]) -> Vec<u8> {
    let mut v = b"PLANETER-IMPORT-1\n".to_vec();
    v.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    v.extend_from_slice(meta.as_bytes());
    v.extend_from_slice(archive);
    v
}

async fn api_import(w: &World, tok: Option<&str>, body: Vec<u8>) -> Reply {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/import")
        .header("content-type", "application/octet-stream")
        .header("content-length", body.len().to_string());
    if let Some(t) = tok {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    send(&w.router, req.body(Body::from(body)).unwrap()).await
}

fn meta(owner: &str, name: &str) -> String {
    format!(r#"{{"owner":"{owner}","name":"{name}","visibility":"private"}}"#)
}

async fn stage_archive(w: &World, tok: &str, owner: &str, name: &str, archive: &Path) -> Reply {
    api_import(
        w,
        Some(tok),
        framed(&meta(owner, name), &std::fs::read(archive).unwrap()),
    )
    .await
}

fn confirm_body(keys: &[KeyInfo]) -> String {
    let adopt: Vec<String> = keys
        .iter()
        .map(|k| {
            format!(
                r#"{{"key_id":"{}","fingerprint":"{}"}}"#,
                k.key_id, k.public_key
            )
        })
        .collect();
    format!(r#"{{"adopt":[{}]}}"#, adopt.join(","))
}

async fn confirm_api(w: &World, tok: &str, stage: &str, body: &str) -> Reply {
    api_post(
        w,
        &format!("/api/v1/repos/import/{stage}/confirm"),
        Some(tok),
        body,
    )
    .await
}

// ------------------------------------------------------------------------------------------------ empty, over the API

#[tokio::test]
async fn an_empty_repository_is_created_in_the_callers_own_namespace_and_is_then_readable() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("empty", |_, _| {});
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"alice","name":"app","visibility":"public"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let v = r.json();
    assert_eq!(v["url"], "/alice/app");
    assert_eq!(v["proposals"], "members", "the default");
    assert_eq!(v["visibility"], "public");
    // Now it is a repository like any other: the read API and the page answer for it (an anonymous reader, as it is public).
    let refs = api_get(&w, "/api/v1/repos/alice/app/refs", None).await;
    assert_eq!(refs.status, StatusCode::OK, "{}", refs.text());
    let page = send(
        &w.router,
        Request::builder()
            .uri("/alice/app")
            .header("host", "forge.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
}

#[tokio::test]
async fn an_organization_member_creates_in_the_organization_and_open_proposals_are_recorded() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("org", |_, _| {});
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"acme","name":"core","visibility":"internal","proposals":"any-authenticated"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(r.json()["proposals"], "any-authenticated");
    assert_eq!(r.json()["owner"], "acme");
}

#[tokio::test]
async fn one_refusal_for_a_stranger_whatever_they_name_and_a_taken_name_only_to_the_allowed() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("neutral", |_, _| {});
    api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"alice","name":"taken","visibility":"private"}"#,
    )
    .await;
    // mallory names an organization that exists, one that does not, alice's namespace, and a name that is taken there.
    let mut bodies = Vec::new();
    for owner in ["acme", "no-such-org", "alice"] {
        let r = api_post(
            &w,
            "/api/v1/repos",
            Some("tok-mallory"),
            &format!(r#"{{"owner":"{owner}","name":"taken","visibility":"private"}}"#),
        )
        .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{owner}: {}", r.text());
        assert_eq!(r.code(), "forbidden");
        bodies.push(r.text());
    }
    assert!(
        bodies.windows(2).all(|p| p[0] == p[1]),
        "one refusal, byte for byte: {bodies:?}"
    );
    // The name is taken: said to alice, who may create there.
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"alice","name":"taken","visibility":"private"}"#,
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::CONFLICT, "name_taken")
    );
    // An anonymous caller and a session cookie alone (bearer only) are refused too, differently worded: they have no account.
    let r = api_post(
        &w,
        "/api/v1/repos",
        None,
        r#"{"owner":"alice","name":"x","visibility":"private"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let cookie = w.session("alice");
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos")
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(
            r#"{"owner":"alice","name":"viacookie","visibility":"private"}"#,
        ))
        .unwrap();
    assert_eq!(
        send(&w.router, req).await.status,
        StatusCode::FORBIDDEN,
        "a session cookie does not create over the API"
    );
    assert!(
        w.repos
            .resolve(&planeter_store::Owner::User("alice".into()), "viacookie")
            .unwrap()
            .is_none()
    );
    // A read-only token creates nothing.
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice-ro"),
        r#"{"owner":"alice","name":"ro","visibility":"private"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_quota_and_the_rate_limit_are_named_refusals() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("quota", |c, rate| {
        c.user_quota = 2;
        *rate = 5;
    });
    for n in ["one", "two"] {
        let r = api_post(
            &w,
            "/api/v1/repos",
            Some("tok-alice"),
            &format!(r#"{{"owner":"alice","name":"{n}","visibility":"private"}}"#),
        )
        .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    }
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"alice","name":"three","visibility":"private"}"#,
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "repository_quota"),
        "{}",
        r.text()
    );
    assert_eq!(r.json()["limit"], 2);
    assert_eq!(r.json()["used"], 2);
    assert!(
        r.json()["message"].as_str().unwrap().contains("alice"),
        "the sentence names the owner"
    );
    assert_eq!(w.repo_dirs(), 2, "nothing was written for the refused one");
    // The organization has its own quota.
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"acme","name":"x","visibility":"private"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    // 5 an hour: four have counted (every attempt that passes the syntax check does); one more, then the sixth is limited, with
    // Retry-After.
    assert_eq!(
        api_post(
            &w,
            "/api/v1/repos",
            Some("tok-alice"),
            r#"{"owner":"alice","name":"nope","visibility":"private"}"#
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"alice","name":"nope","visibility":"private"}"#,
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    );
    assert!(r.h("retry-after").parse::<u64>().unwrap() >= 1);
    // The hour passes.
    NOW.store(BASE + 3601, Ordering::Relaxed);
    let r = api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"acme","name":"later","visibility":"private"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
}

#[tokio::test]
async fn hostile_and_sloppy_values_are_refused_with_the_field_and_the_reason() {
    let _g = SERIAL.lock().await;
    let w = world("hostile", |_, rate| *rate = 10_000);
    let long = "a".repeat(65);
    let cases: Vec<(String, &str, &str)> = vec![
        (
            r#"{"owner":"alice","name":"../x","visibility":"private"}"#.into(),
            "name",
            "bad_character",
        ),
        (
            r#"{"owner":"alice","name":"/etc/passwd","visibility":"private"}"#.into(),
            "name",
            "bad_character",
        ),
        (
            r#"{"owner":"alice","name":"a\u0000b","visibility":"private"}"#.into(),
            "name",
            "bad_character",
        ),
        (
            format!(r#"{{"owner":"alice","name":"{long}","visibility":"private"}}"#),
            "name",
            "too_long",
        ),
        (
            r#"{"owner":"alice","name":"App","visibility":"private"}"#.into(),
            "name",
            "uppercase",
        ),
        (
            r#"{"owner":"alice","name":"..","visibility":"private"}"#.into(),
            "name",
            "bad_start",
        ),
        (
            r#"{"owner":"alice","name":"x.git","visibility":"private"}"#.into(),
            "name",
            "dotted",
        ),
        (
            r#"{"owner":"alice","name":"","visibility":"private"}"#.into(),
            "name",
            "empty",
        ),
        (
            r#"{"owner":"alice","name":"日本","visibility":"private"}"#.into(),
            "name",
            "bad_character",
        ),
        (
            r#"{"owner":"../alice","name":"x","visibility":"private"}"#.into(),
            "owner",
            "bad_character",
        ),
        (
            r#"{"owner":"/abs","name":"x","visibility":"private"}"#.into(),
            "owner",
            "bad_character",
        ),
        (
            r#"{"owner":"a\u0000","name":"x","visibility":"private"}"#.into(),
            "owner",
            "bad_character",
        ),
        (
            r#"{"owner":"alice","name":"x","visibility":"secret"}"#.into(),
            "visibility",
            "unknown",
        ),
        (
            r#"{"owner":"alice","name":"x","visibility":"private","proposals":"anyone"}"#.into(),
            "proposals",
            "unknown",
        ),
        (
            r#"{"owner":"alice","name":"new","visibility":"private"}"#.into(),
            "name",
            "reserved",
        ),
        (
            r#"{"owner":"alice","name":"API","visibility":"private"}"#.into(),
            "name",
            "uppercase",
        ),
        (
            r#"{"owner":"alice","name":"import","visibility":"private"}"#.into(),
            "name",
            "reserved",
        ),
    ];
    for (body, field, reason) in cases {
        let r = api_post(&w, "/api/v1/repos", Some("tok-alice"), &body).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}: {}", r.text());
        assert_eq!(r.code(), "invalid_request", "{body}");
        assert_eq!(r.json()["field"], field, "{body}");
        assert_eq!(r.json()["reason"], reason, "{body}");
    }
    // Not JSON, an unknown field, a missing field, a body that is too long.
    for body in [
        "not json",
        r#"{"owner":"alice","name":"x","visibility":"private","extra":1}"#,
        r#"{"owner":"alice"}"#,
    ] {
        let r = api_post(&w, "/api/v1/repos", Some("tok-alice"), body).await;
        assert_eq!(
            (r.status, r.json()["field"].as_str().unwrap_or("")),
            (StatusCode::BAD_REQUEST, "body"),
            "{body}"
        );
    }
    let huge = format!(
        r#"{{"owner":"alice","name":"x","visibility":"private","pad":"{}"}}"#,
        "p".repeat(20_000)
    );
    assert_eq!(
        api_post(&w, "/api/v1/repos", Some("tok-alice"), &huge)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(w.repo_dirs(), 0, "nothing was written for any of them");
}

// ------------------------------------------------------------------------------------------------ reserved names

#[test]
fn reserved_names_come_from_the_routes_and_a_new_route_reserves_its_word() {
    // Derived: the first segment of every static route, and a static word that stands where another route has a parameter.
    let words = reserved_words();
    for w in [
        "new", "api", "login", "logout", "static", "lang", "go", "import",
    ] {
        assert!(words.contains(w), "{w} must be reserved: {words:?}");
    }
    assert!(
        !words.contains("history"),
        "a word that is only a suffix of a repository's routes is not reserved"
    );
    // The test that fails when a route is added that a name could shadow: every static first segment, and every static segment
    // beside a parameter, of every route of both tables, is reserved.
    let api = planeter_web::api::routes();
    let ui = planeter_web::ui::routes();
    let patterns: Vec<&str> = api
        .table()
        .iter()
        .chain(ui.table().iter())
        .map(|e| e.path)
        .collect();
    for p in &patterns {
        let Some(first) = p.split('/').find(|s| !s.is_empty()) else {
            continue;
        };
        if !first.starts_with('{') {
            assert!(
                is_reserved(first),
                "the first segment of {p} must be reserved"
            );
        }
    }
    // A planted route: the derivation reserves its word, which is how a future route cannot be shadowed.
    let mut planted = patterns.clone();
    planted.push("/zzz-planted/{x}");
    planted.push("/api/v1/repos/zzz-special");
    let derived = reserved_from(&planted);
    assert!(
        derived.contains("zzz-planted") && derived.contains("zzz-special"),
        "{derived:?}"
    );
    assert!(!reserved_from(&patterns).contains("zzz-planted"));
}

// ------------------------------------------------------------------------------------------------ archives over the API

#[tokio::test]
async fn an_archive_is_staged_shows_its_keys_and_trusts_nothing_until_the_fingerprints_are_typed() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-archive", |_, _| {});
    let (archive, keys) = simple_archive(&w.base, "maint", 3, 0);
    let r = stage_archive(&w, "tok-alice", "alice", "arch", &archive).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let v = r.json();
    let stage = v["stage_id"].as_str().unwrap().to_owned();
    assert_eq!(v["keys"][0]["key_id"], keys[0].key_id);
    assert_eq!(
        v["keys"][0]["fingerprint"], keys[0].public_key,
        "the fingerprint is the whole public key"
    );
    assert_eq!(v["keys"][0]["public_key"], keys[0].public_key);
    assert!(
        v["keys"][0]["fingerprint_grouped"]
            .as_str()
            .unwrap()
            .contains(' ')
    );
    assert_eq!(v["refs"][0]["ref_name"], "heads/main");
    assert_eq!(v["refs"][0]["publishable"], true);
    assert_eq!(v["internally_consistent"], true);
    assert_eq!(v["verifies_with_all_keys"], true);
    assert!(
        v["archive_bytes"].as_u64().unwrap() > 1000
            && v["archive_sha256"].as_str().unwrap().len() == 64
    );
    assert_eq!(
        v["expires_at"].as_u64().unwrap() - v["created_at"].as_u64().unwrap(),
        86_400
    );
    // No repository yet: not readable, not listed, nothing under the repository root.
    assert_eq!(
        api_get(&w, "/api/v1/repos/alice/arch/refs", Some("tok-alice"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(w.repo_dirs(), 0);
    // The stage can be read again by its stager and by nobody else.
    assert_eq!(
        api_get(
            &w,
            &format!("/api/v1/repos/import/{stage}"),
            Some("tok-alice")
        )
        .await
        .status,
        StatusCode::OK
    );
    for tok in [Some("tok-mallory"), None] {
        let r = api_get(&w, &format!("/api/v1/repos/import/{stage}"), tok).await;
        assert!(
            matches!(r.status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
            "{tok:?}: {}",
            r.status
        );
    }
    let unknown = api_get(
        &w,
        "/api/v1/repos/import/s-000000000000000000000000",
        Some("tok-mallory"),
    )
    .await;
    let foreign = api_get(
        &w,
        &format!("/api/v1/repos/import/{stage}"),
        Some("tok-mallory"),
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.text()),
        (foreign.status, foreign.text()),
        "a stranger cannot tell a stage from none"
    );

    // A wrong fingerprint: refused naming the key, never saying what was expected; the stage waits.
    let wrong = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
        keys[0].key_id,
        "0".repeat(64)
    );
    let r = confirm_api(&w, "tok-alice", &stage, &wrong).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "adoption_refused"),
        "{}",
        r.text()
    );
    assert_eq!(r.json()["key"], keys[0].key_id);
    assert_eq!(r.json()["reason"], "mismatch");
    assert!(
        !r.text().contains(&keys[0].public_key),
        "the expected fingerprint is never in the answer"
    );
    assert_eq!(w.repo_dirs(), 0);
    // A prefix, the short id, a key nobody carries, nothing at all.
    for body in [
        format!(
            r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
            keys[0].key_id,
            &keys[0].public_key[..32]
        ),
        format!(
            r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
            keys[0].key_id, keys[0].key_id
        ),
        r#"{"adopt":[{"key_id":"ed25519-ffffffffffffffff","fingerprint":"00"}]}"#.to_owned(),
        r#"{}"#.to_owned(),
    ] {
        let r = confirm_api(&w, "tok-alice", &stage, &body).await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, "adoption_refused"),
            "{body}: {}",
            r.text()
        );
    }
    // Another principal cannot confirm it.
    let r = confirm_api(&w, "tok-mallory", &stage, &confirm_body(&keys)).await;
    assert!(
        matches!(r.status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{}",
        r.status
    );
    assert_eq!(w.repo_dirs(), 0);

    // The right one, pasted in groups and capitals.
    let pasted = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
        keys[0].key_id,
        planeter_core::creation::group_fingerprint(&keys[0].public_key).to_uppercase()
    );
    let r = confirm_api(&w, "tok-alice", &stage, &pasted).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let v = r.json();
    assert_eq!(v["verified"], true);
    assert_eq!(v["landings"][0]["landed_as"], "heads/main");
    assert_eq!(v["landings"][0]["published"], true);
    assert_eq!(v["url"], "/alice/arch");
    // It is a repository now, with `heads/main` published; the stage is gone and cannot be confirmed twice.
    let refs = api_get(&w, "/api/v1/repos/alice/arch/refs", Some("tok-alice")).await;
    assert_eq!(refs.status, StatusCode::OK, "{}", refs.text());
    assert!(refs.text().contains("heads/main"));
    let verify = api_get(&w, "/api/v1/repos/alice/arch/verify", Some("tok-alice")).await;
    assert!(
        verify.text().contains("\"verified\":true")
            || verify.text().contains("\"ok\":true")
            || verify.status == StatusCode::OK,
        "{}",
        verify.text()
    );
    let again = confirm_api(&w, "tok-alice", &stage, &pasted).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(w.stage_dirs(), 0);
    assert!(w.stages.all().unwrap().is_empty());
}

#[tokio::test]
async fn no_adopt_creates_a_repository_with_everything_under_remotes_and_says_it_is_not_verified() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-no-adopt", |_, _| {});
    let (archive, _) = simple_archive(&w.base, "maint", 2, 0);
    let stage = stage_archive(&w, "tok-alice", "alice", "arch", &archive)
        .await
        .json()["stage_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = confirm_api(&w, "tok-alice", &stage, r#"{"no_adopt":true}"#).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let v = r.json();
    assert_eq!(v["verified"], false);
    assert_eq!(v["landings"][0]["published"], false);
    assert_eq!(v["landings"][0]["landed_as"], "remotes/heads/main");
    assert!(v["adopted"].as_array().unwrap().is_empty());
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("Nothing is verified or trusted"),
        "{}",
        v["message"]
    );
    // The refs API lists published branches and tags: there is none.
    let refs = api_get(&w, "/api/v1/repos/alice/arch/refs", Some("tok-alice")).await;
    assert!(refs.text().contains("\"branches\":[]"), "{}", refs.text());
}

#[tokio::test]
async fn two_keys_one_typed_and_one_left_out() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-two-keys", |_, _| {});
    let (archive, keys) = two_key_archive(&w.base, "two");
    let stage = stage_archive(&w, "tok-alice", "alice", "arch", &archive)
        .await
        .json()["stage_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Only one of the two answered: refused before anything is made, naming the key that was not.
    let half = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}]}}"#,
        keys[0].key_id, keys[0].public_key
    );
    let r = confirm_api(&w, "tok-alice", &stage, &half).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "adoption_refused")
    );
    assert_eq!(r.json()["reason"], "unaddressed");
    assert_eq!(r.json()["keys"][0], keys[1].key_id);
    assert_eq!(w.repo_dirs(), 0);
    let both = format!(
        r#"{{"adopt":[{{"key_id":"{}","fingerprint":"{}"}}],"leave_out":["{}"]}}"#,
        keys[0].key_id, keys[0].public_key, keys[1].key_id
    );
    let r = confirm_api(&w, "tok-alice", &stage, &both).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(r.json()["verified"], false);
    assert_eq!(r.json()["adopted"].as_array().unwrap().len(), 1);
    assert_eq!(r.json()["left_out"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_tampered_archive_is_refused_at_the_stage_with_prikks_words_and_nothing_is_kept() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-tampered", |_, _| {});
    let (archive, _) = simple_archive(&w.base, "maint", 2, 0);
    let bad = tampered(&archive, &w.base.join("bad.archive"));
    let r = stage_archive(&w, "tok-alice", "alice", "arch", &bad).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "archive_refused"),
        "{}",
        r.text()
    );
    assert!(
        r.json()["detail"].as_str().unwrap().contains("archive"),
        "{}",
        r.text()
    );
    assert_eq!(w.stage_dirs(), 0, "no directory is kept");
    assert!(w.stages.all().unwrap().is_empty(), "no record is kept");
    assert_eq!(w.repo_dirs(), 0);
}

#[tokio::test]
async fn the_edge_checks_come_before_a_byte_of_the_archive_is_read() {
    let _g = SERIAL.lock().await;
    let w = world("api-edge", |c, rate| {
        c.max_archive_bytes = 4096;
        *rate = 10_000;
    });
    let junk = vec![7u8; 1000];
    // Not signed in.
    assert_eq!(
        api_import(&w, None, framed(&meta("alice", "x"), &junk))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    // Bad magic, too short, metadata too long, metadata not JSON, an unknown metadata field.
    let mut bad_magic = framed(&meta("alice", "x"), &junk);
    bad_magic[0] = b'X';
    let mut too_long_meta = b"PLANETER-IMPORT-1\n".to_vec();
    too_long_meta.extend_from_slice(&(1u64 << 40).to_be_bytes());
    too_long_meta.extend_from_slice(&junk);
    for (body, reason) in [
        (bad_magic, "magic"),
        (b"PLANETER".to_vec(), "too_short"),
        (too_long_meta, "metadata_too_long"),
        (framed("not json", &junk), "metadata"),
        (
            framed(
                r#"{"owner":"alice","name":"x","visibility":"private","extra":1}"#,
                &junk,
            ),
            "metadata",
        ),
    ] {
        let r = api_import(&w, Some("tok-alice"), body).await;
        assert_eq!(
            (r.status, r.json()["reason"].as_str().unwrap_or("")),
            (StatusCode::BAD_REQUEST, reason),
            "{}",
            r.text()
        );
    }
    // The wrong content type; a length the server does not accept; no length at all.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/import")
        .header("authorization", "Bearer tok-alice")
        .header("content-type", "text/plain")
        .header("content-length", "10")
        .body(Body::from("0123456789"))
        .unwrap();
    assert_eq!(send(&w.router, req).await.json()["reason"], "content_type");
    let big = framed(&meta("alice", "x"), &vec![1u8; 5000]);
    let r = api_import(&w, Some("tok-alice"), big).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/import")
        .header("authorization", "Bearer tok-alice")
        .body(Body::from(framed(&meta("alice", "x"), &junk)))
        .unwrap();
    let r = send(&w.router, req).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::LENGTH_REQUIRED, "length_required")
    );
    // Hostile names in the metadata: refused as the empty creation refuses them.
    for owner_name in [("alice", "../x"), ("../alice", "x"), ("alice", "App")] {
        let r = api_import(
            &w,
            Some("tok-alice"),
            framed(&meta(owner_name.0, owner_name.1), &junk),
        )
        .await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::BAD_REQUEST, "invalid_request"),
            "{owner_name:?}"
        );
    }
    assert_eq!(w.stage_dirs(), 0);

    // Who may create there and whether there is room are decided from the metadata alone: a stranger naming an organization is
    // refused **without the archive being read** — the body's second chunk is never asked for.
    let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = polled.clone();
    let first = framed(&meta("acme", "x"), &[]);
    let rest = vec![9u8; 2000];
    let total = first.len() + rest.len();
    let stream = futures_stream(first, rest, seen);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/import")
        .header("authorization", "Bearer tok-mallory")
        .header("content-type", "application/octet-stream")
        .header("content-length", total.to_string())
        .body(Body::from_stream(stream))
        .unwrap();
    let r = send(&w.router, req).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    assert!(
        !polled.load(Ordering::SeqCst),
        "the archive's bytes were not asked for"
    );
}

/// A body of two chunks whose second is flagged when it is asked for.
fn futures_stream(
    first: Vec<u8>,
    rest: Vec<u8>,
    polled: Arc<std::sync::atomic::AtomicBool>,
) -> impl futures_core::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    struct S {
        first: Option<Vec<u8>>,
        rest: Option<Vec<u8>>,
        polled: Arc<std::sync::atomic::AtomicBool>,
    }
    impl futures_core::Stream for S {
        type Item = Result<Vec<u8>, std::io::Error>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            if let Some(f) = self.first.take() {
                return std::task::Poll::Ready(Some(Ok(f)));
            }
            if let Some(r) = self.rest.take() {
                self.polled.store(true, Ordering::SeqCst);
                return std::task::Poll::Ready(Some(Ok(r)));
            }
            std::task::Poll::Ready(None)
        }
    }
    S {
        first: Some(first),
        rest: Some(rest),
        polled,
    }
}

#[tokio::test]
async fn stages_are_limited_per_principal_and_a_stage_expires_after_a_day() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-limits", |_, rate| *rate = 10_000);
    let (archive, keys) = simple_archive(&w.base, "maint", 1, 0);
    let mut ids = Vec::new();
    for n in ["a", "b"] {
        let r = stage_archive(&w, "tok-alice", "alice", n, &archive).await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
        ids.push(r.json()["stage_id"].as_str().unwrap().to_owned());
    }
    let r = stage_archive(&w, "tok-alice", "alice", "c", &archive).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "stage_limit"),
        "{}",
        r.text()
    );
    assert_eq!(r.json()["limit"], 2);
    // Another principal is unaffected (bob makes a stage in his own namespace).
    assert_eq!(
        stage_archive(&w, "tok-bob", "bob", "a", &archive)
            .await
            .status,
        StatusCode::CREATED
    );
    // After the day: the stage is `410`, then swept; a new one is possible.
    NOW.store(BASE + 86_400, Ordering::SeqCst);
    let r = api_get(
        &w,
        &format!("/api/v1/repos/import/{}", ids[0]),
        Some("tok-alice"),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::GONE, "stage_expired")
    );
    let r = confirm_api(&w, "tok-alice", &ids[0], &confirm_body(&keys)).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::GONE, "stage_expired")
    );
    let r = stage_archive(&w, "tok-alice", "alice", "d", &archive).await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "expired stages are swept before the limit is applied: {}",
        r.text()
    );
}

#[tokio::test]
async fn the_budget_for_staged_archives_is_a_named_refusal() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-budget", |c, rate| {
        c.stage_limits.total_bytes = 8000;
        *rate = 10_000;
    });
    let (archive, _) = simple_archive(&w.base, "maint", 1, 0);
    let size = std::fs::metadata(&archive).unwrap().len();
    assert!(
        size > 4000 && size < 8000,
        "the fixture is the size the test assumes: {size}"
    );
    assert_eq!(
        stage_archive(&w, "tok-alice", "alice", "a", &archive)
            .await
            .status,
        StatusCode::CREATED
    );
    let r = stage_archive(&w, "tok-bob", "bob", "a", &archive).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "stage_limit"),
        "{}",
        r.text()
    );
    assert_eq!(r.json()["limit_bytes"], 8000);
}

#[tokio::test]
async fn an_archive_creation_obeys_the_quota_and_the_name_at_the_edge() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("api-edge-room", |c, rate| {
        c.user_quota = 1;
        *rate = 10_000;
    });
    let (archive, _) = simple_archive(&w.base, "maint", 1, 0);
    api_post(
        &w,
        "/api/v1/repos",
        Some("tok-alice"),
        r#"{"owner":"alice","name":"first","visibility":"private"}"#,
    )
    .await;
    let r = stage_archive(&w, "tok-alice", "alice", "second", &archive).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "repository_quota"),
        "{}",
        r.text()
    );
    let r = stage_archive(&w, "tok-alice", "acme", "first", &archive).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let r = stage_archive(&w, "tok-alice", "acme", "first", &archive).await;
    assert!(matches!(
        r.status,
        StatusCode::CREATED | StatusCode::CONFLICT
    ));
    assert_eq!(
        w.stage_dirs(),
        2.min(w.stage_dirs()),
        "no more than the stages that were accepted"
    );
}

// ------------------------------------------------------------------------------------------------ the pages

async fn page(w: &World, method: &str, uri: &str, cookie: Option<&str>, lang: &str) -> Reply {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "forge.example")
        .header("accept-language", lang);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    send(&w.router, req.body(Body::empty()).unwrap()).await
}

async fn form(w: &World, uri: &str, cookie: Option<&str>, body: &str, lang: &str) -> Reply {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("host", "forge.example")
        .header("accept-language", lang)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    send(&w.router, req.body(Body::from(body.to_owned())).unwrap()).await
}

fn multipart(fields: &[(&str, &str)], file: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----creationtestboundary7MA4YWxkTrZu0gW";
    let mut body = Vec::new();
    for (k, v) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"project.archive\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn upload(
    w: &World,
    cookie: Option<&str>,
    fields: &[(&str, &str)],
    file: &[u8],
    lang: &str,
) -> Reply {
    let (ct, body) = multipart(fields, file);
    let mut req = Request::builder()
        .method("POST")
        .uri("/new/import")
        .header("host", "forge.example")
        .header("accept-language", lang)
        .header("content-type", ct)
        .header("content-length", body.len().to_string());
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    send(&w.router, req.body(Body::from(body)).unwrap()).await
}

fn violations(html: &str) -> Vec<String> {
    rules::check(&dom::parse(html))
        .into_iter()
        .map(|v| format!("{}: {}", v.rule, v.detail))
        .collect()
}

fn dump(name: &str, lang: &str, html: &str) {
    if let Ok(d) = std::env::var("CREATION_HTML_DUMP") {
        let d = PathBuf::from(d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{name}.{lang}.html")), html).unwrap();
    }
}

fn pasted(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[tokio::test]
async fn the_new_page_offers_two_plain_choices_in_both_languages_and_is_well_formed() {
    let _g = SERIAL.lock().await;
    planeter_web::catalog::enable_preview_languages("ja").unwrap();
    let w = world("page-new", |_, _| {});
    let cookie = w.session("alice");
    for lang in ["en", "ja"] {
        let p = page(&w, "GET", "/new", Some(&cookie), lang).await;
        assert_eq!(p.status, StatusCode::OK);
        let html = p.text();
        dump("new", lang, &html);
        assert_eq!(violations(&html), Vec::<String>::new(), "{lang}");
        for needle in [
            "action=\"/new\"",
            "action=\"/new/import\"",
            "enctype=\"multipart/form-data\"",
            "type=\"file\"",
            "name=\"csrf\"",
        ] {
            assert!(html.contains(needle), "{lang}: {needle}");
        }
        // The file field comes after the text fields (the server checks them before it reads the file).
        let file_at = html.find("type=\"file\"").unwrap();
        let import_form = html.find("action=\"/new/import\"").unwrap();
        assert!(
            html[import_form..file_at].contains("name=\"owner\"")
                && html[import_form..file_at].contains("name=\"name\"")
        );
        // Each button has its consequence beside it; the open choice is explained.
        assert!(html.matches("<button type=\"submit\">").count() >= 2);
        assert!(
            !html.contains("{") || !unfilled_placeholder(&html),
            "{lang}: a placeholder was left in the page"
        );
        // The owner is pre-filled with the person's own name and nothing else is.
        assert!(html.contains("value=\"alice\""));
    }
    // Not signed in: the sign-in sentence, no form.
    let p = page(&w, "GET", "/new", None, "en").await;
    assert_eq!(p.status, StatusCode::FORBIDDEN);
    assert!(
        p.text().contains("Sign in to create a repository.")
            && !p.text().contains("action=\"/new\"")
    );
}

fn unfilled_placeholder(html: &str) -> bool {
    // A catalog placeholder like `{repo}` left unfilled would show as `{word}`.
    html.split('{').skip(1).any(|rest| {
        rest.split('}')
            .next()
            .is_some_and(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
    })
}

#[tokio::test]
async fn the_empty_form_creates_checks_its_token_and_says_what_is_wrong_where_the_person_is_looking()
 {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("page-empty", |_, rate| *rate = 10_000);
    let alice = w.session("alice");
    let ok = "csrf=tok123&owner=alice&name=app&visibility=private&proposals=members";
    // A wrong token: nothing is done.
    let r = form(
        &w,
        "/new",
        Some(&alice),
        "csrf=bogus&owner=alice&name=app&visibility=private&proposals=members",
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(w.repo_dirs(), 0);
    // A name that breaks a rule: the form again, with what was typed and the sentence.
    let r = form(
        &w,
        "/new",
        Some(&alice),
        "csrf=tok123&owner=alice&name=My+App&visibility=private&proposals=members",
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let html = r.text();
    assert!(
        html.contains("value=\"My App\"") && html.contains("The repository was not created"),
        "{html}"
    );
    dump("new-refused", "en", &html);
    assert!(html.contains("Use lowercase letters only"), "{html}");
    assert_eq!(violations(&html), Vec::<String>::new());
    // The same, in Japanese.
    planeter_web::catalog::enable_preview_languages("ja").unwrap();
    let r = form(
        &w,
        "/new",
        Some(&alice),
        "csrf=tok123&owner=alice&name=My+App&visibility=private&proposals=members",
        "ja",
    )
    .await;
    assert!(
        r.text().contains("リポジトリは作成されませんでした") && r.text().contains("小文字"),
        "{}",
        r.text()
    );
    // Success: seen to the repository's page.
    let r = form(&w, "/new", Some(&alice), ok, "en").await;
    assert_eq!(
        (r.status, r.h("location")),
        (StatusCode::SEE_OTHER, "/alice/app")
    );
    // The name is taken; someone else's namespace is one sentence.
    let r = form(&w, "/new", Some(&alice), ok, "en").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(
        r.text()
            .contains("There is already a repository with that name here.")
    );
    let mallory = w.session("mallory");
    let r = form(
        &w,
        "/new",
        Some(&mallory),
        "csrf=tok123&owner=acme&name=x&visibility=private&proposals=members",
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.text().contains("You cannot create a repository there."));
    // The rate limit on the page: the sentence and `Retry-After`.
    let limited = world("page-rate", |_, rate| *rate = 1);
    let c2 = limited.session("alice");
    let ok2 = "csrf=tok123&owner=alice&name=one&visibility=private&proposals=members";
    assert_eq!(
        form(&limited, "/new", Some(&c2), ok2, "en").await.status,
        StatusCode::SEE_OTHER
    );
    let r = form(
        &limited,
        "/new",
        Some(&c2),
        &ok2.replace("one", "two"),
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.h("retry-after").parse::<u64>().unwrap() >= 1);
    assert!(r.text().contains("too many new repositories"));
    // Signed out: the sign-in sentence.
    let r = form(&w, "/new", Some("planeter_csrf=tok123"), ok, "en").await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.text().contains("Sign in to create a repository."));
}

#[tokio::test]
async fn the_archive_flow_on_the_pages_in_two_screens_with_empty_fingerprint_fields() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    planeter_web::catalog::enable_preview_languages("ja").unwrap();
    let w = world("page-archive", |_, rate| *rate = 10_000);
    let (archive, keys) = simple_archive(&w.base, "maint", 3, 0);
    let bytes = std::fs::read(&archive).unwrap();
    let alice = w.session("alice");
    let fields = [
        ("csrf", "tok123"),
        ("owner", "alice"),
        ("name", "arch"),
        ("visibility", "private"),
        ("proposals", "members"),
    ];

    // Screen 1: a wrong token and a missing file are refused; the archive is uploaded and seen to the second screen.
    let mut bad = fields;
    bad[0] = ("csrf", "bogus");
    assert_eq!(
        upload(&w, Some(&alice), &bad, &bytes, "en").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(w.stage_dirs(), 0, "a wrong token reads no archive");
    let r = upload(&w, Some(&alice), &fields, &bytes, "en").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    let location = r.h("location").to_owned();
    assert!(location.starts_with("/new/import/s-"), "{location}");

    // Screen 2, in both languages: the facts, the keys with their fingerprints in groups, **empty fields**, a consequence beside each button.
    for lang in ["en", "ja"] {
        let p = page(&w, "GET", &location, Some(&alice), lang).await;
        assert_eq!(p.status, StatusCode::OK, "{lang}");
        let html = p.text();
        dump("stage", lang, &html);
        assert_eq!(violations(&html), Vec::<String>::new(), "{lang}");
        let grouped = planeter_core::creation::group_fingerprint(&keys[0].public_key);
        assert!(
            html.contains(&grouped),
            "{lang}: the fingerprint is shown in groups for comparison"
        );
        assert!(html.contains(&keys[0].key_id));
        assert!(
            !html.contains(&format!("value=\"{}\"", keys[0].public_key))
                && !html.contains(&format!("value=\"{grouped}\"")),
            "{lang}: never prefilled"
        );
        assert!(
            html.contains("name=\"fp_0\" value=\"\""),
            "{lang}: the field is empty"
        );
        assert!(
            html.contains("name=\"skip_0\"")
                && html.contains("name=\"mode\" value=\"typed\"")
                && html.contains("name=\"mode\" value=\"none\"")
        );
        assert!(!unfilled_placeholder(&html), "{lang}");
    }
    // Another person cannot see it.
    let mallory = w.session("mallory");
    assert_eq!(
        page(&w, "GET", &location, Some(&mallory), "en")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        page(&w, "GET", &location, None, "en").await.status,
        StatusCode::FORBIDDEN,
        "an anonymous visitor is told to sign in"
    );

    // A wrong fingerprint: the screen again, the sentence names the key, and the field is empty again.
    let wrong = format!("csrf=tok123&mode=typed&fp_0={}", "1".repeat(64));
    let r = form(&w, &location, Some(&alice), &wrong, "en").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let html = r.text();
    assert!(
        html.contains("is not the one the archive carries") && html.contains(&keys[0].key_id),
        "{html}"
    );
    dump("stage-refused", "en", &html);
    assert!(
        html.contains("name=\"fp_0\" value=\"\""),
        "nothing typed is echoed back"
    );
    assert!(!html.contains(&"1".repeat(64)));
    assert_eq!(w.repo_dirs(), 0);
    // Nothing typed and nothing skipped: asked what to do with the key.
    let r = form(&w, &location, Some(&alice), "csrf=tok123&mode=typed", "en").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.text().contains("You have not said what to do with"));
    // Typed and skipped at once.
    let r = form(
        &w,
        &location,
        Some(&alice),
        &format!(
            "csrf=tok123&mode=typed&fp_0={}&skip_0=1",
            keys[0].public_key
        ),
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.text().contains("also chose not to trust it"));
    // A wrong token on the confirmation.
    let r = form(
        &w,
        &location,
        Some(&alice),
        &format!("csrf=bogus&mode=typed&fp_0={}", keys[0].public_key),
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // The right fingerprint, pasted in groups: the repository is created and the result says what happened.
    let right = format!(
        "csrf=tok123&mode=typed&fp_0={}",
        pasted(&planeter_core::creation::group_fingerprint(
            &keys[0].public_key
        ))
    );
    let r = form(&w, &location, Some(&alice), &right, "ja").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let html = r.text();
    dump("created", "ja", &html);
    assert_eq!(violations(&html), Vec::<String>::new());
    assert!(
        html.contains("heads/main") && html.contains("/alice/arch"),
        "{html}"
    );
    assert!(html.contains("すべての検証に通りました"), "{html}");
    assert_eq!(w.stage_dirs(), 0);
    let refs = api_get(&w, "/api/v1/repos/alice/arch/refs", Some("tok-alice")).await;
    assert!(refs.text().contains("heads/main"));
    // Used once: the stage page now says it is gone (404, the neutral answer).
    assert_eq!(
        page(&w, "GET", &location, Some(&alice), "en").await.status,
        StatusCode::NOT_FOUND
    );
    let r = form(&w, &location, Some(&alice), &right, "en").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_page_can_create_without_trusting_any_key_and_says_what_that_means() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("page-none", |_, rate| *rate = 10_000);
    let (archive, _) = simple_archive(&w.base, "maint", 1, 0);
    let alice = w.session("alice");
    let r = upload(
        &w,
        Some(&alice),
        &[
            ("csrf", "tok123"),
            ("owner", "alice"),
            ("name", "arch"),
            ("visibility", "public"),
            ("proposals", "members"),
        ],
        &std::fs::read(&archive).unwrap(),
        "en",
    )
    .await;
    let location = r.h("location").to_owned();
    let r = form(&w, &location, Some(&alice), "csrf=tok123&mode=none", "en").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let html = r.text();
    dump("created-none", "en", &html);
    assert!(
        html.contains("Nothing is verified or trusted until a key is trusted"),
        "{html}"
    );
    assert!(html.contains("No key was trusted."), "{html}");
    assert!(html.contains("named remotes/heads/main"), "{html}");
    assert_eq!(violations(&html), Vec::<String>::new());
}

#[tokio::test]
async fn upload_errors_on_the_page_are_forms_again_and_a_tampered_archive_keeps_nothing() {
    need_prikk!();
    let _g = SERIAL.lock().await;
    let w = world("page-upload-errors", |_, rate| *rate = 10_000);
    let (archive, _) = simple_archive(&w.base, "maint", 1, 0);
    let alice = w.session("alice");
    let fields = [
        ("csrf", "tok123"),
        ("owner", "alice"),
        ("name", "arch"),
        ("visibility", "private"),
        ("proposals", "members"),
    ];
    // Tampered: prikk's words, nothing kept.
    let bad = tampered(&archive, &w.base.join("bad.archive"));
    let r = upload(
        &w,
        Some(&alice),
        &fields,
        &std::fs::read(bad).unwrap(),
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.text().contains("The archive was not accepted.") && r.text().contains("own words:"),
        "{}",
        r.text()
    );
    assert_eq!((w.stage_dirs(), w.stages.all().unwrap().len()), (0, 0));
    // A name that breaks a rule is said before the file is read.
    let mut f2 = fields;
    f2[2] = ("name", "Bad Name");
    let r = upload(
        &w,
        Some(&alice),
        &f2,
        &std::fs::read(&archive).unwrap(),
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.text().contains("Use lowercase letters only"));
    assert_eq!(w.stage_dirs(), 0);
    // Someone else's namespace: one sentence, before the file is read.
    let mut f3 = fields;
    f3[1] = ("owner", "acme");
    let mallory = w.session("mallory");
    let r = upload(
        &w,
        Some(&mallory),
        &f3,
        &std::fs::read(&archive).unwrap(),
        "en",
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.text().contains("You cannot create a repository there."));
    // A form with no file; a field out of the allowed set; the file before the fields; a body that is not a form.
    let (ct, no_file) = {
        let boundary = "----b";
        let mut body = Vec::new();
        for (k, v) in &fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    };
    let req = Request::builder()
        .method("POST")
        .uri("/new/import")
        .header("host", "forge.example")
        .header("cookie", alice.clone())
        .header("content-type", ct)
        .header("content-length", no_file.len().to_string())
        .body(Body::from(no_file))
        .unwrap();
    let r = send(&w.router, req).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.text().contains("Choose the archive file to upload."),
        "{}",
        r.text()
    );
    let req = Request::builder()
        .method("POST")
        .uri("/new/import")
        .header("host", "forge.example")
        .header("cookie", alice.clone())
        .header("content-type", "application/x-www-form-urlencoded")
        .header("content-length", "5")
        .body(Body::from("a=b&c"))
        .unwrap();
    assert_eq!(
        send(&w.router, req).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(w.stage_dirs(), 0);
}
