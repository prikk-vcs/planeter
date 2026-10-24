//! RFC 005 part 5: the read side of proposals over a **stand-in** `prikk` — a script that answers `sync pending`, `branch` and
//! `log` from files and writes down every call it gets. What these tests pin is how often prikk is asked (one `sync pending`
//! per repository per state fingerprint, `branch` and `log` only for proposals that could have landed, never one call per
//! item), the landing rule on the facts it is given, and the retention sweep's decisions. (The tests against a real prikk
//! are in `planeter-web/tests/transport_propose.rs`.)

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use planeter_core::Assurance;
use planeter_core::{HostingService, LeaseManager, new_boot_id};
use planeter_store::{
    ChangeRecord, ChangeState, ChangeStore, InMemoryChangeStore, InMemoryLeaseRecords,
    InMemoryRepositoryStore, InMemoryRevisions, Owner, ProposalPolicy, RepoId, RepositoryRecord,
    RepositoryStore, RevisionRecord, RevisionStore, Visibility,
};
use planeter_transport::{
    ArtifactStore, ExportConfig, ExportService, Metrics, ProposalConfig, ProposalError,
    ProposalService, Retention, Spool, StateFilter, record_digest,
};

const T0: u64 = 1_000_000;
const GRACE: u64 = 30 * 86_400;

fn now() -> u64 {
    T0
}
fn after_close_plus_grace() -> u64 {
    T0 + GRACE + 1
}
fn at_the_end_of_the_grace() -> u64 {
    T0 + GRACE
}

fn pid(n: u64) -> String {
    format!("{n:064x}")
}
fn digest_of(n: u64) -> String {
    format!("{n:064x}")
}
fn recorded(n: u64) -> String {
    format!("sha256:{n:064x}")
}

struct Fixture {
    dir: PathBuf,
    repo: RepositoryRecord,
    changes: Arc<InMemoryChangeStore>,
    artifacts: Arc<ArtifactStore>,
    export: Arc<ExportService>,
    hosting: Arc<HostingService>,
    metrics: Arc<Metrics>,
    svc: ProposalService,
    n_files: std::cell::Cell<u32>,
}

fn script(dir: &Path) -> PathBuf {
    let path = dir.join("prikk");
    std::fs::write(
        &path,
        r#"#!/bin/sh
D="$PWD/.standin"
echo "$*" >> "$D/calls"
case "$1" in
  --version) echo "prikk 0.54.1" ;;
  sync) cat "$D/pending" ;;
  branch) cat "$D/branches.json" ;;
  log) ref=""; while [ $# -gt 0 ]; do [ "$1" = "--ref" ] && ref="$2"; shift; done; cat "$D/log-$(echo "$ref" | tr / _).json" ;;
  *) echo "stand-in: unexpected $*" >&2; exit 2 ;;
esac
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A parallel test thread's fork can hold the file open for writing for a moment (ETXTBSY): wait until it runs.
    for _ in 0..200 {
        match std::process::Command::new(&path).arg("--version").output() {
            Ok(_) => break,
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("{e}"),
        }
    }
    path
}

