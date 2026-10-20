//! IS-28 / IS-29 (RFC 003 D-2, STD-3): the API's contract. One error shape; an OpenAPI document that
//! cannot drift from the router or the views; stable validators; and the cache and browser-policy
//! headers on every kind of response.
//!
//! The router is driven with `ServiceExt::oneshot`. Nothing here needs prikk: requests either never
//! open a repository (a hidden or absent one is refused first) or are served by a stand-in `prikk`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    InMemorySessionStore, InsecureStubHasher, PasswordHasher as _,
};
use planeter_core::read::{
    BranchView, ChangePatchView, FailedConditionView, HistoryEntry, OperationView,
    PatchMessageView, StageView, TagView,
};
use planeter_core::{
    Assurance, ChangeView, FileContentView, FileView, HistoryView, HostingService, NullAuditSink,
    ReadService, RefsView, TreeEntryView, TreeView, VerifyView,
};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, UserId, Visibility,
};
use planeter_web::catalog::EN;
use planeter_web::errors::ErrorCode;
use planeter_web::{AppState, BlockingGate, ContentOrigin, PrikkGate, TrustedProxies, router};

/// The routes the API is meant to have, written down independently of `api::paths`.
const EXPECTED_PATHS: [&str; 8] = [
    "/api/v1/repos/{owner}/{name}/history",
    "/api/v1/repos/{owner}/{name}/change/{target}",
    "/api/v1/repos/{owner}/{name}/file",
    "/api/v1/repos/{owner}/{name}/tree",
    "/api/v1/repos/{owner}/{name}/raw",
    "/api/v1/repos/{owner}/{name}/refs",
    "/api/v1/repos/{owner}/{name}/verify",
    "/api/v1/openapi.json",
];

fn state(trusted: &str) -> AppState {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("api-contract");
    let store = Arc::new(InMemoryRepositoryStore::new());
    // Admission measures `<repo>/.prikk` before any prikk call, so these stand-in repositories carry a
    // store directory; what fails afterwards is prikk itself (`/nonexistent/prikk`).
    std::fs::create_dir_all(root.join("never-opened/.prikk")).unwrap();
    for (id, name, vis) in [
        ("r1", "secret", Visibility::Private),
        ("r2", "open", Visibility::Public),
    ] {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(id),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility: vis,
                created_at: 0,
                prikk_format_version: None,
                path: root.join("never-opened"),
            })
            .unwrap();
    }
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
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account {
            user: UserId::new("alice"),
            display_name: "Alice".into(),
            email: None,
            password: Some(hasher.hash("pw")),
        })
        .unwrap();
    AppState {
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
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(TrustedProxies::parse(trusted).unwrap()),
        oidc: None,
    }
}

type Reply = (StatusCode, HeaderMap, Vec<u8>);

