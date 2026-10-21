//! Confinement of the prikk subprocess (RFC 001 T4 / D-3, threat model INV-3 / C-4c; RFC 011 D-2 and D-5).
//!
//! The prikk subprocess parses **untrusted pushed bytes**, so it runs confined: writable access to
//! only its repository directory, **no network**, a wall-time bound and, when the caller sets one, a
//! **memory ceiling**. The mechanism (owner-ruled 2026-09-16) is **bubblewrap** (`bwrap`): a user-namespace
//! sandbox that read-only-binds the system, makes only the repo dir writable, unshares the network, and
//! dies with its parent. A dev-only [`Sandbox::Unconfined`] runs prikk directly — **never** for hosting
//! untrusted input.
//!
//! # Process model (RFC 011 D-2, D-5)
//!
//! - **`bwrap` is the server's direct child.** There is no `timeout` hop: `--die-with-parent` ties the
//!   whole namespace to the server, so a killed server no longer leaves `bwrap`/prikk running (IS-37). The
//!   wall-time bound is kept **by the server** ([`Sandbox::run`]): at the deadline it sends `SIGKILL` and
//!   reports the same status the old `timeout` did ([`TIMEOUT_EXIT_CODE`]).
//! - **Spawn and wait must happen on one thread.** `--die-with-parent` is `PR_SET_PDEATHSIG` on the parent
//!   *thread*: if the thread that spawned `bwrap` exits while the process lives, `bwrap` and the call are
//!   killed — a healthy 14 s call died at exactly 10.0 s when its spawning tokio blocking-pool thread
//!   expired (investigation I3). [`Sandbox::run`] spawns and waits on the calling thread and never hands
//!   the child to another one; keep it that way (a test pins it).
//! - **The memory ceiling is `prlimit --data=<N> --` in front of the vector** ([`CallLimits`]): safe code,
//!   deterministic (the limit is set before `bwrap` is exec'd and inherited by everything under it), about
//!   0.15 ms. It needs `prlimit` (util-linux), which [`probe_prlimit`] checks at start. A call that hits
//!   the ceiling is reported as `PrikkError::CeilingExceeded`, never as a verdict.
//!
//! The artifact size/count/ratio bounds that matter most for untrusted input are enforced by planeter
//! *before* handing bytes to prikk (RFC 004 transport), not here.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The exit status reported for a call killed at the wall-time deadline: the convention of coreutils
/// `timeout` (124), kept so the error callers already saw does not change when `timeout` goes away.
pub const TIMEOUT_EXIT_CODE: i32 = 124;

/// The memory-ceiling wrapper (util-linux). A runtime prerequisite since RFC 011 D-2.
pub const PRLIMIT: &str = "prlimit";

/// Per-call limits applied to the prikk subprocess.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallLimits {
    /// `RLIMIT_DATA` for the call, in bytes (`None` = no ceiling). Applied as `prlimit --data=<N> --`.
    pub data_ceiling: Option<u64>,
}

impl CallLimits {
    /// No limits: what production passes until admission supplies a value (RFC 011 D-1).
    pub const NONE: CallLimits = CallLimits { data_ceiling: None };

    /// A data ceiling of `bytes`.
    pub const fn data_ceiling(bytes: u64) -> Self {
        CallLimits {
            data_ceiling: Some(bytes),
        }
    }
}

/// What a bounded run produced.
#[derive(Debug)]
pub struct Run {
    /// The child's exit status (for a deadline kill: the `SIGKILL` status; check [`Run::timed_out`]).
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The wall-time bound fired and the server killed the call.
    pub timed_out: bool,
}

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

/// One extra mount of the sandbox (RFC 004 D-6). The list is closed on purpose: the transport needs exactly
/// two, and each is named for what it is for. A mount is a directory outside the repository, bound at the same
/// path inside the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountKind {
    /// `--ro-bind`: prikk reads it, cannot write it.
    ReadOnly,
    /// `--bind`: prikk reads and writes it.
    ReadWrite,
}

/// The optional mounts: where accepted artifacts are read from (**artifact-in**, read-only) and where an
/// export is written to (**spool-out**, read-write). Both are `None` by default and then add nothing to the
/// vector: [`BwrapConfig::default`] confines exactly as before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtraMounts {
    /// Read-only: the artifact store (for `sync accept`, `bundle import`, `archive import`).
    pub artifact_in: Option<PathBuf>,
    /// Read-write: the export spool (for `bundle export`, `archive export`, `sync export`).
    pub spool_out: Option<PathBuf>,
}

/// Why a set of mounts was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountError {
    /// A mount path is not absolute or contains `..`.
    NotAbsolute(PathBuf),
    /// The two mounts, or a mount and the repository, overlap: a directory would be reachable through both.
    Overlap(PathBuf, PathBuf),
}

