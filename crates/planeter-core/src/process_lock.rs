//! The single-process rule (RFC 004 D-7): one server process per store.
//!
//! The repository lease is in-process, so two server processes against one store would not see each other's
//! leases and could write one repository at once. The rule is enforced at start: the server takes an **advisory
//! lock file** next to the database (`<database>.lock`), holding its process id, and removes it on exit. A second
//! process finds the file, finds that the process named in it is running, and refuses to start with a message that
//! says what to do.
//!
//! **Limits, plainly.** It is advisory (a person can delete the file); a crash leaves the file, which the next
//! start recognises as stale (its process is gone, checked in `/proc`) and replaces; a process id can be reused by
//! an unrelated process after a crash, in which case the start is refused and the message names the file to
//! remove; it is Linux-shaped (it checks `/proc`), like the rest of the sandbox. It does not protect against a
//! second process started *in the same instant* (the file is created exclusively, so one of the two loses).

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Why the lock was not taken.
#[derive(Debug)]
pub enum ProcessLockError {
    /// Another server process is running against this store.
    AlreadyRunning { pid: u32, path: PathBuf },
    /// The lock file is there, is not ours, and its process cannot be shown to be gone.
    Unclear { path: PathBuf, content: String },
    /// The file could not be created or read.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ProcessLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessLockError::AlreadyRunning { pid, path } => write!(
                f,
                "another planeter server (process {pid}) is already running against this database; only one server \
                 process per database is supported. Stop it first. (If it is not running, remove {} and start again.)",
                path.display()
            ),
            ProcessLockError::Unclear { path, content } => write!(
                f,
                "{} exists and does not name a process that can be checked ({content:?}); if no planeter server is \
                 running against this database, remove the file and start again",
                path.display()
            ),
            ProcessLockError::Io { path, source } => {
                write!(f, "cannot take the lock file {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ProcessLockError {}

/// The lock. Dropping it removes the file (if it is still ours).
#[derive(Debug)]
pub struct ProcessLock {
    path: PathBuf,
    pid: u32,
}

impl ProcessLock {
    /// Take the lock at `path`, or say why not.
    pub fn acquire(path: impl AsRef<Path>) -> Result<ProcessLock, ProcessLockError> {
        Self::acquire_as(path, std::process::id())
    }

    /// As [`ProcessLock::acquire`], for a process with id `pid` (tests).
    pub fn acquire_as(path: impl AsRef<Path>, pid: u32) -> Result<ProcessLock, ProcessLockError> {
        let path = path.as_ref().to_path_buf();
        let io = |source| ProcessLockError::Io {
            path: path.clone(),
            source,
        };
        // Two attempts: the second after removing a stale file.
        for attempt in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    writeln!(f, "{pid}").map_err(io)?;
                    f.sync_all().map_err(io)?;
                    return Ok(ProcessLock { path, pid });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                    let content = fs::read_to_string(&path).unwrap_or_default();
                    match content.trim().parse::<u32>() {
                        Ok(other) if other == pid => {
                            // Our own file from an earlier run of the same pid (a restart that reused it): ours.
                            fs::remove_file(&path).map_err(io)?;
                        }
                        Ok(other) if process_is_running(other) => {
                            return Err(ProcessLockError::AlreadyRunning { pid: other, path });
                        }
                        Ok(_) => {
                            // The process named in it is gone: a crash left it. Replace it.
                            fs::remove_file(&path).map_err(io)?;
                        }
                        Err(_) => return Err(ProcessLockError::Unclear { path, content }),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lost a race with another starter that created it between our check and our retry.
                    let content = fs::read_to_string(&path).unwrap_or_default();
                    let other = content.trim().parse::<u32>().unwrap_or(0);
                    return Err(ProcessLockError::AlreadyRunning { pid: other, path });
                }
                Err(e) => return Err(io(e)),
            }
        }
        unreachable!("the loop returns on both attempts")
    }
}

/// Whether process `pid` exists (Linux: `/proc/<pid>`).
fn process_is_running(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        // Remove the file only if it still names us: never delete another process's lock.
        if fs::read_to_string(&self.path)
            .map(|c| c.trim() == self.pid.to_string())
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("planeter-plock-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_lock_is_taken_and_removed_on_drop() {
        let d = dir("basic");
        let p = d.join("planeter.db.lock");
        let lock = ProcessLock::acquire(&p).unwrap();
        assert_eq!(
            fs::read_to_string(&p).unwrap().trim(),
            std::process::id().to_string()
        );
        drop(lock);
        assert!(!p.exists());
        // And can be taken again.
        drop(ProcessLock::acquire(&p).unwrap());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_second_holder_is_refused_with_the_pid_and_what_to_do() {
        let d = dir("second");
        let p = d.join("x.lock");
        // Process 1 always exists on Linux; the "other server" is pid 1 here.
        fs::write(&p, "1\n").unwrap();
        let e = ProcessLock::acquire_as(&p, 4_000_000).unwrap_err();
        assert!(
            matches!(e, ProcessLockError::AlreadyRunning { pid: 1, .. }),
            "{e:?}"
        );
        let m = e.to_string();
        assert!(
            m.contains("process 1") && m.contains("only one server") && m.contains("x.lock"),
            "{m}"
        );
        // The refused starter leaves the other's file alone.
        assert_eq!(fs::read_to_string(&p).unwrap().trim(), "1");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stale_file_from_a_dead_process_is_replaced() {
        let d = dir("stale");
        let p = d.join("x.lock");
        fs::write(&p, "4194999\n").unwrap(); // beyond pid_max on any default Linux
        let lock = ProcessLock::acquire_as(&p, 4_000_001).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap().trim(), "4000001");
        drop(lock);
        assert!(!p.exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_names_no_process_is_not_guessed_at() {
        let d = dir("unclear");
        let p = d.join("x.lock");
        fs::write(&p, "not a pid").unwrap();
        let e = ProcessLock::acquire(&p).unwrap_err();
        assert!(matches!(e, ProcessLockError::Unclear { .. }), "{e:?}");
        assert!(e.to_string().contains("remove the file"));
        assert!(p.exists(), "an unclear file is left in place");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dropping_never_removes_another_processs_lock() {
        let d = dir("foreign");
        let p = d.join("x.lock");
        let lock = ProcessLock::acquire_as(&p, 4_000_002).unwrap();
        fs::write(&p, "1\n").unwrap(); // someone replaced it
        drop(lock);
        assert_eq!(fs::read_to_string(&p).unwrap().trim(), "1");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unwritable_place_is_a_clear_error() {
        let e = ProcessLock::acquire("/nonexistent-dir-for-planeter/x.lock").unwrap_err();
        assert!(matches!(e, ProcessLockError::Io { .. }));
        assert!(e.to_string().contains("cannot take the lock file"));
    }
}
