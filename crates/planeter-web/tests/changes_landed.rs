//! RFC 008 D-11 (handoff 4, T1): **the view of landed work**, over the real router, **real prikk 0.58.0** (the tests say why they skip) and
//! the real delivery engine. The world is the delivery engine's own (`planeter-transport/tests/support`): the measurement's archives and
//! scenario (P1 one patch, P2 one patch no delivery carries, P3 two patches of which the delivery carries the first), a hosted repository made
//! as creation makes one, proposals made through the propose service, SQLite stores. The sandbox is off here (the real server's tests use the
//! real one).
//!
//! What is shown: a revision whose patches are all in blocks of `heads/main` **in one run** is the files of that run
//! (`compared.landed: true`), equal to a `diff -r` of the two trees prikk extracts; one that is partly landed is `409 partly_landed` (or, while
//! its claims are all still pending, the ordinary view); one that is landed in no single run is `not_one_change`; and what the cache holds for a
//! pending revision is never read for a landed one. `CHANGES_LANDED_DUMP=<dir>` saves the real JSON and the pages.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;
#[allow(dead_code)]
#[path = "structure/dom.rs"]
mod dom;
#[allow(dead_code)]
#[path = "structure/rules.rs"]
mod rules;
#[path = "../../planeter-transport/tests/support/delivery_world.rs"]
mod world;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{NullAuditSink, ReadService, RepoScope, Scope, ScopeAccess};
use planeter_prikk::PrikkRepo as _;
use planeter_store::{
    ChangeStore as _, InMemoryMembershipStore, MembershipStore, Owner, ProposalPolicy, RepoId,
    RepositoryRecord, RepositoryStore, Role, Subject, UserId, Visibility,
};
use planeter_transport::{ChangesConfig, ChangesService, ProposalConfig, ProposalService};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};
use world::*;

macro_rules! need_prikk {
    () => {
        let has = |t: &str| {
            Command::new(t)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !prikk_ok() || !has("prlimit") {
            eprintln!("skipping: prikk or prlimit is not on PATH");
            return;
        }
    };
}

struct Site {
    router: Router,
    svc: Services,
}

fn site(name: &str, tweak: impl FnOnce(&mut ChangesConfig)) -> Site {
    let fx = fixtures();
    let base = scratch(&format!("landed-{name}"));
    create_live(&base, &fx.h0, &[&fx.m1, &fx.m2]);
    let svc = open_full(
        &base,
        None,
        planeter_prikk::Sandbox::Unconfined,
        None,
        |_| {},
    );
    // a private repository (for the hidden-repository answers): the record is enough
    svc.repos
        .create(RepositoryRecord {
            repo_id: RepoId::new("hd01"),
            owner: Owner::User("alice".into()),
            name: "secret".into(),
            visibility: Visibility::Private,
            created_at: 0,
            prikk_format_version: None,
            proposals: ProposalPolicy::Members,
            path: base.join("root/hd/hd01"),
        })
        .unwrap();
    let membership = Arc::new(InMemoryMembershipStore::new());
    for (user, role) in [("alice", Role::Admin), ("bob", Role::Read)] {
        membership
            .grant_repo_role(Subject::User(UserId::new(user)), RepoId::new(REPO), role)
            .unwrap();
    }
    membership
        .grant_repo_role(
            Subject::User(UserId::new("alice")),
            RepoId::new("hd01"),
            Role::Admin,
        )
        .unwrap();
    let read = Arc::new(ReadService::new(
        svc.hosting.clone(),
        membership.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user) in [
        ("tok-alice", "alice"),
        ("tok-bob", "bob"),
        ("tok-eve", "eve"),
    ] {
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&*hasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access: ScopeAccess::Full,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let auth = Arc::new(Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        creds,
        hasher.clone(),
        hasher,
    ));
    let proposals = Arc::new(
        ProposalService::new(
            svc.hosting.clone(),
            svc.changes.clone() as Arc<dyn planeter_store::ChangeStore>,
            svc.export.clone(),
            svc.artifacts.clone(),
            svc.metrics.clone(),
            ProposalConfig::default(),
        )
        .with_clock(real_clock),
    );
    let mut cfg = ChangesConfig {
        spool: Some(base.join("spool")),
        ..ChangesConfig::default()
    };
    tweak(&mut cfg);
    let changes = Arc::new(ChangesService::new(
        svc.hosting.clone(),
        svc.metrics.clone(),
        cfg,
    ));
    let transport = Some(Arc::new(TransportRuntime {
        export: svc.export.clone(),
        propose: Some(svc.propose.clone()),
        recovery: Some(svc.recovery.clone()),
        proposals: Some(proposals),
        reviews: None,
        changes: Some(changes),
        deliveries: Some(svc.delivery.clone()),
        ip_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
        principal_limit: RateLimiter::new(100_000, Duration::from_secs(60)),
    }));
    let state = AppState {
        read,
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(planeter_auth::InMemorySessionStore::new()),
        now_unix: real_clock,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport,
        creation: None,
    };
    Site {
        router: router(state),
        svc,
    }
}

struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

async fn get(s: &Site, uri: &str, token: Option<&str>, lang: Option<&str>) -> Reply {
    let mut b = Request::builder()
        .method("GET")
        .uri(uri)
        .header("host", "forge.example");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    if let Some(l) = lang {
        b = b.header("accept-language", l);
    }
    let resp = s
        .router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 30)
        .await
        .unwrap()
        .to_vec();
    Reply { status, body }
}

fn api(repo: &str, id: &str, seq: u32) -> String {
    format!("/api/v1/repos/alice/{repo}/proposals/{id}/revisions/{seq}/changes")
}

fn changes_of(s: &Site, id: &str) -> impl std::future::Future<Output = Reply> {
    let uri = api("forge", id, 1);
    async move { get(s, &uri, Some("tok-alice"), None).await }
}

fn dump(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("CHANGES_LANDED_DUMP") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(Path::new(&dir).join(name), bytes);
    }
}

