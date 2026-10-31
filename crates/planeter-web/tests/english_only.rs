//! The "English in every language" mechanism (handoff `japanese-consolidated-sheet-handoff-v1`, T3), shown with a **planted** list (the
//! shipped list, `ENGLISH_ONLY_KEYS`, is empty and a test in `catalog_checks.rs` says so): a key on the list is looked up in the English
//! table whatever the caller's language, the other tables do not hold it, the completeness check expects exactly that, and everything else
//! is as before. This file is a test binary of its own because the planted list is process-wide.

use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
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
    Dir, EN, EN_DEF, Entry, JA_DEF, Key, Lang, LangDef, Plural, Problem, T, audit,
    english_only_keys, register_test_language, set_english_only_for_test,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

/// The planted list: an API message and its hint, the heading of a plain page, a counted message, and a key English does not have.
const LIST: &[&str] = &[
    "error.not_found.message",
    "error.not_found.hint",
    "page.csrf.heading",
    "history.patch_count",
    "no.such.key",
];

fn plant() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| set_english_only_for_test(LIST));
}

fn other_only(_: u64) -> Plural {
    Plural::Other
}

/// A test language: English wrapped in guillemets, without the listed keys (it does not hold them) and, if `without` names one more
/// key, without that too (a message that is simply missing).
fn wrapped(tag: &'static str, without: Option<&'static str>) -> &'static LangDef {
    plant();
    let entries: Vec<Entry> = EN_DEF
        .entries
        .iter()
        .filter(|e| !LIST.contains(&e.key) && Some(e.key) != without)
        .map(|e| {
            let text: &'static str = Box::leak(format!("‹{}›", e.other).into_boxed_str());
            Entry {
                key: e.key,
                one: text,
                other: text,
            }
        })
        .collect();
    Box::leak(Box::new(LangDef {
        tag,
        endonym: "Test language",
        dir: Dir::Ltr,
        plural: other_only,
        group_separator: ',',
        decimal_separator: '.',
        entries: Box::leak(entries.into_boxed_slice()),
    }))
}

fn qaa() -> &'static LangDef {
    static DEF: OnceLock<&'static LangDef> = OnceLock::new();
    DEF.get_or_init(|| wrapped("qaa", None))
}

fn lang(def: &'static LangDef) -> Lang {
    register_test_language(def);
    Lang::from_def(def)
}

#[test]
fn the_planted_list_is_what_the_catalog_sees() {
    plant();
    assert_eq!(english_only_keys(), LIST);
}

#[test]
fn a_caller_of_another_language_gets_english_for_the_listed_keys_and_the_catalog_for_the_rest() {
    let (en, q) = (T::new(EN), T::new(lang(qaa())));
    for key in [
        Key::ErrNotFoundMessage,
        Key::ErrNotFoundHint,
        Key::PageCsrfHeading,
    ] {
        assert_eq!(
            q.get(key),
            en.get(key),
            "{}: English in every language",
            key.id()
        );
        assert!(!q.get(key).contains('‹'), "{}", key.id());
    }
    // Everything else is the caller's.
    for key in [
        Key::HomeOpen,
        Key::ErrUnavailableMessage,
        Key::PageCsrfBody,
        Key::NavCode,
    ] {
        assert_eq!(q.get(key), format!("‹{}›", en.get(key)), "{}", key.id());
    }
}

#[test]
fn a_counted_english_only_message_follows_the_english_plural_rule_not_the_callers() {
    let (en, q) = (T::new(EN), T::new(lang(qaa())));
    // The test language has no plural (one form for every count); the listed message is English, so it has two.
    for n in [0u64, 1, 2, 1200] {
        assert_eq!(
            q.msg(Key::HistoryPatchCount).n(n).text(),
            en.msg(Key::HistoryPatchCount).n(n).text(),
            "count {n}"
        );
    }
    assert_eq!(q.msg(Key::HistoryPatchCount).n(1).text(), "1 patch");
    assert_eq!(q.msg(Key::HistoryPatchCount).n(2).text(), "2 patches");
}

#[test]
fn a_message_that_is_missing_and_not_listed_is_still_a_visible_marker_never_english() {
    let gap = lang(wrapped("qab", Some("home.open")));
    assert_eq!(T::new(gap).get(Key::HomeOpen), "[home.open]");
    // ... while the listed keys are English for this caller too.
    assert_eq!(
        T::new(gap).get(Key::ErrNotFoundMessage),
        T::new(EN).get(Key::ErrNotFoundMessage)
    );
}

#[test]
fn the_completeness_check_expects_exactly_the_list() {
    plant();
    // A table without the listed keys is complete.
    assert_eq!(
        audit(&[&EN_DEF, qaa()]),
        vec![Problem::EnglishOnlyUnknown { key: "no.such.key" }],
        "only the key English does not have is a problem"
    );
    // A table that holds a listed key is a problem, key by key (the Japanese draft holds them all).
    let problems = audit(&[&EN_DEF, &JA_DEF]);
    for key in [
        "error.not_found.message",
        "error.not_found.hint",
        "page.csrf.heading",
        "history.patch_count",
    ] {
        assert!(
            problems.contains(&Problem::EnglishOnlyTranslated { lang: "ja", key }),
            "{key}: {problems:?}"
        );
    }
    assert!(problems.contains(&Problem::EnglishOnlyUnknown { key: "no.such.key" }));
    assert_eq!(problems.len(), 5, "{problems:?}");
}

// ---------------------------------------------------------------- through the router

fn app() -> Router {
    lang(qaa());
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("english-only-root"),
        store,
    );
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

async fn send(r: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let resp = r.clone().oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn the_apis_accept_language_path_answers_the_listed_message_and_hint_in_english_and_the_rest_in_the_language()
 {
    let r = app();
    let get = |uri: &str| {
        Request::builder()
            .uri(uri)
            .header(header::ACCEPT_LANGUAGE, "qaa")
            .body(Body::empty())
            .unwrap()
    };
    let (s, headers, body) = send(&r, get("/api/v1/nonsense")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let en = T::new(EN);
    assert_eq!(v["code"], "not_found");
    assert_eq!(v["message"], en.get(Key::ErrNotFoundMessage), "{body}");
    assert_eq!(v["hint"], en.get(Key::ErrNotFoundHint), "{body}");
    assert_eq!(
        headers.get("content-language").unwrap(),
        "qaa",
        "the language of the response is still the caller's"
    );
    // Another error of the same request path is the language's.
    let post = Request::builder()
        .method("POST")
        .uri("/api/v1/openapi.json")
        .header(header::ACCEPT_LANGUAGE, "qaa")
        .body(Body::empty())
        .unwrap();
    let (s, _, body) = send(&r, post).await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        body.contains("‹That method is not supported here.›"),
        "{body}"
    );
}

#[tokio::test]
async fn a_page_shows_the_listed_heading_in_english_and_the_rest_of_the_page_in_the_language() {
    let r = app();
    let post = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, "planeter_csrf=aaa")
        .header(header::ACCEPT_LANGUAGE, "qaa")
        .body(Body::from("csrf=bbb&username=a&password=b"))
        .unwrap();
    let (s, _, page) = send(&r, post).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the status is the same");
    let en = T::new(EN);
    assert!(
        page.contains(&format!("<h1>{}</h1>", en.get(Key::PageCsrfHeading))),
        "{page}"
    );
    assert!(
        page.contains(&format!("‹{}›", en.get(Key::PageCsrfBody))),
        "{page}"
    );
}
