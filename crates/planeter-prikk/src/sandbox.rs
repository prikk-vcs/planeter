//! Confinement of the prikk subprocess (RFC 001 T4 / D-3, threat model INV-3 / C-4c).
//!
//! The prikk subprocess parses **untrusted pushed bytes**, so it runs confined: writable access to
//! only its repository directory, **no network**, and a wall-time bound. The mechanism (owner-ruled
//! 2026-09-16) is **bubblewrap** (`bwrap`): a user-namespace sandbox that read-only-binds the system,
//! makes only the repo dir writable, unshares the network, and dies with its parent. A dev-only
//! [`Sandbox::Unconfined`] runs prikk directly — **never** for hosting untrusted input.
//!
//! First-cut scope: fs-scope + no-network + wall-time (via `timeout`). Memory/CPU/fsize rlimits are a
//! documented hardening follow-on; the artifact size/count/ratio bounds that matter most for untrusted
//! input are enforced by planeter *before* handing bytes to prikk (RFC 004 transport), not here.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// How the prikk subprocess is confined.
#[derive(Debug, Clone)]
pub enum Sandbox {
    /// Confine via bubblewrap (the production default).
    Bwrap(BwrapConfig),
    /// **Dev only:** run prikk directly, unconfined. Never use this to host untrusted input.
    Unconfined,
}

impl Default for Sandbox {
    fn default() -> Self {
        Sandbox::Bwrap(BwrapConfig::default())
    }
}

/// Bounds on incoming artifacts that prikk reads from its **environment** (dependency-ledger PK-31,
/// PK-40). The sandbox clears the environment, so prikk's own defaults (256 MiB / 100,000 objects)
/// apply unless a bound is set here; each `Some` value is passed as `--setenv NAME <n>` and a `None`
/// emits nothing. This is the seam only: the accept-edge values are chosen by the transport design
/// (RFC 004), not here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IncomingBounds {
    /// `PRIKK_EXCHANGE_MAX_BYTES` — largest sync exchange artifact `sync accept` will read.
    pub exchange_max_bytes: Option<u64>,
    /// `PRIKK_EXCHANGE_MAX_OBJECTS` — most patches/blobs/keys/claims an exchange artifact may declare.
    pub exchange_max_objects: Option<u64>,
    /// `PRIKK_BUNDLE_MAX_BYTES` — largest bundle `bundle import`/`preview`/`verify` will read.
    pub bundle_max_bytes: Option<u64>,
    /// `PRIKK_BUNDLE_MAX_OBJECTS` — most objects a bundle may declare.
    pub bundle_max_objects: Option<u64>,
}

impl IncomingBounds {
    /// The `(NAME, value)` pairs that are set, in a fixed order.
    fn pairs(&self) -> impl Iterator<Item = (&'static str, u64)> {
        [
            ("PRIKK_EXCHANGE_MAX_BYTES", self.exchange_max_bytes),
            ("PRIKK_EXCHANGE_MAX_OBJECTS", self.exchange_max_objects),
            ("PRIKK_BUNDLE_MAX_BYTES", self.bundle_max_bytes),
            ("PRIKK_BUNDLE_MAX_OBJECTS", self.bundle_max_objects),
        ]
        .into_iter()
        .filter_map(|(name, v)| v.map(|n| (name, n)))
    }
}

/// Tunables for the bubblewrap sandbox.
#[derive(Debug, Clone)]
pub struct BwrapConfig {
    /// The `bwrap` binary (default: `bwrap` on `PATH`).
    pub bwrap: OsString,
    /// The `timeout` binary used for the wall-time bound (default: `timeout`).
    pub timeout: OsString,
    /// Wall-time bound, in seconds, for one prikk invocation.
    pub timeout_secs: u64,
    /// Incoming-artifact bounds passed to prikk through the environment (none set by default).
    pub bounds: IncomingBounds,
}

impl Default for BwrapConfig {
    fn default() -> Self {
        Self {
            bwrap: OsString::from("bwrap"),
            timeout: OsString::from("timeout"),
            timeout_secs: 120,
            bounds: IncomingBounds::default(),
        }
    }
}

impl Sandbox {
    /// Build the [`Command`] that runs `prikk_binary <args>` against repository `root`, confined
    /// according to this policy. `prikk_binary` should be an absolute path when sandboxing (so bwrap can
    /// bind it); [`Sandbox::command`] resolves it via `PATH` when it is not.
    pub fn command(&self, prikk_binary: &OsStr, root: &Path, args: &[&str]) -> Command {
        match self {
            Sandbox::Unconfined => {
                let mut c = Command::new(prikk_binary);
                c.current_dir(root).args(args);
                c
            }
            Sandbox::Bwrap(cfg) => cfg.command(prikk_binary, root, args),
        }
    }
}

