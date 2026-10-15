#![forbid(unsafe_code)]
//! `planeter-pin-check` — the build-chain pin gate (RFC 010 D-5, threat model C-5d / C-11 / INV-7).
//!
//! It fails on any reference that a third party can move under us:
//!
//! - a workflow `uses:` that is not `owner/repo[/path]@<40-hex commit SHA>` (a local `./` action is
//!   allowed; `docker://` must carry `@sha256:`);
//! - a workflow `image:` without `@sha256:`;
//! - a Dockerfile `FROM` without `@sha256:<64 hex>` (`scratch` and an earlier stage's alias are fine);
//! - a toolchain named by channel (`stable`, `beta`, `nightly`) rather than by `X.Y.Z`: the `toolchain:`
//!   input, a `rustup` command line, or the `channel` of a `rust-toolchain` file;
//! - `toolchain:` inputs that are not one and the same version across the workflows (CI and release
//!   must build with one compiler, RFC 010 D-5);
//! - a runner image named by a moving label: any `runs-on` or matrix `os` value ending in `-latest`.
//!
//! It has no allow-list. It reads text only, so it understands the shapes this repository uses, and
//! says so: `cargo install` of tools is deliberately current and is out of scope.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One mutable reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// Repository-relative path, `/`-separated.
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// Everything found under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub files_scanned: usize,
    pub findings: Vec<Finding>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for x in &self.findings {
            writeln!(f, "{}:{}: {}", x.file, x.line, x.message)?;
        }
        if self.findings.is_empty() {
            writeln!(
                f,
                "pin-check: OK — no mutable reference in {} file(s)",
                self.files_scanned
            )
        } else {
            writeln!(
                f,
                "pin-check: FAILED — {} mutable reference(s) in {} file(s) scanned",
                self.findings.len(),
                self.files_scanned
            )
        }
    }
}

/// Scan `root` for workflows, Dockerfiles and toolchain files.
pub fn check(root: &Path) -> io::Result<Report> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", root.display()),
        ));
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();

    let mut report = Report {
        files_scanned: 0,
        findings: Vec::new(),
    };
    let mut workflows: Vec<PathBuf> = Vec::new();
    for rel in files {
        let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let rel_str = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let is_workflow = rel_str.starts_with(".github/workflows/")
            && (name.ends_with(".yml") || name.ends_with(".yaml"));
        let is_docker = name == "Dockerfile"
            || name.starts_with("Dockerfile.")
            || name.ends_with(".Dockerfile");
        let is_toolchain = name == "rust-toolchain" || name == "rust-toolchain.toml";
        if !(is_workflow || is_docker || is_toolchain) {
            continue;
        }
        let text = fs::read_to_string(root.join(&rel))?;
        report.files_scanned += 1;
        if is_workflow {
            workflows.push(rel.clone());
        }
        let found = if is_workflow {
            scan_workflow(&text)
        } else if is_docker {
            scan_dockerfile(&text)
        } else {
            scan_toolchain_file(&text)
        };
        report
            .findings
            .extend(found.into_iter().map(|(line, message)| Finding {
                file: rel_str.clone(),
                line,
                message,
            }));
    }
    report
        .findings
        .extend(toolchain_disagreements(root, &workflows)?);
    report.findings.sort();
    Ok(report)
}

