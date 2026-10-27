//! [`CliPrikkRepo`] — the subprocess implementation of [`PrikkRepo`](crate::PrikkRepo) (RFC 001 D-2/D-3).
//!
//! It shells out to the prikk binary against one repository directory (the one holding `.prikk`),
//! parses `--format json` output into the [`crate::model`] types after checking the `schema_version`,
//! and maps a non-zero exit to a typed [`PrikkError`] (never a panic). The version pin (D-3) refuses a
//! prikk older than planeter's floor (PK-26/PK-42: planeter requires prikk ≥ 0.54.0). Confinement of the
//! subprocess (sandbox) is layered on in T4 — this module invokes prikk directly.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::error::{PrikkError, Result};
use crate::model;
use crate::sandbox::{CallLimits, Run, Sandbox, TIMEOUT_EXIT_CODE};

/// planeter's minimum supported prikk version (RFC 001 D-3 / dependency-ledger PK-26).
///
/// Raised to 0.46.0 (2026-09-22): it ships the read verbs planeter's browse needs — `tree`
/// (`tree-listing-v1`), `cat` (`path-content-v1`), `diff` (`diff-report-v1`) — and carries the 0.44.0
/// `bundle import` corruption fix (GHSA-px5q-233r-6hq5), the 0.45.0 format-7 signature union, and the
/// key-id fix.
///
/// Raised to 0.52.0 (2026-10-15, re-baseline handoff v2): the tested version. It adds the 0.47.0 PK-30
/// fix and accept size bound, the 0.49.0 crash-state recovery, dependency-ordered `bundle import` /
/// `sync accept` writes and linear resync over hostile content, and the 0.52.0 archive closure fix.
/// The pin refuses anything older rather than guess.
///
/// Raised to 0.54.0 (2026-10-24, the 0.54 re-baseline, ledger PK-70): it fixes `show <block-id>` for a block that edits a
/// file three or more times (PK-57, planeter IS-46), ships `extract` with `largest_file_bytes`, `claim_parent_block_id` and
/// the `refusal-v1` document (the three things planeter asked for, PK-56/PK-66), and adds the `unreached objects` line to
/// `verify`. 0.54.1 is an installer fix; prikk's code is 0.54.0's.
pub const MIN_PRIKK_VERSION: (u64, u64, u64) = (0, 54, 0);

/// What the start-up probe of the prikk binary found ([`probe_binary`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryProbe {
    /// This version, at or above [`MIN_PRIKK_VERSION`].
    Supported(String),
    /// Below the floor: the pin's own sentence (`unsupported prikk version X; planeter requires >= 0.54.0`).
    Unsupported(String),
    /// There is no such program. Reported as it always was: when a repository is first opened.
    Missing,
    /// It ran and said something this code cannot read, or it failed: the text.
    Unreadable(String),
}

