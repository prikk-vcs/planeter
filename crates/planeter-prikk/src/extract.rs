//! The `extract` driver (RFC 005 D-4, prikk 0.54.0; dependency-ledger PK-56, PK-65, PK-66, PK-70): the files of one point of
//! history, written into a **new directory outside the repository**. This is the driver only: no page, no API route, no
//! catalog text (the review UI is RFC 005 part 6).
//!
//! # What is ours to make safe
//!
//! * **The destination.** prikk 0.54.x checks only that the destination does not exist yet: a path *inside* the repository
//!   (even under `.prikk/`) is accepted, and a copy left in the worktree would be taken as content by the next
//!   `commit --from-worktree` (0.55.0 refuses it). So the driver never takes a destination from a caller: it makes one
//!   itself, a fresh, not-yet-existing path in a per-call working directory ([`ExtractWorkdir`]: `<parent>/work/<id>/out/tree`),
//!   mounts **only** `out` read-write in the sandbox, refuses (before any prikk call) a working directory that resolves
//!   inside the repository root, through a symlink or not, and removes the whole directory when the guard is dropped.
//! * **The extracted tree is untrusted data.** File names and contents come from whoever pushed the history; a symlink in the
//!   tree may point anywhere. The driver returns the path and the report and **never reads, walks or follows anything in the
//!   tree**. Any later reader (RFC 005 part 6) must treat every name and every byte as hostile, and must not follow links.
//!
//! # What the readers refuse
//!
//! `extract-report-v1` and `refusal-v1` are read strictly: a missing or mistyped field, or another `schema_version`, is a
//! [`PrikkError::Parse`]. Unknown *fields* are tolerated (a later prikk may add some). A refusal id or a `reason` this driver
//! does not know is kept as it came in an `Other(..)` case and **logged**, never mapped to a known one.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

use crate::cli::{CliPrikkRepo, parse_schema_json};
use crate::error::PrikkError;

/// How many refusal ids or reasons this driver did not know have been read since the process started (each also wrote a
/// log line). A test, and an operator's metric one day, can see that the catch-all was used.
static UNKNOWN_SEEN: AtomicU64 = AtomicU64::new(0);

/// The number of refusal ids / reasons read that this driver does not know (see [`RefusalId::Other`]).
pub fn unknown_refusals_seen() -> u64 {
    UNKNOWN_SEEN.load(Ordering::Relaxed)
}

/// The default bound on the total bytes of a point to extract (64 MiB), in one place.
pub const DEFAULT_MAX_BYTES: u64 = 64 << 20;
/// The default bound on the number of files of a point to extract, in one place.
pub const DEFAULT_MAX_FILES: u64 = 10_000;

/// Which point of history to extract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractPoint {
    /// A ref (`heads/main`, `remotes/heads/main`).
    Ref(String),
    /// A block id (prikk takes it through `--ref`).
    Block(String),
    /// A pending recognition claim (`--claim`): the claim's parent's files plus the claim's patches.
    Claim(String),
}

impl ExtractPoint {
    fn args(&self) -> [&str; 2] {
        match self {
            ExtractPoint::Ref(r) => ["--ref", r],
            ExtractPoint::Block(b) => ["--ref", b],
            ExtractPoint::Claim(c) => ["--claim", c],
        }
    }
}

/// What to extract and how far to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOptions {
    /// `--plan-only`: report the point's size and write nothing (prikk says it costs the same as a write).
    pub plan_only: bool,
    /// `--max-bytes`: refuse the whole point, before the first write, if it holds more content than this.
    pub max_bytes: Option<u64>,
    /// `--max-files`: refuse the whole point, before the first write, if it has more files than this.
    pub max_files: Option<u64>,
    /// `--allow-full-replay`: with no admitted checkpoint in reach, replay from the start (the result is `provisional`).
    pub allow_full_replay: bool,
}

