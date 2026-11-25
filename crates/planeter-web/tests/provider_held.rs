//! RFC 014 ID-1, ID-2 **held, not built**: planeter reads **no `name` and no `preferred_username`** from the provider (it asks the provider
//! for no userinfo at all: discovery, the key set and the token exchange are the only requests), and it **stores no email** from it. These
//! two tests exist so that a later change cannot add either quietly.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemorySessionStore, OidcConfig, OidcProvider,
    PasswordHash, PasswordHasher, Sha256TokenHasher, SqliteAccountStore, SqliteCredentialStore,
};
use planeter_core::{
    FetchError, FetchRequest, HostingService, HttpsFetcher, NullAuditSink, ReadService,
};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore, SqliteDb,
    UserId,
};
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, router};

const ISSUER: &str = "https://idp.example";
const CLIENT: &str = "planeter-client";
const KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgopZeFiRGZXWvz9h4
G2WRp6WFSymfcUW7Kavsub8xSMehRANCAATQ+QQEj43jJBGpA4RYiR3l6Pnckdxw
BXaTdqRaPA7mOrNmL/VGO0eeJS9V/25SRXPdRAdaGWs75OxYcjsHqSZV
-----END PRIVATE KEY-----";
const JWK_X: &str = "0PkEBI-N4yQRqQOEWIkd5ej53JHccAV2k3akWjwO5jo";
const JWK_Y: &str = "s2Yv9UY7R54lL1X_blJFc91EB1oZazvk7FhyOwepJlU";
const HOSTILE: &str = "Mallory <script>alert(1)</script> \u{202E}";

struct Stub;
impl PasswordHasher for Stub {
    fn hash(&self, password: &str) -> PasswordHash {
        PasswordHash(format!("h:{password}"))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        hash.0 == format!("h:{password}")
    }
}

