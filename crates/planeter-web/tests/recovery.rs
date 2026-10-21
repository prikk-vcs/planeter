//! RFC 004 part 4: recovery. A repository that `propose` flagged `needs_recovery` becomes writable again by a written
//! procedure, run by itself only when **every witness agrees** and otherwise by an administrator; each step is
//! recorded. These tests use real prikk (they self-skip without it) and stale locks written exactly as prikk writes
//! them; the real-binary kill tests are in `crates/planeter/tests/propose_kill.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher, TokenHasher,
    TokenRecord,
};
use planeter_core::{
    HostingService, LeaseClass, LeaseManager, NullAuditSink, ReadService, RepoScope, Scope,
    ScopeAccess, new_boot_id,
};
use planeter_prikk::PrikkRepo;
use planeter_store::{
    ChangeStore, HolderRecord, InMemoryChangeStore, InMemoryLeaseRecords, InMemoryMembershipStore,
    InMemoryRecoveries, InMemoryRepositoryStore, InMemoryRevisions, IntentState, LeaseRecordStore,
    MembershipStore, Owner, ProposalPolicy, RecoveryStore, RepoId, RepositoryRecord,
    RepositoryStore, RevisionStore, Role, Subject, UserId, Visibility,
};
use planeter_transport::{
    ExportConfig, ExportService, Metrics, ProcessInfo, ProcessTable, RecoveryConfig,
    RecoveryOutcome, RecoveryService, Spool,
};
use planeter_web::{AppState, ContentOrigin, RateLimiter, TransportRuntime, router};

fn now() -> u64 {
    1_000_000
}

