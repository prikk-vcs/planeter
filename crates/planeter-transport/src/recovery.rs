//! Recovery: a repository that `propose` flagged `needs_recovery` becomes writable again by a written, tested
//! procedure (RFC 004 D-6, part 4).
//!
//! **What the procedure removes, and on whose evidence.** An accept that did not finish leaves locks of prikk's
//! storage engine behind (`active.lock`, `objects.lock`) and a prefix of its patches pending. Clearing those locks is
//! the one place planeter deliberately removes files of the storage engine, so every removal rests on **witnesses**
//! that must all agree, and every step is written to a **recovery record** before it is taken:
//!
//! 1. the flag is set, and its cause is one of ours (an unfinished accept, or a server that stopped during a write);
//! 2. the durable lease table has only **dead** holders for the repository — a record written by another boot of the
//!    server (the server runs once per database, so that process is gone); a record of *this* boot is a lease held now;
//! 3. no process on the host that is not ours has a `prikk` or `bwrap` command line naming the repository's directory;
//! 4. `doctor` reports nothing but information (stale locks are not something `doctor` reports), and `unlock` lists
//!    only locks of the kinds an accept leaves, inside the repository's own `.prikk` directory;
//! 5. both outputs are of a shape the strict readers know — otherwise the procedure stops with prikk's text kept.
//!
//! **`unlock`'s liveness line is never used** (inside our PID namespace it is wrong, U-16): the decision rests on the
//! witnesses above. The procedure takes the repository's **W lease** (so it cannot race an accept), re-reads the
//! witnesses under it, clears each listed lock with `unlock --lock <path> --force`, runs `doctor`'s repairs only for
//! what `doctor` names and only when a person asked (never automatically), requires a fresh `verify` to pass, removes
//! the dead lease records, clears the flag and moves the state fingerprint. A step that fails stops the procedure, leaves
//! the flag set, and the plan names the exact commands for an operator.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use planeter_core::admission::{ReadVerb, ceiling_for, store_size};
use planeter_core::{CallLimits, HostingService, LeaseClass, LeaseError, LeaseManager};
use planeter_prikk::PrikkRepo;
use planeter_prikk::maintenance::{DoctorReport, LockInfo, Repair, RepairPlan};
use planeter_store::{ChangeStore, IntentRecord, RecoveryStore, RepoId, RepositoryRecord};

use crate::export::ExportService;
use crate::metrics::Metrics;

/// The flag causes that are ours (`ProposeService::incomplete` writes the first five, `reconcile_at_start` the last).
pub const OUR_CAUSES: [&str; 6] = [
    "accept_deadline",
    "accept_ceiling",
    "accept_failed",
    "accept_unreadable",
    "accept_not_recorded",
    "server_stopped",
];

/// The kinds of lock an accept leaves behind. A lock of any other kind is evidence we do not have.
pub const KNOWN_LOCK_KINDS: [&str; 2] = ["active", "container:object-store"];

// ------------------------------------------------------------------------------------------------ the process table

/// What was read of a process's working directory and open files (RFC 004 part 4a, T2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// Where the process is (`/proc/<pid>/cwd`), if it could be read.
    pub cwd: Option<String>,
    /// What its open file descriptors point at (`/proc/<pid>/fd/*`).
    pub open_files: Vec<String>,
    /// Everything that was asked could be read. `false` for a process of another user (or any entry the system would
    /// not let us read): it is **not** treated as safe, and it is counted and said.
    pub complete: bool,
}

impl Inspection {
    /// Nothing to read (the process is not a candidate), or nothing known: complete, empty.
    pub fn none() -> Self {
        Inspection {
            cwd: None,
            open_files: Vec::new(),
            complete: true,
        }
    }
}

/// One process of the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent: u32,
    pub argv: Vec<String>,
    pub inspection: Inspection,
}

impl ProcessInfo {
    /// A process with a command line and nothing read of its directory and files.
    pub fn new(pid: u32, parent: u32, argv: Vec<String>) -> Self {
        ProcessInfo {
            pid,
            parent,
            argv,
            inspection: Inspection::none(),
        }
    }
}

/// The host's process table, behind a trait so a test can show a disagreeing witness.
pub trait ProcessTable: Send + Sync {
    fn processes(&self) -> Vec<ProcessInfo>;
}

/// `/proc`.
pub struct ProcFs;

impl ProcessTable for ProcFs {
    fn processes(&self) -> Vec<ProcessInfo> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return out;
        };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let raw = match std::fs::read(e.path().join("cmdline")) {
                Ok(raw) => raw,
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                    // Cannot even see what it is: counted as not inspected (never as safe).
                    out.push(ProcessInfo {
                        pid,
                        parent: 0,
                        argv: Vec::new(),
                        inspection: Inspection {
                            cwd: None,
                            open_files: Vec::new(),
                            complete: false,
                        },
                    });
                    continue;
                }
                Err(_) => continue, // gone while we looked
            };
            let argv: Vec<String> = raw
                .split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            if argv.is_empty() {
                continue; // a kernel thread, or a process that is exiting
            }
            // Only the tools can hold the repository: read where they are and what they hold.
            let inspection = if is_tool(&argv) {
                inspect(&e.path())
            } else {
                Inspection::none()
            };
            // `/proc/<pid>/stat`: "pid (comm) state ppid …"; comm may hold spaces and parentheses, so cut at the last ')'.
            let parent = std::fs::read_to_string(e.path().join("stat"))
                .ok()
                .and_then(|s| {
                    let rest = s.get(s.rfind(')')? + 2..)?.to_owned();
                    rest.split_whitespace().nth(1)?.parse::<u32>().ok()
                })
                .unwrap_or(0);
            out.push(ProcessInfo {
                pid,
                parent,
                argv,
                inspection,
            });
        }
        out
    }
}

