//! RFC 008 handoff 2, T1/T2/T4: the delivery routes and the repository page, over the real router, with **real prikk** (the tests that need
//! one say why they skip). The world is the engine's own (`planeter-transport/tests/support`): real archives of the measurement, a hosted
//! repository made as creation makes one, proposals made through the propose service, SQLite stores. The sandbox is off here (the real
//! server's tests use the real one).

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;
#[path = "../../planeter-transport/tests/support/delivery_world.rs"]
mod world;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{NullAuditSink, ReadService, RepoScope, Scope, ScopeAccess};
use planeter_store::{
    ChangeStore as _, DeliveryStore as _, InMemoryMembershipStore, MembershipStore, Owner,
    ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore, Role, Subject, UserId, Visibility,
};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};
use world::*;

macro_rules! need_prikk {
    () => {
        if !prikk_ok() {
            eprintln!("skipping: prikk is not on PATH");
            return;
        }
    };
}

const MAGIC: &[u8] = b"PLANETER-IMPORT-1\n";

fn frame(archive: &[u8]) -> Vec<u8> {
    let meta = b"{}";
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    out.extend_from_slice(meta);
    out.extend_from_slice(archive);
    out
}

struct Site {
    router: Router,
    svc: Services,
    state: AppState,
}

fn site(name: &str, hook: Option<Hook>) -> Site {
    site_with(name, hook, |_| {})
}

fn site_with(
    name: &str,
    hook: Option<Hook>,
    tweak: impl FnOnce(&mut planeter_transport::DeliveryConfig),
) -> Site {
    site_full(name, hook, None, tweak)
}

