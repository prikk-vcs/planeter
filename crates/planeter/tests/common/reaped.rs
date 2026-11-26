//! A server a test starts is **always stopped** (RFC 010 Amendment 1, principle 6: a gate run leaves nothing behind).
//!
//! Shared by the test binaries that start the real `planeter` server (`#[path = "common/reaped.rs"] mod reaped;`, with
//! `#[allow(dead_code)]` because each file uses part of it).
//!
//! * [`Reaped`] holds the server's [`Child`] from the moment it is spawned. Its `Drop` **kills the child (SIGKILL) and waits for it**, so
//!   a test that fails an assertion, panics, or never gets as far as its own `stop()` cannot leave the server running: the panic unwinds,
//!   the value is dropped, the process is reaped. A graceful stop in a test works as before (it sends SIGTERM, waits, and the later
//!   `Drop` finds the child already waited for: `Child::kill` then does nothing). `Reaped` dereferences to `Child`, so `child.kill()`,
//!   `child.wait()`, `child.try_wait()`, `child.id()` and `child.stderr.take()` are unchanged.
//! * What dies with the server: the server runs prikk through bubblewrap, and `bwrap` is started with `--die-with-parent` (the
//!   sandbox's own contract, `planeter-prikk/src/sandbox.rs`), so killing the server takes its helpers with it. The server is **not**
//!   put in a process group of its own: a Ctrl-C at the terminal still reaches it together with the test binary.
//! * [`server_command`] covers the one case `Drop` cannot: the **test binary itself being killed** (`timeout -s KILL`, the OOM killer,
//!   `kill -9`), where no destructor runs. It starts the server under `setpriv --pdeathsig KILL` (util-linux, a safe, separate
//!   program: the kernel kills the server when the test thread that started it is gone), when `setpriv` is on the `PATH`; without
//!   it the server is started directly and that one case is not covered (a plain `Drop` still is). No `unsafe`, no new dependency.

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::OnceLock;

/// A child process that is killed and waited for when this value is dropped.
pub struct Reaped(Child);

impl Reaped {
    /// Take over `child`.
    pub fn new(child: Child) -> Reaped {
        Reaped(child)
    }

    /// Spawn `command` and take over the child at once (before anything can panic).
    pub fn spawn(command: &mut Command) -> Reaped {
        Reaped(command.spawn().expect("spawn planeter"))
    }
}

impl Deref for Reaped {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for Reaped {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        // Already exited and waited for: both calls are no-ops. Errors are the only thing left to ignore.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `setpriv`, if it is on the `PATH` (found once; an absolute path, so that a test which gives the child its own `PATH` still finds it).
fn setpriv() -> Option<&'static PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .map(|d| d.join("setpriv"))
                .find(|p| p.is_file())
        })
        .as_ref()
}

/// The command that starts `program` (the server) so that the kernel kills it if the test that started it dies without running a destructor.
pub fn server_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    match setpriv() {
        Some(tool) => {
            let mut c = Command::new(tool);
            c.args(["--pdeathsig", "KILL", "--"]).arg(program);
            c
        }
        None => Command::new(program),
    }
}
