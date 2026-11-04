//! The language switcher with four languages or more (handoff `language-registry-handoff-v1` T5): the buttons would not fit a line of
//! the footer, so they sit in one native `<details>` "Language" — each entry still a no-script form with `lang` set and the current one
//! marked, the navigation block keeping its name — and the structural checker passes on both renderings. Two test languages are
//! registered here (process-wide, which is why this is a test binary of its own) to make four with the shipped ones.
//! `tests/language_switching.rs` is the inline rendering with up to three.

#[allow(dead_code)]
mod structure {
    #[path = "dom.rs"]
    pub mod dom;
    #[path = "rules.rs"]
    pub mod rules;
}

use std::sync::{Arc, OnceLock};

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
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore,
};
use planeter_web::catalog::{
    Category, Dir, EN_DEF, Key, Lang, LangDef, T, msg, planted_language, register_test_language,
    shipped,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn other_only(_: u64) -> Category {
    Category::Other
}

/// English wrapped in guillemets under a tag and an endonym of its own.
fn wrapped(tag: &'static str, endonym: &'static str, dir: Dir) -> &'static LangDef {
    let entries: Vec<planeter_web::catalog::Entry> = EN_DEF
        .entries
        .iter()
        .map(|e| {
            let text: &'static str = Box::leak(format!("‹{}›", e.other()).into_boxed_str());
            msg(e.key, text)
        })
        .collect();
    planted_language(
        tag,
        endonym,
        dir,
        planeter_web::catalog::OTHER_ONLY_CATEGORIES,
        other_only,
        ',',
        '.',
        Box::leak(entries.into_boxed_slice()),
    )
}

fn four_languages() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        register_test_language(wrapped("qaa", "Test language A", Dir::Rtl));
        register_test_language(wrapped("qab", "Test language B", Dir::Ltr));
    });
}

fn app() -> Router {
    four_languages();
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("language-menu-root"),
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
        transport: None,
        creation: None,
    })
}

async fn page(r: &Router, uri: &str, accept: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .header(header::ACCEPT_LANGUAGE, accept)
        .body(Body::empty())
        .unwrap();
    let resp = r.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn with_four_languages_the_switcher_is_one_details_menu_of_no_script_forms() {
    let r = app();
    let all = ["en", "ja", "qaa", "qab"];
    for accept in all {
        let current = accept;
        for uri in ["/", "/login", "/alice/none"] {
            let (_, body) = page(&r, uri, accept).await;
            let footer = &body[body.find("<footer>").expect("a footer")..];
            let t = T::new(Lang::from_tag(accept).expect("negotiable"));
            let name = t.get(Key::LayoutLanguage);
            // The navigation block keeps its name; the menu is a native <details> whose summary says the same word.
            assert!(
                footer.contains(&format!("<nav class=\"lang\" aria-label=\"{name}\">")),
                "{accept} {uri}: {footer}"
            );
            assert!(
                footer.contains(&format!("<details><summary>{name}</summary>")),
                "{accept} {uri}: {footer}"
            );
            assert!(footer.contains("</details>"), "{accept} {uri}");
            assert_eq!(footer.matches("<details").count(), 1);
            // Every language is still a real form, with `lang` on its button and the current one marked.
            assert_eq!(
                footer.matches("action=\"/lang\"").count(),
                4,
                "{accept} {uri}"
            );
            for d in shipped().iter().copied().chain([
                Lang::from_tag("qaa").unwrap().def(),
                Lang::from_tag("qab").unwrap().def(),
            ]) {
                let button = format!(
                    "<button type=\"submit\" lang=\"{}\"{}>{}</button>",
                    d.tag,
                    if d.tag == current {
                        " aria-current=\"true\""
                    } else {
                        ""
                    },
                    d.endonym
                );
                assert!(footer.contains(&button), "{accept} {uri}: {button}");
            }
            assert!(!body.to_lowercase().contains("<script"));
        }
    }
}

#[tokio::test]
async fn the_structural_checker_passes_on_the_menu_in_english_and_in_japanese() {
    let r = app();
    for accept in ["en", "ja"] {
        for uri in ["/", "/login", "/alice/none", "/no-such-page"] {
            let (_, body) = page(&r, uri, accept).await;
            let found = structure::rules::check(&structure::dom::parse(&body));
            assert!(found.is_empty(), "{accept} {uri}: {found:#?}");
        }
    }
}

#[test]
fn three_languages_are_inline_and_the_fourth_makes_the_menu() {
    four_languages();
    let t = T::new(Lang::from_tag("en").unwrap());
    assert_eq!(t.languages().len(), 4);
    assert!(t.language_menu());
}

/// Prints the footer of the sign-in page with four languages, for the review request: `cargo test --test language_menu dump -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "prints the footer"]
async fn dump_the_footer_with_four_languages() {
    let r = app();
    let (_, body) = page(&r, "/login", "en").await;
    println!("{}", &body[body.find("<footer>").unwrap()..]);
    let found = structure::rules::check(&structure::dom::parse(&body));
    println!("<!-- structural checker: {} violation(s) -->", found.len());
}
