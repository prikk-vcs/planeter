//! IS-45: a missing **path** in a repository the reader may read is said to be missing; a missing, hidden or
//! unreadable **repository** is still said nothing about.
//!
//! * the API answers a missing path exactly as it answers a hidden or an absent repository (same status, headers and
//!   body), for the stranger and for the authorized reader alike (`file`, `raw`); `tree` keeps its contract (`200`,
//!   no entries);
//! * the UI says "This file or folder was not found in this repository…" **only** to a reader who may read it, and
//!   shows a stranger of a private repository the repository-level sentence, byte-identical to an absent one;
//! * a repository whose only content is queued work is pinned (T4): history empty, tree and refs `200` and empty,
//!   `file`/`raw` are a missing path and **`checkout --patch-plan` is never run**; a repository with sealed **and**
//!   queued commits shows the sealed content only.
//!
//! Needs `prikk` on `PATH` (self-skips otherwise). Keys for sealing live in per-test directories.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher, SessionStore,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, UserId, Visibility,
};
use planeter_web::catalog::{EN, Key, T};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

const APP: &str = "forge.example";
const RAW: &str = "raw.planeter.example";

fn have_prikk() -> bool {
    Command::new("prikk").arg("--version").output().is_ok()
}

fn scratch(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("missing-path-{name}"))
}

/// A repository on disk: `sealed` blocks (each one a commit and a seal), then `queued` commits left unsealed.
fn make_repo(name: &str, sealed: usize, queued: usize) -> PathBuf {
    let base = scratch(name);
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
    let mut n = 0;
    let commit = |n: &mut usize, seal: bool| {
        *n += 1;
        let file = format!("file{n}.txt");
        std::fs::write(repo.join(&file), format!("content {n}\n")).unwrap();
        if *n == 1 {
            std::fs::create_dir_all(repo.join("src")).unwrap();
            std::fs::write(repo.join("src/main.rs"), b"fn main() {}\n").unwrap();
            std::fs::write(repo.join("README.md"), b"hello world\n").unwrap();
        }
        prikk(&["commit", "--from-worktree", "-m", &format!("change {n}")]);
        if seal {
            prikk(&["seal", "--allow-no-audit"]);
        }
    };
    for _ in 0..sealed {
        commit(&mut n, true);
    }
    for _ in 0..queued {
        commit(&mut n, false);
    }
    repo
}

/// A stand-in `prikk` that logs every invocation's arguments to `log` and then runs the real program.
fn recording_binary(name: &str) -> (String, PathBuf) {
    let dir = scratch(&format!("{name}-recorder"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("calls.log");
    let script = dir.join("prikk-recorder.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec prikk \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script.display().to_string(), log)
}

struct Repos {
    /// (name, visibility, path)
    list: Vec<(&'static str, Visibility, PathBuf)>,
    binary: Option<String>,
}

struct App {
    router: Router,
    /// A session cookie for alice, who owns every repository.
    alice: String,
}

fn app(repos: Repos) -> App {
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (i, (name, visibility, path)) in repos.list.into_iter().enumerate() {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(format!("r{i}")),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path,
            })
            .unwrap();
    }
    let mut hosting = HostingService::new("/unused", store as Arc<dyn RepositoryStore>)
        .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    if let Some(b) = repos.binary {
        hosting = hosting.with_binary(b);
    }
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        Arc::new(hosting),
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    let sessions = Arc::new(InMemorySessionStore::new());
    let alice = sessions
        .create(UserId::new("alice"), 1_000_000, 2_000_000)
        .unwrap();
    let router = router(AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth: Arc::new(Authenticator::new(
            Arc::new(InMemoryAccountStore::new()),
            Arc::new(InMemoryCredentialStore::new()),
            hasher.clone(),
            hasher,
        )),
        content_origin: ContentOrigin::new(format!("https://{RAW}")),
        sessions,
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::default()),
        oidc: None,
        transport: None,
    });
    App {
        router,
        alice: format!("planeter_session={}", alice.id.0),
    }
}

