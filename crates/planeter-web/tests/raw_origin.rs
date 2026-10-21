//! IS-16 (RFC 003 D-4, threat model C-6c): the raw route answers only on the content origin's host.
//!
//! The router is driven with `ServiceExt::oneshot`; the TCP peer is supplied as `ConnectInfo`. Whether
//! a request **passed the host gate** is read from what the route does next, since a refused host is
//! deliberately indistinguishable from an unknown route: a request without the `path` parameter then
//! answers `400 invalid_request` (the gate decides before the query is looked at), and a request for a
//! public repository whose prikk cannot run answers `503 unavailable`.
//!
//! Nothing here needs prikk.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
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
use planeter_web::errors::ErrorCode;
use planeter_web::{
    AppState, BlockingGate, ContentOrigin, ContentOriginCheck, PrikkGate, REFUSE_SAME_HOST,
    TrustedProxies, WARN_SAME_HOST, check_content_origin, router,
};

const APP: &str = "app.example.org";
const RAW: &str = "raw.example.org";
const TRUSTED_PEER: &str = "10.0.0.1:5000";
const OUTSIDER: &str = "198.51.100.7:4000";

fn router_for(content: ContentOrigin) -> Router {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raw-origin");
    let store = Arc::new(InMemoryRepositoryStore::new());
    store
        .create(RepositoryRecord {
            repo_id: RepoId::new("r2"),
            owner: Owner::User("alice".into()),
            name: "open".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: root.join("never-opened"),
        })
        .unwrap();
    let hosting = Arc::new(
        HostingService::new(root, store as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined)
            .with_binary("/nonexistent/prikk"),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));
    let hasher = Arc::new(InsecureStubHasher);
    let auth = Arc::new(Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    ));
    router(AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth,
        content_origin: content,
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::parse("10.0.0.0/8").unwrap()),
        oidc: None,
    })
}

/// A deployment with a distinct content host, the application's own host being `APP`.
fn isolated() -> Router {
    router_for(ContentOrigin::new(format!("https://{RAW}")).with_app_host(APP))
}

struct Sent {
    status: StatusCode,
    body: Vec<u8>,
}

/// One request: `uri`, optional `Host`, optional `X-Forwarded-Host` lines, optional TCP peer.
async fn send(r: &Router, uri: &str, host: Option<&str>, xfh: &[&str], peer: Option<&str>) -> Sent {
    let mut req = Request::builder().uri(uri);
    if let Some(h) = host {
        req = req.header(header::HOST, h);
    }
    for x in xfh {
        req = req.header("x-forwarded-host", *x);
    }
    let mut req = req.body(Body::empty()).unwrap();
    if let Some(p) = peer {
        let addr: SocketAddr = p.parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
    }
    let resp = r.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 16)
        .await
        .unwrap()
        .to_vec();
    Sent { status, body }
}

/// The raw route with no `path`: past the gate this is `400`; refused by the gate it is `404`.
const RAW_NO_PATH: &str = "/api/v1/repos/alice/open/raw";
const RAW_PATH: &str = "/api/v1/repos/alice/open/raw?path=a.txt";
const UNKNOWN: &str = "/api/v1/repos/alice/open/not-a-route";

fn passed(s: &Sent) -> bool {
    s.status == StatusCode::BAD_REQUEST
}

// ---------------------------------------------------------------- T1: the gate

#[tokio::test]
async fn raw_is_served_on_the_content_host_and_refused_on_the_app_host() {
    let r = isolated();
    let on_content = send(&r, RAW_NO_PATH, Some(RAW), &[], Some(OUTSIDER)).await;
    assert!(passed(&on_content), "{:?}", on_content.status);
    let on_app = send(&r, RAW_NO_PATH, Some(APP), &[], Some(OUTSIDER)).await;
    assert_eq!(on_app.status, StatusCode::NOT_FOUND);
    // …with the code that says where raw bytes live, which is not the unknown-route body.
    let unknown = send(&r, UNKNOWN, Some(APP), &[], Some(OUTSIDER)).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(on_app.body, ErrorCode::RawElsewhere.body().into_bytes());
    assert_eq!(unknown.body, ErrorCode::NotFound.body().into_bytes());
}

