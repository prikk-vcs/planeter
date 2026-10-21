//! The text prikk prints for `doctor` and `unlock`, read into typed values (RFC 004 part 4, D-6).
//!
//! Recovery removes locks of the storage engine, so what it reads must be exactly what it thinks it is. Neither verb has
//! a JSON form (prikk 0.53.0), and both readers are strict: an output of a shape they do not know is an **error that
//! keeps the text**, and the caller stops and changes nothing.
//!
//! * `prikk doctor` prints `doctor repository: <path>`, many `name: value` lines, the diagnostics
//!   (`error|warning|info [CODE]: message`, each optionally followed by an indented `recommendation: …`) and, last,
//!   `issue summary: errors=E, warnings=W, info=I`. The diagnostics must add up to the summary.
//! * `prikk unlock` prints either `no locks currently held`, or for each lock an absolute path line followed by
//!   indented `kind:`, `recorded pid:` and `liveness:` lines, and then an advisory paragraph. **The liveness line is
//!   read and kept but never trusted** (inside our PID namespace it is wrong, U-16).

use crate::error::{PrikkError, Result};

fn unreadable(what: &str) -> PrikkError {
    PrikkError::Parse(format!("unreadable `doctor`/`unlock` output: {what}"))
}

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One diagnostic of `doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// `PRIKK-DOCTOR-VERIFY-OK`, …
    pub code: String,
    pub message: String,
    /// The indented `recommendation:` line(s) that followed, verbatim.
    pub recommendation: Option<String>,
}

/// What `doctor` reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub errors: u32,
    pub warnings: u32,
    pub info: u32,
    pub diagnostics: Vec<Diagnostic>,
}

impl DoctorReport {
    /// Nothing but information: no error, no warning. (Stale locks are not a diagnostic of `doctor`.)
    pub fn is_clean(&self) -> bool {
        self.errors == 0
            && self.warnings == 0
            && self
                .diagnostics
                .iter()
                .all(|d| d.severity == Severity::Info)
    }

    /// The repair flags (`--repair-…`, `--cancel-import`, …) that the diagnostics' recommendations name, in order of
    /// appearance and without repeats. Only these may ever be run, and only if the caller also allows them.
    pub fn named_repairs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for d in &self.diagnostics {
            let text = format!(
                "{} {}",
                d.message,
                d.recommendation.as_deref().unwrap_or("")
            );
            for tok in text.split(|c: char| {
                c.is_whitespace() || matches!(c, '`' | ',' | ';' | '(' | ')' | '\'' | '"')
            }) {
                if tok.starts_with("--") && tok.len() > 2 && !out.iter().any(|t| t == tok) {
                    out.push(tok.trim_end_matches('.').to_owned());
                }
            }
        }
        out
    }
}

/// Read the stdout of `prikk doctor`.
pub fn parse_doctor_output(stdout: &str) -> Result<DoctorReport> {
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty()).peekable();
    match lines.next() {
        Some(first) if first.starts_with("doctor repository: ") => {}
        _ => return Err(unreadable("the first line is not `doctor repository: …`")),
    }
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut summary: Option<(u32, u32, u32)> = None;
    for line in lines {
        if let Some(rec) = line.trim_start().strip_prefix("recommendation:") {
            if !line.starts_with(' ') {
                continue;
            }
            match diagnostics.last_mut() {
                Some(d) => {
                    let text = rec.trim();
                    d.recommendation = Some(match d.recommendation.take() {
                        Some(prev) => format!("{prev} {text}"),
                        None => text.to_owned(),
                    });
                }
                None => return Err(unreadable("a recommendation with no diagnostic")),
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("issue summary: ") {
            let mut e = None;
            let mut w = None;
            let mut i = None;
            for part in rest.split(", ") {
                let (k, v) = part
                    .split_once('=')
                    .ok_or_else(|| unreadable("the issue summary has a part without `=`"))?;
                let v: u32 = v
                    .trim()
                    .parse()
                    .map_err(|_| unreadable("a count in the issue summary is not a number"))?;
                match k {
                    "errors" => e = Some(v),
                    "warnings" => w = Some(v),
                    "info" => i = Some(v),
                    _ => return Err(unreadable("the issue summary names something else")),
                }
            }
            summary = Some((
                e.ok_or_else(|| unreadable("no errors count"))?,
                w.ok_or_else(|| unreadable("no warnings count"))?,
                i.ok_or_else(|| unreadable("no info count"))?,
            ));
            continue;
        }
        for (prefix, severity) in [
            ("error [", Severity::Error),
            ("warning [", Severity::Warning),
            ("info [", Severity::Info),
        ] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let (code, message) = rest
                    .split_once("]: ")
                    .ok_or_else(|| unreadable("a diagnostic has no `]: `"))?;
                diagnostics.push(Diagnostic {
                    severity,
                    code: code.to_owned(),
                    message: message.trim().to_owned(),
                    recommendation: None,
                });
                break;
            }
        }
    }
    let (errors, warnings, info) = summary.ok_or_else(|| unreadable("no issue summary"))?;
    let count = |s: Severity| diagnostics.iter().filter(|d| d.severity == s).count() as u32;
    if (
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Info),
    ) != (errors, warnings, info)
    {
        return Err(unreadable(
            "the diagnostics do not add up to the issue summary",
        ));
    }
    Ok(DoctorReport {
        errors,
        warnings,
        info,
        diagnostics,
    })
}

