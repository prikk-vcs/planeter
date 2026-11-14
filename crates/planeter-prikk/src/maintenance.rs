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

/// The code prikk 0.54 gives to an object that nothing refers to (`verify` reports the same as `unreached objects`). Since prikk 0.57.0
/// it is given only to a **Block or RefState**; unreached patches and blobs are one [`UNREACHED_CONTENT_CODE`] finding.
pub const UNREACHED_OBJECT_CODE: &str = "PRIKK-DOCTOR-UNREACHED-OBJECT";

/// The code prikk 0.57.0 gives to **all** unreached patches and blobs together: one finding at `info` level, "unreached content: N
/// object(s) (P patch(es), B blob(s)) that nothing reaches …", with the recommendation "nothing to do; the bytes stay until a command that
/// reclaims them exists". (Content left behind by a killed or failed accept, a received ref removed, a refused import.)
pub const UNREACHED_CONTENT_CODE: &str = "PRIKK-DOCTOR-UNREACHED-CONTENT";

/// Every `PRIKK-DOCTOR-UNREACHED-…` code this reader knows. **A code of that family that is not in this list makes the output unreadable**
/// (recovery stops and keeps the text), so a renamed or split finding is never silently counted as something else.
pub const KNOWN_UNREACHED_CODES: [&str; 2] = [UNREACHED_OBJECT_CODE, UNREACHED_CONTENT_CODE];

/// The counts in an [`UNREACHED_CONTENT_CODE`] finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnreachedContent {
    pub objects: u32,
    pub patches: u32,
    pub blobs: u32,
}

/// Read `unreached content: N object(s) (P patch(es), B blob(s)) …`; `None` unless it has exactly that start and `N = P + B`.
pub fn parse_unreached_content(message: &str) -> Option<UnreachedContent> {
    let rest = message.strip_prefix("unreached content: ")?;
    let (n, rest) = rest.split_once(" object(s) (")?;
    let (p, rest) = rest.split_once(" patch(es), ")?;
    let (b, _) = rest.split_once(" blob(s))")?;
    let (objects, patches, blobs) = (n.parse().ok()?, p.parse().ok()?, b.parse().ok()?);
    (objects == patches + blobs).then_some(UnreachedContent {
        objects,
        patches,
        blobs,
    })
}

impl Diagnostic {
    /// A **warning of exactly [`UNREACHED_OBJECT_CODE`] that names no repair** ("the bytes stay; no command removes them
    /// yet"): there is nothing to do and nothing to wait for, so recovery notes it and goes on. An error of that code, a
    /// differently coded warning, or one that names a repair is not this.
    pub fn is_unreached_note(&self) -> bool {
        self.severity == Severity::Warning
            && self.code == UNREACHED_OBJECT_CODE
            && self.named_flags().is_empty()
    }
}

impl DoctorReport {
    /// The unreached content prikk 0.57.0 reports (patches and blobs that nothing reaches), if it does.
    pub fn unreached_content(&self) -> Option<UnreachedContent> {
        self.diagnostics
            .iter()
            .filter(|d| d.code == UNREACHED_CONTENT_CODE)
            .find_map(|d| parse_unreached_content(&d.message))
    }

    /// How many diagnostics are [`Diagnostic::is_unreached_note`]s.
    pub fn unreached_notes(&self) -> u32 {
        self.diagnostics
            .iter()
            .filter(|d| d.is_unreached_note())
            .count() as u32
    }

    /// Whether recovery may go on: nothing but information and unreached-object notes. (A report with one of those beside any
    /// other problem is not clean for recovery.)
    pub fn is_clean_for_recovery(&self) -> bool {
        self.errors == 0
            && self.warnings == self.unreached_notes()
            && self
                .diagnostics
                .iter()
                .all(|d| d.severity == Severity::Info || d.is_unreached_note())
    }

    /// Nothing but information: no error, no warning. (Stale locks are not a diagnostic of `doctor`.)
    pub fn is_clean(&self) -> bool {
        self.errors == 0
            && self.warnings == 0
            && self
                .diagnostics
                .iter()
                .all(|d| d.severity == Severity::Info)
    }

