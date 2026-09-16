//! [`CliPrikkRepo`] — the subprocess implementation of [`PrikkRepo`] (RFC 001 D-2/D-3).
//!
//! It shells out to the prikk binary against one repository directory (the one holding `.prikk`),
//! parses `--format json` output into the [`crate::model`] types after checking the `schema_version`,
//! and maps a non-zero exit to a typed [`PrikkError`] (never a panic). The version pin (D-3) refuses a
//! prikk older than planeter's floor (PK-22: transport requires prikk ≥ 0.43.0). Confinement of the
//! subprocess (sandbox) is layered on in T4 — this module invokes prikk directly.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::error::{PrikkError, Result};
use crate::model;
use crate::sandbox::Sandbox;

/// planeter's minimum supported prikk version (RFC 001 D-3 / PK-22).
pub const MIN_PRIKK_VERSION: (u64, u64, u64) = (0, 43, 0);

/// A prikk repository driven through the prikk CLI as a subprocess.
#[derive(Debug, Clone)]
pub struct CliPrikkRepo {
    /// The working root that contains the `.prikk` directory.
    root: PathBuf,
    /// The prikk binary to invoke (default: `prikk` on `PATH`).
    binary: OsString,
    /// How the prikk subprocess is confined (default: bubblewrap — RFC 001 T4).
    sandbox: Sandbox,
}

impl CliPrikkRepo {
    /// Bind to the repository at `root` (the directory containing `.prikk`), using `prikk` from `PATH`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            binary: OsString::from("prikk"),
            sandbox: Sandbox::default(),
        }
    }

    /// Bind with an explicit prikk binary path.
    pub fn with_binary(root: impl Into<PathBuf>, binary: impl Into<OsString>) -> Self {
        Self {
            root: root.into(),
            binary: binary.into(),
            sandbox: Sandbox::default(),
        }
    }

    /// Set the confinement policy (default: bubblewrap). Use [`Sandbox::Unconfined`] only in dev, never
    /// to host untrusted input (RFC 001 T4).
    #[must_use]
    pub fn with_sandbox(mut self, sandbox: Sandbox) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// Bind and **verify the prikk version is in planeter's supported range** (RFC 001 D-3): refuse an
    /// out-of-range binary rather than guess.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let repo = Self::new(root);
        repo.check_version()?;
        Ok(repo)
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

    /// Run a prikk subcommand, returning its stdout. A non-zero exit is a typed [`PrikkError::Command`].
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = self
            .sandbox
            .command(&self.binary, &self.root, args)
            .output()
            .map_err(PrikkError::Spawn)?;
        if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|e| PrikkError::Parse(format!("prikk stdout was not UTF-8: {e}")))
        } else {
            Err(PrikkError::Command {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }

    /// Run a JSON-emitting read command: parse stdout, **check `schema_version`**, deserialize to `T`.
    fn run_json<T: DeserializeOwned>(&self, args: &[&str], expected_schema: &str) -> Result<T> {
        let stdout = self.run(args)?;
        let value: serde_json::Value = serde_json::from_str(&stdout)
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

    fn verify(&self) -> Result<model::VerifyReport> {
        self.run_json(&["verify", "--format", "json"], "verify-report-v1")
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

    fn key_status(&self) -> Result<model::KeyStatus> {
        self.run_json(&["key", "status", "--format", "json"], "key-status-v1")
    }

    fn trust_list(&self) -> Result<model::TrustList> {
        self.run_json(
            &["trust", "maintainer", "list", "--format", "json"],
            "trust-list-v1",
        )
    }

    fn trust_check(&self, key_id: &str) -> Result<model::TrustCheck> {
        self.run_json(
            &[
                "trust",
                "maintainer",
                "check",
                "--key-id",
                key_id,
                "--format",
                "json",
            ],
            "trust-check-v1",
        )
    }

    fn bundle_export(&self, ref_name: &str, out: &Path) -> Result<()> {
        let out = out.to_string_lossy();
        self.run(&["bundle", "export", "--ref", ref_name, "--output", &out])
            .map(drop)
    }

    fn bundle_import(&self, artifact: &Path) -> Result<()> {
        let a = artifact.to_string_lossy();
        self.run(&["bundle", "import", &a]).map(drop)
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
        assert!((0, 43, 0) >= MIN_PRIKK_VERSION);
        assert!((0, 43, 1) >= MIN_PRIKK_VERSION);
        assert!((1, 0, 0) >= MIN_PRIKK_VERSION);
        assert!((0, 42, 9) < MIN_PRIKK_VERSION);
    }
}