/// Ask the prikk binary its version, once, at start-up (under `prlimit`, so a broken binary cannot take the host's memory),
/// and apply the same pin that every repository open applies. The server refuses to start on [`BinaryProbe::Unsupported`]
/// instead of answering every read with "temporarily unavailable".
pub fn probe_binary(binary: &std::ffi::OsStr) -> BinaryProbe {
    use std::process::{Command, Stdio};
    let out = Command::new(crate::sandbox::PRLIMIT)
        .arg("--cpu=20")
        .arg(format!("--data={}", 512u64 << 20))
        .arg("--")
        .arg(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return BinaryProbe::Unreadable(e.to_string()),
    };
    if !out.status.success() {
        // `prlimit` itself cannot find the program: exit 127 (or 126), and its own message on stderr.
        let err = String::from_utf8_lossy(&out.stderr);
        if matches!(out.status.code(), Some(126 | 127))
            && (err.contains("No such file") || err.contains("not found"))
        {
            return BinaryProbe::Missing;
        }
        return BinaryProbe::Unreadable(format!(
            "`--version` exited with {}: {}",
            out.status,
            err.trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(found) = parse_version_line(&text) else {
        return BinaryProbe::Unreadable(format!("unrecognized --version output: {text:?}"));
    };
    match parse_semver(found) {
        Some(v) if v >= MIN_PRIKK_VERSION => BinaryProbe::Supported(found.to_owned()),
        Some(_) => BinaryProbe::Unsupported(
            PrikkError::UnsupportedVersion {
                found: found.to_owned(),
                required: format!(
                    ">= {}.{}.{}",
                    MIN_PRIKK_VERSION.0, MIN_PRIKK_VERSION.1, MIN_PRIKK_VERSION.2
                ),
            }
            .to_string(),
        ),
        None => BinaryProbe::Unreadable(format!("unparsable prikk version {found:?}")),
    }
}

/// A prikk repository driven through the prikk CLI as a subprocess.
#[derive(Debug, Clone)]
pub struct CliPrikkRepo {
    /// The working root that contains the `.prikk` directory.
    root: PathBuf,
    /// The prikk binary to invoke (default: `prikk` on `PATH`).
    binary: OsString,
    /// How the prikk subprocess is confined (default: bubblewrap — RFC 001 T4).
    sandbox: Sandbox,
    /// Per-call limits (RFC 011 D-2): none by default.
    limits: CallLimits,
}

impl CliPrikkRepo {
    /// Bind to the repository at `root` (the directory containing `.prikk`), using `prikk` from `PATH`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            binary: OsString::from("prikk"),
            sandbox: Sandbox::default(),
            limits: CallLimits::NONE,
        }
    }

    /// Bind with an explicit prikk binary path.
    pub fn with_binary(root: impl Into<PathBuf>, binary: impl Into<OsString>) -> Self {
        Self {
            root: root.into(),
            binary: binary.into(),
            sandbox: Sandbox::default(),
            limits: CallLimits::NONE,
        }
    }

    /// Set the confinement policy (default: bubblewrap). Use [`Sandbox::Unconfined`] only in dev, never
    /// to host untrusted input (RFC 001 T4).
    #[must_use]
    pub fn with_sandbox(mut self, sandbox: Sandbox) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// Apply per-call limits (a memory ceiling) to every invocation made through this handle
    /// (RFC 011 D-2). A call that hits the ceiling fails as [`PrikkError::CeilingExceeded`].
    #[must_use]
    pub fn with_limits(mut self, limits: CallLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The repository root this driver is bound to (the directory holding `.prikk`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// This driver with its sandbox replaced by `sandbox` (for a call that needs another mount).
    pub(crate) fn sandboxed_as(&self, sandbox: Sandbox) -> Self {
        Self {
            sandbox,
            ..self.clone()
        }
    }

    /// The sandbox policy of this driver.
    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    /// Bind and **verify the prikk version is in planeter's supported range** (RFC 001 D-3): refuse an
    /// out-of-range binary rather than guess.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let repo = Self::new(root);
        repo.check_version()?;
        Ok(repo)
    }

    /// Create a new prikk repository at `root` via `prikk init`, then bind to it with the version pin
    /// (RFC 001 D-3). `root` must already exist; confinement binds it as the one writable location, so
    /// `prikk init` writes its `.prikk` there. A non-zero exit (e.g. an existing `.prikk`) is a typed
    /// [`PrikkError::Command`].
    pub fn init(root: impl Into<PathBuf>) -> Result<Self> {
        let repo = Self::new(root);
        repo.check_version()?;
        repo.run(&["init", "."])?;
        Ok(repo)
    }

    /// Create with an explicit binary/sandbox already configured, then `prikk init` in place.
    pub fn init_configured(self) -> Result<Self> {
        self.check_version()?;
        self.run(&["init", "."])?;
        Ok(self)
    }

    /// Run the version pin (RFC 001 D-3) on an already-configured instance (binary/sandbox set) and
    /// return it, or a typed error if prikk is out of range. The configured analogue of [`Self::open`].
    pub fn checked(self) -> Result<Self> {
        self.check_version()?;
        Ok(self)
    }

    /// Return the prikk binary's version string (from `prikk --version`, e.g. `"0.43.0"`).
    fn version_string(&self) -> Result<String> {
        let out = self.run(&["--version"])?;
        parse_version_line(&out)
            .map(str::to_owned)
            .ok_or_else(|| PrikkError::Parse(format!("unrecognized --version output: {out:?}")))
    }

    /// Refuse a prikk older than [`MIN_PRIKK_VERSION`].
    fn check_version(&self) -> Result<()> {
        let found = self.version_string()?;
        let parsed = parse_semver(&found)
            .ok_or_else(|| PrikkError::Parse(format!("unparsable prikk version {found:?}")))?;
        if parsed >= MIN_PRIKK_VERSION {
            Ok(())
        } else {
            Err(PrikkError::UnsupportedVersion {
                found,
                required: format!(
                    ">= {}.{}.{}",
                    MIN_PRIKK_VERSION.0, MIN_PRIKK_VERSION.1, MIN_PRIKK_VERSION.2
                ),
            })
        }
    }

    /// Run one prikk subcommand under the sandbox, the limits and the server-side wall-time bound.
    pub(crate) fn execute(&self, args: &[&str]) -> Result<Run> {
        self.sandbox
            .run(&self.binary, &self.root, args, self.limits)
            .map_err(PrikkError::Spawn)
    }

    /// The typed failure for a call that did not succeed.
    ///
    /// - the wall-time bound fired: the status `timeout` used to report ([`TIMEOUT_EXIT_CODE`]);
    /// - a **ceiling hit** (only when a ceiling was set): prikk aborted on an allocation failure (exit 134
    ///   behind `bwrap`, `SIGABRT` unconfined, or the Rust "memory allocation of … failed" line), or
    ///   `verify` stopped with "did not complete every stage". Either way the result is an error, never a
    ///   verdict. (`verify` itself decides between a ceiling hit and a genuine failure from the report on
    ///   stdout, not from this text: see `verify`, IS-39.)
    /// - anything else: the plain non-zero exit.
    pub(crate) fn failure(&self, args: &[&str], run: &Run) -> PrikkError {
        use std::os::unix::process::ExitStatusExt;
        if run.timed_out {
            return PrikkError::Command {
                code: Some(TIMEOUT_EXIT_CODE),
                stderr: String::new(),
            };
        }
        let stderr = String::from_utf8_lossy(&run.stderr);
        if let Some(bytes) = self.limits.data_ceiling {
            let aborted = run.status.code() == Some(134) || run.status.signal() == Some(6);
            let alloc_failed = stderr.contains("memory allocation of") && stderr.contains("failed");
            let partial_verify =
                run.status.code() == Some(1) && stderr.contains("did not complete every stage");
            if aborted || alloc_failed || partial_verify {
                return PrikkError::CeilingExceeded {
                    verb: args.first().copied().unwrap_or("").to_owned(),
                    ceiling_bytes: bytes,
                };
            }
        }
        PrikkError::Command {
            code: run.status.code(),
            stderr: stderr.into_owned(),
        }
    }

    /// Run a prikk subcommand, returning raw stdout bytes. A non-zero exit is a typed
    /// [`PrikkError::Command`] (or [`PrikkError::CeilingExceeded`]). Used by the byte-oriented verbs
    /// (e.g. `cat` of binary content).
    fn run_bytes(&self, args: &[&str]) -> Result<Vec<u8>> {
        let run = self.execute(args)?;
        if run.status.success() && !run.timed_out {
            Ok(run.stdout)
        } else {
            Err(self.failure(args, &run))
        }
    }

    /// Run a prikk subcommand, returning its stdout. A non-zero exit is a typed [`PrikkError::Command`]
    /// (or [`PrikkError::CeilingExceeded`]).
    fn run(&self, args: &[&str]) -> Result<String> {
        let run = self.execute(args)?;
        if run.status.success() && !run.timed_out {
            String::from_utf8(run.stdout)
                .map_err(|e| PrikkError::Parse(format!("prikk stdout was not UTF-8: {e}")))
        } else {
            Err(self.failure(args, &run))
        }
    }

    /// Run a JSON-emitting read command: parse stdout, **check `schema_version`**, deserialize to `T`.
    fn run_json<T: DeserializeOwned>(&self, args: &[&str], expected_schema: &str) -> Result<T> {
        let stdout = self.run(args)?;
        parse_schema_json(&stdout, expected_schema)
    }

    /// Collect non-empty trimmed stdout lines (used by the exchange verbs whose exact output is
    /// finalized against the binary when transport is built — RFC 004).
    fn run_lines(&self, args: &[&str]) -> Result<Vec<String>> {
        Ok(self
            .run(args)?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

/// Parse a prikk `--format json` document: it must be JSON, carry `schema_version == expected_schema`
/// (drift is refused loudly, PK-18), and deserialize into `T`.
pub(crate) fn parse_schema_json<T: DeserializeOwned>(
    stdout: &str,
    expected_schema: &str,
) -> Result<T> {
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|e| PrikkError::Parse(format!("invalid JSON: {e}")))?;
    match value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
    {
        Some(s) if s == expected_schema => {}
        Some(other) => {
            return Err(PrikkError::Parse(format!(
                "unexpected schema_version {other:?} (expected {expected_schema:?}) — prikk JSON may have drifted (PK-18)"
            )));
        }
        None => return Err(PrikkError::Parse("missing schema_version".to_owned())),
    }
    serde_json::from_value(value).map_err(|e| PrikkError::Parse(e.to_string()))
}

/// Whether a stage's failure message is the process running out of memory — what a `RLIMIT_DATA` ceiling
/// does to prikk's `objects` stage — as opposed to a finding about the repository.
///
/// The shape is the measured one and nothing looser: the message **starts with** prikk's error-class prefix
/// `i/o error:` and what follows is **exactly** the out-of-memory condition. On prikk 0.52.0, across six
/// repositories and about a hundred ceilings each, every ceiling hit that prints a report prints exactly
/// `i/o error: out of memory` (the rest abort: exit 134 / `SIGABRT`, which never reaches this function). The
/// second phrase is the text Rust gives for the raw `ENOMEM` (`os error 12`); it has not been seen from prikk
/// and is accepted only in full, never as a substring. A message can embed data the repository owner chooses
/// (a ref or a path name); a name containing "out of memory" must not turn damage into a ceiling hit, so no
/// substring of a message is ever matched. Re-measure the message at every prikk re-baseline.
fn is_resource_failure(message: &str) -> bool {
    message
        .strip_prefix("i/o error:")
        .map(str::trim)
        .is_some_and(|rest| {
            rest == "out of memory" || rest == "Cannot allocate memory (os error 12)"
        })
}

/// A failing report that is a complete verdict about the repository: every stage ran to an outcome, or was
/// not run **because a stage of the same report failed with a finding** (see [`CliPrikkRepo::verify`]).
///
/// * `evaluated`: ran. `failed` with a finding: ran, and found damage. `failed` out of memory: a ceiling hit.
/// * `not_evaluated` carries the id of the stage that blocked it (`blocked_by`, a field of the report, not
///   text to parse). It is part of the verdict only if that id is a stage of this report that is `failed` with
///   a finding; a blocker that is itself `not_evaluated`, missing from the report, unknown, or failed for lack
///   of memory leaves the report partial.
/// * `halted`: the run stopped; partial.
fn is_complete_verdict(report: &model::VerifyReport) -> bool {
    // A blocker must be a stage of this report that failed. That it failed with a finding and not for lack
    // of memory needs no second check here: a resource failure is itself a stage of the report, and the
    // `Failed` arm below rejects the whole report for it.
    let failed = |id: &str| {
        report
            .stages
            .iter()
            .any(|s| matches!(s, model::Stage::Failed { stage, .. } if stage == id))
    };
    report.stages.iter().all(|s| match s {
        model::Stage::Evaluated { .. } => true,
        model::Stage::Failed { message, .. } => !is_resource_failure(message),
        model::Stage::NotEvaluated { blocked_by, .. } => failed(blocked_by),
        model::Stage::Halted { .. } => false,
    })
}

/// Extract the version token from a `prikk --version` line (`"prikk 0.43.0\n"` → `"0.43.0"`).
fn parse_version_line(line: &str) -> Option<&str> {
    line.trim().strip_prefix("prikk ").map(str::trim)
}

/// Parse a `major.minor.patch` core from a version string (ignoring any `-pre`/`+build` suffix).
fn parse_semver(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .trim()
        .split(['-', '+'])
        .next()
        .unwrap_or(version)
        .trim();
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

impl super::repo::PrikkRepo for CliPrikkRepo {
    fn version(&self) -> Result<String> {
        self.version_string()
    }

    fn log(&self, ref_name: Option<&str>, limit: Option<usize>) -> Result<model::LogReport> {
        let mut args = vec!["log", "--format", "json"];
        let limit_s;
        if let Some(r) = ref_name {
            args.extend(["--ref", r]);
        }
        if let Some(n) = limit {
            limit_s = n.to_string();
            args.extend(["--limit", &limit_s]);
        }
        self.run_json(&args, "log-report-v1")
    }

    fn show(&self, target: &str) -> Result<model::ShowReport> {
        self.run_json(&["show", target, "--format", "json"], "show-report-v1")
    }

    /// Re-derive the repository's verification (`prikk verify`).
    ///
    /// prikk exits **1 when verification fails and still prints the failing `verify-report-v1`** on
    /// stdout; that report *is* the answer ("this repository does not verify, and why"), so it is returned
    /// as `Ok` — like `trust_check` reads exit 1 — and not turned into an outage (IS-39). Only
    /// the following is read as a verdict:
    ///
    /// * exit status **1** (never 2, 124, 134 or a signal) and no timeout;
    /// * stdout that parses as `verify-report-v1` with `verdict.ok == false`;
    /// * **every stage ran to an outcome** (`is_complete_verdict`): none `halted`; no `failed` stage whose
    ///   message is the out-of-memory condition (`is_resource_failure`, exact shape); and every
    ///   `not_evaluated` stage names, in its `blocked_by` field, a stage of the same report that failed with a
    ///   finding. A stage that *failed* because the repository is damaged ("integrity error: …") is part of
    ///   the verdict; one that failed for lack of memory is a memory ceiling hit (RFC 011 D-2), which must
    ///   never read as "your repository is damaged".
    ///
    /// Everything else goes through `failure` as before: a ceiling hit is
    /// [`PrikkError::CeilingExceeded`], a partial report is that too when a ceiling was set (and a plain
    /// [`PrikkError::Command`] otherwise), a timeout, garbage or another schema are errors. The verdict is
    /// decided from the **report on stdout**, never from stderr text.
    fn verify(&self) -> Result<model::VerifyReport> {
        const SCHEMA: &str = "verify-report-v1";
        let args = ["verify", "--format", "json"];
        let run = self.execute(&args)?;
        if run.timed_out {
            return Err(self.failure(&args, &run));
        }
        if run.status.success() {
            let stdout = String::from_utf8(run.stdout)
                .map_err(|e| PrikkError::Parse(format!("prikk stdout was not UTF-8: {e}")))?;
            return parse_schema_json(&stdout, SCHEMA);
        }
        if run.status.code() == Some(1) {
            let report = std::str::from_utf8(&run.stdout)
                .ok()
                .and_then(|out| parse_schema_json::<model::VerifyReport>(out, SCHEMA).ok())
                .filter(|r| !r.verdict.ok);
            match report {
                Some(r) if is_complete_verdict(&r) => return Ok(r),
                // A failing report that is not a complete verdict: a partial run. With a ceiling set it
                // is a ceiling hit by construction (the stage that did not finish ran out of memory or was
                // blocked behind it); without one it is a plain failure.
                Some(_) => {
                    return Err(match self.limits.data_ceiling {
                        Some(ceiling_bytes) => PrikkError::CeilingExceeded {
                            verb: "verify".to_owned(),
                            ceiling_bytes,
                        },
                        None => PrikkError::Command {
                            code: Some(1),
                            stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
                        },
                    });
                }
                None => {}
            }
        }
        Err(self.failure(&args, &run))
    }

    fn branches(&self, all: bool) -> Result<model::BranchList> {
        let mut args = vec!["branch", "--format", "json"];
        if all {
            args.push("--all");
        }
        self.run_json(&args, "branch-list-v1")
    }

    fn tags(&self) -> Result<model::TagList> {
        self.run_json(&["tag", "--format", "json"], "tag-list-v1")
    }

    fn status(&self) -> Result<model::StatusReport> {
        self.run_json(&["status", "--format", "json"], "status-report-v1")
    }

    fn worktree_status(&self) -> Result<model::WorktreeStatusReport> {
        self.run_json(
            &["worktree-status", "--format", "json"],
            "worktree-status-report-v1",
        )
    }

    fn patch_plan_content(
        &self,
        ref_name: Option<&str>,
        paths: &[&str],
    ) -> Result<model::PatchPlanContent> {
        let mut args = vec!["checkout", "--patch-plan", "--format", "json"];
        if let Some(r) = ref_name {
            args.extend(["--ref", r]);
        }
        for p in paths {
            args.extend(["--content-path", p]);
        }
        self.run_json(&args, "patch-plan-content-v1")
    }

    fn tree(&self, point: Option<&str>, prefix: Option<&str>) -> Result<model::TreeListing> {
        let mut args = vec!["tree", "--format", "json"];
        if let Some(p) = point {
            args.extend(["--ref", p]);
        }
        if let Some(pre) = prefix {
            args.extend(["--prefix", pre]);
        }
        self.run_json(&args, "tree-listing-v1")
    }

    fn path_content_meta(&self, point: Option<&str>, path: &str) -> Result<model::PathContentMeta> {
        let mut args = vec!["cat", "--path", path, "--format", "json"];
        if let Some(p) = point {
            args.extend(["--ref", p]);
        }
        self.run_json(&args, "path-content-v1")
    }

    fn cat_bytes(
        &self,
        point: Option<&str>,
        path: &str,
        max_bytes: Option<u64>,
    ) -> Result<Vec<u8>> {
        let max_s;
        let mut args = vec!["cat", "--path", path];
        if let Some(p) = point {
            args.extend(["--ref", p]);
        }
        if let Some(n) = max_bytes {
            max_s = n.to_string();
            args.extend(["--max-bytes", &max_s]);
        }
        self.run_bytes(&args)
    }

    fn key_status(&self) -> Result<model::KeyStatus> {
        self.run_json(&["key", "status", "--format", "json"], "key-status-v1")
    }

    fn trust_list(&self) -> Result<model::TrustList> {
        self.run_json(
            &["trust", "maintainer", "list", "--format", "json"],
            "trust-list-v1",
        )
    }

    /// `trust maintainer check`. prikk ≤ 0.46.0 answers "not trusted" with exit 0; **0.52.0 exits 1**
    /// (ledger PK-42 / IS-21) and still writes the `trust-check-v1` document. Exit 1 is therefore read
    /// as the answer `trusted: false` **only if** stdout is a well-formed `trust-check-v1` that itself
    /// says `trusted: false`; every other non-zero exit (usage `2`, an unreadable policy, a missing or
    /// contradictory document) stays a typed [`PrikkError::Command`] — exit status remains the signal
    /// for real failures.
    fn trust_check(&self, key_id: &str) -> Result<model::TrustCheck> {
        const SCHEMA: &str = "trust-check-v1";
        let args = [
            "trust",
            "maintainer",
            "check",
            "--key-id",
            key_id,
            "--format",
            "json",
        ];
        let run = self.execute(&args)?;
        if run.timed_out {
            return Err(self.failure(&args, &run));
        }
        if run.status.success() {
            let stdout = String::from_utf8(run.stdout)
                .map_err(|e| PrikkError::Parse(format!("prikk stdout was not UTF-8: {e}")))?;
            return parse_schema_json(&stdout, SCHEMA);
        }
        let code = run.status.code();
        if code == Some(1) {
            let parsed = std::str::from_utf8(&run.stdout)
                .ok()
                .and_then(|out| parse_schema_json::<model::TrustCheck>(out, SCHEMA).ok());
            if let Some(check) = parsed.filter(|c| !c.trusted) {
                return Ok(check);
            }
        }
        Err(self.failure(&args, &run))
    }

    fn bundle_export(&self, ref_name: &str, out: &Path) -> Result<()> {
        let out = out.to_string_lossy();
        self.run(&["bundle", "export", "--ref", ref_name, "--output", &out])
            .map(drop)
    }

    fn archive_export(&self, out: &Path) -> Result<()> {
        let out = out.to_string_lossy();
        self.run(&["archive", "export", &out]).map(drop)
    }

    fn archive_verify(&self, artifact: &Path) -> Result<model::ArchiveVerifyReport> {
        let a = artifact.to_string_lossy();
        // Its schema is the archive report's own (`prepo-verify-report-v1`), not a `--format json` of a repository.
        self.run_json(
            &["archive", "verify", &a, "--format", "json"],
            "prepo-verify-report-v1",
        )
    }

    fn archive_import(&self, artifact: &Path, adopt: bool) -> Result<model::ArchiveImportReport> {
        let a = artifact.to_string_lossy();
        let mut args = vec!["archive", "import", &*a];
        if adopt {
            args.push("--adopt");
        }
        args.extend(["--format", "json"]);
        self.run_json(&args, "prepo-import-report-v1")
    }

    fn trust_maintainer_add(&self, key_id: &str, public_key: &str) -> Result<()> {
        let key = model::CarriedKey {
            key_id: key_id.to_owned(),
            public_key: public_key.to_owned(),
            status: String::new(),
        };
        if !key.is_well_formed() {
            return Err(PrikkError::Parse(
                "a maintainer key id or public key is not in the shape prikk gives them".to_owned(),
            ));
        }
        self.run(&[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            key_id,
            "--public-key",
            public_key,
        ])
        .map(drop)
    }

    fn bundle_verify(&self, artifact: &Path) -> Result<()> {
        let a = artifact.to_string_lossy();
        self.run(&["bundle", "verify", "--input", &a]).map(drop)
    }

    fn bundle_import(&self, artifact: &Path) -> Result<()> {
        let a = artifact.to_string_lossy();
        self.run(&["bundle", "import", "--input", &a]).map(drop)
    }

    fn sync_summary(&self, out: &Path) -> Result<()> {
        let out = out.to_string_lossy();
        self.run(&["sync", "summary", "--output", &out]).map(drop)
    }

    fn sync_have(&self, ref_name: &str, out: &Path) -> Result<()> {
        let out = out.to_string_lossy();
        self.run(&["sync", "have", ref_name, "--output", &out])
            .map(drop)
    }

    fn sync_build(&self, ref_name: &str, have: &Path, out: &Path) -> Result<()> {
        let have = have.to_string_lossy();
        let out = out.to_string_lossy();
        self.run(&["sync", "build", ref_name, "--have", &have, "--output", &out])
            .map(drop)
    }

    fn sync_accept(&self, artifact: &Path) -> Result<Vec<String>> {
        let a = artifact.to_string_lossy();
        self.run_lines(&["sync", "accept", &a])
    }

    fn sync_pending(&self) -> Result<Vec<String>> {
        self.run_lines(&["sync", "pending"])
    }

    fn sync_accept_report(
        &self,
        artifact: &Path,
        claims_out: &Path,
    ) -> Result<crate::exchange::AcceptReport> {
        let a = artifact.to_string_lossy();
        let c = claims_out.to_string_lossy();
        let stdout = self.run(&["sync", "accept", &a, "--claims-out", &c])?;
        let report = crate::exchange::parse_accept_output(&stdout)?;
        let file = std::fs::read_to_string(claims_out).map_err(|e| {
            PrikkError::Parse(format!("unreadable `sync` output: the claims file: {e}"))
        })?;
        let mut from_file = crate::exchange::parse_claims_file(&file)?;
        let mut from_stdout: Vec<String> = report
            .claim_lines
            .iter()
            .map(|c| c.claim_id.clone())
            .collect();
        from_file.sort_unstable();
        from_stdout.sort_unstable();
        if from_file != from_stdout {
            return Err(PrikkError::Parse(
                "unreadable `sync` output: the claims file and the report name different claims"
                    .to_owned(),
            ));
        }
        Ok(report)
    }

    fn doctor_report(&self) -> Result<crate::maintenance::DoctorReport> {
        let run = self.execute(&["doctor"])?;
        if run.timed_out {
            return Err(self.failure(&["doctor"], &run));
        }
        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        match crate::maintenance::parse_doctor_output(&stdout) {
            Ok(report) => Ok(report),
            Err(parse) if run.status.success() => Err(parse),
            // A failed run whose output is not a report: the plain failure, with prikk's own text.
            Err(_) => Err(self.failure(&["doctor"], &run)),
        }
    }

    fn doctor_repair(&self, repair: crate::maintenance::Repair) -> Result<String> {
        self.run(&["doctor", repair.flag()])
    }

    fn unlock_list(&self) -> Result<Vec<crate::maintenance::LockInfo>> {
        let stdout = self.run(&["unlock"])?;
        crate::maintenance::parse_unlock_output(&stdout)
    }

    fn unlock_force(&self, lock_path: &str) -> Result<String> {
        self.run(&["unlock", "--lock", lock_path, "--force"])
    }

    fn sync_pending_patches(&self) -> Result<Vec<String>> {
        let stdout = self.run(&["sync", "pending"])?;
        crate::exchange::parse_pending_output(&stdout)
    }

    fn sync_seal(&self, ref_name: &str, claim: &str) -> Result<()> {
        self.run(&["sync", "seal", ref_name, "--claim", claim])
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_line() {
        assert_eq!(parse_version_line("prikk 0.43.0\n"), Some("0.43.0"));
        assert_eq!(parse_version_line("  prikk 1.2.3  "), Some("1.2.3"));
        assert_eq!(parse_version_line("git version 2.40"), None);
    }

    #[test]
    fn parses_semver_core_ignoring_suffix() {
        assert_eq!(parse_semver("0.43.0"), Some((0, 43, 0)));
        assert_eq!(parse_semver("1.2.3-dev.4"), Some((1, 2, 3)));
        assert_eq!(parse_semver("0.43.0+build7"), Some((0, 43, 0)));
        assert_eq!(parse_semver("0.43"), None);
        assert_eq!(parse_semver("nope"), None);
    }

    #[test]
    fn version_ordering_meets_the_floor() {
        assert!((0, 54, 0) >= MIN_PRIKK_VERSION);
        assert!((0, 54, 1) >= MIN_PRIKK_VERSION);
        assert!((1, 0, 0) >= MIN_PRIKK_VERSION);
        assert!((0, 53, 9) < MIN_PRIKK_VERSION);
        assert!((0, 53, 0) < MIN_PRIKK_VERSION);
        assert!((0, 52, 0) < MIN_PRIKK_VERSION);
    }
}