impl Default for ExtractOptions {
    /// Bounded: [`DEFAULT_MAX_BYTES`] and [`DEFAULT_MAX_FILES`]; a write; no full replay.
    fn default() -> Self {
        ExtractOptions {
            plan_only: false,
            max_bytes: Some(DEFAULT_MAX_BYTES),
            max_files: Some(DEFAULT_MAX_FILES),
            allow_full_replay: false,
        }
    }
}

/// How a point is described in `extract-report-v1` (`point_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PointKind {
    Ref,
    Block,
    Claim,
}

/// A field that may be `null` but must be there: serde would otherwise read an absent `Option` as `None`, and a document
/// that lacks the field is not the document this driver was written for.
fn required_nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

/// `extract-report-v1`: what a successful `extract` (or `--plan-only`) says.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExtractReport {
    pub extract_repository: String,
    pub point: String,
    pub point_kind: PointKind,
    pub target_block_id: String,
    /// The block a claim names as its parent; `None` unless the point is a claim. (The "declared base disagrees" refusal is
    /// RFC 005 part 6, not built here.)
    #[serde(deserialize_with = "required_nullable")]
    pub claim_parent_block_id: Option<String>,
    /// No admitted checkpoint was in reach: the files match the history replayed, but nothing has checked that history.
    pub provisional: bool,
    /// Whether this call wrote the tree (`false` for `--plan-only`).
    pub written: bool,
    pub file_count: u64,
    pub total_bytes: u64,
    /// The largest single file: prikk's memory follows it (about twice, PK-56), so it is what admission needs.
    pub largest_file_bytes: u64,
}

/// `refusal-v1`'s fifteen ids (prikk 0.55.0, `extract.rs` `ExtractRefusalId`: the thirteen of 0.54.1 and the two 0.55.0 added), and
/// a named catch-all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusalId {
    OverMaxBytes,
    OverMaxFiles,
    DestinationExists,
    TemporarySiblingExists,
    PointNotFound,
    ClaimNotFound,
    NotAClaim,
    ClaimHasSeveralParents,
    CaseCollision,
    NoCheckpointInReach,
    DamagedRepository,
    IoFailure,
    NotARepository,
    /// 0.55.0: a claim whose parent block is still pending here (accepted, not yet sealed). 0.54.x said `damaged-repository`.
    ClaimParentNotSealed,
    /// 0.55.0: the destination lies inside the repository. The driver never passes such a destination (see [`ExtractError`]).
    DestinationInsideRepository,
    /// An id this driver does not know (a later prikk). Kept as it came; a log line says so when it is read.
    Other(String),
}

impl RefusalId {
    /// prikk's id for it.
    pub fn id(&self) -> &str {
        match self {
            RefusalId::OverMaxBytes => "over-max-bytes",
            RefusalId::OverMaxFiles => "over-max-files",
            RefusalId::DestinationExists => "destination-exists",
            RefusalId::TemporarySiblingExists => "temporary-sibling-exists",
            RefusalId::PointNotFound => "point-not-found",
            RefusalId::ClaimNotFound => "claim-not-found",
            RefusalId::NotAClaim => "not-a-claim",
            RefusalId::ClaimHasSeveralParents => "claim-has-several-parents",
            RefusalId::CaseCollision => "case-collision",
            RefusalId::NoCheckpointInReach => "no-checkpoint-in-reach",
            RefusalId::DamagedRepository => "damaged-repository",
            RefusalId::IoFailure => "io-failure",
            RefusalId::NotARepository => "not-a-repository",
            RefusalId::ClaimParentNotSealed => "claim-parent-not-sealed",
            RefusalId::DestinationInsideRepository => "destination-inside-repository",
            RefusalId::Other(s) => s,
        }
    }

    /// The fifteen ids this driver knows, in prikk's order.
    pub const KNOWN: [&'static str; 15] = [
        "over-max-bytes",
        "over-max-files",
        "destination-exists",
        "temporary-sibling-exists",
        "point-not-found",
        "claim-not-found",
        "not-a-claim",
        "claim-has-several-parents",
        "case-collision",
        "no-checkpoint-in-reach",
        "damaged-repository",
        "io-failure",
        "not-a-repository",
        "claim-parent-not-sealed",
        "destination-inside-repository",
    ];

