//! RFC 012 D-2 / D-6a: the sign-in and address forms say what went wrong where the person is looking, the
//! lock-out has its own message without telling anyone which accounts exist, and the signed-in header shows a
//! person's name intact. Needs no prikk (no repository is read).

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, InsecureStubHasher, PasswordHasher as _, SessionStore,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore, UserId,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

const CSRF: &str = "tok123";

struct App {
    router: Router,
    sessions: Arc<InMemorySessionStore>,
}

/// An app whose accounts are `names`: (account id, display name). Every account's password is `pw`.
fn app(names: &[(&str, &str)]) -> App {
    let accounts = Arc::new(InMemoryAccountStore::new());
    for (id, name) in names {
        accounts
            .upsert(Account {
                user: UserId::new(*id),
                display_name: (*name).into(),
                email: None,
                password: Some(InsecureStubHasher.hash("pw")),
            })
            .unwrap();
    }
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("signin-root"),
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
    let sessions = Arc::new(InMemorySessionStore::new());
    let router = router(AppState {
        read,
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth: Arc::new(Authenticator::new(
            accounts,
            Arc::new(InMemoryCredentialStore::new()),
            hasher.clone(),
            hasher,
        )),
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: sessions.clone(),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::default()),
        oidc: None,
        transport: None,
    });
    App { router, sessions }
}

struct Reply {
    status: StatusCode,
    /// Every header, sorted: the whole of what a client sees besides the body.
    headers: Vec<(String, String)>,
    body: String,
}

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let resp = r.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let mut headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    headers.sort();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .uri(uri)
        .header(header::HOST, "forge.example");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

fn login(user: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::HOST, "forge.example")
        .header(header::COOKIE, format!("planeter_csrf={CSRF}"))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "csrf={CSRF}&username={user}&password={password}"
        )))
        .unwrap()
}

// ---------------------------------------------------------------------------------------------------------
// The sign-in failure and the lock-out

#[tokio::test]
async fn a_failed_sign_in_is_an_alert_tied_to_the_form_and_a_fresh_form_has_none() {
    let a = app(&[("alice", "Alice")]);
    let fresh = send(&a.router, get("/login", None)).await;
    assert!(!fresh.body.contains("role=\"alert\""), "{}", fresh.body);
    assert!(!fresh.body.contains("aria-describedby"), "{}", fresh.body);

    let failed = send(&a.router, login("alice", "wrong")).await;
    assert_eq!(failed.status, StatusCode::UNAUTHORIZED);
    assert!(
        failed.body.contains(
            "<p id=\"login-error\" class=\"badge bad\" role=\"alert\">The username or password was not correct. Check both and try again.</p>"
        ),
        "{}",
        failed.body
    );
    assert!(
        failed
            .body
            .contains("<form method=\"post\" action=\"/login\" aria-describedby=\"login-error\">"),
        "{}",
        failed.body
    );
    // The fields keep their autofill hints and their visible labels.
    assert!(failed.body.contains("autocomplete=\"username\""));
    assert!(failed.body.contains("autocomplete=\"current-password\""));
    assert!(failed.body.contains("<label>Username <input"));
    assert!(failed.body.contains("<label>Password <input"));
}

/// Fail `n` times for `user`, then ask once more with the right password: the lock-out's answer.
async fn locked_answer(a: &App, user: &str, n: usize) -> Reply {
    for _ in 0..n {
        let r = send(&a.router, login(user, "wrong")).await;
        assert!(matches!(
            r.status,
            StatusCode::UNAUTHORIZED | StatusCode::TOO_MANY_REQUESTS
        ));
    }
    send(&a.router, login(user, "pw")).await
}

#[tokio::test]
async fn a_locked_account_gets_its_own_message_and_a_locked_unknown_name_gets_exactly_the_same_response()
 {
    let a = app(&[("alice", "Alice")]);
    let existing = locked_answer(&a, "alice", 12).await;
    let unknown = locked_answer(&a, "mallory", 12).await;

    assert_eq!(existing.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        existing.body.contains(
            "<p id=\"login-error\" class=\"badge bad\" role=\"alert\">Too many sign-in attempts. Wait a few minutes and try again.</p>"
        ),
        "{}",
        existing.body
    );
    assert!(
        existing
            .body
            .contains("<title>Sign in · too many attempts · planeter</title>"),
        "{}",
        existing.body
    );
    // The lock does not say which names exist: status, every header and the whole body are identical.
    assert_eq!(existing.status, unknown.status);
    assert_eq!(existing.headers, unknown.headers, "headers differ");
    assert_eq!(existing.body, unknown.body, "bodies differ");

    // Wrong credentials keep their single generic message, the same for an existing and an unknown name,
    // and it is not the lock-out's.
    let b = app(&[("alice", "Alice")]);
    let wrong_existing = send(&b.router, login("alice", "wrong")).await;
    let wrong_unknown = send(&b.router, login("nobody", "wrong")).await;
    assert_eq!(wrong_existing.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_existing.status, wrong_unknown.status);
    assert_eq!(wrong_existing.headers, wrong_unknown.headers);
    assert_eq!(wrong_existing.body, wrong_unknown.body);
    assert!(
        wrong_existing
            .body
            .contains(">The username or password was not correct. Check both and try again.</p>")
    );
    assert!(!wrong_existing.body.contains("Too many sign-in attempts"));
    assert_ne!(wrong_existing.body, existing.body);

    // The lock is on the account name the person typed, so a locked name stays locked with the right password.
    assert!(!existing.body.contains("Welcome"));
}