/// Is `argv` the command line of a `prikk` or `bwrap` (the tools that can hold a repository)?
fn is_tool(argv: &[String]) -> bool {
    let base = |a: &str| a.rsplit('/').next().unwrap_or(a).to_owned();
    argv.iter()
        .take(8)
        .any(|a| matches!(base(a).as_str(), "prikk" | "bwrap"))
}

/// Is `argv` a `prikk` or `bwrap` command line that names `dir`?
fn names_repository(argv: &[String], dir: &str) -> bool {
    is_tool(argv) && argv.iter().any(|a| a.contains(dir))
}

/// Read `/proc/<pid>/cwd` and `/proc/<pid>/fd/*` of a process. Whatever cannot be read (another user's process) makes
/// the inspection incomplete.
fn inspect(proc_dir: &std::path::Path) -> Inspection {
    let mut complete = true;
    let cwd = match std::fs::read_link(proc_dir.join("cwd")) {
        Ok(p) => Some(p.to_string_lossy().into_owned()),
        Err(err) => {
            // A process that is gone is not unreadable; one we may not look at is.
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                complete = false;
            }
            None
        }
    };
    let mut open_files = Vec::new();
    match std::fs::read_dir(proc_dir.join("fd")) {
        Ok(fds) => {
            for fd in fds.flatten() {
                match std::fs::read_link(fd.path()) {
                    Ok(p) => open_files.push(p.to_string_lossy().into_owned()),
                    Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                        complete = false
                    }
                    Err(_) => {}
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => complete = false,
        Err(_) => {}
    }
    Inspection {
        cwd,
        open_files,
        complete,
    }
}

/// Does this tool process hold the repository: its directory is in it, or it has a file of its `.prikk` open?
fn holds_repository(info: &ProcessInfo, repo_real: &std::path::Path) -> bool {
    use std::path::Path;
    let store = repo_real.join(".prikk");
    info.inspection
        .cwd
        .as_deref()
        .is_some_and(|c| Path::new(c).starts_with(repo_real))
        || info
            .inspection
            .open_files
            .iter()
            .any(|f| Path::new(f).starts_with(&store))
}

/// Is the lock at `lock` inside the repository's own `.prikk` directory? Both sides are resolved, so a repository
/// reached through a symbolic link is still recognised, and a lock that is a link to somewhere else is not.
pub fn inside_repository(repo_dir: &str, lock: &str) -> bool {
    use std::path::Path;
    let root = Path::new(repo_dir).join(".prikk");
    let (Ok(root), Ok(lock_real)) = (std::fs::canonicalize(&root), std::fs::canonicalize(lock))
    else {
        return false;
    };
    lock_real.starts_with(&root) && lock_real.extension().is_some_and(|e| e == "lock")
}

// ------------------------------------------------------------------------------------------------ settings

/// Operator settings of recovery.
#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// How long a run waits for the repository's write lease before it is "not now" (default 30 s: an accept in
    /// flight may be about to finish).
    pub lease_wait: Duration,
    /// The wall-time bound of each prikk call of a run (default 600 s).
    pub deadline: Duration,
    /// An open intent older than this is shown as expired and may be abandoned by an administrator's run (the
    /// retention period of changes: 30 days).
    pub intent_expiry_secs: u64,
    /// Open intents beyond the dead accepts the lease table accounts for that automatic recovery tolerates: a
    /// refused declaration leaves an open intent that is not a crash.
    pub unattended_intent_allowance: usize,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        RecoveryConfig {
            lease_wait: Duration::from_secs(30),
            deadline: Duration::from_secs(600),
            intent_expiry_secs: 30 * 24 * 60 * 60,
            unattended_intent_allowance: 8,
        }
    }
}

// ------------------------------------------------------------------------------------------------ what a flag says

/// Whose doing a flag is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// One of [`OUR_CAUSES`].
    Ours(&'static str),
    /// Anything else (set by hand, by a later version, by a test).
    Other,
}

impl Cause {
    /// The stable code (`accept_deadline`, …, or `other`).
    pub fn code(&self) -> &'static str {
        match self {
            Cause::Ours(c) => c,
            Cause::Other => "other",
        }
    }
}

/// Read a flag's reason (`ProposeService::incomplete` writes `a write did not finish (<cause>)`; `reconcile_at_start`
/// writes `the server stopped during a write (…)`).
pub fn classify_cause(reason: &str) -> Cause {
    if reason.starts_with("the server stopped during a write") {
        return Cause::Ours("server_stopped");
    }
    if let Some(rest) = reason.strip_prefix("a write did not finish (")
        && let Some(token) = rest.strip_suffix(')')
        && let Some(c) = OUR_CAUSES.iter().find(|c| **c == token)
    {
        return Cause::Ours(c);
    }
    Cause::Other
}

// ------------------------------------------------------------------------------------------------ the plan

/// One thing that stops the procedure (or, for an administrator, only some of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    /// `cause_not_ours`, `lease_held`, `foreign_process`, `doctor_problems`, `output_unreadable`, `unexpected_lock`,
    /// `too_many_open_intents`.
    pub code: &'static str,
    /// What was seen, verbatim where it is prikk's text.
    pub detail: String,
    /// An administrator's run may go on despite it (the others always stop: they are about evidence of danger).
    pub overridable: bool,
}

/// A lease holder record of the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderView {
    pub holder_id: String,
    pub class: String,
    pub operation: String,
    pub since_unix: u64,
    /// Written by another boot of the server: that process is gone.
    pub dead: bool,
}