struct Reply {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

/// The headers that say something about the response (not the per-request cookie that mints a CSRF token).
fn visible_headers(h: &HeaderMap) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = h
        .iter()
        .filter(|(k, _)| *k != header::SET_COOKIE)
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("?").to_owned()))
        .collect();
    v.sort();
    v
}

async fn get(a: &App, uri: &str, host: &str, cookie: Option<&str>) -> Reply {
    let mut req = Request::builder().uri(uri).header(header::HOST, host);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = a
        .router
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = visible_headers(resp.headers());
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    // `MISSING_PATH_DUMP=<dir>` saves every response the tests fetched (for the review request).
    if let Ok(dir) = std::env::var("MISSING_PATH_DUMP") {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let who = if cookie.is_some() { "reader" } else { "anon" };
        let name: String = format!("{host}{uri}")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{n:03}-{who}-{name}.txt")),
            format!(
                "HTTP {status}\n{headers:?}\n\n{}",
                String::from_utf8_lossy(&bytes)
            ),
        )
        .unwrap();
    }
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn text(key: Key) -> String {
    T::new(EN).get(key)
}

/// The page's sentence and link, as plain text.
/// The page with the value of every CSRF hidden field replaced by a fixed token.
fn sans_csrf_token(body: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let mut out = String::new();
    let mut rest = body;
    while let Some(i) = rest.find(marker) {
        let start = i + marker.len();
        out.push_str(&rest[..start]);
        out.push_str("TOKEN");
        let end = rest[start..].find('"').map_or(rest.len(), |j| start + j);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn missing_sentence(html: &str) -> bool {
    html.contains(&text(Key::NoSuchPathBody)) && html.contains(&text(Key::NoSuchPathLink))
}

fn repo_sentence(html: &str) -> bool {
    html.contains(&text(Key::NotFoundBody))
}

// ---------------------------------------------------------------------------------------------------------

fn two_repos(name: &str) -> App {
    let open = make_repo(&format!("{name}-open"), 1, 0);
    let secret = make_repo(&format!("{name}-secret"), 1, 0);
    app(Repos {
        list: vec![
            ("open", Visibility::Public, open),
            ("secret", Visibility::Private, secret),
        ],
        binary: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_api_answers_a_missing_path_exactly_as_a_hidden_or_an_absent_repository() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let a = two_repos("api-identity");
    // (route, host). `file` on the application host, `raw` on the content host, the same query.
    for (route, host) in [("file", APP), ("raw", RAW)] {
        let q = |repo: &str| format!("/api/v1/repos/alice/{repo}/{route}?path=src/nope.txt");
        let hidden = get(&a, &q("secret"), host, None).await; // a stranger, a private repository
        let absent = get(&a, &q("ghost"), host, None).await; // no such repository
        let public_missing = get(&a, &q("open"), host, None).await; // readable, the path is not there
        let owner_missing = get(&a, &q("secret"), host, Some(&a.alice)).await; // the authorized reader
        for (what, r) in [
            ("absent", &absent),
            ("public repository, missing path", &public_missing),
            ("authorized reader, missing path", &owner_missing),
        ] {
            assert_eq!(r.status, StatusCode::NOT_FOUND, "{route} {what}");
            assert_eq!(r.status, hidden.status, "{route} {what}: status");
            assert_eq!(r.headers, hidden.headers, "{route} {what}: headers");
            assert_eq!(r.body, hidden.body, "{route} {what}: body");
        }
        assert!(
            hidden.body.contains("\"code\":\"not_found\""),
            "{}",
            hidden.body
        );
    }
    // And a path that is there is still served (the control).
    let present = get(
        &a,
        "/api/v1/repos/alice/open/file?path=README.md",
        APP,
        None,
    )
    .await;
    assert_eq!(present.status, StatusCode::OK, "{}", present.body);
    // `tree` keeps its contract: a prefix with no entries is `200` with an empty listing (not a `404`).
    let tree = get(&a, "/api/v1/repos/alice/open/tree?prefix=nope", APP, None).await;
    assert_eq!(tree.status, StatusCode::OK, "{}", tree.body);
    assert!(tree.body.contains("\"entries\":[]"), "{}", tree.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_says_a_path_is_missing_only_to_a_reader_of_the_repository() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let a = two_repos("ui-visibility");
    for (kind, path) in [
        ("blob", "src/nope.txt"),
        ("tree", "src/nodir"),
        ("blob", "nope.txt"),
        ("tree", "nodir"),
    ] {
        let uri = |repo: &str| format!("/alice/{repo}/{kind}/{path}");

        // A reader of a public repository: the new sentence, 404, a link back to the files.
        let open = get(&a, &uri("open"), APP, None).await;
        assert_eq!(
            open.status,
            StatusCode::NOT_FOUND,
            "{uri}",
            uri = uri("open")
        );
        assert!(missing_sentence(&open.body), "{kind} {path}: {}", open.body);
        assert!(!repo_sentence(&open.body), "{kind} {path}");
        assert!(
            open.body.contains("<a href=\"/alice/open\">"),
            "the link to the files: {}",
            open.body
        );
        assert!(
            open.body.contains(&format!(
                "<title>Not found · /alice/open/{kind}/{path} · planeter</title>"
            )),
            "{}",
            open.body
        );
        assert!(open.body.contains("<h1>Not found</h1>"));

        // The authorized reader of a private repository: the same.
        let owner = get(&a, &uri("secret"), APP, Some(&a.alice)).await;
        assert_eq!(owner.status, StatusCode::NOT_FOUND);
        assert!(
            missing_sentence(&owner.body),
            "{kind} {path} (owner): {}",
            owner.body
        );

        // A stranger of the private repository: the repository-level sentence, never the new one…
        let stranger = get(&a, &uri("secret"), APP, None).await;
        assert_eq!(stranger.status, StatusCode::NOT_FOUND);
        assert!(
            repo_sentence(&stranger.body),
            "{kind} {path} (stranger): {}",
            stranger.body
        );
        assert!(
            !missing_sentence(&stranger.body),
            "{kind} {path} (stranger)"
        );
        // …and the same page, byte for byte but for the name in the address, as for a repository that is not there.
        let absent = get(&a, &uri("ghost"), APP, None).await;
        assert_eq!(absent.status, stranger.status);
        // (Every page now carries the language switcher's form with this visitor's own CSRF token, which is random
        // per visitor by design and says nothing about the repository: it is compared as a fixed token.)
        assert_eq!(
            sans_csrf_token(&absent.body.replace("/alice/ghost/", "/alice/secret/")),
            sans_csrf_token(&stranger.body),
            "{kind} {path}: a hidden and an absent repository must read the same"
        );
        // (`content-length` differs by the length of the name in the title, which the replacement above undoes.)
        let without_length = |h: &[(String, String)]| -> Vec<(String, String)> {
            h.iter()
                .filter(|(k, _)| k != "content-length")
                .cloned()
                .collect()
        };
        assert_eq!(
            without_length(&absent.headers),
            without_length(&stranger.headers)
        );
    }
    // A present file and folder are still shown (the controls).
    for uri in ["/alice/open/blob/README.md", "/alice/open/tree/src"] {
        let r = get(&a, uri, APP, None).await;
        assert_eq!(r.status, StatusCode::OK, "{uri}");
        assert!(!missing_sentence(&r.body), "{uri}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_link_on_the_missing_path_page_keeps_the_ref() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let a = two_repos("ui-ref-link");
    let r = get(&a, "/alice/open/blob/nope.txt?ref=heads/main", APP, None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.body);
    assert!(
        r.body.contains("<a href=\"/alice/open?ref=heads/main\">"),
        "{}",
        r.body
    );
}

// ----------------------------------------------------------------------------------------- T4 queued-only

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repository_whose_only_content_is_queued_work_is_pinned() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let queued_only = make_repo("queued-only", 0, 3);
    let (recorder, log) = recording_binary("queued-only");
    let a = app(Repos {
        list: vec![("q", Visibility::Public, queued_only)],
        binary: Some(recorder),
    });

    // History: empty, and it says so in the plain words.
    let h = get(&a, "/alice/q/history", APP, None).await;
    assert_eq!(h.status, StatusCode::OK, "{}", h.body);
    assert!(h.body.contains(&text(Key::HistoryEmpty)), "{}", h.body);
    // Tree and refs: 200 with empty content, page and API.
    for uri in ["/alice/q", "/alice/q/tree/src", "/alice/q/refs"] {
        let r = get(&a, uri, APP, None).await;
        assert_eq!(r.status, StatusCode::OK, "{uri}: {}", r.body);
    }
    let api_tree = get(&a, "/api/v1/repos/alice/q/tree", APP, None).await;
    assert_eq!(api_tree.status, StatusCode::OK);
    assert!(
        api_tree.body.contains("\"entries\":[]"),
        "{}",
        api_tree.body
    );
    let api_refs = get(&a, "/api/v1/repos/alice/q/refs", APP, None).await;
    assert_eq!(api_refs.status, StatusCode::OK);
    assert!(
        api_refs.body.contains("\"branches\":[]") && api_refs.body.contains("\"tags\":[]"),
        "{}",
        api_refs.body
    );
    // A folder in it: no sealed block, so nothing is said about the path (an empty listing, not a missing folder).
    let folder = get(&a, "/alice/q/tree/src", APP, None).await;
    assert!(!missing_sentence(&folder.body), "{}", folder.body);

    // `file` and `raw`: a missing path — the page and the API's `not_found`.
    let file = get(&a, "/alice/q/blob/README.md", APP, None).await;
    assert_eq!(file.status, StatusCode::NOT_FOUND);
    assert!(missing_sentence(&file.body), "{}", file.body);
    for (route, host) in [("file", APP), ("raw", RAW)] {
        let r = get(
            &a,
            &format!("/api/v1/repos/alice/q/{route}?path=README.md"),
            host,
            None,
        )
        .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{route}: {}", r.body);
        assert!(
            r.body.contains("\"code\":\"not_found\""),
            "{route}: {}",
            r.body
        );
    }

    // The call log: prikk was asked to read this repository many times, and **never** to plan a checkout.
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.lines().count() > 8,
        "the recorder saw the calls:\n{calls}"
    );
    assert!(calls.contains("tree "), "{calls}");
    assert!(
        !calls.contains("checkout"),
        "`checkout --patch-plan` must never run on a branch with no sealed block:\n{calls}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repository_with_sealed_and_queued_commits_shows_the_sealed_content_only() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let mixed = make_repo("mixed", 1, 2); // change 1 sealed; changes 2 and 3 queued
    let a = app(Repos {
        list: vec![("m", Visibility::Public, mixed)],
        binary: None,
    });
    // The sealed history: one change.
    let h = get(&a, "/api/v1/repos/alice/m/history", APP, None).await;
    assert_eq!(h.status, StatusCode::OK);
    assert_eq!(h.body.matches("\"block_id\"").count(), 1, "{}", h.body);
    // The sealed files are there; the queued ones are not.
    let tree = get(&a, "/api/v1/repos/alice/m/tree", APP, None).await;
    assert!(tree.body.contains("file1.txt"), "{}", tree.body);
    assert!(
        !tree.body.contains("file2.txt") && !tree.body.contains("file3.txt"),
        "{}",
        tree.body
    );
    let ok = get(&a, "/api/v1/repos/alice/m/file?path=file1.txt", APP, None).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert!(ok.body.contains("content 1"), "{}", ok.body);
    for queued in ["file2.txt", "file3.txt"] {
        let r = get(
            &a,
            &format!("/api/v1/repos/alice/m/file?path={queued}"),
            APP,
            None,
        )
        .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{queued}");
        let page = get(&a, &format!("/alice/m/blob/{queued}"), APP, None).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND);
        assert!(missing_sentence(&page.body), "{queued}");
    }
    // A missing folder in a sealed repository is a missing folder.
    let nodir = get(&a, "/alice/m/tree/nodir", APP, None).await;
    assert_eq!(nodir.status, StatusCode::NOT_FOUND);
    assert!(missing_sentence(&nodir.body), "{}", nodir.body);
    // An existing folder is not.
    let src = get(&a, "/alice/m/tree/src", APP, None).await;
    assert_eq!(src.status, StatusCode::OK);
}