    fn from_id(s: &str) -> RefusalId {
        match s {
            "over-max-bytes" => RefusalId::OverMaxBytes,
            "over-max-files" => RefusalId::OverMaxFiles,
            "destination-exists" => RefusalId::DestinationExists,
            "temporary-sibling-exists" => RefusalId::TemporarySiblingExists,
            "point-not-found" => RefusalId::PointNotFound,
            "claim-not-found" => RefusalId::ClaimNotFound,
            "not-a-claim" => RefusalId::NotAClaim,
            "claim-has-several-parents" => RefusalId::ClaimHasSeveralParents,
            "case-collision" => RefusalId::CaseCollision,
            "no-checkpoint-in-reach" => RefusalId::NoCheckpointInReach,
            "damaged-repository" => RefusalId::DamagedRepository,
            "io-failure" => RefusalId::IoFailure,
            "not-a-repository" => RefusalId::NotARepository,
            "claim-parent-not-sealed" => RefusalId::ClaimParentNotSealed,
            "destination-inside-repository" => RefusalId::DestinationInsideRepository,
            other => {
                UNKNOWN_SEEN.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "planeter: prikk gave an extract refusal id this version does not know ({other:?}); kept as it came"
                );
                RefusalId::Other(other.to_owned())
            }
        }
    }
}

impl<'de> Deserialize<'de> for RefusalId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(RefusalId::from_id(&s))
    }
}

/// Why no checkpoint was in reach (`reason` of `no-checkpoint-in-reach`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoCheckpointReason {
    /// A recorded checkpoint is in reach but its signer is not adopted here: adopt the key.
    SignerNotAdopted,
    /// A block with a snapshot is in reach but this repository never recorded it: run `verify`.
    NotRecorded,
    /// No block within 63 blocks carries a snapshot at all: `verify` cannot help; `--allow-full-replay` reads it.
    NoCheckpoint,
    /// A value this driver does not know. Kept as it came; logged.
    Other(String),
}

impl<'de> Deserialize<'de> for NoCheckpointReason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "signer-not-adopted" => NoCheckpointReason::SignerNotAdopted,
            "not-recorded" => NoCheckpointReason::NotRecorded,
            "no-checkpoint" => NoCheckpointReason::NoCheckpoint,
            other => {
                UNKNOWN_SEEN.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "planeter: prikk gave an extract refusal reason this version does not know ({other:?}); kept as it came"
                );
                NoCheckpointReason::Other(other.to_owned())
            }
        })
    }
}

/// `refusal-v1`: prikk's refusal of an `extract`, on stdout (the message is also on stderr), exit status 1.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Refusal {
    pub command: String,
    pub refusal: RefusalId,
    /// Set only for `no-checkpoint-in-reach`.
    #[serde(deserialize_with = "required_nullable")]
    pub reason: Option<NoCheckpointReason>,
    /// prikk's own English text, verbatim. Not user-facing: the review UI says it in its own words.
    pub message: String,
}

