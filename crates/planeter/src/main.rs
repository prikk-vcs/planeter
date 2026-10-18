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
    MembershipStore, RepositoryStore, SqliteDb, SqliteMembershipStore, SqliteRepositoryStore,
};
use planeter_web::AppState;

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

    // One SQLite database for every store; production hashers.
    let db = match SqliteDb::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("planeter: cannot open database {db_path}: {e}");
            std::process::exit(1);
        }
    };
    let repo_store: Arc<dyn RepositoryStore> = Arc::new(SqliteRepositoryStore::new(db.clone()));
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
            std::process::exit(1);
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
                std::process::exit(1);
            }
        },
    };
    let (budget_bytes, budget_source) =
        match planeter_web::budget::budget_from_var(budget_var.as_deref(), usable) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("planeter: {e}");
                std::process::exit(1);
            }
        };
    eprintln!(
        "planeter: read memory budget {} MiB ({}); change it with {}",
        budget_bytes / planeter_web::budget::MIB,
        budget_source.label(),
        planeter_web::budget::VAR
    );
    let gate = Arc::new(
        Arc::try_unwrap(gate)
            .expect("the gate is not shared yet")
            .with_budget(budget_bytes),
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
            std::process::exit(1);
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
            std::process::exit(1);
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
            std::process::exit(1);
        }
    };

    let oidc = match std::env::var("PLANETER_OIDC_ISSUER") {
        Ok(issuer) if !issuer.is_empty() => {
            let need = |k: &str| match std::env::var(k) {
                Ok(v) if !v.is_empty() => v,
                _ => {
                    eprintln!("planeter: {k} is required when PLANETER_OIDC_ISSUER is set");
                    std::process::exit(1);
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

    eprintln!("planeter listening on http://{addr} (db: {db_path})");
    if let Err(e) = planeter_web::serve(state, addr).await {
        eprintln!("planeter: server error: {e}");
        std::process::exit(1);
    }
}