impl std::fmt::Display for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountError::NotAbsolute(p) => write!(
                f,
                "a sandbox mount must be an absolute path without `..` (got {})",
                p.display()
            ),
            MountError::Overlap(a, b) => write!(
                f,
                "sandbox mounts must not overlap ({} and {})",
                a.display(),
                b.display()
            ),
        }
    }
}

impl std::error::Error for MountError {}

impl ExtraMounts {
    /// The mounts that are set, in a fixed order: artifact-in, then spool-out.
    fn list(&self) -> impl Iterator<Item = (MountKind, &Path)> {
        [
            (MountKind::ReadOnly, self.artifact_in.as_deref()),
            (MountKind::ReadWrite, self.spool_out.as_deref()),
        ]
        .into_iter()
        .filter_map(|(k, p)| p.map(|p| (k, p)))
    }

    /// Check the mounts against the repository directory `root` of a call: absolute, no `..`, and no overlap
    /// between them or with `root`.
    pub fn validate(&self, root: &Path) -> Result<(), MountError> {
        let mut seen: Vec<&Path> = vec![root];
        for (_, p) in self.list() {
            if !p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(MountError::NotAbsolute(p.to_path_buf()));
            }
            for other in &seen {
                if p.starts_with(other) || other.starts_with(p) {
                    return Err(MountError::Overlap(p.to_path_buf(), other.to_path_buf()));
                }
            }
            seen.push(p);
        }
        Ok(())
    }
}

/// Tunables for the bubblewrap sandbox.
#[derive(Debug, Clone)]
pub struct BwrapConfig {
    /// The `bwrap` binary (default: `bwrap` on `PATH`).
    pub bwrap: OsString,
    /// The `prlimit` binary used for the memory ceiling (default: `prlimit` on `PATH`).
    pub prlimit: OsString,
    /// Wall-time bound, in seconds, for one prikk invocation. Enforced by the server ([`Sandbox::run`]),
    /// not by an external `timeout`.
    pub timeout_secs: u64,
    /// Incoming-artifact bounds passed to prikk through the environment (none set by default).
    pub bounds: IncomingBounds,
    /// Optional artifact-in / spool-out mounts (none by default; see [`ExtraMounts`]).
    pub mounts: ExtraMounts,
}

impl Default for BwrapConfig {
    fn default() -> Self {
        Self {
            bwrap: OsString::from("bwrap"),
            prlimit: OsString::from(PRLIMIT),
            timeout_secs: 120,
            bounds: IncomingBounds::default(),
            mounts: ExtraMounts::default(),
        }
    }
}

impl Sandbox {
    /// Build the [`Command`] that runs `prikk_binary <args>` against repository `root`, confined
    /// according to this policy, with **no per-call limits**. `prikk_binary` should be an absolute path when
    /// sandboxing (so bwrap can bind it); it is resolved via `PATH` when it is not.
    pub fn command(&self, prikk_binary: &OsStr, root: &Path, args: &[&str]) -> Command {
        self.command_with(prikk_binary, root, args, CallLimits::NONE)
    }

    /// Like [`Sandbox::command`], with per-call `limits`. A data ceiling puts `prlimit --data=<N> --` in
    /// front of the whole vector (`prlimit` then `exec`s `bwrap`, so the limit is set before anything under
    /// it starts and is inherited by all of it). No ceiling: the vector is exactly the unlimited one.
    pub fn command_with(
        &self,
        prikk_binary: &OsStr,
        root: &Path,
        args: &[&str],
        limits: CallLimits,
    ) -> Command {
        let (inner, prlimit) = match self {
            Sandbox::Unconfined => {
                let mut c = Command::new(prikk_binary);
                c.current_dir(root).args(args);
                (c, OsString::from(PRLIMIT))
            }
            Sandbox::Bwrap(cfg) => (cfg.command(prikk_binary, root, args), cfg.prlimit.clone()),
        };
        match limits.data_ceiling {
            None => inner,
            Some(bytes) => {
                let mut c = Command::new(prlimit);
                c.arg(format!("--data={bytes}"))
                    .arg("--")
                    .arg(inner.get_program())
                    .args(inner.get_args());
                if let Some(dir) = inner.get_current_dir() {
                    c.current_dir(dir);
                }
                c
            }
        }
    }

    /// The wall-time bound of one call: [`BwrapConfig::timeout_secs`] when sandboxed; none for the
    /// dev-only unconfined mode.
    pub fn deadline(&self) -> Option<Duration> {
        match self {
            Sandbox::Unconfined => None,
            Sandbox::Bwrap(cfg) => Some(Duration::from_secs(cfg.timeout_secs)),
        }
    }