fn runs(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn skip() -> bool {
    if !runs("prikk") {
        eprintln!("skipping: needs prikk on PATH");
        return true;
    }
    false
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("recovery-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn prikk(dir: &Path, keys: &Path, args: &[&str]) {
    let out = Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A sealed repository (it `verify`s and `doctor`s clean).
fn sealed_repo(base: &Path, name: &str) -> PathBuf {
    let repo = base.join(name);
    let keys = base.join(format!("{name}-keys"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    prikk(&repo, &keys, &["setup"]);
    std::fs::write(repo.join("README.md"), b"# recovery\n").unwrap();
    prikk(&repo, &keys, &["commit", "--from-worktree", "-m", "init"]);
    prikk(&repo, &keys, &["seal", "--allow-no-audit"]);
    repo
}

/// The two locks a killed `sync accept` leaves, written as prikk writes them, recorded for a process that is gone.
fn stale_locks(repo: &Path) {
    let pid = 4_000_000; // far above any pid_max here
    for (rel, kind) in [
        (".prikk/active/default/active.lock", "active"),
        (".prikk/containers/objects.lock", "container:object-store"),
    ] {
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("pid={pid}\nkind={kind}\nnote=PR-07 lock has no stale-lock stealing yet\n"),
        )
        .unwrap();
    }
}

fn locks_listed(repo: &Path) -> usize {
    planeter_prikk::CliPrikkRepo::new(repo)
        .unlock_list()
        .unwrap()
        .len()
}

const FLAG: &str = "a write did not finish (accept_failed)";

// ------------------------------------------------------------------------------------------------ the fixture

#[derive(Default)]
struct Opts {
    binary: Option<PathBuf>,
    processes: Option<Arc<dyn ProcessTable>>,
}

struct W {
    router: Router,
    base: PathBuf,
    repo_path: PathBuf,
    record: RepositoryRecord,
    svc: Arc<RecoveryService>,
    changes: Arc<InMemoryChangeStore>,
    recoveries: Arc<InMemoryRecoveries>,
    leases: Arc<LeaseManager>,
    records: Arc<InMemoryLeaseRecords>,
    export: Arc<ExportService>,
    store: Arc<InMemoryRepositoryStore>,
}

fn repo_id() -> RepoId {
    RepoId::new("r-forge")
}

fn world(name: &str, o: Opts) -> W {
    let base = scratch(name);
    let repo_path = sealed_repo(&base, "forge");
    let private_path = base.join("private");
    let st = Command::new("cp")
        .arg("-a")
        .arg(&repo_path)
        .arg(&private_path)
        .status()
        .unwrap();
    assert!(st.success());
    let store = Arc::new(InMemoryRepositoryStore::new());
    let mut record = None;
    for (id, rname, vis, path) in [
        ("r-forge", "forge", Visibility::Public, &repo_path),
        ("r-private", "private", Visibility::Private, &private_path),
    ] {
        let r = RepositoryRecord {
            repo_id: RepoId::new(id),
            owner: Owner::User("alice".into()),
            name: rname.into(),
            visibility: vis,
            created_at: 0,
            prikk_format_version: None,
            proposals: ProposalPolicy::default(),
            path: path.clone(),
        };
        if id == "r-forge" {
            record = Some(r.clone());
        }
        store.create(r).unwrap();
    }
    let mut hosting =
        HostingService::new(base.join("root"), store.clone() as Arc<dyn RepositoryStore>)
            .with_sandbox(planeter_prikk::Sandbox::Unconfined);
    if let Some(b) = &o.binary {
        hosting = hosting.with_binary(b);
    }
    let hosting = Arc::new(hosting);
    let membership = Arc::new(InMemoryMembershipStore::new());
    membership
        .grant_repo_role(Subject::User(UserId::new("bob")), repo_id(), Role::Read)
        .unwrap();
    membership
        .grant_repo_role(Subject::User(UserId::new("dave")), repo_id(), Role::Write)
        .unwrap();
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        membership.clone() as Arc<dyn MembershipStore>,
        Arc::new(NullAuditSink),
    ));
    let creds = Arc::new(InMemoryCredentialStore::new());
    let hasher = Arc::new(InsecureStubHasher);
    for (tok, user, access) in [
        ("tok-alice", "alice", ScopeAccess::Full),
        ("tok-bob", "bob", ScopeAccess::Write),
        ("tok-carol", "carol", ScopeAccess::Write),
        ("tok-dave", "dave", ScopeAccess::Write),
    ] {
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&*hasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access,
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
    let records = Arc::new(InMemoryLeaseRecords::new());
    let leases = Arc::new(LeaseManager::new(
        records.clone() as Arc<dyn LeaseRecordStore>,
        new_boot_id(),
        now,
        Duration::from_secs(1),
    ));
    let metrics = Arc::new(Metrics::new());
    let export = Arc::new(ExportService::new(
        hosting.clone(),
        Spool::open(base.join("spool"), 64 << 20).unwrap(),
        leases.clone(),
        Arc::new(InMemoryRevisions::new()) as Arc<dyn RevisionStore>,
        ExportConfig::default(),
        metrics.clone(),
    ));
    let changes = Arc::new(InMemoryChangeStore::new());
    let recoveries = Arc::new(InMemoryRecoveries::new());
    let mut svc = RecoveryService::new(
        hosting.clone(),
        leases.clone(),
        changes.clone() as Arc<dyn ChangeStore>,
        recoveries.clone() as Arc<dyn RecoveryStore>,
        export.clone(),
        RecoveryConfig {
            lease_wait: Duration::from_secs(20),
            ..RecoveryConfig::default()
        },
        metrics,
    )
    .with_clock(now);
    if let Some(p) = o.processes {
        svc = svc.with_process_table(p);
    }
    let svc = Arc::new(svc);
    let router = router(AppState {
        read,
        gate: Arc::new(planeter_web::PrikkGate::default()),
        auth_gate: Arc::new(planeter_web::BlockingGate::auth_default()),
        auth,
        content_origin: ContentOrigin::new("https://raw.planeter.example"),
        sessions: Arc::new(planeter_auth::InMemorySessionStore::new()),
        now_unix: now,
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(planeter_web::TrustedProxies::default()),
        oidc: None,
        transport: Some(Arc::new(TransportRuntime {
            export: export.clone(),
            propose: None,
            recovery: Some(svc.clone()),
            ip_limit: RateLimiter::new(10_000, Duration::from_secs(60)),
            principal_limit: RateLimiter::new(10_000, Duration::from_secs(60)),
        })),
    });
    W {
        router,
        base,
        repo_path,
        record: record.unwrap(),
        svc,
        changes,
        recoveries,
        leases,
        records,
        export,
        store,
    }
}

impl W {
    /// The state of a repository whose accept was killed: flagged by one of our causes, the stale locks, a dead
    /// write-lease record, an open intent.
    fn killed_accept(&self) {
        self.changes
            .set_needs_recovery(&repo_id(), FLAG, now())
            .unwrap();
        stale_locks(&self.repo_path);
        self.records
            .record(&HolderRecord {
                repo_id: repo_id(),
                class: "W".into(),
                operation: "sync accept".into(),
                holder_id: "old-boot-1".into(),
                boot_id: "old-boot".into(),
                since_unix: 1,
            })
            .unwrap();
        self.changes
            .open_or_resume_intent("i1", &repo_id(), "sha256:aa", 100, &[], "alice", 5)
            .unwrap();
    }

    fn flagged(&self) -> bool {
        self.changes.repo_flags(&repo_id()).unwrap().needs_recovery
    }

    fn fingerprint(&self) -> String {
        self.export.state(&self.record).unwrap()
    }

    fn steps_of_last(&self) -> Vec<(String, Option<String>)> {
        let r = self.recoveries.recent(&repo_id(), 1).unwrap();
        r[0].steps
            .iter()
            .map(|s| (s.name.clone(), s.outcome.clone()))
            .collect()
    }
}

// ------------------------------------------------------------------------------------------------ a stand-in prikk

/// A stand-in that passes everything to the real prikk except what its control file says, so a witness can be made to
/// disagree: unreadable `unlock`, a `doctor` that reports a warning naming a repair (until that repair has been run),
/// a `doctor` error that names no repair, a `verify` that fails.
fn standin(base: &Path) -> (PathBuf, PathBuf) {
    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v prikk"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let ctrl = base.join("standin.mode");
    let repaired = base.join("standin.repaired");
    std::fs::write(&ctrl, "pass").unwrap();
    let script = base.join("prikk-standin");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
REAL={real}
MODE="$(cat {ctrl})"
if [ "$1" = unlock ] && [ -z "$2" ] && [ "$MODE" = unlock-garbage ]; then echo "this is not what unlock prints"; exit 0; fi
if [ "$1" = doctor ] && [ "$2" = --repair-wal-tail ] && [ "$MODE" = doctor-warn ]; then touch {repaired}; echo "repair run id: fake-run-1"; exit 0; fi
if [ "$1" = doctor ] && [ -z "$2" ]; then
  if [ "$MODE" = doctor-warn ] && [ ! -f {repaired} ]; then
    printf 'doctor repository: %s/.prikk\nwarning [PRIKK-DOCTOR-WAL-TAIL]: trailing partial WAL bytes: 12\n  recommendation: run `prikk doctor --repair-wal-tail`\nissue summary: errors=0, warnings=1, info=0\n' "$PWD"; exit 0
  fi
  if [ "$MODE" = doctor-error ]; then
    printf 'doctor repository: %s/.prikk\nerror [PRIKK-DOCTOR-DAMAGE]: an object container is damaged\n  recommendation: restore from a backup\nissue summary: errors=1, warnings=0, info=0\n' "$PWD"; exit 1
  fi
fi
if [ "$1" = verify ] && [ "$MODE" = verify-fails ]; then echo "verify went wrong" >&2; exit 1; fi
exec "$REAL" "$@"
"#,
            ctrl = ctrl.display(),
            repaired = repaired.display()
        ),
    )
    .unwrap();
    let st = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(st.success());
    (script, ctrl)
}

fn standin_world(name: &str) -> (W, PathBuf) {
    let base =
        std::env::temp_dir().join(format!("planeter-recovery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let (script, ctrl) = standin(&base);
    (
        world(
            name,
            Opts {
                binary: Some(script),
                ..Opts::default()
            },
        ),
        ctrl,
    )
}

// ------------------------------------------------------------------------------------------------ the happy path

#[test]
fn a_killed_accept_is_recovered_by_itself_when_every_witness_agrees_and_every_step_is_recorded() {
    if skip() {
        return;
    }
    let w = world("auto", Opts::default());
    w.killed_accept();
    let fp = w.fingerprint();
    assert_eq!(
        locks_listed(&w.repo_path),
        2,
        "the killed accept left its locks"
    );

    // The plan, read-only, says it may run unattended.
    let plan = w.svc.plan(&w.record).unwrap();
    assert!(
        plan.flagged && plan.blockers.is_empty() && plan.may_run_unattended,
        "{plan:?}"
    );
    assert_eq!(plan.cause.code(), "accept_failed");
    assert_eq!(plan.locks.len(), 2);
    assert!(plan.locks.iter().all(|l| l.expected));
    assert_eq!(plan.holders.len(), 1);
    assert!(
        plan.holders[0].dead,
        "a record of another boot is a dead holder"
    );
    assert_eq!(plan.intents.len(), 1);
    assert!(
        plan.commands
            .iter()
            .any(|c| c.contains("prikk unlock --lock") && c.contains("--force"))
    );
    assert_eq!(locks_listed(&w.repo_path), 2, "planning changed nothing");
    assert!(
        w.recoveries.recent(&repo_id(), 5).unwrap().is_empty(),
        "and wrote no record"
    );

    let r = w.svc.auto(&w.record).expect("it ran by itself");
    assert_eq!(r.outcome, RecoveryOutcome::Recovered, "{r:?}");
    assert!(!w.flagged());
    assert_eq!(locks_listed(&w.repo_path), 0, "the stale locks are cleared");
    assert!(
        w.records.list().unwrap().is_empty(),
        "the dead lease record is removed"
    );
    assert_ne!(fp, w.fingerprint(), "the state fingerprint moved");
    assert_eq!(
        w.changes.open_intents(&repo_id()).unwrap().len(),
        1,
        "an unattended run never abandons an intent"
    );
    assert!(
        planeter_prikk::CliPrikkRepo::new(&w.repo_path)
            .verify()
            .unwrap()
            .verdict
            .ok
    );

    // The record: every step, in order, with the evidence.
    let steps = w.steps_of_last();
    let names: Vec<&str> = steps.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "witnesses",
            "lease",
            "witnesses-under-lease",
            "doctor",
            "unlock-list",
            "unlock-force",
            "unlock-force",
            "unlock-list",
            "verify",
            "clear-lease-records",
            "clear-flag",
        ]
    );
    assert!(
        steps.iter().all(|(_, o)| o.as_deref() == Some("ok")),
        "{steps:?}"
    );
    let rec = &w.recoveries.recent(&repo_id(), 1).unwrap()[0];
    assert_eq!(
        (rec.actor.as_str(), rec.outcome.as_deref()),
        ("automatic", Some("recovered"))
    );
    let force = rec.steps.iter().find(|s| s.name == "unlock-force").unwrap();
    assert!(force.detail.contains("--force"), "{}", force.detail);
    assert!(
        force.detail.contains("was not used"),
        "prikk's liveness line is recorded as not used: {}",
        force.detail
    );
    assert!(
        rec.steps[0].detail.contains("cause: accept_failed"),
        "{}",
        rec.steps[0].detail
    );
    assert!(
        rec.steps[5].detail.contains("active.lock") || rec.steps[5].detail.contains("objects.lock")
    );
}

#[test]
fn a_healthy_flagged_repository_is_only_cleared_after_a_fresh_verify_and_a_second_run_does_nothing()
{
    if skip() {
        return;
    }
    let w = world("healthy", Opts::default());
    w.changes
        .set_needs_recovery(&repo_id(), FLAG, now())
        .unwrap();
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::Recovered);
    let names: Vec<String> = w.steps_of_last().into_iter().map(|(n, _)| n).collect();
    assert!(
        names.contains(&"verify".to_owned()) && !names.contains(&"unlock-force".to_owned()),
        "{names:?}"
    );
    assert!(!w.flagged());
    // Not flagged any more: nothing to do (and nothing is touched).
    let again = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(again.outcome, RecoveryOutcome::NothingToDo);
    // `auto` leaves a healthy repository alone altogether.
    assert!(w.svc.auto(&w.record).is_none());
}