fn counter(s: &Site, name: &str) -> u64 {
    s.svc
        .metrics
        .snapshot()
        .changes
        .get(name)
        .copied()
        .unwrap_or(0)
}

/// Deliver an archive through the engine and confirm it: the live repository's branch moves.
fn land(s: &Site, archive: &Path) {
    let (id, report) = deliver(&s.svc, archive).unwrap();
    let _ = report;
    s.svc
        .delivery
        .confirm(&alice(), &s.svc.record, &id)
        .unwrap();
}

/// `prikk extract --ref <block> <dir>` in the live repository: a tree to hold the numbers against.
fn extract(live: &Path, block: &str, into: &Path) {
    let _ = std::fs::remove_dir_all(into);
    let out = Command::new("prikk")
        .current_dir(live)
        .args(["extract", "--ref", block, "--allow-full-replay"])
        .arg(into)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// What `diff -r` says of two trees: the files that differ (or are only on one side), and the lines added and removed over the files both of
/// which are text (`diff -ruN`'s `+` and `-` lines).
fn diff_r(before: &Path, after: &Path) -> (Vec<String>, u64, u64) {
    let q = Command::new("diff")
        .args(["-rq"])
        .arg(before)
        .arg(after)
        .output()
        .unwrap();
    let mut files: Vec<String> = String::from_utf8_lossy(&q.stdout)
        .lines()
        .filter_map(|l| {
            if let Some(rest) = l.strip_prefix("Only in ") {
                let (dir, name) = rest.split_once(": ")?;
                let rel = Path::new(dir)
                    .strip_prefix(before)
                    .or_else(|_| Path::new(dir).strip_prefix(after))
                    .ok()?
                    .join(name);
                Some(rel.to_string_lossy().into_owned())
            } else {
                let after_side = l.split(" and ").nth(1)?.trim_end_matches(" differ");
                Some(
                    Path::new(after_side)
                        .strip_prefix(after)
                        .ok()?
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        })
        .filter(|p| !p.starts_with(".prikk"))
        .collect();
    files.sort();
    let u = Command::new("diff")
        .args(["-ruN", "--exclude=.prikk"])
        .arg(before)
        .arg(after)
        .output()
        .unwrap();
    let (mut added, mut removed) = (0, 0);
    for l in String::from_utf8_lossy(&u.stdout).lines() {
        if l.starts_with("+++") || l.starts_with("---") {
            continue;
        }
        if l.starts_with('+') {
            added += 1;
        } else if l.starts_with('-') {
            removed += 1;
        }
    }
    (files, added, removed)
}

/// The block of `heads/main` that carries a patch.
fn block_of_patch(s: &Services, patch: &str) -> (String, Vec<String>) {
    let log = s.live().log(Some("heads/main"), Some(100_000)).unwrap();
    let b = log
        .blocks
        .iter()
        .find(|b| b.patch_messages.iter().any(|p| p.patch_id == patch))
        .unwrap_or_else(|| panic!("no block carries {patch}"));
    (b.block_id.clone(), b.parent_block_ids.clone())
}

fn patches_of(s: &Services, id: &str) -> Vec<String> {
    let rev = s.changes.revisions_of(id).unwrap().remove(0);
    let mut v: Vec<String> = rev
        .declared_patch_ids
        .iter()
        .flatten()
        .chain(rev.added_patch_ids.iter())
        .cloned()
        .collect();
    v.sort();
    v.dedup();
    v
}

fn paths(list: &Value) -> Vec<String> {
    list["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_owned())
        .collect()
}

fn check_page(html: &str, lang: &str) {
    let v: Vec<String> = rules::check(&dom::parse(html))
        .into_iter()
        .map(|v| format!("{}: {}", v.rule, v.detail))
        .collect();
    assert_eq!(v, Vec::<String>::new(), "{lang}: {html}");
    assert_eq!(html.matches("<h1").count(), 1, "{lang}");
    assert!(
        !html.contains("style=") && !html.contains("<script"),
        "{lang}"
    );
}

// ------------------------------------------------------------------------------------------------ the measurement's scenario

#[tokio::test]
async fn the_landed_view_of_the_measurements_scenario_equals_a_diff_r_and_the_others_answer_as_the_facts_say()
 {
    need_prikk!();
    let fx = fixtures();
    let s = site("scenario", |_| {});
    let sc = scenario(&s.svc, fx);
    let old_tip = s.svc.live_tip();
    // P2b: one patch on the base of the history, independent of P1
    let at = archives::Maintainer {
        work: fx.m1_at_h0.clone(),
        keys: fx.m1_at_h0_keys.clone(),
    };
    let pb = clone_of(&at, &sc.work, "pb");
    step(&pb, "p2b");
    let p2b = s
        .svc
        .propose_artifact(
            "P2b",
            &artifact_of(
                &pb,
                &s.svc.record.path,
                &s.svc.base.join("forge-keys"),
                "heads/main",
                "a2b",
            ),
            None,
        )
        .proposal_id;

    // before the delivery: every proposal is pending and shows its ordinary view, not landed
    for id in [&sc.p1, &sc.p2, &sc.p3, &p2b] {
        let r = changes_of(&s, id).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        assert_eq!(r.json()["compared"]["landed"], false);
    }
    let ok_before = counter(&s, "ok");
    assert_eq!(ok_before, 4);

    land(&s, &sc.d);

    // ---- P1: one patch, landed
    let r = changes_of(&s, &sc.p1).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let list = r.json();
    dump("landed-list.json", &r.body);
    let p1_patch = patches_of(&s.svc, &sc.p1);
    assert_eq!(p1_patch.len(), 1);
    let (after_block, parents) = block_of_patch(&s.svc, &p1_patch[0]);
    assert_eq!(list["compared"]["landed"], true);
    assert_eq!(list["compared"]["after_block"], after_block.as_str());
    assert_eq!(list["compared"]["before_block"], parents[0].as_str());
    assert_eq!(list["compared"]["base_differs"], false);
    assert_eq!(list["rebuilt"], true);
    assert_eq!(list["provisional"], false);
    assert_eq!(paths(&list), vec!["src/p1.txt"]);
    assert_eq!(list["files"][0]["status"], "added");
    // the numbers equal a `diff -r` of the two trees prikk extracts
    let tmp = s.svc.base.join("trees");
    std::fs::create_dir_all(&tmp).unwrap();
    let (tb, ta) = (tmp.join("p1-before"), tmp.join("p1-after"));
    extract(&s.svc.record.path, &parents[0], &tb);
    extract(&s.svc.record.path, &after_block, &ta);
    let (files, added, removed) = diff_r(&tb, &ta);
    assert_eq!(files, vec!["src/p1.txt"]);
    assert_eq!(list["totals"]["files_changed"], files.len() as u64);
    assert_eq!(list["totals"]["lines_added"], added);
    assert_eq!(list["totals"]["lines_removed"], removed);
    // a file's diff
    let fid = list["files"][0]["file_id"].as_str().unwrap().to_owned();
    let d = get(
        &s,
        &format!("{}/{fid}", api("forge", &sc.p1, 1)),
        Some("tok-alice"),
        None,
    )
    .await;
    assert_eq!(d.status, StatusCode::OK, "{}", d.text());
    dump("landed-file-diff.json", &d.body);
    assert_eq!(d.json()["lines_added"], 8);
    assert_eq!(counter(&s, "landed_ok"), 1);

    // ---- P2 is stacked on P1 (its artifact carries P1's block, so its claims are P1's and its own): P1's claim is sealed now, its own is
    // still pending, and its own patch is in no block of the branch. It is not a landed view and not a pending one; what was cached while its claims
    // waited is still right and is served (as in 0.7.0: a pure function of immutable inputs), and it says it has not landed
    let hits = counter(&s, "cache_hit");
    let r = changes_of(&s, &sc.p2).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["compared"]["landed"], false);
    assert_eq!(counter(&s, "cache_hit"), hits + 1);
    // the same revision with nothing cached is `claim_not_held`: the facts do not make it either
    let stacked = fabricate(&s, &sc.p2, "stacked-uncached", &patches_of(&s.svc, &sc.p2));
    let r = changes_of(&s, &stacked).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
    assert_eq!(r.json()["reason"], "claim_not_held");
    dump("claim-not-held.json", &r.body);

    // ---- P2b is independent of P1 and still open: its ordinary view, not landed
    let r = changes_of(&s, &p2b).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["compared"]["landed"], false);
    assert_eq!(paths(&r.json()), vec!["src/p2b.txt"]);

    // ---- P3: two patches, the delivery carries the first: some landed and some did not
    let r = changes_of(&s, &sc.p3).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
    assert_eq!(r.json()["code"], "changes_unavailable");
    assert_eq!(r.json()["reason"], "partly_landed");
    assert_eq!(
        r.json()["message"],
        "Some of this revision's work has landed in the project's branch and some is still waiting here, so its changes cannot be shown as one change."
    );
    assert_eq!(
        r.json()["hint"],
        "Look at the project's history for what landed. The rest is still held for review."
    );
    dump("partly-landed.json", &r.body);
    assert_eq!(
        counter(&s, "partly_landed"),
        1,
        "{:?}",
        s.svc.metrics.snapshot().changes
    );
    // its pages, in both languages
    let uri = format!("/alice/forge/proposals/{}/revisions/1/changes", sc.p3);
    let (en, ja) = (
        get(&s, &uri, Some("tok-alice"), Some("en")).await,
        get(&s, &uri, Some("tok-alice"), Some("ja")).await,
    );
    assert_eq!(
        (en.status, ja.status),
        (StatusCode::CONFLICT, StatusCode::CONFLICT)
    );
    check_page(&en.text(), "en");
    check_page(&ja.text(), "ja");
    dump("partly-landed-page.en.html", &en.body);
    dump("partly-landed-page.ja.html", &ja.body);
    assert!(
        en.text()
            .contains("These changes cannot be shown as one change")
    );
    assert!(
        ja.text()
            .contains("この変更点は、1 つの変更としては表示できません")
    );
    assert!(!en.text().contains("<table"));

    // ---- the page of a landed view: the statement first, in both languages
    let uri = format!("/alice/forge/proposals/{}/revisions/1/changes", sc.p1);
    let en = get(&s, &uri, Some("tok-alice"), Some("en")).await;
    assert_eq!(en.status, StatusCode::OK, "{}", en.text());
    let (en_html, ja) = (
        en.text(),
        get(&s, &uri, Some("tok-alice"), Some("ja")).await,
    );
    check_page(&en_html, "en");
    check_page(&ja.text(), "ja");
    dump("landed-page.en.html", en_html.as_bytes());
    dump("landed-page.ja.html", &ja.body);
    let landed_en = "These changes have landed: they are part of the project&#39;s branch.";
    let rebuilt_en = "These files are rebuilt from the patches in this revision.";
    let (a, b) = (en_html.find(landed_en), en_html.find(rebuilt_en));
    assert!(a.is_some() && b.is_some() && a < b, "{en_html}");
    let landed_ja = "この変更は反映済みです。プロジェクトのブランチの一部になっています。";
    assert!(ja.text().contains(landed_ja), "{}", ja.text());
    assert!(ja.text().contains("<html lang=\"ja\""));
    // a pending revision's page does not say it
    let p2 = get(
        &s,
        &format!("/alice/forge/proposals/{}/revisions/1/changes", sc.p2),
        Some("tok-alice"),
        Some("en"),
    )
    .await;
    assert!(!p2.text().contains("have landed"), "{}", p2.text());

    // ---- authorization and 404 equality, over the landed case too
    let uri = api("forge", &sc.p1, 1);
    assert_eq!(
        get(&s, &uri, Some("tok-bob"), None).await.status,
        StatusCode::OK
    );
    let anon = get(&s, &uri, None, None).await;
    assert_eq!(
        anon.status,
        StatusCode::OK,
        "a public repository is readable"
    );
    let hidden = get(&s, &api("secret", &sc.p1, 1), Some("tok-bob"), None).await;
    let unknown = get(&s, &api("secret", "no-such", 1), Some("tok-bob"), None).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(
        hidden.body, unknown.body,
        "a hidden repository and an unknown proposal are one 404"
    );
    let nope = get(&s, &api("forge", &sc.p1, 9), Some("tok-alice"), None).await;
    assert_eq!(nope.status, StatusCode::NOT_FOUND);
    let _ = old_tip;
}

// ------------------------------------------------------------------------------------------------ the other cases

/// A worker that has the live history and `n` further sealed blocks (`tags`), as a delivery archive, plus the worker.
fn at_live(s: &Site, name: &str) -> archives::Maintainer {
    let fx = fixtures();
    let base = s.svc.base.join("work");
    std::fs::create_dir_all(&base).unwrap();
    let at = archives::Maintainer {
        work: fx.m1_at_h0.clone(),
        keys: fx.m1_at_h0_keys.clone(),
    };
    clone_of(&at, &base, name)
}

fn propose_from(s: &Site, w: &archives::Maintainer, reference: &str, title: &str) -> String {
    let art = artifact_of(
        w,
        &s.svc.record.path,
        &s.svc.base.join("forge-keys"),
        reference,
        &format!("art-{title}"),
    );
    // `artifact_of` leaves its two files in the worker's directory: a later `commit --from-worktree` would take them
    for ext in ["art", "have"] {
        let _ = std::fs::remove_file(w.work.join(format!("art-{title}.{ext}")));
    }
    s.svc.propose_artifact(title, &art, None).proposal_id
}

/// A revision record of its own with `patches` as its declared set (the claims are none: it is asked of the history only).
fn fabricate(s: &Site, like: &str, name: &str, patches: &[String]) -> String {
    let like_rev = s.svc.changes.revisions_of(like).unwrap().remove(0);
    let mut change = s.svc.changes.get_change(like).unwrap().unwrap();
    change.change_id = format!("c-{name}");
    let mut rev = like_rev;
    rev.change_id = change.change_id.clone();
    rev.revision_id = format!("v-{name}");
    rev.seq = 1;
    rev.claim_ids = vec![];
    rev.added_patch_ids = vec![];
    rev.declared_patch_ids = Some(patches.to_vec());
    rev.artifact_digest = planeter_auth::sha256_hex(name.as_bytes());
    s.svc
        .changes
        .open_or_resume_intent(
            &format!("i-{name}"),
            &rev.repo_id,
            &rev.artifact_digest,
            rev.artifact_bytes,
            &[],
            "alice",
            real_clock(),
        )
        .unwrap();
    s.svc
        .changes
        .complete_intent_with_revision(&format!("i-{name}"), Some(&change), &rev, real_clock())
        .unwrap();
    change.change_id
}

#[tokio::test]
async fn a_revision_landed_in_two_deliveries_is_partly_landed_after_the_first_and_a_view_after_the_second_and_its_cache_is_not_the_pendings()
 {
    need_prikk!();
    let s = site("split", |_| {});
    let c = at_live(&s, "c");
    step(&c, "s1");
    let c2 = clone_of(&c, &s.svc.base.join("work"), "c2");
    step(&c, "s2");
    let split = propose_from(&s, &c, "heads/main", "SPLIT");
    let work = s.svc.base.join("work");
    let (d1, d2) = (work.join("split-1.archive"), work.join("split-2.archive"));
    c2.archive(&d1);
    c.archive(&d2);

    // pending: the ordinary view, cached under the claims
    let r = changes_of(&s, &split).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["compared"]["landed"], false);
    assert_eq!(counter(&s, "extractions"), 1);

    // the first delivery carries s1 only
    land(&s, &d1);
    let r = changes_of(&s, &split).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
    assert_eq!(r.json()["reason"], "partly_landed");
    assert_eq!(counter(&s, "partly_landed"), 1);

    // the second carries s2: the view, and it is **not** the pending one that is in the cache under this revision
    land(&s, &d2);
    let r = changes_of(&s, &split).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let list = r.json();
    assert_eq!(list["compared"]["landed"], true, "{list}");
    assert_eq!(counter(&s, "landed_ok"), 1);
    assert_eq!(paths(&list), vec!["src/s1.txt", "src/s2.txt"]);
    let log = s.svc.live().log(Some("heads/main"), Some(100)).unwrap();
    assert_eq!(
        list["compared"]["after_block"],
        log.blocks[0].block_id.as_str()
    );
    assert_eq!(
        list["compared"]["before_block"],
        log.blocks[2].block_id.as_str()
    );
    // equal to a `diff -r`
    let tmp = s.svc.base.join("trees");
    std::fs::create_dir_all(&tmp).unwrap();
    let (tb, ta) = (tmp.join("b"), tmp.join("a"));
    extract(&s.svc.record.path, &log.blocks[2].block_id, &tb);
    extract(&s.svc.record.path, &log.blocks[0].block_id, &ta);
    let (files, added, removed) = diff_r(&tb, &ta);
    assert_eq!(files, vec!["src/s1.txt", "src/s2.txt"]);
    assert_eq!(list["totals"]["lines_added"], added);
    assert_eq!(list["totals"]["lines_removed"], removed);

    let split_body = r.body.clone();
    // the second request is served from the cache (no extraction)
    let (hits, extractions) = (counter(&s, "cache_hit"), counter(&s, "extractions"));
    let again = changes_of(&s, &split).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body, r.body, "the same bytes");
    assert_eq!(counter(&s, "cache_hit"), hits + 1);
    assert_eq!(counter(&s, "extractions"), extractions);

    // P4: three commits sealed together by the maintainer: one block, three files
    let p4w = clone_of(&c, &work, "p4");
    for n in 1..=3 {
        p4w.write(
            &format!("src/p4-{n}.txt"),
            &format!("p4 file {n}\n").repeat(4),
        );
        p4w.commit(&format!("p4 commit {n}"));
    }
    p4w.seal(None);
    let p4 = propose_from(&s, &p4w, "heads/main", "P4");
    let d3 = work.join("p4.archive");
    p4w.archive(&d3);
    land(&s, &d3);
    let r = changes_of(&s, &p4).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let list = r.json();
    assert_eq!(list["compared"]["landed"], true);
    assert_eq!(
        paths(&list),
        vec!["src/p4-1.txt", "src/p4-2.txt", "src/p4-3.txt"]
    );
    let log = s.svc.live().log(Some("heads/main"), Some(100)).unwrap();
    assert_eq!(
        log.blocks[0].patch_messages.len(),
        3,
        "one block carries the three patches"
    );
    assert_eq!(
        list["compared"]["after_block"],
        log.blocks[0].block_id.as_str()
    );
    assert_eq!(
        list["compared"]["before_block"],
        log.blocks[1].block_id.as_str()
    );
    let (tb, ta) = (tmp.join("p4b"), tmp.join("p4a"));
    extract(&s.svc.record.path, &log.blocks[1].block_id, &tb);
    extract(&s.svc.record.path, &log.blocks[0].block_id, &ta);
    let (files, added, removed) = diff_r(&tb, &ta);
    assert_eq!(files.len(), 3);
    assert_eq!(list["totals"]["files_changed"], 3);
    assert_eq!(list["totals"]["lines_added"], added);
    assert_eq!(list["totals"]["lines_removed"], removed);

    // a later delivery moves the branch on: the earlier landed views are content-addressed and still served from the cache
    let c3 = clone_of(&p4w, &work, "c3");
    step(&c3, "later");
    let d4 = work.join("later.archive");
    c3.archive(&d4);
    land(&s, &d4);
    let (hits, extractions) = (counter(&s, "cache_hit"), counter(&s, "extractions"));
    let r2 = changes_of(&s, &split).await;
    assert_eq!(r2.status, StatusCode::OK, "{}", r2.text());
    assert_eq!(r2.json()["compared"]["landed"], true);
    assert_eq!(r2.body, split_body, "the same bytes");
    assert_eq!(
        counter(&s, "cache_hit"),
        hits + 1,
        "served from the cache after another delivery"
    );
    assert_eq!(counter(&s, "extractions"), extractions);
}