impl Fixture {
    fn new(name: &str, clock: fn() -> u64, sweep_batch: usize) -> Fixture {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("proposals-unit-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let repo_dir = dir.join("repo");
        std::fs::create_dir_all(repo_dir.join(".prikk")).unwrap();
        std::fs::create_dir_all(repo_dir.join(".standin")).unwrap();
        let repo = RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "app".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: ProposalPolicy::default(),
            path: repo_dir,
        };
        let store = Arc::new(InMemoryRepositoryStore::new());
        store.create(repo.clone()).unwrap();
        let hosting = Arc::new(
            HostingService::new(dir.join("root"), store as Arc<dyn RepositoryStore>)
                .with_sandbox(planeter_prikk::Sandbox::Unconfined)
                .with_binary(script(&dir)),
        );
        let leases = Arc::new(LeaseManager::new(
            Arc::new(InMemoryLeaseRecords::new()),
            new_boot_id(),
            now,
            Duration::from_secs(3),
        ));
        let metrics = Arc::new(Metrics::new());
        let export = Arc::new(ExportService::new(
            hosting.clone(),
            Spool::open(dir.join("spool"), 64 << 20).unwrap(),
            leases,
            Arc::new(InMemoryRevisions::new()) as Arc<dyn RevisionStore>,
            ExportConfig::default(),
            metrics.clone(),
        ));
        let artifacts = Arc::new(ArtifactStore::open(dir.join("artifacts")).unwrap());
        let changes = Arc::new(InMemoryChangeStore::new());
        let svc = ProposalService::new(
            hosting.clone(),
            changes.clone() as Arc<dyn ChangeStore>,
            export.clone(),
            artifacts.clone(),
            metrics.clone(),
            ProposalConfig {
                grace_secs: GRACE,
                sweep_batch,
            },
        )
        .with_clock(clock);
        let f = Fixture {
            dir,
            repo,
            changes,
            artifacts,
            export,
            hosting,
            metrics,
            svc,
            n_files: std::cell::Cell::new(0),
        };
        f.say("pending", "pending (accepted, unsealed) patches: 0\n");
        f.say(
            "branches.json",
            r#"{"schema_version":"branch-list-v1","branches":[{"ref_name":"heads/main","ref_state_id":"aa","closed":false,"current":true}],"received":[{"ref_name":"remotes/heads/main","ref_state_id":"bb"}],"unpublished_current_branch":null}"#,
        );
        f.log("heads/main", &[]);
        f.log("remotes/heads/main", &[]);
        f
    }

    fn standin(&self) -> PathBuf {
        self.repo.path.join(".standin")
    }
    fn say(&self, file: &str, text: &str) {
        std::fs::write(self.standin().join(file), text).unwrap();
    }
    fn pending(&self, ids: &[String]) {
        let mut out = format!("pending (accepted, unsealed) patches: {}\n", ids.len());
        for id in ids {
            out += &format!("  {id}\n");
        }
        self.say("pending", &out);
    }
    /// What `log` answers for `ref_name`: one block per patch id.
    fn log(&self, ref_name: &str, patch_ids: &[String]) {
        let blocks: Vec<String> = patch_ids
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    r#"{{"block_id":"{b}","ref_state_id":"{b}","update_seq":{i},"kind":"Normal","rollback_block":false,"parent_count":1,"patch_count":1,"rollback_patch_count":0,"required_attestation_count":0,"patch_messages":[{{"patch_id":"{p}","message":"m"}}],"previous_ref_state_id":null}}"#,
                    b = pid(9_000 + i as u64)
                )
            })
            .collect();
        self.say(
            &format!("log-{}.json", ref_name.replace('/', "_")),
            &format!(
                r#"{{"schema_version":"log-report-v1","repository":"x","ref":"{ref_name}","current_branch":"heads/main","blocks":[{}]}}"#,
                blocks.join(",")
            ),
        );
    }
    /// The state fingerprint moves (a file of the repository changed).
    fn move_state(&self) {
        let n = self.n_files.get() + 1;
        self.n_files.set(n);
        std::fs::write(self.repo.path.join(".prikk").join(format!("f{n}")), "x").unwrap();
    }
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.standin().join("calls"))
            .unwrap_or_default()
            .lines()
            .filter(|l| *l != "--version")
            .map(str::to_owned)
            .collect()
    }
    fn count(&self, prefix: &str) -> usize {
        self.calls()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }

    /// A proposal with one revision holding `ids` as its patches and `digest` as its artifact.
    fn propose(&self, id: &str, by: &str, ids: &[String], digest: u64) {
        let change = ChangeRecord {
            change_id: id.into(),
            repo_id: self.repo.repo_id.clone(),
            title: format!("title {id}"),
            description: String::new(),
            target_ref: None,
            state: ChangeState::Open,
            closed_reason: None,
            opened_by: by.into(),
            opened_at: T0,
            updated_at: T0,
            retain_until: None,
        };
        self.revision(&change, 1, ids, digest);
    }
    fn revision(&self, change: &ChangeRecord, seq: u32, ids: &[String], digest: u64) {
        let rev = RevisionRecord {
            revision_id: format!("v-{}-{seq}", change.change_id),
            change_id: change.change_id.clone(),
            repo_id: self.repo.repo_id.clone(),
            seq,
            artifact_digest: recorded(digest),
            artifact_bytes: 100,
            declared_base: None,
            claim_ids: vec![pid(500 + u64::from(seq))],
            added_patch_ids: ids.to_vec(),
            declared_patch_ids: Some(ids.to_vec()),
            claim_outcomes: vec![],
            submitter: change.opened_by.clone(),
            author_key_ids: vec![],
            created_at: T0,
        };
        let intent = format!("i-{}-{seq}", change.change_id);
        self.changes
            .open_or_resume_intent(
                &intent,
                &self.repo.repo_id,
                &rev.artifact_digest,
                0,
                &[],
                &change.opened_by,
                T0,
            )
            .unwrap();
        let first = (seq == 1).then_some(change);
        self.changes
            .complete_intent_with_revision(&intent, first, &rev, T0)
            .unwrap();
    }
    /// Store an artifact object under a digest of this test's choosing, by writing the file where the store keeps it.
    fn keep(&self, digest: u64) {
        let hex = digest_of(digest);
        let dir = self.dir.join("artifacts/objects").join(&hex[..2]);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(&hex), b"artifact bytes").unwrap();
    }
    fn kept(&self, digest: u64) -> bool {
        self.artifacts
            .contains(&record_digest(&recorded(digest)).unwrap())
    }
    fn state_of(&self, id: &str) -> ChangeState {
        self.changes.get_change(id).unwrap().unwrap().state
    }
    fn list(&self, filter: StateFilter) -> Vec<String> {
        self.svc
            .list(&self.repo, filter, 50, None)
            .unwrap()
            .items
            .into_iter()
            .map(|s| s.proposal.change_id)
            .collect()
    }
    fn close(&self, id: &str) {
        self.changes
            .leave_open(id, ChangeState::Closed, Some("done"), T0, Some(T0 + GRACE))
            .unwrap();
    }
}

