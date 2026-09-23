//! Browse-UI integration tests (WEB-01/05/06): the router driven via `ServiceExt::oneshot`. Pages that
//! never open a repository (home, login, denied-private) run without prikk; the authorized browse runs
//! against a real repo and self-skips without prikk.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, InsecureStubHasher, PasswordHasher as _,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepositoryStore,
    UserId, Visibility,
};
use planeter_web::{AppState, ContentOrigin, router};

fn prikk_available() -> bool {
    std::process::Command::new("prikk")
        .arg("--version")
        .output()
        .is_ok()
}

/// A private repo owned by alice (password `pw`); `real` = create it through prikk init.
fn app(subdir: &str, real: bool) -> AppState {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(subdir);
    let _ = std::fs::remove_dir_all(&root);
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(
        HostingService::new(root.clone(), store.clone() as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    if real {
        hosting
            .create_repo(Owner::User("alice".into()), "app", Visibility::Private)
            .expect("create repo (needs prikk)");
    } else {
        store
            .create(planeter_store::RepositoryRecord {
                repo_id: planeter_store::RepoId::new("r1"),
                owner: Owner::User("alice".into()),
                name: "app".into(),
                visibility: Visibility::Private,
                created_at: 0,
                prikk_format_version: None,
                path: root.join("never-opened"),
            })
            .expect("seed record");
    }
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
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
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Arc::new(Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    ));
    AppState {
        read,
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
    }
}

async fn send(r: &axum::Router, req: Request<Body>) -> Response {
    r.clone().oneshot(req).await.unwrap()
}

async fn body_string(resp: Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn cookie_value(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| {
            c.split(';')
                .next()
                .unwrap()
                .split_once('=')
                .unwrap()
                .1
                .to_owned()
        })
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn home_is_html_with_csp_and_no_inline_script() {
    let r = router(app("ui-home", false));
    let resp = send(&r, get("/")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(resp.headers().contains_key("content-security-policy"));
    let csrf = cookie_value(&resp, "planeter_csrf").expect("csrf cookie minted");
    assert!(!csrf.is_empty());
    let body = body_string(resp).await;
    assert!(body.contains("planeter"));
    assert!(!body.contains("<script"), "no inline script");
    assert!(body.contains(r#"href="/static/app.css""#));
}

#[tokio::test]
async fn stylesheet_is_served_from_the_app_origin() {
    let r = router(app("ui-css", false));
    let resp = send(&r, get("/static/app.css")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/css")
    );
}

#[tokio::test]
async fn anonymous_private_repo_page_is_404() {
    let r = router(app("ui-404", false));
    let resp = send(&r, get("/alice/app")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = send(&r, get("/alice/ghost")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn login_requires_csrf_and_a_correct_password_then_sets_a_session_cookie() {
    let r = router(app("ui-login", false));
    // GET /login mints the CSRF cookie.
    let page = send(&r, get("/login")).await;
    assert_eq!(page.status(), StatusCode::OK);
    let csrf = cookie_value(&page, "planeter_csrf").unwrap();

    let post = |form: String, csrf_cookie: Option<&str>| {
        let mut b = Request::builder()
            .method("POST")
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = csrf_cookie {
            b = b.header(header::COOKIE, format!("planeter_csrf={c}"));
        }
        b.body(Body::from(form)).unwrap()
    };

    // Wrong CSRF → 403.
    let resp = send(
        &r,
        post("csrf=bogus&username=alice&password=pw".into(), Some(&csrf)),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    // Right CSRF, wrong password → 401, no session cookie.
    let resp = send(
        &r,
        post(
            format!("csrf={csrf}&username=alice&password=nope"),
            Some(&csrf),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(cookie_value(&resp, "planeter_session").is_none());
    // Right CSRF + password → 303 with an HttpOnly/Strict/Secure session cookie.
    let resp = send(
        &r,
        post(
            format!("csrf={csrf}&username=alice&password=pw"),
            Some(&csrf),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let raw = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find(|v| v.to_str().unwrap().starts_with("planeter_session="))
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(raw.contains("HttpOnly") && raw.contains("SameSite=Strict") && raw.contains("Secure"));
    let session = cookie_value(&resp, "planeter_session").unwrap();

    // Logout with the session + CSRF clears it.
    let resp = send(
        &r,
        Request::builder()
            .method("POST")
            .uri("/logout")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(
                header::COOKIE,
                format!("planeter_csrf={csrf}; planeter_session={session}"),
            )
            .body(Body::from(format!("csrf={csrf}")))
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert!(
        resp.headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap().starts_with("planeter_session=;"))
    );
}

#[tokio::test]
async fn signed_in_owner_browses_her_private_repo() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let state = app("ui-browse", true);
    let session = state
        .sessions
        .create(UserId::new("alice"), 1_000_000, 2_000_000)
        .unwrap();
    let r = router(state);
    let with_session = |uri: &str| {
        Request::builder()
            .uri(uri)
            .header(header::COOKIE, format!("planeter_session={}", session.id.0))
            .body(Body::empty())
            .unwrap()
    };
    let resp = send(&r, with_session("/alice/app")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    assert!(body.contains("alice/app"));
    assert!(
        body.contains("prikk-verified"),
        "fresh repo verifies: {body}"
    );
    assert!(body.contains("sign out"));
    for uri in ["/alice/app/history", "/alice/app/refs", "/alice/app/verify"] {
        assert_eq!(
            send(&r, with_session(uri)).await.status(),
            StatusCode::OK,
            "{uri}"
        );
    }
    // Still 404 without the session.
    assert_eq!(
        send(&r, get("/alice/app")).await.status(),
        StatusCode::NOT_FOUND
    );
}