/// A process that is not ours and names the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcView {
    pub pid: u32,
    pub command: String,
}

/// An open intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentView {
    pub intent_id: String,
    pub principal: String,
    pub started_at: u64,
    pub attempts: u32,
    pub artifact_bytes: u64,
    /// Older than the retention period: an administrator may abandon it.
    pub expired: bool,
}

/// A lock `unlock` lists, and whether it is of a kind an accept leaves, inside the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockView {
    pub path: String,
    pub kind: String,
    pub recorded_pid: u32,
    /// prikk's own words, kept and never trusted.
    pub liveness: String,
    pub expected: bool,
}

/// What `doctor` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorView {
    pub errors: u32,
    pub warnings: u32,
    pub info: u32,
    pub clean: bool,
    /// Flags its recommendations name.
    pub repairs_named: Vec<String>,
    /// The repairs an administrator's run would run: there are some only when **every** problem is covered by a named,
    /// allow-listed, idempotent repair (the tail and index repairs).
    pub repairs_allowed: Vec<String>,
    /// Why planeter repairs nothing here, when the report has problems that are not all covered that way.
    pub repair_refusal: Option<String>,
}

/// What recovery would do and why — read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub repo_id: RepoId,
    pub flagged: bool,
    pub flag_reason: Option<String>,
    pub flag_since: Option<u64>,
    pub cause: Cause,
    pub holders: Vec<HolderView>,
    pub processes: Vec<ProcView>,
    /// How many processes could not be inspected (a warning; it stops an unattended run, not an administrator's).
    pub processes_uninspected: usize,
    pub intents: Vec<IntentView>,
    pub locks: Vec<LockView>,
    pub doctor: Option<DoctorView>,
    pub blockers: Vec<Blocker>,
    /// The exact commands for an operator when the procedure stays stopped (run in the repository's directory).
    pub commands: Vec<String>,
    /// Every witness agrees and the cause is ours: the server may run it by itself.
    pub may_run_unattended: bool,
    /// The steps a run would take, in order.
    pub steps: Vec<&'static str>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The locks were cleared where there were any, `verify` passed, the flag is cleared.
    Recovered,
    /// A witness disagreed or a step failed; the flag is still set and the plan says what to do.
    Stopped,
    /// The repository's write lease is held (an accept in flight): try again.
    NotNow,
    /// The repository was not flagged.
    NothingToDo,
}

impl Outcome {
    pub fn code(self) -> &'static str {
        match self {
            Outcome::Recovered => "recovered",
            Outcome::Stopped => "stopped",
            Outcome::NotNow => "not_now",
            Outcome::NothingToDo => "nothing_to_do",
        }
    }
}

/// The result of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub outcome: Outcome,
    /// The record of the run (absent when a run was refused before it began).
    pub recovery_id: Option<String>,
    /// The plan as it stood when the run ended.
    pub plan: Plan,
    /// Why a stopped run stopped, in a line.
    pub stopped: Option<String>,
}

/// Why a run could not even begin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// An intent to abandon is not an open intent of this repository.
    UnknownIntent(String),
    /// A store or the host failed before anything changed.
    Unavailable,
}

// ------------------------------------------------------------------------------------------------ gathered facts

struct Gathered {
    flagged: bool,
    reason: Option<String>,
    since: Option<u64>,
    holders: Vec<HolderView>,
    /// A lease this process holds right now (an accept, an export, a recovery) other than the one the run took.
    live_lease: bool,
    processes: Vec<ProcView>,
    /// Tool processes (or unidentifiable ones) whose directory and files could not be read.
    uninspected: usize,
    intents: Vec<IntentRecord>,
    doctor: Result<(DoctorReport, String), String>,
    locks: Result<(Vec<LockInfo>, String), String>,
}

fn new_id() -> String {
    let token = planeter_auth::random_token();
    format!("rec-{}", &planeter_auth::sha256_hex(token.as_bytes())[..20])
}

