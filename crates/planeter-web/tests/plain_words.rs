//! RFC 012 D-4 (plain words), the page half: the in-place explanation of every assurance badge (T2) and the
//! verification table in plain words with the blocker as data (T3).
//!
//! The pages are rendered by the real router over a hosted repository; where a state cannot be made with a real
//! repository (a failed verification with blocked checks, a check that could not run, a queued patch) a stand-in
//! `prikk` answers `verify` or `show` with a fixed report and passes everything else to the real program. Needs
//! `prikk` on `PATH`; self-skips without it, like the other tests that read a repository.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InMemorySessionStore,
    InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepositoryStore,
    Visibility,
};
use planeter_web::catalog::{EN, Key, T};
use planeter_web::{AppState, ContentOrigin, router};

fn have_prikk() -> bool {
    std::process::Command::new("prikk")
        .arg("--version")
        .output()
        .is_ok()
}

fn scratch(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("plain-words-{name}"))
}

/// A public repository `alice/app` made by the real prikk; the router reads it through `binary` (the real
/// program when `None`, a stand-in script otherwise).
fn app(name: &str, script: Option<&str>) -> Router {
    let root = scratch(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(InMemoryRepositoryStore::new());
    let hosting = HostingService::new(
        root.join("repos"),
        store.clone() as Arc<dyn RepositoryStore>,
    )
    .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    hosting
        .create_repo(Owner::User("alice".into()), "app", Visibility::Public)
        .expect("create repo (needs prikk)");
    let mut reader = HostingService::new(root.join("repos"), store as Arc<dyn RepositoryStore>)
        .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    if let Some(body) = script {
        let path = root.join("prikk-standin.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        reader = reader.with_binary(path.display().to_string());
    }
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        Arc::new(reader),
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
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(InMemorySessionStore::new()),
        now_unix: || 1_000_000,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport: None,
        creation: None,
    })
}

async fn get(r: &Router, uri: &str) -> (StatusCode, String) {
    let resp = r
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    // `PLAIN_WORDS_DUMP=<dir>` saves every page the tests fetched (for the review request).
    if let Ok(dir) = std::env::var("PLAIN_WORDS_DUMP") {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name: String = uri
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Path::new(&dir).join(format!("{n:02}{name}.html")), &body).unwrap();
    }
    (status, body)
}

/// A stand-in program body: `verify` and `show` print the given files, everything else is the real prikk.
fn standin(verify: Option<&Path>, show: Option<&Path>) -> String {
    let mut s = String::from("case \"$*\" in\n");
    if let Some(v) = verify {
        s += &format!("  *verify*) cat '{}'; exit 0 ;;\n", v.display());
    }
    if let Some(v) = show {
        s += &format!("  *show*) cat '{}'; exit 0 ;;\n", v.display());
    }
    s + "  *) exec prikk \"$@\" ;;\nesac"
}

