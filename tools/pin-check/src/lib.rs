#![forbid(unsafe_code)]
//! `planeter-pin-check` — the build-chain pin gate (RFC 010 D-5, threat model C-5d / C-11 / INV-7).
//!
//! It fails on any reference that a third party can move under us:
//!
//! - a workflow `uses:` that is not `owner/repo[/path]@<40-hex commit SHA>` (a local `./` action is
//!   allowed; `docker://` must carry `@sha256:`);
//! - a workflow `image:` without `@sha256:`;
//! - a Dockerfile `FROM` without `@sha256:<64 hex>` (`scratch` and an earlier stage's alias are fine);
//! - a toolchain named by channel (`stable`, `beta`, `nightly`, `latest`) rather than by `X.Y.Z`: the
//!   `toolchain:` input, the version argument of a `rustup toolchain install` / `rustup default` /
//!   `rustup install` / `rustup update` / `rustup override set` command line, or the `channel` of a
//!   `rust-toolchain` file;
//! - toolchain versions (the `toolchain:` input or the `rustup` command's argument) that are not one and the
//!   same version across the workflows (CI and release must build with one compiler, RFC 010 D-5);
//! - the one named exception to "one compiler": a job whose id is `msrv` builds with the workspace's
//!   declared minimum, so its `toolchain:` must equal `rust-version` in the root `Cargo.toml` (as
//!   `X.Y.Z`; `1.88` means `1.88.0`), and that job must exist whenever `rust-version` is declared (IS-32);
//! - a runner image named by a moving label: any `runs-on` or matrix `os` value ending in `-latest`;
//! - a workflow `uses:` whose owner is not `actions` (RFC 010 D-11: the release chain trusts GitHub-owned
//!   actions only; everything else is a shell step in the repository);
//! - an image the workflow runs or builds with that is not referenced by `sha256:` digest: a `docker run` or
//!   `docker pull` line without one, and a `docker buildx create` step without `image=<ref>@sha256:<64 hex>`
//!   in its `--driver-opt` (the BuildKit image of a `docker-container` builder);
//! - a registry credential on a command line: a `docker login` step without `--password-stdin` or with
//!   `--password`/`-p`, a `docker login` that runs outside a tag push or a rehearsal, and a `secrets.*` /
//!   `github.token` expression anywhere in a `run:` script (it belongs in `env:` or `with:`);
//! - a release path that can reach the real image from a rehearsal (RFC 010 D-10, "the rehearsal cannot
//!   reach the real package, structurally"): a workflow step that names the real image
//!   `ghcr.io/prikk-vcs/planeter` (not `-rehearsal`) without `github.event_name == 'push'` in its (or its
//!   job's) `if`; a step that names both the real image and the rehearsal package; a step that names the
//!   rehearsal package without `inputs.mode == 'rehearsal'`, or a rehearsal-conditioned step that can also
//!   run on a tag push (`github.event_name == 'push'` or `||` in its condition); a rehearsal-conditioned
//!   step that pushes to anything but the rehearsal package; and a rehearsal-conditioned step that names a
//!   release, tag or crates.io command;
//! - an image that is pushed or attested before it has been run (D-51): a step that pushes an image or attests
//!   one (`ghcr.io` in a push, an `actions/attest` of an image) must be preceded, in its job or in a job it
//!   `needs`, by a step that **runs the image the job built** (its own entrypoint) (`docker run … "${IMAGE_ID}"`, the id read from the
//!   build's `--iidfile` and checked to be `sha256:<64 hex>` in the same step) and by a step that runs
//!   `planeter-glibc-check`. A `docker run` of that locally built image id is the one `docker run` that needs no
//!   `@sha256:` in its line;
//! - a pushed image that was not checked to be the image that was run (D-51, identity): in a job that pushes an image **by digest**
//!   (`push-by-digest` to `ghcr.io`), a step that runs `planeter-image-identity … same` must come after each such push and before the
//!   next push or the upload of the digest (`actions/upload-artifact`), so the digest is not handed on unchecked.
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
            let mut hits = scan_workflow(&text);
            hits.extend(scan_release_paths(&text));
            hits.extend(scan_image_gate(&text));
            hits.extend(scan_image_identity(&text));
            hits
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

/// The id of the one job allowed to build with a different compiler: the declared minimum (IS-32).
const MSRV_JOB: &str = "msrv";

