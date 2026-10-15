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
  PLANETER_CONTENT_ORIGIN       the isolated origin for raw bytes (default http://<PLANETER_ADDR>)
  PLANETER_DB                   the SQLite database file (default ./planeter.db; created if absent)
  PLANETER_TRUSTED_PROXIES      comma-separated addresses/CIDRs of the TLS-terminating reverse proxies;
                                a non-loopback PLANETER_ADDR is refused unless this is set
  PLANETER_PRIKK_CONCURRENCY    most prikk subprocesses at once (default min(CPUs, 8))
  PLANETER_PRIKK_QUEUE_WAIT_SECS  seconds a request waits for a slot before 503 (default 10)
  PLANETER_OIDC_ISSUER, PLANETER_OIDC_CLIENT_ID, PLANETER_OIDC_REDIRECT_URI
                                all three enable OpenID Connect sign-in
  PLANETER_OIDC_CLIENT_SECRET   optional OpenID Connect client secret
";