/// A fixed report on disk for the stand-in program to print.
fn fixture(name: &str, content: &str) -> PathBuf {
    let path = scratch(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn text(key: Key) -> String {
    T::new(EN).get(key)
}

/// The paragraphs of the page's explanation block, in order, as plain text; `None` when there is no block.
fn explanation(html: &str) -> Option<Vec<String>> {
    let start = html.find("<details class=\"explain\">")?;
    let end = start + html[start..].find("</details>")?;
    let block = &html[start..end];
    let summary = block
        .split("<summary>")
        .nth(1)?
        .split("</summary>")
        .next()?;
    assert_eq!(summary, text(Key::ExplainSummary), "the summary");
    Some(
        block
            .split("<p>")
            .skip(1)
            .map(|p| {
                p.split("</p>")
                    .next()
                    .unwrap()
                    .replace("&#39;", "'")
                    .replace("<strong>", "")
                    .replace("</strong><br>", " | ")
            })
            .collect(),
    )
}

/// The block follows the page's heading directly, and there is exactly one.
fn assert_directly_after_the_heading(html: &str, what: &str) {
    assert_eq!(
        html.matches("<details").count(),
        1,
        "{what}: one block\n{html}"
    );
    let at = html.find("<details class=\"explain\">").unwrap();
    assert!(
        html[..at].trim_end().ends_with("</h1>"),
        "{what}: the block is not directly after the heading:\n{html}"
    );
}

const EXPLAIN_SENTENCES: [Key; 6] = [
    Key::ExplainVerified,
    Key::ExplainSigned,
    Key::ExplainQueued,
    Key::ExplainFailed,
    Key::ExplainFailedHere,
    Key::ExplainUnavailable,
];

/// The page carries the sentence of its own state and none of the others: the meaning is asserted, not
/// only the words (a failed check must never be explained as a pass, nor a pass as a failure).
fn assert_only(html: &str, want: Key, what: &str) {
    let paragraphs = explanation(html).unwrap_or_else(|| panic!("{what}: no explanation block"));
    assert_eq!(paragraphs, [text(want)], "{what}");
    for other in EXPLAIN_SENTENCES.into_iter().filter(|k| *k != want) {
        assert!(
            !paragraphs.iter().any(|p| p.contains(&text(other))),
            "{what}: says {other:?}"
        );
    }
}

// ----------------------------------------------------------------------------------------------- T2 pages

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verified_pages_explain_what_verified_means() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let r = app("verified", None);
    for uri in ["/alice/app", "/alice/app/history", "/alice/app/verify"] {
        let (s, html) = get(&r, uri).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(html.contains("badge ok\">Verified</span>"), "{uri}: {html}");
        assert_directly_after_the_heading(&html, uri);
        assert_only(&html, Key::ExplainVerified, uri);
    }
    // What the sentence claims is what the badge needs: every check ran and none found a problem.
    assert!(text(Key::ExplainVerified).contains("all of its checks"));
    assert!(text(Key::ExplainVerified).contains("no problems"));
}

const FAILED_REPORT: &str = r#"{
  "schema_version": "verify-report-v1",
  "verdict": {"ok": false, "failed_conditions": [
    {"id": "stage-failure", "message": "repository verification did not complete every stage"},
    {"id": "item-failure", "message": "repository verification found at least one failed object <b>x</b>"}
  ]},
  "active_sessions": {"count": 2, "verified_count": 1},
  "stages": [
    {"stage": "objects", "status": "failed", "message": "integrity error: ref pointer index has a damaged entry"},
    {"stage": "refs", "status": "failed", "message": "integrity error: refs"},
    {"stage": "ref-update-schema-trust", "status": "not_evaluated", "blocked_by": "refs"},
    {"stage": "wal-replay", "status": "not_evaluated", "blocked_by": ""},
    {"stage": "commit-index", "status": "halted", "after": "refs"},
    {"stage": "lifecycle-cache", "status": "evaluated"}
  ]
}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_pages_explain_the_failure_and_never_call_it_a_pass() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let report = fixture("failed-report.json", FAILED_REPORT);
    let r = app("failed", Some(&standin(Some(&report), None)));
    for uri in ["/alice/app", "/alice/app/history", "/alice/app/verify"] {
        let (s, html) = get(&r, uri).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(
            html.contains("badge bad\">Verification failed</span>"),
            "{uri}: {html}"
        );
        assert!(!html.contains("badge ok"), "{uri}: {html}");
        assert_directly_after_the_heading(&html, uri);
        // The Verification page lists the findings itself, so its sentence says "below"; the other two point at it.
        let want = if uri.ends_with("/verify") {
            Key::ExplainFailedHere
        } else {
            Key::ExplainFailed
        };
        assert_only(&html, want, uri);
    }
    for key in [Key::ExplainFailed, Key::ExplainFailedHere] {
        assert!(text(key).contains("found problems"));
        assert!(!text(key).contains("no problems"));
    }
    assert!(text(Key::ExplainFailedHere).ends_with("They are listed below."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pages_whose_check_could_not_run_say_so_and_what_to_do() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    // `verify` exits with an error and prints nothing: not a verdict, an outage.
    let r = app(
        "unavailable",
        Some(
            "case \"$*\" in\n  *verify*) echo 'boom' >&2; exit 3 ;;\n  *) exec prikk \"$@\" ;;\nesac",
        ),
    );
    for uri in ["/alice/app", "/alice/app/history"] {
        let (s, html) = get(&r, uri).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert!(
            html.contains("badge bad\">Could not be checked now</span>"),
            "{uri}: {html}"
        );
        assert!(!html.contains("Verification failed"), "{uri}");
        assert_directly_after_the_heading(&html, uri);
        assert_only(&html, Key::ExplainUnavailable, uri);
    }
    assert_eq!(
        text(Key::ExplainUnavailable),
        "Try again in a little while."
    );
}

/// The change page for a `show` report with the given patches (`true` = queued).
async fn change_page(name: &str, queued: &[bool]) -> String {
    let patches: Vec<String> = queued
        .iter()
        .enumerate()
        .map(|(i, q)| {
            format!(
                r#"{{"patch_id": "{id}", "queued": {q}, "operations": [
                  {{"kind": "create-file", "paths": [{{"path": "f{i}.txt"}}],
                   "content": {{"kind": "create-file", "content": {{"kind": "text", "text": "hi"}}, "mode": 420}}}}]}}"#,
                id = format!("{:x}", i + 10).repeat(20),
            )
        })
        .collect();
    let show = fixture(
        &format!("show-{name}.json"),
        &format!(
            r#"{{"schema_version": "show-report-v1", "patches": [{}]}}"#,
            patches.join(",")
        ),
    );
    let r = app(name, Some(&standin(None, Some(&show))));
    let (s, html) = get(&r, "/alice/app/change/abc").await;
    assert_eq!(s, StatusCode::OK, "{html}");
    html
}