#[test]
fn a_verify_that_fails_leaves_the_flag_set_and_says_so_in_the_record() {
    if skip() {
        return;
    }
    let (w, ctrl) = standin_world("verify-fails");
    w.killed_accept();
    std::fs::write(&ctrl, "verify-fails").unwrap();
    let r = w.svc.auto(&w.record).expect("it ran");
    assert_eq!(r.outcome, RecoveryOutcome::Stopped);
    assert!(w.flagged(), "no flag is cleared without a fresh verify");
    assert!(
        w.records.list().unwrap().len() == 1,
        "and the dead lease record stays"
    );
    let steps = w.steps_of_last();
    assert_eq!(
        steps.last().unwrap(),
        &("verify".to_owned(), Some("stopped".to_owned())),
        "{steps:?}"
    );
    // The locks WERE cleared before verify (recorded), which is what the plan then shows.
    assert_eq!(locks_listed(&w.repo_path), 0);
    assert!(r.stopped.unwrap().contains("verify"));
    // Verify passes again: a second run clears the flag.
    std::fs::write(&ctrl, "pass").unwrap();
    assert_eq!(
        w.svc.recover(&w.record, "alice", &[]).unwrap().outcome,
        RecoveryOutcome::Recovered
    );
    assert!(!w.flagged());
}