    /// The flags (`--repair-…`, `--cancel-import`, …) that the diagnostics' recommendations name, in order of
    /// appearance and without repeats. `--plan-only` is a preview modifier, not a repair, and is not listed. This is
    /// **text for the plan**: what may be *run* is decided by [`DoctorReport::repair_plan`].
    pub fn named_repairs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        // Information names no repair: its text may mention a flag of another command (`verify --format json`).
        for d in self
            .diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Info)
        {
            for f in d.named_flags() {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
        out
    }

    /// What recovery may do about this report (RFC 004 part 4a, T1): nothing when it is clean; **the repairs it names,
    /// in order**, when *every* problem (every error and every warning) names at least one flag and **every flag it names
    /// is on the idempotent allow-list** ([`Repair`]); otherwise a refusal with the reason. One problem with no named
    /// repair, or naming any repair off the list (even beside an allowed one), makes the whole report unrepairable by
    /// planeter: a person decides.
    pub fn repair_plan(&self) -> RepairPlan {
        if self.is_clean_for_recovery() {
            return RepairPlan::Nothing;
        }
        let mut repairs: Vec<Repair> = Vec::new();
        for d in self
            .diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Info && !d.is_unreached_note())
        {
            let flags = d.named_flags();
            if flags.is_empty() {
                return RepairPlan::Refused(format!("[{}] names no repair", d.code));
            }
            for f in flags {
                match Repair::from_flag(&f) {
                    Some(r) => {
                        if !repairs.contains(&r) {
                            repairs.push(r);
                        }
                    }
                    None => {
                        return RepairPlan::Refused(format!(
                            "[{}] names {f}, which is not an idempotent repair planeter runs",
                            d.code
                        ));
                    }
                }
            }
        }
        RepairPlan::Allowed(repairs)
    }
}

/// What recovery may do about a `doctor` report ([`DoctorReport::repair_plan`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairPlan {
    /// Nothing to repair.
    Nothing,
    /// Every problem is covered by these allow-listed, idempotent repairs.
    Allowed(Vec<Repair>),
    /// planeter repairs nothing here; the reason.
    Refused(String),
}

impl Diagnostic {
    /// The `--flags` this diagnostic's message and recommendation name (without `--plan-only`), in order, without repeats.
    pub fn named_flags(&self) -> Vec<String> {
        let text = format!(
            "{} {}",
            self.message,
            self.recommendation.as_deref().unwrap_or("")
        );
        let mut out: Vec<String> = Vec::new();
        for tok in text.split(|c: char| {
            c.is_whitespace() || matches!(c, '`' | ',' | ';' | '(' | ')' | '\'' | '"')
        }) {
            let tok = tok.trim_end_matches('.');
            if tok.starts_with("--")
                && tok.len() > 2
                && tok != "--plan-only"
                && !out.iter().any(|t| t == tok)
            {
                out.push(tok.to_owned());
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
                // The unreached family is closed: a code of it that this reader does not know, or the content finding in another
                // shape than prikk 0.57.0 gives it (one finding, `info`, with the counts), is not guessed at.
                if code.starts_with("PRIKK-DOCTOR-UNREACHED-") {
                    if !KNOWN_UNREACHED_CODES.contains(&code) {
                        return Err(unreadable(
                            "an unreached-object code this reader does not know",
                        ));
                    }
                    if code == UNREACHED_CONTENT_CODE
                        && (severity != Severity::Info
                            || parse_unreached_content(message.trim()).is_none())
                    {
                        return Err(unreadable(
                            "the unreached-content finding is not an info line with its counts",
                        ));
                    }
                }
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
    /// prikk 0.57.0: when the holder started, as the platform counts it (kept as text, never used); `None` before 0.57.0.
    pub recorded_start: Option<String>,
    /// prikk's own words about whether the process is running: kept for the record, **never trusted**.
    pub liveness: String,
}

/// One detail line of a lock (`  name: value`), exactly the one named.
fn detail(lines: &mut std::iter::Peekable<std::str::Lines<'_>>, name: &str) -> Result<String> {
    let next = lines
        .next()
        .ok_or_else(|| unreadable("a lock without its detail lines"))?;
    next.strip_prefix("  ")
        .and_then(|l| l.strip_prefix(name))
        .and_then(|l| l.strip_prefix(": "))
        .map(|v| v.trim().to_owned())
        .ok_or_else(|| {
            unreadable(
                "a lock's detail lines are not kind, recorded pid, [recorded start,] liveness",
            )
        })
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
        let kind = detail(&mut lines, "kind")?;
        let pid = detail(&mut lines, "recorded pid")?;
        // prikk 0.57.0 records when the holder started (`recorded start: <number>`, in the platform's own unit) beside its pid, and
        // judges `running`, `gone` or `unknown` by it. Optional (0.54 to 0.56 print none); no other detail line is known.
        let start = match lines.peek() {
            Some(next) if next.starts_with("  recorded start: ") => {
                let value = lines
                    .next()
                    .and_then(|l| {
                        l.strip_prefix("  recorded start: ")
                            .map(|v| v.trim().to_owned())
                    })
                    .unwrap_or_default();
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(unreadable("a recorded start is not a number"));
                }
                Some(value)
            }
            _ => None,
        };
        let liveness = detail(&mut lines, "liveness")?;
        out.push(LockInfo {
            path: line.to_owned(),
            kind,
            recorded_pid: pid
                .parse()
                .map_err(|_| unreadable("a recorded pid is not a number"))?,
            recorded_start: start,
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