/// Why an extraction did not give a tree.
#[derive(Debug)]
pub enum ExtractError {
    /// prikk refused the point, by name. Nothing was written.
    Refused(Refusal),
    /// A claim point and `damaged-repository`. In prikk 0.54.x that id is **also** what prikk says for a claim whose parent
    /// is still pending (accepted, not yet sealed here): the repository is not damaged then (letter of 2026-10-24 §3,
    /// ledger PK-70). The driver does not tell the two apart and does not say "damaged". 0.55.0 and later report
    /// `claim-parent-not-sealed` ([`ExtractError::ClaimParentNotSealed`]); the floor is 0.54.0, so this stays.
    ClaimParentNotSealedOrDamaged(Refusal),
    /// prikk 0.55.0 and later: the claim's parent block is still pending here (accepted, not yet sealed), said by its own id
    /// `claim-parent-not-sealed`; the repository is not damaged. `refusal-v1` carries no field for the claim or its parent (only
    /// `command`, `refusal`, `reason`, `message`), so the two ids are in prikk's text, kept **verbatim** in `message` and never
    /// parsed; the claim is the one the caller asked for.
    ClaimParentNotSealed(Refusal),
    /// prikk 0.55.0 and later refused the destination as lying inside the repository (`destination-inside-repository`). The
    /// driver only ever names a destination in its own working directory outside every repository, so this cannot be reached:
    /// if it is, it is a **bug of this driver** (or of the directory it was given), not a state of the repository.
    DestinationInsideRepository(Refusal),
    /// The working directory or the destination was not made safe: refused before any prikk call.
    UnsafeDestination(String),
    /// Anything else: a crash, a timeout, a memory ceiling, an unreadable output (as for every other verb).
    Prikk(PrikkError),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::Refused(r) => write!(
                f,
                "prikk refused the extraction ({}): {}",
                r.refusal.id(),
                r.message
            ),
            ExtractError::ClaimParentNotSealedOrDamaged(r) => write!(
                f,
                "prikk could not read the claim's parent: either the parent is still pending here (not sealed yet) or the repository is damaged; prikk 0.54 gives both the same refusal ({})",
                r.message
            ),
            ExtractError::ClaimParentNotSealed(r) => write!(
                f,
                "the claim's parent is still pending here (accepted, not sealed yet): {}",
                r.message
            ),
            ExtractError::DestinationInsideRepository(r) => write!(
                f,
                "driver bug: prikk refused the extraction destination as lying inside the repository ({})",
                r.message
            ),
            ExtractError::UnsafeDestination(why) => {
                write!(f, "the extraction destination is not safe: {why}")
            }
            ExtractError::Prikk(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ExtractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExtractError::Prikk(e) => Some(e),
            _ => None,
        }
    }
}

impl From<PrikkError> for ExtractError {
    fn from(e: PrikkError) -> Self {
        ExtractError::Prikk(e)
    }
}

/// The result of a successful extraction: the report, and where the tree is (`None` for `--plan-only`).
///
/// **The tree is untrusted data** (see the module documentation): nothing here reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub report: ExtractReport,
    pub tree: Option<PathBuf>,
}

/// A per-call working directory for one extraction, outside every repository: `<parent>/work/<id>/`, with `out/` (the one
/// directory the sandbox mounts read-write) and, inside it, `tree` — the destination, which does not exist until prikk makes
/// it. Dropping the guard removes the whole directory with everything in it.
#[derive(Debug)]
pub struct ExtractWorkdir {
    dir: PathBuf,
    out: PathBuf,
    tree: PathBuf,
}

