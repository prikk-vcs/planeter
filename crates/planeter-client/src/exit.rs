//! The exit codes (RFC 004 part 9, T1) and the one shape of a failure a command returns.

/// Everything went as asked.
pub const OK: i32 = 0;
/// The server refused (a named answer: the message and the hint are shown).
pub const REFUSED: i32 = 1;
/// The command line is wrong.
pub const USAGE: i32 = 2;
/// The network, or TLS, failed before an answer arrived.
pub const NETWORK: i32 = 3;
/// The user's own prikk failed or is missing.
pub const PRIKK: i32 = 4;
/// Local state: files, directories, keys, the token file.
pub const LOCAL: i32 = 5;

/// What a command returns when it cannot do what was asked: an exit code, the plain sentence(s) for the person, and the raw
/// reason that is shown only under `--verbose`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: i32,
    /// Already worded, in the person's language; one line per sentence.
    pub message: String,
    /// The raw cause (a rustls reason, prikk's own text, the server's verbatim text): only under `--verbose`.
    pub detail: Option<String>,
}

impl Failure {
    pub fn new(code: i32, message: String) -> Failure {
        Failure {
            code,
            message,
            detail: None,
        }
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Failure {
        self.detail = Some(detail.into());
        self
    }
}