/// The badge labels on a page, in order (`<span class="badge …">label</span>`).
fn badges(html: &str) -> Vec<(String, String)> {
    html.split("<span class=\"badge")
        .skip(1)
        .map(|b| {
            let (class, rest) = b.split_once("\">").unwrap();
            (
                class.trim().to_owned(),
                rest.split("</span>").next().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_change_page_never_says_verified_and_explains_each_badge_once_and_the_two_terms() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let sealed = ("neutral".to_owned(), text(Key::AssuranceSealed));
    let queued = ("warn".to_owned(), text(Key::AssuranceQueued));
    let explain_sealed = format!(
        "{} | {}",
        text(Key::AssuranceSealed),
        text(Key::ExplainSigned)
    );
    let explain_queued = format!(
        "{} | {}",
        text(Key::AssuranceQueued),
        text(Key::ExplainQueued)
    );
    let terms = [text(Key::ExplainPatch), text(Key::ExplainRollback)];

    // (patches as queued?, the badges in order, the explanation paragraphs in order)
    // (name, patches as queued?, the badges in order, the explanation paragraphs in order)
    type Case<'a> = (&'a str, &'a [bool], Vec<(String, String)>, Vec<String>);
    let cases: [Case; 3] = [
        (
            "sealed",
            &[false, false],
            vec![sealed.clone(), sealed.clone()],
            vec![explain_sealed.clone()],
        ),
        (
            "queued",
            &[true],
            vec![queued.clone()],
            vec![explain_queued.clone()],
        ),
        (
            // two queued patches and one sealed: one paragraph per distinct badge, in order of first appearance
            "both",
            &[true, false, true],
            vec![queued.clone(), sealed.clone(), queued.clone()],
            vec![explain_queued, explain_sealed],
        ),
    ];
    for (name, patches, want_badges, mut want_explain) in cases {
        let html = change_page(&format!("change-{name}"), patches).await;
        assert_eq!(badges(&html), want_badges, "{name}: the badges");
        // No green badge and the word "Verified" is in no badge, whatever the patches are (IS-44).
        assert!(!html.contains("badge ok"), "{name}: {html}");
        assert!(!html.contains(">Verified<"), "{name}: {html}");
        assert!(
            !explanation(&html)
                .unwrap()
                .iter()
                .any(|p| p.contains(&text(Key::ExplainVerified)))
        );
        assert_directly_after_the_heading(&html, name);
        want_explain.extend(terms.clone());
        assert_eq!(
            explanation(&html).unwrap(),
            want_explain,
            "{name}: the explanation"
        );
    }
    // The meaning of "held here, not signed in" is the glossary's, word for word: held on this server, not
    // signed by a maintainer, not part of the verified record. Nobody "approved" it (D-50: that word belongs
    // to a reviewer's decision about a revision).
    assert_eq!(
        text(Key::ExplainQueued),
        "This patch is held on this server. A maintainer has not signed it into the project's \
         history, so it is not part of the verified record."
    );
    assert_eq!(text(Key::AssuranceQueued), "Held here, not signed in");
    // A sealed patch on this page is signed in; nothing here has checked the repository, so the sentence must
    // not claim a check (it points to the Verification page instead). The label is the plain, not the green, one.
    assert_eq!(text(Key::AssuranceSealed), "Signed into the history");
    let signed = text(Key::ExplainSigned);
    assert!(
        signed.contains("signed this patch into the project's history"),
        "{signed}"
    );
    assert!(
        !signed.contains("no problems") && !signed.contains("all of its checks"),
        "{signed}"
    );
    assert_eq!(text(Key::ExplainPatch), "A patch is one recorded edit.");
    assert_eq!(
        text(Key::ExplainRollback),
        "A rollback is a change that undoes an earlier change."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_change_api_says_sealed_for_a_sealed_patch_and_nothing_is_called_verified() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let show = fixture(
        "show-api.json",
        r#"{"schema_version": "show-report-v1", "patches": [
            {"patch_id": "aaaaaaaaaaaaaaaaaaaa", "queued": true, "operations": []},
            {"patch_id": "bbbbbbbbbbbbbbbbbbbb", "queued": false, "operations": []}]}"#,
    );
    let r = app("change-api", Some(&standin(None, Some(&show))));
    let (s, body) = get(&r, "/api/v1/repos/alice/app/change/abc").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["patches"][0]["assurance"], "forge-approved-unsealed",
        "{body}"
    );
    assert_eq!(v["patches"][1]["assurance"], "sealed", "{body}");
    assert!(!body.contains("prikk-verified"), "{body}");
}

const LOG_WITH_ROLLBACK: &str = r#"{"schema_version":"log-report-v1","repository":"/r/.prikk","ref":"heads/main",
  "current_branch":"main","blocks":[
  {"block_id":"b2","ref_state_id":"r2","update_seq":1,"kind":"Rollback","rollback_block":true,"parent_count":1,
   "patch_count":1,"rollback_patch_count":1,"required_attestation_count":0,
   "patch_messages":[{"patch_id":"p2","message":"undo it"}],"previous_ref_state_id":"r1"},
  {"block_id":"b1","ref_state_id":"r1","update_seq":0,"kind":"Root","rollback_block":false,"parent_count":0,
   "patch_count":1,"rollback_patch_count":0,"required_attestation_count":0,
   "patch_messages":[{"patch_id":"p1","message":"init"}],"previous_ref_state_id":null}]}"#;