// ------------------------------------------------------------------------------------------------ the witnesses

/// A foreign process: one that is not a descendant of this process, with a `prikk` command line naming the repo.
/// It is made by a shell that starts it and exits, so the system adopts it.
fn spawn_foreign_prikk(base: &Path, repo: &Path) -> u32 {
    let script = base.join("prikk");
    std::fs::write(&script, "#!/bin/sh\nsleep 60\n").unwrap();
    let st = Command::new("chmod")
        .arg("+x")
        .arg(&script)
        .status()
        .unwrap();
    assert!(st.success());
    let pidfile = base.join("foreign.pid");
    let st = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "setsid sh {} {} >/dev/null 2>&1 & echo $! > {}",
            script.display(),
            repo.display(),
            pidfile.display()
        ))
        .status()
        .unwrap();
    assert!(st.success());
    std::thread::sleep(Duration::from_millis(300));
    std::fs::read_to_string(pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn kill(pid: u32) {
    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
}

#[test]
fn a_live_foreign_process_leaves_the_flag_set_changes_nothing_and_names_the_commands() {
    if skip() {
        return;
    }
    let w = world("foreign", Opts::default());
    w.killed_accept();
    let pid = spawn_foreign_prikk(&w.base, &w.repo_path);
    let plan = w.svc.plan(&w.record).unwrap();
    let codes: Vec<&str> = plan.blockers.iter().map(|b| b.code).collect();
    assert_eq!(codes, vec!["foreign_process"], "{plan:?}");
    assert!(
        plan.processes.iter().any(|p| p.pid == pid),
        "{:?}",
        plan.processes
    );
    assert!(!plan.may_run_unattended);
    assert!(
        plan.commands.iter().any(|c| c.starts_with("ps ")),
        "the commands start with how to look"
    );
    // Neither the unattended run nor an administrator's goes on.
    let fp = w.fingerprint();
    assert!(w.svc.auto(&w.record).is_none());
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::Stopped);
    assert!(r.stopped.unwrap().contains("foreign_process"));
    assert!(w.flagged());
    assert_eq!(locks_listed(&w.repo_path), 2, "no lock was touched");
    assert_eq!(w.records.list().unwrap().len(), 1);
    assert_eq!(fp, w.fingerprint());
    let steps = w.steps_of_last();
    assert_eq!(
        steps,
        vec![("witnesses".to_owned(), Some("disagree".to_owned()))],
        "it never reached a step that changes anything"
    );
    // The process goes away; the same plan now agrees, and the unattended run recovers.
    kill(pid);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        w.svc.auto(&w.record).unwrap().outcome,
        RecoveryOutcome::Recovered
    );
}

/// A process that is a descendant of this one is ours (a read of the server's own), not a foreign process.
struct OurChild(u32);
impl ProcessTable for OurChild {
    fn processes(&self) -> Vec<ProcessInfo> {
        vec![
            ProcessInfo {
                pid: std::process::id(),
                parent: 1,
                argv: vec!["planeter".into()],
            },
            ProcessInfo {
                pid: 90_001,
                parent: std::process::id(),
                argv: vec![
                    "bwrap".into(),
                    "--bind".into(),
                    self.0.to_string(),
                    "--".into(),
                    "prikk".into(),
                    "log".into(),
                ],
            },
        ]
    }
}

