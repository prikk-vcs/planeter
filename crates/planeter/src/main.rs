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
    // fail later with a misleading "No such file or directory", so a missing one refuses to start here,
    // in the same way a prikk below the floor does.
    if let Err(detail) = planeter_prikk::sandbox::probe_prlimit(std::ffi::OsStr::new(
        planeter_prikk::sandbox::PRLIMIT,
    )) {
        eprintln!(
            "planeter: refusing to start: `prlimit` (part of util-linux) is required to bound the memory of prikk calls, \
             but it could not be run ({detail}). Install util-linux (the package that provides `prlimit`) and start again."
        );
        std::process::exit(1);
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
    match planeter_transport::DirsConfig::from_vars(
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
        }
        Err(e) => {
            eprintln!("planeter: {e}");
            exit_releasing_lock(1);
        }
    }

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
            leases
        }
        Err(e) => {
            eprintln!("planeter: cannot initialise the lease records: {e}");
            exit_releasing_lock(1);
        }
    };
    let hosting = Arc::new(HostingService::new(repos_root, repo_store));
    let membership: Arc<dyn MembershipStore> = Arc::new(SqliteMembershipStore::new(db.clone()));
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));

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
    let leases = Arc::new(leases);
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