/// The workspace's declared minimum compiler as `(line, "X.Y.Z")`, from the root `Cargo.toml`. `None`
/// when there is no root manifest or it declares none.
fn declared_minimum(root: &Path) -> io::Result<Option<(usize, String)>> {
    let Ok(text) = fs::read_to_string(root.join("Cargo.toml")) else {
        return Ok(None);
    };
    for (i, line) in text.lines().enumerate() {
        let code = strip_comment(line).trim();
        let value = code
            .strip_prefix("rust-version")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .map(|v| v.trim().trim_matches(['"', '\'']).to_owned());
        if let Some(v) = value {
            let mut parts: Vec<&str> = v.split('.').collect();
            if parts.len() == 2 {
                parts.push("0");
            }
            return Ok(Some((i + 1, parts.join("."))));
        }
    }
    Ok(None)
}

/// The id of the job a line of a workflow belongs to: a two-space-indented `id:` key under `jobs:`.
fn job_id(line: &str) -> Option<&str> {
    let code = strip_comment(line).trim_end();
    let rest = code.strip_prefix("  ")?;
    if rest.starts_with(' ') {
        return None;
    }
    let id = rest.strip_suffix(':')?;
    (!id.is_empty() && !id.contains([' ', ':'])).then_some(id)
}

/// Findings when the workflows do not agree on a single compiler, outside the one named exception: the
/// `msrv` job, whose compiler must be the declared minimum. Inputs that are not `X.Y.Z` are already
/// findings of their own and are left out of the comparison.
fn toolchain_disagreements(root: &Path, workflows: &[PathBuf]) -> io::Result<Vec<Finding>> {
    let mut inputs: Vec<(String, usize, String)> = Vec::new();
    let mut msrv_inputs: Vec<(String, usize, String)> = Vec::new();
    for rel in workflows {
        let text = fs::read_to_string(root.join(rel))?;
        let file = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let mut job = "";
        for (i, line) in text.lines().enumerate() {
            if let Some(id) = job_id(line) {
                job = id;
            }
            let named = value_of(line, "toolchain")
                .map(str::to_owned)
                .or_else(|| rustup_toolchain_arg(line));
            if let Some(v) = named
                && is_semver(&v)
            {
                let list = if job == MSRV_JOB {
                    &mut msrv_inputs
                } else {
                    &mut inputs
                };
                list.push((file.clone(), i + 1, v));
            }
        }
    }
    let mut findings = msrv_findings(root, &msrv_inputs)?;
    let mut versions: Vec<&str> = inputs.iter().map(|(_, _, v)| v.as_str()).collect();
    versions.sort_unstable();
    versions.dedup();
    if versions.len() < 2 {
        return Ok(findings);
    }
    let summary = inputs
        .iter()
        .map(|(f, l, v)| format!("{v} in {f}:{l}"))
        .collect::<Vec<_>>()
        .join("; ");
    findings.extend(inputs.iter().map(|(file, line, v)| Finding {
        file: file.clone(),
        line: *line,
        message: format!(
            "toolchain {v:?} disagrees with the other workflows; CI and release must use one version ({summary})"
        ),
    }));
    Ok(findings)
}