#[test]
fn the_servers_own_processes_are_not_foreign_and_a_stranger_command_line_naming_the_repository_is()
{
    if skip() {
        return;
    }
    struct Table(PathBuf, bool);
    impl ProcessTable for Table {
        fn processes(&self) -> Vec<ProcessInfo> {
            let dir = self.0.to_string_lossy().into_owned();
            let me = std::process::id();
            vec![
                // ours: a child of this process
                ProcessInfo {
                    pid: 91_001,
                    parent: me,
                    argv: vec![
                        "bwrap".into(),
                        "--bind".into(),
                        dir.clone(),
                        dir.clone(),
                        "--".into(),
                        "/x/prikk".into(),
                        "log".into(),
                    ],
                },
                // ours: a grandchild
                ProcessInfo {
                    pid: 91_002,
                    parent: 91_001,
                    argv: vec!["/x/prikk".into(), "log".into(), dir.clone()],
                },
                // not a prikk or bwrap command line, though it names the directory
                ProcessInfo {
                    pid: 91_003,
                    parent: 1,
                    argv: vec!["tail".into(), "-f".into(), format!("{dir}/log")],
                },
                // prikk, but of another repository
                ProcessInfo {
                    pid: 91_004,
                    parent: 1,
                    argv: vec!["prikk".into(), "log".into(), "/somewhere/else".into()],
                },
                // foreign, when asked for
                ProcessInfo {
                    pid: 91_005,
                    parent: if self.1 { 1 } else { me },
                    argv: vec![
                        "/usr/bin/prikk".into(),
                        "sync".into(),
                        "accept".into(),
                        "a".into(),
                        dir,
                    ],
                },
            ]
        }
    }
    let _ = OurChild(0);
    for (foreign, expected) in [(false, 0usize), (true, 1)] {
        let base = scratch("ancestry");
        let _ = base;
        let w = world(&format!("ancestry-{foreign}"), Opts::default());
        // Rebuild the service with this table (the repository path is only known after the world is made).
        let svc = RecoveryService::new(
            Arc::new(
                HostingService::new(
                    w.base.join("root2"),
                    w.store.clone() as Arc<dyn RepositoryStore>,
                )
                .with_sandbox(planeter_prikk::Sandbox::Unconfined),
            ),
            w.leases.clone(),
            w.changes.clone() as Arc<dyn ChangeStore>,
            w.recoveries.clone() as Arc<dyn RecoveryStore>,
            w.export.clone(),
            RecoveryConfig::default(),
            Arc::new(Metrics::new()),
        )
        .with_process_table(Arc::new(Table(w.repo_path.clone(), foreign)));
        let plan = svc.plan(&w.record).unwrap();
        assert_eq!(
            plan.processes.len(),
            expected,
            "foreign={foreign}: {:?}",
            plan.processes
        );
        if foreign {
            assert_eq!(plan.processes[0].pid, 91_005);
        }
    }
}

#[test]
fn a_lock_of_an_unknown_kind_or_outside_the_repository_is_evidence_we_do_not_have() {
    if skip() {
        return;
    }
    let w = world("lockkind", Opts::default());
    w.killed_accept();
    // An extra lock of a kind an accept never leaves.
    let odd = w.repo_path.join(".prikk/active/other/active.lock");
    std::fs::create_dir_all(odd.parent().unwrap()).unwrap();
    std::fs::write(&odd, "pid=4000000\nkind=something-new\nnote=x\n").unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    let blockers: Vec<(&str, bool)> = plan
        .blockers
        .iter()
        .map(|b| (b.code, b.overridable))
        .collect();
    assert_eq!(blockers, vec![("unexpected_lock", false)], "{plan:?}");
    assert!(plan.locks.iter().any(|l| !l.expected));
    assert!(plan.blockers[0].detail.contains("something-new"));
    assert!(w.svc.auto(&w.record).is_none());
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(
        r.outcome,
        RecoveryOutcome::Stopped,
        "an administrator's run does not override evidence of danger"
    );
    assert!(w.flagged());
    assert_eq!(
        locks_listed(&w.repo_path),
        3,
        "not one lock was cleared, including the two it would have cleared"
    );
    // Remove the odd lock and the same run recovers.
    std::fs::remove_file(&odd).unwrap();
    assert_eq!(
        w.svc.auto(&w.record).unwrap().outcome,
        RecoveryOutcome::Recovered
    );
}

#[test]
fn an_unreadable_unlock_output_stops_with_prikks_text_kept_and_changes_nothing() {
    if skip() {
        return;
    }
    let (w, ctrl) = standin_world("unreadable");
    w.killed_accept();
    std::fs::write(&ctrl, "unlock-garbage").unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    assert_eq!(plan.blockers.len(), 1);
    assert_eq!(plan.blockers[0].code, "output_unreadable");
    assert!(
        plan.blockers[0].detail.contains("unlock"),
        "{}",
        plan.blockers[0].detail
    );
    assert!(w.svc.auto(&w.record).is_none());
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::Stopped);
    assert!(w.flagged());
    assert_eq!(w.records.list().unwrap().len(), 1);
    std::fs::write(&ctrl, "pass").unwrap();
    assert_eq!(locks_listed(&w.repo_path), 2, "no lock was cleared");
}

