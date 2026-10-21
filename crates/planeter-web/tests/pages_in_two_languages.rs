//! Every page the UI renders from a repository, in English and in the test language (`qaa`, every message
//! wrapped in `‹ ›`): no placeholder is left unfilled, every word comes from the catalog (the test language
//! shows it), and English says what it said before the catalog. Real prikk; self-skips without it.

use std::path::PathBuf;
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
use planeter_web::catalog::{Dir, Entry, LangDef, Plural, register_test_language, shipped};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn other_only(_: u64) -> Plural {
    Plural::Other
}

fn register_qaa() {
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
}

fn sealed() -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pages-two-languages");
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
            .unwrap();
        assert!(ok.success(), "prikk {args:?}");
    };
    prikk(&["setup"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("README.md"), b"# Hello\n\nA readme.\n").unwrap();
    std::fs::write(repo.join("src/main.rs"), b"fn main() {}\n").unwrap();
    std::fs::write(repo.join("data.bin"), vec![0xFFu8; 3000]).unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    prikk(&["branch", "create", "heads/dev"]);
    prikk(&["branch", "create", "heads/old"]);
    prikk(&["branch", "close", "heads/old"]);
    prikk(&["tag", "create", "tags/v1", "--target", "heads/main"]);
    repo
}

fn app(path: PathBuf) -> Router {
    register_qaa();
    let store = Arc::new(InMemoryRepositoryStore::new());
    store
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "app".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path,
        })
        .unwrap();
    let hosting = HostingService::new(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pages-root"),
        store as Arc<dyn RepositoryStore>,
    )
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

async fn get(r: &Router, uri: &str, accept: &str) -> (StatusCode, String) {
    let resp = r
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::ACCEPT_LANGUAGE, accept)
                .header(header::COOKIE, "planeter_csrf=t")
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

