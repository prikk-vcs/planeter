//! Responses the framework would write by itself — a path that is not UTF-8, a body over its limit, an address no route serves, a method
//! a page route does not take — are the project's own: for `/api/` the one JSON error shape `{code, message, hint}` in the caller's
//! language, for every other path a page with a heading, one sentence and one link; the statuses are the honest ones (`413` for a body
//! that is too large) and nothing a stranger can learn about a repository's existence changes (these are route-level answers).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

fn app() -> Router {
    planeter_web::catalog::enable_preview_languages("ja").expect("ja is previewable");
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("edge-rejections-root"),
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

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Reply {
    fn h(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

/// One probe: a method, an address, the body (if any) and the language asked for.
struct Probe {
    name: &'static str,
    method: Method,
    uri: &'static str,
    body: Option<(&'static str, usize)>,
}

const BIG: usize = 3 * 1024 * 1024;

fn probes() -> Vec<Probe> {
    let p = |name, method, uri, body| Probe {
        name,
        method,
        uri,
        body,
    };
    vec![
        p(
            "api: a path that is not UTF-8",
            Method::GET,
            "/api/v1/repos/%ff/b/history",
            None,
        ),
        p(
            "api: a name that is not UTF-8",
            Method::GET,
            "/api/v1/repos/a/%ff/refs",
            None,
        ),
        p(
            "api: an unknown address",
            Method::GET,
            "/api/v1/nonsense",
            None,
        ),
        p(
            "api: a wrong method",
            Method::POST,
            "/api/v1/openapi.json",
            None,
        ),
        p(
            "api: create, body over the limit",
            Method::POST,
            "/api/v1/repos",
            Some(("application/json", BIG)),
        ),
        p(
            "api: close a proposal, body over the limit",
            Method::POST,
            "/api/v1/repos/a/b/proposals/1/close",
            Some(("application/json", BIG)),
        ),
        p(
            "page: sign in, body over the limit",
            Method::POST,
            "/login",
            Some(("application/x-www-form-urlencoded", BIG)),
        ),
        p(
            "page: create, body over the limit",
            Method::POST,
            "/new",
            Some(("application/x-www-form-urlencoded", BIG)),
        ),
        p(
            "page: close a proposal, body over the limit",
            Method::POST,
            "/a/b/proposals/1/close",
            Some(("application/x-www-form-urlencoded", BIG)),
        ),
        p(
            "page: an unknown address",
            Method::GET,
            "/no-such-page",
            None,
        ),
        p("page: DELETE /", Method::DELETE, "/", None),
        p(
            "page: a wrong method on a page route",
            Method::GET,
            "/new/import",
            None,
        ),
        p(
            "page: a query that is not UTF-8",
            Method::GET,
            "/a/b/?x=%ff",
            None,
        ),
    ]
}

async fn send(r: &Router, probe: &Probe, accept: &str) -> Reply {
    let mut b = Request::builder()
        .method(probe.method.clone())
        .uri(probe.uri)
        .header(header::ACCEPT_LANGUAGE, accept)
        .header(header::COOKIE, "planeter_csrf=t");
    let body = match probe.body {
        Some((ct, n)) => {
            b = b
                .header(header::CONTENT_TYPE, ct)
                .header(header::CONTENT_LENGTH, n);
            Body::from(vec![b'a'; n])
        }
        None => Body::empty(),
    };
    let resp = r.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn json(reply: &Reply) -> serde_json::Value {
    serde_json::from_str(&reply.body).unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", reply.body))
}

fn has_japanese(text: &str) -> bool {
    text.chars()
        .any(|c| ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// A chunked body of `n` bytes with no `Content-Length` (so that only the count can notice).
fn chunked(
    method: Method,
    uri: &'static str,
    content_type: &'static str,
    n: usize,
) -> Request<Body> {
    let chunks = (0..n / 65_536).map(|_| Ok::<_, std::io::Error>(vec![b'a'; 65_536]));
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ACCEPT_LANGUAGE, "ja")
        .header(header::CONTENT_TYPE, content_type)
        .header(header::COOKIE, "planeter_csrf=t")
        .body(Body::from_stream(futures_util_stream(chunks)))
        .unwrap()
}

/// A stream over an iterator, without a dependency for it (boxed, so that it is `Unpin`).
fn futures_util_stream<T: Send + 'static>(
    items: impl Iterator<Item = Result<T, std::io::Error>> + Send + 'static,
) -> impl futures_core::Stream<Item = Result<T, std::io::Error>> + Send + 'static {
    struct Iter<T>(Box<dyn Iterator<Item = Result<T, std::io::Error>> + Send>);
    impl<T> futures_core::Stream for Iter<T> {
        type Item = Result<T, std::io::Error>;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Ready(self.get_mut().0.next())
        }
    }
    Iter(Box::new(items))
}

async fn raw(r: &Router, req: Request<Body>) -> Reply {
    let resp = r.clone().oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

#[tokio::test]
async fn an_api_client_gets_the_one_json_shape_for_every_error_in_both_languages() {
    let r = app();
    for accept in ["en", "ja"] {
        for probe in probes().into_iter().filter(|p| p.uri.starts_with("/api/")) {
            let reply = send(&r, &probe, accept).await;
            assert!(
                reply.status.is_client_error() || reply.status.is_server_error(),
                "{}",
                probe.name
            );
            assert_eq!(
                reply.h("content-type"),
                "application/json",
                "{accept}: {}",
                probe.name
            );
            assert_eq!(
                reply.h("content-language"),
                accept,
                "{accept}: {}",
                probe.name
            );
            let v = json(&reply);
            let o = v.as_object().unwrap();
            assert!(
                o.contains_key("code") && o.contains_key("message") && o.contains_key("hint"),
                "{}",
                reply.body
            );
            assert_eq!(
                has_japanese(v["message"].as_str().unwrap()),
                accept == "ja",
                "{accept}: {}",
                probe.name
            );
        }
    }
}

#[tokio::test]
async fn the_statuses_and_codes_are_the_honest_ones() {
    let r = app();
    let by = |name: &str| probes().into_iter().find(|p| p.name == name).unwrap();
    for accept in ["en", "ja"] {
        let undecodable = send(&r, &by("api: a path that is not UTF-8"), accept).await;
        assert_eq!(
            (undecodable.status, json(&undecodable)["code"].clone()),
            (StatusCode::BAD_REQUEST, "invalid_request".into())
        );
        let name = send(&r, &by("api: a name that is not UTF-8"), accept).await;
        assert_eq!(name.status, StatusCode::BAD_REQUEST);
        // A body that is too large is 413 `payload_too_large`, never 400, never the proposal's sentence.
        for probe in [
            "api: create, body over the limit",
            "api: close a proposal, body over the limit",
        ] {
            let big = send(&r, &by(probe), accept).await;
            assert_eq!(big.status, StatusCode::PAYLOAD_TOO_LARGE, "{probe}");
            let v = json(&big);
            assert_eq!(v["code"], "payload_too_large");
            assert!(
                !v["message"].as_str().unwrap().contains("proposal")
                    && !v["message"].as_str().unwrap().contains("提案"),
                "{v}"
            );
        }
        let nope = send(&r, &by("api: an unknown address"), accept).await;
        assert_eq!(
            (nope.status, json(&nope)["code"].clone()),
            (StatusCode::NOT_FOUND, "not_found".into())
        );
        let method = send(&r, &by("api: a wrong method"), accept).await;
        assert_eq!(
            (method.status, method.h("allow")),
            (StatusCode::METHOD_NOT_ALLOWED, "GET")
        );
    }
}

#[tokio::test]
async fn a_browser_never_gets_an_empty_body_for_a_client_error_and_the_page_is_in_the_callers_language()
 {
    let r = app();
    let want = [
        (
            "page: sign in, body over the limit",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "page: create, body over the limit",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "page: close a proposal, body over the limit",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        ("page: an unknown address", StatusCode::NOT_FOUND),
        ("page: DELETE /", StatusCode::METHOD_NOT_ALLOWED),
        (
            "page: a wrong method on a page route",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        ("page: a query that is not UTF-8", StatusCode::NOT_FOUND),
    ];
    for accept in ["en", "ja"] {
        for (name, status) in want {
            let probe = probes().into_iter().find(|p| p.name == name).unwrap();
            let reply = send(&r, &probe, accept).await;
            assert_eq!(reply.status, status, "{accept}: {name}");
            assert!(
                reply.h("content-type").starts_with("text/html"),
                "{accept}: {name}"
            );
            assert_eq!(reply.h("content-language"), accept, "{accept}: {name}");
            assert!(
                reply.body.contains(&format!("<html lang=\"{accept}\"")),
                "{accept}: {name}"
            );
            assert!(
                reply.body.contains("<h1>") && reply.body.contains("<a href="),
                "a heading and a link: {accept}: {name}"
            );
            let h1 = reply
                .body
                .split("<h1>")
                .nth(1)
                .and_then(|x| x.split("</h1>").next())
                .unwrap_or("");
            assert_eq!(has_japanese(h1), accept == "ja", "{accept}: {name}: {h1}");
        }
    }
    // The method the route does take is still told (`Allow` survives).
    let del = probes()
        .into_iter()
        .find(|p| p.name == "page: DELETE /")
        .unwrap();
    assert_eq!(send(&r, &del, "en").await.h("allow"), "GET,HEAD");
}

#[tokio::test]
async fn no_probe_answers_with_an_empty_body_or_the_frameworks_own_words() {
    let r = app();
    for accept in ["en", "ja"] {
        for probe in probes() {
            let reply = send(&r, &probe, accept).await;
            assert!(
                !reply.body.is_empty(),
                "{accept}: {} answered an empty body",
                probe.name
            );
            assert!(
                !reply.h("content-type").starts_with("text/plain"),
                "{accept}: {}",
                probe.name
            );
            for words in ["Invalid URL", "Failed to buffer", "length limit exceeded"] {
                assert!(
                    !reply.body.contains(words),
                    "{accept}: {}: {words}",
                    probe.name
                );
            }
        }
    }
}

#[tokio::test]
async fn a_body_with_no_declared_length_is_counted_and_refused_the_same_way() {
    let r = app();
    let api = raw(
        &r,
        chunked(
            Method::POST,
            "/api/v1/repos/a/b/proposals/1/close",
            "application/json",
            BIG,
        ),
    )
    .await;
    assert_eq!(api.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(json(&api)["code"], "payload_too_large");
    let page = raw(
        &r,
        chunked(
            Method::POST,
            "/login",
            "application/x-www-form-urlencoded",
            BIG,
        ),
    )
    .await;
    assert_eq!(page.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(page.body.contains("<html lang=\"ja\""));
    let h1 = page
        .body
        .split("<h1>")
        .nth(1)
        .unwrap()
        .split("</h1>")
        .next()
        .unwrap();
    assert!(has_japanese(h1), "{h1}");
    // Just under the limit is not refused for its size (a small body is the handler's business, as before).
    let under = raw(
        &r,
        chunked(
            Method::POST,
            "/login",
            "application/x-www-form-urlencoded",
            1_048_576,
        ),
    )
    .await;
    assert_ne!(
        under.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body under the limit is not too large"
    );
}

#[tokio::test]
async fn a_small_body_is_untouched_and_a_route_with_its_own_bound_is_not_limited_here() {
    let r = app();
    // A small, wrong CSRF form is the sign-in page's own refusal, as before.
    let small = Request::builder()
        .method(Method::POST)
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, "planeter_csrf=aaa")
        .body(Body::from("csrf=bbb&username=a&password=b"))
        .unwrap();
    assert_eq!(raw(&r, small).await.status, StatusCode::FORBIDDEN);
    // The proposal route and the staging of an archive bound their own bodies: the edge answers neither with its 413.
    for (uri, ct) in [
        (
            "/api/v1/repos/a/b/transport/propose",
            "application/octet-stream",
        ),
        ("/api/v1/repos/import", "application/octet-stream"),
        ("/new/import", "multipart/form-data; boundary=x"),
    ] {
        let big = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::CONTENT_TYPE, ct)
            .header(header::CONTENT_LENGTH, BIG)
            .body(Body::from(vec![b'a'; BIG]))
            .unwrap();
        let reply = raw(&r, big).await;
        assert_ne!(
            reply.status,
            StatusCode::PAYLOAD_TOO_LARGE,
            "{uri}: its own handler answers, not the edge: {}",
            reply.body
        );
        assert!(
            !reply.body.contains("payload_too_large"),
            "{uri}: {}",
            reply.body
        );
    }
}

#[tokio::test]
async fn an_unknown_address_says_nothing_about_a_repository() {
    // The page for an address nobody serves is the page a missing or hidden repository gets: the same status, the same heading.
    let r = app();
    let missing = raw(
        &r,
        Request::builder()
            .uri("/alice/none")
            .header(header::ACCEPT_LANGUAGE, "en")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let unknown = raw(
        &r,
        Request::builder()
            .uri("/no-such-page")
            .header(header::ACCEPT_LANGUAGE, "en")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(missing.status, unknown.status);
    let heading = |b: &str| {
        b.split("<h1>")
            .nth(1)
            .and_then(|x| x.split("</h1>").next())
            .unwrap_or("")
            .to_owned()
    };
    assert_eq!(heading(&missing.body), heading(&unknown.body));
    assert!(!heading(&unknown.body).is_empty());
}

/// The transcript the review request prints (status, content type, language, first bytes): `cargo test … -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "prints the probe transcript"]
async fn transcript() {
    let r = app();
    for accept in ["en", "ja"] {
        for probe in probes() {
            let reply = send(&r, &probe, accept).await;
            let head: String = reply
                .body
                .chars()
                .take(70)
                .collect::<String>()
                .replace('\n', " ");
            println!(
                "{accept} | {} | {} {} | content-type {:?} | content-language {:?} | allow {:?} | {} bytes | {:?}",
                probe.name,
                probe.method,
                probe.uri,
                reply.h("content-type"),
                reply.h("content-language"),
                reply.h("allow"),
                reply.body.len(),
                head
            );
            println!("    -> {}", reply.status);
        }
    }
}

/// The pseudo-language (every message wrapped in guillemets) reaches these answers too: nothing here is written outside the catalog.
#[tokio::test]
async fn the_pseudo_language_wraps_every_edge_message() {
    use planeter_web::catalog::{Dir, EN_DEF, Entry, LangDef, Plural, register_test_language};
    fn one(_: u64) -> Plural {
        Plural::Other
    }
    let entries: Vec<Entry> = EN_DEF
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
    let def: &'static LangDef = Box::leak(Box::new(LangDef {
        tag: "qaa",
        endonym: "Test language",
        dir: Dir::Ltr,
        plural: one,
        group_separator: ',',
        decimal_separator: '.',
        entries: Box::leak(entries.into_boxed_slice()),
    }));
    register_test_language(def);
    let r = app();
    let by = |name: &str| probes().into_iter().find(|p| p.name == name).unwrap();
    for name in [
        "api: a path that is not UTF-8",
        "api: close a proposal, body over the limit",
        "api: an unknown address",
    ] {
        let v = json(&send(&r, &by(name), "qaa").await);
        assert!(
            v["message"].as_str().unwrap().starts_with('‹'),
            "{name}: {v}"
        );
        assert!(v["hint"].as_str().unwrap().starts_with('‹'), "{name}: {v}");
    }
    for name in [
        "page: an unknown address",
        "page: DELETE /",
        "page: sign in, body over the limit",
    ] {
        let reply = send(&r, &by(name), "qaa").await;
        let h1 = reply
            .body
            .split("<h1>")
            .nth(1)
            .unwrap()
            .split("</h1>")
            .next()
            .unwrap();
        assert!(h1.starts_with('‹'), "{name}: {h1}");
    }
}