#[test]
fn a_live_lease_holder_is_not_now_and_a_record_of_this_boot_is_a_live_holder() {
    if skip() {
        return;
    }
    let w = world("holders", Opts::default());
    w.killed_accept();
    // A lease held by this server right now (an accept in flight).
    let held = w
        .leases
        .acquire(
            &repo_id(),
            LeaseClass::Write,
            "sync accept",
            Duration::from_secs(1),
        )
        .unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    assert!(
        plan.blockers.iter().any(|b| b.code == "lease_held"),
        "{plan:?}"
    );
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::NotNow, "{r:?}");
    assert!(w.flagged());
    assert_eq!(locks_listed(&w.repo_path), 2);
    drop(held);
    // The restart case: after a restart this boot is new, so the old process's records are dead, and a record whose
    // boot id is this one's is a lease held now — the lease table, not a process id, is the authority.
    w.records
        .record(&HolderRecord {
            repo_id: repo_id(),
            class: "W".into(),
            operation: "sync accept".into(),
            holder_id: "same-boot-1".into(),
            boot_id: w.leases.boot_id().to_owned(),
            since_unix: 2,
        })
        .unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    assert!(
        plan.holders
            .iter()
            .any(|h| h.holder_id == "same-boot-1" && !h.dead)
    );
    assert!(
        plan.holders
            .iter()
            .any(|h| h.holder_id == "old-boot-1" && h.dead)
    );
    assert!(plan.blockers.iter().any(|b| b.code == "lease_held"));
    w.records.clear("same-boot-1").unwrap();
    assert_eq!(
        w.svc.auto(&w.record).unwrap().outcome,
        RecoveryOutcome::Recovered
    );
}

#[test]
fn a_flag_that_is_not_ours_is_left_to_an_administrator_who_may_go_on() {
    if skip() {
        return;
    }
    let w = world("cause", Opts::default());
    w.killed_accept();
    w.changes
        .set_needs_recovery(&repo_id(), "set by hand", now())
        .unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    assert_eq!(plan.cause.code(), "other");
    assert_eq!(
        plan.blockers
            .iter()
            .map(|b| (b.code, b.overridable))
            .collect::<Vec<_>>(),
        vec![("cause_not_ours", true)]
    );
    assert!(!plan.may_run_unattended);
    assert!(
        w.svc.auto(&w.record).is_none(),
        "the server does not clear a flag it did not set"
    );
    assert!(w.flagged());
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::Recovered, "{r:?}");
    assert_eq!(
        w.recoveries.recent(&repo_id(), 1).unwrap()[0].actor,
        "alice"
    );
}

#[test]
fn too_many_open_intents_are_left_to_an_administrator_and_a_run_never_abandons_one_unasked() {
    if skip() {
        return;
    }
    let w = world("intents", Opts::default());
    w.killed_accept();
    for i in 0..12 {
        w.changes
            .open_or_resume_intent(
                &format!("x{i}"),
                &repo_id(),
                &format!("sha256:{i:064}"),
                10,
                &[],
                "alice",
                5,
            )
            .unwrap();
    }
    let plan = w.svc.plan(&w.record).unwrap();
    assert_eq!(
        plan.blockers.iter().map(|b| b.code).collect::<Vec<_>>(),
        vec!["too_many_open_intents"]
    );
    assert!(w.svc.auto(&w.record).is_none());
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::Recovered);
    assert_eq!(
        w.changes.open_intents(&repo_id()).unwrap().len(),
        13,
        "none was abandoned"
    );
}

#[test]
fn doctor_problems_stop_the_unattended_run_and_an_administrator_may_run_only_the_repairs_doctor_names()
 {
    if skip() {
        return;
    }
    let (w, ctrl) = standin_world("doctor");
    w.killed_accept();
    // A warning that names an idempotent repair.
    std::fs::write(&ctrl, "doctor-warn").unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    assert_eq!(
        plan.blockers.iter().map(|b| b.code).collect::<Vec<_>>(),
        vec!["doctor_problems"]
    );
    let d = plan.doctor.as_ref().unwrap();
    assert!(!d.clean);
    assert_eq!(d.repairs_allowed, vec!["--repair-wal-tail"]);
    assert!(
        w.svc.auto(&w.record).is_none(),
        "an unattended run never repairs"
    );
    assert!(w.flagged());
    // An administrator's run still stops: doctor_problems is not overridable ...
    let r = w.svc.recover(&w.record, "alice", &[]).unwrap();
    assert_eq!(r.outcome, RecoveryOutcome::Stopped);
    assert_eq!(locks_listed(&w.repo_path), 2);
    // ... an error doctor names no repair for stops everyone, with doctor's text.
    std::fs::write(&ctrl, "doctor-error").unwrap();
    let plan = w.svc.plan(&w.record).unwrap();
    assert_eq!(plan.blockers[0].code, "doctor_problems");
    assert!(plan.doctor.as_ref().unwrap().repairs_allowed.is_empty());
    assert_eq!(
        w.svc.recover(&w.record, "alice", &[]).unwrap().outcome,
        RecoveryOutcome::Stopped
    );
    assert!(w.flagged());
}

// ------------------------------------------------------------------------------------------------ twice at once