const LOG_WITHOUT_ROLLBACK: &str = r#"{"schema_version":"log-report-v1","repository":"/r/.prikk","ref":"heads/main",
  "current_branch":"main","blocks":[
  {"block_id":"b1","ref_state_id":"r1","update_seq":0,"kind":"Root","rollback_block":false,"parent_count":0,
   "patch_count":1,"rollback_patch_count":0,"required_attestation_count":0,
   "patch_messages":[{"patch_id":"p1","message":"init"}],"previous_ref_state_id":null}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_history_page_explains_the_rollback_tag_once_and_only_when_it_shows_one() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let with = fixture("log-with-rollback.json", LOG_WITH_ROLLBACK);
    let without = fixture("log-without-rollback.json", LOG_WITHOUT_ROLLBACK);
    let standin_log = |f: &Path| {
        format!(
            "case \"$*\" in\n  *\"log --format\"*) cat '{}'; exit 0 ;;\n  *) exec prikk \"$@\" ;;\nesac",
            f.display()
        )
    };
    let r = app("history-rollback", Some(&standin_log(&with)));
    let (s, html) = get(&r, "/alice/app/history").await;
    assert_eq!(s, StatusCode::OK, "{html}");
    assert!(
        html.contains("<span class=\"badge warn\">rollback</span>"),
        "{html}"
    );
    // The badge's sentence, then the rollback one — once, although the log could hold many rollbacks.
    assert_eq!(
        explanation(&html).unwrap(),
        [text(Key::ExplainVerified), text(Key::ExplainRollback)]
    );
    // No rollback row, no rollback paragraph.
    let r = app("history-plain", Some(&standin_log(&without)));
    let (s, html) = get(&r, "/alice/app/history").await;
    assert_eq!(s, StatusCode::OK, "{html}");
    assert!(!html.contains("badge warn\">rollback"), "{html}");
    assert_eq!(explanation(&html).unwrap(), [text(Key::ExplainVerified)]);
}