/// One lock `unlock` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockInfo {
    pub path: String,
    /// `active`, `container:object-store`, …
    pub kind: String,
    pub recorded_pid: u32,
    /// prikk's own words about whether the process is running: kept for the record, **never trusted**.
    pub liveness: String,
}

/// Read the stdout of `prikk unlock` (the listing; it clears nothing).
pub fn parse_unlock_output(stdout: &str) -> Result<Vec<LockInfo>> {
    let mut out: Vec<LockInfo> = Vec::new();
    let mut lines = stdout.lines().peekable();
    let mut saw_anything = false;
    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        saw_anything = true;
        if line.trim() == "no locks currently held" {
            if !out.is_empty() {
                return Err(unreadable("locks and `no locks currently held` together"));
            }
            continue;
        }
        if line.starts_with("liveness is advisory only") {
            // The advisory paragraph closes the listing; nothing may follow it but blank lines.
            if lines.any(|l| !l.trim().is_empty()) {
                return Err(unreadable("text after the advisory paragraph"));
            }
            break;
        }
        if !line.starts_with('/') || line.starts_with(' ') || line.contains(' ') {
            return Err(unreadable(
                "a line that is neither a lock path nor the known text",
            ));
        }
        let mut field = |name: &str| -> Result<String> {
            let next = lines
                .next()
                .ok_or_else(|| unreadable("a lock without its detail lines"))?;
            next.strip_prefix("  ")
                .and_then(|l| l.strip_prefix(name))
                .and_then(|l| l.strip_prefix(": "))
                .map(|v| v.trim().to_owned())
                .ok_or_else(|| {
                    unreadable("a lock's detail lines are not kind, recorded pid, liveness")
                })
        };
        let kind = field("kind")?;
        let pid = field("recorded pid")?;
        let liveness = field("liveness")?;
        out.push(LockInfo {
            path: line.to_owned(),
            kind,
            recorded_pid: pid
                .parse()
                .map_err(|_| unreadable("a recorded pid is not a number"))?,
            liveness,
        });
    }
    if !saw_anything {
        return Err(unreadable("empty unlock output"));
    }
    Ok(out)
}

/// The repairs recovery may run (RFC 004 part 4, T1 step 5): the idempotent tail and index repairs, and only when
/// `doctor` names them. Everything else `doctor` can do (discarding commits, restoring a queue target, cancelling an
/// import, rebuilding a pointer index, `--repair-main-ref`) loses or invents data, and is for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    WalTail,
    Index,
    PointerIndexTail,
    Tails,
}

impl Repair {
    pub const ALL: [Repair; 4] = [
        Repair::WalTail,
        Repair::Index,
        Repair::PointerIndexTail,
        Repair::Tails,
    ];

    /// The flag, as `prikk doctor` spells it.
    pub fn flag(self) -> &'static str {
        match self {
            Repair::WalTail => "--repair-wal-tail",
            Repair::Index => "--repair-index",
            Repair::PointerIndexTail => "--repair-pointer-index-tail",
            Repair::Tails => "--repair-tails",
        }
    }

    pub fn from_flag(flag: &str) -> Option<Repair> {
        Repair::ALL.into_iter().find(|r| r.flag() == flag)
    }
}
