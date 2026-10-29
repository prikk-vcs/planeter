//! `planeter-client`: a person's own prikk, talking to a planeter server over https (RFC 004 part 9, D-10).
//!
//! It **clones** a repository (and shows the maintainer keys it carries, so that only keys the person types are trusted),
//! **fetches** newer state (landing it as received copies, never touching the person's branches) and **proposes** work (builds an
//! artifact against what the server already holds and sends it as a proposal). It holds no key, links no prikk code and signs
//! nothing: every history operation is the user's own prikk. It never follows a redirect, never speaks plain http and has no
//! `--insecure`.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod cli;
pub mod commands;
pub mod exit;
pub mod http;
pub mod prikk;
pub mod state;
pub mod token;
pub mod url;

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use catalog::{Key, Lang, t};
use exit::Failure;

/// The commit this binary was built from (`PLANETER_GIT_COMMIT` at build time), as the server's `--version` does.
pub const BUILD_COMMIT: &str = match option_env!("PLANETER_GIT_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

/// Everything a command needs from the outside, so that tests can drive the whole program without a terminal.
pub struct Ctx<'a> {
    pub lang: Lang,
    pub verbose: bool,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub stdin: &'a mut dyn BufRead,
    /// Whether stdin is a terminal (a person can type a key); false for a pipe or a file.
    pub interactive: bool,
    pub cwd: PathBuf,
    pub prikk_program: Option<OsString>,
    pub timeouts: http::Timeouts,
}

impl Ctx<'_> {
    /// One whole line on stdout.
    pub fn say(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.out, "{}", line.as_ref());
    }
}

/// Run the program: the arguments after its name. Returns the exit code.
pub fn run(args: &[String], ctx: &mut Ctx<'_>) -> i32 {
    let hint = ctx.lang;
    let parsed = match cli::parse(args, hint) {
        Ok(p) => p,
        Err(f) => return report(ctx, &f),
    };
    if let Some(l) = parsed.lang {
        ctx.lang = l;
    }
    use cli::Command;
    let result = match parsed.command {
        Command::Help(topic) => {
            let key = match topic {
                cli::Topic::Main => Key::HelpMain,
                cli::Topic::Propose => Key::HelpPropose,
                cli::Topic::Clone => Key::HelpClone,
                cli::Topic::Fetch => Key::HelpFetch,
            };
            let text = t(ctx.lang, key).text();
            ctx.say(text);
            Ok(())
        }
        Command::Version => {
            let line = t(ctx.lang, Key::VersionLine)
                .with("version", env!("CARGO_PKG_VERSION"))
                .with("commit", BUILD_COMMIT)
                .with("floor", prikk::floor_text())
                .text();
            ctx.say(line);
            Ok(())
        }
        Command::Propose(a) => {
            ctx.verbose = a.common.verbose;
            commands::propose::run(ctx, &a)
        }
        Command::Clone(a) => {
            ctx.verbose = a.common.verbose;
            commands::clone::run(ctx, &a)
        }
        Command::Fetch(a) => {
            ctx.verbose = a.common.verbose;
            commands::fetch::run(ctx, &a)
        }
    };
    match result {
        Ok(()) => exit::OK,
        Err(f) => report(ctx, &f),
    }
}

/// Print a failure: its sentence(s) on stderr, and the raw detail only under `--verbose`.
fn report(ctx: &mut Ctx<'_>, f: &Failure) -> i32 {
    let _ = writeln!(ctx.err, "{}", f.message);
    if ctx.verbose
        && let Some(d) = &f.detail
    {
        let _ = writeln!(ctx.err, "(detail) {d}");
    }
    f.code
}