/// Answers from a table and **records every request** (the URL).
#[derive(Default)]
struct Recording {
    routes: Mutex<Vec<(String, String)>>,
    seen: Mutex<Vec<String>>,
}
impl Recording {
    fn set(&self, url: &str, body: String) {
        self.routes.lock().unwrap().push((url.to_owned(), body));
    }
}
impl HttpsFetcher for Recording {
    fn fetch(&self, req: &FetchRequest) -> Result<Vec<u8>, FetchError> {
        self.seen.lock().unwrap().push(req.url.clone());
        self.routes
            .lock()
            .unwrap()
            .iter()
            .find(|(u, _)| *u == req.url)
            .map(|(_, b)| b.clone().into_bytes())
            .ok_or_else(|| FetchError::Transport(format!("no route for {}", req.url)))
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn id_token(nonce: &str) -> String {
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some("k1".into());
    let now = now_unix();
    encode(
        &h,
        // The provider volunteers everything: a hostile name, a preferred username, an email it says is verified.
        &serde_json::json!({
            "iss": ISSUER, "sub": "user-42", "aud": CLIENT, "exp": now + 300, "iat": now, "nonce": nonce,
            "name": HOSTILE, "preferred_username": "root", "given_name": "Eve", "email": "mallory@evil.example", "email_verified": true
        }),
        &EncodingKey::from_ec_pem(KEY_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn query(url: &str, name: &str) -> String {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|kv| {
            kv.split_once('=')
                .filter(|(k, _)| *k == name)
                .map(|(_, v)| v.to_owned())
        })
        .unwrap()
}

#[tokio::test]
async fn planeter_asks_the_provider_for_no_userinfo_and_keeps_no_name_and_no_email() {
    let db = SqliteDb::open_in_memory().unwrap();
    let accounts = Arc::new(SqliteAccountStore::new(db.clone()).unwrap());
    accounts
        .upsert(Account::local("alice", "Alice A", Some(Stub.hash("pw"))))
        .unwrap();
    accounts
        .link_oidc(ISSUER, "user-42", UserId::new("alice"))
        .unwrap();
    let before = accounts.get(&UserId::new("alice")).unwrap().unwrap();
    assert_eq!(before.email, None);

    let ok = |u: &str| format!("{ISSUER}{u}");
    let fetcher = Arc::new(Recording::default());
    // The discovery document **offers** a userinfo endpoint, and a hostile answer waits there.
    fetcher.set(
        &ok("/.well-known/openid-configuration"),
        format!(r#"{{"issuer":"{ISSUER}","authorization_endpoint":"{ISSUER}/auth","token_endpoint":"{ISSUER}/token","jwks_uri":"{ISSUER}/jwks","userinfo_endpoint":"{ISSUER}/userinfo"}}"#),
    );
    fetcher.set(&ok("/jwks"), format!(r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"{JWK_X}","y":"{JWK_Y}","kid":"k1","alg":"ES256","use":"sig"}}]}}"#));
    fetcher.set(&ok("/userinfo"), format!(r#"{{"sub":"user-42","name":"{HOSTILE}","preferred_username":"root","email":"mallory@evil.example"}}"#));
    let oidc = Arc::new(OidcProvider::new(
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: CLIENT.into(),
            client_secret: None,
            redirect_uri: "https://forge.example/login/oidc/callback".into(),
        },
        fetcher.clone(),
    ));
    let auth = Arc::new(Authenticator::new(
        accounts.clone(),
        Arc::new(SqliteCredentialStore::new(db.clone()).unwrap()),
        Arc::new(Stub),
        Arc::new(Sha256TokenHasher),
    ));
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("provider-held");
    let _ = std::fs::remove_dir_all(&root);
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(root, store));
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let state = AppState {
        read: Arc::new(ReadService::new(
            hosting,
            membership,
            Arc::new(NullAuditSink),
        )),
        gate: Arc::new(PrikkGate::default()),
        auth_gate: Arc::new(BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: Some(oidc),
        transport: None,
        creation: None,
    };
    let r = router(state);
    let begin = r
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login/oidc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let location = begin
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let st = query(&location, "state");
    fetcher.set(
        &ok("/token"),
        format!(
            r#"{{"id_token":"{}"}}"#,
            id_token(&query(&location, "nonce"))
        ),
    );
    let cb = r
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/login/oidc/callback?code=abc&state={st}"))
                .header(header::COOKIE, format!("planeter_oidc_state={st}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        cb.status(),
        StatusCode::SEE_OTHER,
        "alice signs in: {:?}",
        cb.headers()
    );
    // The session cookie is set, the page the person lands on shows their own display name and nothing the provider said.
    let session = cb
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("planeter_session="))
        .and_then(|c| c.split(';').next())
        .map(str::to_owned)
        .expect("a session cookie");
    let home = r
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let page = String::from_utf8_lossy(
        &axum::body::to_bytes(home.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .into_owned();
    assert!(page.contains("action=\"/logout\""), "signed in");
    for never in [
        "Mallory",
        "mallory",
        "script",
        "root",
        "Eve",
        "evil.example",
    ] {
        assert!(
            !page.contains(never),
            "the page shows `{never}` from the provider"
        );
    }

    // (a) The requests the provider received: discovery, the token exchange, the key set. **No userinfo, no second of anything.**
    let seen = fetcher.seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![
            ok("/.well-known/openid-configuration"),
            ok("/token"),
            ok("/jwks")
        ],
        "{seen:?}"
    );
    // (b) The account row is exactly as it was: the display name is planeter's, and **no email was stored**, though the ID token carried one
    // that the provider called verified.
    let after = accounts.get(&UserId::new("alice")).unwrap().unwrap();
    assert_eq!(after, before);
    assert_eq!(after.display_name, "Alice A");
    assert_eq!(after.email, None);
    let dump: String = {
        let conn = db.lock().unwrap();
        let mut rows = Vec::new();
        for t in ["accounts", "oidc_links", "sessions", "account_events"] {
            let Ok(mut st) = conn.prepare(&format!("SELECT * FROM {t}")) else {
                continue;
            };
            let n = st.column_count();
            let mapped = st
                .query_map([], |row| {
                    Ok((0..n)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap();
            rows.extend(mapped.map(Result::unwrap));
        }
        rows.join("\n")
    };
    for never in ["mallory", "evil.example", "Mallory", "preferred", "Eve"] {
        assert!(
            !dump.contains(never),
            "`{never}` reached the database:\n{dump}"
        );
    }
}

#[test]
fn the_id_token_claims_have_no_field_for_a_name_or_a_username() {
    // A scan of the source: the struct that reads the ID token's claims names no `name`, `preferred_username`, `given_name`, `family_name`,
    // `nickname` or `picture`. (A later change that adds one fails here and has to say why.)
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../planeter-auth/src/oidc.rs"
    ))
    .unwrap();
    let start = src.find("struct IdClaims {").expect("IdClaims");
    let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
    for field in [
        "name",
        "preferred_username",
        "given_name",
        "family_name",
        "nickname",
        "picture",
        "profile",
    ] {
        assert!(
            !body.lines().any(|l| l
                .trim_start()
                .trim_start_matches("pub ")
                .starts_with(&format!("{field}:"))),
            "IdClaims has a field `{field}`:\n{body}"
        );
    }
    // And the identity the provider returns carries no name either.
    let vi = &src[src.find("pub struct VerifiedIdentity {").unwrap()..];
    let vi = &vi[..vi.find("\n}\n").unwrap()];
    assert!(!vi.contains("name"), "{vi}");
}
