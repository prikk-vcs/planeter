//! IS-33 (API contract refinements, T2/T3): `GET …/file` over a **real sealed repository**. A missing
//! path is a `404 not_found`; a present one is `200` with a content object tagged by `kind`.
//!
//! Self-skips when `prikk` is not on `PATH`. Keys for sealing live in a per-test directory, never the
//! developer's own.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
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
use planeter_web::errors::ErrorCode;
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn prikk_available() -> bool {
    Command::new("prikk").arg("--version").output().is_ok()
}

/// A sealed repository with a text file, a nested text file and a binary file.
fn sealed_repo(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let keys = base.join("keys");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    let prikk = |args: &[&str]| {
        let ok = Command::new("prikk")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &keys)
            .env("XDG_CONFIG_HOME", &keys)
            .status()
            .expect("spawn prikk");
        assert!(ok.success(), "prikk {args:?} failed");
    };
    prikk(&["setup"]);
    std::fs::write(repo.join("README.md"), b"hello world\n").unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/main.rs"), b"fn main() {}\n").unwrap();
    std::fs::write(repo.join("data.bin"), [0x00u8, 0x01, 0x02, b'X', 0xff]).unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

fn app(name: &str) -> Router {
    let repo = sealed_repo(name);
    let store = Arc::new(InMemoryRepositoryStore::new());
    store
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "open".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: repo,
        })
        .unwrap();
    let hosting = Arc::new(
        HostingService::new(
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name),
            store as Arc<dyn RepositoryStore>,
        )
        .with_sandbox(planeter_prikk::Sandbox::Unconfined),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    router(AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth: Arc::new(Authenticator::new(
            Arc::new(InMemoryAccountStore::new()),
            Arc::new(InMemoryCredentialStore::new()),
            hasher.clone(),
            hasher,
        )),
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::default()),
        oidc: None,
    })
}

async fn get(r: &Router, uri: &str) -> (StatusCode, Vec<u8>) {
    let resp = r
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, body.to_vec())
}

#[tokio::test]
async fn a_present_file_is_200_with_a_kind_tagged_content_and_a_missing_one_is_404() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let r = app("file-contract");

    let (status, body) = get(&r, "/api/v1/repos/alice/open/file?path=README.md").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        v["content"],
        json!({"kind": "text", "text": "hello world\n"})
    );

    let (status, body) = get(&r, "/api/v1/repos/alice/open/file?path=src/main.rs").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"]["kind"], "text");

    let (status, body) = get(&r, "/api/v1/repos/alice/open/file?path=data.bin").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"]["kind"], "binary");
    assert_eq!(v["content"]["size"], 5);
    assert!(v["content"]["blob_id"].is_string());

    // A path that is not in the repository: 404 `not_found`, the one shared body, no "content".
    let (status, missing) = get(&r, "/api/v1/repos/alice/open/file?path=nope.txt").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing, ErrorCode::NotFound.body().into_bytes());
    // …byte-identical to an absent repository.
    let (status, absent) = get(&r, "/api/v1/repos/alice/no-such-repo/file?path=nope.txt").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing, absent);
}