async fn send(r: &Router, req: Request<Body>) -> Reply {
    let resp = r.clone().oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
    (parts.status, parts.headers, bytes.to_vec())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

// ---------------------------------------------------------------- T1: one error shape

fn assert_error(reply: &Reply, code: ErrorCode) {
    let (status, headers, body) = reply;
    assert_eq!(*status, code.status(), "{}", String::from_utf8_lossy(body));
    assert_eq!(header_str(headers, "content-type"), "application/json");
    let v: Value = serde_json::from_slice(body).expect("a JSON error body");
    assert_eq!(v["code"], code.code());
    assert_eq!(v["message"], code.message());
    assert_eq!(v["hint"], code.hint());
    assert_eq!(
        v.as_object().unwrap().len(),
        3,
        "exactly code, message, hint"
    );
}

#[tokio::test]
async fn not_found_is_identical_for_a_hidden_and_an_absent_repository() {
    let r = router(state(""));
    let hidden = send(&r, get("/api/v1/repos/alice/secret/history")).await;
    let absent = send(&r, get("/api/v1/repos/alice/no-such-repo/history")).await;
    assert_error(&hidden, ErrorCode::NotFound);
    assert_error(&absent, ErrorCode::NotFound);
    assert_eq!(hidden.0, absent.0);
    assert_eq!(hidden.1, absent.1, "headers must be identical");
    assert_eq!(hidden.2, absent.2, "bodies must be byte-identical");
    // And for every other read route.
    for tail in [
        "refs",
        "verify",
        "tree",
        "file?path=a",
        "raw?path=a",
        "change/abc",
    ] {
        let a = send(&r, get(&format!("/api/v1/repos/alice/secret/{tail}"))).await;
        let b = send(&r, get(&format!("/api/v1/repos/alice/no-such-repo/{tail}"))).await;
        assert_eq!((&a.0, &a.1, &a.2), (&b.0, &b.1, &b.2), "{tail}");
    }
}

#[tokio::test]
async fn a_malformed_query_is_invalid_request() {
    let r = router(state(""));
    // `limit` is not a number; `path` is required and missing.
    let bad_limit = send(&r, get("/api/v1/repos/alice/open/history?limit=lots")).await;
    let no_path = send(&r, get("/api/v1/repos/alice/open/file")).await;
    let bad_all = send(&r, get("/api/v1/repos/alice/open/refs?all=maybe")).await;
    for reply in [&bad_limit, &no_path, &bad_all] {
        assert_error(reply, ErrorCode::InvalidRequest);
    }
}

#[tokio::test]
async fn unavailable_is_503_without_tool_output_or_paths() {
    // The repository is public and readable, but its prikk binary cannot be run: `unavailable`.
    let r = router(state(""));
    let reply = send(&r, get("/api/v1/repos/alice/open/history")).await;
    assert_error(&reply, ErrorCode::Unavailable);
    let text = String::from_utf8_lossy(&reply.2);
    for leak in ["nonexistent", "never-opened", "No such file", "spawn", "/"] {
        assert!(!text.contains(leak), "{leak:?} leaked into {text}");
    }
}

#[tokio::test]
async fn unknown_api_routes_and_wrong_methods_use_the_same_shape() {
    let r = router(state(""));
    assert_error(
        &send(&r, get("/api/v1/nonsense")).await,
        ErrorCode::NotFound,
    );
    let post = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/open/history")
        .body(Body::empty())
        .unwrap();
    let reply = send(&r, post).await;
    assert_error(&reply, ErrorCode::MethodNotAllowed);
    assert_eq!(header_str(&reply.1, "allow"), "GET");
    assert_eq!(reply.0, StatusCode::METHOD_NOT_ALLOWED);
    // A non-API unknown path keeps the UI's bare 404.
    let ui = send(&r, get("/definitely/not/a/page/at/all/x")).await;
    assert_eq!(ui.0, StatusCode::NOT_FOUND);
}

#[test]
fn every_code_has_one_plain_sentence_and_the_shape_is_stable() {
    let mut seen = std::collections::HashSet::new();
    for c in ErrorCode::ALL {
        assert!(seen.insert(c.code()), "duplicate code");
        assert!(
            c.code()
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch == '_')
        );
        for text in [c.message(), c.hint()] {
            assert!(text.ends_with('.'), "{text:?}");
            assert_eq!(text.matches(". ").count(), 0, "one sentence: {text:?}");
        }
    }
    assert_eq!(seen.len(), 8);
}

#[test]
fn too_large_has_one_code_and_a_hint_per_cause() {
    // The raw route's hint is one plain sentence, like the others, and differs from the default one.
    let on_raw: Value =
        serde_json::from_str(&ErrorCode::TooLarge.body_for(ErrorCode::TOO_LARGE_RAW, EN)).unwrap();
    let raw = on_raw["hint"].as_str().unwrap().to_owned();
    assert!(
        raw.ends_with('.') && raw.matches(". ").count() == 0,
        "{raw:?}"
    );
    assert_ne!(raw, ErrorCode::TooLarge.hint());
    let default: Value = serde_json::from_str(&ErrorCode::TooLarge.body()).unwrap();
    for v in [&default, &on_raw] {
        assert_eq!(v["code"], "too_large");
        assert_eq!(v["message"], ErrorCode::TooLarge.message());
        assert_eq!(v.as_object().unwrap().len(), 3);
    }
    assert_eq!(default["hint"], ErrorCode::TooLarge.hint());
    assert_eq!(
        on_raw["hint"],
        "This server does not serve files that large; ask the operator."
    );
    // No other code is given a second hint: the plain body is the code's own.
    for c in ErrorCode::ALL {
        let v: Value = serde_json::from_str(&c.body()).unwrap();
        assert_eq!(v["hint"], c.hint());
    }
}

