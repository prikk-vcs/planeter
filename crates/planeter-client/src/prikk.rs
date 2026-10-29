//! The user's own prikk, run as a program (RFC 004 part 9). The helper holds no key and links no prikk code: it asks the
//! installed `prikk` to build, check and import, in the repository it is told to, with the user's own key directory (prikk finds it
//! by itself). `PLANETER_CLIENT_PRIKK` names another program (the tests use it for a stand-in).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::catalog::{Key, Lang, t};
use crate::exit::{Failure, PRIKK};

/// The oldest prikk this program works with (the server's own floor; `tests/floor.rs` keeps the two equal).
pub const PRIKK_FLOOR: (u64, u64, u64) = (0, 54, 0);

pub fn floor_text() -> String {
    format!("{}.{}.{}", PRIKK_FLOOR.0, PRIKK_FLOOR.1, PRIKK_FLOOR.2)
}

/// A prikk, aimed at one directory.
#[derive(Debug, Clone)]
pub struct Prikk {
    bin: OsString,
    dir: PathBuf,
    lang: Lang,
}

/// `prikk 0.55.0` → (0, 55, 0).
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let v = text
        .split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

impl Prikk {
    pub fn new(dir: &Path, lang: Lang, program: Option<OsString>) -> Prikk {
        Prikk {
            bin: program.unwrap_or_else(|| "prikk".into()),
            dir: dir.to_path_buf(),
            lang,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    /// Is prikk there, and new enough?
    pub fn check(&self) -> Result<(), Failure> {
        let out = Command::new(&self.bin)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|_| {
                Failure::new(
                    PRIKK,
                    t(self.lang, Key::PrikkMissing)
                        .with("floor", floor_text())
                        .text(),
                )
            })?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        match parse_version(&text) {
            Some(v) if v >= PRIKK_FLOOR => Ok(()),
            Some(v) => Err(Failure::new(
                PRIKK,
                t(self.lang, Key::PrikkTooOld)
                    .with("found", format!("{}.{}.{}", v.0, v.1, v.2))
                    .with("floor", floor_text())
                    .text(),
            )),
            None => Err(Failure::new(
                PRIKK,
                t(self.lang, Key::PrikkMissing)
                    .with("floor", floor_text())
                    .text(),
            )
            .with_detail(text)),
        }
    }

    /// Run prikk and keep its output; a non-zero exit is a [`Failure`] naming the `step` in plain words, with prikk's own text as
    /// the detail (shown under `--verbose`).
    pub fn run(&self, step: &str, args: &[&str]) -> Result<Output, Failure> {
        let out = self.command(args).output().map_err(|e| {
            Failure::new(
                PRIKK,
                t(self.lang, Key::PrikkMissing)
                    .with("floor", floor_text())
                    .text(),
            )
            .with_detail(e.to_string())
        })?;
        if out.status.success() {
            return Ok(out);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        let first = stderr.lines().next().unwrap_or("").to_owned();
        Err(Failure::new(
            PRIKK,
            t(self.lang, Key::PrikkFailed)
                .with("step", step)
                .with("reason", first)
                .text(),
        )
        .with_detail(stderr))
    }

    /// Run prikk with `--format json` and parse its output.
    pub fn json(&self, step: &str, args: &[&str]) -> Result<serde_json::Value, Failure> {
        let out = self.run(step, args)?;
        serde_json::from_slice(&out.stdout).map_err(|e| {
            Failure::new(
                PRIKK,
                t(self.lang, Key::PrikkFailed)
                    .with("step", step)
                    .with("reason", "its report was not JSON")
                    .text(),
            )
            .with_detail(e.to_string())
        })
    }

    /// Is `dir` a prikk repository?
    pub fn is_repository(dir: &Path) -> bool {
        dir.join(".prikk").is_dir()
    }
}
