//! Reading a password for `planeter operator account …` (RFC 014 D-3, D-4): **never from an argument, an environment variable or a file
//! name**. Two ways:
//!
//! * **the terminal**: the prompt is written to standard error, the echo of the terminal is turned off by running `stty -echo` (no new
//!   dependency and no `unsafe`: the lock holds only `libc`, and these crates forbid `unsafe`), the password is read twice, and **a guard
//!   turns the echo on again on every way out of the function** (a normal return, an error, a panic that unwinds). A `SIGKILL` and a
//!   Ctrl-C cannot be guarded: the help says "if your terminal stops echoing, run `stty sane`". If echo cannot be turned off, **no
//!   password is asked for**: the password would be shown;
//! * **a file descriptor** (`--password-fd N`, for scripts): one line is read from the open descriptor through `/dev/fd/N` (a pipe or a
//!   file the script opened on that number).
//!
//! Standard input that is not a terminal, with no `--password-fd`, is refused: a password is never read from a pipe by accident.

use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::process::{Command, Stdio};

/// The most bytes read for one password line (a password is at most 128 characters; the rule says so after reading).
const LINE_MAX_BYTES: u64 = 16 * 1024;

/// Where the password comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Typed at the terminal, twice, without echo.
    Terminal,
    /// One line from this open file descriptor.
    Fd(u32),
}

/// Why no password was read. Nothing here ever contains the password.
#[derive(Debug, PartialEq, Eq)]
pub enum InputError {
    /// Standard input is not a terminal and no descriptor was named.
    NoTerminal,
    /// The terminal's echo could not be turned off (so nothing was asked).
    Echo(String),
    /// The descriptor could not be opened or read as a line.
    Fd { fd: u32, why: String },
    /// The two entries differ.
    Differ,
}

/// Turn the echo off now; turn it on again when this is dropped. Dropping runs on a normal return, on an early return and on unwinding.
struct EchoOff;

impl EchoOff {
    fn new() -> Result<Self, InputError> {
        let status = Command::new("stty")
            .arg("-echo")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| InputError::Echo(e.to_string()))?;
        if !status.success() {
            return Err(InputError::Echo(format!("stty -echo exited with {status}")));
        }
        Ok(EchoOff)
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        // Nothing can be done about a failure here but try; the help says `stty sane`.
        let _ = Command::new("stty")
            .arg("echo")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn strip_newline(mut line: String) -> String {
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    line
}

/// One line from the terminal, after `prompt` on standard error.
fn ask(prompt: &str) -> String {
    let mut err = std::io::stderr();
    let _ = err.write_all(prompt.as_bytes());
    let _ = err.flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    // The terminal did not echo the return: end the line on screen.
    let _ = err.write_all(b"\n");
    strip_newline(line)
}

/// Read a new password from `source`. For the terminal, `prompts` are the two prompts (the first and the confirmation).
pub fn read_new(source: Source, prompts: (&str, &str)) -> Result<String, InputError> {
    match source {
        Source::Terminal => {
            if !std::io::stdin().is_terminal() {
                return Err(InputError::NoTerminal);
            }
            let guard = EchoOff::new()?;
            let first = ask(prompts.0);
            let second = ask(prompts.1);
            drop(guard);
            if first != second {
                return Err(InputError::Differ);
            }
            Ok(first)
        }
        Source::Fd(fd) => read_fd(fd),
    }
}

fn read_fd(fd: u32) -> Result<String, InputError> {
    let fail = |why: String| InputError::Fd { fd, why };
    let file =
        std::fs::File::open(format!("/dev/fd/{fd}")).map_err(|e| fail(e.kind().to_string()))?;
    let mut reader = BufReader::new(file.take(LINE_MAX_BYTES));
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| fail(e.kind().to_string()))?;
    if line.is_empty() {
        return Err(fail("end of input".to_owned()));
    }
    if !line.ends_with('\n') && line.len() as u64 >= LINE_MAX_BYTES {
        return Err(fail("a line too long to be a password".to_owned()));
    }
    Ok(strip_newline(line))
}
