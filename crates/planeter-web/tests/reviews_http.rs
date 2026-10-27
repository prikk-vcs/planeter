//! RFC 005 part 6 over HTTP: comments, decisions, hiding, the review in the detail and the list, and the pages with their forms —
//! through the real router, with the proposals **seeded in the records** (no prikk is run: the landing observation fails to
//! start and the pages say nothing about patches, which these tests do not need). One suite for the JSON and the HTML, in the
//! two runs of the gates alike.
//!
//! The clock is one static, so the tests take turns (`SERIAL`) and each sets it first.

use std::path::PathBuf;
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
    ChangeRecord, ChangeState, ChangeStore, InMemoryChangeStore, InMemoryLeaseRecords,
    InMemoryMembershipStore, InMemoryRepositoryStore, InMemoryReviewStore, InMemoryRevisions,
    MembershipStore, Owner, ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore, ReviewStore,
    RevisionRecord, RevisionStore, Role, Subject, UserId, Visibility,
};
use planeter_transport::{
    ArtifactStore, ExportConfig, ExportService, Metrics, ProposalConfig, ProposalService,
    ReviewConfig, ReviewService, Spool,
};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};

#[allow(dead_code)]
#[path = "structure/dom.rs"]
mod dom;
#[allow(dead_code)]
#[path = "structure/rules.rs"]
mod rules;

static NOW: AtomicU64 = AtomicU64::new(1_000_000);
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}
fn at(t: u64) {
    NOW.store(t, Ordering::Relaxed);
}
const BASE: u64 = 1_000_000;
const GRACE: u64 = 30 * 86_400;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn pid(n: u64) -> String {
    format!("{n:064x}")
}

struct World {
    router: Router,
    changes: Arc<InMemoryChangeStore>,
    reviews: Arc<InMemoryReviewStore>,
    sessions: Arc<planeter_auth::InMemorySessionStore>,
}

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("reviews-http-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// alice owns `open` (public) and `secret` (private), and so administers both; bob may read, dave and gina may write;
/// carol has no role (she can still read `open`).
fn world(name: &str, cfg: ReviewConfig) -> World {
    at(BASE);
    let base = dir(name);
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (id, repo, vis) in [
        ("r-open", "open", Visibility::Public),
        ("r-secret", "secret", Visibility::Private),
    ] {
        let path = base.join(repo);
        std::fs::create_dir_all(path.join(".prikk")).unwrap();
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(id),
                owner: Owner::User("alice".into()),
                name: repo.into(),
                visibility: vis,
                created_at: 0,
                prikk_format_version: None,
                proposals: ProposalPolicy::default(),
                path,
            })
            .unwrap();
    }
    let hosting = Arc::new(
        HostingService::new(base.join("root"), store.clone() as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined)
            .with_binary("/nonexistent/prikk"),
    );
    let membership = Arc::new(InMemoryMembershipStore::new());
    for (who, role) in [
        ("bob", Role::Read),
        ("dave", Role::Write),
        ("gina", Role::Write),
    ] {
        membership
            .grant_repo_role(Subject::User(UserId::new(who)), RepoId::new("r-open"), role)
            .unwrap();
    }
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        membership.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user, access) in [
        ("tok-alice", "alice", ScopeAccess::Full),
        ("tok-alice-write", "alice", ScopeAccess::Write),
        ("tok-bob", "bob", ScopeAccess::Write),
        ("tok-bob-ro", "bob", ScopeAccess::ReadOnly),
        ("tok-carol", "carol", ScopeAccess::Write),
        ("tok-dave", "dave", ScopeAccess::Write),
        ("tok-dave-ro", "dave", ScopeAccess::ReadOnly),
        ("tok-gina", "gina", ScopeAccess::Write),
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
    let spool = Spool::open(base.join("spool"), 64 << 20).unwrap();
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        now,
        Duration::from_secs(3),
    ));
    let metrics = Arc::new(Metrics::new());
    let export = Arc::new(ExportService::new(
        hosting.clone(),
        spool,
        leases,
        Arc::new(InMemoryRevisions::new()) as Arc<dyn RevisionStore>,
        ExportConfig::default(),
        metrics.clone(),
    ));
    let artifacts = Arc::new(ArtifactStore::open(base.join("artifacts")).unwrap());
    let changes = Arc::new(InMemoryChangeStore::new());
    let reviews = Arc::new(InMemoryReviewStore::new());
    let proposals = Arc::new(
        ProposalService::new(
            hosting,
            changes.clone() as Arc<dyn ChangeStore>,
            export.clone(),
            artifacts,
            metrics,
            ProposalConfig::default(),
        )
        .with_clock(now),
    );
    let review_svc = Arc::new(
        ReviewService::new(
            changes.clone() as Arc<dyn ChangeStore>,
            reviews.clone() as Arc<dyn ReviewStore>,
            cfg,
        )
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
        transport: Some(Arc::new(TransportRuntime {
            export,
            propose: None,
            recovery: None,
            proposals: Some(proposals),
            reviews: Some(review_svc),
            ip_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
            principal_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
        })),
    });
    World {
        router,
        changes,
        reviews,
        sessions,
    }
}

impl World {
    /// A proposal in `repo` by `opener` with one revision (patches `ids`).
    fn propose(&self, repo: &str, id: &str, opener: &str, ids: &[u64], digest: u64) {
        let change = ChangeRecord {
            change_id: id.into(),
            repo_id: RepoId::new(repo),
            title: format!("Title of {id}"),
            description: "Some *words*.".into(),
            target_ref: None,
            state: ChangeState::Open,
            closed_reason: None,
            opened_by: opener.into(),
            opened_at: now(),
            updated_at: now(),
            retain_until: None,
        };
        self.revision(repo, Some(&change), id, opener, 1, ids, digest);
    }

    #[allow(clippy::too_many_arguments)]
    fn revision(
        &self,
        repo: &str,
        new: Option<&ChangeRecord>,
        id: &str,
        who: &str,
        seq: u32,
        ids: &[u64],
        digest: u64,
    ) {
        let rev = RevisionRecord {
            revision_id: format!("v-{id}-{seq}"),
            change_id: id.into(),
            repo_id: RepoId::new(repo),
            seq,
            artifact_digest: format!("sha256:{digest:064x}"),
            artifact_bytes: 10,
            declared_base: None,
            claim_ids: vec![pid(500 + u64::from(seq))],
            added_patch_ids: ids.iter().map(|n| pid(*n)).collect(),
            declared_patch_ids: None,
            claim_outcomes: vec![],
            submitter: who.into(),
            author_key_ids: vec![],
            created_at: now(),
        };
        let intent = format!("i-{id}-{seq}");
        self.changes
            .open_or_resume_intent(
                &intent,
                &RepoId::new(repo),
                &rev.artifact_digest,
                0,
                &[],
                who,
                now(),
            )
            .unwrap();
        self.changes
            .complete_intent_with_revision(&intent, new, &rev, now())
            .unwrap();
    }

    fn close(&self, id: &str) {
        self.changes
            .leave_open(
                id,
                ChangeState::Closed,
                Some("done"),
                now(),
                Some(now() + GRACE),
            )
            .unwrap();
    }