/// One finding per `toolchain:` input when the workflows do not agree on a single version. Inputs that
/// are not `X.Y.Z` are already findings of their own and are left out of the comparison.
fn toolchain_disagreements(root: &Path, workflows: &[PathBuf]) -> io::Result<Vec<Finding>> {
    let mut inputs: Vec<(String, usize, String)> = Vec::new();
    for rel in workflows {
        let text = fs::read_to_string(root.join(rel))?;
        let file = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        for (i, line) in text.lines().enumerate() {
            if let Some(v) = value_of(line, "toolchain") {
                if is_semver(v) {
                    inputs.push((file.clone(), i + 1, v.to_owned()));
                }
            }
        }
    }
    let mut versions: Vec<&str> = inputs.iter().map(|(_, _, v)| v.as_str()).collect();
    versions.sort_unstable();
    versions.dedup();
    if versions.len() < 2 {
        return Ok(Vec::new());
    }
    let summary = inputs
        .iter()
        .map(|(f, l, v)| format!("{v} in {f}:{l}"))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(inputs
        .iter()
        .map(|(file, line, v)| Finding {
            file: file.clone(),
            line: *line,
            message: format!(
                "toolchain {v:?} disagrees with the other workflows; CI and release must use one version ({summary})"
            ),
        })
        .collect())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if matches!(&*name, ".git" | "target" | ".git-exclude" | "node_modules") {
                continue;
            }
            walk(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

/// `(line number, message)` pairs.
type Hits = Vec<(usize, String)>;

fn strip_comment(line: &str) -> &str {
    // A `#` starts a comment at line start or after whitespace (YAML); `#` inside a value is rare
    // here and would only make the check more lenient on a value we then also validate as a token.
    let mut prev_ws = true;
    for (i, c) in line.char_indices() {
        if c == '#' && prev_ws {
            return &line[..i];
        }
        prev_ws = c.is_whitespace();
    }
    line
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_semver(v: &str) -> bool {
    let p: Vec<&str> = v.split('.').collect();
    p.len() == 3
        && p.iter()
            .all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
}

fn is_channel(w: &str) -> bool {
    matches!(w, "stable" | "beta" | "nightly")
}

fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let t = strip_comment(line).trim();
    let t = t.strip_prefix("- ").map_or(t, str::trim_start);
    let rest = t.strip_prefix(key)?.strip_prefix(':')?;
    Some(rest.trim().trim_matches(['"', '\'']))
}

fn scan_workflow(text: &str) -> Hits {
    let mut hits = Hits::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if let Some(v) = value_of(line, "uses") {
            if v.starts_with("./") {
                continue;
            }
            if let Some(image) = v.strip_prefix("docker://") {
                if !image.contains("@sha256:") {
                    hits.push((n, format!("docker action {v:?} has no @sha256: digest")));
                }
                continue;
            }
            match v.rsplit_once('@') {
                Some((_, sha)) if is_hex(sha, 40) => {}
                Some((_, r#ref)) => hits.push((
                    n,
                    format!(
                        "action {v:?} is pinned to {:?}, not a 40-hex commit SHA",
                        r#ref
                    ),
                )),
                None => hits.push((n, format!("action {v:?} has no @<commit SHA>"))),
            }
        } else if let Some(v) = value_of(line, "toolchain") {
            if !is_semver(v) {
                hits.push((
                    n,
                    format!("toolchain {v:?} is not an explicit X.Y.Z version"),
                ));
            }
        } else if let Some(v) = value_of(line, "runs-on").or_else(|| value_of(line, "os")) {
            let moving = v
                .split(|c: char| c.is_whitespace() || matches!(c, ',' | '[' | ']' | '"' | '\''))
                .find(|t| t.ends_with("-latest"));
            if let Some(label) = moving {
                hits.push((
                    n,
                    format!("runner image {label:?} is a moving label; name the image (e.g. ubuntu-24.04)"),
                ));
            }
        } else if let Some(v) = value_of(line, "image") {
            if !v.is_empty() && !v.contains("${{") && !v.contains("@sha256:") {
                hits.push((n, format!("image {v:?} has no @sha256: digest")));
            }
        }
        // `rustup default stable`, `rustup toolchain install nightly`, `rustup update beta`, …
        let code = strip_comment(line);
        if code.contains("rustup") && code.split_whitespace().any(is_channel) {
            hits.push((
                n,
                "rustup selects a toolchain by channel, not by X.Y.Z".to_owned(),
            ));
        }
    }
    hits
}

fn scan_dockerfile(text: &str) -> Hits {
    let mut hits = Hits::new();
    let mut stages: Vec<String> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let words: Vec<&str> = strip_comment(line).split_whitespace().collect();
        if !words
            .first()
            .is_some_and(|w| w.eq_ignore_ascii_case("FROM"))
        {
            continue;
        }
        let mut rest = words[1..].iter().filter(|w| !w.starts_with("--"));
        let Some(image) = rest.next() else {
            continue;
        };
        let alias = match (rest.next(), rest.next()) {
            (Some(kw), Some(alias)) if kw.eq_ignore_ascii_case("AS") => {
                Some(alias.to_ascii_lowercase())
            }
            _ => None,
        };
        // An earlier stage's alias is a local reference, not a registry image.
        let known_stage = stages.iter().any(|s| *s == image.to_ascii_lowercase());
        stages.extend(alias);
        if *image == "scratch" || known_stage {
            continue;
        }
        match image.split_once("@sha256:") {
            Some((_, digest)) if is_hex(digest, 64) => {}
            Some(_) => hits.push((n, format!("FROM {image:?} has a malformed sha256 digest"))),
            None if image.contains('$') => hits.push((
                n,
                format!("FROM {image:?} is built from a variable; its digest cannot be verified"),
            )),
            None => hits.push((n, format!("FROM {image:?} has no @sha256: digest"))),
        }
    }
    hits
}

fn scan_toolchain_file(text: &str) -> Hits {
    let mut hits = Hits::new();
    let trimmed = text.trim();
    // Legacy `rust-toolchain`: the whole file is the channel.
    if !trimmed.contains('=') && !trimmed.is_empty() && !trimmed.contains('\n') {
        if !is_semver(trimmed) {
            hits.push((
                1,
                format!("toolchain {trimmed:?} is not an explicit X.Y.Z version"),
            ));
        }
        return hits;
    }
    for (i, line) in text.lines().enumerate() {
        let t = strip_comment(line).trim();
        if let Some(rest) = t.strip_prefix("channel") {
            let v = rest
                .trim_start()
                .strip_prefix('=')
                .map_or("", |v| v.trim().trim_matches(['"', '\'']));
            if !is_semver(v) {
                hits.push((
                    i + 1,
                    format!("channel {v:?} is not an explicit X.Y.Z version"),
                ));
            }
        }
    }
    hits
}
