//! Command-line handling for the `planeter` binary (RFC 010 D-6): the binary identifies itself and never
//! starts a server by accident. Three flags are matched by hand (no argument-parsing crate).
//!
//! Everything here runs **before** configuration is read or a socket is opened, and prints nothing but
//! the version, the build's commit and the prikk floor, or the usage text: no path, host name or
//! environment value.

/// The commit this binary was built from. Injected through the `PLANETER_GIT_COMMIT` environment
/// variable at **compile time** (`option_env!`, not a build script). It is `unknown` when the variable
/// was absent, as with `cargo install` from crates.io. Because it is evaluated at compile time, an
/// incremental build can embed a stale value; the release workflow builds clean.
pub const GIT_COMMIT: &str = match option_env!("PLANETER_GIT_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// No argument: read configuration and serve.
    Serve,
    /// `--version` / `-V`.
    Version,
    /// `--help` / `-h`.
    Help,
    /// Anything else: an argument that is not understood.
    Unknown,
}

/// Interpret the arguments after the program name. Exactly one recognized flag, or none, is valid.
pub fn parse<I>(args: I) -> Action
where
    I: IntoIterator,
    I::Item: AsRef<std::ffi::OsStr>,
{
    let mut it = args.into_iter();
    let Some(first) = it.next() else {
        return Action::Serve;
    };
    if it.next().is_some() {
        return Action::Unknown;
    }
    match first.as_ref().to_str() {
        Some("--version" | "-V") => Action::Version,
        Some("--help" | "-h") => Action::Help,
        _ => Action::Unknown,
    }
}

/// The three-line identity: version, commit, prikk floor.
pub fn version_text(commit: &str, floor: (u64, u64, u64)) -> String {
    format!(
        "planeter {}\ncommit {commit}\nprikk-floor {}.{}.{}\n",
        env!("CARGO_PKG_VERSION"),
        floor.0,
        floor.1,
        floor.2
    )
}

/// The usage text: the flags and the environment variables the binary reads.
pub const USAGE: &str = "\
usage: planeter [--version | -V | --help | -h]

With no argument, planeter reads its configuration from the environment and serves.

  -V, --version   print the version, the build's commit and the prikk floor, then exit
  -h, --help      print this text, then exit

environment:
  PLANETER_ADDR                 bind address (default 127.0.0.1:8080)
  PLANETER_REPOS_ROOT           where hosted repositories live (default ./planeter-repos)
  PLANETER_CONTENT_ORIGIN       the origin raw bytes are served from; its host must differ from the
                                application's (default http://<PLANETER_ADDR>, development only: a
                                non-loopback bind with the same host is refused)
  PLANETER_DB                   the SQLite database file (default ./planeter.db; created if absent)
                                (one server per database: <file>.lock refuses a second one)
  PLANETER_ARTIFACT_DIR         directory for accepted artifacts (content-addressed; never evicted);
                                an absolute path outside PLANETER_REPOS_ROOT; unset: proposals are off
  PLANETER_SPOOL_DIR            directory for exports being served (content-addressed; the oldest are
                                evicted to stay within the budget); absolute, outside PLANETER_REPOS_ROOT,
                                and not overlapping PLANETER_ARTIFACT_DIR; unset: none
  PLANETER_SPOOL_BUDGET_MB      the spool's disk budget in MiB (default 1024; needs PLANETER_SPOOL_DIR)
  PLANETER_SHUTDOWN_DRAIN_SECS  on SIGTERM/SIGINT, seconds to let requests in flight finish before stopping
                                (default 30); a second signal stops at once
  PLANETER_TRUSTED_PROXIES      comma-separated addresses/CIDRs of the TLS-terminating reverse proxies;
                                a non-loopback PLANETER_ADDR is refused unless this is set
  PLANETER_PRIKK_CONCURRENCY    most prikk subprocesses at once (default min(CPUs, 8))
  PLANETER_PRIKK_QUEUE_WAIT_SECS  seconds a request waits for a slot before 503 (default 10)
  PLANETER_PRIKK_MEMORY_BUDGET_MB  memory reads may hold at once, in MiB (default a quarter of the memory
                                the process may use, at least 256 MiB and at most 8 GiB; a value you set
                                is never capped; a read above it is refused with 422 too_large)
  PLANETER_AUTH_CONCURRENCY     most password verifications at once (default min(CPUs, 4))
  PLANETER_AUTH_QUEUE_WAIT_SECS seconds a sign-in waits for a slot before 503 (default 5)
  PLANETER_EXPORT_CONCURRENCY   most artifact exports at once (default 2; needs PLANETER_SPOOL_DIR)
  PLANETER_EXPORT_LEASE_WAIT_SECS  seconds a fetch waits for an export slot and the repository's export lease
                                before 503 busy (default 5)
  PLANETER_EXPORT_DEADLINE_SECS wall-time bound of each prikk call of an export, export and verify (default 600)
  PLANETER_EXPORT_MEMORY_MB     memory ceiling of each prikk call of an export, in MiB (default 2048)
  PLANETER_TRANSPORT_RATE_PER_MIN  advertise and fetch requests a minute per client address and per signed-in
                                user, before 429 (default 120)
  PRIKK_EXCHANGE_MAX_BYTES      largest artifact a proposal may carry, in bytes (default 268435456 = 256 MiB):
                                refused with 413 before it is read, and again by prikk (needs PLANETER_ARTIFACT_DIR)
  PRIKK_EXCHANGE_MAX_OBJECTS    most objects an artifact may declare (default 20000); prikk refuses a larger one
  PLANETER_ACCEPT_DEADLINE_SECS wall-time bound of one accept (default 600); past it the accept is killed and the
                                repository needs recovery
  PLANETER_PROPOSE_CONCURRENCY  most proposals being uploaded or accepted at once (default 2)
  PLANETER_PROPOSE_LEASE_WAIT_SECS  seconds a proposal waits for an upload slot and the repository's write lease
                                before 503 busy (default 5)
  PLANETER_PROPOSAL_CAPACITY_MB  most megabytes of artifacts a repository retains for open and recently
                                finished changes before proposals are refused for repository capacity (default 4096)
  PLANETER_PROPOSER_QUOTA_MB    most megabytes of artifacts one person may have accepted in one repository, in all
                                (default 512, or 64 where proposals are open to any signed-in user)
  PLANETER_AUTOMATIC_RECOVERY   on (default) or off: whether the server recovers a repository left unfinished by a
                                write by itself, at start and right after the write, when every witness agrees;
                                off leaves it to an administrator (GET/POST …/recovery)
  PLANETER_PREVIEW_LANGUAGES    comma-separated draft languages to offer for review (only ja exists); a
                                draft is not signed off, so it is off by default and logged at start
  PLANETER_OIDC_ISSUER, PLANETER_OIDC_CLIENT_ID, PLANETER_OIDC_REDIRECT_URI
                                all three enable OpenID Connect sign-in
  PLANETER_OIDC_CLIENT_SECRET   optional OpenID Connect client secret
";
