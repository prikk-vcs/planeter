#![forbid(unsafe_code)]
//! `planeter-floor-check` — the prikk-floor gate (RFC 010 D-2).
//!
//! planeter names its minimum prikk version in four files that must move together:
//! `MIN_PRIKK_VERSION` in `crates/planeter-prikk/src/cli.rs`, `PRIKK_VERSION` in
//! `.github/workflows/release.yml` and in `.github/workflows/ci.yml` (the prikk the client's tests install), and
//! `ARG PRIKK_VERSION` in the `Dockerfile`. This tool reads all four and reports whether they are one and the same `X.Y.Z`. It parses only the declaration lines
//! (a mention in a comment or a doc line is not a floor).

use std::fmt;
use std::fs;
use std::path::Path;

/// The four files, relative to the repository root, and how each declares the floor.
const SOURCES: [(&str, Kind); 4] = [
    ("crates/planeter-prikk/src/cli.rs", Kind::RustConst),
    (".github/workflows/release.yml", Kind::WorkflowEnv),
    (".github/workflows/ci.yml", Kind::WorkflowEnv),
    ("Dockerfile", Kind::DockerArg),
];

#[derive(Clone, Copy)]
enum Kind {
    RustConst,
    WorkflowEnv,
    DockerArg,
}

/// What one file declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Floor {
    /// Repository-relative path.
    pub file: &'static str,
    /// `Ok("X.Y.Z")`, or why no valid floor was found in the file.
    pub value: Result<String, String>,
}

/// The outcome for a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub floors: Vec<Floor>,
}

impl Report {
    /// `true` when every file declares a valid `X.Y.Z` and they are all equal.
    pub fn agrees(&self) -> bool {
        let mut values = self.floors.iter().map(|f| f.value.as_ref());
        match values.next() {
            Some(Ok(first)) => values.all(|v| v.is_ok_and(|v| v == first)),
            _ => false,
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for floor in &self.floors {
            match &floor.value {
                Ok(v) => writeln!(f, "  {:<40} {v}", floor.file)?,
                Err(why) => writeln!(f, "  {:<40} ERROR: {why}", floor.file)?,
            }
        }
        if self.agrees() {
            writeln!(f, "floor-check: OK — the four prikk floors agree")
        } else {
            writeln!(
                f,
                "floor-check: FAILED — the four prikk floors must be one X.Y.Z"
            )
        }
    }
}

/// Read the three declarations under `root`. I/O failures (a missing file) are reported per file, not
/// as an error, so one report always names all three.
pub fn check(root: &Path) -> std::io::Result<Report> {
    if !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} is not a directory", root.display()),
        ));
    }
    let floors = SOURCES
        .iter()
        .map(|&(file, kind)| Floor {
            file,
            value: fs::read_to_string(root.join(file))
                .map_err(|e| format!("cannot read: {e}"))
                .and_then(|text| find(&text, kind)),
        })
        .collect();
    Ok(Report { floors })
}

fn find(text: &str, kind: Kind) -> Result<String, String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let raw = match kind {
            Kind::RustConst => t
                .strip_prefix("pub const MIN_PRIKK_VERSION")
                .and_then(|r| r.split_once('='))
                .map(|(_, v)| rust_tuple(v)),
            Kind::WorkflowEnv => t
                .strip_prefix("PRIKK_VERSION:")
                .map(|v| unquote(v).to_owned()),
            Kind::DockerArg => t
                .strip_prefix("ARG PRIKK_VERSION=")
                .map(|v| unquote(v).to_owned()),
        };
        if let Some(v) = raw {
            found.push(v);
        }
    }
    match found.as_slice() {
        [] => Err("no floor declaration found".to_owned()),
        [one] if is_semver(one) => Ok(one.clone()),
        [one] => Err(format!("{one:?} is not X.Y.Z")),
        many => Err(format!(
            "{} floor declarations found, expected one",
            many.len()
        )),
    }
}

/// `(0, 52, 0);` → `0.52.0`; anything else is returned unchanged so it fails the semver check.
fn rust_tuple(v: &str) -> String {
    let v = v.trim().trim_end_matches(';').trim();
    match v.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        Some(inner) => inner
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("."),
        None => v.to_owned(),
    }
}

fn unquote(v: &str) -> &str {
    v.trim().trim_matches(['"', '\''])
}

fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