/// The `msrv` job's compiler is the declared minimum, and the job exists when a minimum is declared.
fn msrv_findings(root: &Path, msrv_inputs: &[(String, usize, String)]) -> io::Result<Vec<Finding>> {
    let Some((line, declared)) = declared_minimum(root)? else {
        return Ok(Vec::new());
    };
    if msrv_inputs.is_empty() {
        return Ok(vec![Finding {
            file: "Cargo.toml".to_owned(),
            line,
            message: format!(
                "rust-version {declared} is declared but no job `{MSRV_JOB}` in .github/workflows builds with it"
            ),
        }]);
    }
    Ok(msrv_inputs
        .iter()
        .filter(|(_, _, v)| *v != declared)
        .map(|(file, l, v)| Finding {
            file: file.clone(),
            line: *l,
            message: format!(
                "job `{MSRV_JOB}` builds with {v:?} but Cargo.toml:{line} declares rust-version {declared}; they must be the same version"
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
    matches!(w, "stable" | "beta" | "nightly" | "latest")
}

/// The toolchain a `rustup` command line selects, as written: the first argument after
/// `rustup toolchain install` / `rustup install` / `rustup default` / `rustup update` /
/// `rustup override set`, skipping flags and the values of the flags that take one. `None` when the line
/// has no such command or names no toolchain.
fn rustup_toolchain_arg(line: &str) -> Option<String> {
    let code = strip_comment(line);
    let words: Vec<&str> = code
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|'))
        .filter(|w| !w.is_empty())
        .collect();
    let at = words.iter().position(|w| *w == "rustup")?;
    let rest = &words[at + 1..];
    let skip = match rest {
        ["toolchain", "install", ..] | ["override", "set", ..] => 2,
        ["install" | "default" | "update", ..] => 1,
        _ => return None,
    };
    let mut args = rest[skip..].iter();
    while let Some(w) = args.next() {
        if matches!(*w, "--profile" | "--component" | "-c" | "--target" | "-t") {
            args.next();
        } else if !w.starts_with('-') {
            return Some(w.trim_matches(['"', '\'']).to_owned());
        }
    }
    None
}

fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let t = strip_comment(line).trim();
    let t = t.strip_prefix("- ").map_or(t, str::trim_start);
    let rest = t.strip_prefix(key)?.strip_prefix(':')?;
    Some(rest.trim().trim_matches(['"', '\'']))
}

/// `true` when line `i` is a `docker run` of the image this job just built: its image argument is `"${IMAGE_ID}"` and, within the
/// ten lines before it (the same step), `IMAGE_ID` was checked to be `sha256:<64 hex>`.
fn runs_local_image_id(text: &str, i: usize) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let Some(line) = lines.get(i) else {
        return false;
    };
    if !line.contains("\"${IMAGE_ID}\"") {
        return false;
    }
    let from = i.saturating_sub(10);
    lines[from..i]
        .iter()
        .any(|l| l.contains("IMAGE_ID") && l.contains("grep -Eqx 'sha256:[0-9a-f]{64}'"))
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
                hits.push((
                    n,
                    format!("action {v:?} is not from the `actions/` owner (RFC 010 D-11)"),
                ));
                continue;
            }
            if !v.starts_with("actions/") {
                hits.push((
                    n,
                    format!("action {v:?} is not from the `actions/` owner (RFC 010 D-11)"),
                ));
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
        } else if let Some(v) = value_of(line, "image")
            && !v.is_empty()
            && !v.contains("${{")
            && !v.contains("@sha256:")
        {
            hits.push((n, format!("image {v:?} has no @sha256: digest")));
        }
        let code = strip_comment(line);
        if (code.contains("secrets.") || code.contains("github.token")) && code.contains("${{") {
            let t = code.trim();
            let t = t.strip_prefix("- ").map_or(t, str::trim_start);
            let key = t.split_once(':').map(|(k, _)| k.trim());
            let is_mapping = key.is_some_and(|k| {
                !k.is_empty()
                    && k != "run"
                    && k.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            });
            if !is_mapping {
                hits.push((
                    n,
                    "a secret or the job token is interpolated into a script or command line; pass it in `env:` and pipe it on standard input".to_owned(),
                ));
            }
        }
        for cmd in ["docker run", "docker pull"] {
            let local_id = cmd == "docker run" && runs_local_image_id(text, i);
            if code.contains(cmd) && !has_digest_ref(code) && !local_id {
                hits.push((
                    n,
                    format!("`{cmd}` of an image not referenced by @sha256:<64 hex> digest"),
                ));
            }
        }
        // `rustup default stable`, `rustup toolchain install nightly`, `rustup update beta`, …
        if code.contains("rustup") && code.split_whitespace().any(is_channel) {
            hits.push((
                n,
                "rustup selects a toolchain by channel, not by X.Y.Z".to_owned(),
            ));
        } else if let Some(v) = rustup_toolchain_arg(line)
            && !is_semver(&v)
        {
            hits.push((
                n,
                format!("rustup selects toolchain {v:?}, which is not an explicit X.Y.Z version"),
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

/// The real image: a step that names it may run only on a tag push.
const REAL_IMAGE: &str = "ghcr.io/prikk-vcs/planeter";
/// The throwaway package of a rehearsal (RFC 010 D-10).
const REHEARSAL_IMAGE: &str = "ghcr.io/prikk-vcs/planeter-rehearsal";
/// The condition that makes a step run in a rehearsal and only there.
const REHEARSAL_COND: &str = "inputs.mode == 'rehearsal'";
const PUSH_COND: &str = "github.event_name == 'push'";
/// Commands and actions of a release, a tag or crates.io: never in a rehearsal.
const RELEASE_WORDS: [&str; 10] = [
    "gh release",
    "git tag",
    "git push",
    "cargo publish",
    "cargo login",
    "crates.io",
    "action-gh-release",
    "create-release",
    "softprops/",
    "ncipollo/",
];

/// `true` when `text` names the real image (`…/planeter` followed by anything but `-`, so the
/// rehearsal package `…/planeter-rehearsal` does not count).
fn names_real_image(text: &str) -> bool {
    text.match_indices(REAL_IMAGE).any(|(i, _)| {
        !text[i + REAL_IMAGE.len()..]
            .starts_with(|c: char| c == '-' || c == '_' || c.is_ascii_alphanumeric())
    })
}

/// The expression of an `if:` line, whole: a quoted YAML scalar loses its quotes, an expression that
/// ends in a string literal (`… == 'push'`) keeps its own.
fn if_expr(line: &str) -> Option<String> {
    let t = strip_comment(line).trim();
    let t = t.strip_prefix("- ").map_or(t, str::trim_start);
    let v = t.strip_prefix("if:")?.trim();
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|x| x.strip_suffix(q)) {
            return Some(inner.to_owned());
        }
    }
    Some(v.to_owned())
}

/// `true` when `text` contains a `…@sha256:<64 lowercase hex>` reference.
fn has_digest_ref(text: &str) -> bool {
    text.match_indices("@sha256:")
        .any(|(i, m)| is_hex(text[i + m.len()..].get(..64).unwrap_or(""), 64))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// One workflow step: where it starts, its own `if`, its job's `if`, and its text without comments.
struct Step {
    line: usize,
    condition: String,
    text: String,
}

/// The steps of every job of a workflow (the repository's shape: jobs at two spaces, `steps:` at four,
/// items at six). A step's condition is its own `if` joined with its job's `if`.
fn steps_of(text: &str) -> Vec<Step> {
    let lines: Vec<&str> = text.lines().collect();
    let mut jobs: Vec<(usize, usize, String)> = Vec::new(); // (first line, end, job-level if)
    let mut in_jobs = false;
    for (i, line) in lines.iter().enumerate() {
        if strip_comment(line).trim_end() == "jobs:" {
            in_jobs = true;
            continue;
        }
        if in_jobs && job_id(line).is_some() {
            if let Some(last) = jobs.last_mut() {
                last.1 = i;
            }
            jobs.push((i, lines.len(), String::new()));
        } else if in_jobs
            && indent_of(line) == 4
            && let Some(v) = if_expr(line)
            && let Some(last) = jobs.last_mut()
        {
            last.2 = v;
        }
    }
    let mut steps = Vec::new();
    for (start, end, job_if) in jobs {
        let mut in_steps = false;
        let mut current: Option<Step> = None;
        for (i, line) in lines.iter().enumerate().take(end).skip(start) {
            let code = strip_comment(line);
            if code.trim().is_empty() {
                continue;
            }
            let ind = indent_of(code);
            if ind == 4 && code.trim() == "steps:" {
                in_steps = true;
                continue;
            }
            if in_steps && ind <= 4 {
                in_steps = false;
            }
            if !in_steps {
                continue;
            }
            if ind == 6 && code.trim_start().starts_with("- ") {
                steps.extend(current.take());
                current = Some(Step {
                    line: i + 1,
                    condition: job_if.clone(),
                    text: String::new(),
                });
            }
            if let Some(step) = current.as_mut() {
                if (ind == 6 || ind == 8)
                    && let Some(v) = if_expr(code)
                {
                    step.condition = format!("{} {}", step.condition, v);
                }
                step.text.push_str(code);
                step.text.push('\n');
            }
        }
        steps.extend(current.take());
    }
    steps
}

/// A step that pushes: `push:` / `push-to-registry:` with any value but `false`, `docker push`, or a
/// manifest push.
fn pushes(text: &str) -> bool {
    text.lines().any(|l| {
        value_of(l, "push").is_some_and(|v| v != "false")
            || l.contains("push=true")
            || l.split_whitespace().any(|w| w == "--push")
            || value_of(l, "push-to-registry").is_some_and(|v| v != "false")
            || l.contains("docker push")
            || l.contains("imagetools create")
    })
}

/// RFC 010 D-10: the rehearsal cannot reach the real package, structurally.
fn scan_release_paths(text: &str) -> Hits {
    let mut hits = Hits::new();
    for step in steps_of(text) {
        let names_real = names_real_image(&step.text);
        let names_rehearsal = step.text.contains(REHEARSAL_IMAGE);
        let rehearsal_conditioned = step.condition.contains("rehearsal");
        let n = step.line;
        if step.text.contains("docker buildx create") {
            let pinned = step.text.lines().any(|l| {
                l.split_whitespace().any(|w| {
                    w.strip_prefix("image=")
                        .is_some_and(|r| has_digest_ref(r) && !r.contains('$'))
                })
            });
            if !pinned {
                hits.push((
                    n,
                    "a `docker buildx create` step must set `--driver-opt image=<ref>@sha256:<64 hex>` (the BuildKit image pinned by digest)".to_owned(),
                ));
            }
        }
        if step.text.contains("docker login") {
            let mut words = step.text.split_whitespace();
            let bad_flag =
                words.any(|w| w == "--password" || w == "-p" || w.starts_with("--password="));
            if bad_flag || !step.text.contains("--password-stdin") {
                hits.push((
                    n,
                    "a `docker login` must read the credential with `--password-stdin` (never `--password` / `-p`)".to_owned(),
                ));
            }
            if !(step.condition.contains(PUSH_COND) || step.condition.contains(REHEARSAL_COND)) {
                hits.push((
                    n,
                    format!(
                        "a registry login must run only on a tag push ({PUSH_COND}) or in a rehearsal ({REHEARSAL_COND})"
                    ),
                ));
            }
        }
        if names_real && !step.condition.contains(PUSH_COND) {
            hits.push((
                n,
                format!(
                    "a step naming the real image {REAL_IMAGE} must have {PUSH_COND} in its `if` (it must not run in a dispatch)"
                ),
            ));
        }
        if names_real && names_rehearsal {
            hits.push((
                n,
                format!("a step names both the real image {REAL_IMAGE} and the rehearsal package"),
            ));
        }
        if (names_rehearsal || rehearsal_conditioned) && !step.condition.contains(REHEARSAL_COND) {
            hits.push((
                n,
                format!("a step in the rehearsal path must have {REHEARSAL_COND} in its `if`"),
            ));
        }
        if rehearsal_conditioned
            && (step.condition.contains(PUSH_COND) || step.condition.contains("||"))
        {
            hits.push((
                n,
                format!(
                    "a rehearsal step can also run on a tag push ({PUSH_COND} or `||` in its `if`)"
                ),
            ));
        }
        if rehearsal_conditioned && pushes(&step.text) && !names_rehearsal {
            hits.push((
                n,
                format!("a rehearsal step pushes to something other than {REHEARSAL_IMAGE}"),
            ));
        }
        if rehearsal_conditioned
            && let Some(w) = RELEASE_WORDS.iter().find(|w| step.text.contains(**w))
        {
            hits.push((
                n,
                format!(
                    "a rehearsal step names a release, tag or crates.io action or command ({w:?})"
                ),
            ));
        }
    }
    hits
}

// ---------------------------------------------------------------- the image gate (D-51)

/// One job of a workflow: its id, the jobs it needs, and its steps in order.
struct Job {
    id: String,
    needs: Vec<String>,
    steps: Vec<Step>,
}

/// The jobs of a workflow (the repository's shape: jobs at two spaces, `needs:` at four).
fn jobs_of(text: &str) -> Vec<Job> {
    let lines: Vec<&str> = text.lines().collect();
    let mut starts: Vec<(usize, String)> = Vec::new();
    let mut in_jobs = false;
    for (i, line) in lines.iter().enumerate() {
        if strip_comment(line).trim_end() == "jobs:" {
            in_jobs = true;
        } else if in_jobs && let Some(id) = job_id(line) {
            starts.push((i, id.to_owned()));
        }
    }
    let all_steps = steps_of(text);
    let mut jobs = Vec::new();
    for (k, (start, id)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(lines.len(), |(s, _)| *s);
        let mut needs = Vec::new();
        for line in &lines[*start..end] {
            if indent_of(line) == 4
                && let Some(v) = value_of(line, "needs")
            {
                needs.extend(
                    v.trim_matches(['[', ']'])
                        .split(',')
                        .map(|w| w.trim().trim_matches(['"', '\'']).to_owned())
                        .filter(|w| !w.is_empty()),
                );
            }
        }
        let steps = all_steps
            .iter()
            .filter(|s| s.line > *start && s.line <= end)
            .map(|s| Step {
                line: s.line,
                condition: s.condition.clone(),
                text: s.text.clone(),
            })
            .collect();
        jobs.push(Job {
            id: id.clone(),
            needs,
            steps,
        });
    }
    jobs
}

/// A step that publishes an image: it pushes to a registry, or it attests an image.
fn publishes_image(step: &Step) -> bool {
    let names_registry = step.text.contains("ghcr.io");
    names_registry && (pushes(&step.text) || step.text.contains("actions/attest"))
}

/// The image's own entrypoint (planeter) is what is run: a `docker run --entrypoint …` of some other program proves nothing.
fn runs_the_image(step: &Step) -> bool {
    step.text.lines().any(|l| {
        l.contains("docker run") && l.contains("\"${IMAGE_ID}\"") && !l.contains("--entrypoint")
    })
}

fn runs_the_glibc_check(step: &Step) -> bool {
    step.text.contains("planeter-glibc-check")
}

/// D-51: nothing that is pushed or attested as an image before the image has been run and its binary's glibc needs checked.
fn scan_image_gate(text: &str) -> Hits {
    let jobs = jobs_of(text);
    // A job is a gate when both steps come before its first publishing step (or it publishes nothing).
    let gated = |job: &Job| {
        let first_publish = job
            .steps
            .iter()
            .position(publishes_image)
            .unwrap_or(job.steps.len());
        let before = &job.steps[..first_publish];
        before.iter().any(runs_the_image) && before.iter().any(runs_the_glibc_check)
    };
    let mut hits = Hits::new();
    for job in &jobs {
        let Some(step) = job.steps.iter().find(|s| publishes_image(s)) else {
            continue;
        };
        if gated(job) {
            continue;
        }
        // Or every route into it passes a gate: the jobs it needs (transitively) include one.
        let mut seen: Vec<&str> = Vec::new();
        let mut stack: Vec<&str> = job.needs.iter().map(String::as_str).collect();
        let mut upstream_gate = false;
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(j) = jobs.iter().find(|j| j.id == id) {
                if gated(j) && j.steps.iter().any(runs_the_image) {
                    upstream_gate = true;
                }
                stack.extend(j.needs.iter().map(String::as_str));
            }
        }
        if !upstream_gate {
            hits.push((
                step.line,
                format!(
                    "job `{}` pushes or attests an image without a step before it that runs the image (`docker run … \"${{IMAGE_ID}}\"`) and a step that runs `planeter-glibc-check`, in this job or in a job it needs (D-51)",
                    job.id
                ),
            ));
        }
    }
    hits
}

/// A step that pushes an image to the registry by digest (the way the matrix jobs hand a digest on).
fn pushes_by_digest(step: &Step) -> bool {
    step.text.contains("ghcr.io") && step.text.contains("push-by-digest") && pushes(&step.text)
}

fn uploads_an_artifact(step: &Step) -> bool {
    step.text.contains("actions/upload-artifact")
}

fn runs_the_identity_check(step: &Step) -> bool {
    step.text
        .lines()
        .any(|l| l.contains("planeter-image-identity") && l.split_whitespace().any(|w| w == "same"))
}

/// D-51 (identity): after every push by digest, and before the next push or the upload of the digest, the registry's answer is
/// compared with the image that was run.
fn scan_image_identity(text: &str) -> Hits {
    let mut hits = Hits::new();
    for job in jobs_of(text) {
        for (i, step) in job.steps.iter().enumerate() {
            if !pushes_by_digest(step) {
                continue;
            }
            let end = job.steps[i + 1..]
                .iter()
                .position(|s| pushes_by_digest(s) || uploads_an_artifact(s))
                .map_or(job.steps.len(), |k| i + 1 + k);
            if !job.steps[i + 1..end].iter().any(runs_the_identity_check) {
                hits.push((
                    step.line,
                    format!(
                        "job `{}` pushes an image by digest without a step after it, before the next push or the upload of the digest, that runs `planeter-image-identity … same` (D-51: the pushed image must be the image that was run)",
                        job.id
                    ),
                ));
            }
        }
    }
    hits
}
