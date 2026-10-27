#![forbid(unsafe_code)]
//! `planeter` — the forge server binary.
//!
//! The read/host spine (RFC 001–003) as a runnable server: create/host repositories over sandboxed
//! prikk and browse them through the browse UI and the authorized, honest read API; sign in with a
//! local password (Argon2id) or OpenID Connect, behind sessions, CSRF protection and login throttles;
//! every store persists in **SQLite**. Clone/push (RFC 004) is not here yet. `--version`/`-V` and
//! `--help`/`-h` answer without reading configuration (see [`args`]); any other argument is refused
//! with exit 2. Configuration is by environment variable; `planeter --help` lists every variable, with
//! its default (the list lives in `args::USAGE` and nowhere else).

mod args;

use std::sync::Arc;

use planeter_auth::{
    Argon2idHasher, Authenticator, Sha256TokenHasher, SqliteAccountStore, SqliteCredentialStore,
    SqliteSessionStore,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    MembershipStore, RepositoryStore, SqliteDb, SqliteLeaseRecords, SqliteMembershipStore,
    SqliteRepositoryStore,
};
use planeter_web::AppState;

/// The single-process lock (RFC 004 D-7), held for the life of the server. A static so that every refusal to
/// start after it was taken can remove the file before exiting.
static PROCESS_LOCK: std::sync::Mutex<Option<planeter_core::ProcessLock>> =
    std::sync::Mutex::new(None);

fn exit_releasing_lock(code: i32) -> ! {
    drop(PROCESS_LOCK.lock().ok().and_then(|mut l| l.take()));
    std::process::exit(code)
}