impl BwrapConfig {
    fn command(&self, prikk_binary: &OsStr, root: &Path, args: &[&str]) -> Command {
        let prikk = resolve_binary(prikk_binary);

        // Outer: `timeout <secs> bwrap <flags> -- <prikk> <args>`.
        let mut c = Command::new(&self.timeout);
        c.arg("--kill-after=5").arg(self.timeout_secs.to_string());
        c.arg(&self.bwrap);

        // A read-only view of the system so prikk finds its libraries and binary...
        c.arg("--ro-bind").arg("/usr").arg("/usr");
        // ...with the usr-merge symlinks a dynamically-linked binary needs.
        for (target, link) in [
            ("usr/lib", "/lib"),
            ("usr/lib", "/lib64"),
            ("usr/bin", "/bin"),
            ("usr/bin", "/sbin"),
        ] {
            c.arg("--symlink").arg(target).arg(link);
        }
        // The prikk binary itself (in case it lives outside /usr, e.g. ~/.cargo/bin).
        c.arg("--ro-bind").arg(&prikk).arg(&prikk);
        // Minimal /proc and /dev.
        c.arg("--proc").arg("/proc");
        c.arg("--dev").arg("/dev");
        // The one writable location: this repository.
        c.arg("--bind").arg(root).arg(root);
        c.arg("--chdir").arg(root);
        // Isolation: new namespaces (incl. network), no lingering session, die with parent.
        c.arg("--unshare-all");
        c.arg("--die-with-parent");
        c.arg("--new-session");
        // A clean environment: no HOME, no ambient config. This deliberately withholds the operator's
        // key directory from the subprocess — a keyless forge holds no user keys (INV-2). Repo-level
        // reads (log/show/verify/branch/tag/status/worktree-status/trust) need no HOME; operator-key
        // queries (`key status`) correctly cannot run confined.
        c.arg("--clearenv");
        c.arg("--setenv").arg("PATH").arg("/usr/bin");
        // Incoming-artifact bounds (PK-31/PK-40): only the ones that are set, never a default of ours.
        for (name, value) in self.bounds.pairs() {
            c.arg("--setenv").arg(name).arg(value.to_string());
        }

        c.arg("--").arg(&prikk).args(args);
        c
    }
}

/// Resolve a binary name to an absolute path via `PATH`; pass through paths that already contain a
/// separator. Falls back to the input unchanged if resolution fails (the spawn then surfaces the error).
fn resolve_binary(binary: &OsStr) -> PathBuf {
    let p = Path::new(binary);
    if p.components().count() > 1 || p.is_absolute() {
        return p.to_path_buf();
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(binary);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfined_runs_the_binary_directly() {
        let c = Sandbox::Unconfined.command(OsStr::new("prikk"), Path::new("/repo"), &["status"]);
        assert_eq!(c.get_program(), OsStr::new("prikk"));
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(args, ["status"]);
    }

    #[test]
    fn bwrap_wraps_with_timeout_isolation_and_the_repo_as_the_only_writable_bind() {
        let c = Sandbox::default().command(OsStr::new("prikk"), Path::new("/repo"), &["status"]);
        assert_eq!(c.get_program(), OsStr::new("timeout"));
        let args: Vec<String> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"bwrap".to_owned()));
        assert!(args.contains(&"--unshare-all".to_owned()));
        assert!(args.contains(&"--die-with-parent".to_owned()));
        // the repo is bound rw with --bind (not --ro-bind), and is the chdir target
        let bind = args
            .iter()
            .position(|a| a == "--bind")
            .expect("--bind present");
        assert_eq!(args[bind + 1], "/repo");
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--chdir" && w[1] == "/repo")
        );
        // prikk + its args come after the `--`
        let sep = args.iter().position(|a| a == "--").expect("-- separator");
        assert_eq!(args.last().map(String::as_str), Some("status"));
        assert!(sep < args.len() - 1);
    }

    #[test]
    fn resolve_binary_passes_through_absolute_paths() {
        assert_eq!(
            resolve_binary(OsStr::new("/opt/prikk")),
            PathBuf::from("/opt/prikk")
        );
    }
}