    /// Run one call to completion under this policy: spawn it, collect stdout and stderr, and enforce the
    /// wall-time bound **in the server** (`SIGKILL` at the deadline; [`Run::timed_out`]).
    ///
    /// The child is spawned **and** waited for on the calling thread (see the module docs): never move
    /// the spawn or the wait to another thread.
    pub fn run(
        &self,
        prikk_binary: &OsStr,
        root: &Path,
        args: &[&str],
        limits: CallLimits,
    ) -> std::io::Result<Run> {
        if let Sandbox::Bwrap(cfg) = self {
            cfg.mounts
                .validate(root)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        }
        run_bounded(
            self.command_with(prikk_binary, root, args, limits),
            self.deadline(),
        )
    }
}

impl BwrapConfig {
    fn command(&self, prikk_binary: &OsStr, root: &Path, args: &[&str]) -> Command {
        let prikk = resolve_binary(prikk_binary);

        // `bwrap <flags> -- <prikk> <args>`: bwrap is the server's direct child (no `timeout` hop).
        let mut c = Command::new(&self.bwrap);

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
        // The optional mounts (RFC 004 D-6), only when configured: none configured, nothing added here.
        for (kind, path) in self.mounts.list() {
            c.arg(match kind {
                MountKind::ReadOnly => "--ro-bind",
                MountKind::ReadWrite => "--bind",
            })
            .arg(path)
            .arg(path);
        }
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

/// Spawn `cmd`, collect its output, and kill it at `deadline`.
///
/// Spawn and wait both happen on **this** thread: the sandbox's `--die-with-parent` binds the child to the
/// spawning *thread*, so handing the child to another thread (or spawning from one that then exits) would
/// kill healthy calls. Only the two pipe readers run elsewhere, and they never touch the child.
///
/// No polling in the common path: the wait is a `recv_timeout` on "stdout reached EOF", which happens when
/// the call is over, so a normal call adds no latency; only an exit that lags EOF is polled, briefly.
fn run_bounded(mut cmd: Command, deadline: Option<Duration>) -> std::io::Result<Run> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let started = Instant::now();
    let (mut out, mut err) = match (child.stdout.take(), child.stderr.take()) {
        (Some(o), Some(e)) => (o, e),
        _ => unreachable!("stdout and stderr were requested as pipes"),
    };
    let (eof_tx, eof_rx) = mpsc::channel::<()>();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = eof_tx.send(());
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    // 1. Wait for the call's output to end (or the deadline).
    let mut timed_out = match deadline {
        None => {
            let _ = eof_rx.recv();
            false
        }
        Some(d) => matches!(eof_rx.recv_timeout(d), Err(mpsc::RecvTimeoutError::Timeout)),
    };
    // 2. Output has ended; the exit normally follows at once. Poll only if it lags, still bounded.
    if !timed_out {
        let mut pause = Duration::from_micros(50);
        let status = loop {
            if let Some(st) = child.try_wait()? {
                break Some(st);
            }
            if let Some(d) = deadline
                && started.elapsed() >= d
            {
                timed_out = true;
                break None;
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(5));
        };
        if let Some(status) = status {
            let stdout = out_reader.join().unwrap_or_default();
            let stderr = err_reader.join().unwrap_or_default();
            return Ok(Run {
                status,
                stdout,
                stderr,
                timed_out: false,
            });
        }
    }
    // Deadline: SIGKILL the direct child (bwrap, or the `prlimit` that exec'd into it). Its
    // `--die-with-parent` takes the namespace with it. The readers are left to finish by themselves when
    // the pipes close; nothing waits on them, so a stray pipe holder can never hang the server.
    let _ = child.kill();
    let status = child.wait()?;
    drop((out_reader, err_reader));
    Ok(Run {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
        timed_out,
    })
}

/// Check that the memory-ceiling wrapper can be run (`<binary> --version`). Called once at start: since
/// RFC 011 D-2 `prlimit` (util-linux) is a runtime prerequisite, and its absence must refuse to start
/// rather than surface later as a misleading "No such file or directory" from a read.
pub fn probe_prlimit(binary: &OsStr) -> Result<(), String> {
    match Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(st) if st.success() => Ok(()),
        Ok(st) => Err(format!("`--version` exited with {st}")),
        Err(e) => Err(e.to_string()),
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
    fn bwrap_wraps_with_isolation_and_the_repo_as_the_only_writable_bind() {
        let c = Sandbox::default().command(OsStr::new("prikk"), Path::new("/repo"), &["status"]);
        // RFC 011 D-5: bwrap is the direct child — no `timeout` in front of it.
        assert_eq!(c.get_program(), OsStr::new("bwrap"));
        let args: Vec<String> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
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
