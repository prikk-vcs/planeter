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
