#![forbid(unsafe_code)]
//! The image gate's static check (D-51, `image-glibc-handoff-v1`): a release binary is built on a runner with one glibc and copied into
//! an image with another. The dynamic loader refuses to start a binary whose **version needs** (`GLIBC_x.y` in the ELF's
//! `.gnu.version_r`) the image's `libc.so.6` does not define — unless the need is flagged `WEAK`, which the loader tolerates. This
//! reads the needs from `readelf -V`, separates **strong** from **weak**, and compares the highest strong one with the image's glibc
//! (read from the image with `ldd --version`, never a constant).
//!
//! A weak need above the image's glibc does not fail the check and is never silent: it is reported.

use std::cmp::Ordering;
use std::fmt;

/// A glibc version, `2.38` or `2.2.5`.
#[derive(Debug, Clone)]
pub struct Version(Vec<u32>);

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Version {}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let parts: Option<Vec<u32>> = text.split('.').map(|p| p.parse().ok()).collect();
        parts.filter(|p| p.len() >= 2).map(Version)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // Missing trailing parts count as zero: 2.3 == 2.3.0.
        let n = self.0.len().max(other.0.len());
        for i in 0..n {
            let (a, b) = (
                self.0.get(i).copied().unwrap_or(0),
                other.0.get(i).copied().unwrap_or(0),
            );
            match a.cmp(&b) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(u32::to_string).collect();
        write!(f, "{}", parts.join("."))
    }
}

/// One `GLIBC_x.y` version need of one library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    /// The library the version is needed from (`libc.so.6`).
    pub file: String,
    pub version: Version,
    /// Flagged `WEAK`: the loader starts the program without it.
    pub weak: bool,
}

/// The `GLIBC_` version needs in the text of `readelf -V` (or a filtered copy of it): a `File:` line names the library, the
/// `Name: … Flags: …` lines after it are its needs. `GLIBC_PRIVATE` and the needs of other libraries (`GCC_`, `CXXABI_`) are not glibc
/// versions and are ignored.
pub fn parse_needs(readelf_v: &str) -> Vec<Need> {
    let mut file = String::new();
    let mut out = Vec::new();
    for line in readelf_v.lines() {
        if let Some(at) = line.find("File:") {
            file = line[at + 5..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
        } else if let Some(at) = line.find("Name:") {
            let mut words = line[at + 5..].split_whitespace();
            let Some(name) = words.next() else { continue };
            let Some(version) = name.strip_prefix("GLIBC_").and_then(Version::parse) else {
                continue;
            };
            let weak = line
                .find("Flags:")
                .is_some_and(|f| line[f + 6..].split_whitespace().next() == Some("WEAK"));
            out.push(Need {
                file: file.clone(),
                version,
                weak,
            });
        }
    }
    out
}

/// The base image's glibc from the first line of `ldd --version` (`ldd (Debian GLIBC 2.36-9+deb12u10) 2.36`), or a bare version.
pub fn base_version(text: &str) -> Option<Version> {
    let line = text.lines().next()?.trim();
    let last = line.split_whitespace().last()?;
    Version::parse(last)
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub base: Version,
    pub highest_strong: Option<Version>,
    pub highest_weak: Option<Version>,
    /// Strong needs above the base: the loader would refuse to start the binary.
    pub too_new: Vec<Need>,
    /// Weak needs above the base: tolerated by the loader, reported.
    pub weak_above: Vec<Need>,
}

impl Verdict {
    pub fn ok(&self) -> bool {
        self.too_new.is_empty()
    }
}

pub fn check(needs: &[Need], base: &Version) -> Verdict {
    let highest = |weak: bool| {
        needs
            .iter()
            .filter(|n| n.weak == weak)
            .map(|n| n.version.clone())
            .max()
    };
    let above = |weak: bool| {
        let mut v: Vec<Need> = needs
            .iter()
            .filter(|n| n.weak == weak && n.version > *base)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.version.cmp(&b.version));
        v
    };
    Verdict {
        base: base.clone(),
        highest_strong: highest(false),
        highest_weak: highest(true),
        too_new: above(false),
        weak_above: above(true),
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |v: &Option<Version>| v.as_ref().map_or("none".to_owned(), ToString::to_string);
        writeln!(
            f,
            "glibc-check: the image's glibc is {}; the binary's highest strong need is {}, its highest weak need {}",
            self.base,
            show(&self.highest_strong),
            show(&self.highest_weak)
        )?;
        for n in &self.weak_above {
            writeln!(
                f,
                "glibc-check: NOTE: a WEAK need GLIBC_{} of {} is above the image's glibc; the loader tolerates it, the code that uses it must test for it",
                n.version, n.file
            )?;
        }
        if self.ok() {
            writeln!(
                f,
                "glibc-check: OK — the image's glibc defines every version the binary strictly needs"
            )
        } else {
            for n in &self.too_new {
                writeln!(
                    f,
                    "glibc-check: FAIL: the binary strictly needs GLIBC_{} of {}, which the image's glibc {} does not define",
                    n.version, n.file, self.base
                )?;
            }
            writeln!(
                f,
                "glibc-check: FAILED — the image would not start; build against an older glibc or use a newer base"
            )
        }
    }
}
