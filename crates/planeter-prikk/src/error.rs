//! Failure modes of driving the prikk CLI. (RFC 001 D-2: "a non-zero prikk exit is a typed failure,
//! never a panic".)

use std::fmt;

/// An error from invoking or interpreting the prikk CLI.
#[derive(Debug)]
#[non_exhaustive]
pub enum PrikkError {
    /// The prikk process could not be spawned (binary missing, permission, etc.).
    Spawn(std::io::Error),
    /// prikk ran but exited non-zero. `code` is the process exit code where known
    /// (prikk uses `2` for usage errors and `1` for failures); `stderr` is its captured error output.
    Command { code: Option<i32>, stderr: String },
    /// The call hit its **memory ceiling** (RFC 011 D-2): prikk aborted on an allocation failure, or
    /// stopped early with a partial verification report. This says nothing about the repository: it is
    /// never a verification result and never "unverified". `verb` is the prikk subcommand; no path and no
    /// tool output is carried.
    CeilingExceeded { verb: String, ceiling_bytes: u64 },
    /// The call printed more than this server reads (RFC 011 follow-on, IS-55): the child was killed and **nothing it printed is
    /// used** — a truncated report is never parsed as a whole one, and the call is neither "empty" nor "none". `stream` says which
    /// of the two pipes went over `limit` bytes. For a call that writes (an accept, an import) the repository may have changed:
    /// the caller treats it as it treats any output it cannot read.
    OutputTooLarge {
        verb: String,
        limit: u64,
        stream: crate::sandbox::OutputStream,
    },
    /// The prikk binary's version is outside planeter's supported range (RFC 001 D-3 / PK-22:
    /// transport requires prikk ≥ 0.43.0).
    UnsupportedVersion { found: String, required: String },
    /// The `--format json` output could not be parsed into the expected shape.
    Parse(String),
}

impl fmt::Display for PrikkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrikkError::Spawn(e) => write!(f, "failed to spawn the prikk process: {e}"),
            PrikkError::Command { code, stderr } => {
                let code = code.map_or_else(|| "signal".to_owned(), |c| c.to_string());
                write!(f, "prikk exited with status {code}: {}", stderr.trim())
            }
            PrikkError::CeilingExceeded {
                verb,
                ceiling_bytes,
            } => write!(
                f,
                "the prikk `{verb}` call exceeded its memory ceiling of {ceiling_bytes} bytes"
            ),
            PrikkError::OutputTooLarge {
                verb,
                limit,
                stream,
            } => write!(
                f,
                "the prikk `{verb}` call printed more than {limit} bytes on {stream}, which is more than this server reads"
            ),
            PrikkError::UnsupportedVersion { found, required } => {
                write!(
                    f,
                    "unsupported prikk version {found}; planeter requires {required}"
                )
            }
            PrikkError::Parse(msg) => write!(f, "could not parse prikk JSON output: {msg}"),
        }
    }
}

impl std::error::Error for PrikkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PrikkError::Spawn(e) => Some(e),
            _ => None,
        }
    }
}

/// Convenience alias for the crate's fallible operations.
pub type Result<T> = std::result::Result<T, PrikkError>;