#[tokio::main]
async fn main() {
    // Identity and usage first (RFC 010 D-6): before any configuration is read or socket opened, so an
    // operator or a release check can ask the binary what it is without starting a server.
    match args::parse(std::env::args_os().skip(1)) {
        args::Action::Serve => {}
        args::Action::Version => {
            print!(
                "{}",
                args::version_text(args::GIT_COMMIT, planeter_prikk::cli::MIN_PRIKK_VERSION)
            );
            return;
        }
        args::Action::Help => {
            print!("{}", args::USAGE);
            return;
        }
        args::Action::Unknown => {
            eprintln!("planeter: unrecognized argument");
            eprint!("{}", args::USAGE);
            std::process::exit(2);
        }
    }

    let addr: std::net::SocketAddr = std::env::var("PLANETER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .expect("PLANETER_ADDR must be a socket address");
    let repos_root =
        std::env::var("PLANETER_REPOS_ROOT").unwrap_or_else(|_| "./planeter-repos".to_owned());
    let content_origin =
        std::env::var("PLANETER_CONTENT_ORIGIN").unwrap_or_else(|_| format!("http://{addr}"));
    let db_path = std::env::var("PLANETER_DB").unwrap_or_else(|_| "./planeter.db".to_owned());
    let trusted_proxies = match planeter_web::TrustedProxies::parse(
        &std::env::var("PLANETER_TRUSTED_PROXIES").unwrap_or_default(),
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("planeter: PLANETER_TRUSTED_PROXIES: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = planeter_web::bind_allowed(addr, &trusted_proxies) {
        eprintln!("planeter: {e}");
        std::process::exit(1);
    }

    // IS-16: raw bytes are isolated only if the content origin is a different host. A production-shaped
    // bind with the application's own host is refused; a loopback bind warns once.
    let content_origin =
        planeter_web::ContentOrigin::new(content_origin).with_app_host(addr.ip().to_string());
    match planeter_web::check_content_origin(&content_origin, addr) {
        Ok(planeter_web::ContentOriginCheck::Isolated) => {}
        Ok(planeter_web::ContentOriginCheck::SameHostLoopback) => {
            eprintln!("{}", planeter_web::WARN_SAME_HOST);
        }
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }

    // RFC 011 D-2: `prlimit` (util-linux) bounds the memory of every prikk call. Without it a read would
    // fail later with a misleading "No such file or directory", so a missing one refuses to start here.
    if let Err(detail) = planeter_prikk::sandbox::probe_prlimit(std::ffi::OsStr::new(
        planeter_prikk::sandbox::PRLIMIT,
    )) {
        eprintln!(
            "planeter: refusing to start: `prlimit` (part of util-linux) is required to bound the memory of prikk calls, \
             but it could not be run ({detail}). Install util-linux (the package that provides `prlimit`) and start again."
        );
        std::process::exit(1);
    }

    // The prikk binary is asked its version once, here, and the floor is applied: a prikk below `MIN_PRIKK_VERSION` refuses
    // to start with the pin's own sentence, instead of answering every read with "temporarily unavailable" after start. A
    // missing prikk is reported when a repository is first opened, as it always was; a prikk that runs and cannot be read
    // is said once and left to the pin at that first open.
    match planeter_prikk::cli::probe_binary(std::ffi::OsStr::new("prikk")) {
        planeter_prikk::cli::BinaryProbe::Supported(_)
        | planeter_prikk::cli::BinaryProbe::Missing => {}
        planeter_prikk::cli::BinaryProbe::Unsupported(sentence) => {
            eprintln!("planeter: refusing to start: {sentence}");
            std::process::exit(1);
        }
        planeter_prikk::cli::BinaryProbe::Unreadable(why) => {
            eprintln!(
                "planeter: warning: could not read the version of prikk ({why}); each repository open applies the version pin"
            );
        }
    }

    // RFC 004 D-7: the repository lease is in-process, so a second server against one database is refused here,
    // before the database is opened. The lock file sits next to the database and is removed on a normal exit; a
    // killed server leaves it, and the next start recognises it as stale.
    match planeter_core::ProcessLock::acquire(format!("{db_path}.lock")) {
        Ok(lock) => *PROCESS_LOCK.lock().expect("lock slot") = Some(lock),
        Err(e) => {
            eprintln!("planeter: refusing to start: {e}");
            std::process::exit(1);
        }
    }

    // RFC 004 D-2: the artifact store and the export spool. Unset means not configured (nothing uses them yet).
    // A directory inside the repository root, or two that overlap, is refused here.
    let dirs = match planeter_transport::DirsConfig::from_vars(
        std::env::var("PLANETER_ARTIFACT_DIR").ok().as_deref(),
        std::env::var("PLANETER_SPOOL_DIR").ok().as_deref(),
        std::env::var("PLANETER_SPOOL_BUDGET_MB").ok().as_deref(),
        std::path::Path::new(&repos_root),
    )
    .map_err(|e| e.to_string())
    .and_then(|c| {
        planeter_transport::TransportDirs::open(&c)
            .map(|d| (c, d))
            .map_err(|e| e.to_string())
    }) {
        Ok((config, dirs)) => {
            if let Some(a) = &dirs.artifacts {
                eprintln!(
                    "planeter: artifact store at {} ({} orphaned partial file(s) removed)",
                    a.root().display(),
                    a.cleaned_partials()
                );
            }
            if let Some(s) = &dirs.spool {
                eprintln!(
                    "planeter: export spool at {} (budget {} MiB, {} orphaned partial file(s) removed)",
                    s.root().display(),
                    config.spool_budget_bytes >> 20,
                    s.cleaned_partials()
                );
            }
            dirs
        }
        Err(e) => {
            eprintln!("planeter: {e}");
            exit_releasing_lock(1);
        }
    };

    // RFC 012 D-3f: a draft language (Japanese, not yet signed off by its reviewers) is offered only when the
    // operator asks for it. Off by default; logged once.
    match planeter_web::catalog::enable_preview_languages(
        &std::env::var("PLANETER_PREVIEW_LANGUAGES").unwrap_or_default(),
    ) {
        Ok(tags) if tags.is_empty() => {}
        Ok(tags) => eprintln!(
            "planeter: PREVIEW languages on ({}): these catalogs are drafts that their reviewers have not signed off; \
             turn them off by unsetting PLANETER_PREVIEW_LANGUAGES",
            tags.join(", ")
        ),
        Err(e) => {
            eprintln!("planeter: PLANETER_PREVIEW_LANGUAGES: {e}");
            exit_releasing_lock(1);
        }
    }

    // One SQLite database for every store; production hashers.
    let db = match SqliteDb::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("planeter: cannot open database {db_path}: {e}");
            exit_releasing_lock(1);
        }
    };
    let repo_store: Arc<dyn RepositoryStore> = Arc::new(SqliteRepositoryStore::new(db.clone()));

    // The repository lease (RFC 004 D-7). Nothing takes a lease yet (the transport routes come later); what a
    // previous process left in the durable holder table is reported, so an operator sees it.
    let leases = match SqliteLeaseRecords::new(db.clone()) {
        Ok(records) => {
            let leases = planeter_core::LeaseManager::new(
                Arc::new(records),
                planeter_core::new_boot_id(),
                || {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                },
                std::time::Duration::from_secs(5),
            );
            match leases.orphans() {
                Ok(o) if o.is_empty() => {}
                Ok(o) => eprintln!(
                    "planeter: {} exclusive repository lease(s) were held by a server that stopped without releasing them (listed in the lease_holders table)",
                    o.len()
                ),
                Err(e) => eprintln!("planeter: cannot read the lease records: {e}"),
            }
            Arc::new(leases)
        }
        Err(e) => {
            eprintln!("planeter: cannot initialise the lease records: {e}");
            exit_releasing_lock(1);
        }
    };
    let hosting = Arc::new(HostingService::new(repos_root, repo_store.clone()));
    let membership: Arc<dyn MembershipStore> = Arc::new(SqliteMembershipStore::new(db.clone()));
    let read = Arc::new(ReadService::new(
        hosting.clone(),
        membership.clone(),
        Arc::new(NullAuditSink),
    ));

    // RFC 004 part 7: creating a repository. **The start-up sweep runs first of everything that reads the repositories or the
    // lease records**: a server that was killed while it was creating a repository left a `creating` record, perhaps a
    // directory, a grant and a lease record, and all of it is removed (and logged) before any of it is looked at.
    let (creation_runtime, import_bound) = {
        let stage_store: Arc<dyn planeter_store::StageStore> =
            match planeter_store::SqliteStageStore::new(db.clone()) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!("planeter: cannot initialise the stage records: {e}");
                    exit_releasing_lock(1);
                }
            };
        let defaults = planeter_transport::CreationConfig::default();
        let cfg = planeter_transport::CreationConfig {
            max_archive_bytes: positive_env(
                "PLANETER_IMPORT_MAX_MB",
                defaults.max_archive_bytes >> 20,
            ) << 20,
            deadline: std::time::Duration::from_secs(positive_env(
                "PLANETER_CREATE_DEADLINE_SECS",
                defaults.deadline.as_secs(),
            )),
            stage_limits: planeter_store::StageLimits {
                per_principal: defaults.stage_limits.per_principal,
                total_bytes: positive_env(
                    "PLANETER_STAGE_BUDGET_MB",
                    defaults.stage_limits.total_bytes >> 20,
                ) << 20,
            },
            user_quota: positive_env("PLANETER_REPO_QUOTA_USER", defaults.user_quota),
            org_quota: positive_env("PLANETER_REPO_QUOTA_ORG", defaults.org_quota),
            ..defaults
        };
        let per_hour = positive_env(
            "PLANETER_CREATIONS_PER_HOUR",
            u64::from(planeter_core::creation::DEFAULT_CREATIONS_PER_HOUR),
        ) as u32;
        let stage_root = dirs.artifacts.as_ref().map(|a| a.root().join("stages"));
        let service = match planeter_transport::CreationService::new(
            hosting.clone(),
            repo_store.clone(),
            membership.clone(),
            stage_store,
            leases.clone(),
            stage_root,
            cfg.clone(),
        ) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("planeter: cannot set up repository creation: {e}");
                exit_releasing_lock(1);
            }
        };
        let swept = service.sweep_at_start();
        eprintln!(
            "planeter: start-up sweep: {} unfinished creation(s) and {} stray director(ies) removed, {} stage(s) removed",
            swept.repositories.removed_creating.len(),
            swept.repositories.removed_orphans.len(),
            swept.stages_removed.len()
        );
        for (path, why) in &swept.repositories.refused {
            eprintln!(
                "planeter: start-up sweep left {} alone: {why}",
                path.display()
            );
        }
        if service.import_enabled() {
            eprintln!(
                "planeter: repositories can be created from an archive: up to {} MiB, {} s deadline, {} MiB of staged archives, at most {} at once; {} creation(s) an hour per person; quotas {} per user and {} per organization",
                cfg.max_archive_bytes >> 20,
                cfg.deadline.as_secs(),
                cfg.stage_limits.total_bytes >> 20,
                cfg.stage_limits.per_principal,
                per_hour,
                cfg.user_quota,
                cfg.org_quota
            );
            // Stages that are not confirmed in their day are removed every hour.
            let sweeper = service.clone();
            if let Err(e) = std::thread::Builder::new()
                .name("stage-sweep".to_owned())
                .spawn(move || {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                        let n = sweeper.sweep_stages();
                        if n > 0 {
                            eprintln!("planeter: removed {n} expired archive stage(s)");
                        }
                    }
                })
            {
                eprintln!("planeter: cannot start the stage sweep: {e}");
            }
        } else {
            eprintln!(
                "planeter: repositories cannot be created from an archive (no PLANETER_ARTIFACT_DIR); empty ones can"
            );
        }
        let bound = service.import_enabled().then_some(cfg.max_archive_bytes);
        (
            Arc::new(planeter_web::creation::CreationRuntime {
                service,
                rate: planeter_web::RateLimiter::new(
                    per_hour,
                    std::time::Duration::from_secs(3600),
                ),
            }),
            bound,
        )
    };

    // The configured size and object bounds of an accept (for the start-up message about the memory budget).
    let mut propose_cost: Option<(u64, u64)> = None;
    let mut recovery_service: Option<Arc<planeter_transport::RecoveryService>> = None;
    let mut proposals_service: Option<Arc<planeter_transport::ProposalService>> = None;
    let mut reviews_service: Option<Arc<planeter_transport::ReviewService>> = None;
    // RFC 004 part 2: advertise and fetch need the export spool; without it the routes answer `transport_off`
    // to a reader of the repository. Every setting is a positive whole number, refused at start otherwise.
    let transport = match &dirs.spool {
        None => {
            eprintln!("planeter: artifact fetches are off (no PLANETER_SPOOL_DIR)");
            None
        }
        Some(spool) => {
            let revisions = match planeter_store::SqliteRevisions::new(db.clone()) {
                Ok(r) => Arc::new(r),
                Err(e) => {
                    eprintln!("planeter: cannot initialise the revision counters: {e}");
                    exit_releasing_lock(1);
                }
            };
            let cfg = planeter_transport::ExportConfig {
                max_concurrent: positive_env("PLANETER_EXPORT_CONCURRENCY", 2) as usize,
                lease_wait: std::time::Duration::from_secs(positive_env(
                    "PLANETER_EXPORT_LEASE_WAIT_SECS",
                    5,
                )),
                deadline: std::time::Duration::from_secs(positive_env(
                    "PLANETER_EXPORT_DEADLINE_SECS",
                    600,
                )),
                data_ceiling: positive_env("PLANETER_EXPORT_MEMORY_MB", 2048) << 20,
            };
            let rate = positive_env("PLANETER_TRANSPORT_RATE_PER_MIN", 120) as u32;
            eprintln!(
                "planeter: artifact fetches on: at most {} export(s) at once, {} s deadline per prikk call, {} MiB memory ceiling, {} request(s) a minute per client and per signed-in user",
                cfg.max_concurrent,
                cfg.deadline.as_secs(),
                cfg.data_ceiling >> 20,
                rate
            );
            let metrics = Arc::new(planeter_transport::Metrics::new());
            let export = Arc::new(planeter_transport::ExportService::new(
                hosting.clone(),
                spool.clone(),
                leases.clone(),
                revisions,
                cfg,
                metrics.clone(),
            ));
            // RFC 004 part 3b: proposals need the artifact store (accepted artifacts are retained there).
            let propose = match &dirs.artifacts {
                None => {
                    eprintln!("planeter: proposals are off (no PLANETER_ARTIFACT_DIR)");
                    None
                }
                Some(artifacts) => {
                    let changes: Arc<dyn planeter_store::ChangeStore> =
                        match planeter_store::SqliteChangeStore::new(db.clone()) {
                            Ok(c) => Arc::new(c),
                            Err(e) => {
                                eprintln!("planeter: cannot initialise the change records: {e}");
                                exit_releasing_lock(1);
                            }
                        };
                    let pcfg = planeter_transport::ProposeConfig {
                        max_artifact_bytes: positive_env(
                            "PRIKK_EXCHANGE_MAX_BYTES",
                            planeter_transport::propose::DEFAULT_MAX_ARTIFACT_BYTES,
                        ),
                        max_objects: positive_env("PRIKK_EXCHANGE_MAX_OBJECTS", 5_000),
                        accept_deadline: std::time::Duration::from_secs(positive_env(
                            "PLANETER_ACCEPT_DEADLINE_SECS",
                            600,
                        )),
                        lease_wait: std::time::Duration::from_secs(positive_env(
                            "PLANETER_PROPOSE_LEASE_WAIT_SECS",
                            5,
                        )),
                        max_concurrent: positive_env("PLANETER_PROPOSE_CONCURRENCY", 2) as usize,
                        repo_capacity_bytes: positive_env("PLANETER_PROPOSAL_CAPACITY_MB", 4096)
                            << 20,
                        proposer_bytes_limit: std::env::var("PLANETER_PROPOSER_QUOTA_MB")
                            .ok()
                            .filter(|v| !v.is_empty())
                            .map(|_| positive_env("PLANETER_PROPOSER_QUOTA_MB", 1) << 20),
                    };
                    eprintln!(
                        "planeter: proposals on: artifacts up to {} MiB and {} objects, {} s accept deadline, at most {} at once, repository capacity {} MiB",
                        pcfg.max_artifact_bytes >> 20,
                        pcfg.max_objects,
                        pcfg.accept_deadline.as_secs(),
                        pcfg.max_concurrent,
                        pcfg.repo_capacity_bytes >> 20
                    );
                    propose_cost = Some((pcfg.max_artifact_bytes, pcfg.max_objects));
                    // Recovery (RFC 004 part 4): the records of its runs, and the procedure itself.
                    let recoveries: Arc<dyn planeter_store::RecoveryStore> =
                        match planeter_store::SqliteRecoveries::new(db.clone()) {
                            Ok(r) => Arc::new(r),
                            Err(e) => {
                                eprintln!("planeter: cannot initialise the recovery records: {e}");
                                exit_releasing_lock(1);
                            }
                        };
                    let recovery = Arc::new(planeter_transport::RecoveryService::new(
                        hosting.clone(),
                        leases.clone(),
                        changes.clone(),
                        recoveries,
                        export.clone(),
                        planeter_transport::RecoveryConfig::default(),
                        metrics.clone(),
                    ));
                    // Unattended recovery is on unless the operator turns it off: the administrator's door
                    // (GET/POST …/recovery) works either way.
                    let unattended = match std::env::var("PLANETER_AUTOMATIC_RECOVERY")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .as_deref()
                    {
                        None | Some("on") => true,
                        Some("off") => false,
                        Some(other) => {
                            eprintln!(
                                "planeter: PLANETER_AUTOMATIC_RECOVERY must be `on` or `off` (got {other:?})"
                            );
                            exit_releasing_lock(1);
                        }
                    };
                    eprintln!(
                        "planeter: recovery of a repository left unfinished by a write: {}",
                        if unattended {
                            "unattended when every witness agrees, otherwise by an administrator"
                        } else {
                            "by an administrator only (PLANETER_AUTOMATIC_RECOVERY=off)"
                        }
                    );
                    // RFC 005 part 5: what a person sees of the proposals, and the retention sweep of the artifacts
                    // that nothing needs any more (records stay; only files go).
                    let grace_days = positive_env("PLANETER_PROPOSAL_GRACE_DAYS", 30);
                    let sweep_minutes = positive_env("PLANETER_RETENTION_SWEEP_MINUTES", 60);
                    let proposals = Arc::new(planeter_transport::ProposalService::new(
                        hosting.clone(),
                        changes.clone(),
                        export.clone(),
                        artifacts.clone(),
                        metrics.clone(),
                        planeter_transport::ProposalConfig {
                            grace_secs: grace_days.saturating_mul(24 * 60 * 60),
                            ..planeter_transport::ProposalConfig::default()
                        },
                    ));
                    eprintln!(
                        "planeter: a closed or landed proposal keeps its files for {grace_days} day(s); the retention sweep runs at start and every {sweep_minutes} minute(s)"
                    );
                    let sweeper = proposals.clone();
                    let spawned = std::thread::Builder::new()
                        .name("retention-sweep".to_owned())
                        .spawn(move || {
                            loop {
                                sweeper.sweep();
                                std::thread::sleep(std::time::Duration::from_secs(
                                    sweep_minutes.saturating_mul(60),
                                ));
                            }
                        });
                    if let Err(e) = spawned {
                        eprintln!("planeter: cannot start the retention sweep: {e}");
                    }
                    proposals_service = Some(proposals);
                    // RFC 005 part 6: comments and decisions, over the same database. The limits are per principal
                    // per proposal per hour and are computed from the records.
                    let review_store: Arc<dyn planeter_store::ReviewStore> =
                        match planeter_store::SqliteReviewStore::new(db.clone()) {
                            Ok(r) => Arc::new(r),
                            Err(e) => {
                                eprintln!("planeter: cannot initialise the review records: {e}");
                                exit_releasing_lock(1);
                            }
                        };
                    let comments_per_hour = positive_env(
                        "PLANETER_COMMENTS_PER_HOUR",
                        planeter_core::review::COMMENTS_PER_HOUR_DEFAULT,
                    );
                    let decisions_per_hour = positive_env(
                        "PLANETER_DECISIONS_PER_HOUR",
                        planeter_core::review::DECISIONS_PER_HOUR_DEFAULT,
                    );
                    eprintln!(
                        "planeter: review: at most {comments_per_hour} comment(s) and {decisions_per_hour} decision(s) per person per proposal per hour"
                    );
                    reviews_service = Some(Arc::new(planeter_transport::ReviewService::new(
                        changes.clone(),
                        review_store,
                        planeter_transport::ReviewConfig {
                            comments_per_hour,
                            decisions_per_hour,
                        },
                    )));
                    let mut service = planeter_transport::ProposeService::new(
                        hosting.clone(),
                        artifacts.clone(),
                        leases.clone(),
                        changes,
                        export.clone(),
                        pcfg,
                        metrics,
                    );
                    if unattended {
                        service = service.with_recovery(recovery.clone());
                    }
                    // A server that was killed in the middle of a write comes back with the repository flagged ...
                    service.reconcile_at_start();
                    // ... and every flagged repository gets one unattended try, on a thread of its own so that the
                    // server is listening meanwhile (a flagged repository refuses writes and still answers reads).
                    let at_start = recovery.clone();
                    if unattended {
                        std::thread::spawn(move || {
                            for (repo, outcome) in at_start.recover_flagged_at_start() {
                                match outcome {
                                    Some(o) => eprintln!(
                                        "planeter: recovery at start: repository {} {}",
                                        repo.as_str(),
                                        o.code()
                                    ),
                                    None => eprintln!(
                                        "planeter: recovery at start: repository {} is left to an administrator (GET …/recovery)",
                                        repo.as_str()
                                    ),
                                }
                            }
                        });
                    }
                    recovery_service = Some(recovery);
                    Some(Arc::new(service))
                }
            };
            Some(Arc::new(planeter_web::TransportRuntime {
                export,
                propose,
                recovery: recovery_service,
                proposals: proposals_service,
                reviews: reviews_service,
                ip_limit: planeter_web::RateLimiter::new(rate, std::time::Duration::from_secs(60)),
                principal_limit: planeter_web::RateLimiter::new(
                    rate,
                    std::time::Duration::from_secs(60),
                ),
            }))
        }
    };

    // The bound on concurrent prikk invocations and on the wait for a slot (IS-4). Logged once.
    let gate = match planeter_web::PrikkGate::from_vars(
        std::env::var("PLANETER_PRIKK_CONCURRENCY").ok().as_deref(),
        std::env::var("PLANETER_PRIKK_QUEUE_WAIT_SECS")
            .ok()
            .as_deref(),
    ) {
        Ok(g) => Arc::new(g),
        Err(e) => {
            eprintln!("planeter: {e}");
            exit_releasing_lock(1);
        }
    };
    eprintln!(
        "planeter: at most {} concurrent prikk invocations; a request waits up to {} s for a slot, then gets 503",
        gate.concurrency(),
        gate.wait().as_secs()
    );

    // RFC 011 D-1: the read memory budget. Default: a quarter of the memory this process may use (its
    // cgroup limit, else physical memory), floored at 256 MiB and capped at 8 GiB; an explicit value is
    // never capped. The log line says which one it is and how to change it.
    let budget_var = std::env::var("PLANETER_PRIKK_MEMORY_BUDGET_MB").ok();
    let usable = match budget_var.as_deref() {
        // An explicit value needs no probe of the host.
        Some(v) if !v.is_empty() => 0,
        _ => match planeter_web::budget::read_usable_memory() {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "planeter: {e}; set {} to the memory budget for reads, in MiB",
                    planeter_web::budget::VAR
                );
                exit_releasing_lock(1);
            }
        },
    };
    let (budget_bytes, budget_source) =
        match planeter_web::budget::budget_from_var(budget_var.as_deref(), usable) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("planeter: {e}");
                exit_releasing_lock(1);
            }
        };
    let gate = Arc::new(
        Arc::try_unwrap(gate)
            .expect("the gate is not shared yet")
            .with_budget(budget_bytes),
    );
    // The budget is split in a light pool (small reads) and a heavy pool (the rest) — RFC 011 R-1.
    let (light_bytes, heavy_bytes) = gate.pool_bytes().unwrap_or((0, budget_bytes));
    eprintln!(
        "planeter: read memory budget {} MiB ({}): light pool {} MiB, heavy pool {} MiB; change it with {}",
        (light_bytes + heavy_bytes) / planeter_web::budget::MIB,
        budget_source.label(),
        light_bytes / planeter_web::budget::MIB,
        heavy_bytes / planeter_web::budget::MIB,
        planeter_web::budget::VAR
    );

    // A write draws from the heavy pool too (RFC 004 D-5.3): say whether the budget admits the largest accept.
    if let Some((bound, objects)) = propose_cost {
        let (_, line) =
            planeter_web::budget::accept_startup_line(bound, objects, heavy_bytes, budget_bytes);
        eprintln!("planeter: {line}");
    }

    // Creation from an archive draws from the heavy pool as well: say whether the budget admits the largest archive.
    if let Some(bound) = import_bound {
        let admitted = planeter_transport::creation::cost_model::admission_cost(bound);
        if admitted <= heavy_bytes {
            eprintln!(
                "planeter: the largest archive ({} MiB) needs {} MiB of the heavy pool ({} MiB): admitted",
                bound >> 20,
                admitted >> 20,
                heavy_bytes >> 20
            );
        } else {
            let largest =
                planeter_transport::creation::cost_model::largest_archive_admitted(heavy_bytes)
                    .map_or(0, |b| b >> 20);
            eprintln!(
                "planeter: the largest archive ({} MiB) needs {} MiB of the heavy pool ({} MiB), so it is NOT admitted: archives above about {} MiB are refused; raise {} or lower PLANETER_IMPORT_MAX_MB",
                bound >> 20,
                admitted >> 20,
                heavy_bytes >> 20,
                largest,
                planeter_web::budget::VAR
            );
        }
    }

    // The separate, small bound on password verification (IS-24). Logged once.
    let auth_gate = match planeter_web::BlockingGate::auth_from_vars(
        std::env::var("PLANETER_AUTH_CONCURRENCY").ok().as_deref(),
        std::env::var("PLANETER_AUTH_QUEUE_WAIT_SECS")
            .ok()
            .as_deref(),
    ) {
        Ok(g) => Arc::new(g),
        Err(e) => {
            eprintln!("planeter: {e}");
            exit_releasing_lock(1);
        }
    };
    eprintln!(
        "planeter: at most {} concurrent password verifications; a sign-in waits up to {} s for a slot, then gets 503",
        auth_gate.concurrency(),
        auth_gate.wait().as_secs()
    );

    let (accounts, credentials) = match (
        SqliteAccountStore::new(db.clone()),
        SqliteCredentialStore::new(db.clone()),
    ) {
        (Ok(a), Ok(c)) => (a, c),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("planeter: cannot initialise auth tables: {e}");
            exit_releasing_lock(1);
        }
    };
    let auth = Arc::new(Authenticator::new(
        Arc::new(accounts),
        Arc::new(credentials),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    ));

    let sessions = match SqliteSessionStore::new(db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("planeter: cannot initialise sessions: {e}");
            exit_releasing_lock(1);
        }
    };

    let oidc = match std::env::var("PLANETER_OIDC_ISSUER") {
        Ok(issuer) if !issuer.is_empty() => {
            let need = |k: &str| match std::env::var(k) {
                Ok(v) if !v.is_empty() => v,
                _ => {
                    eprintln!("planeter: {k} is required when PLANETER_OIDC_ISSUER is set");
                    exit_releasing_lock(1);
                }
            };
            let cfg = planeter_auth::OidcConfig {
                issuer,
                client_id: need("PLANETER_OIDC_CLIENT_ID"),
                client_secret: std::env::var("PLANETER_OIDC_CLIENT_SECRET")
                    .ok()
                    .filter(|s| !s.is_empty()),
                redirect_uri: need("PLANETER_OIDC_REDIRECT_URI"),
            };
            let fetcher = Arc::new(planeter_core::CurlFetcher::new(Box::new(
                planeter_core::StdEgressGuard::default(),
            )));
            Some(Arc::new(planeter_auth::OidcProvider::new(cfg, fetcher)))
        }
        _ => None,
    };

    let state = AppState {
        read,
        gate,
        auth_gate,
        auth,
        content_origin,
        sessions: Arc::new(sessions),
        login_throttle: Arc::new(planeter_auth::LoginThrottle::default()),
        ip_throttle: Arc::new(planeter_auth::LoginThrottle::new(20, 15 * 60)),
        trusted_proxies: Arc::new(trusted_proxies),
        oidc,
        transport,
        creation: Some(creation_runtime),
        now_unix: || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        },
    };

    // RFC 004 part 2 T0(b): graceful shutdown. The first SIGTERM or SIGINT stops accepting and lets the requests in
    // flight finish, for at most the drain; a second signal, or the end of the drain, ends the process at once.
    // Either way the lock file and this process's lease records are released.
    let drain = match drain_from(
        std::env::var("PLANETER_SHUTDOWN_DRAIN_SECS")
            .ok()
            .as_deref(),
    ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("planeter: {e}");
            exit_releasing_lock(1);
        }
    };
    let mut signals = match Signals::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("planeter: cannot listen for shutdown signals: {e}");
            exit_releasing_lock(1);
        }
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let for_signals = leases.clone();
    tokio::spawn(async move {
        let first = signals.next().await;
        eprintln!(
            "planeter: {first} received: no new requests; letting requests in flight finish (at most {} s); send the signal again to stop at once",
            drain.as_secs()
        );
        let _ = stop_tx.send(());
        tokio::select! {
            second = signals.next() => eprintln!("planeter: {second} received again: stopping at once"),
            () = tokio::time::sleep(drain) => eprintln!(
                "planeter: the drain of {} s ended with requests still running: stopping",
                drain.as_secs()
            ),
        }
        release_and_exit(&for_signals, 1);
    });

    eprintln!("planeter listening on http://{addr} (db: {db_path})");
    let served = planeter_web::serve_until(state, addr, async {
        let _ = stop_rx.await;
    })
    .await;
    if let Err(e) = served {
        eprintln!("planeter: server error: {e}");
        release_and_exit(&leases, 1);
    }
    eprintln!("planeter: stopped");
    release_and_exit(&leases, 0);
}

