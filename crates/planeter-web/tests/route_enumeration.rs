//! IS-14 / ENF-4: every route the web surface serves is in one table with its authorization class, and the table
//! is held to its word.
//!
//! * the table is complete by construction (a route can only be registered through `routes::Routes`) and a source
//!   scan fails on any other registration;
//! * a `Public` entry carries a reason; the table lists exactly the routes the contract tests know about;
//! * the declared methods are the methods the route answers (and no others);
//! * **an `Authorized` route denies a stranger**: an anonymous caller asking for a *private* repository is answered
//!   `404` (existence-neutral), while the same request for a *public* repository whose store is unreadable is not
//!   (the control: proof that the probe can tell a handler that checked from one that did not).
//!
//! Each check is shown able to fail below (a planted unauthorized route, a planted undeclared method, a planted
//! stray registration); the review request lists the source mutations applied and restored.
//!
//! No `prikk` is needed: authorization happens before any repository is opened, and the "public but unreadable"
//! control fails at opening, which is the point.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::get;
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};
use planeter_web::routes::{Action, Class, Entry, Routes};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, registry};

const APP: &str = "forge.example";
const RAW: &str = "raw.planeter.example";

fn state() -> AppState {
    // One directory per call: the tests run in parallel and each builds its own state.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("route-enumeration-{n}"));
    let _ = std::fs::remove_dir_all(&base);
    // Two repositories whose directories exist but hold no prikk repository: reading either would fail at
    // opening (503). One is private (a stranger must never get that far), one public (the control).
    let (secret, open) = (base.join("secret"), base.join("open"));
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::create_dir_all(&open).unwrap();
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (i, (name, vis, path)) in [
        ("secret", Visibility::Private, secret),
        ("open", Visibility::Public, open),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(format!("r{i}")),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility: vis,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path,
            })
            .unwrap();
    }
    let hosting = Arc::new(
        HostingService::new(base.join("unused"), store as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    AppState {
        read: Arc::new(ReadService::new(
            hosting,
            membership,
            Arc::new(NullAuditSink),
        )),
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth: Arc::new(Authenticator::new(
            Arc::new(InMemoryAccountStore::new()),
            Arc::new(InMemoryCredentialStore::new()),
            hasher.clone(),
            hasher,
        )),
        content_origin: ContentOrigin::new(format!("https://{RAW}")),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::default()),
        oidc: None,
        transport: None,
    }
}

/// A route pattern made concrete for a repository: `{owner}`/`{name}` filled, wildcards given a path, queries
/// added where the route needs one.
fn concrete(pattern: &str, repo: &str) -> String {
    let p = pattern
        .replace("{owner}", "alice")
        .replace("{name}", repo)
        .replace("{target}", "abcdef")
        .replace("{id}", "p1")
        .replace("{seq}", "1")
        .replace("{*path}", "some/path.txt");
    if p.ends_with("/file") || p.ends_with("/raw") {
        format!("{p}?path=README.md")
    } else {
        p
    }
}

async fn status(router: &Router, method: &str, uri: &str, host: &str) -> StatusCode {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::from_bytes(method.as_bytes()).unwrap())
                .uri(uri)
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

fn host_for(path: &str) -> &'static str {
    if path.ends_with("/raw") { RAW } else { APP }
}