#[test]
fn a_digest_in_a_record_is_the_stores_digest_without_its_prefix() {
    let hex = pid(7);
    assert_eq!(
        record_digest(&format!("sha256:{hex}")).unwrap().as_str(),
        hex
    );
    assert_eq!(record_digest(&hex).unwrap().as_str(), hex);
    assert!(record_digest("sha256:short").is_err());
    assert!(record_digest("sha512:".to_owned().as_str()).is_err());
}

#[test]
fn prikk_is_asked_once_per_state_however_many_proposals_and_requests() {
    let f = Fixture::new("once", now, 200);
    // Ten proposals, all still pending: nobody could have landed, so `sync pending` is all that is read.
    let all: Vec<String> = (1..=10).map(pid).collect();
    for (i, id) in all.iter().enumerate() {
        f.propose(
            &format!("c{i}"),
            "alice",
            std::slice::from_ref(id),
            100 + i as u64,
        );
    }
    f.pending(&all);
    for _ in 0..5 {
        assert_eq!(f.list(StateFilter::Open).len(), 10);
    }
    for i in 0..10 {
        let d = f.svc.detail(&f.repo, &format!("c{i}")).unwrap();
        assert_eq!(
            d.patches[0].assurance,
            Some(Assurance::ForgeApprovedUnsealed)
        );
    }
    assert_eq!(f.count("sync pending"), 1, "{:?}", f.calls());
    assert_eq!(
        f.count("branch"),
        0,
        "no proposal could have landed: no log is read"
    );
    assert_eq!(f.count("log"), 0);

    // The state moves: one more `sync pending`, once.
    f.move_state();
    for _ in 0..3 {
        f.list(StateFilter::All);
    }
    assert_eq!(f.count("sync pending"), 2, "{:?}", f.calls());
}