/// Remove this process's lease records and the lock file, then exit.
fn release_and_exit(leases: &planeter_core::LeaseManager, code: i32) -> ! {
    if let Err(e) = leases.clear_own_records() {
        eprintln!("planeter: cannot clear this process's lease records: {e}");
    }
    exit_releasing_lock(code)
}

/// A positive whole-number setting from the environment, or `default`; anything else refuses to start.
fn positive_env(name: &str, default: u64) -> u64 {
    match std::env::var(name).ok().filter(|v| !v.is_empty()) {
        None => default,
        Some(v) => match v.parse::<u64>() {
            Ok(n) if n > 0 && n <= (u64::MAX >> 20) => n,
            _ => {
                eprintln!("planeter: {name} must be a whole number greater than zero (got {v:?})");
                exit_releasing_lock(1);
            }
        },
    }
}

/// `PLANETER_SHUTDOWN_DRAIN_SECS`: the longest a graceful shutdown waits for requests in flight (default 30).
fn drain_from(value: Option<&str>) -> Result<std::time::Duration, String> {
    match value.filter(|v| !v.is_empty()) {
        None => Ok(std::time::Duration::from_secs(30)),
        Some(v) => match v.parse::<u64>() {
            Ok(n) if n > 0 => Ok(std::time::Duration::from_secs(n)),
            _ => Err(format!(
                "PLANETER_SHUTDOWN_DRAIN_SECS must be a whole number of seconds greater than zero (got {v:?})"
            )),
        },
    }
}

/// SIGTERM and SIGINT, both registered at once so that a second signal is never lost.
struct Signals {
    term: tokio::signal::unix::Signal,
    int: tokio::signal::unix::Signal,
}

impl Signals {
    fn new() -> std::io::Result<Signals> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Signals {
            term: signal(SignalKind::terminate())?,
            int: signal(SignalKind::interrupt())?,
        })
    }
    async fn next(&mut self) -> &'static str {
        tokio::select! {
            _ = self.term.recv() => "SIGTERM",
            _ = self.int.recv() => "SIGINT",
        }
    }
}