#[tokio::test]
async fn a_request_that_reaches_the_read_path_on_the_content_host_is_served_there() {
    // The public repository's prikk cannot run: past the gate this is `503 unavailable`, not `404`.
    let r = isolated();
    let on_content = send(&r, RAW_PATH, Some(RAW), &[], Some(OUTSIDER)).await;
    assert_eq!(on_content.status, StatusCode::SERVICE_UNAVAILABLE);
    let on_app = send(&r, RAW_PATH, Some(APP), &[], Some(OUTSIDER)).await;
    assert_eq!(on_app.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_comparison_ignores_the_port_and_the_case() {
    let r = isolated();
    for host in [
        "raw.example.org:8443",
        "RAW.Example.ORG",
        "raw.example.org.:443",
    ] {
        let s = send(&r, RAW_NO_PATH, Some(host), &[], Some(OUTSIDER)).await;
        assert!(passed(&s), "{host}: {:?}", s.status);
    }
    // A different name that merely contains the content host is not it.
    for host in [
        "raw.example.org.evil.test",
        "xraw.example.org",
        "example.org",
    ] {
        let s = send(&r, RAW_NO_PATH, Some(host), &[], Some(OUTSIDER)).await;
        assert_eq!(s.status, StatusCode::NOT_FOUND, "{host}");
    }
}

#[tokio::test]
async fn no_host_at_all_is_refused() {
    let r = isolated();
    let s = send(&r, RAW_NO_PATH, None, &[], Some(OUTSIDER)).await;
    assert_eq!(s.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_gate_decides_before_the_query_and_before_authorization() {
    let r = isolated();
    // Same wrong host: a malformed query and a well-formed one are indistinguishable.
    let a = send(&r, RAW_NO_PATH, Some(APP), &[], None).await;
    let b = send(&r, RAW_PATH, Some(APP), &[], None).await;
    assert_eq!((a.status, &a.body), (b.status, &b.body));
}

// ---------------------------------------------------------------- X-Forwarded-Host trust

#[tokio::test]
async fn a_spoofed_forwarded_host_from_an_untrusted_peer_is_ignored() {
    let r = isolated();
    // Host is the application's; the forwarded header claims the content host.
    let s = send(&r, RAW_NO_PATH, Some(APP), &[RAW], Some(OUTSIDER)).await;
    assert_eq!(s.status, StatusCode::NOT_FOUND);
    // No peer at all counts as untrusted.
    let s = send(&r, RAW_NO_PATH, Some(APP), &[RAW], None).await;
    assert_eq!(s.status, StatusCode::NOT_FOUND);
    // And the reverse: an untrusted peer cannot *hide* behind Host by sending a different forwarded
    // value; Host alone decides.
    let s = send(&r, RAW_NO_PATH, Some(RAW), &[APP], Some(OUTSIDER)).await;
    assert!(passed(&s));
}

#[tokio::test]
async fn a_forwarded_host_from_a_trusted_proxy_is_honoured() {
    let r = isolated();
    // The proxy rewrote Host to its upstream's name and says what the client asked for.
    let s = send(
        &r,
        RAW_NO_PATH,
        Some("planeter.internal"),
        &[RAW],
        Some(TRUSTED_PEER),
    )
    .await;
    assert!(passed(&s), "{:?}", s.status);
    let s = send(
        &r,
        RAW_NO_PATH,
        Some("planeter.internal"),
        &[APP],
        Some(TRUSTED_PEER),
    )
    .await;
    assert_eq!(s.status, StatusCode::NOT_FOUND);
    // A trusted proxy that sends no forwarded host: Host decides.
    let s = send(&r, RAW_NO_PATH, Some(RAW), &[], Some(TRUSTED_PEER)).await;
    assert!(passed(&s));
}

#[tokio::test]
async fn behind_a_trusted_proxy_the_clients_own_forwarded_host_entry_cannot_win() {
    let r = isolated();
    // The client sent `X-Forwarded-Host: raw…`; the proxy appended the name it saw. The last entry,
    // which the proxy wrote, decides — in either header layout.
    let s = send(
        &r,
        RAW_NO_PATH,
        Some("x"),
        &[&format!("{RAW}, {APP}")],
        Some(TRUSTED_PEER),
    )
    .await;
    assert_eq!(s.status, StatusCode::NOT_FOUND);
    let s = send(&r, RAW_NO_PATH, Some("x"), &[RAW, APP], Some(TRUSTED_PEER)).await;
    assert_eq!(s.status, StatusCode::NOT_FOUND);
    let s = send(
        &r,
        RAW_NO_PATH,
        Some("x"),
        &[&format!("{APP}, {RAW}")],
        Some(TRUSTED_PEER),
    )
    .await;
    assert!(passed(&s));
}

// ---------------------------------------------------------------- same host

#[tokio::test]
async fn a_same_host_content_origin_leaves_the_route_ungated_for_development() {
    // Loopback development: content origin http://127.0.0.1:8080, application host 127.0.0.1.
    let r = router_for(ContentOrigin::new("http://127.0.0.1:8080").with_app_host("127.0.0.1"));
    for host in ["127.0.0.1:8080", "localhost:8080"] {
        let s = send(&r, RAW_NO_PATH, Some(host), &[], Some("127.0.0.1:4000")).await;
        assert!(passed(&s), "{host}: {:?}", s.status);
    }
}

#[tokio::test]
async fn the_content_origin_string_is_unchanged_so_ui_links_still_point_at_it() {
    let c = ContentOrigin::new("https://raw.example.org").with_app_host("10.1.2.3");
    assert_eq!(c.as_str(), "https://raw.example.org");
    assert_eq!(c.host().as_deref(), Some("raw.example.org"));
    // The app CSP keeps allowing images from it.
    assert!(planeter_web::app_csp(&c).contains("https://raw.example.org"));
}

// ---------------------------------------------------------------- T2: the start-up check

fn bind(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn origin_on(origin: &str, bind_addr: &str) -> ContentOrigin {
    ContentOrigin::new(origin).with_app_host(bind(bind_addr).ip().to_string())
}

#[test]
fn a_distinct_host_is_always_accepted() {
    for b in [
        "127.0.0.1:8080",
        "10.0.0.5:8080",
        "[::1]:8080",
        "0.0.0.0:8080",
    ] {
        let c = origin_on("https://raw.example.org", b);
        assert_eq!(
            check_content_origin(&c, bind(b)),
            Ok(ContentOriginCheck::Isolated),
            "{b}"
        );
    }
}

#[test]
fn the_same_host_on_a_non_loopback_bind_is_refused_with_the_stated_message() {
    for (origin, b) in [
        ("http://10.0.0.5:8080", "10.0.0.5:8080"), // the default shape
        ("https://10.0.0.5:8443", "10.0.0.5:8080"), // a port-only difference is the same host
        ("http://0.0.0.0:8080", "0.0.0.0:8080"),
        ("http://[2001:db8::5]:8080", "[2001:db8::5]:8080"),
    ] {
        let c = origin_on(origin, b);
        let got = check_content_origin(&c, bind(b));
        assert_eq!(got, Err(REFUSE_SAME_HOST.to_owned()), "{origin} on {b}");
    }
    assert_eq!(
        REFUSE_SAME_HOST,
        "planeter: refusing to start: raw repository content would be served from the \
         application's own host, which breaks the isolation the documentation promises. Set \
         PLANETER_CONTENT_ORIGIN to a different host (for example https://raw.example.org)."
    );
}

#[test]
fn the_same_host_on_a_loopback_bind_warns_and_starts() {
    for (origin, b) in [
        ("http://127.0.0.1:8080", "127.0.0.1:8080"),
        ("https://127.0.0.1:8443", "127.0.0.1:8080"),
        ("http://[::1]:8080", "[::1]:8080"),
    ] {
        let c = origin_on(origin, b);
        assert_eq!(
            check_content_origin(&c, bind(b)),
            Ok(ContentOriginCheck::SameHostLoopback),
            "{origin} on {b}"
        );
    }
    assert!(WARN_SAME_HOST.starts_with("planeter: warning:"));
    assert!(!WARN_SAME_HOST.contains('\n'));
}

#[test]
fn a_content_origin_without_a_host_is_refused_not_guessed() {
    for origin in ["raw.example.org", "https://", "https:///x", ""] {
        let c = ContentOrigin::new(origin).with_app_host("10.0.0.5");
        let got = check_content_origin(&c, bind("10.0.0.5:8080"));
        assert!(got.is_err(), "{origin:?}: {got:?}");
    }
}