fn site_full(
    name: &str,
    hook: Option<Hook>,
    binary: Option<PathBuf>,
    tweak: impl FnOnce(&mut planeter_transport::DeliveryConfig),
) -> Site {
    let fx = fixtures();
    let base = scratch(&format!("http-{name}"));
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let svc = open_full(
        &base,
        hook,
        planeter_prikk::Sandbox::Unconfined,
        binary,
        tweak,
    );
    // a private repository (for the hidden-repository answers): the record is enough
    svc.repos
        .create(RepositoryRecord {
            repo_id: RepoId::new("hd01"),
            owner: Owner::User("alice".into()),
            name: "secret".into(),
            visibility: Visibility::Private,
            created_at: 0,
            prikk_format_version: None,
            proposals: ProposalPolicy::Members,
            path: base.join("root/hd/hd01"),
        })
        .unwrap();
    let membership = Arc::new(InMemoryMembershipStore::new());
    for (user, role) in [
        ("alice", Role::Admin),
        ("dave", Role::Maintain),
        ("carol", Role::Write),
        ("bob", Role::Read),
    ] {
        membership
            .grant_repo_role(Subject::User(UserId::new(user)), RepoId::new(REPO), role)
            .unwrap();
    }
    membership
        .grant_repo_role(
            Subject::User(UserId::new("alice")),
            RepoId::new("hd01"),
            Role::Admin,
        )
        .unwrap();
    let read = Arc::new(ReadService::new(
        svc.hosting.clone(),
        membership.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user, access) in [
        ("tok-alice", "alice", ScopeAccess::Full),
        ("tok-alice-w", "alice", ScopeAccess::Write),
        ("tok-alice-ro", "alice", ScopeAccess::ReadOnly),
        ("tok-bob", "bob", ScopeAccess::Full),
        ("tok-carol", "carol", ScopeAccess::Full),
        ("tok-dave", "dave", ScopeAccess::Full),
        ("tok-eve", "eve", ScopeAccess::Full),
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
    let transport = Some(Arc::new(TransportRuntime {
        export: svc.export.clone(),
        propose: Some(svc.propose.clone()),
        recovery: Some(svc.recovery.clone()),
        proposals: None,
        reviews: None,
        changes: None,
        deliveries: Some(svc.delivery.clone()),
        ip_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
        principal_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
    }));
    let state = AppState {
        read,
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(planeter_auth::InMemorySessionStore::new()),
        now_unix: real_clock,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport,
        creation: None,
    };
    Site {
        router: router(state.clone()),
        svc,
        state,
    }
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let resp = r.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = axum::body::to_bytes(resp.into_body(), 1 << 30)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

fn req(method: &str, uri: &str, token: Option<&str>, body: Vec<u8>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/octet-stream")
        .header("content-length", body.len().to_string());
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body)).unwrap()
}

fn start_req(repo: &str, token: Option<&str>, archive: &Path) -> Request<Body> {
    let bytes = std::fs::read(archive).unwrap();
    req(
        "POST",
        &format!("/api/v1/repos/alice/{repo}/deliveries"),
        token,
        frame(&bytes),
    )
}

async fn start(s: &Site, token: &str, archive: &Path) -> Reply {
    send(&s.router, start_req("forge", Some(token), archive)).await
}

async fn view(s: &Site, token: &str, id: &str) -> Reply {
    send(
        &s.router,
        req(
            "GET",
            &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
            Some(token),
            vec![],
        ),
    )
    .await
}

async fn confirm(s: &Site, token: &str, id: &str) -> Reply {
    send(
        &s.router,
        req(
            "POST",
            &format!("/api/v1/repos/alice/forge/deliveries/{id}/confirm"),
            Some(token),
            vec![],
        ),
    )
    .await
}

async fn wait_for_state(s: &Site, token: &str, id: &str, state: &str) -> Value {
    for _ in 0..600 {
        let v = view(s, token, id).await.json();
        if v["state"] == state {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the delivery never reached {state}");
}

/// The code and the status of an error reply.
fn code(r: &Reply) -> (u16, String) {
    (
        r.status.as_u16(),
        r.json()["code"].as_str().unwrap_or("").to_owned(),
    )
}

// ------------------------------------------------------------------------------------------------ the whole road

#[tokio::test]
async fn the_whole_road_over_http_with_the_proposals_in_their_places() {
    need_prikk!();
    let fx = fixtures();
    let s = site("road", None);
    let sc = scenario(&s.svc, fx);
    let old_tip = s.svc.live_tip();

    let r = start(&s, "tok-alice", &sc.d).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(
        r.headers.get("cache-control").unwrap().to_str().unwrap(),
        "no-store"
    );
    let v = r.json();
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(v["state"], "reported");
    assert_eq!(v["report"]["verdict"], "acceptable");
    assert_eq!(v["report"]["old_tip"], old_tip);
    assert_eq!(v["report"]["new_tip"], sc.d_tip);
    assert_eq!(v["report"]["new_blocks"], 4);
    assert_eq!(v["report"]["proposals_land"][0]["id"], sc.p1);
    assert_eq!(v["report"]["proposals_land"][0]["title"], "P1");
    assert_eq!(v["report"]["proposals_stay"].as_array().unwrap().len(), 2);
    assert_eq!(v["report"]["trusted_keys"].as_array().unwrap().len(), 2);
    assert!(v["failure"].is_null());
    // the sentence the owner accepted, word for word (RFC 008 D-4)
    assert_eq!(
        v["message"],
        "Delivering replaces this repository's branch heads/main with yours. It is accepted only if yours continues from the current one; nothing is signed by this server."
    );
    assert_eq!(s.svc.live_tip(), old_tip, "a report moves nothing");

    // the same view, by GET
    let g = view(&s, "tok-alice", &id).await;
    assert_eq!(g.status, StatusCode::OK);
    assert_eq!(g.json()["id"], id.as_str());

    // confirm: the whole install fits in the wait
    let c = confirm(&s, "tok-alice", &id).await;
    assert_eq!(c.status, StatusCode::OK, "{}", c.text());
    let done = c.json();
    assert_eq!(done["state"], "done");
    assert_eq!(done["needs_recovery"], false);
    assert!(done["message"].as_str().unwrap().contains("now yours"));
    assert_eq!(s.svc.live_tip(), sc.d_tip);
    assert_eq!(s.svc.pending(), sc.stays);

    // the refs say which branch is the project's and who delivered last
    let refs = send(
        &s.router,
        req("GET", "/api/v1/repos/alice/forge/refs", None, vec![]),
    )
    .await
    .json();
    assert_eq!(refs["followed_ref"], "heads/main");
    assert_eq!(refs["last_delivery"]["delivered_by"], "alice");
    assert_eq!(refs["last_delivery"]["new_tip"], sc.d_tip);
    assert_eq!(
        refs["last_delivery"]["trusted_keys"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // a second confirmation of a finished delivery: not confirmable
    let again = confirm(&s, "tok-alice", &id).await;
    assert_eq!(code(&again), (409, "delivery_not_confirmable".into()));
    // give up a finished delivery: the same
    let del = send(
        &s.router,
        req(
            "DELETE",
            &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
            Some("tok-alice"),
            vec![],
        ),
    )
    .await;
    assert_eq!(code(&del), (409, "delivery_not_confirmable".into()));
}

#[tokio::test]
async fn a_checked_delivery_can_be_given_up_and_the_repository_is_free_again() {
    need_prikk!();
    let s = site("cancel", None);
    let fx = fixtures();
    let r = start(&s, "tok-alice", &fx.d1).await;
    let id = r.json()["id"].as_str().unwrap().to_owned();
    // in flight: another delivery is `delivery_busy`, by anyone who may deliver
    let busy = start(&s, "tok-dave", &fx.d2).await;
    assert_eq!(code(&busy), (409, "delivery_busy".into()));
    assert!(busy.headers.get("retry-after").is_some());
    let del = send(
        &s.router,
        req(
            "DELETE",
            &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
            Some("tok-alice"),
            vec![],
        ),
    )
    .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    assert!(del.body.is_empty());
    assert_eq!(
        view(&s, "tok-alice", &id).await.json()["state"],
        "abandoned"
    );
    let free = start(&s, "tok-dave", &fx.d2).await;
    assert_eq!(free.status, StatusCode::CREATED, "{}", free.text());
}

// ------------------------------------------------------------------------------------------------ every archive of the measurement

fn cases() -> Vec<(&'static str, PathBuf)> {
    let fx = fixtures();
    let mut v = vec![
        ("D1", fx.d1.clone()),
        ("D2", fx.d2.clone()),
        ("older (H0)", fx.h0.clone()),
        ("div", fx.div.clone()),
        ("m2", fx.m2_archive.clone()),
        ("m2div", fx.m2div.clone()),
        ("x", fx.x_archive.clone()),
        ("mix", fx.mix.clone()),
        ("xcarry", fx.xcarry.clone()),
        ("dev", fx.dev.clone()),
        ("unrelated", fx.unrelated.clone()),
        ("truncated", fx.truncated.clone()),
    ];
    for (n, p) in &fx.flips {
        v.push((Box::leak(format!("flip {n}").into_boxed_str()), p.clone()));
    }
    v
}

/// `Ok` = acceptable; `Err(reason)` = refused with that closed word.
fn expected(label: &str, after_d1: bool) -> Result<(), &'static str> {
    match label {
        "D1" => {
            if after_d1 {
                Err("nothing_to_deliver")
            } else {
                Ok(())
            }
        }
        "D2" | "m2" | "xcarry" | "dev" => Ok(()),
        "older (H0)" => Err(if after_d1 {
            "older_or_diverged"
        } else {
            "nothing_to_deliver"
        }),
        "div" | "m2div" => {
            if after_d1 {
                Err("older_or_diverged")
            } else {
                Ok(())
            }
        }
        "x" | "mix" => Err("unadopted_key"),
        "unrelated" => Err("unrelated"),
        _ => Err("damaged_archive"),
    }
}

async fn run_table(s: &Site, after_d1: bool) {
    for (label, archive) in cases() {
        let before = tree_hash(&s.svc.record.path);
        let r = start(s, "tok-alice", &archive).await;
        assert_eq!(r.status, StatusCode::CREATED, "{label}: {}", r.text());
        let v = r.json();
        let id = v["id"].as_str().unwrap().to_owned();
        match expected(label, after_d1) {
            Ok(()) => {
                assert_eq!(v["report"]["verdict"], "acceptable", "{label}: {v}");
                assert_eq!(v["state"], "reported");
                let del = send(
                    &s.router,
                    req(
                        "DELETE",
                        &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
                        Some("tok-alice"),
                        vec![],
                    ),
                )
                .await;
                assert_eq!(del.status, StatusCode::NO_CONTENT, "{label}");
            }
            Err(reason) => {
                assert_eq!(v["report"]["verdict"], "refused", "{label}: {v}");
                assert_eq!(v["report"]["reason"], reason, "{label}");
                assert_eq!(v["state"], "failed", "{label}");
                assert_eq!(v["failure"]["reason"], reason, "{label}");
                // a refusal says what is wrong and what to do, in words; and confirming it is refused
                assert!(!v["message"].as_str().unwrap().is_empty());
                assert!(!v["hint"].as_str().unwrap().is_empty());
                let c = confirm(s, "tok-alice", &id).await;
                assert_eq!(
                    code(&c),
                    (409, "delivery_not_confirmable".into()),
                    "{label}"
                );
            }
        }
        assert_eq!(
            tree_hash(&s.svc.record.path),
            before,
            "the live .prikk tree is byte-identical: {label}"
        );
    }
}

#[tokio::test]
async fn every_archive_of_the_measurement_over_http_against_the_forge_as_created() {
    need_prikk!();
    let s = site("table-s0", None);
    run_table(&s, false).await;
}

#[tokio::test]
async fn every_archive_of_the_measurement_over_http_after_a_delivery() {
    need_prikk!();
    let fx = fixtures();
    let s = site("table-s1", None);
    let id = start(&s, "tok-alice", &fx.d1).await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(confirm(&s, "tok-alice", &id).await.json()["state"], "done");
    run_table(&s, true).await;
}

// ------------------------------------------------------------------------------------------------ who may

#[tokio::test]
async fn the_authorization_table_over_http_and_the_one_404() {
    need_prikk!();
    let fx = fixtures();
    let s = site("authz", None);
    let post = |repo: &str, tok: Option<&str>| start_req(repo, tok, &fx.d1);
    // a maintainer and an administrator may; everyone else answers as the table says
    let ok = send(&s.router, post("forge", Some("tok-dave"))).await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text());
    let id = ok.json()["id"].as_str().unwrap().to_owned();
    let denied = [
        ("a reader", Some("tok-bob")),
        ("a contributor", Some("tok-carol")),
        ("a stranger on a public repository", Some("tok-eve")),
        ("the anonymous", None),
        ("a read-only token", Some("tok-alice-ro")),
        (
            "a write token, capped below merge-seal",
            Some("tok-alice-w"),
        ),
    ];
    for (who, tok) in denied {
        let r = send(&s.router, post("forge", tok)).await;
        assert_eq!(code(&r), (403, "forbidden".into()), "{who}");
        // before the body is read: the same for a request that has none
        let g = send(
            &s.router,
            req(
                "GET",
                &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
                tok,
                vec![],
            ),
        )
        .await;
        assert_eq!(code(&g), (403, "forbidden".into()), "{who}");
    }
    // a hidden repository, an absent one and an unknown delivery: one 404, byte for byte
    let hidden = send(&s.router, post("secret", Some("tok-eve"))).await;
    let absent = send(&s.router, post("nowhere", Some("tok-eve"))).await;
    assert_eq!(code(&hidden), (404, "not_found".into()));
    assert_eq!(hidden.body, absent.body);
    let anon_hidden = send(&s.router, post("secret", None)).await;
    assert_eq!(anon_hidden.body, absent.body);
    let unknown = view(&s, "tok-dave", "dl-000000000000000000000000").await;
    assert_eq!(code(&unknown), (404, "not_found".into()));
    assert_eq!(
        unknown.body, hidden.body,
        "an unknown delivery and a hidden repository"
    );
    // another maintainer's delivery is the same 404
    let theirs = view(&s, "tok-alice", &id).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    assert_eq!(theirs.body, unknown.body);
    let theirs_confirm = confirm(&s, "tok-alice", &id).await;
    assert_eq!(theirs_confirm.body, unknown.body);
    // a session cookie is not a credential here
    let mut cookie = start_req("forge", None, &fx.d1);
    cookie
        .headers_mut()
        .insert("cookie", "planeter_session=whatever".parse().unwrap());
    assert_eq!(
        code(&send(&s.router, cookie).await),
        (403, "forbidden".into())
    );
}

// ------------------------------------------------------------------------------------------------ the edge

#[tokio::test]
async fn the_edge_answers_before_a_byte_of_the_archive_is_read() {
    need_prikk!();
    let fx = fixtures();
    let s = site("edge", None);
    let bytes = std::fs::read(&fx.d1).unwrap();
    // no length
    let mut no_length = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/forge/deliveries")
        .header("authorization", "Bearer tok-alice")
        .body(Body::from(frame(&bytes)))
        .unwrap();
    no_length.headers_mut().remove("content-length");
    assert_eq!(code(&send(&s.router, no_length).await).1, "length_required");
    // a wrong content type
    let mut ct = start_req("forge", Some("tok-alice"), &fx.d1);
    ct.headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    assert_eq!(
        code(&send(&s.router, ct).await),
        (400, "invalid_request".into())
    );
    // a body that is not framed
    let bad = req(
        "POST",
        "/api/v1/repos/alice/forge/deliveries",
        Some("tok-alice"),
        vec![7u8; 100],
    );
    assert_eq!(
        code(&send(&s.router, bad).await),
        (400, "invalid_request".into())
    );
    // metadata with a field
    let mut with_field = MAGIC.to_vec();
    let meta = br#"{"x":1}"#;
    with_field.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    with_field.extend_from_slice(meta);
    with_field.extend_from_slice(&bytes);
    let r = send(
        &s.router,
        req(
            "POST",
            "/api/v1/repos/alice/forge/deliveries",
            Some("tok-alice"),
            with_field,
        ),
    )
    .await;
    assert_eq!(code(&r), (400, "invalid_request".into()));
    // over the bound: refused from the headers, nothing is begun
    let limit = s.svc.delivery.config().max_archive_bytes;
    let huge = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/forge/deliveries")
        .header("authorization", "Bearer tok-alice")
        .header("content-type", "application/octet-stream")
        .header("content-length", (limit + (1 << 20)).to_string())
        .body(Body::empty())
        .unwrap();
    let r = send(&s.router, huge).await;
    assert_eq!(code(&r), (413, "payload_too_large".into()));
    assert_eq!(r.json()["limit"], limit);
    // nothing was begun by any of them
    assert!(s.svc.deliveries.non_final().unwrap().is_empty());
    // a repository that waits for recovery refuses before the body
    s.svc
        .changes
        .set_needs_recovery(
            &s.svc.record.repo_id,
            "a write did not finish (accept_failed)",
            1,
        )
        .unwrap();
    let r = send(&s.router, start_req("forge", Some("tok-alice"), &fx.d1)).await;
    assert_eq!(code(&r), (503, "needs_recovery".into()));
}

// ------------------------------------------------------------------------------------------------ the long road: 202 and polling

#[tokio::test]
async fn a_check_that_takes_longer_than_the_wait_answers_202_and_the_delivery_is_polled() {
    need_prikk!();
    let fx = fixtures();
    let hook: Hook = Arc::new(|name: &str| {
        if name == "stage_before_import" {
            std::thread::sleep(Duration::from_millis(2500));
        }
    });
    let s = site_with("slow", Some(hook), |c| {
        c.request_wait = Duration::from_millis(500)
    });
    let r = start(&s, "tok-alice", &fx.d1).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    let v = r.json();
    assert_eq!(v["state"], "staged");
    assert!(v["report"].is_null());
    let id = v["id"].as_str().unwrap().to_owned();
    let done = wait_for_state(&s, "tok-alice", &id, "reported").await;
    assert_eq!(done["report"]["verdict"], "acceptable");
}

// ------------------------------------------------------------------------------------------------ detached from the request

#[tokio::test]
async fn a_confirmation_whose_connection_is_dropped_is_carried_out() {
    need_prikk!();
    let fx = fixtures();
    // one permit in the gate, held: the confirmation waits in the queue while its connection goes away
    let s = {
        let mut s = site("detach", None);
        let gate = Arc::new(planeter_web::PrikkGate::from_vars(Some("1"), Some("30")).unwrap());
        s.state.gate = gate;
        s.router = router(s.state.clone());
        s
    };
    let id = start(&s, "tok-alice", &fx.d1).await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let hold = {
        let gate = s.state.gate.clone();
        tokio::spawn(async move {
            gate.run(|| std::thread::sleep(Duration::from_millis(1500)))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    // the request is dropped after 300 ms, while it waits for the gate
    let dropped = tokio::time::timeout(
        Duration::from_millis(300),
        s.router.clone().oneshot(req(
            "POST",
            &format!("/api/v1/repos/alice/forge/deliveries/{id}/confirm"),
            Some("tok-alice"),
            vec![],
        )),
    )
    .await;
    assert!(dropped.is_err(), "the request was still waiting");
    let _ = hold.await;
    let done = wait_for_state(&s, "tok-alice", &id, "done").await;
    assert_eq!(done["failure"], Value::Null);
    assert_eq!(s.svc.live_tip(), s.svc.live_tip());
}

// ------------------------------------------------------------------------------------------------ busy

#[tokio::test]
async fn a_confirmation_that_meets_an_accept_is_busy_and_stays_reported() {
    need_prikk!();
    let fx = fixtures();
    let s = site("busy", None);
    let id = start(&s, "tok-alice", &fx.d1).await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let lease = s
        .svc
        .leases
        .acquire(
            &s.svc.record.repo_id,
            planeter_core::LeaseClass::Write,
            "sync accept",
            Duration::from_secs(1),
        )
        .unwrap();
    let c = confirm(&s, "tok-alice", &id).await;
    assert_eq!(code(&c), (409, "delivery_busy".into()), "{}", c.text());
    assert!(c.headers.get("retry-after").is_some());
    assert_eq!(view(&s, "tok-alice", &id).await.json()["state"], "reported");
    drop(lease);
    assert_eq!(confirm(&s, "tok-alice", &id).await.json()["state"], "done");
}

// ------------------------------------------------------------------------------------------------ the contract

#[tokio::test]
async fn the_views_are_exactly_what_the_document_declares() {
    need_prikk!();
    let fx = fixtures();
    let s = site("contract", None);
    let doc = send(&s.router, req("GET", "/api/v1/openapi.json", None, vec![]))
        .await
        .json();
    let schemas = &doc["components"]["schemas"];
    let keys = |schema: &str| -> Vec<String> {
        let mut k: Vec<String> = schemas[schema]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        k.sort();
        k
    };
    let got = |v: &Value| -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    for archive in [&fx.d1, &fx.unrelated] {
        let v = start(&s, "tok-alice", archive).await.json();
        assert_eq!(got(&v), keys("DeliveryView"));
        assert_eq!(got(&v["report"]), keys("DeliveryReport"));
        if let Some(f) = v["failure"].as_object() {
            assert_eq!(got(&Value::Object(f.clone())), keys("DeliveryFailure"));
        }
        let id = v["id"].as_str().unwrap().to_owned();
        let _ = send(
            &s.router,
            req(
                "DELETE",
                &format!("/api/v1/repos/alice/forge/deliveries/{id}"),
                Some("tok-alice"),
                vec![],
            ),
        )
        .await;
    }
    // the last delivery, once there is one
    let id = start(&s, "tok-alice", &fx.d1).await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    confirm(&s, "tok-alice", &id).await;
    let refs = send(
        &s.router,
        req("GET", "/api/v1/repos/alice/forge/refs", None, vec![]),
    )
    .await
    .json();
    assert_eq!(got(&refs["last_delivery"]), keys("LastDelivery"));
    assert_eq!(got(&refs), keys("RefsView"));
}

// ------------------------------------------------------------------------------------------------ what cannot be kept

#[tokio::test]
async fn a_proposal_whose_files_are_gone_is_named_before_the_confirmation() {
    need_prikk!();
    let fx = fixtures();
    let s = site("gone", None);
    let sc = scenario(&s.svc, fx);
    // the retained artifact of P2 is removed (the retention sweep does this to a finished proposal)
    let rev = &planeter_store::ChangeStore::revisions_of(&*s.svc.changes, &sc.p2).unwrap()[0];
    let digest = planeter_transport::artifacts::Digest::parse(
        rev.artifact_digest.strip_prefix("sha256:").unwrap(),
    )
    .unwrap();
    assert!(s.svc.artifacts.remove(&digest).unwrap());
    let v = start(&s, "tok-alice", &sc.d).await.json();
    assert_eq!(v["report"]["proposals_cannot_be_kept"][0]["id"], sc.p2);
    assert_eq!(v["report"]["proposals_cannot_be_kept"][0]["title"], "P2");
    assert_eq!(
        v["report"]["proposals_cannot_be_kept"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let id = v["id"].as_str().unwrap().to_owned();
    let done = confirm(&s, "tok-alice", &id).await.json();
    assert_eq!(done["state"], "done");
}

// ------------------------------------------------------------------------------------------------ the pages

async fn page(s: &Site, path: &str, language: &str) -> Reply {
    send(
        &s.router,
        Request::builder()
            .method("GET")
            .uri(path)
            .header("accept-language", language)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

fn save(s: &Site, name: &str, html: &str) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("delivery-pages");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), html).unwrap();
    let _ = s;
}

#[tokio::test]
async fn the_repository_pages_say_who_delivered_only_when_somebody_did() {
    need_prikk!();
    let fx = fixtures();
    let s = site("pages", None);
    let line_en = "Delivered by";
    let line_ja = "に引き渡しました";
    // never delivered: nothing new, in either language, on either page
    for (path, file) in [
        ("/alice/forge", "home"),
        ("/alice/forge/history", "history"),
    ] {
        for (lang, tag) in [("en", "en"), ("ja", "ja")] {
            let r = page(&s, path, lang).await;
            assert_eq!(r.status, StatusCode::OK, "{path}");
            let html = r.text();
            save(&s, &format!("{file}-without-delivery-{tag}.html"), &html);
            assert!(
                !html.contains(line_en) && !html.contains(line_ja),
                "{path} {lang}"
            );
            assert!(!html.contains("heads/main\u{0}"), "no stray text");
            assert!(!html.contains("class=\"delivered\""), "{path} {lang}");
        }
    }
    // delivered
    let id = start(&s, "tok-dave", &fx.d1).await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(confirm(&s, "tok-dave", &id).await.json()["state"], "done");
    let today = chrono_free_today();
    for (path, file) in [
        ("/alice/forge", "home"),
        ("/alice/forge/history", "history"),
    ] {
        let en = page(&s, path, "en").await.text();
        save(&s, &format!("{file}-with-delivery-en.html"), &en);
        assert!(
            en.contains(&format!(
                "Delivered by dave on {today}: signed by a key this repository trusts and checked by prikk."
            )),
            "{path}: {en}"
        );
        assert!(
            en.contains("The project&#39;s branch is heads/main.")
                || en.contains("The project's branch is heads/main.")
        );
        assert!(en.contains("<details><summary>Keys this repository trusts</summary>"));
        assert_eq!(en.matches("<li><code>ed25519-").count(), 2, "the two keys");
        let ja = page(&s, path, "ja").await.text();
        save(&s, &format!("{file}-with-delivery-ja.html"), &ja);
        assert!(
            ja.contains(&format!("dave が {today} に引き渡しました。")),
            "{path}"
        );
        assert!(ja.contains("このリポジトリが信頼する鍵"));
        for bad in ["received", "selected", "by this server's rule"] {
            assert!(!en.contains(bad), "{path}: {bad}");
        }
    }
}

/// Today's date as the pages write it (ISO 8601, UTC), from the clock the test world runs on.
fn chrono_free_today() -> String {
    let unix = real_clock();
    let days = i64::try_from(unix / 86_400).unwrap();
    // the same civil-from-days arithmetic the pages use (Howard Hinnant), so that the test reads the date independently of `timefmt`
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", if m <= 2 { y + 1 } else { y })
}

// ------------------------------------------------------------------------------------------------ the long confirmation

#[tokio::test]
async fn a_confirmation_that_takes_longer_than_the_wait_answers_202_and_is_polled_to_done() {
    need_prikk!();
    let fx = fixtures();
    let hook: Hook = Arc::new(|name: &str| {
        if name == "reaccept_first" {
            std::thread::sleep(Duration::from_millis(2500));
        }
    });
    let s = site_with("slow-confirm", Some(hook), |c| {
        c.request_wait = Duration::from_millis(500)
    });
    let sc = scenario(&s.svc, fx);
    let v = start(&s, "tok-alice", &sc.d).await.json();
    let id = v["id"].as_str().unwrap().to_owned();
    // (the check itself may outlast the short wait of this service: poll until it is reported)
    wait_for_state(&s, "tok-alice", &id, "reported").await;
    let r = confirm(&s, "tok-alice", &id).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    assert_eq!(r.json()["state"], "reaccepting");
    let done = wait_for_state(&s, "tok-alice", &id, "done").await;
    assert_eq!(done["needs_recovery"], false);
    assert_eq!(s.svc.pending(), sc.stays);
}

/// A `prikk` that is the real one except that from the `from`-th `sync accept` on it fails.
fn failing_accepts(base: &Path, from: u32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = base.join("prikk-failing.sh");
    let real =
        String::from_utf8(Command::new("which").arg("prikk").output().unwrap().stdout).unwrap();
    let count = base.join("accept-count");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"sync accept\"*)\n    n=$(cat \"{c}\" 2>/dev/null || echo 0); n=$((n+1)); echo $n > \"{c}\"\n    if [ \"$n\" -ge {from} ]; then echo 'error: injected failure' >&2; exit 1; fi ;;\nesac\nexec {real} \"$@\"\n",
            c = count.display(),
            real = real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

use std::process::Command;

#[tokio::test]
async fn an_install_that_stops_after_its_exchange_says_so_and_a_recovery_run_finishes_it() {
    need_prikk!();
    let fx = fixtures();
    // the tests' own base is not known before the site is made: the wrapper is written beside it
    let wrapper_dir = scratch("http-unfinished-wrapper");
    let s = site_full(
        "unfinished",
        None,
        Some(failing_accepts(&wrapper_dir, 5)),
        |_| {},
    );
    let sc = scenario(&s.svc, fx); // the first three accepts are the proposals'
    let id = start(&s, "tok-alice", &sc.d).await.json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = confirm(&s, "tok-alice", &id).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    let v = r.json();
    assert_eq!(v["state"], "reaccepting");
    assert_eq!(v["needs_recovery"], true);
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("The server stopped")
    );
    // the repository waits for recovery: a new delivery is refused cleanly, reads answer
    let again = start(&s, "tok-alice", &fx.d2).await;
    assert_eq!(code(&again).1, "needs_recovery");
    assert_eq!(
        send(
            &s.router,
            req("GET", "/api/v1/repos/alice/forge/refs", None, vec![])
        )
        .await
        .status,
        StatusCode::OK
    );
    // the administrator's door: a recovery run clears the flag and finishes the delivery (with a prikk that works again)
    std::fs::write(wrapper_dir.join("accept-count"), "0").unwrap();
    let mut fixed = std::fs::read_to_string(wrapper_dir.join("prikk-failing.sh")).unwrap();
    fixed = fixed.replace("-ge 5", "-ge 100000");
    std::fs::write(wrapper_dir.join("prikk-failing.sh"), fixed).unwrap();
    let run = send(
        &s.router,
        req(
            "POST",
            "/api/v1/repos/alice/forge/recovery",
            Some("tok-alice"),
            b"{}".to_vec(),
        ),
    )
    .await;
    assert_eq!(run.status, StatusCode::OK, "{}", run.text());
    let done = wait_for_state(&s, "tok-alice", &id, "done").await;
    assert_eq!(done["needs_recovery"], false);
    assert_eq!(s.svc.pending(), sc.stays);
}