/// The recovery service. Cheap to share (`Arc`); every method is **blocking** (call it from `spawn_blocking`).
pub struct RecoveryService {
    hosting: Arc<HostingService>,
    leases: Arc<LeaseManager>,
    changes: Arc<dyn ChangeStore>,
    recoveries: Arc<dyn RecoveryStore>,
    export: Arc<ExportService>,
    processes: Arc<dyn ProcessTable>,
    config: RecoveryConfig,
    metrics: Arc<Metrics>,
    now_unix: fn() -> u64,
    /// Repositories an unattended run is on now (an unattended run never starts twice on one repository).
    running: Mutex<HashSet<RepoId>>,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pure: what stops the procedure, from the facts. `admin` is whether a person asked.
fn evaluate(
    g: &Gathered,
    repo_dir: &str,
    cfg: &RecoveryConfig,
    dead_writes: usize,
    now: u64,
) -> Vec<Blocker> {
    let mut out = Vec::new();
    let cause = classify_cause(g.reason.as_deref().unwrap_or(""));
    if g.flagged && cause == Cause::Other {
        out.push(Blocker {
            code: "cause_not_ours",
            detail: g.reason.clone().unwrap_or_default(),
            overridable: true,
        });
    }
    let live: Vec<&HolderView> = g.holders.iter().filter(|h| !h.dead).collect();
    if !live.is_empty() || g.live_lease {
        out.push(Blocker {
            code: "lease_held",
            detail: if live.is_empty() {
                "the repository's lease is held by this server now".to_owned()
            } else {
                live.iter()
                    .map(|h| format!("{} {} since {}", h.class, h.operation, h.since_unix))
                    .collect::<Vec<_>>()
                    .join("; ")
            },
            overridable: false,
        });
    }
    if !g.processes.is_empty() {
        out.push(Blocker {
            code: "foreign_process",
            detail: g
                .processes
                .iter()
                .map(|p| format!("{}: {}", p.pid, p.command))
                .collect::<Vec<_>>()
                .join("; "),
            overridable: false,
        });
    }
    if g.uninspected > 0 {
        out.push(Blocker {
            code: "processes_uninspected",
            detail: format!(
                "{} process(es) could not be inspected (another user's, or hidden): it cannot be ruled out that one of them is using the repository",
                g.uninspected
            ),
            // A person who knows the host may go on; the server does not guess.
            overridable: true,
        });
    }
    match &g.doctor {
        Err(text) => out.push(Blocker {
            code: "output_unreadable",
            detail: format!("doctor: {text}"),
            overridable: false,
        }),
        Ok((report, text)) if !report.is_clean() => out.push(Blocker {
            code: "doctor_problems",
            detail: format!(
                "errors={} warnings={}; {}",
                report.errors,
                report.warnings,
                text.lines()
                    .find(|l| l.starts_with("error [") || l.starts_with("warning ["))
                    .unwrap_or("")
            ),
            // An administrator's run may go on only when every problem is covered by a named, allow-listed,
            // idempotent repair (RFC 004 part 4a). One problem without such a repair keeps it a stop for everyone,
            // and an unattended run is never given the override (see `stopping`).
            overridable: matches!(report.repair_plan(), RepairPlan::Allowed(_)),
        }),
        Ok(_) => {}
    }
    match &g.locks {
        Err(text) => out.push(Blocker {
            code: "output_unreadable",
            detail: format!("unlock: {text}"),
            overridable: false,
        }),
        Ok((locks, _)) => {
            for l in locks {
                let known = KNOWN_LOCK_KINDS.contains(&l.kind.as_str());
                let inside = inside_repository(repo_dir, &l.path);
                if !known || !inside {
                    out.push(Blocker {
                        code: "unexpected_lock",
                        detail: format!("{} (kind {})", l.path, l.kind),
                        overridable: false,
                    });
                }
            }
        }
    }
    let allowance = dead_writes + cfg.unattended_intent_allowance;
    if g.intents.len() > allowance {
        out.push(Blocker {
            code: "too_many_open_intents",
            detail: format!(
                "{} open intents; {} are accounted for",
                g.intents.len(),
                allowance
            ),
            overridable: true,
        });
    }
    let _ = now;
    out
}

impl RecoveryService {
    pub fn new(
        hosting: Arc<HostingService>,
        leases: Arc<LeaseManager>,
        changes: Arc<dyn ChangeStore>,
        recoveries: Arc<dyn RecoveryStore>,
        export: Arc<ExportService>,
        config: RecoveryConfig,
        metrics: Arc<Metrics>,
    ) -> Self {
        RecoveryService {
            hosting,
            leases,
            changes,
            recoveries,
            export,
            processes: Arc::new(ProcFs),
            config,
            metrics,
            now_unix: system_now,
            running: Mutex::new(HashSet::new()),
        }
    }

    /// Test seam: the host's process table.
    #[must_use]
    pub fn with_process_table(mut self, table: Arc<dyn ProcessTable>) -> Self {
        self.processes = table;
        self
    }

    /// Test seam: the clock of the records.
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    pub fn config(&self) -> &RecoveryConfig {
        &self.config
    }

    /// The recovery records of a repository, newest first.
    pub fn recent(&self, repo: &RepoId, limit: usize) -> Vec<planeter_store::RecoveryRecord> {
        self.recoveries.recent(repo, limit).unwrap_or_default()
    }

    fn unavailable(what: &str, e: impl std::fmt::Display) -> RecoveryError {
        eprintln!("planeter: recovery: {what}: {e}");
        RecoveryError::Unavailable
    }

    /// The driver of a recovery call: the repository mounted read-write (the default), a long deadline, and the
    /// address-space ceiling of a `verify` of this repository (the dearest of the calls).
    fn driver(
        &self,
        record: &RepositoryRecord,
    ) -> Result<planeter_prikk::CliPrikkRepo, RecoveryError> {
        let size = store_size(&record.path).unwrap_or(0);
        let ceiling = ceiling_for(ReadVerb::Verify.cost(size), None);
        self.hosting
            .open_record_sandboxed(
                record,
                CallLimits::data_ceiling(ceiling),
                self.hosting
                    .sandbox()
                    .with_deadline(self.config.deadline.as_secs().max(1)),
            )
            .map_err(|e| Self::unavailable("cannot open the repository", e))
    }