// ---------------------------------------------------------------- T2: the OpenAPI document

async fn document(r: &Router) -> Value {
    let (s, _, body) = send(r, get("/api/v1/openapi.json")).await;
    assert_eq!(s, StatusCode::OK);
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn the_document_lists_exactly_the_routes_the_router_serves() {
    let r = router(state(""));
    let doc = document(&r).await;
    let mut doc_paths: Vec<&str> = doc["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    doc_paths.sort_unstable();
    let mut expected = EXPECTED_PATHS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        doc_paths, expected,
        "the document and the expected routes must agree"
    );
    assert_eq!(planeter_web::api::paths::ALL.len(), EXPECTED_PATHS.len());

    // Every documented path is really registered: a wrong method answers 405, an unknown path 404.
    for path in EXPECTED_PATHS {
        let concrete = path
            .replace("{owner}", "alice")
            .replace("{name}", "open")
            .replace("{target}", "abc");
        let req = Request::builder()
            .method("DELETE")
            .uri(&concrete)
            .body(Body::empty())
            .unwrap();
        let (status, ..) = send(&r, req).await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{path} is not registered"
        );
    }
    let (status, ..) = send(&r, get("/api/v1/repos/alice/open/not-a-route")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_document_describes_parameters_responses_security_and_the_error_schema() {
    let r = router(state(""));
    let doc = document(&r).await;
    assert_eq!(doc["info"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(doc["components"]["securitySchemes"]["bearerToken"].is_object());
    assert_eq!(
        doc["components"]["securitySchemes"]["sessionCookie"]["in"],
        "cookie"
    );

    let history = &doc["paths"]["/api/v1/repos/{owner}/{name}/history"]["get"];
    let limit = history["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "limit")
        .expect("limit is documented");
    assert_eq!(limit["schema"]["default"], 50);
    assert_eq!(limit["schema"]["maximum"], 200);
    assert_eq!(limit["required"], false);
    for status in ["200", "304", "400", "404", "422", "500", "503"] {
        assert!(history["responses"][status].is_object(), "{status}");
    }
    for status in ["400", "404", "422", "500", "503"] {
        assert_eq!(
            history["responses"][status]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/Error"
        );
    }
    assert!(history["responses"]["503"]["headers"]["Retry-After"].is_object());
    let file = &doc["paths"]["/api/v1/repos/{owner}/{name}/file"]["get"];
    let path = file["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "path")
        .unwrap();
    assert_eq!(path["required"], true);
    // Views never omit a field, so every declared property of every object schema is required.
    for (name, schema) in doc["components"]["schemas"].as_object().unwrap() {
        if schema["type"] != "object" {
            continue;
        }
        let mut props: Vec<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        props.sort_unstable();
        required.sort_unstable();
        assert_eq!(
            props, required,
            "{name}: properties and required must agree"
        );
    }
    // The error schema's codes are the real codes.
    let codes = doc["components"]["schemas"]["Error"]["properties"]["code"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(codes.len(), ErrorCode::ALL.len());
}

/// Samples of every view, serialized and compared with the declared schema: every `required` field is
/// present, and nothing is serialized that the schema does not declare.
#[tokio::test]
async fn every_view_serializes_to_exactly_the_declared_fields() {
    let r = router(state(""));
    let doc = document(&r).await;
    let schemas = &doc["components"]["schemas"];

    let samples: Vec<(&str, Value)> = vec![
        (
            "HistoryView",
            serde_json::to_value(HistoryView {
                ref_name: "heads/main".into(),
                current_branch: Some("heads/main".into()),
                entries: vec![HistoryEntry {
                    block_id: "b".into(),
                    ref_state_id: "r".into(),
                    kind: "Root".into(),
                    rollback_block: false,
                    patch_count: 1,
                    messages: vec![PatchMessageView {
                        patch_id: "p".into(),
                        message: "m".into(),
                    }],
                }],
            })
            .unwrap(),
        ),
        (
            "ChangeView",
            serde_json::to_value(ChangeView {
                patches: vec![ChangePatchView {
                    patch_id: "p".into(),
                    assurance: Assurance::ForgeApprovedUnsealed,
                    operations: vec![OperationView {
                        kind: "create-file".into(),
                        paths: vec!["a".into()],
                    }],
                }],
            })
            .unwrap(),
        ),
        (
            "FileView",
            serde_json::to_value(FileView {
                ref_name: "heads/main".into(),
                path: "a".into(),
                content: FileContentView::Binary {
                    blob_id: "x".into(),
                    size: 3,
                },
            })
            .unwrap(),
        ),
        (
            "TreeView",
            serde_json::to_value(TreeView {
                point: "heads/main".into(),
                prefix: None,
                entries: vec![TreeEntryView {
                    path: "a".into(),
                    kind: "file".into(),
                    encoding: None,
                    mode: 0o644,
                    size: 3,
                    content_id: Some("c".into()),
                }],
            })
            .unwrap(),
        ),
        (
            "RefsView",
            serde_json::to_value(RefsView {
                current_branch: None,
                branches: vec![BranchView {
                    ref_name: "heads/main".into(),
                    ref_state_id: "r".into(),
                    closed: false,
                    current: true,
                }],
                tags: vec![TagView {
                    ref_name: "tags/v1".into(),
                    target_block_id: "b".into(),
                }],
            })
            .unwrap(),
        ),
        (
            "VerifyView",
            serde_json::to_value(VerifyView {
                ok: false,
                verified_sessions: 1,
                total_sessions: 2,
                failed_conditions: vec![FailedConditionView {
                    id: "stage-failure".into(),
                    message: "m".into(),
                }],
                stages: vec![StageView {
                    stage: "objects".into(),
                    status: "evaluated".into(),
                    detail: None,
                }],
            })
            .unwrap(),
        ),
    ];

    fn resolve<'a>(schemas: &'a Value, schema: &'a Value) -> &'a Value {
        match schema["$ref"].as_str() {
            Some(r) => &schemas[r.rsplit('/').next().unwrap()],
            None => schema,
        }
    }

    fn check(schemas: &Value, schema: &Value, sample: &Value, at: &str) -> Result<(), String> {
        let schema = resolve(schemas, schema);
        if let Some(one_of) = schema["oneOf"].as_array() {
            return if one_of
                .iter()
                .any(|alt| check(schemas, alt, sample, at).is_ok())
            {
                Ok(())
            } else {
                Err(format!("{at}: no oneOf alternative matches {sample}"))
            };
        }
        match schema["type"].as_str() {
            Some("object") => {
                let obj = sample.as_object().ok_or(format!("{at}: not an object"))?;
                let props = schema["properties"].as_object().ok_or("no properties")?;
                for req in schema["required"].as_array().into_iter().flatten() {
                    if !obj.contains_key(req.as_str().unwrap()) {
                        return Err(format!("{at}: missing {req}"));
                    }
                }
                // The views never omit a field (an absent value is `null`), so a declared property
                // that is not serialized is a document that over-promises.
                for k in props.keys() {
                    if !obj.contains_key(k) {
                        return Err(format!("{at}: declared but not serialized: {k}"));
                    }
                }
                for (k, v) in obj {
                    let p = props.get(k).ok_or(format!("{at}: undeclared field {k}"))?;
                    check(schemas, p, v, &format!("{at}.{k}"))?;
                }
                Ok(())
            }
            Some("array") => {
                let items = sample.as_array().ok_or(format!("{at}: not an array"))?;
                for (i, item) in items.iter().enumerate() {
                    check(schemas, &schema["items"], item, &format!("{at}[{i}]"))?;
                }
                Ok(())
            }
            Some("string") => {
                let nullable_null = sample.is_null() && schema["nullable"] == true;
                if !(sample.is_string() || nullable_null) {
                    return Err(format!("{at}: not a string: {sample}"));
                }
                match schema["enum"].as_array() {
                    Some(e) if !e.contains(sample) => Err(format!("{at}: {sample} not in {e:?}")),
                    _ => Ok(()),
                }
            }
            Some("boolean") if sample.is_boolean() => Ok(()),
            Some("integer") if sample.is_u64() || sample.is_i64() => Ok(()),
            other => Err(format!("{at}: {sample} does not fit {other:?}")),
        }
    }

    for (name, sample) in &samples {
        check(
            schemas,
            &json!({"$ref": format!("#/components/schemas/{name}")}),
            sample,
            name,
        )
        .unwrap();
    }
    // The other FileContent shapes, and the Assurance values, are also declared.
    for content in [
        serde_json::to_value(FileContentView::Text { text: "t".into() }).unwrap(),
        serde_json::to_value(FileContentView::Binary {
            blob_id: "b".into(),
            size: 2,
        })
        .unwrap(),
        serde_json::to_value(FileContentView::Opaque { size: 1 }).unwrap(),
    ] {
        check(
            schemas,
            &json!({"$ref": "#/components/schemas/FileContent"}),
            &content,
            "FileContent",
        )
        .unwrap();
    }
    for a in [
        Assurance::PrikkVerified,
        Assurance::ForgeApprovedUnsealed,
        Assurance::Unverified,
    ] {
        let schema = json!({"$ref": "#/components/schemas/Assurance"});
        check(
            schemas,
            &schema,
            &serde_json::to_value(a).unwrap(),
            "Assurance",
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------- T3: stable validators

#[test]
fn the_etag_is_specified_stable_and_independent_of_key_order() {
    let a: Value = serde_json::from_str(r#"{"b":[1,{"y":2,"x":1}],"a":"é","c":null}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"c":null,"a":"é","b":[1,{"x":1,"y":2}]}"#).unwrap();
    let ea = planeter_web::api::etag_of_json(&a);
    assert_eq!(
        ea,
        planeter_web::api::etag_of_json(&b),
        "key order must not matter"
    );
    // Specified output: a quoted strong validator, 128 bits of SHA-256 in hex.
    assert!(ea.starts_with('"') && ea.ends_with('"') && !ea.starts_with("W/"));
    let hex = &ea[1..ea.len() - 1];
    assert_eq!(hex.len(), 32);
    assert!(
        hex.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    // A pinned vector: the canonical text is `{"a":"é","b":[1,{"x":1,"y":2}],"c":null}`.
    let canonical = r#"{"a":"é","b":[1,{"x":1,"y":2}],"c":null}"#;
    assert_eq!(hex, &planeter_auth::sha256_hex(canonical.as_bytes())[..32]);
    assert_eq!(
        planeter_auth::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // A different value gives a different validator.
    assert_ne!(ea, planeter_web::api::etag_of_json(&json!({"a": 1})));
}

// ---------------------------------------------------------------- T4: cache and browser policy

/// `varies_by_language`: HTML pages and API error bodies are written in the caller's language, so they also
/// `Vary: Accept-Language`; a successful API document does not depend on it.
fn assert_dynamic(h: &HeaderMap, what: &str, varies_by_language: bool) {
    assert_eq!(
        header_str(h, "cache-control"),
        "private, no-cache",
        "{what}"
    );
    let vary = if varies_by_language {
        "Cookie, Authorization, Accept-Language"
    } else {
        "Cookie, Authorization"
    };
    assert_eq!(header_str(h, "vary"), vary, "{what}");
    assert_eq!(
        h.contains_key("content-language"),
        varies_by_language,
        "{what}"
    );
    assert_eq!(
        header_str(h, "permissions-policy"),
        "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        "{what}"
    );
}

#[tokio::test]
async fn dynamic_responses_are_private_no_cache_and_vary() {
    let r = router(state(""));
    // A UI page that mints no cookie (the CSRF cookie is present), an API success, and API errors.
    let ui = send(
        &r,
        Request::builder()
            .uri("/")
            .header(header::COOKIE, "planeter_csrf=t")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(ui.0, StatusCode::OK);
    assert_dynamic(&ui.1, "ui home", true);
    let api = send(&r, get("/api/v1/openapi.json")).await;
    assert_dynamic(&api.1, "openapi", false);
    for (uri, what) in [
        ("/api/v1/repos/alice/secret/history", "404"),
        ("/api/v1/repos/alice/open/history?limit=x", "400"),
        ("/api/v1/repos/alice/open/history", "503"),
        ("/api/v1/nonsense", "unknown api route"),
    ] {
        let e = send(&r, get(uri)).await;
        assert_dynamic(&e.1, what, true);
    }
}

#[tokio::test]
async fn credential_flows_are_no_store() {
    let r = router(state(""));
    // The first visit mints a CSRF cookie: the page that sets it is no-store.
    let first = send(&r, get("/login")).await;
    assert!(first.1.get(header::SET_COOKIE).is_some());
    assert_eq!(header_str(&first.1, "cache-control"), "no-store");
    let csrf = first
        .1
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| c.strip_prefix("planeter_csrf="))
        .map(|c| c.split(';').next().unwrap().to_owned())
        .unwrap();

    // A failed and a successful sign-in, and a sign-out.
    let post = |form: String| {
        Request::builder()
            .method("POST")
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, format!("planeter_csrf={csrf}"))
            .body(Body::from(form))
            .unwrap()
    };
    let bad = send(
        &r,
        post(format!("csrf={csrf}&username=alice&password=nope")),
    )
    .await;
    assert_eq!(bad.0, StatusCode::UNAUTHORIZED);
    assert_eq!(header_str(&bad.1, "cache-control"), "no-store");
    let good = send(&r, post(format!("csrf={csrf}&username=alice&password=pw"))).await;
    assert_eq!(good.0, StatusCode::SEE_OTHER);
    assert_eq!(header_str(&good.1, "cache-control"), "no-store");
    let out = send(
        &r,
        Request::builder()
            .method("POST")
            .uri("/logout")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, format!("planeter_csrf={csrf}"))
            .body(Body::from(format!("csrf={csrf}")))
            .unwrap(),
    )
    .await;
    assert_eq!(header_str(&out.1, "cache-control"), "no-store");
}

#[tokio::test]
async fn the_stylesheet_is_public_cacheable_and_not_vary() {
    let r = router(state(""));
    let css = send(&r, get("/static/app.css")).await;
    assert_eq!(css.0, StatusCode::OK);
    assert_eq!(header_str(&css.1, "cache-control"), "public, max-age=3600");
    assert!(css.1.get("vary").is_none());
}

#[tokio::test]
async fn hsts_is_sent_only_when_trusted_proxies_are_configured() {
    let none = router(state(""));
    let behind = router(state("10.0.0.0/8"));
    let plain = send(&none, get("/api/v1/openapi.json")).await;
    assert!(plain.1.get("strict-transport-security").is_none());
    let tls = send(&behind, get("/api/v1/openapi.json")).await;
    assert_eq!(
        header_str(&tls.1, "strict-transport-security"),
        "max-age=15552000"
    );
    // Errors and the UI too, and never includeSubDomains or preload.
    let err = send(&behind, get("/api/v1/nonsense")).await;
    let v = header_str(&err.1, "strict-transport-security");
    assert_eq!(v, "max-age=15552000");
    assert!(!v.contains("includeSubDomains") && !v.contains("preload"));
}

/// The grep-gate: routes are registered in one place, with the shared path constants.
#[test]
fn api_routes_are_registered_only_through_the_shared_paths() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api.rs");
    let text = std::fs::read_to_string(src).unwrap();
    let route_lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.contains(".route("))
        .collect();
    assert_eq!(route_lines.len(), 8, "{route_lines:?}");
    assert!(
        route_lines.iter().all(|l| l.contains("paths::")),
        "a route is registered with a literal path: {route_lines:?}"
    );
}

// ---------------------------------------------------------------- IS-33 refinements

#[test]
fn each_content_kind_serializes_exactly_as_documented() {
    assert_eq!(
        serde_json::to_value(FileContentView::Text { text: "x".into() }).unwrap(),
        json!({"kind": "text", "text": "x"})
    );
    assert_eq!(
        serde_json::to_value(FileContentView::Binary {
            blob_id: "b".into(),
            size: 3
        })
        .unwrap(),
        json!({"kind": "binary", "blob_id": "b", "size": 3})
    );
    assert_eq!(
        serde_json::to_value(FileContentView::Opaque { size: 9 }).unwrap(),
        json!({"kind": "opaque", "size": 9})
    );
}

#[tokio::test]
async fn the_content_schema_is_a_kind_discriminated_union_with_no_not_found() {
    let r = router(state(""));
    let doc = document(&r).await;
    let content = &doc["components"]["schemas"]["FileContent"];
    assert_eq!(content["discriminator"]["propertyName"], "kind");
    let mapping = content["discriminator"]["mapping"].as_object().unwrap();
    let mut kinds: Vec<&str> = mapping.keys().map(String::as_str).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["binary", "opaque", "text"]);
    assert_eq!(content["oneOf"].as_array().unwrap().len(), 3);
    assert!(
        !doc.to_string().contains("NotFound"),
        "the old marker is gone"
    );
}

#[tokio::test]
async fn the_raw_operation_names_the_content_origin_and_only_it() {
    let r = router(state(""));
    let doc = document(&r).await;
    let raw = &doc["paths"]["/api/v1/repos/{owner}/{name}/raw"];
    assert_eq!(
        raw["servers"],
        json!([{
            "url": "https://raw.planeter.example",
            "description": "The content origin: the only host that serves raw file bytes."
        }])
    );
    assert!(
        raw["get"]["description"]
            .as_str()
            .unwrap()
            .contains("raw_elsewhere")
    );
    assert!(
        raw["get"]["responses"]["404"]["description"]
            .as_str()
            .unwrap()
            .contains("raw_elsewhere")
    );
    // No other path item carries `servers`, and the origin appears nowhere else in the document.
    for (path, item) in doc["paths"].as_object().unwrap() {
        if !path.ends_with("/raw") {
            assert!(item.get("servers").is_none(), "{path}");
        }
    }
    assert_eq!(doc.to_string().matches("raw.planeter.example").count(), 1);
}

#[tokio::test]
async fn every_operation_documents_method_not_allowed_with_allow() {
    let r = router(state(""));
    let doc = document(&r).await;
    for (path, item) in doc["paths"].as_object().unwrap() {
        if path.ends_with("openapi.json") {
            continue;
        }
        let resp = &item["get"]["responses"]["405"];
        assert!(
            resp["description"]
                .as_str()
                .unwrap()
                .contains("method_not_allowed"),
            "{path}"
        );
        assert!(resp["headers"]["Allow"].is_object(), "{path}");
    }
}

/// The 404 for the raw route on the wrong host depends on nothing about any repository.
#[tokio::test]
async fn raw_elsewhere_is_identical_for_hidden_absent_and_public_repositories() {
    let r = router(state(""));
    let ask = |repo: &str| {
        Request::builder()
            .uri(format!("/api/v1/repos/alice/{repo}/raw?path=a.txt"))
            .header(header::HOST, "app.planeter.example")
            .body(Body::empty())
            .unwrap()
    };
    let hidden = send(&r, ask("secret")).await;
    let absent = send(&r, ask("no-such-repo")).await;
    let public = send(&r, ask("open")).await;
    assert_error(&hidden, ErrorCode::RawElsewhere);
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert_eq!(
        (&hidden.0, &hidden.1, &hidden.2),
        (&absent.0, &absent.1, &absent.2)
    );
    assert_eq!(
        (&hidden.0, &hidden.1, &hidden.2),
        (&public.0, &public.1, &public.2)
    );
    // The content origin appears nowhere in the response.
    assert!(!String::from_utf8_lossy(&hidden.2).contains("planeter.example"));
    // (The app CSP already names the content origin on every response; no other header may.)
    for (name, value) in &hidden.1 {
        if name != "content-security-policy" {
            assert!(
                !value
                    .to_str()
                    .unwrap_or("")
                    .contains("raw.planeter.example"),
                "{name}"
            );
        }
    }
    // And it is not `not_found`: a client can tell it to look elsewhere.
    let v: Value = serde_json::from_slice(&hidden.2).unwrap();
    assert_eq!(v["code"], "raw_elsewhere");
}

/// A view that cannot be serialized is `internal`, never an ETag over `null`.
#[tokio::test]
async fn an_unserializable_view_is_an_internal_error_not_a_null_etag() {
    use planeter_web::api::respond_for_test;
    struct Broken;
    impl serde::Serialize for Broken {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("boom"))
        }
    }
    let reply = respond_for_test(&state(""), Broken);
    assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.headers().get(header::ETAG).is_none());
    let bytes = axum::body::to_bytes(reply.into_body(), 1 << 16)
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), ErrorCode::Internal.body().as_bytes());
}