#[test]
fn log_is_read_only_for_proposals_whose_patches_have_all_left_pending() {
    let f = Fixture::new("candidates", now, 200);
    let (a, b, c) = (pid(1), pid(2), pid(3));
    f.propose("held", "alice", std::slice::from_ref(&a), 101);
    f.propose("left", "alice", std::slice::from_ref(&b), 102);
    f.propose("half", "alice", &[b.clone(), c.clone()], 103);
    // `held` is still pending; `left` has left pending; `half` has one patch pending.
    f.pending(&[a.clone(), c.clone()]);
    f.log("heads/main", &[]);
    f.list(StateFilter::Open);
    assert_eq!(f.count("sync pending"), 1);
    assert_eq!(
        f.count("branch"),
        1,
        "`left` is a candidate, so the refs are listed"
    );
    assert_eq!(
        f.count("log"),
        2,
        "one log per ref the repository holds: {:?}",
        f.calls()
    );
    assert_eq!(
        f.state_of("left"),
        ChangeState::Open,
        "absent from pending is not landed"
    );
    assert_eq!(f.state_of("held"), ChangeState::Open);
    assert_eq!(f.state_of("half"), ChangeState::Open);
    // Asking again in the same state reads nothing more.
    f.list(StateFilter::Open);
    f.svc.detail(&f.repo, "left").unwrap();
    assert_eq!(f.count("sync pending"), 1);
    assert_eq!(f.count("log"), 2);
}

#[test]
fn a_patch_in_a_block_of_a_received_ref_counts_and_one_that_is_in_no_block_does_not() {
    let f = Fixture::new("received", now, 200);
    let (a, b) = (pid(1), pid(2));
    f.propose("both", "alice", &[a.clone(), b.clone()], 101);
    f.propose("neither", "alice", &[pid(3)], 102);
    f.pending(&[]);
    // One patch in the published ref's log, the other only in the received ref's.
    f.log("heads/main", std::slice::from_ref(&a));
    f.log("remotes/heads/main", std::slice::from_ref(&b));
    assert_eq!(f.list(StateFilter::Landed), vec!["both"]);
    assert_eq!(f.state_of("neither"), ChangeState::Open);
    let d = f.svc.detail(&f.repo, "both").unwrap();
    assert_eq!(d.proposal.state, ChangeState::Landed);
    assert_eq!(
        d.proposal.retain_until,
        Some(T0 + GRACE),
        "a landing starts the grace"
    );
    assert!(
        d.patches
            .iter()
            .all(|p| p.assurance == Some(Assurance::Sealed))
    );
    let n = f.svc.detail(&f.repo, "neither").unwrap();
    assert_eq!(
        n.patches[0].assurance, None,
        "nothing is claimed about a patch that is neither held nor in a block"
    );
    assert_eq!(n.still_pending, 0);
}

#[test]
fn a_log_that_may_have_been_cut_short_lands_nothing() {
    // The observation reads at most 100,000 blocks of a ref; a log that fills that may be a part, so nothing lands on it.
    let f = Fixture::new("cut-short", now, 200);
    f.propose("p", "alice", &[pid(1)], 101);
    f.pending(&[]);
    // The proposal's patch is in the log; the log is exactly as long as the limit, so it may be only a part of the history.
    let mut ids: Vec<String> = (1..100_000u64).map(|n| pid(1_000_000 + n)).collect();
    ids.push(pid(1));
    f.log("heads/main", &ids);
    assert_eq!(f.list(StateFilter::Open), vec!["p"]);
    assert_eq!(f.state_of("p"), ChangeState::Open);
}

#[test]
fn an_unreadable_repository_still_lists_what_the_records_say_and_claims_nothing_about_patches() {
    let f = Fixture::new("unreadable", now, 200);
    f.propose("p", "alice", &[pid(1)], 101);
    std::fs::remove_file(f.standin().join("pending")).unwrap(); // the stand-in's `cat` now fails
    assert_eq!(f.list(StateFilter::Open), vec!["p"]);
    let d = f.svc.detail(&f.repo, "p").unwrap();
    assert_eq!(d.proposal.state, ChangeState::Open);
    assert_eq!(d.patches[0].assurance, None);
    assert_eq!(d.still_pending, 0);
}