// ----------------------------------------------------------------------------------------------- T3 table

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_verification_table_uses_plain_words_and_names_the_blocker_from_the_field() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let report = fixture("table-report.json", FAILED_REPORT);
    let r = app("table", Some(&standin(Some(&report), None)));
    let (s, html) = get(&r, "/alice/app/verify").await;
    assert_eq!(s, StatusCode::OK, "{html}");

    // The column is "Check", and the old words are gone from the page.
    assert!(html.contains("<th scope=\"col\">Check</th>"), "{html}");
    assert!(!html.contains(">Stage<"), "{html}");
    for old in ["not_evaluated", "halted", "evaluated", ">failed<"] {
        assert!(
            !html.contains(old),
            "the old word {old:?} is on the page:\n{html}"
        );
    }

    // One row per check: the check's name (prikk's), its status in plain words, prikk's own detail if any.
    let row = |name: &str| -> String {
        let at = html
            .find(&format!("<th scope=\"row\"><code>{name}</code></th>"))
            .unwrap_or_else(|| panic!("no row for {name}:\n{html}"));
        html[at..].split("</tr>").next().unwrap().to_owned()
    };
    assert!(row("objects").contains("<td>problem found</td>"));
    assert!(row("objects").contains("integrity error: ref pointer index has a damaged entry"));
    assert!(row("lifecycle-cache").contains("<td>completed</td>"));
    // The stopped check names what it stopped after, from the field; the English detail is not shown.
    let stopped = row("commit-index");
    assert!(
        stopped.contains("<td>stopped early (after refs)</td>"),
        "{stopped}"
    );
    assert_eq!(
        stopped.matches("after refs").count(),
        1,
        "said once: {stopped}"
    );
    // The blocked check names its blocker, taken from the field (not parsed out of the English `detail`).
    let blocked = row("ref-update-schema-trust");
    assert!(
        blocked.contains("<td>not run (blocked by refs)</td>"),
        "{blocked}"
    );
    assert!(
        !blocked.contains("blocked by refs</td>\n")
            && blocked.matches("blocked by refs").count() == 1,
        "the blocker is said once: {blocked}"
    );
    // A blocker prikk did not name: the plain status alone, never "blocked by " with nothing after it.
    let unnamed = row("wal-replay");
    assert!(unnamed.contains("<td>not run</td>"), "{unnamed}");
    assert!(!unnamed.contains("blocked by"), "{unnamed}");

    // The findings: a heading, prikk's ids and messages as prikk wrote them (escaped), nothing of ours.
    assert!(html.contains("<h2>What was found</h2>"), "{html}");
    assert!(html.contains("<li><code>stage-failure</code> repository verification did not complete every stage</li>"), "{html}");
    assert!(
        html.contains("found at least one failed object &#60;b&#62;x&#60;/b&#62;"),
        "{html}"
    );
    assert!(!html.contains("<b>x</b>"), "{html}");
    // The intro is as the glossary says, and nothing about "sessions" follows it (IS-49: prikk's report says
    // `{"count": 2, "verified_count": 1}` here, and the page says nothing of it).
    assert!(html.contains(&text(Key::VerifyIntro)));
    assert!(!html.to_lowercase().contains("session"), "{html}");
    assert!(!html.contains("of 2"), "{html}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_verified_repository_has_no_findings_heading_and_the_api_keeps_its_fields() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let r = app("clean", None);
    let (_, html) = get(&r, "/alice/app/verify").await;
    assert!(!html.contains("What was found"), "{html}");
    assert!(html.contains("<td>completed</td>"), "{html}");
    assert!(!html.to_lowercase().contains("session"), "{html}");

    // The API: `blocked_by` only on a not-run check; `detail` is as it always was.
    let report = fixture("api-report.json", FAILED_REPORT);
    let r = app("api", Some(&standin(Some(&report), None)));
    let (s, body) = get(&r, "/api/v1/repos/alice/app/verify").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    // IS-49: the verify body no longer carries the two session fields (prikk's report has `active_sessions`).
    assert!(
        v.get("verified_sessions").is_none() && v.get("total_sessions").is_none(),
        "{body}"
    );
    assert!(!body.contains("sessions"), "{body}");
    let stage = |name: &str| {
        v["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["stage"] == name)
            .unwrap_or_else(|| panic!("{name}: {body}"))
            .clone()
    };
    let blocked = stage("ref-update-schema-trust");
    assert_eq!(blocked["blocked_by"], "refs");
    assert_eq!(
        blocked["detail"], "blocked by refs",
        "the English detail is unchanged"
    );
    for name in ["objects", "commit-index", "lifecycle-cache"] {
        let s = stage(name);
        assert!(s.get("blocked_by").is_none(), "{name} has no blocker: {s}");
    }
    // `after` only on the stopped check; its English detail is unchanged.
    let halted = stage("commit-index");
    assert_eq!(halted["after"], "refs");
    assert_eq!(halted["detail"], "after refs");
    for name in ["objects", "ref-update-schema-trust", "lifecycle-cache"] {
        assert!(stage(name).get("after").is_none(), "{name} did not stop");
    }
}