    fn session(&self, user: &str) -> String {
        let s = self
            .sessions
            .create(UserId::new(user), now(), now() + 3600)
            .unwrap();
        format!("planeter_csrf=tok123; planeter_session={}", s.id.0)
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
    let asked = std::env::var("REVIEWS_TRANSCRIPT").is_ok().then(|| {
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
    if let (Some(asked), Ok(path)) = (asked, std::env::var("REVIEWS_TRANSCRIPT")) {
        use std::io::Write as _;
        let is_json = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("application/json"));
        let shown = if is_json {
            String::from_utf8_lossy(&body)
                .chars()
                .take(900)
                .collect::<String>()
        } else {
            format!("(a page of {} bytes)", body.len())
        };
        let retry = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(|v| format!("\n    retry-after: {v}"))
            .unwrap_or_default();
        let test = std::thread::current().name().unwrap_or("?").to_owned();
        if is_json {
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

async fn api_get(w: &World, uri: &str, tok: Option<&str>) -> Reply {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(t) = tok {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    send(&w.router, req.body(Body::empty()).unwrap()).await
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

fn base(repo: &str, id: &str) -> String {
    format!("/api/v1/repos/alice/{repo}/proposals/{id}")
}

async fn comment(
    w: &World,
    repo: &str,
    id: &str,
    tok: Option<&str>,
    body: serde_json::Value,
) -> Reply {
    api_post(
        w,
        &format!("{}/comments", base(repo, id)),
        tok,
        &body.to_string(),
    )
    .await
}

async fn decide(
    w: &World,
    repo: &str,
    id: &str,
    tok: Option<&str>,
    body: serde_json::Value,
) -> Reply {
    api_post(
        w,
        &format!("{}/decisions", base(repo, id)),
        tok,
        &body.to_string(),
    )
    .await
}

async fn detail(w: &World, repo: &str, id: &str, tok: Option<&str>) -> serde_json::Value {
    let r = api_get(w, &base(repo, id), tok).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()
}

// ------------------------------------------------------------------------------------------------ comments

#[tokio::test]
async fn a_comment_is_added_by_any_signed_in_reader_even_with_a_read_only_token_and_shows_in_the_detail()
 {
    let _g = SERIAL.lock().await;
    let w = world("comment", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1, 2], 100);
    // bob only reads the repository, and this token is read-only: commenting is still his.
    let r = comment(&w, "open", "p1", Some("tok-bob-ro"), serde_json::json!({"body": "Nice *work*.\n\n<script>alert(1)</script><img src=x onerror=alert(2)>"})).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let c = r.json();
    assert_eq!(c["author"], "bob");
    assert_eq!(c["revision"], 1);
    assert_eq!(c["hidden"], false);
    assert!(
        c["body"]["html"]
            .as_str()
            .unwrap()
            .contains("<em>work</em>")
    );
    let html = c["body"]["html"].as_str().unwrap();
    assert!(
        !html.contains("<script") && !html.contains("onerror"),
        "sanitised: {html}"
    );
    assert!(
        c["body"]["source"].as_str().unwrap().contains("<script>"),
        "the source is as written"
    );
    // Anyone who can read the proposal sees it in the detail, anonymously too.
    let d = detail(&w, "open", "p1", None).await;
    assert_eq!(d["comments"]["total"], 1);
    assert_eq!(d["comments"]["items"][0]["id"], c["id"]);
    assert_eq!(d["comments"]["items"][0]["replies"], serde_json::json!([]));
    assert_eq!(d["review"]["state"], "awaiting_review");
}

#[tokio::test]
async fn a_comment_may_be_anchored_and_a_reply_nests_one_level() {
    let _g = SERIAL.lock().await;
    let w = world("anchor", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1, 2], 100);
    let top = comment(&w, "open", "p1", Some("tok-bob"), serde_json::json!({
        "body": "About this line.", "patch_id": pid(1), "path": "src/lib.rs", "line_from": 12, "line_to": 14
    })).await;
    assert_eq!(top.status, StatusCode::CREATED, "{}", top.text());
    let top = top.json();
    assert_eq!(
        (
            top["patch_id"].clone(),
            top["path"].clone(),
            top["line_from"].clone(),
            top["line_to"].clone()
        ),
        (pid(1).into(), "src/lib.rs".into(), 12.into(), 14.into())
    );
    at(now() + 5);
    let reply = comment(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"body": "Agreed.", "parent": top["id"]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
    let reply = reply.json();
    // A reply to a reply: refused by name.
    let deeper = comment(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"body": "x", "parent": reply["id"]}),
    )
    .await;
    assert_eq!(deeper.status, StatusCode::BAD_REQUEST);
    assert_eq!(deeper.json()["reason"], "parent_not_top_level");
    let d = detail(&w, "open", "p1", None).await;
    let items = d["comments"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "a reply is not a thread of its own");
    assert_eq!(items[0]["replies"][0]["id"], reply["id"]);
    assert_eq!(d["comments"]["total"], 2);
}

#[tokio::test]
async fn every_comment_rule_has_its_named_refusal() {
    let _g = SERIAL.lock().await;
    let w = world(
        "comment-rules",
        ReviewConfig {
            comments_per_hour: 1000,
            decisions_per_hour: 1000,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.propose("r-open", "p2", "olivia", &[2], 101);
    let other = comment(
        &w,
        "open",
        "p2",
        Some("tok-bob"),
        serde_json::json!({"body": "On p2."}),
    )
    .await
    .json();
    let cases: Vec<(serde_json::Value, &str)> = vec![
        (serde_json::json!({}), "body_empty"),
        (serde_json::json!({"body": "   "}), "body_empty"),
        (
            serde_json::json!({"body": "x".repeat(16 * 1024 + 1)}),
            "body_too_long",
        ),
        (
            serde_json::json!({"body": "x", "patch_id": pid(9)}),
            "patch_not_in_set",
        ),
        (
            serde_json::json!({"body": "x", "patch_id": "abc"}),
            "patch_id_invalid",
        ),
        (serde_json::json!({"body": "x", "path": ""}), "path_empty"),
        (
            serde_json::json!({"body": "x", "path": "p".repeat(1025)}),
            "path_too_long",
        ),
        (
            serde_json::json!({"body": "x", "path": "a\nb"}),
            "path_invalid",
        ),
        (
            serde_json::json!({"body": "x", "line_from": 3}),
            "lines_need_a_path",
        ),
        (
            serde_json::json!({"body": "x", "path": "a", "line_to": 3}),
            "lines_incomplete",
        ),
        (
            serde_json::json!({"body": "x", "path": "a", "line_from": 0, "line_to": 3}),
            "line_zero",
        ),
        (
            serde_json::json!({"body": "x", "path": "a", "line_from": 5, "line_to": 3}),
            "line_order",
        ),
        (
            serde_json::json!({"body": "x", "parent": "m-ghost"}),
            "parent_unknown",
        ),
        (
            serde_json::json!({"body": "x", "parent": other["id"]}),
            "parent_other_proposal",
        ),
        (
            serde_json::json!({"body": "x", "surprise": 1}),
            "unknown_field",
        ),
    ];
    for (body, reason) in cases {
        let r = comment(&w, "open", "p1", Some("tok-bob"), body.clone()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{reason}: {}", r.text());
        assert_eq!(r.code(), "invalid_request", "{reason}");
        assert_eq!(r.json()["reason"], reason);
        assert!(
            r.json()["hint"]
                .as_str()
                .unwrap()
                .contains("send the comment again"),
            "{}",
            r.text()
        );
    }
    let r = api_post(
        &w,
        &format!("{}/comments", base("open", "p1")),
        Some("tok-bob"),
        "not json",
    )
    .await;
    assert_eq!(r.json()["reason"], "not_json");
    let r = api_post(
        &w,
        &format!("{}/comments", base("open", "p1")),
        Some("tok-bob"),
        &"x".repeat(200_000),
    )
    .await;
    assert_eq!(r.json()["reason"], "too_large");
    assert_eq!(
        detail(&w, "open", "p1", None).await["comments"]["total"],
        0,
        "nothing was recorded"
    );
}

#[tokio::test]
async fn who_may_comment_and_what_a_stranger_sees() {
    let _g = SERIAL.lock().await;
    let w = world("comment-who", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.propose("r-secret", "s1", "olivia", &[2], 101);
    // Anonymous: forbidden, with the hint for it. A cookie is not a credential here.
    let r = comment(&w, "open", "p1", None, serde_json::json!({"body": "x"})).await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    assert!(
        r.json()["message"].as_str().unwrap().contains("signed in"),
        "{}",
        r.text()
    );
    let cookie = w.session("bob");
    let req = Request::builder()
        .method("POST")
        .uri(format!("{}/comments", base("open", "p1")))
        .header("cookie", cookie)
        .body(Body::from(r#"{"body":"x"}"#))
        .unwrap();
    assert_eq!(send(&w.router, req).await.status, StatusCode::FORBIDDEN);
    // A signed-in stranger of a public repository may comment (carol holds no role).
    assert_eq!(
        comment(
            &w,
            "open",
            "p1",
            Some("tok-carol"),
            serde_json::json!({"body": "hello"})
        )
        .await
        .status,
        StatusCode::CREATED
    );
    // A private repository: a stranger is told nothing — the same 404 as for an absent proposal.
    let hidden = comment(
        &w,
        "secret",
        "s1",
        Some("tok-carol"),
        serde_json::json!({"body": "x"}),
    )
    .await;
    let absent = comment(
        &w,
        "secret",
        "nope",
        Some("tok-carol"),
        serde_json::json!({"body": "x"}),
    )
    .await;
    let no_repo = comment(
        &w,
        "nowhere",
        "p1",
        Some("tok-carol"),
        serde_json::json!({"body": "x"}),
    )
    .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.body, absent.body);
    assert_eq!(hidden.body, no_repo.body);
    // The owner of the private repository may comment there; an unknown proposal is 404, another repository's too.
    assert_eq!(
        comment(
            &w,
            "secret",
            "s1",
            Some("tok-alice"),
            serde_json::json!({"body": "ok"})
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        comment(
            &w,
            "open",
            "s1",
            Some("tok-bob"),
            serde_json::json!({"body": "x"})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        comment(
            &w,
            "open",
            "nope",
            Some("tok-bob"),
            serde_json::json!({"body": "x"})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn comments_close_when_the_grace_of_a_finished_proposal_ends() {
    let _g = SERIAL.lock().await;
    let w = world(
        "comment-window",
        ReviewConfig {
            comments_per_hour: 1000,
            decisions_per_hour: 1000,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.close("p1");
    let end = now() + GRACE;
    at(end);
    assert_eq!(
        comment(
            &w,
            "open",
            "p1",
            Some("tok-bob"),
            serde_json::json!({"body": "after the fact"})
        )
        .await
        .status,
        StatusCode::CREATED
    );
    at(end + 1);
    let r = comment(
        &w,
        "open",
        "p1",
        Some("tok-bob"),
        serde_json::json!({"body": "too late"}),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "comments_closed")
    );
    assert_eq!(r.json()["state"], "closed");
    assert_eq!(r.json()["kept_until"], end);
    assert!(
        r.json()["message"].as_str().unwrap().contains("1970-"),
        "the date is in the sentence: {}",
        r.text()
    );
    // The comments that were made stay readable.
    assert_eq!(detail(&w, "open", "p1", None).await["comments"]["total"], 1);
}

#[tokio::test]
async fn the_comment_limit_is_a_429_with_retry_after_and_is_per_proposal_per_person() {
    let _g = SERIAL.lock().await;
    let w = world(
        "comment-rate",
        ReviewConfig {
            comments_per_hour: 2,
            decisions_per_hour: 10,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.propose("r-open", "p2", "olivia", &[2], 101);
    for i in 0..2 {
        assert_eq!(
            comment(
                &w,
                "open",
                "p1",
                Some("tok-bob"),
                serde_json::json!({"body": format!("c{i}")})
            )
            .await
            .status,
            StatusCode::CREATED
        );
        at(now() + 10);
    }
    let r = comment(
        &w,
        "open",
        "p1",
        Some("tok-bob"),
        serde_json::json!({"body": "one too many"}),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    );
    assert_eq!(
        r.h("retry-after"),
        (3600 - 20).to_string(),
        "until the oldest one is an hour old"
    );
    assert!(
        r.json()["message"]
            .as_str()
            .unwrap()
            .contains("commented on this proposal"),
        "{}",
        r.text()
    );
    assert_eq!(
        comment(
            &w,
            "open",
            "p2",
            Some("tok-bob"),
            serde_json::json!({"body": "elsewhere"})
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        comment(
            &w,
            "open",
            "p1",
            Some("tok-carol"),
            serde_json::json!({"body": "someone else"})
        )
        .await
        .status,
        StatusCode::CREATED
    );
}

// ------------------------------------------------------------------------------------------------ decisions

#[tokio::test]
async fn only_reviewers_decide_and_the_review_state_follows_their_decisions() {
    let _g = SERIAL.lock().await;
    let w = world("decide", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    // bob reads only; a read-only token of a reviewer is not enough either; anonymous is forbidden.
    for tok in [
        Some("tok-bob"),
        Some("tok-dave-ro"),
        Some("tok-carol"),
        None,
    ] {
        let r = decide(
            &w,
            "open",
            "p1",
            tok,
            serde_json::json!({"decision": "approve"}),
        )
        .await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{tok:?}"
        );
        assert!(
            r.json()["message"]
                .as_str()
                .unwrap()
                .contains("only people who review here"),
            "{}",
            r.text()
        );
    }
    assert_eq!(
        detail(&w, "open", "p1", None).await["review"]["state"],
        "awaiting_review"
    );
    let r = decide(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve", "note": "  Looks right.  "}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["decision"]["decision"], "approve");
    assert_eq!(r.json()["decision"]["note"], "Looks right.");
    assert_eq!(r.json()["review"]["state"], "approved");
    at(now() + 5);
    assert_eq!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-gina"),
            serde_json::json!({"decision": "request-changes", "note": "One thing."})
        )
        .await
        .status,
        StatusCode::OK
    );
    let d = detail(&w, "open", "p1", None).await;
    assert_eq!(d["review"]["state"], "changes_requested");
    assert_eq!(
        (
            d["review"]["approved_count"].clone(),
            d["review"]["changes_requested_count"].clone()
        ),
        (1.into(), 1.into())
    );
    let who: Vec<(String, String)> = d["review"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["reviewer"].as_str().unwrap().to_owned(),
                x["decision"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        who,
        [
            ("dave".to_owned(), "approve".to_owned()),
            ("gina".to_owned(), "request-changes".to_owned())
        ]
    );
    // gina changes her mind: the newest wins, and she is still one reviewer.
    at(now() + 5);
    decide(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    let d = detail(&w, "open", "p1", None).await;
    assert_eq!(
        (
            d["review"]["state"].clone(),
            d["review"]["approved_count"].clone()
        ),
        ("approved".into(), 2.into())
    );
    assert_eq!(w.reviews.decision_history("p1").unwrap().len(), 1);
    // The list says the same word for the open proposal.
    let list = api_get(&w, "/api/v1/repos/alice/open/proposals", None)
        .await
        .json();
    assert_eq!(list["proposals"][0]["review"], "approved");
}

/// The test the handoff names, over HTTP: approve revision 1, then a second revision arrives.
#[tokio::test]
async fn an_approval_of_revision_one_is_shown_on_revision_two_and_counts_for_nothing() {
    let _g = SERIAL.lock().await;
    let w = world("stale", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    assert_eq!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-dave"),
            serde_json::json!({"decision": "approve"})
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        detail(&w, "open", "p1", None).await["review"]["state"],
        "approved"
    );
    w.revision("r-open", None, "p1", "olivia", 2, &[2], 101);
    let d = detail(&w, "open", "p1", None).await;
    assert_eq!(d["review"]["state"], "awaiting_review");
    assert_eq!(d["review"]["revision"], 2);
    assert_eq!(d["review"]["approved_on"], serde_json::json!([1]));
    assert_eq!(d["review"]["approved_count"], 0);
    assert_eq!(d["review"]["decisions"], serde_json::json!([]));
    assert_eq!(d["review"]["earlier_decisions"][0]["reviewer"], "dave");
    let list = api_get(&w, "/api/v1/repos/alice/open/proposals", None)
        .await
        .json();
    assert_eq!(list["proposals"][0]["review"], "awaiting_review");
    // Naming the old revision is refused, with the newest named.
    let r = decide(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"decision": "approve", "revision": 1}),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "revision_not_current")
    );
    assert_eq!(r.json()["current"], 2);
    assert!(
        r.json()["message"].as_str().unwrap().contains("revision 2"),
        "{}",
        r.text()
    );
    // Deciding on the new revision is dave's to do again.
    assert_eq!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-dave"),
            serde_json::json!({"decision": "approve", "revision": 2})
        )
        .await
        .status,
        StatusCode::OK
    );
    let d = detail(&w, "open", "p1", None).await;
    assert_eq!(
        (
            d["review"]["state"].clone(),
            d["review"]["approved_on"].clone()
        ),
        ("approved".into(), serde_json::json!([1]))
    );
}

#[tokio::test]
async fn nobody_approves_their_own_work_but_may_ask_for_changes() {
    let _g = SERIAL.lock().await;
    let w = world("self", ReviewConfig::default());
    w.propose("r-open", "p1", "dave", &[1], 100);
    let r = decide(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "self_approval")
    );
    assert_eq!(r.json()["reason"], "own_revision");
    // gina sends the second revision: dave submitted an earlier one, so he cannot approve the proposal either.
    w.revision("r-open", None, "p1", "gina", 2, &[2], 101);
    let r = decide(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(r.json()["reason"], "own_proposal");
    let r = decide(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(r.json()["reason"], "own_revision");
    assert_eq!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-dave"),
            serde_json::json!({"decision": "request-changes"})
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        comment(
            &w,
            "open",
            "p1",
            Some("tok-dave"),
            serde_json::json!({"body": "my own comment"})
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-alice"),
            serde_json::json!({"decision": "approve"})
        )
        .await
        .status,
        StatusCode::OK,
        "someone else may"
    );
}

#[tokio::test]
async fn a_decision_is_refused_for_a_finished_proposal_and_for_bad_input() {
    let _g = SERIAL.lock().await;
    let w = world(
        "decide-rules",
        ReviewConfig {
            comments_per_hour: 100,
            decisions_per_hour: 100,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.propose("r-open", "p2", "olivia", &[2], 101);
    w.close("p2");
    let r = decide(
        &w,
        "open",
        "p2",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "decisions_closed")
    );
    assert_eq!(r.json()["state"], "closed");
    for (body, reason) in [
        (serde_json::json!({}), "decision_unknown"),
        (serde_json::json!({"decision": "maybe"}), "decision_unknown"),
        (
            serde_json::json!({"decision": "Approve"}),
            "decision_unknown",
        ),
        (
            serde_json::json!({"decision": "approve", "note": "n".repeat(2049)}),
            "note_too_long",
        ),
        (
            serde_json::json!({"decision": "approve", "extra": true}),
            "unknown_field",
        ),
    ] {
        let r = decide(&w, "open", "p1", Some("tok-dave"), body).await;
        assert_eq!(
            (r.status, r.json()["reason"].as_str().unwrap().to_owned()),
            (StatusCode::BAD_REQUEST, reason.to_owned()),
            "{}",
            r.text()
        );
        assert!(
            r.json()["hint"]
                .as_str()
                .unwrap()
                .contains("send the decision again"),
            "{}",
            r.text()
        );
    }
    assert!(w.reviews.current_decisions("p1").unwrap().is_empty());
    assert_eq!(
        decide(
            &w,
            "open",
            "nope",
            Some("tok-dave"),
            serde_json::json!({"decision": "approve"})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_decision_limit_is_a_429_and_counts_replaced_decisions() {
    let _g = SERIAL.lock().await;
    let w = world(
        "decide-rate",
        ReviewConfig {
            comments_per_hour: 20,
            decisions_per_hour: 2,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    for v in ["approve", "request-changes"] {
        assert_eq!(
            decide(
                &w,
                "open",
                "p1",
                Some("tok-dave"),
                serde_json::json!({"decision": v})
            )
            .await
            .status,
            StatusCode::OK
        );
        at(now() + 7);
    }
    let r = decide(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
    );
    assert_eq!(r.h("retry-after"), (3600 - 14).to_string());
    assert_eq!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-gina"),
            serde_json::json!({"decision": "approve"})
        )
        .await
        .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn two_reviewers_at_once_both_count_and_one_reviewer_twice_at_once_is_one_decision() {
    let _g = SERIAL.lock().await;
    let w = world(
        "concurrent",
        ReviewConfig {
            comments_per_hour: 100,
            decisions_per_hour: 100,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    let (a, b) = tokio::join!(
        decide(
            &w,
            "open",
            "p1",
            Some("tok-dave"),
            serde_json::json!({"decision": "approve"})
        ),
        decide(
            &w,
            "open",
            "p1",
            Some("tok-gina"),
            serde_json::json!({"decision": "approve"})
        ),
    );
    assert_eq!((a.status, b.status), (StatusCode::OK, StatusCode::OK));
    assert_eq!(
        detail(&w, "open", "p1", None).await["review"]["approved_count"],
        2
    );
    // The same reviewer, twelve times at once, with different verdicts: exactly one current decision for her.
    let mut calls = Vec::new();
    for i in 0..12 {
        let v = if i % 2 == 0 {
            "approve"
        } else {
            "request-changes"
        };
        calls.push(decide(
            &w,
            "open",
            "p1",
            Some("tok-alice"),
            serde_json::json!({"decision": v}),
        ));
    }
    let replies = futures_join_all(calls).await;
    assert!(replies.iter().all(|r| r.status == StatusCode::OK));
    let current: Vec<_> = w
        .reviews
        .current_decisions("p1")
        .unwrap()
        .into_iter()
        .filter(|d| d.reviewer == "alice")
        .collect();
    assert_eq!(current.len(), 1, "one current decision for the key");
    assert_eq!(
        w.reviews
            .decision_history("p1")
            .unwrap()
            .iter()
            .filter(|d| d.reviewer == "alice")
            .count(),
        11
    );
}

/// `futures::future::join_all` without the dependency: the requests run on one task, in turn, but each is polled to completion
/// in the order given while the others wait — enough to interleave them at every `await` inside the handlers.
async fn futures_join_all<F: std::future::Future<Output = Reply>>(calls: Vec<F>) -> Vec<Reply> {
    let mut handles = Vec::new();
    for c in calls {
        handles.push(Box::pin(c));
    }
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await);
    }
    out
}

// ------------------------------------------------------------------------------------------------ hiding

const SECRET_WORDS: &str = "the-rude-words-that-must-not-leak";

#[tokio::test]
async fn a_hidden_comments_text_is_released_to_administrators_only() {
    let _g = SERIAL.lock().await;
    let w = world("hide", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.propose("r-open", "p2", "olivia", &[2], 101);
    let c = comment(
        &w,
        "open",
        "p1",
        Some("tok-bob"),
        serde_json::json!({"body": format!("**{SECRET_WORDS}**")}),
    )
    .await
    .json();
    let cid = c["id"].as_str().unwrap().to_owned();
    let hide = |tok: Option<&'static str>, reason: &'static str| {
        let uri = format!("{}/comments/{cid}/hide", base("open", "p1"));
        let w = &w;
        async move {
            api_post(
                w,
                &uri,
                tok,
                &serde_json::json!({"reason": reason}).to_string(),
            )
            .await
        }
    };
    // Only RepoAdmin: dave writes here and may review, but may not moderate; a token that cannot administer cannot either.
    for tok in [
        Some("tok-dave"),
        Some("tok-bob"),
        Some("tok-alice-write"),
        None,
    ] {
        let r = hide(tok, "no").await;
        assert_eq!(
            (r.status, r.code().as_str()),
            (StatusCode::FORBIDDEN, "forbidden"),
            "{tok:?}"
        );
        assert!(
            r.json()["message"].as_str().unwrap().contains("administer"),
            "{}",
            r.text()
        );
    }
    assert!(
        w.reviews
            .get_comment(&cid)
            .unwrap()
            .unwrap()
            .hidden
            .is_none()
    );
    // The reason is required and bounded.
    for bad in ["", "   "] {
        let r = hide(Some("tok-alice"), bad).await;
        assert_eq!(
            (r.status, r.json()["reason"].clone()),
            (StatusCode::BAD_REQUEST, "reason_required".into())
        );
    }
    let r = hide(
        Some("tok-alice"),
        Box::leak("r".repeat(501).into_boxed_str()),
    )
    .await;
    assert_eq!(r.json()["reason"], "reason_too_long");
    // The administrator hides it and still reads it.
    let r = hide(Some("tok-alice"), "Rude.").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["hidden"], true);
    assert!(
        r.json()["body"]["source"]
            .as_str()
            .unwrap()
            .contains(SECRET_WORDS),
        "the administrator reads it"
    );
    assert_eq!(r.json()["hidden_by"], "alice");
    // Everyone else: hidden, the reason, and no body anywhere in the JSON.
    for tok in [
        None,
        Some("tok-bob"),
        Some("tok-dave"),
        Some("tok-carol"),
        Some("tok-alice-write"),
    ] {
        let r = api_get(&w, &base("open", "p1"), tok).await;
        assert!(
            !r.text().contains(SECRET_WORDS),
            "{tok:?} saw the text of a hidden comment: {}",
            r.text()
        );
        let item = &r.json()["comments"]["items"][0];
        assert_eq!(
            (
                item["hidden"].clone(),
                item["hidden_reason"].clone(),
                item["body"].clone()
            ),
            (true.into(), "Rude.".into(), serde_json::Value::Null)
        );
        assert_eq!(
            (item["hidden_by"].clone(), item["hidden_at"].clone()),
            (serde_json::Value::Null, serde_json::Value::Null),
            "who and when are for administrators"
        );
    }
    let admin = api_get(&w, &base("open", "p1"), Some("tok-alice")).await;
    assert!(admin.text().contains(SECRET_WORDS));
    assert_eq!(admin.json()["comments"]["items"][0]["hidden_by"], "alice");
    // Hiding twice is refused by name; so is showing what is not hidden.
    let r = hide(Some("tok-alice"), "again").await;
    assert_eq!(
        (r.status, r.code().as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "comment_state")
    );
    assert_eq!(r.json()["hidden"], true);
    // Showing it again is reversible, and recorded.
    let uri = format!("{}/comments/{cid}/unhide", base("open", "p1"));
    let r = api_post(&w, &uri, Some("tok-alice"), "").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["hidden"], false);
    assert!(
        api_get(&w, &base("open", "p1"), None)
            .await
            .text()
            .contains(SECRET_WORDS),
        "visible to everyone again"
    );
    let r = api_post(&w, &uri, Some("tok-alice"), "").await;
    assert_eq!(
        (r.status, r.code().as_str(), r.json()["hidden"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "comment_state",
            false.into()
        )
    );
    assert_eq!(w.reviews.moderation_log(&cid).unwrap().len(), 2);
    // A comment of another proposal and an unknown comment are the same 404.
    let r = api_post(
        &w,
        &format!("{}/comments/{cid}/hide", base("open", "p2")),
        Some("tok-alice"),
        r#"{"reason":"x"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = api_post(
        &w,
        &format!("{}/comments/m-ghost/hide", base("open", "p1")),
        Some("tok-alice"),
        r#"{"reason":"x"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_comments_of_the_detail_are_paged_with_a_cursor() {
    let _g = SERIAL.lock().await;
    let w = world(
        "paging",
        ReviewConfig {
            comments_per_hour: 1000,
            decisions_per_hour: 1000,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1], 100);
    for i in 0..5 {
        comment(
            &w,
            "open",
            "p1",
            Some("tok-bob"),
            serde_json::json!({"body": format!("c{i}")}),
        )
        .await;
        at(now() + 1);
    }
    let one = api_get(&w, &format!("{}?limit=2", base("open", "p1")), None)
        .await
        .json();
    let bodies = |v: &serde_json::Value| {
        v["comments"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["body"]["source"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(bodies(&one), ["c0", "c1"]);
    assert_eq!(one["comments"]["total"], 5);
    let next = one["comments"]["next"].as_str().unwrap().to_owned();
    let two = api_get(
        &w,
        &format!("{}?limit=2&after={next}", base("open", "p1")),
        None,
    )
    .await
    .json();
    assert_eq!(bodies(&two), ["c2", "c3"]);
    let three = api_get(
        &w,
        &format!(
            "{}?limit=2&after={}",
            base("open", "p1"),
            two["comments"]["next"].as_str().unwrap()
        ),
        None,
    )
    .await
    .json();
    assert_eq!(bodies(&three), ["c4"]);
    assert!(three["comments"]["next"].is_null());
    assert_eq!(
        api_get(&w, &format!("{}?limit=0", base("open", "p1")), None)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        api_get(&w, &format!("{}?after=nonsense", base("open", "p1")), None)
            .await
            .status,
        StatusCode::BAD_REQUEST
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

async fn form(w: &World, uri: &str, cookie: Option<&str>, body: &str) -> Reply {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("host", "forge.example")
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    send(&w.router, req.body(Body::from(body.to_owned())).unwrap()).await
}

fn violations(html: &str) -> Vec<String> {
    rules::check(&dom::parse(html))
        .into_iter()
        .map(|v| format!("{}: {}", v.rule, v.detail))
        .collect()
}

fn unfilled(page: &str) -> Option<String> {
    let b: Vec<char> = page.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '{' {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_lowercase() || b[j] == '_') {
                j += 1;
            }
            if j > i + 1 && j < b.len() && b[j] == '}' {
                return Some(b[i..=j].iter().collect());
            }
        }
        i += 1;
    }
    None
}

/// The part of the page above `<summary>Details</summary>`: the default view.
fn default_view(html: &str) -> &str {
    html.split_once("<summary>Details</summary>")
        .map_or(html, |(a, _)| a)
}

fn details(html: &str) -> &str {
    html.split_once("<summary>Details</summary>")
        .map_or("", |(_, b)| b)
}

#[tokio::test]
async fn the_default_view_has_one_review_line_and_everything_else_is_under_details() {
    let _g = SERIAL.lock().await;
    let w = world("page-line", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    let uri = "/alice/open/proposals/p1";
    let p = page(&w, "GET", uri, None, "en").await.text();
    let d = default_view(&p);
    assert!(
        d.contains("<p class=\"review-line\"><strong>Awaiting review</strong></p>"),
        "{d}"
    );
    assert!(
        d.contains(
            "No reviewer has given a decision on revision 1 yet. Reviewing does not merge anything."
        ),
        "{d}"
    );
    assert!(
        !d.contains("Decisions on revision") && !d.contains("Comments"),
        "nothing of the review but the line: {d}"
    );
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");

    decide(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve", "note": "Fine."}),
    )
    .await;
    decide(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"decision": "approve"}),
    )
    .await;
    let p = page(&w, "GET", uri, None, "en").await.text();
    assert!(
        default_view(&p).contains("<strong>Approved by 2 people</strong>"),
        "{}",
        default_view(&p)
    );
    assert!(default_view(&p).contains("Approved here means reviewers said they are happy with revision 1. It does not merge anything."));
    let dt = details(&p);
    assert!(dt.contains("<h3>Decisions on revision 1</h3>"), "{dt}");
    assert!(
        dt.contains("dave approved revision 1")
            && dt.contains("gina approved revision 1")
            && dt.contains("Note: Fine."),
        "{dt}"
    );
    decide(
        &w,
        "open",
        "p1",
        Some("tok-alice"),
        serde_json::json!({"decision": "request-changes"}),
    )
    .await;
    let p = page(&w, "GET", uri, None, "en").await.text();
    assert!(default_view(&p).contains("<strong>Changes requested by 1 person</strong>"));
    assert!(default_view(&p).contains("A reviewer asked for changes to revision 1. That is their opinion; it does not merge or block anything."));
    assert_eq!(violations(&p), Vec::<String>::new());

    // The stale approval: revision 2 arrives.
    w.revision("r-open", None, "p1", "olivia", 2, &[2], 101);
    for who in ["dave", "gina", "alice"] {
        w.reviews.current_decisions("p1").unwrap(); // (the decisions are on revision 1; nobody has decided on 2)
        let _ = who;
    }
    let p = page(&w, "GET", uri, None, "en").await.text();
    let d = default_view(&p);
    assert!(d.contains("<strong>Awaiting review</strong>"), "{d}");
    assert!(
        d.contains("Approved on an earlier revision; revision 2 has not been reviewed."),
        "{d}"
    );
    assert!(
        details(&p).contains("Decisions on earlier revisions"),
        "{}",
        details(&p)
    );
    assert!(details(&p).contains("dave approved revision 1"));
    assert_eq!(violations(&p), Vec::<String>::new());
    // Once someone has asked for changes to revision 2 it has been reviewed: the note about the old approval is gone.
    decide(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"decision": "request-changes"}),
    )
    .await;
    let p = page(&w, "GET", uri, None, "en").await.text();
    assert!(default_view(&p).contains("Changes requested by 1 person"));
    assert!(
        !default_view(&p).contains("Approved on an earlier revision"),
        "{}",
        default_view(&p)
    );
    // Withdrawn again for the rest of the test (gina's decision is replaced by an approval of nothing: remove it).
    w.reviews
        .record_decision(&planeter_store::DecisionRecord {
            change_id: "p1".into(),
            revision_seq: 2,
            reviewer: "gina".into(),
            verdict: "approve".into(),
            note: None,
            created_at: now(),
        })
        .unwrap();
    // The list shows the same word.
    let l = page(&w, "GET", "/alice/open/proposals", None, "en")
        .await
        .text();
    assert!(
        l.contains(">Review</th>") && l.contains("<td>Approved</td>"),
        "{l}"
    );
    assert_eq!(violations(&l), Vec::<String>::new());
    // A closed proposal shows no review line and the list shows a dash.
    w.close("p1");
    let p = page(&w, "GET", uri, None, "en").await.text();
    assert!(
        !default_view(&p).contains("review-line"),
        "{}",
        default_view(&p)
    );
    let l = page(&w, "GET", "/alice/open/proposals?state=closed", None, "en")
        .await
        .text();
    assert!(l.contains("<span class=\"muted\">–</span>"), "{l}");
}

#[tokio::test]
async fn the_forms_are_for_the_people_who_may_use_them() {
    let _g = SERIAL.lock().await;
    let w = world("page-forms", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    w.propose("r-open", "p2", "dave", &[2], 101);
    comment(
        &w,
        "open",
        "p1",
        Some("tok-bob"),
        serde_json::json!({"body": "A comment to hide."}),
    )
    .await;
    let uri = "/alice/open/proposals/p1";
    let see = |cookie: Option<String>| {
        let w = &w;
        async move { page(w, "GET", uri, cookie.as_deref(), "en").await.text() }
    };
    // Anonymous: the comments, no forms at all.
    let p = see(None).await;
    assert!(
        details(&p).contains("A comment to hide."),
        "{}",
        details(&p)
    );
    for needle in [
        "<form method=\"post\" action=\"/alice/open/proposals/p1/comments",
        "/decisions",
        "/hide",
        "Add a comment",
        "Record decision",
    ] {
        assert!(
            !details(&p).contains(needle),
            "an anonymous reader sees {needle}: {}",
            details(&p)
        );
    }
    // A reader (bob): the comment form and the reply form, no decision form, no moderation.
    let p = see(Some(w.session("bob"))).await;
    assert!(details(&p).contains("Add a comment") && details(&p).contains("Your reply"));
    assert!(details(&p).contains("Everyone who can read this repository will see your comment. You cannot edit or delete it afterwards; a maintainer can hide it."));
    assert!(!details(&p).contains("Record decision") && !details(&p).contains("Hide comment"));
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");
    // A reviewer (dave): the decision form with the consequence beside the button, approve enabled.
    let p = see(Some(w.session("dave"))).await;
    let dt = details(&p);
    assert!(
        dt.contains("Your decision on revision 1") && dt.contains("Record decision"),
        "{dt}"
    );
    assert!(dt.contains("This is recorded on revision 1 only, and everyone who can read this repository will see it."), "{dt}");
    assert!(
        dt.contains("value=\"approve\" required>")
            && !dt.contains("value=\"approve\" required disabled"),
        "{dt}"
    );
    assert!(!dt.contains("Hide comment"));
    assert!(
        dt.contains("<input type=\"hidden\" name=\"revision\" value=\"1\">"),
        "the form says which revision the decision is for: {dt}"
    );
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");
    // The submitter (dave on p2): approve is disabled and the page says why; asking for changes is there.
    let p2 = page(
        &w,
        "GET",
        "/alice/open/proposals/p2",
        Some(&w.session("dave")),
        "en",
    )
    .await
    .text();
    assert!(
        details(&p2).contains("value=\"approve\" required disabled>"),
        "{}",
        details(&p2)
    );
    assert!(details(&p2).contains(
        "You sent part of this proposal, so you cannot approve it. You can still ask for changes."
    ));
    assert!(details(&p2).contains("value=\"request-changes\" required>"));
    // An administrator: moderation too.
    let p = see(Some(w.session("alice"))).await;
    assert!(
        details(&p).contains("Hide comment") && details(&p).contains("Reason for hiding"),
        "{}",
        details(&p)
    );
    assert!(details(&p).contains("The comment stays on record. Only people who administer this repository will see its text; everyone else sees your reason. You can show it again."));
    assert_eq!(violations(&p), Vec::<String>::new(), "{p}");
    // A closed proposal takes no decision form; comments within the grace still have theirs; past the grace, none.
    w.close("p1");
    let p = see(Some(w.session("dave"))).await;
    assert!(!details(&p).contains("Record decision") && details(&p).contains("Add a comment"));
    at(now() + GRACE + 1);
    let p = see(Some(w.session("dave"))).await;
    assert!(
        !details(&p).contains("Add a comment") && !details(&p).contains("Your reply"),
        "{}",
        details(&p)
    );
    assert!(
        details(&p).contains("A comment to hide."),
        "the comments stay readable"
    );
}

#[tokio::test]
async fn a_hidden_comments_text_is_never_in_a_page_for_anyone_but_an_administrator() {
    let _g = SERIAL.lock().await;
    let w = world("page-hidden", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1], 100);
    let c = comment(
        &w,
        "open",
        "p1",
        Some("tok-bob"),
        serde_json::json!({"body": format!("a {SECRET_WORDS} b")}),
    )
    .await
    .json();
    let rep = comment(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"body": format!("reply {SECRET_WORDS}"), "parent": c["id"]}),
    )
    .await
    .json();
    let hide_top = api_post(
        &w,
        &format!(
            "{}/comments/{}/hide",
            base("open", "p1"),
            c["id"].as_str().unwrap()
        ),
        Some("tok-alice"),
        r#"{"reason":"Rude <b>words</b>."}"#,
    )
    .await;
    assert_eq!(hide_top.status, StatusCode::OK);
    let hide_rep = api_post(
        &w,
        &format!(
            "{}/comments/{}/hide",
            base("open", "p1"),
            rep["id"].as_str().unwrap()
        ),
        Some("tok-alice"),
        r#"{"reason":"Same."}"#,
    )
    .await;
    assert_eq!(hide_rep.status, StatusCode::OK);
    for lang in ["en", "ja"] {
        planeter_web::catalog::enable_preview_languages("ja").unwrap();
        for cookie in [None, Some(w.session("bob")), Some(w.session("dave"))] {
            let p = page(
                &w,
                "GET",
                "/alice/open/proposals/p1",
                cookie.as_deref(),
                lang,
            )
            .await
            .text();
            assert!(
                !p.contains(SECRET_WORDS),
                "[{lang}] a page shows a hidden comment's text:\n{p}"
            );
            if lang == "en" {
                assert!(
                    p.contains("Hidden by a maintainer: Rude &#60;b&#62;words&#60;/b&#62;."),
                    "the reason, escaped: {}",
                    details(&p)
                );
            }
            assert_eq!(violations(&p), Vec::<String>::new());
        }
        let admin = page(
            &w,
            "GET",
            "/alice/open/proposals/p1",
            Some(&w.session("alice")),
            lang,
        )
        .await
        .text();
        assert!(admin.contains(SECRET_WORDS), "the administrator reads it");
        if lang == "en" {
            assert!(admin.contains("You can read it because you administer this repository."));
            assert!(admin.contains("Show comment again"));
        }
        assert_eq!(violations(&admin), Vec::<String>::new());
    }
}

#[tokio::test]
async fn the_forms_work_and_their_refusals_are_pages_with_a_way_back() {
    let _g = SERIAL.lock().await;
    let w = world(
        "page-post",
        ReviewConfig {
            comments_per_hour: 2,
            decisions_per_hour: 2,
        },
    );
    w.propose("r-open", "p1", "olivia", &[1, 2], 100);
    w.propose("r-secret", "s1", "olivia", &[3], 101);
    let bob = w.session("bob");
    let dave = w.session("dave");
    let alice = w.session("alice");
    let post_comment = "/alice/open/proposals/p1/comments";
    // CSRF: a wrong token is refused; a missing form is the incomplete-form page.
    let r = form(&w, post_comment, Some(&bob), "csrf=bogus&body=hi").await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = form(&w, post_comment, Some(&bob), "").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    // Not signed in: refused with the sentence and a way back.
    let r = form(
        &w,
        post_comment,
        Some("planeter_csrf=tok123"),
        "csrf=tok123&body=hi",
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(
        r.text()
            .contains("you need to be signed in to comment on it"),
        "{}",
        r.text()
    );
    assert!(
        r.text().contains("/alice/open/proposals/p1"),
        "a way back: {}",
        r.text()
    );
    // A good comment lands back on the page, at the comment.
    let r = form(
        &w,
        post_comment,
        Some(&bob),
        "csrf=tok123&body=Hello+%2Aworld%2A&path=src%2Flib.rs&line_from=3&line_to=5",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    assert!(
        r.h("location")
            .starts_with("/alice/open/proposals/p1#comment-m-"),
        "{}",
        r.h("location")
    );
    let p = page(&w, "GET", "/alice/open/proposals/p1", Some(&bob), "en")
        .await
        .text();
    assert!(
        details(&p).contains("<em>world</em>") && details(&p).contains("On src/lib.rs, lines 3–5"),
        "{}",
        details(&p)
    );
    // The patch is named by the first characters of its id.
    // (The ids of this test differ only in their last character: 8 characters name both patches, all 64 name one.)
    let ambiguous = &pid(1)[..8];
    let r = form(
        &w,
        post_comment,
        Some(&bob),
        &format!("csrf=tok123&body=About+it&patch={ambiguous}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    let prefix = pid(1);
    let r = form(
        &w,
        post_comment,
        Some(&bob),
        &format!("csrf=tok123&body=About+it&patch={prefix}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    let r = form(
        &w,
        post_comment,
        Some(&bob),
        "csrf=tok123&body=About+it&patch=deadbeef",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.text().contains("Your comment was not added")
            && r.text()
                .contains("No patch of this proposal starts with those characters"),
        "{}",
        r.text()
    );
    // The limit is a 429 page.
    let r = form(&w, post_comment, Some(&bob), "csrf=tok123&body=one+more").await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.text());
    assert!(r.text().contains("many times in the last hour"));
    // Rule refusals: an empty body, bad lines.
    let r = form(&w, post_comment, Some(&dave), "csrf=tok123&body=+").await;
    assert!(
        r.text().contains("A comment needs some text."),
        "{}",
        r.text()
    );
    let r = form(
        &w,
        post_comment,
        Some(&dave),
        "csrf=tok123&body=x&line_from=3&line_to=2&path=a",
    )
    .await;
    assert!(
        r.text()
            .contains("The first line must not come after the last."),
        "{}",
        r.text()
    );
    let r = form(
        &w,
        post_comment,
        Some(&dave),
        "csrf=tok123&body=x&line_from=abc&path=a",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);

    // Decisions.
    let dec = "/alice/open/proposals/p1/decisions";
    let r = form(
        &w,
        dec,
        Some(&bob),
        "csrf=tok123&decision=approve&revision=1",
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::FORBIDDEN,
        "bob does not review: {}",
        r.text()
    );
    assert!(
        r.text()
            .contains("only people who review here can record a decision")
    );
    let r = form(
        &w,
        dec,
        Some(&dave),
        "csrf=tok123&decision=maybe&revision=1",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let r = form(
        &w,
        dec,
        Some(&dave),
        "csrf=tok123&decision=approve&revision=7",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.text().contains("Your decision was not recorded")
            && r.text().contains("the newest is revision 1"),
        "{}",
        r.text()
    );
    let r = form(
        &w,
        dec,
        Some(&dave),
        "csrf=tok123&decision=approve&revision=1&note=Good",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    assert_eq!(r.h("location"), "/alice/open/proposals/p1");
    let p = page(&w, "GET", "/alice/open/proposals/p1", None, "en")
        .await
        .text();
    assert!(
        default_view(&p).contains("Approved by 1 person"),
        "{}",
        default_view(&p)
    );
    // The limit applies to forms too (two decisions an hour).
    form(
        &w,
        dec,
        Some(&dave),
        "csrf=tok123&decision=request-changes&revision=1",
    )
    .await;
    let r = form(
        &w,
        dec,
        Some(&dave),
        "csrf=tok123&decision=approve&revision=1",
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.text());
    // Self-approval through the form (alice opened nothing here, but olivia has no session; use a proposal of dave's).
    w.propose("r-open", "p3", "dave", &[9], 102);
    let r = form(
        &w,
        "/alice/open/proposals/p3/decisions",
        Some(&dave),
        "csrf=tok123&decision=approve&revision=1",
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.text()
            .contains("You cannot approve work that you sent yourself."),
        "{}",
        r.text()
    );

    // Hiding and showing.
    let cid = w.reviews.comment_threads("p1", None, 1).unwrap()[0]
        .comment_id
        .clone();
    let hide = format!("/alice/open/proposals/p1/comments/{cid}/hide");
    let r = form(&w, &hide, Some(&dave), "csrf=tok123&reason=Rude").await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    assert!(
        r.text()
            .contains("only people who administer it can hide or show a comment")
    );
    let r = form(&w, &hide, Some(&alice), "csrf=tok123&reason=+").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.text()
            .contains("Hiding needs a reason of up to 500 characters.")
    );
    let r = form(&w, &hide, Some(&alice), "csrf=tok123&reason=Rude").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    let r = form(&w, &hide, Some(&alice), "csrf=tok123&reason=Again").await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.text().contains("already in the state you asked for"));
    let r = form(
        &w,
        &format!("/alice/open/proposals/p1/comments/{cid}/unhide"),
        Some(&alice),
        "csrf=tok123",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    assert!(
        w.reviews
            .get_comment(&cid)
            .unwrap()
            .unwrap()
            .hidden
            .is_none()
    );

    // A stranger of a private repository: the same not-found page whatever is asked, for every form.
    let carol = w.session("carol");
    for uri in [
        "/alice/secret/proposals/s1/comments".to_owned(),
        "/alice/secret/proposals/s1/decisions".to_owned(),
        format!("/alice/secret/proposals/s1/comments/{cid}/hide"),
        "/alice/nothing/proposals/s1/comments".to_owned(),
    ] {
        let r = form(
            &w,
            &uri,
            Some(&carol),
            "csrf=tok123&body=x&decision=approve&reason=x",
        )
        .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn the_review_parts_of_the_pages_pass_the_checker_in_three_languages() {
    let _g = SERIAL.lock().await;
    let w = world("page-langs", ReviewConfig::default());
    w.propose("r-open", "p1", "olivia", &[1, 2], 100);
    w.propose("r-open", "p2", "olivia", &[3], 101);
    decide(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"decision": "approve", "note": "Fine."}),
    )
    .await;
    w.revision("r-open", None, "p1", "olivia", 2, &[4], 102);
    decide(
        &w,
        "open",
        "p1",
        Some("tok-gina"),
        serde_json::json!({"decision": "request-changes", "note": "Not yet."}),
    )
    .await;
    let top = comment(&w, "open", "p1", Some("tok-bob"), serde_json::json!({"body": "A question.", "patch_id": pid(1), "path": "src/a.rs", "line_from": 4})).await.json();
    comment(
        &w,
        "open",
        "p1",
        Some("tok-dave"),
        serde_json::json!({"body": "An answer.", "parent": top["id"]}),
    )
    .await;
    let hidden = comment(
        &w,
        "open",
        "p1",
        Some("tok-carol"),
        serde_json::json!({"body": "Hide me."}),
    )
    .await
    .json();
    api_post(
        &w,
        &format!(
            "{}/comments/{}/hide",
            base("open", "p1"),
            hidden["id"].as_str().unwrap()
        ),
        Some("tok-alice"),
        r#"{"reason":"Off topic."}"#,
    )
    .await;
    register_qaa();
    planeter_web::catalog::enable_preview_languages("ja").unwrap();
    let dump = std::env::var("REVIEWS_HTML_DUMP").ok().map(PathBuf::from);
    if let Some(d) = &dump {
        std::fs::create_dir_all(d).unwrap();
    }
    for lang in ["en", "ja", "qaa"] {
        for (who, cookie) in [
            ("anon", None),
            ("bob", Some(w.session("bob"))),
            ("dave", Some(w.session("dave"))),
            ("alice", Some(w.session("alice"))),
        ] {
            for (name, uri) in [
                ("detail", "/alice/open/proposals/p1"),
                ("list", "/alice/open/proposals"),
            ] {
                let r = page(&w, "GET", uri, cookie.as_deref(), lang).await;
                assert_eq!(r.status, StatusCode::OK, "{lang} {who} {uri}");
                let html = r.text();
                assert_eq!(unfilled(&html), None, "{lang} {who} {uri}");
                if lang != "qaa" {
                    assert_eq!(
                        violations(&html),
                        Vec::<String>::new(),
                        "{lang} {who} {uri}:\n{html}"
                    );
                } else {
                    assert!(html.contains("dir=\"rtl\""), "{lang} {who}");
                }
                if let Some(d) = &dump {
                    std::fs::write(d.join(format!("{name}-{who}-{lang}.html")), &html).unwrap();
                }
            }
        }
    }
    let ja = page(
        &w,
        "GET",
        "/alice/open/proposals/p1",
        Some(&w.session("dave")),
        "ja",
    )
    .await
    .text();
    assert!(
        ja.contains("リビジョン 2 への判断")
            && ja.contains("コメントを追加")
            && ja.contains("判断を記録する"),
        "{ja}"
    );
}

/// The test language: right to left, every message of the English catalog wrapped in `‹ ›` (registered once).
fn register_qaa() {
    static ONCE: std::sync::Once = std::sync::Once::new();
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
}