#[test]
fn closing_needs_an_open_proposal_and_a_reason_and_starts_the_grace() {
    let f = Fixture::new("close", now, 200);
    f.propose("p", "alice", &[pid(1)], 101);
    f.keep(101);
    assert!(matches!(
        f.svc.close(&f.repo, "p", "  "),
        Err(ProposalError::ReasonRequired)
    ));
    assert!(matches!(
        f.svc.close(&f.repo, "p", &"x".repeat(1001)),
        Err(ProposalError::ReasonTooLong)
    ));
    assert!(matches!(
        f.svc.close(&f.repo, "nope", "r"),
        Err(ProposalError::NotFound)
    ));
    let closed = f.svc.close(&f.repo, "p", " Superseded ").unwrap();
    assert_eq!(closed.state, ChangeState::Closed);
    assert_eq!(closed.closed_reason.as_deref(), Some("Superseded"));
    assert_eq!(closed.retain_until, Some(T0 + GRACE));
    assert!(matches!(
        f.svc.close(&f.repo, "p", "again"),
        Err(ProposalError::AlreadyFinal(ChangeState::Closed))
    ));
    let d = f.svc.detail(&f.repo, "p").unwrap();
    assert_eq!(d.retention, Retention::Kept { until: T0 + GRACE });
    // Another repository's id is the same as an unknown one.
    let mut other = f.repo.clone();
    other.repo_id = RepoId::new("r2");
    assert!(matches!(
        f.svc.detail(&other, "p"),
        Err(ProposalError::NotFound)
    ));
    assert!(matches!(
        f.svc.close(&other, "p", "r"),
        Err(ProposalError::NotFound)
    ));
}

#[test]
fn the_list_pages_by_a_cursor_and_refuses_one_it_did_not_give() {
    let f = Fixture::new("cursor", now, 200);
    f.pending(&(1..=5).map(pid).collect::<Vec<_>>());
    for i in 1..=5u64 {
        f.propose(&format!("c{i}"), "alice", &[pid(i)], 100 + i);
    }
    let first = f.svc.list(&f.repo, StateFilter::Open, 2, None).unwrap();
    assert_eq!(first.items.len(), 2);
    let cursor = first.next.clone().expect("more to show");
    let second = f
        .svc
        .list(&f.repo, StateFilter::Open, 2, Some(&cursor))
        .unwrap();
    let third = f
        .svc
        .list(&f.repo, StateFilter::Open, 2, second.next.as_deref())
        .unwrap();
    let mut seen: Vec<String> = [&first, &second, &third]
        .iter()
        .flat_map(|p| p.items.iter().map(|s| s.proposal.change_id.clone()))
        .collect();
    seen.sort();
    assert_eq!(seen, ["c1", "c2", "c3", "c4", "c5"], "every proposal once");
    assert!(third.next.is_none());
    for bad in ["", "x", "1.", ".c1", "1.../etc", "1.a b"] {
        assert!(
            matches!(
                f.svc.list(&f.repo, StateFilter::Open, 2, Some(bad)),
                Err(ProposalError::BadCursor)
            ),
            "{bad:?}"
        );
    }
}

// ------------------------------------------------------------------------------------------------ the sweep

#[test]
fn the_sweep_keeps_a_closed_proposals_files_through_the_last_second_of_the_grace_and_removes_them_after()
 {
    for (name, clock, removed) in [
        ("sweep-at-end", at_the_end_of_the_grace as fn() -> u64, 0),
        ("sweep-after", after_close_plus_grace as fn() -> u64, 1),
    ] {
        let f = Fixture::new(name, clock, 200);
        f.propose("p", "alice", &[pid(1)], 101);
        f.keep(101);
        f.close("p");
        let r = f.svc.sweep();
        assert_eq!(r.removed, removed, "{name}: {r:?}");
        assert_eq!(f.kept(101), removed == 0, "{name}");
        if removed == 1 {
            assert_eq!(r.removed_bytes, 100);
            let d = f.svc.detail(&f.repo, "p").unwrap();
            assert_eq!(d.retention, Retention::Removed { until: T0 + GRACE });
            assert!(!d.revisions[0].artifact_available);
            assert_eq!(f.metrics.snapshot().sweep_removed, 1);
        }
    }
}

#[test]
fn the_sweep_never_removes_the_files_of_an_open_proposal() {
    let f = Fixture::new("sweep-open", after_close_plus_grace, 200);
    f.propose("p", "alice", &[pid(1)], 101);
    f.keep(101);
    let r = f.svc.sweep();
    assert_eq!((r.examined, r.removed), (0, 0));
    assert!(f.kept(101));
}