/// What the probe finds wrong with a table and the router built from it: one line per violation.
async fn violations(table: &[Entry], router: &Router) -> Vec<String> {
    let mut out = Vec::new();
    for e in table {
        let host = host_for(e.path);
        // 1. The declared methods are answered (not `405`), and a method that is not declared is refused (`405`).
        for m in e.methods {
            let uri = if matches!(e.class, Class::Authorized(_) | Class::AuthorizedAny(_)) {
                concrete(e.path, "secret")
            } else {
                concrete(e.path, "open")
            };
            if status(router, m, &uri, host).await == StatusCode::METHOD_NOT_ALLOWED {
                out.push(format!("{} declares {m} but answers 405", e.path));
            }
        }
        for undeclared in ["PUT", "DELETE", "PATCH"] {
            if !e.methods.contains(&undeclared) {
                let uri = concrete(e.path, "open");
                if status(router, undeclared, &uri, host).await != StatusCode::METHOD_NOT_ALLOWED {
                    out.push(format!(
                        "{} does not declare {undeclared} but does not answer 405",
                        e.path
                    ));
                }
            }
        }
        // 2. An authorized route denies a stranger: a private repository is `404`, and the control (public,
        //    unreadable store) is not, so the probe has power.
        let declared = match e.class {
            Class::Authorized(action) => Some(format!("{action:?}")),
            Class::AuthorizedAny(actions) => Some(format!("any of {actions:?}")),
            _ => None,
        };
        if let Some(action) = declared {
            for m in e.methods {
                let hidden = status(router, m, &concrete(e.path, "secret"), host).await;
                let control = status(router, m, &concrete(e.path, "open"), host).await;
                if hidden != StatusCode::NOT_FOUND {
                    out.push(format!(
                        "{} ({action:?}): an anonymous {m} for a PRIVATE repository answered {hidden}, not 404 — no authorize()",
                        e.path
                    ));
                }
                if control == StatusCode::NOT_FOUND {
                    out.push(format!(
                        "{}: the control (a public repository with an unreadable store) also answered 404, so the probe cannot tell a check from its absence",
                        e.path
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn the_table_lists_the_routes_the_contract_knows_and_every_public_entry_has_a_reason() {
    let reg = registry();
    let table = reg.table();
    // The independent list: the API's paths (also held by `tests/api_contract.rs`) and the UI's routes.
    let mut expected: Vec<&str> = planeter_web::api::paths::ALL.to_vec();
    expected.extend([
        "/",
        "/go",
        "/static/app.css",
        "/login",
        "/login/oidc",
        "/login/oidc/callback",
        "/logout",
        "/lang",
        "/{owner}/{name}",
        "/{owner}/{name}/tree/{*path}",
        "/{owner}/{name}/blob/{*path}",
        "/{owner}/{name}/history",
        "/{owner}/{name}/change/{target}",
        "/{owner}/{name}/refs",
        "/{owner}/{name}/verify",
        "/{owner}/{name}/proposals",
        "/{owner}/{name}/proposals/{id}",
        "/{owner}/{name}/proposals/{id}/close",
    ]);
    let mut have: Vec<&str> = table.iter().map(|e| e.path).collect();
    have.sort_unstable();
    expected.sort_unstable();
    assert_eq!(have, expected, "the table and the known routes differ");
    for e in table {
        if let Class::Public(reason) = e.class {
            assert!(
                reason.len() > 15,
                "{}: the reason is too thin: {reason:?}",
                e.path
            );
        }
        assert!(!e.methods.is_empty());
    }
    // Every route that can show repository data is authorized for `Read`.
    let authorized: Vec<&str> = table
        .iter()
        .filter(|e| matches!(e.class, Class::Authorized(Action::Read)))
        .map(|e| e.path)
        .collect();
    assert_eq!(
        authorized.len(),
        9 + 2 + 9,
        "9 API reads, the transport's advertise and fetch, and 9 repository pages"
    );
    assert!(authorized.iter().all(|p| p.contains("{owner}")));
}

#[tokio::test]
async fn every_authorized_route_denies_a_stranger_and_every_entry_keeps_its_declared_methods() {
    let router = planeter_web::router(state());
    let v = violations(registry().table(), &router).await;
    assert_eq!(v, Vec::<String>::new());
}

/// The probe can fail: a route declared `Authorized` whose handler serves data to anyone.
#[tokio::test]
async fn a_planted_route_without_an_authorization_check_is_found() {
    let table_and_router = Routes::new()
        .route(
            "/{owner}/{name}/leak",
            Class::Authorized(Action::Read),
            &["GET"],
            get(|| async { "secret data for anyone" }),
        )
        .route(
            "/{owner}/{name}/guarded",
            Class::Authorized(Action::Read),
            &["GET"],
            get(|| async { StatusCode::NOT_FOUND }),
        );
    let table = table_and_router.table().to_vec();
    let router = table_and_router.build(state());
    let v = violations(&table, &router).await;
    assert!(
        v.iter()
            .any(|l| l.contains("/leak") && l.contains("no authorize()")),
        "{v:?}"
    );
    // The guarded one answers 404 to the stranger — and also to the control, so the probe says it cannot tell.
    assert!(
        v.iter()
            .any(|l| l.contains("/guarded") && l.contains("cannot tell")),
        "{v:?}"
    );
}

/// The method check can fail: a route that declares GET but also answers POST.
#[tokio::test]
async fn a_planted_undeclared_method_is_found() {
    let reg = Routes::new().route(
        "/extra",
        Class::public("a planted route for the test: it serves nothing"),
        &["GET"],
        get(|| async { "x" }).post(|| async { "x" }),
    );
    let table = reg.table().to_vec();
    let router = reg.build(state());
    // PUT/DELETE/PATCH are 405 as declared; POST is not probed as "undeclared" (the probe tries the three
    // write methods), so plant PUT instead.
    let reg2 = Routes::new().route(
        "/extra",
        Class::public("a planted route for the test: it serves nothing"),
        &["GET"],
        get(|| async { "x" }).put(|| async { "x" }),
    );
    let table2 = reg2.table().to_vec();
    let router2 = reg2.build(state());
    assert_eq!(violations(&table, &router).await, Vec::<String>::new());
    let v = violations(&table2, &router2).await;
    assert!(
        v.iter().any(|l| l.contains("does not declare PUT")),
        "{v:?}"
    );
}

#[test]
#[should_panic(expected = "a public route needs a reason")]
fn a_public_entry_without_a_reason_cannot_be_registered() {
    let _ = Class::public("  ");
}

#[test]
#[should_panic(expected = "registered twice")]
fn a_route_registered_twice_is_refused() {
    let _ = Routes::new()
        .route(
            "/x",
            Class::public("a planted route for the test"),
            &["GET"],
            get(|| async { "x" }),
        )
        .route(
            "/x",
            Class::public("a planted route for the test"),
            &["GET"],
            get(|| async { "x" }),
        );
}

/// The registry's own call has four arguments, one of them the methods list (`&["GET"]`); axum's own `.route`
/// has two. A line that registers a route the second way is a registration the table cannot see.
fn stray_registrations(source: &str) -> Vec<String> {
    let mut stray = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let axum_style_construction = code.contains("Router::new")
            || code.contains(".nest(")
            || code.contains(".nest_service(")
            || code.contains(".route_service(");
        let route_without_class =
            code.contains(".route(") && !code.contains("&[\"") && !code.trim_end().ends_with('(');
        if axum_style_construction || route_without_class {
            stray.push(format!("line {}: {}", i + 1, line.trim()));
        }
    }
    stray
}

/// The mechanism's other half: the table is complete only if nothing registers a route another way.
#[test]
fn nothing_registers_a_route_outside_the_registry() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stray = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "rs")
            || path.file_name().is_some_and(|n| n == "routes.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for l in stray_registrations(&text) {
            stray.push(format!("{}: {l}", path.display()));
        }
    }
    assert_eq!(
        stray,
        Vec::<String>::new(),
        "a route is registered outside `routes::Routes`"
    );
}

/// The scan can fail: the shapes of a registration that bypasses the registry.
#[test]
fn the_source_scan_finds_a_planted_stray_registration() {
    for planted in [
        "    Router::new().route(\"/x\", get(h))",
        "    .route(\"/leak\", get(leak))",
        "    app.nest(\"/admin\", admin_router())",
        "    .route_service(\"/f\", ServeDir::new(\".\"))",
    ] {
        assert!(!stray_registrations(planted).is_empty(), "{planted}");
    }
    // …and does not flag the registry's own calls.
    for fine in [
        "        .route(\"/ok\", Class::public(\"a reason that is long enough\"), &[\"GET\"], get(h))",
        "        .route(",
        "        .route(paths::HISTORY, read, &[\"GET\"], get(history_handler))",
        "    // Router::new() in a comment",
    ] {
        assert_eq!(stray_registrations(fine), Vec::<String>::new(), "{fine}");
    }
}