#[test]
fn recovery_run_twice_at_once_is_serialized_and_recovers_once() {
    if skip() {
        return;
    }
    let w = world("twice", Opts::default());
    w.killed_accept();
    let (a, b) = {
        let (s1, r1) = (w.svc.clone(), w.record.clone());
        let (s2, r2) = (w.svc.clone(), w.record.clone());
        let h1 = std::thread::spawn(move || s1.recover(&r1, "alice", &[]).unwrap());
        let h2 = std::thread::spawn(move || s2.recover(&r2, "dave", &[]).unwrap());
        (h1.join().unwrap(), h2.join().unwrap())
    };
    let mut outcomes = vec![a.outcome, b.outcome];
    outcomes.sort_by_key(|o| o.code());
    assert_eq!(
        outcomes,
        vec![RecoveryOutcome::NothingToDo, RecoveryOutcome::Recovered],
        "{a:?} {b:?}"
    );
    assert!(!w.flagged());
    assert_eq!(locks_listed(&w.repo_path), 0);
    let runs = w.recoveries.recent(&repo_id(), 10).unwrap();
    assert_eq!(runs.len(), 2, "each run has its record");
    let unlocks: usize = runs
        .iter()
        .map(|r| r.steps.iter().filter(|s| s.name == "unlock-force").count())
        .sum();
    assert_eq!(unlocks, 2, "the locks were cleared once, by one run");
    // The unattended run never starts twice on one repository.
    w.changes
        .set_needs_recovery(&repo_id(), FLAG, now())
        .unwrap();
}

// ------------------------------------------------------------------------------------------------ the administrator's door

async fn call(
    r: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
    extra: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let resp = r
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 24)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

const RECOVERY: &str = "/api/v1/repos/alice/forge/recovery";