/// `{name}` left in a page: a placeholder the code did not fill.
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_repository_page_fills_every_placeholder_and_takes_every_word_from_the_catalog() {
    if !have("prikk") {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let r = app(sealed());
    let block = {
        let (_, history) = get(&r, "/alice/app/history", "en").await;
        let at =
            history.find("/alice/app/change/").expect("a change link") + "/alice/app/change/".len();
        history[at..at + 64].to_owned()
    };
    // (uri, English phrases the page has always said, whether the page has a nav)
    let pages: [(&str, &[&str]); 8] = [
        (
            "/alice/app",
            &[
                "<span class=\"badge ok\">Verified</span>",
                "Showing <code>heads/main</code> · branch <code>heads/main</code>",
                "2.9 KiB · binary",
                "<title>alice/app · planeter</title>",
            ],
        ),
        (
            "/alice/app/tree/src",
            &[
                "<h1><code>src</code></h1>",
                "<span class=\"muted\">Showing heads/main</span>",
                "13 B · text",
                "<title>src · alice/app · planeter</title>",
            ],
        ),
        (
            "/alice/app/blob/data.bin",
            &[
                "<span class=\"muted\">2.9 KiB · binary</span>",
                "This file is not text, so it cannot be shown here. <a href=\"https://raw.planeter.example/api/v1/repos/alice/app/raw?path=data.bin\">Download data.bin</a>.",
                ">Download data.bin</a>",
            ],
        ),
        (
            "/alice/app/blob/src/main.rs",
            &[
                "13 B · text",
                "<title>src/main.rs · alice/app · planeter</title>",
            ],
        ),
        (
            "/alice/app/history",
            &[
                "History <span class=\"muted\">heads/main</span>",
                "1 patch</td>",
                "<title>History · alice/app · planeter</title>",
                ">init</a>",
            ],
        ),
        (
            "/alice/app/refs",
            &[
                "<h1>Branches and tags</h1>",
                "<h2>Branches</h2>",
                "<span class=\"badge\">current</span>",
                "<span class=\"badge warn\">closed</span>",
                "<h2>Tags</h2>",
                "<title>Branches and tags · alice/app · planeter</title>",
            ],
        ),
        (
            "/alice/app/verify",
            &[
                "<h1>Verification <span class=\"badge ok\">Verified</span></h1>",
                "These checks were run just now, when you opened this page. They are never reused from an earlier visit. 1 of 1 session verified.",
                "<td>completed</td>",
                "<title>Verification · alice/app · planeter</title>",
            ],
        ),
        (
            "/alice/app/blob/README.md",
            &[
                "<h1><code>README.md</code></h1>",
                "<section class=\"readme\"><h2>Hello</h2>",
            ],
        ),
    ];
    for (uri, phrases) in pages {
        let (s, en) = get(&r, uri, "en").await;
        assert_eq!(s, StatusCode::OK, "{uri}: {en}");
        assert_eq!(
            unfilled(&en),
            None,
            "{uri}: an unfilled placeholder in English"
        );
        for p in phrases {
            assert!(en.contains(p), "{uri}: English no longer says {p:?}:\n{en}");
        }
        for nav in ["Files", "History", "Branches and tags", "Verification"] {
            if uri != "/alice/app/blob/README.md" || nav == "Files" {
                assert!(en.contains(&format!(">{nav}</a>")), "{uri}: nav {nav}");
            }
        }
        // The test language: right-to-left, every catalog word wrapped, nothing unfilled.
        let (s, q) = get(&r, uri, "qaa").await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(q.contains("<html lang=\"qaa\" dir=\"rtl\">"), "{uri}: {q}");
        assert_eq!(unfilled(&q), None, "{uri}: an unfilled placeholder in qaa");
        assert!(
            q.contains("‹Files›") && q.contains("‹History›"),
            "{uri}: the nav is not from the catalog: {q}"
        );
        // No English sentence of the catalog survives outside a marker: strip every ‹…› and look for words.
        let outside: String = {
            let mut out = String::new();
            let mut depth = 0;
            for c in q.chars() {
                match c {
                    '‹' => depth += 1,
                    '›' => depth -= 1,
                    c if depth == 0 => out.push(c),
                    _ => {}
                }
            }
            out
        };
        for word in [
            "Verified",
            "This file is not text",
            "There are no signed changes",
            "Branches",
            "Verification",
            "Sign in",
            "What does this mean?",
        ] {
            assert!(
                !outside.contains(word),
                "{uri}: {word:?} is not from the catalog:\n{q}"
            );
        }
    }
    // The change page (a block id), plus the three plain pages and the error pages.
    let (s, en) = get(&r, &format!("/alice/app/change/{block}"), "en").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        en.contains("<h1>Change <code>")
            && en.contains("<h2>patch <code>")
            && en.contains("badge neutral\">Signed into the history"),
        "{en}"
    );
    assert_eq!(unfilled(&en), None);
    let (_, q) = get(&r, &format!("/alice/app/change/{block}"), "qaa").await;
    assert!(
        q.contains("<h1>‹Change <code>") && q.contains("<title>‹Change "),
        "{q}"
    );
    assert_eq!(unfilled(&q), None);
    let (_, home) = get(&r, "/", "en").await;
    assert!(
        home.contains("<p class=\"muted\">planeter hosts prikk repositories. To open one, enter its address, written as <code>owner/name</code>.</p>")
            && home.contains("placeholder=\"owner/name\"")
            && home.contains(">Open repository</button>"),
        "{home}"
    );
    let (_, home_q) = get(&r, "/", "qaa").await;
    assert!(home_q.contains("‹planeter hosts prikk repositories. To open one, enter its address, written as <code>‹owner/name›</code>.›"), "{home_q}");
    for (uri, status, en_heading) in [
        ("/alice/none", StatusCode::NOT_FOUND, "<h1>Not found</h1>"),
        ("/", StatusCode::OK, "<h1>planeter</h1>"),
        ("/login", StatusCode::OK, "<h1>Sign in</h1>"),
    ] {
        let (s, en) = get(&r, uri, "en").await;
        assert_eq!(
            (s, en.contains(en_heading), unfilled(&en)),
            (status, true, None),
            "{uri}: {en}"
        );
        let (_, q) = get(&r, uri, "qaa").await;
        assert_eq!(unfilled(&q), None, "{uri}");
        assert!(
            !q.contains(">Sign in<") && !q.contains(">Not found<"),
            "{uri}: {q}"
        );
    }
}