#[tokio::test]
async fn a_root_a_scattered_set_and_a_set_outside_the_window_answer_as_the_facts_say() {
    need_prikk!();
    let fx = fixtures();
    let s = site("odd", |_| {});
    let sc = scenario(&s.svc, fx);
    land(&s, &sc.d);
    let log = s.svc.live().log(Some("heads/main"), Some(1000)).unwrap();
    let ids = |i: usize| -> Vec<String> {
        log.blocks[i]
            .patch_messages
            .iter()
            .map(|p| p.patch_id.clone())
            .collect()
    };

    // a revision whose patches are the first block of the history: nothing before it
    let root = log.blocks.last().unwrap();
    assert_eq!(root.parent_count, 0);
    let root_patches: Vec<String> = root
        .patch_messages
        .iter()
        .map(|p| p.patch_id.clone())
        .collect();
    let id = fabricate(&s, &sc.p1, "root", &root_patches);
    let r = changes_of(&s, &id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    dump("root-view.json", &r.body);
    let v = r.json();
    assert_eq!(v["compared"]["landed"], true);
    assert!(v["compared"]["before_block"].is_null());
    assert_eq!(v["compared"]["after_block"], root.block_id.as_str());
    assert!(v["totals"]["files_changed"].as_u64().unwrap() >= 1);
    assert_eq!(v["totals"]["lines_removed"], 0);
    assert!(
        v["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["status"] == "added")
    );
    let uri = format!("/alice/forge/proposals/{id}/revisions/1/changes");
    let page = get(&s, &uri, Some("tok-alice"), Some("en")).await;
    assert_eq!(page.status, StatusCode::OK);
    check_page(&page.text(), "en");
    dump("root-page.en.html", &page.body);

    // the newest block and the oldest of the delivered run are not one run: another block lies between
    let scattered: Vec<String> = ids(0).into_iter().chain(ids(3)).collect();
    let id = fabricate(&s, &sc.p1, "scattered", &scattered);
    let r = changes_of(&s, &id).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    assert_eq!(r.json()["reason"], "not_one_change");
    // two adjacent blocks are one run
    let adjacent: Vec<String> = ids(0).into_iter().chain(ids(1)).collect();
    let id = fabricate(&s, &sc.p1, "adjacent", &adjacent);
    let r = changes_of(&s, &id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(
        r.json()["compared"]["after_block"],
        log.blocks[0].block_id.as_str()
    );
    assert_eq!(
        r.json()["compared"]["before_block"],
        log.blocks[2].block_id.as_str()
    );
    // a patch nobody has: not landed at all, the old answer
    let id = fabricate(&s, &sc.p1, "nowhere", &["ab".repeat(32)]);
    let r = changes_of(&s, &id).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.json()["reason"], "claim_not_held");
    // and a revision with no patches to ask about
    let id = fabricate(&s, &sc.p1, "nothing", &[]);
    let r = changes_of(&s, &id).await;
    assert_eq!(r.json()["reason"], "claim_not_held");

    // the window: the same history read through a window of two blocks cannot see P1's block, and does not claim a view of it
    let narrow = site("window", |c| c.observation_blocks = 2);
    let sc2 = scenario(&narrow.svc, fx);
    land(&narrow, &sc2.d);
    let r = changes_of(&narrow, &sc2.p1).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
    assert_eq!(r.json()["reason"], "claim_not_held");
    assert_eq!(counter(&narrow, "landed_ok"), 0);
}

#[tokio::test]
async fn a_patch_in_an_extra_published_branch_is_not_landed_only_heads_main_is() {
    need_prikk!();
    let s = site("extra", |_| {});
    let w = at_live(&s, "dv");
    let run = |args: &[&str]| archives::run(&w.work, &w.keys, args);
    run(&["branch", "create", "heads/dev", "--from", "heads/main"]);
    run(&["branch", "switch", "heads/dev"]);
    w.write("src/on-dev.txt", &"on dev\n".repeat(8));
    w.commit("work on dev");
    w.seal(Some("heads/dev"));
    let px = propose_from(&s, &w, "heads/dev", "ONDEV");
    run(&["branch", "switch", "heads/main"]);
    step(&w, "main-moves");
    let d = s.svc.base.join("work").join("extra.archive");
    w.archive(&d);
    land(&s, &d);
    // the delivery published the extra branch with the patch in it; heads/main does not hold it
    let branches = s.svc.live().branches(true).unwrap();
    assert!(
        branches.branches.iter().any(|b| b.ref_name == "heads/dev"),
        "the precondition: the extra branch is published"
    );
    let r = changes_of(&s, &px).await;
    eprintln!("extra branch: {} {}", r.status, r.text());
    assert!(
        r.status != StatusCode::OK || r.json()["compared"]["landed"] == false,
        "a patch of an extra branch made a landed view: {}",
        r.text()
    );
    assert_eq!(counter(&s, "landed_ok"), 0);
}