#[test]
fn a_file_two_revisions_name_stays_while_either_is_needed() {
    let f = Fixture::new("sweep-shared", after_close_plus_grace, 200);
    // The same artifact sent as two proposals (two repositories, or a resend): one is closed and past its grace, the other open.
    f.propose("old", "alice", &[pid(1)], 101);
    f.keep(101);
    f.close("old");
    let mut other = f.repo.clone();
    other.repo_id = RepoId::new("r2");
    let change = ChangeRecord {
        change_id: "still-open".into(),
        repo_id: other.repo_id.clone(),
        title: "t".into(),
        description: String::new(),
        target_ref: None,
        state: ChangeState::Open,
        closed_reason: None,
        opened_by: "bob".into(),
        opened_at: T0,
        updated_at: T0,
        retain_until: None,
    };
    let rev = RevisionRecord {
        revision_id: "v-open".into(),
        change_id: "still-open".into(),
        repo_id: other.repo_id.clone(),
        seq: 1,
        artifact_digest: recorded(101),
        artifact_bytes: 100,
        declared_base: None,
        claim_ids: vec![pid(600)],
        added_patch_ids: vec![pid(1)],
        declared_patch_ids: None,
        claim_outcomes: vec![],
        submitter: "bob".into(),
        author_key_ids: vec![],
        created_at: T0,
    };
    f.changes
        .open_or_resume_intent(
            "i-open",
            &other.repo_id,
            &rev.artifact_digest,
            0,
            &[],
            "bob",
            T0,
        )
        .unwrap();
    f.changes
        .complete_intent_with_revision("i-open", Some(&change), &rev, T0)
        .unwrap();
    let r = f.svc.sweep();
    assert_eq!((r.removed, r.kept), (0, 1), "{r:?}");
    assert!(f.kept(101));
    // Once the other one leaves open and its own grace passes, the file goes.
    f.changes
        .leave_open(
            "still-open",
            ChangeState::Closed,
            Some("done"),
            T0,
            Some(T0 + GRACE),
        )
        .unwrap();
    assert_eq!(f.svc.sweep().removed, 1);
    assert!(!f.kept(101));
}

#[test]
fn a_file_an_open_intent_names_is_kept() {
    let f = Fixture::new("sweep-intent", after_close_plus_grace, 200);
    f.propose("old", "alice", &[pid(1)], 101);
    f.keep(101);
    f.close("old");
    // An accept of the same artifact is under way (its intent is open): the file is what it is accepting.
    f.changes
        .open_or_resume_intent(
            "i-resend",
            &f.repo.repo_id,
            &recorded(101),
            100,
            &[],
            "alice",
            T0,
        )
        .unwrap();
    let r = f.svc.sweep();
    assert_eq!((r.removed, r.kept), (0, 1), "{r:?}");
    assert!(f.kept(101));
}

#[test]
fn a_file_already_gone_is_counted_and_is_not_an_error() {
    let f = Fixture::new("sweep-gone", after_close_plus_grace, 200);
    f.propose("old", "alice", &[pid(1)], 101);
    f.close("old"); // the file was never kept (or was removed by hand)
    let r = f.svc.sweep();
    assert_eq!((r.removed, r.already_gone, r.errors), (0, 1, 0), "{r:?}");
}

#[test]
fn a_sweep_looks_at_a_bounded_batch_and_the_next_run_goes_on() {
    let f = Fixture::new("sweep-batch", after_close_plus_grace, 2);
    for n in 1..=5u64 {
        f.propose(&format!("c{n}"), "alice", &[pid(n)], 100 + n);
        f.keep(100 + n);
        f.close(&format!("c{n}"));
    }
    let mut removed = 0;
    for _ in 0..3 {
        let r = f.svc.sweep();
        assert!(r.examined <= 2, "{r:?}");
        removed += r.removed;
    }
    assert_eq!(removed, 5, "three runs of two clear five");
    assert!((1..=5u64).all(|n| !f.kept(100 + n)));
}

#[test]
fn a_sweep_names_what_it_did_in_the_metrics() {
    let f = Fixture::new("sweep-metrics", after_close_plus_grace, 200);
    f.propose("p", "alice", &[pid(1)], 101);
    f.keep(101);
    f.close("p");
    f.svc.sweep();
    let s = f.metrics.snapshot();
    assert_eq!(
        (s.sweeps, s.sweep_removed, s.sweep_removed_bytes),
        (1, 1, 100)
    );
    let _ = (&f.export, &f.hosting);
}