#[tokio::test]
async fn the_door_shows_the_plan_to_an_administrator_and_tells_nobody_else_anything() {
    if skip() {
        return;
    }
    let w = world("door", Opts::default());
    w.killed_accept();
    let (status, v) = call(&w.router, "GET", RECOVERY, Some("tok-alice"), "", &[]).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["flagged"], true);
    assert_eq!(v["plan"]["may_run_unattended"], true);
    assert_eq!(v["plan"]["cause"]["code"], "accept_failed");
    assert!(
        v["plan"]["cause"]["text"]
            .as_str()
            .unwrap()
            .contains("failed part-way")
    );
    assert_eq!(v["plan"]["locks"].as_array().unwrap().len(), 2);
    assert_eq!(v["plan"]["holders"][0]["dead"], true);
    assert_eq!(v["plan"]["intents"][0]["artifact_bytes"], 100);
    assert!(v["plan"]["commands"].as_array().unwrap().len() >= 4);
    assert!(v["recoveries"].as_array().unwrap().is_empty());

    // Not an administrator: a writer, a reader, a signed-in stranger of a public repository, the anonymous.
    for token in [Some("tok-dave"), Some("tok-bob"), Some("tok-carol"), None] {
        let (status, v) = call(&w.router, "GET", RECOVERY, token, "", &[]).await;
        assert_eq!(
            (status, v["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("forbidden")),
            "{token:?}"
        );
        let (status, _) = call(&w.router, "POST", RECOVERY, token, "", &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token:?}");
    }
    // A hidden repository and an absent one are the same 404, for GET and for POST.
    for method in ["GET", "POST"] {
        let hidden = call(
            &w.router,
            method,
            "/api/v1/repos/alice/private/recovery",
            Some("tok-carol"),
            "",
            &[],
        )
        .await;
        let absent = call(
            &w.router,
            method,
            "/api/v1/repos/alice/ghost/recovery",
            Some("tok-carol"),
            "",
            &[],
        )
        .await;
        assert_eq!(
            (hidden.0, &hidden.1),
            (StatusCode::NOT_FOUND, &absent.1),
            "{method}"
        );
        let anon = call(
            &w.router,
            method,
            "/api/v1/repos/alice/private/recovery",
            None,
            "",
            &[],
        )
        .await;
        assert_eq!(anon.0, StatusCode::NOT_FOUND);
    }
    // A session cookie is not looked at.
    use planeter_auth::SessionStore;
    let sessions = planeter_auth::InMemorySessionStore::new();
    let s = sessions
        .create(UserId::new("alice"), now(), now() + 100)
        .unwrap();
    let (status, _) = call(
        &w.router,
        "GET",
        RECOVERY,
        None,
        "",
        &[("cookie", &format!("planeter_session={}", s.id.0))],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Nothing above changed anything.
    assert!(w.flagged());
    assert_eq!(locks_listed(&w.repo_path), 2);
    // Other methods: 405 with the right Allow.
    let resp = w
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(RECOVERY)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(resp.headers().get("allow").unwrap(), "GET, POST");
}

#[tokio::test]
async fn the_door_runs_recovery_abandons_the_intents_it_is_asked_to_and_refuses_an_unknown_one() {
    if skip() {
        return;
    }
    let w = world("door-run", Opts::default());
    w.killed_accept();
    // The plan before the run (for the review request: `PLANETER_PRINT_RECOVERY_SAMPLE=<dir>` writes it and the record).
    let (_, before) = call(&w.router, "GET", RECOVERY, Some("tok-alice"), "", &[]).await;
    // An unknown intent id refuses the whole request before anything is touched.
    let (status, v) = call(
        &w.router,
        "POST",
        RECOVERY,
        Some("tok-alice"),
        r#"{"abandon_intents":["nope"]}"#,
        &[],
    )
    .await;
    assert_eq!(
        (status, v["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("unknown_intent")),
        "{v}"
    );
    assert!(w.flagged());
    assert_eq!(locks_listed(&w.repo_path), 2);
    assert!(w.recoveries.recent(&repo_id(), 5).unwrap().is_empty());
    // An unknown field in the body is a bad request.
    let (status, _) = call(
        &w.router,
        "POST",
        RECOVERY,
        Some("tok-alice"),
        r#"{"surprise":1}"#,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The run, asking for the intent to be abandoned.
    let (status, v) = call(
        &w.router,
        "POST",
        RECOVERY,
        Some("tok-alice"),
        r#"{"abandon_intents":["i1"]}"#,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["outcome"], "recovered");
    assert!(
        v["outcome_text"]
            .as_str()
            .unwrap()
            .contains("writable again")
    );
    assert!(v["recovery_id"].as_str().unwrap().starts_with("rec-"));
    assert!(!w.flagged());
    assert_eq!(
        w.changes.get_intent("i1").unwrap().unwrap().state,
        IntentState::Abandoned
    );
    // The record is on the plan afterwards, with its steps, attributed to the administrator.
    let (_, v) = call(&w.router, "GET", RECOVERY, Some("tok-alice"), "", &[]).await;
    assert_eq!(v["plan"]["flagged"], false);
    let run = &v["recoveries"][0];
    assert_eq!(run["actor"], "alice");
    assert_eq!(run["outcome"], "recovered");
    let names: Vec<&str> = run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"abandon-intent")
            && names.contains(&"verify")
            && names.contains(&"clear-flag"),
        "{names:?}"
    );
    if let Ok(dir) = std::env::var("PLANETER_PRINT_RECOVERY_SAMPLE") {
        std::fs::write(
            format!("{dir}/plan-before.json"),
            serde_json::to_string_pretty(&before["plan"]).unwrap(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/record-after.json"),
            serde_json::to_string_pretty(&v["recoveries"][0]).unwrap(),
        )
        .unwrap();
    }
    // Run again: nothing to do.
    let (_, v) = call(&w.router, "POST", RECOVERY, Some("tok-alice"), "", &[]).await;
    assert_eq!(v["outcome"], "nothing_to_do");
}

#[tokio::test]
async fn a_run_that_stops_answers_200_with_the_plan_and_the_commands() {
    if skip() {
        return;
    }
    let w = world("door-stop", Opts::default());
    w.killed_accept();
    let odd = w.repo_path.join(".prikk/active/other/active.lock");
    std::fs::create_dir_all(odd.parent().unwrap()).unwrap();
    std::fs::write(odd, "pid=4000000\nkind=something-new\n").unwrap();
    let (status, v) = call(&w.router, "POST", RECOVERY, Some("tok-alice"), "", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["outcome"], "stopped");
    assert!(v["stopped"].as_str().unwrap().contains("unexpected_lock"));
    assert_eq!(v["plan"]["blockers"][0]["code"], "unexpected_lock");
    assert_eq!(v["plan"]["blockers"][0]["overridable"], false);
    assert!(
        v["plan"]["blockers"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unexpected kind")
    );
    assert!(
        v["plan"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("prikk unlock"))
    );
    assert!(w.flagged());
}

#[test]
fn a_lock_that_is_a_link_to_somewhere_else_is_not_the_repositorys_and_a_repository_behind_a_link_still_is()
 {
    if skip() {
        return;
    }
    let w = world("symlink", Opts::default());
    w.killed_accept();
    // A lock that is a link to a file outside the repository.
    let outside = w.base.join("outside.lock");
    std::fs::write(&outside, "pid=4000000\nkind=active\nnote=elsewhere\n").unwrap();
    let link = w.repo_path.join(".prikk/active/other/active.lock");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    // prikk itself refuses to list a lock that is a link (its own i/o error), which stops the procedure as an
    // unreadable output; our own check (the lock resolved must be inside the repository) is the second fence.
    let plan = w.svc.plan(&w.record).unwrap();
    assert!(
        !plan.blockers.is_empty() && plan.blockers.iter().all(|b| !b.overridable),
        "{plan:?}"
    );
    assert!(!plan.may_run_unattended);
    assert_eq!(
        w.svc.recover(&w.record, "alice", &[]).unwrap().outcome,
        RecoveryOutcome::Stopped
    );
    assert!(w.flagged());
    assert!(w.records.list().unwrap().len() == 1, "nothing was cleared");
    std::fs::remove_file(&link).unwrap();

    // The repository is registered by a path that goes through a symbolic link: the same locks are recognised.
    let alias = w.base.join("alias");
    std::os::unix::fs::symlink(&w.repo_path, &alias).unwrap();
    let mut record = w.record.clone();
    record.path = alias;
    let plan = w.svc.plan(&record).unwrap();
    assert!(plan.blockers.is_empty(), "{plan:?}");
    assert!(plan.locks.iter().all(|l| l.expected));
}
