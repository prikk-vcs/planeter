//! IS-39: a repository whose verification **fails** is shown as unverified, with the failed conditions —
//! never as "temporarily unavailable" and never as verified. Real prikk, three real corruptions (the
//! accessibility baseline's), through the router with a read budget set (so every call runs under a memory
//! ceiling, as in production). Self-skips without `prikk`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
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
use planeter_web::budget::MIB;
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("verify-failure-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A sealed repository with two small files.
fn sealed(name: &str) -> PathBuf {
    let base = scratch(name);
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
            .unwrap();
        assert!(ok.success(), "prikk {args:?}");
    };
    prikk(&["setup"]);
    std::fs::write(repo.join("a.txt"), b"hello\n").unwrap();
    std::fs::write(repo.join("README.md"), b"# hi\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

/// Damage seven bytes of one container (the baseline's three corruptions).
fn corrupt(repo: &Path, rel: &str) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(repo.join(rel))
        .unwrap();
    f.seek(SeekFrom::Start(40)).unwrap();
    f.write_all(b"CORRUPT").unwrap();
}

fn app(repos: Vec<(&'static str, PathBuf)>) -> Router {
    let root = scratch("hosting-root");
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (i, (name, path)) in repos.into_iter().enumerate() {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(format!("r{i}")),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility: Visibility::Public,
                created_at: 0,
                prikk_format_version: None,
                path,
            })
            .unwrap();
    }
    let hosting = HostingService::new(root, store as Arc<dyn RepositoryStore>)
        .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        Arc::new(hosting),
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    router(AppState {
        read,
        // A budget is set, so every call runs under `prlimit --data=…` exactly as in production.
        gate: Arc::new(PrikkGate::default().with_budget(2048 * MIB)),
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

async fn get(r: &Router, uri: &str) -> (StatusCode, String) {
    let resp = r
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::HOST, "forge.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_verification_is_unverified_with_its_conditions_never_an_outage_and_never_verified()
 {
    if !have("prikk") || !have("prlimit") {
        eprintln!("skipping: needs prikk and prlimit on PATH");
        return;
    }
    let good = sealed("good");
    let mut repos = vec![("good", good)];
    // (name, damaged file, a failed-condition id the page must show)
    let damage = [
        ("policy", ".prikk/trust/policy-a.container", "stage-failure"),
        (
            "patch",
            ".prikk/containers/patch/a.container",
            "item-failure",
        ),
        (
            "block",
            ".prikk/containers/block/a.container",
            "item-failure",
        ),
        // 3 failed stages and 2 blocked behind `refs` (IS-43): a verdict, not an outage
        (
            "pointer-index",
            ".prikk/refs/containers/pointer-index-a.container",
            "stage-failure",
        ),
    ];
    for (name, rel, _) in damage {
        let repo = sealed(name);
        corrupt(&repo, rel);
        repos.push((name, repo));
    }
    let r = app(repos);

    // The control: an intact repository is verified, on every surface.
    let (s, page) = get(&r, "/alice/good/verify").await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert!(page.contains("badge ok\">Verified<"), "{page}");
    let (s, body) = get(&r, "/api/v1/repos/alice/good/verify").await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("\"ok\":true"), "{body}");

    for (name, _, condition) in damage {
        // The verify page: 200, unverified, the conditions listed, no outage wording.
        let (s, page) = get(&r, &format!("/alice/{name}/verify")).await;
        assert_eq!(s, StatusCode::OK, "{name}: {page}");
        assert!(
            page.contains("badge bad\">Verification failed<"),
            "{name}: {page}"
        );
        assert!(
            page.contains(condition),
            "{name}: {condition} missing: {page}"
        );
        assert!(!page.contains("Temporarily unavailable"), "{name}: {page}");
        assert!(!page.contains("badge ok"), "{name}: {page}");

        // The repository and history pages. For the trust-policy damage the listing and the log still read, so
        // the badge is the only thing that was wrong: it must say unverified, not "could not be checked now". For
        // damaged patch or block objects `tree` and `log` themselves fail (the pages are a real outage, which
        // is the honest answer for pages whose content cannot be read): the one thing they must never do is
        // say the repository is verified.
        for page_uri in [format!("/alice/{name}"), format!("/alice/{name}/history")] {
            let (s, page) = get(&r, &page_uri).await;
            assert!(!page.contains("badge ok"), "{page_uri}: {page}");
            if name == "policy" {
                assert_eq!(s, StatusCode::OK, "{page_uri}: {page}");
                assert!(
                    page.contains("badge bad\">Verification failed<"),
                    "{page_uri}: {page}"
                );
                assert!(
                    !page.contains("Could not be checked now"),
                    "{page_uri}: {page}"
                );
            }
        }

        // The API: the resource is readable and the verdict is its content: 200 with ok:false.
        let (s, body) = get(&r, &format!("/api/v1/repos/alice/{name}/verify")).await;
        assert_eq!(s, StatusCode::OK, "{name}: {body}");
        assert!(body.contains("\"ok\":false"), "{name}: {body}");
        assert!(body.contains(condition), "{name}: {body}");
    }
}
