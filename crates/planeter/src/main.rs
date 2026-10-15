#![forbid(unsafe_code)]
//! `planeter` — the forge server binary.
//!
//! The read/host spine (RFC 001–003) as a runnable server: create/host repositories over sandboxed
//! prikk and browse them through the browse UI and the authorized, honest read API; sign in with a
//! local password (Argon2id) or OpenID Connect, behind sessions, CSRF protection and login throttles;
//! every store persists in **SQLite**. Clone/push (RFC 004) is not here yet. `--version`/`-V` and
//! `--help`/`-h` answer without reading configuration (see [`args`]); any other argument is refused
//! with exit 2. Configuration is by environment variable:
//!
//! - `PLANETER_ADDR` — bind address (default `127.0.0.1:8080`)
//! - `PLANETER_REPOS_ROOT` — where hosted repositories live (default `./planeter-repos`)
//! - `PLANETER_CONTENT_ORIGIN` — the isolated origin for raw bytes (default `http://127.0.0.1:8080`)
//! - `PLANETER_DB` — the SQLite database file (default `./planeter.db`; created if absent)
//! - `PLANETER_TRUSTED_PROXIES` — comma-separated addresses/CIDRs of the TLS-terminating reverse proxies
//!   (client IPs are taken from `X-Forwarded-For` only behind these). **A non-loopback `PLANETER_ADDR`
//!   is refused unless this is set**: planeter speaks plain HTTP and must sit behind TLS termination.
//! - `PLANETER_PRIKK_CONCURRENCY` — the most prikk subprocesses running at once, a positive integer
//!   (default `min(available_parallelism, 8)`). Read-path prikk work runs off the async workers.
//! - `PLANETER_PRIKK_QUEUE_WAIT_SECS` — how long a request waits for a slot before it is refused with
//!   `503` and `Retry-After`, a positive integer (default 10).
//! - `PLANETER_AUTH_CONCURRENCY` — the most password verifications (Argon2id) running at once, a
//!   positive integer (default `min(available_parallelism, 4)`); a separate bound from the prikk one.
//! - `PLANETER_AUTH_QUEUE_WAIT_SECS` — how long a sign-in waits for a slot before it is refused with
//!   `503` and `Retry-After`, a positive integer (default 5). Unknown and existing accounts alike.
//! - `PLANETER_OIDC_ISSUER`, `PLANETER_OIDC_CLIENT_ID`, `PLANETER_OIDC_REDIRECT_URI` (all three to enable
//!   SSO) and optional `PLANETER_OIDC_CLIENT_SECRET` — the OpenID Connect provider. Discovery, JWKS and the
//!   token exchange go through the egress guard and a confined `curl` (a runtime prerequisite like prikk
//!   and bubblewrap). Accounts are linked to `(issuer, subject)` administratively; nothing is
//!   auto-provisioned.

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
use planeter_web::{AppState, ContentOrigin};

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
        content_origin: ContentOrigin::new(content_origin),
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