    /// Everything the witnesses read. `own_holder` is the lease this run holds (it is not a live holder).
    fn gather(
        &self,
        record: &RepositoryRecord,
        driver: &planeter_prikk::CliPrikkRepo,
        own_holder: Option<&str>,
    ) -> Result<Gathered, RecoveryError> {
        let repo = &record.repo_id;
        let flags = self
            .changes
            .repo_flags(repo)
            .map_err(|e| Self::unavailable("cannot read the flag", e))?;
        let now = (self.now_unix)();
        let holders: Vec<HolderView> = self
            .leases
            .records_of(repo)
            .map_err(|e| Self::unavailable("cannot read the lease records", e))?
            .into_iter()
            .filter(|(r, _)| Some(r.holder_id.as_str()) != own_holder)
            .map(|(r, ours)| HolderView {
                holder_id: r.holder_id,
                class: r.class,
                operation: r.operation,
                since_unix: r.since_unix,
                dead: !ours,
            })
            .collect();
        // An exclusive lease this process holds that has no record (it cannot be ours alone): any lease held now
        // other than the run's own is a live holder.
        let live_lease = own_holder.is_none() && self.leases.holder_of(repo).is_some();
        let dir = record.path.to_string_lossy().into_owned();
        let me = std::process::id();
        let table = self.processes.processes();
        let parent_of: std::collections::HashMap<u32, u32> =
            table.iter().map(|p| (p.pid, p.parent)).collect();
        let ours = |mut pid: u32| {
            for _ in 0..64 {
                if pid == me {
                    return true;
                }
                match parent_of.get(&pid) {
                    Some(p) if *p != 0 && *p != pid => pid = *p,
                    _ => return false,
                }
            }
            false
        };
        let repo_real = std::fs::canonicalize(&record.path).unwrap_or_else(|_| record.path.clone());
        let mut uninspected = 0usize;
        let mut processes: Vec<ProcView> = Vec::new();
        for p in table.iter().filter(|p| p.pid != me && !ours(p.pid)) {
            let tool = is_tool(&p.argv);
            let foreign =
                tool && (names_repository(&p.argv, &dir) || holds_repository(p, &repo_real));
            if foreign {
                processes.push(ProcView {
                    pid: p.pid,
                    command: p.argv.join(" ").chars().take(300).collect(),
                });
            } else if !p.inspection.complete && (tool || p.argv.is_empty()) {
                // A tool (or something we cannot even identify) that we were not allowed to look into: not safe,
                // not unsafe — counted, and said.
                uninspected += 1;
            }
        }
        let intents = self
            .changes
            .open_intents(repo)
            .map_err(|e| Self::unavailable("cannot read the open intents", e))?;
        let _ = now;
        let doctor = match driver.doctor_report() {
            Ok(r) => {
                let text = r
                    .diagnostics
                    .iter()
                    .map(|d| format!("{:?} [{}]: {}", d.severity, d.code, d.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok((r, text))
            }
            Err(e) => Err(e.to_string()),
        };
        let locks = match driver.unlock_list() {
            Ok(l) => {
                let text = l
                    .iter()
                    .map(|l| {
                        format!(
                            "{} kind={} pid={} liveness={}",
                            l.path, l.kind, l.recorded_pid, l.liveness
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok((l, text))
            }
            Err(e) => Err(e.to_string()),
        };
        Ok(Gathered {
            flagged: flags.needs_recovery,
            reason: flags.reason,
            since: flags.since,
            holders,
            live_lease,
            processes,
            uninspected,
            intents,
            doctor,
            locks,
        })
    }

    fn plan_from(&self, record: &RepositoryRecord, g: &Gathered) -> Plan {
        let dir = record.path.to_string_lossy().into_owned();
        let now = (self.now_unix)();
        let dead_writes = g
            .holders
            .iter()
            .filter(|h| h.dead && h.class == "W")
            .count();
        let blockers = evaluate(g, &dir, &self.config, dead_writes, now);
        let cause = classify_cause(g.reason.as_deref().unwrap_or(""));
        let locks: Vec<LockView> = match &g.locks {
            Ok((locks, _)) => locks
                .iter()
                .map(|l| LockView {
                    path: l.path.clone(),
                    kind: l.kind.clone(),
                    recorded_pid: l.recorded_pid,
                    liveness: l.liveness.clone(),
                    expected: KNOWN_LOCK_KINDS.contains(&l.kind.as_str())
                        && inside_repository(&dir, &l.path),
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let doctor = g.doctor.as_ref().ok().map(|(r, _)| {
            let (repairs_allowed, repair_refusal) = match r.repair_plan() {
                RepairPlan::Allowed(v) => (v.iter().map(|x| x.flag().to_owned()).collect(), None),
                RepairPlan::Refused(why) => (Vec::new(), Some(why)),
                RepairPlan::Nothing => (Vec::new(), None),
            };
            DoctorView {
                errors: r.errors,
                warnings: r.warnings,
                info: r.info,
                clean: r.is_clean(),
                repairs_named: r.named_repairs(),
                repairs_allowed,
                repair_refusal,
            }
        });
        let mut commands = vec![
            "ps -eo pid,args | grep -E 'prikk|bwrap' | grep -F -- '".to_owned() + &dir + "'",
            format!("prikk doctor {dir}"),
            format!("cd {dir} && prikk unlock"),
        ];
        for l in locks.iter().filter(|l| l.expected) {
            commands.push(format!(
                "cd {dir} && prikk unlock --lock {} --force",
                l.path
            ));
        }
        commands.push(format!("cd {dir} && prikk verify"));
        let mut steps = vec![
            "witnesses",
            "lease",
            "witnesses-under-lease",
            "doctor",
            "unlock-list",
        ];
        if doctor
            .as_ref()
            .is_some_and(|d| !d.clean && !d.repairs_allowed.is_empty())
        {
            steps.push("repair (only if an administrator asks)");
        }
        if !locks.is_empty() {
            steps.push("unlock-force (once per lock)");
        }
        steps.extend(["verify", "clear-lease-records", "clear-flag"]);
        let may_run_unattended =
            g.flagged && blockers.is_empty() && matches!(cause, Cause::Ours(_));
        Plan {
            repo_id: record.repo_id.clone(),
            flagged: g.flagged,
            flag_reason: g.reason.clone(),
            flag_since: g.since,
            cause,
            holders: g.holders.clone(),
            processes: g.processes.clone(),
            processes_uninspected: g.uninspected,
            intents: g
                .intents
                .iter()
                .map(|i| IntentView {
                    intent_id: i.intent_id.clone(),
                    principal: i.principal.clone(),
                    started_at: i.started_at,
                    attempts: i.attempts,
                    artifact_bytes: i.artifact_bytes,
                    expired: now.saturating_sub(i.started_at) > self.config.intent_expiry_secs,
                })
                .collect(),
            locks,
            doctor,
            blockers,
            commands,
            may_run_unattended,
            steps,
        }
    }

    /// What a run would do and whether it could run unattended. **Read-only**: it runs `doctor` and `unlock`'s
    /// listing and changes nothing.
    pub fn plan(&self, record: &RepositoryRecord) -> Result<Plan, RecoveryError> {
        let driver = self.driver(record)?;
        let g = self.gather(record, &driver, None)?;
        Ok(self.plan_from(record, &g))
    }
}

// ------------------------------------------------------------------------------------------------ a run

/// The writer of one run's record: a step is written before it is taken and completed after.
struct Recorder<'a> {
    store: &'a dyn RecoveryStore,
    id: String,
    seq: u32,
    now: fn() -> u64,
}

impl Recorder<'_> {
    /// Write the step. If it cannot be written the step is **not taken**.
    fn begin(&mut self, name: &str) -> Result<u32, RecoveryError> {
        self.seq += 1;
        self.store
            .begin_step(&self.id, self.seq, name, (self.now)())
            .map_err(|e| RecoveryService::unavailable("cannot write a recovery step", e))?;
        Ok(self.seq)
    }

    fn end(&self, seq: u32, outcome: &str, detail: &str) {
        if let Err(e) = self
            .store
            .end_step(&self.id, seq, outcome, detail, (self.now)())
        {
            eprintln!("planeter: recovery: cannot complete a recovery step: {e}");
        }
    }
}

impl RecoveryService {
    /// Run the procedure for `record` on behalf of `actor` (a user id, or `automatic`). An administrator's run
    /// (`actor` other than `automatic`) may go on despite a flag that is not ours or an unusual number of open
    /// intents, may run `doctor`'s named idempotent repairs, and abandons the intents named in `abandon`; an automatic
    /// run does none of these. Every other disagreement stops both.
    pub fn recover(
        &self,
        record: &RepositoryRecord,
        actor: &str,
        abandon: &[String],
    ) -> Result<RecoveryResult, RecoveryError> {
        let admin = actor != "automatic";
        let repo = &record.repo_id;
        for id in abandon {
            match self
                .changes
                .get_intent(id)
                .map_err(|e| Self::unavailable("cannot read an intent", e))?
            {
                Some(i) if &i.repo_id == repo && i.state == planeter_store::IntentState::Open => {}
                _ => return Err(RecoveryError::UnknownIntent(id.clone())),
            }
        }
        let driver = self.driver(record)?;
        let rec_id = new_id();
        self.recoveries
            .begin(&rec_id, repo, actor, (self.now_unix)())
            .map_err(|e| Self::unavailable("cannot begin a recovery record", e))?;
        let mut rec = Recorder {
            store: &*self.recoveries,
            id: rec_id.clone(),
            seq: 0,
            now: self.now_unix,
        };
        let result = self.run_steps(record, &driver, &mut rec, admin, abandon);
        let (outcome, plan, stopped) = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = self.recoveries.end(&rec_id, "stopped", (self.now_unix)());
                return Err(e);
            }
        };
        let _ = self
            .recoveries
            .end(&rec_id, outcome.code(), (self.now_unix)());
        self.metrics.recovery(outcome.code());
        eprintln!(
            "planeter: recovery repo={} actor={} outcome={} record={}{}",
            repo.as_str(),
            if admin { "administrator" } else { "automatic" },
            outcome.code(),
            rec_id,
            stopped
                .as_deref()
                .map(|s| format!(" stopped={s}"))
                .unwrap_or_default()
        );
        Ok(RecoveryResult {
            outcome,
            recovery_id: Some(rec_id),
            plan,
            stopped,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn run_steps(
        &self,
        record: &RepositoryRecord,
        driver: &planeter_prikk::CliPrikkRepo,
        rec: &mut Recorder<'_>,
        admin: bool,
        abandon: &[String],
    ) -> Result<(Outcome, Plan, Option<String>), RecoveryError> {
        let repo = &record.repo_id;
        let dir = record.path.to_string_lossy().into_owned();

        // 1. the witnesses, before anything is taken.
        let s = rec.begin("witnesses")?;
        let g = self.gather(record, driver, None)?;
        let plan = self.plan_from(record, &g);
        rec.end(
            s,
            if plan.blockers.is_empty() {
                "ok"
            } else {
                "disagree"
            },
            &describe(&plan),
        );
        if !g.flagged && abandon.is_empty() {
            return Ok((Outcome::NothingToDo, plan, None));
        }
        if let Some(b) = stopping(&plan, admin) {
            // A live holder (an accept in flight) is "not now", not a disagreement: it may be about to finish.
            let only_a_holder = plan.blockers.iter().all(|b| b.code == "lease_held");
            let outcome = if only_a_holder {
                Outcome::NotNow
            } else {
                Outcome::Stopped
            };
            return Ok((outcome, plan, Some(b)));
        }

        // 2. the repository's write lease: a run cannot race an accept.
        let s = rec.begin("lease")?;
        let lease =
            match self
                .leases
                .acquire(repo, LeaseClass::Write, "recovery", self.config.lease_wait)
            {
                Ok(l) => l,
                Err(LeaseError::Busy { .. }) => {
                    rec.end(s, "not_now", "the repository's write lease is held");
                    return Ok((Outcome::NotNow, plan, None));
                }
                Err(e) => {
                    rec.end(s, "stopped", &e.to_string());
                    return Err(Self::unavailable("cannot take the write lease", e));
                }
            };
        rec.end(s, "ok", "write lease taken for the run");

        // 3. the witnesses again, under the lease (another run may have finished first; a holder may have appeared).
        let s = rec.begin("witnesses-under-lease")?;
        let g = self.gather(record, driver, lease.holder_id())?;
        let plan = self.plan_from(record, &g);
        rec.end(
            s,
            if plan.blockers.is_empty() {
                "ok"
            } else {
                "disagree"
            },
            &describe(&plan),
        );
        if !g.flagged && abandon.is_empty() {
            return Ok((
                Outcome::NothingToDo,
                plan,
                Some("the flag was already cleared".to_owned()),
            ));
        }
        if let Some(b) = stopping(&plan, admin) {
            return Ok((Outcome::Stopped, plan, Some(b)));
        }

        // The prikk calls whose text the record keeps: doctor and unlock's listing, as read under the lease.
        let s = rec.begin("doctor")?;
        let mut doctor = match &g.doctor {
            Ok((r, text)) => {
                rec.end(s, "ok", text);
                r.clone()
            }
            Err(text) => {
                rec.end(s, "stopped", text);
                return Ok((
                    Outcome::Stopped,
                    plan,
                    Some("doctor's output could not be read".to_owned()),
                ));
            }
        };
        let s = rec.begin("unlock-list")?;
        let locks = match &g.locks {
            Ok((l, text)) => {
                rec.end(
                    s,
                    "ok",
                    &if text.is_empty() {
                        "no locks currently held".to_owned()
                    } else {
                        text.clone()
                    },
                );
                l.clone()
            }
            Err(text) => {
                rec.end(s, "stopped", text);
                return Ok((
                    Outcome::Stopped,
                    plan,
                    Some("unlock's output could not be read".to_owned()),
                ));
            }
        };

        // 4. repairs: only what `doctor` names, only the idempotent ones, only when a person asked.
        if !doctor.is_clean() {
            // Only when every problem is covered by a named, allow-listed, idempotent repair, and only for a person.
            let wanted: Vec<Repair> = match doctor.repair_plan() {
                RepairPlan::Allowed(v) if admin => v,
                _ => {
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some("doctor reports problems that only a person may repair".to_owned()),
                    ));
                }
            };
            for repair in wanted {
                let s = rec.begin("repair")?;
                match driver.doctor_repair(repair) {
                    Ok(text) => {
                        rec.end(s, "ok", &format!("prikk doctor {}\n{text}", repair.flag()))
                    }
                    Err(e) => {
                        rec.end(
                            s,
                            "stopped",
                            &format!("prikk doctor {}\n{e}", repair.flag()),
                        );
                        return Ok((
                            Outcome::Stopped,
                            plan,
                            Some(format!("the repair {} failed", repair.flag())),
                        ));
                    }
                }
            }
            let s = rec.begin("doctor")?;
            match driver.doctor_report() {
                Ok(r) if r.is_clean() => {
                    rec.end(s, "ok", "clean after the repairs");
                    doctor = r;
                }
                Ok(r) => {
                    rec.end(
                        s,
                        "stopped",
                        &format!("errors={} warnings={}", r.errors, r.warnings),
                    );
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some("doctor is not clean after the repairs".to_owned()),
                    ));
                }
                Err(e) => {
                    rec.end(s, "stopped", &e.to_string());
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some("doctor failed after the repairs".to_owned()),
                    ));
                }
            }
        }
        let _ = &doctor;

        // 5. clear each listed lock — on the witnesses above, never on prikk's liveness line (U-16).
        for lock in &locks {
            let s = rec.begin("unlock-force")?;
            let evidence = format!(
                "witnesses agreed (flag ours, no live holder, no foreign process, doctor clean); prikk's own liveness line was `{}` and was not used",
                lock.liveness
            );
            match driver.unlock_force(&lock.path) {
                Ok(text) => rec.end(
                    s,
                    "ok",
                    &format!(
                        "prikk unlock --lock {} --force\n{evidence}\n{text}",
                        lock.path
                    ),
                ),
                Err(e) => {
                    rec.end(
                        s,
                        "stopped",
                        &format!("prikk unlock --lock {} --force\n{e}", lock.path),
                    );
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some(format!("clearing {} failed", lock.path)),
                    ));
                }
            }
        }
        if !locks.is_empty() {
            let s = rec.begin("unlock-list")?;
            match driver.unlock_list() {
                Ok(l) if l.is_empty() => rec.end(s, "ok", "no locks currently held"),
                Ok(l) => {
                    rec.end(s, "stopped", &format!("{} lock(s) remain", l.len()));
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some("locks remain after clearing".to_owned()),
                    ));
                }
                Err(e) => {
                    rec.end(s, "stopped", &e.to_string());
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some("unlock's output could not be read".to_owned()),
                    ));
                }
            }
        }

        // 6. a fresh verify must pass: no flag is cleared without it.
        let s = rec.begin("verify")?;
        match driver.verify() {
            Ok(r) if r.verdict.ok => rec.end(s, "ok", "verify: ok"),
            Ok(r) => {
                rec.end(
                    s,
                    "stopped",
                    &format!(
                        "verify did not pass: {}",
                        r.verdict
                            .failed_conditions
                            .iter()
                            .map(|c| c.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
                return Ok((
                    Outcome::Stopped,
                    plan,
                    Some("verify did not pass".to_owned()),
                ));
            }
            Err(e) => {
                rec.end(s, "stopped", &e.to_string());
                return Ok((
                    Outcome::Stopped,
                    plan,
                    Some("verify could not be run".to_owned()),
                ));
            }
        }

        // 7. the intents an administrator named are abandoned (the accepted patches stay in the repository).
        for id in abandon {
            let s = rec.begin("abandon-intent")?;
            match self.changes.abandon_intent(id, (self.now_unix)()) {
                Ok(()) => rec.end(s, "ok", &format!("intent {id} abandoned by {}", rec.id)),
                Err(e) => {
                    rec.end(s, "stopped", &e.to_string());
                    return Ok((
                        Outcome::Stopped,
                        plan,
                        Some(format!("intent {id} could not be abandoned")),
                    ));
                }
            }
        }

        // 8. the dead lease records, the flag, the fingerprint.
        let s = rec.begin("clear-lease-records")?;
        let mut cleared = Vec::new();
        for h in g.holders.iter().filter(|h| h.dead) {
            if let Err(e) = self.leases.clear_orphan(&h.holder_id) {
                rec.end(s, "stopped", &e.to_string());
                return Ok((
                    Outcome::Stopped,
                    plan,
                    Some("a dead lease record could not be removed".to_owned()),
                ));
            }
            cleared.push(format!("{} {} ({})", h.class, h.operation, h.holder_id));
        }
        rec.end(
            s,
            "ok",
            &if cleared.is_empty() {
                "none".to_owned()
            } else {
                cleared.join("; ")
            },
        );
        let s = rec.begin("clear-flag")?;
        if g.flagged {
            if let Err(e) = self.changes.clear_needs_recovery(repo) {
                rec.end(s, "stopped", &e.to_string());
                return Err(Self::unavailable("cannot clear the flag", e));
            }
            let _ = self.export.write_completed(&lease);
            rec.end(s, "ok", "flag cleared; state fingerprint moved");
        } else {
            rec.end(s, "skipped", "the repository was not flagged");
        }
        drop(lease);
        let _ = dir;
        let plan = self.plan(record).unwrap_or(plan);
        Ok((Outcome::Recovered, plan, None))
    }

    /// Run the procedure by itself if, and only if, **every witness agrees** (see the module documentation): the
    /// flag's cause is ours, nothing is held, nothing foreign is running, `doctor` and `unlock` read and are
    /// unremarkable, and the open intents are the ones the lease table accounts for. Never abandons an intent, never
    /// runs a repair, and never starts a second run on a repository it is already on. Returns what it did, or `None`
    /// if it left the repository to an administrator.
    pub fn auto(&self, record: &RepositoryRecord) -> Option<RecoveryResult> {
        {
            let mut running = self
                .running
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !running.insert(record.repo_id.clone()) {
                return None;
            }
        }
        let _guard = RunningGuard {
            set: &self.running,
            repo: record.repo_id.clone(),
        };
        // A child that was just killed may take a moment to disappear from the process table: the only disagreement
        // that is given a little time is a foreign process.
        let mut plan = self.plan(record).ok()?;
        for _ in 0..8 {
            if plan.blockers.iter().all(|b| b.code == "foreign_process")
                && !plan.blockers.is_empty()
            {
                std::thread::sleep(Duration::from_millis(250));
                plan = self.plan(record).ok()?;
            } else {
                break;
            }
        }
        if !plan.flagged {
            return None;
        }
        if !plan.may_run_unattended {
            eprintln!(
                "planeter: repository {} needs an administrator: {} (see GET …/recovery for the plan and the commands)",
                record.repo_id.as_str(),
                plan.blockers
                    .iter()
                    .map(|b| b.code)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            self.metrics.recovery("left_to_administrator");
            return None;
        }
        self.recover(record, "automatic", &[]).ok()
    }

    /// At start-up, after `reconcile_at_start`: every flagged repository gets one unattended try. Returns the
    /// repositories and what happened.
    pub fn recover_flagged_at_start(&self) -> Vec<(RepoId, Option<Outcome>)> {
        let flagged = match self.changes.flagged_repos() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("planeter: recovery: cannot list the flagged repositories: {e}");
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for repo in flagged {
            let record = match self.hosting.record(&repo) {
                Ok(Some(r)) => r,
                _ => continue,
            };
            let r = self.auto(&record);
            out.push((repo, r.map(|r| r.outcome)));
        }
        out
    }
}

struct RunningGuard<'a> {
    set: &'a Mutex<HashSet<RepoId>>,
    repo: RepoId,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.repo);
    }
}