// ---------------------------------------------------------------------------------------------------------
// The address field

#[tokio::test]
async fn a_bad_address_shows_the_home_page_again_with_the_error_beside_the_field() {
    let a = app(&[]);
    for uri in [
        "/go?repo=alicefull",
        "/go?repo=alice/",
        "/go?repo=a/b/c",
        "/go?repo=",
        "/go",
    ] {
        let r = send(&a.router, get(uri, None)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{uri}");
        assert!(r.body.contains("<h1>planeter</h1>"), "{uri}: {}", r.body);
        assert!(
            r.body
                .contains("<label for=\"repo\">Repository address</label>"),
            "{uri}: {}",
            r.body
        );
        assert!(
            r.body.contains("autocomplete=\"off\" spellcheck=\"false\" required aria-invalid=\"true\" aria-describedby=\"repo-error\">"),
            "{uri}: {}",
            r.body
        );
        assert!(
            r.body.contains("<p id=\"repo-error\" class=\"badge bad\">That is not a repository address. Write it as <code>owner/name</code>.</p>"),
            "{uri}: {}",
            r.body
        );
        assert!(
            r.body
                .contains("<title>Find a repository · address not recognised · planeter</title>")
        );
    }
    // What was typed comes back in the field, escaped.
    let r = send(
        &a.router,
        get(
            "/go?repo=%22%3E%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E",
            None,
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(!r.body.contains("<img"), "{}", r.body);
    assert!(
        r.body
            .contains("value=\"&#34;&#62;&#60;img src=x onerror=alert(1)&#62;\""),
        "{}",
        r.body
    );
    // A good address still redirects, and the plain home page has no error and no invalid mark.
    let ok = send(&a.router, get("/go?repo=alice/app", None)).await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER);
    let home = send(&a.router, get("/", None)).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(
        !home.body.contains("aria-invalid") && !home.body.contains("repo-error"),
        "{}",
        home.body
    );
}

// ---------------------------------------------------------------------------------------------------------
// Names (D-6a)

/// The page header for a person with this display name.
async fn header_for(id: &str, display_name: &str) -> String {
    let a = app(&[(id, display_name)]);
    let session = a
        .sessions
        .create(UserId::new(id), 1_000_000, 1_000_000 + 3600)
        .unwrap();
    let r = send(
        &a.router,
        get(
            "/",
            Some(&format!(
                "planeter_csrf={CSRF}; planeter_session={}",
                session.id.0
            )),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let start = r.body.find("<div class=\"who\">").expect("a who block");
    let end = r.body[start..].find("</div>").unwrap() + start;
    r.body[start..end].to_owned()
}

#[tokio::test]
async fn a_display_name_is_shown_whole_in_order_escaped_and_isolated_with_the_account_id_beside_it()
{
    let long = "Maximiliane-Alexandra ".repeat(15) + "Zweitnamenlos";
    for name in [
        "Zoë Müller-Ñandú",
        "山田 太郎",
        "محمد الأحمد",
        "שרה כהן",
        long.as_str(),
        "Ada <b>\"O'Brien\"</b> & Co",
        "A\u{200F}B",
    ] {
        let who = header_for("user-7", name).await;
        let mut escaped = String::new();
        for c in name.chars() {
            match c {
                '&' => escaped.push_str("&#38;"),
                '<' => escaped.push_str("&#60;"),
                '>' => escaped.push_str("&#62;"),
                '"' => escaped.push_str("&#34;"),
                '\'' => escaped.push_str("&#39;"),
                c => escaped.push(c),
            }
        }
        assert!(
            who.contains(&format!(
                "<bdi class=\"name\">{escaped}</bdi> <bdi class=\"muted\">user-7</bdi>"
            )),
            "name {name:?}: {who}"
        );
        // Nothing the name holds became markup.
        assert!(!who.contains("<b>") && !who.contains("<script"), "{who}");
        // Once, whole: the name is not truncated, split or repeated.
        assert_eq!(who.matches(&escaped).count(), 1, "{who}");
    }
}

#[tokio::test]
async fn with_no_display_name_the_account_id_alone_is_shown() {
    for name in ["", "   ", "\t\n"] {
        let who = header_for("user-7", name).await;
        assert!(
            who.contains("<span class=\"account\"><bdi class=\"name\">user-7</bdi></span>"),
            "{name:?}: {who}"
        );
        assert!(!who.contains("class=\"muted\""), "{name:?}: {who}");
    }
    // Anonymous: no account block at all, the sign-in link.
    let a = app(&[]);
    let r = send(&a.router, get("/", None)).await;
    assert!(
        r.body
            .contains("<div class=\"who\"><a href=\"/login\">Sign in</a></div>"),
        "{}",
        r.body
    );
}