impl ExtractWorkdir {
    /// Make a working directory under `parent` (an existing directory) for an extraction from the repository at
    /// `repo_root`. Refused, with nothing created, if `parent` is not absolute, if it resolves (symlinks followed) to the
    /// repository root, inside it, or to a directory that contains it.
    pub fn create(parent: &Path, repo_root: &Path) -> Result<Self, ExtractError> {
        let unsafe_ = |why: String| ExtractError::UnsafeDestination(why);
        if !parent.is_absolute() {
            return Err(unsafe_(format!(
                "the working area {} is not an absolute path",
                parent.display()
            )));
        }
        let parent_real = parent.canonicalize().map_err(|e| {
            unsafe_(format!(
                "the working area {} cannot be resolved: {e}",
                parent.display()
            ))
        })?;
        let root_real = repo_root.canonicalize().map_err(|e| {
            unsafe_(format!(
                "the repository {} cannot be resolved: {e}",
                repo_root.display()
            ))
        })?;
        Self::check_outside(&parent_real, &root_real)?;
        let work = parent_real.join("work");
        match std::fs::create_dir(&work) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(unsafe_(format!("cannot make {}: {e}", work.display()))),
        }
        // `work` itself must be a real directory under `parent`, not a link someone put there.
        let work_real = work
            .canonicalize()
            .map_err(|e| unsafe_(format!("cannot resolve {}: {e}", work.display())))?;
        if work_real != work {
            return Err(unsafe_(format!(
                "{} is a link to {}",
                work.display(),
                work_real.display()
            )));
        }
        Self::check_outside(&work_real, &root_real)?;
        let dir = work_real.join(fresh_id());
        std::fs::create_dir(&dir)
            .map_err(|e| unsafe_(format!("cannot make {}: {e}", dir.display())))?;
        let out = dir.join("out");
        if let Err(e) = std::fs::create_dir(&out) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(unsafe_(format!("cannot make {}: {e}", out.display())));
        }
        let tree = out.join("tree");
        Ok(ExtractWorkdir { dir, out, tree })
    }

    fn check_outside(candidate: &Path, root_real: &Path) -> Result<(), ExtractError> {
        if candidate.starts_with(root_real) {
            return Err(ExtractError::UnsafeDestination(format!(
                "{} is the repository {} or inside it",
                candidate.display(),
                root_real.display()
            )));
        }
        if root_real.starts_with(candidate) {
            return Err(ExtractError::UnsafeDestination(format!(
                "{} contains the repository {}",
                candidate.display(),
                root_real.display()
            )));
        }
        Ok(())
    }

    /// The directory the sandbox mounts read-write (`<dir>/out`).
    pub fn out_dir(&self) -> &Path {
        &self.out
    }

    /// The destination handed to prikk: `<dir>/out/tree`. It does not exist until prikk writes it.
    pub fn destination(&self) -> &Path {
        &self.tree
    }
}