/// The blocker that stops this run, if any (an administrator's run overrides only the overridable ones).
fn stopping(plan: &Plan, admin: bool) -> Option<String> {
    plan.blockers
        .iter()
        .find(|b| !b.overridable || !admin)
        .map(|b| format!("{}: {}", b.code, b.detail))
}

/// The witnesses, in a few lines, for the record.
fn describe(plan: &Plan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "flag: {} ({})\n",
        if plan.flagged { "set" } else { "not set" },
        plan.flag_reason.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!("cause: {}\n", plan.cause.code()));
    out.push_str(&format!(
        "lease holders: {}\n",
        if plan.holders.is_empty() {
            "none".to_owned()
        } else {
            plan.holders
                .iter()
                .map(|h| {
                    format!(
                        "{} {} {}",
                        h.class,
                        h.operation,
                        if h.dead { "dead" } else { "LIVE" }
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }
    ));
    out.push_str(&format!("foreign processes: {}\n", plan.processes.len()));
    out.push_str(&format!("open intents: {}\n", plan.intents.len()));
    out.push_str(&format!(
        "locks: {}\n",
        plan.locks
            .iter()
            .map(|l| l.path.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    ));
    if let Some(d) = &plan.doctor {
        out.push_str(&format!(
            "doctor: errors={} warnings={} info={}\n",
            d.errors, d.warnings, d.info
        ));
    }
    for b in &plan.blockers {
        out.push_str(&format!("DISAGREES {}: {}\n", b.code, b.detail));
    }
    out
}
