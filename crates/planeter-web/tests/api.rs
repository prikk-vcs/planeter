//! Integration tests for the read API (RFC 003 T3). The router is driven directly via
//! `ServiceExt::oneshot` (no live socket). The authorized-read case **self-skips** without `prikk`; the
//! security-header and 404-indistinguishability cases need no prikk (they never open a repo).

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{HostingService, NullAuditSink, ReadService, RepoScope, Scope, ScopeAccess};
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

/// An app hosting a private repo owned by `alice`, with token `tok-alice` authenticating as alice
/// (owner → Admin, so she may read her own repo).
fn app(subdir: &str) -> AppState {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(subdir);
    let _ = std::fs::remove_dir_all(&root);
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(
        HostingService::new(root, store).with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    hosting
        .create_repo(Owner::User("alice".into()), "app", Visibility::Private)
        .expect("create repo");
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));

    let accounts = Arc::new(InMemoryAccountStore::new());
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    creds
        .add_token(TokenRecord {
            hash: TokenHasher::hash(&*hasher, "tok-alice"),
            user: UserId::new("alice"),
            scope: Scope {
                access: ScopeAccess::ReadOnly,
                repos: RepoScope::All,
                expires_at: None,
            },
        })
        .unwrap();
    let auth = Arc::new(Authenticator::new(accounts, creds, hasher.clone(), hasher));

    AppState {
        read,
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
    }
}

async fn send(
    router: &axum::Router,
    uri: &str,
    bearer: Option<&str>,
    inm: Option<&str>,
) -> Response {
    let mut req = Request::builder().uri(uri);
    if let Some(t) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(m) = inm {
        req = req.header(header::IF_NONE_MATCH, m);
    }
    router
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn security_headers_present_on_every_response() {
    let r = router(app("api-headers"));
    let resp = send(&r, "/api/v1/openapi.json", None, None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("content-security-policy"));
    assert_eq!(
        resp.headers().get("x-content-type-options").unwrap(),
        "nosniff"
    );
    assert!(resp.headers().contains_key("x-frame-options"));
}

#[tokio::test]
async fn anonymous_private_repo_is_404_and_matches_missing() {
    let r = router(app("api-404"));
    // Anonymous read of a private repo → 404 (never opens prikk).
    let denied = send(&r, "/api/v1/repos/alice/app/history", None, None).await;
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
    // A genuinely missing repo → also 404, indistinguishable (T-7).
    let missing = send(&r, "/api/v1/repos/alice/ghost/history", None, None).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn authorized_history_returns_json_then_304() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let r = router(app("api-etag"));
    // alice's token authorizes reading her own private repo.
    let ok = send(
        &r,
        "/api/v1/repos/alice/app/history",
        Some("tok-alice"),
        None,
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(
        ok.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let etag = ok
        .headers()
        .get(header::ETAG)
        .expect("ETag present")
        .to_str()
        .unwrap()
        .to_owned();

    // A conditional GET with the same ETag → 304.
    let not_modified = send(
        &r,
        "/api/v1/repos/alice/app/history",
        Some("tok-alice"),
        Some(&etag),
    )
    .await;
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
}