impl Drop for ExtractWorkdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fresh identifier for a working directory: 24 hexadecimal characters from the operating system's randomness (with the
/// process id, a clock reading and a counter mixed in if that cannot be read; a collision is refused by `create_dir`).
fn fresh_id() -> String {
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0u8; 12];
    let got = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !got {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = (u128::from(std::process::id()) << 64) ^ t ^ u128::from(n);
        bytes.copy_from_slice(&mix.to_le_bytes()[..12]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl CliPrikkRepo {
    /// `extract`: write the files of `point` into `workdir`'s destination, or (with `plan_only`) report the point's size and
    /// write nothing. Lease class **R** (it takes no lock and never writes into the repository).
    ///
    /// Before any prikk call the driver checks, against **this** repository's root, that the destination and its mounted
    /// directory resolve outside it ([`ExtractError::UnsafeDestination`]). The sandbox mounts `workdir.out_dir()` read-write
    /// and nothing else beyond the repository.
    pub fn extract(
        &self,
        point: &ExtractPoint,
        workdir: &ExtractWorkdir,
        options: &ExtractOptions,
    ) -> Result<Extraction, ExtractError> {
        // The guard is re-checked here, against the repository this driver is bound to: a working directory made for
        // another repository may lie inside this one.
        let root_real = self.root().canonicalize().map_err(|e| {
            ExtractError::UnsafeDestination(format!("the repository cannot be resolved: {e}"))
        })?;
        let out_real = workdir.out_dir().canonicalize().map_err(|e| {
            ExtractError::UnsafeDestination(format!(
                "the working directory cannot be resolved: {e}"
            ))
        })?;
        ExtractWorkdir::check_outside(&out_real, &root_real)?;
        if out_real.join("tree") != workdir.destination()
            || workdir.destination().symlink_metadata().is_ok()
        {
            return Err(ExtractError::UnsafeDestination(
                "the destination is not a fresh path in the working directory".to_owned(),
            ));
        }

        let destination = workdir.destination().to_string_lossy().into_owned();
        let max_bytes = options.max_bytes.map(|n| n.to_string());
        let max_files = options.max_files.map(|n| n.to_string());
        let mut args: Vec<&str> = vec!["extract"];
        args.extend(point.args());
        args.push(&destination);
        args.extend(["--format", "json"]);
        if options.plan_only {
            args.push("--plan-only");
        }
        if options.allow_full_replay {
            args.push("--allow-full-replay");
        }
        if let Some(n) = &max_bytes {
            args.extend(["--max-bytes", n]);
        }
        if let Some(n) = &max_files {
            args.extend(["--max-files", n]);
        }

        let run_repo = self.sandboxed_as(self.sandbox().with_out_mount(workdir.out_dir()));
        let run = run_repo.execute(&args).map_err(ExtractError::Prikk)?;
        if run.timed_out {
            return Err(ExtractError::Prikk(run_repo.failure(&args, &run)));
        }
        if run.status.success() {
            let stdout = String::from_utf8(run.stdout).map_err(|e| {
                ExtractError::Prikk(PrikkError::Parse(format!(
                    "prikk stdout was not UTF-8: {e}"
                )))
            })?;
            let report: ExtractReport =
                parse_schema_json(&stdout, "extract-report-v1").map_err(ExtractError::Prikk)?;
            check_report(&report, point, options)?;
            let tree = (report.written).then(|| workdir.destination().to_path_buf());
            return Ok(Extraction { report, tree });
        }
        if run.status.code() == Some(1) {
            // A refusal is a document on stdout even though the exit status is 1. Anything else that exits 1 (a crash
            // that printed nothing parseable, a ceiling hit) is the usual typed failure.
            if let Some(refusal) = std::str::from_utf8(&run.stdout)
                .ok()
                .and_then(|out| parse_refusal(out).ok())
            {
                if refusal.command != "extract" {
                    return Err(ExtractError::Prikk(PrikkError::Parse(format!(
                        "a refusal for the command {:?}, not extract",
                        refusal.command
                    ))));
                }
                return Err(match (point, &refusal.refusal) {
                    (ExtractPoint::Claim(_), RefusalId::DamagedRepository) => {
                        ExtractError::ClaimParentNotSealedOrDamaged(refusal)
                    }
                    (_, RefusalId::ClaimParentNotSealed) => {
                        ExtractError::ClaimParentNotSealed(refusal)
                    }
                    (_, RefusalId::DestinationInsideRepository) => {
                        ExtractError::DestinationInsideRepository(refusal)
                    }
                    _ => ExtractError::Refused(refusal),
                });
            }
        }
        Err(ExtractError::Prikk(run_repo.failure(&args, &run)))
    }
}

/// Read prikk's `refusal-v1` document (stdout of a refused `extract`): the schema is checked, every field is required.
pub fn parse_refusal(stdout: &str) -> Result<Refusal, PrikkError> {
    parse_schema_json(stdout, "refusal-v1")
}

/// Read prikk's `extract-report-v1` document.
pub fn parse_report(stdout: &str) -> Result<ExtractReport, PrikkError> {
    parse_schema_json(stdout, "extract-report-v1")
}

/// The report must agree with the call: a mismatch is prikk's output not being what this driver was written for.
fn check_report(
    report: &ExtractReport,
    point: &ExtractPoint,
    options: &ExtractOptions,
) -> Result<(), ExtractError> {
    let bad =
        |why: &str| ExtractError::Prikk(PrikkError::Parse(format!("extract-report-v1: {why}")));
    if report.written == options.plan_only {
        return Err(bad("`written` disagrees with --plan-only"));
    }
    if report.largest_file_bytes > report.total_bytes {
        return Err(bad("the largest file is larger than the total"));
    }
    let claim = matches!(point, ExtractPoint::Claim(_));
    if claim != matches!(report.point_kind, PointKind::Claim) {
        return Err(bad("`point_kind` disagrees with the point asked for"));
    }
    if !claim && report.claim_parent_block_id.is_some() {
        return Err(bad("a claim parent for a point that is not a claim"));
    }
    Ok(())
}
