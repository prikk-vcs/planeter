//! Creating a repository (RFC 004 part 7): empty, or from a maintainer's archive in two steps.
//!
//! # The two paths
//!
//! * **Empty**: reserve the name, `prikk init`, make the creator the administrator, mark the repository `ready`
//!   ([`CreationService::create_empty`]).
//! * **From an archive**: *stage and verify* ([`CreationService::start_stage`], [`CreationService::finish_stage`]), then *confirm
//!   with the maintainer keys typed by the creator* ([`CreationService::confirm`]). The stage is the archive on disk outside
//!   every repository, with what the forge found in it; **nothing is trusted** until the creator types the fingerprints.
//!
//! # What the stage does
//!
//! In a scratch repository of its own (inside the stage's directory, deleted when the stage is made) it runs `archive verify`,
//! then — only to *find out where each ref would land* — adopts **every carried key in that scratch repository** and runs
//! `archive import --adopt` and `verify`. prikk's `archive verify` lists the carried keys but not the refs; the import report
//! is the only place prikk says which refs an archive holds and why one would not be published. The scratch repository is the
//! stager's own and never becomes a hosted repository, so nothing about it is trusted by anything.
//!
//! # What the confirmation does
//!
//! One operation under the new repository's **W lease** with a deadline of its own: reserve → `init` → `trust maintainer add`
//! for **exactly the keys the creator typed** → `archive import` (`--adopt` only when at least one key was typed) → `verify` →
//! the creator becomes administrator → `ready`. Any failure, timeout or kill before `ready` removes the repository whole.
//!
//! # The rule for the final `verify`
//!
//! A repository whose every carried key was adopted must `verify` clean. When the creator left a key out (or the archive
//! carries none), a ref signed by the key left out lands under `remotes/` and `verify` exits 1 with **`publication-trust` and
//! nothing else**: that is the consequence of the choice and is reported as such. Any other failed condition removes the
//! repository.
//!
//! # Memory
//!
//! [`cost_model`] is the one place where the memory cost of the archive calls lives, with its provenance.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use planeter_auth::Sha256Stream;
use planeter_core::creation::{
    Adoption, AdoptionError, AdoptionRequest, Carried, TargetRefusal, creation_target, fits_quota,
    quota_for, resolve_adoption,
};
use planeter_core::{
    CallLimits, HostingError, HostingService, LeaseClass, LeaseError, LeaseManager, Principal,
};
use planeter_prikk::findings::{ArchiveFindings, FoundKey, FoundRef, KEYS_MAX, REFS_LISTED_MAX};
use planeter_prikk::model::ArchiveImportReport;
use planeter_prikk::{PrikkError, PrikkRepo, Sandbox};
use planeter_store::{
    ClaimError, MembershipStore, Owner, ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore,
    Role, StageLimits, StageRecord, StageRefusal, StageState, StageStore, UserId, Visibility,
};

use crate::propose::{SlotGuard, Slots};

const MIB: u64 = 1 << 20;

/// The name of the staged archive inside a stage's read-only `in` directory.
const ARCHIVE_FILE: &str = "archive";

// ------------------------------------------------------------------------------------------------ the cost model

/// The memory cost model of the archive calls (RFC 004 part 7, T0). **One table, one place.**
///
/// What is modelled is the **data segment** (`VmData`, the quantity `RLIMIT_DATA` limits) of the `prikk` child, as in
/// [`crate::propose::cost_model`]. Every constant was measured with **prikk 0.54.1 on 2026-10-27** (the real calls, a sampler
/// of `/proc/<pid>/status`, eleven archives from 0.12 MiB to 256 MiB; the table, the fit and the scripts are in the part 7
/// review request, `repository-creation-logs/t0/`).
///
/// * `archive verify` holds **3.00 ×** the archive's largest blob (3.00 to 3.02 × for one blob of 10, 50, 100 and 200 MiB,
///   random or zeros; 0.03 × for 100 files of 1 MiB; 0.15 × for a 4 MiB file edited 50 times);
/// * `archive import` holds **6.00 ×** the largest blob (6.00 to 6.03 ×; 0.07 × and 0.24 × for the two many-file shapes);
/// * the memory does **not** follow the object count: 5,026 objects (2.8 MiB of archive) cost 2.4 MiB, 0.5 KiB each;
/// * prikk bounds **none** of this itself: `PRIKK_BUNDLE_MAX_*`, `PRIKK_EXCHANGE_MAX_*` and `incoming.max-object-bytes` are
///   ignored by `archive verify` and `archive import` (measured), so the edge bound, the ceiling and the deadline are all the
///   protection there is.
///
/// The largest blob is not known before prikk has read the archive, so — as in part 4b — the whole archive is assumed to be
/// one. An admission that is too small kills a legitimate call; one that is too large only queues.
pub mod cost_model {
    const MIB: u64 = 1 << 20;

    /// The fixed part: prikk's own start. The many-patch archives measured 0.6 to 2.4 MiB of data segment (6.5 to 7.6 MiB
    /// resident) and the repository's own `verify` of 5,026 objects 10.9 MiB; 10 MiB covers them, and about 20,000 objects.
    pub const BASE_BYTES: u64 = 10 * MIB;
    /// `archive verify`: 3.0 × the archive (tenths).
    pub const VERIFY_TENTHS: u64 = 30;
    /// `archive import`: 6.0 × the archive (tenths).
    pub const IMPORT_TENTHS: u64 = 60;
    /// The ceiling over the cost (RFC 011 D-2): unchanged, 1.5 ×. Measured: `archive verify` of 100 MiB of one blob fails at
    /// 2.9 × and passes at 3.1 ×; `archive import` fails at 5.8 × and passes at 6.2 ×; the cost × 1.5 leaves half again.
    pub const CEILING_TENTHS: u64 = 15;

    /// The cost of `archive verify` of an archive of `bytes`.
    pub fn verify_cost(bytes: u64) -> u64 {
        BASE_BYTES.saturating_add(bytes.saturating_mul(VERIFY_TENTHS) / 10)
    }

    /// The cost of `archive import` of an archive of `bytes` (and of the `verify` of the repository it makes, which measured
    /// 2.0 to 4.1 × the store, never above the import's 6.0 ×).
    pub fn import_cost(bytes: u64) -> u64 {
        BASE_BYTES.saturating_add(bytes.saturating_mul(IMPORT_TENTHS) / 10)
    }

    /// What one staging, or one confirmation, is admitted for: its calls run one after another, so it holds the largest of
    /// them, the import's.
    pub fn admission_cost(bytes: u64) -> u64 {
        import_cost(bytes).max(verify_cost(bytes))
    }

    /// The `RLIMIT_DATA` of `archive verify`.
    pub fn verify_ceiling(bytes: u64) -> u64 {
        verify_cost(bytes).saturating_mul(CEILING_TENTHS) / 10
    }

    /// The `RLIMIT_DATA` of `archive import`, of the `verify` after it, and of the other calls of a creation.
    pub fn import_ceiling(bytes: u64) -> u64 {
        import_cost(bytes).saturating_mul(CEILING_TENTHS) / 10
    }

    /// The largest archive a memory pool of `pool_bytes` admits (the inverse of [`admission_cost`]); `None` when the pool
    /// cannot hold even the fixed part.
    pub fn largest_archive_admitted(pool_bytes: u64) -> Option<u64> {
        pool_bytes
            .checked_sub(BASE_BYTES)
            .map(|rest| rest.saturating_mul(10) / IMPORT_TENTHS)
    }
}

// ------------------------------------------------------------------------------------------------ settings and errors

/// Operator settings of repository creation.
#[derive(Debug, Clone)]
pub struct CreationConfig {
    /// The largest archive (`PLANETER_IMPORT_MAX_MB`), default 100 MiB: refused at the edge before the first byte is read.
    pub max_archive_bytes: u64,
    /// The wall-time bound of one staging and of one confirmation (default 600 s, `PLANETER_CREATE_DEADLINE_SECS`).
    pub deadline: Duration,
    /// How long a stage lives (24 h).
    pub stage_ttl: Duration,
    /// At most this many stages per principal, and this many archive bytes in all (`PLANETER_STAGE_BUDGET_MB`).
    pub stage_limits: StageLimits,
    /// The most repositories a user (50) and an organization (500) may own.
    pub user_quota: u64,
    pub org_quota: u64,
    /// How long a creation waits for a slot and for the new repository's write lease before it is refused as busy.
    pub lease_wait: Duration,
    /// Stagings and confirmations at once on this host (default 2).
    pub max_concurrent: usize,
}

impl Default for CreationConfig {
    fn default() -> Self {
        CreationConfig {
            max_archive_bytes: 100 * MIB,
            deadline: Duration::from_secs(600),
            stage_ttl: Duration::from_secs(24 * 3600),
            stage_limits: StageLimits {
                per_principal: 2,
                total_bytes: 1024 * MIB,
            },
            user_quota: planeter_core::creation::DEFAULT_USER_QUOTA,
            org_quota: planeter_core::creation::DEFAULT_ORG_QUOTA,
            lease_wait: Duration::from_secs(5),
            max_concurrent: 2,
        }
    }
}

/// Why a creation, a staging or a confirmation was refused.
#[derive(Debug)]
#[non_exhaustive]
pub enum CreateError {
    /// The caller is not signed in.
    NotSignedIn,
    /// The caller may not create there — someone else's namespace, an organization they are not in, one that does not exist:
    /// **not told apart**.
    Refused,
    /// The namespace holds its quota.
    Quota { limit: u64, owned: u64 },
    /// `owner/name` is taken (said only to someone who may create there).
    NameTaken,
    /// The server has no stage directory (no `PLANETER_ARTIFACT_DIR`): creation from an archive is off.
    ImportOff,
    /// A slot or a lease did not come free in time.
    Busy { retry_after: Duration },
    /// The archive is larger than the bound.
    TooLarge { limit: u64 },
    /// Too many stages for this principal, or the stages' byte budget is spent.
    StageLimit(StageRefusal),
    /// prikk refused the archive; `text` is prikk's own words.
    ArchiveRefused { text: String },
    /// The archive carries more maintainer keys than a stage keeps (or one with a malformed id or key).
    Keys { carried: usize },
    /// No such stage for this principal (also: already confirmed, and a stage that is someone else's).
    StageNotFound,
    /// The stage's day is over.
    StageExpired,
    /// A confirmation is running on this stage.
    StageInUse,
    /// The typed fingerprints do not match (never says what was expected).
    Adoption(AdoptionError),
    /// The repository the import made does not verify; the ids of the conditions that failed.
    VerifyFailed { conditions: Vec<String> },
    /// The wall-time bound fired.
    TimedOut,
    /// A call hit its memory ceiling.
    CeilingExceeded,
    /// Something the caller cannot fix (logged).
    Unavailable,
}

impl CreateError {
    /// A stable word for logs and counters.
    pub fn reason(&self) -> &'static str {
        match self {
            CreateError::NotSignedIn => "not_signed_in",
            CreateError::Refused => "refused",
            CreateError::Quota { .. } => "quota",
            CreateError::NameTaken => "name_taken",
            CreateError::ImportOff => "import_off",
            CreateError::Busy { .. } => "busy",
            CreateError::TooLarge { .. } => "too_large",
            CreateError::StageLimit(_) => "stage_limit",
            CreateError::ArchiveRefused { .. } => "archive_refused",
            CreateError::Keys { .. } => "keys",
            CreateError::StageNotFound => "stage_not_found",
            CreateError::StageExpired => "stage_expired",
            CreateError::StageInUse => "stage_in_use",
            CreateError::Adoption(_) => "adoption",
            CreateError::VerifyFailed { .. } => "verify_failed",
            CreateError::TimedOut => "timed_out",
            CreateError::CeilingExceeded => "ceiling",
            CreateError::Unavailable => "unavailable",
        }
    }

    /// The failure is the archive's own: a stage that met it is not kept for another try.
    fn is_the_archives(&self) -> bool {
        matches!(
            self,
            CreateError::ArchiveRefused { .. }
                | CreateError::Keys { .. }
                | CreateError::VerifyFailed { .. }
        )
    }
}

/// The text of prikk's refusal, trimmed and bounded (it is shown behind our own sentence).
fn prikk_text(stderr: &str) -> String {
    let t = stderr.trim();
    let t = t.strip_prefix("error:").map_or(t, str::trim);
    t.chars().take(1000).collect()
}

fn log_unavailable(what: &str, e: impl std::fmt::Display) -> CreateError {
    eprintln!("planeter: creation: {what}: {e}");
    CreateError::Unavailable
}

/// A prikk failure of a call that reads the **archive**: prikk's refusal (exit 1) is the archive's fault.
fn archive_call(e: PrikkError) -> CreateError {
    match e {
        PrikkError::Command {
            code: Some(planeter_prikk::sandbox::TIMEOUT_EXIT_CODE),
            ..
        } => CreateError::TimedOut,
        PrikkError::Command {
            code: Some(1),
            stderr,
        } => CreateError::ArchiveRefused {
            text: prikk_text(&stderr),
        },
        PrikkError::CeilingExceeded { .. } => CreateError::CeilingExceeded,
        // An archive whose report is larger than this server reads (a million one-line branches, say) is refused as the archive's:
        // the call was killed (IS-55) and nothing of it is kept.
        PrikkError::OutputTooLarge { .. } => CreateError::ArchiveRefused {
            text: "prikk's report on the archive is larger than this server reads".to_owned(),
        },
        other => log_unavailable("an archive call failed", other),
    }
}

/// A prikk failure of a call that does not read the archive (`init`, `trust`, `verify` of the made repository).
fn infra_call(e: PrikkError) -> CreateError {
    match e {
        PrikkError::Command {
            code: Some(planeter_prikk::sandbox::TIMEOUT_EXIT_CODE),
            ..
        } => CreateError::TimedOut,
        PrikkError::CeilingExceeded { .. } => CreateError::CeilingExceeded,
        other => log_unavailable("a prikk call failed", other),
    }
}

// ------------------------------------------------------------------------------------------------ small pieces

struct Deadline(Instant);

impl Deadline {
    fn after(d: Duration) -> Self {
        Deadline(Instant::now() + d)
    }

    /// Whole seconds left, at least one; `TimedOut` when none.
    fn remaining_secs(&self) -> Result<u64, CreateError> {
        let left = self.0.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(CreateError::TimedOut)
        } else {
            Ok(left.as_secs().max(1))
        }
    }
}

/// `s-` and 24 hexadecimal characters: the only shape of a stage id, so one taken from a request can never name a path
/// outside the stage root.
fn stage_id_ok(id: &str) -> bool {
    id.len() == 26
        && id.starts_with("s-")
        && id[2..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn new_stage_id() -> String {
    let token = planeter_auth::random_token();
    format!("s-{}", &planeter_auth::sha256_hex(token.as_bytes())[..24])
}

/// Remove the directory of stage `id` under `root`. Only a name of the stage shape; a link is unlinked, never entered.
fn remove_stage_dir(root: &Path, id: &str) -> std::io::Result<bool> {
    if !stage_id_ok(id) {
        return Err(std::io::Error::other("not a stage id"));
    }
    let p = root.join(id);
    match fs::symlink_metadata(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
        Ok(m) if m.file_type().is_symlink() => fs::remove_file(&p).map(|()| true),
        Ok(m) if m.is_dir() => fs::remove_dir_all(&p).map(|()| true),
        Ok(_) => Err(std::io::Error::other("not a directory")),
    }
}

/// A stage's directory, removed when dropped unless it is kept.
struct StageDirGuard {
    root: PathBuf,
    id: String,
    keep: bool,
}

impl Drop for StageDirGuard {
    fn drop(&mut self) {
        if !self.keep
            && let Err(e) = remove_stage_dir(&self.root, &self.id)
        {
            eprintln!(
                "planeter: creation: could not remove the stage {}: {e}; the next start removes it",
                self.id
            );
        }
    }
}

impl StageDirGuard {
    fn path(&self) -> PathBuf {
        self.root.join(&self.id)
    }
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    DirBuilder::new().mode(0o700).create(path)
}

/// The SHA-256 of a file, streamed.
fn digest_of(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256Stream::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finish_hex().as_str().to_owned())
}

// ------------------------------------------------------------------------------------------------ the upload

/// An archive being written to a stage's directory: bounded, hashed as it arrives, never held in memory.
pub struct StageUpload {
    guard: StageDirGuard,
    file: Option<File>,
    hash: Sha256Stream,
    written: u64,
    limit: u64,
    _slot: SlotGuard,
}

impl StageUpload {
    /// Append the next bytes. Past the bound is refused.
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), CreateError> {
        self.written = self.written.saturating_add(chunk.len() as u64);
        if self.written > self.limit {
            return Err(CreateError::TooLarge { limit: self.limit });
        }
        self.hash.update(chunk);
        self.file
            .as_mut()
            .expect("the file lives until finish")
            .write_all(chunk)
            .map_err(|e| log_unavailable("cannot write the staged archive", e))
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The upload is complete: the file is synced and its digest known.
    pub fn finish(self) -> Result<Uploaded, CreateError> {
        let StageUpload {
            guard,
            file,
            hash,
            written,
            _slot,
            ..
        } = self;
        let file = file.expect("the file lives until finish");
        file.sync_all()
            .map_err(|e| log_unavailable("cannot sync the staged archive", e))?;
        drop(file);
        Ok(Uploaded {
            guard,
            digest: hash.finish_hex().as_str().to_owned(),
            size: written,
            _slot,
        })
    }
}

/// A finished upload, waiting to be verified ([`CreationService::finish_stage`]).
pub struct Uploaded {
    guard: StageDirGuard,
    pub digest: String,
    pub size: u64,
    _slot: SlotGuard,
}

// ------------------------------------------------------------------------------------------------ what comes back

/// What the creator asked a stage to become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageMeta {
    pub owner: Owner,
    pub name: String,
    pub visibility: Visibility,
    pub proposals: ProposalPolicy,
}

/// A stage as the creator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSummary {
    pub stage_id: String,
    pub owner: Owner,
    pub name: String,
    pub visibility: Visibility,
    pub proposals: ProposalPolicy,
    pub archive_bytes: u64,
    pub archive_digest: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub findings: ArchiveFindings,
}

impl StageSummary {
    fn from_record(r: StageRecord) -> Option<StageSummary> {
        let findings = ArchiveFindings::from_json(&r.report)?;
        Some(StageSummary {
            stage_id: r.stage_id,
            owner: r.owner,
            name: r.name,
            visibility: r.visibility,
            proposals: r.proposals,
            archive_bytes: r.archive_bytes,
            archive_digest: r.archive_digest,
            created_at: r.created_at,
            expires_at: r.expires_at,
            findings,
        })
    }
}

/// Where one ref of the archive landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingOutcome {
    pub ref_name: String,
    pub landed_as: String,
    /// A published branch (adopted); otherwise it is under `remotes/`.
    pub published: bool,
    /// prikk's reason when it is not published.
    pub reason: Option<String>,
}

/// The result of a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmed {
    pub record: RepositoryRecord,
    pub landings: Vec<LandingOutcome>,
    /// How many refs the archive carries (more than `landings` when a great many).
    pub landings_total: u64,
    pub adopted: Vec<FoundKey>,
    pub left_out: Vec<FoundKey>,
    /// The repository passed `verify`. False only when keys were left out: the refs they sign are under `remotes/` and
    /// `verify` says `publication-trust` until they are adopted.
    pub verified: bool,
    pub objects: u64,
    pub note: Option<String>,
}

/// What the start-up sweep did.
#[derive(Debug, Default)]
pub struct StartupSweep {
    pub repositories: planeter_core::SweepReport,
    pub stages_removed: Vec<String>,
}

// ------------------------------------------------------------------------------------------------ the service

/// The creation service. Every method is **blocking**: call it from `spawn_blocking`, under the gate's admission for the ones
/// that run prikk ([`cost_model::admission_cost`]).
pub struct CreationService {
    hosting: Arc<HostingService>,
    repos: Arc<dyn RepositoryStore>,
    membership: Arc<dyn MembershipStore>,
    stages: Arc<dyn StageStore>,
    leases: Arc<LeaseManager>,
    stage_root: Option<PathBuf>,
    config: CreationConfig,
    slots: Arc<Slots>,
    now_unix: fn() -> u64,
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl CreationService {
    /// A service. `stage_root` is the directory stages live in — **outside every repository** (the caller has checked it); `None`
    /// turns creation from an archive off. The directory is created.
    pub fn new(
        hosting: Arc<HostingService>,
        repos: Arc<dyn RepositoryStore>,
        membership: Arc<dyn MembershipStore>,
        stages: Arc<dyn StageStore>,
        leases: Arc<LeaseManager>,
        stage_root: Option<PathBuf>,
        config: CreationConfig,
    ) -> std::io::Result<Self> {
        let stage_root = match stage_root {
            Some(p) => {
                fs::create_dir_all(&p)?;
                Some(fs::canonicalize(&p)?)
            }
            None => None,
        };
        let slots = Slots::new(config.max_concurrent);
        Ok(CreationService {
            hosting,
            repos,
            membership,
            stages,
            leases,
            stage_root,
            config,
            slots,
            now_unix: system_now,
        })
    }

    /// Test seam: the clock.
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    pub fn config(&self) -> &CreationConfig {
        &self.config
    }

    /// Creation from an archive is available (a stage directory is configured).
    pub fn import_enabled(&self) -> bool {
        self.stage_root.is_some()
    }

    // -------------------------------------------------------------------------------- who, where, how many

    /// Where `principal` may create a repository named for `handle`.
    pub fn target(&self, principal: &Principal, handle: &str) -> Result<Owner, CreateError> {
        creation_target(principal, handle, &*self.membership, (self.now_unix)()).map_err(
            |r| match r {
                TargetRefusal::NotSignedIn => CreateError::NotSignedIn,
                TargetRefusal::Refused => CreateError::Refused,
            },
        )
    }

    /// There is room for `owner/name`: the quota (a reservation counts) and the name. Said only to someone `target` allowed.
    pub fn check_room(&self, owner: &Owner, name: &str) -> Result<(), CreateError> {
        let limit = quota_for(owner, self.config.user_quota, self.config.org_quota);
        let owned = self
            .repos
            .count_owned(owner)
            .map_err(|e| log_unavailable("cannot count the repositories", e))?;
        if !fits_quota(owned, limit) {
            return Err(CreateError::Quota { limit, owned });
        }
        if self
            .repos
            .resolve(owner, name)
            .map_err(|e| log_unavailable("cannot look the name up", e))?
            .is_some()
        {
            return Err(CreateError::NameTaken);
        }
        Ok(())
    }

    fn begin(
        &self,
        owner: Owner,
        name: String,
        visibility: Visibility,
        proposals: ProposalPolicy,
    ) -> Result<planeter_core::Creation<'_>, CreateError> {
        self.hosting
            .begin_creation(owner, name, visibility, proposals)
            .map_err(|e| match e {
                HostingError::NameTaken { .. } => CreateError::NameTaken,
                other => log_unavailable("cannot reserve the repository", other),
            })
    }

    // -------------------------------------------------------------------------------- empty

    /// Create an empty repository. The caller has run [`CreationService::target`]; the quota and the name are checked here,
    /// before any byte is written.
    pub fn create_empty(
        &self,
        owner: Owner,
        name: String,
        visibility: Visibility,
        proposals: ProposalPolicy,
        creator: &UserId,
    ) -> Result<RepositoryRecord, CreateError> {
        self.check_room(&owner, &name)?;
        let creation = self.begin(owner, name, visibility, proposals)?;
        let id = creation.record().repo_id.clone();
        let result = (|| {
            creation.driver().init_configured().map_err(infra_call)?;
            self.membership
                .grant_user(creator.clone(), id.clone(), Role::Admin)
                .map_err(|e| log_unavailable("cannot make the creator the administrator", e))?;
            creation
                .finish()
                .map_err(|e| log_unavailable("cannot mark the repository ready", e))
        })();
        if result.is_err() {
            let _ = self.membership.forget_repo(&id);
        }
        result
    }

    // -------------------------------------------------------------------------------- stage

    fn root(&self) -> Result<&Path, CreateError> {
        self.stage_root.as_deref().ok_or(CreateError::ImportOff)
    }

    /// Begin a stage: the expired ones are swept, a slot is taken, and the principal's limits are checked against the
    /// `expected` size **before a byte is read** (the final decision is atomic with the insert). The archive is written into
    /// the returned upload.
    pub fn start_stage(
        &self,
        principal: &UserId,
        expected: u64,
    ) -> Result<StageUpload, CreateError> {
        let root = self.root()?.to_path_buf();
        if expected > self.config.max_archive_bytes {
            return Err(CreateError::TooLarge {
                limit: self.config.max_archive_bytes,
            });
        }
        self.sweep_stages();
        let rows = self
            .stages
            .all()
            .map_err(|e| log_unavailable("cannot read the stages", e))?;
        let limits = self.config.stage_limits;
        let mine = rows
            .iter()
            .filter(|r| r.principal == principal.as_str())
            .count();
        if mine as u64 >= u64::from(limits.per_principal) {
            return Err(CreateError::StageLimit(StageRefusal::TooManyForPrincipal {
                limit: limits.per_principal,
            }));
        }
        let held: u64 = rows.iter().map(|r| r.archive_bytes).sum();
        if held.saturating_add(expected) > limits.total_bytes {
            return Err(CreateError::StageLimit(StageRefusal::BudgetExceeded {
                limit: limits.total_bytes,
            }));
        }
        let slot = self
            .slots
            .acquire(self.config.lease_wait)
            .ok_or(CreateError::Busy {
                retry_after: self.config.lease_wait,
            })?;
        let id = new_stage_id();
        let guard = StageDirGuard {
            root: root.clone(),
            id: id.clone(),
            keep: false,
        };
        private_dir(&guard.path())
            .map_err(|e| log_unavailable("cannot make the stage directory", e))?;
        private_dir(&guard.path().join("in"))
            .map_err(|e| log_unavailable("cannot make the stage directory", e))?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(guard.path().join("in").join(ARCHIVE_FILE))
            .map_err(|e| log_unavailable("cannot create the staged archive", e))?;
        Ok(StageUpload {
            guard,
            file: Some(file),
            hash: Sha256Stream::new(),
            written: 0,
            limit: self.config.max_archive_bytes,
            _slot: slot,
        })
    }

    /// The sandbox of one call that reads the staged archive: the stage's `in` directory read-only, a wall-time bound of what
    /// is left of the operation's deadline.
    fn sandbox_reading(&self, in_dir: &Path, deadline: &Deadline) -> Result<Sandbox, CreateError> {
        Ok(self
            .hosting
            .sandbox()
            .with_artifact_in(in_dir)
            .with_deadline(deadline.remaining_secs()?))
    }

    /// Verify the uploaded archive and keep it as a stage (see the module documentation). `meta` was checked at the edge
    /// (who may create there, the quota, the name) before the body was read; `principal` is the stager.
    pub fn finish_stage(
        &self,
        uploaded: Uploaded,
        meta: StageMeta,
        principal: &UserId,
    ) -> Result<StageSummary, CreateError> {
        let Uploaded {
            mut guard,
            digest,
            size,
            _slot,
        } = uploaded;
        if size == 0 {
            return Err(CreateError::ArchiveRefused {
                text: "the archive is empty".to_owned(),
            });
        }
        let stage_dir = guard.path();
        let in_dir = stage_dir.join("in");
        let archive = in_dir.join(ARCHIVE_FILE);
        let scratch = stage_dir.join("scratch");
        let deadline = Deadline::after(self.config.deadline);
        private_dir(&scratch)
            .map_err(|e| log_unavailable("cannot make the scratch repository", e))?;
        let call = |ceiling: u64| -> Result<planeter_prikk::CliPrikkRepo, CreateError> {
            Ok(self.hosting.driver_in(
                &scratch,
                self.sandbox_reading(&in_dir, &deadline)?,
                CallLimits::data_ceiling(ceiling),
            ))
        };
        let verify_ceiling = cost_model::verify_ceiling(size);
        let import_ceiling = cost_model::import_ceiling(size);

        call(verify_ceiling)?
            .init_configured()
            .map_err(infra_call)?;
        let report = call(verify_ceiling)?
            .archive_verify(&archive)
            .map_err(archive_call)?;
        if !report.passed() {
            return Err(CreateError::ArchiveRefused {
                text: "the archive is not internally consistent".to_owned(),
            });
        }
        let mut keys: Vec<FoundKey> = Vec::new();
        for k in &report.maintainer_keys {
            if !k.is_well_formed() {
                return Err(CreateError::Keys {
                    carried: report.maintainer_keys.len(),
                });
            }
            if let Some(same) = keys.iter().find(|f| f.key_id == k.key_id) {
                if same.public_key != k.public_key {
                    return Err(CreateError::Keys {
                        carried: report.maintainer_keys.len(),
                    });
                }
                continue;
            }
            keys.push(FoundKey {
                key_id: k.key_id.clone(),
                public_key: k.public_key.clone(),
            });
        }
        if keys.len() > KEYS_MAX {
            return Err(CreateError::Keys {
                carried: keys.len(),
            });
        }

        // The dry run, in the scratch repository only: every carried key adopted there, to learn where each ref would land.
        for k in &keys {
            call(import_ceiling)?
                .trust_maintainer_add(&k.key_id, &k.public_key)
                .map_err(infra_call)?;
        }
        let imported: ArchiveImportReport = call(import_ceiling)?
            .archive_import(&archive, true)
            .map_err(archive_call)?;
        let verified = call(import_ceiling)?.verify().map_err(infra_call)?;

        let mut objects: std::collections::BTreeMap<String, u64> = imported.written_objects.clone();
        for (kind, n) in &imported.merged_objects {
            *objects.entry(kind.clone()).or_default() += n;
        }
        let findings = ArchiveFindings {
            refs_total: imported.landings.len() as u64,
            refs: imported
                .landings
                .iter()
                .take(REFS_LISTED_MAX)
                .map(|l| FoundRef {
                    ref_name: l.ref_name.clone(),
                    publishable: l.adopted,
                    landed_as: l.landed_as.clone(),
                    reason: l.remote_reason.clone(),
                })
                .collect(),
            keys,
            object_total: imported.object_count(),
            objects,
            verifies_with_all_keys: verified.verdict.ok,
            verify_failures: verified
                .verdict
                .failed_conditions
                .iter()
                .map(|c| c.id.clone())
                .collect(),
            note: imported.note.clone().map(|n| prikk_text(&n)),
        };

        // The scratch repository has done its work: its bytes go now, the archive stays.
        fs::remove_dir_all(&scratch)
            .map_err(|e| log_unavailable("cannot remove the scratch repository", e))?;
        let now = (self.now_unix)();
        let record = StageRecord {
            stage_id: guard.id.clone(),
            principal: principal.as_str().to_owned(),
            owner: meta.owner,
            name: meta.name,
            visibility: meta.visibility,
            proposals: meta.proposals,
            archive_digest: digest,
            archive_bytes: size,
            created_at: now,
            expires_at: now.saturating_add(self.config.stage_ttl.as_secs()),
            state: StageState::Staged,
            report: findings.to_json(),
        };
        match self
            .stages
            .put(record.clone(), self.config.stage_limits)
            .map_err(|e| log_unavailable("cannot keep the stage", e))?
        {
            Ok(()) => {}
            Err(refusal) => return Err(CreateError::StageLimit(refusal)),
        }
        guard.keep = true;
        StageSummary::from_record(record).ok_or(CreateError::Unavailable)
    }

    /// The stage `stage_id` of `principal`. Someone else's stage, and one that does not exist, are one answer.
    pub fn stage(&self, principal: &UserId, stage_id: &str) -> Result<StageSummary, CreateError> {
        if !stage_id_ok(stage_id) {
            return Err(CreateError::StageNotFound);
        }
        let rec = self
            .stages
            .get(stage_id)
            .map_err(|e| log_unavailable("cannot read the stage", e))?
            .filter(|r| r.principal == principal.as_str())
            .ok_or(CreateError::StageNotFound)?;
        if rec.expires_at <= (self.now_unix)() {
            return Err(CreateError::StageExpired);
        }
        StageSummary::from_record(rec).ok_or(CreateError::Unavailable)
    }

    // -------------------------------------------------------------------------------- confirm

    /// Confirm a stage: the typed fingerprints decide which keys are trusted (see the module documentation). `principal` is the
    /// caller as the authorization sees it, `user` the same caller as the stage's owner.
    pub fn confirm(
        &self,
        principal: &Principal,
        user: &UserId,
        stage_id: &str,
        req: &AdoptionRequest,
    ) -> Result<Confirmed, CreateError> {
        let root = self.root()?.to_path_buf();
        let summary = self.stage(user, stage_id)?;
        let carried: Vec<Carried> = summary
            .findings
            .keys
            .iter()
            .map(|k| Carried {
                key_id: k.key_id.clone(),
                fingerprint: k.public_key.clone(),
            })
            .collect();
        let adoption = resolve_adoption(&carried, req).map_err(CreateError::Adoption)?;
        // Who may create where is asked again: a membership may have changed since the stage was made.
        let owner = self.target(principal, summary.owner.handle())?;
        if owner != summary.owner {
            return Err(CreateError::Refused);
        }
        self.check_room(&owner, &summary.name)?;
        let claimed = match self
            .stages
            .claim(stage_id, user.as_str(), (self.now_unix)())
            .map_err(|e| log_unavailable("cannot claim the stage", e))?
        {
            Ok(r) => r,
            Err(ClaimError::NotFound) => return Err(CreateError::StageNotFound),
            Err(ClaimError::Expired) => return Err(CreateError::StageExpired),
            Err(ClaimError::InUse) => return Err(CreateError::StageInUse),
        };
        let outcome = self.run_confirmation(&root, &claimed, &summary, &adoption, user);
        match &outcome {
            Ok(_) => self.drop_stage(&root, stage_id),
            Err(e) if e.is_the_archives() => self.drop_stage(&root, stage_id),
            Err(_) => {
                // Not the archive's fault (busy, timed out, a ceiling, something broken): the stage may be tried again.
                if let Err(e) = self.stages.release(stage_id) {
                    eprintln!("planeter: creation: cannot release stage {stage_id}: {e}");
                }
            }
        }
        outcome
    }

    fn drop_stage(&self, root: &Path, stage_id: &str) {
        if let Err(e) = remove_stage_dir(root, stage_id) {
            eprintln!("planeter: creation: cannot remove stage {stage_id}: {e}");
        }
        if let Err(e) = self.stages.remove(stage_id) {
            eprintln!("planeter: creation: cannot forget stage {stage_id}: {e}");
        }
    }

    fn run_confirmation(
        &self,
        root: &Path,
        claimed: &StageRecord,
        summary: &StageSummary,
        adoption: &Adoption,
        creator: &UserId,
    ) -> Result<Confirmed, CreateError> {
        let in_dir = root.join(&claimed.stage_id).join("in");
        let archive = in_dir.join(ARCHIVE_FILE);
        let deadline = Deadline::after(self.config.deadline);
        // The bytes that are imported are the bytes that were verified.
        match digest_of(&archive) {
            Ok(d) if d == claimed.archive_digest => {}
            Ok(_) => {
                eprintln!(
                    "planeter: creation: the staged archive of {} changed on disk",
                    claimed.stage_id
                );
                return Err(CreateError::ArchiveRefused {
                    text: "the staged archive is not the one that was verified".to_owned(),
                });
            }
            Err(e) => return Err(log_unavailable("cannot read the staged archive", e)),
        }
        let creation = self.begin(
            claimed.owner.clone(),
            claimed.name.clone(),
            claimed.visibility,
            claimed.proposals,
        )?;
        let id: RepoId = creation.record().repo_id.clone();
        let result = (|| {
            let lease = self
                .leases
                .acquire(
                    &id,
                    LeaseClass::Write,
                    "archive import",
                    self.config.lease_wait,
                )
                .map_err(|e| match e {
                    LeaseError::Busy { retry_after, .. } => CreateError::Busy { retry_after },
                    other => log_unavailable("cannot take the write lease", other),
                })?;
            let ceiling =
                CallLimits::data_ceiling(cost_model::import_ceiling(claimed.archive_bytes));
            let driver = || -> Result<planeter_prikk::CliPrikkRepo, CreateError> {
                Ok(creation.driver_sandboxed(self.sandbox_reading(&in_dir, &deadline)?, ceiling))
            };
            driver()?.init_configured().map_err(infra_call)?;
            // Exactly the keys the creator typed, and no others.
            for k in &adoption.adopted {
                let public_key = summary
                    .findings
                    .keys
                    .iter()
                    .find(|f| f.key_id == k.key_id)
                    .map(|f| f.public_key.clone())
                    .ok_or(CreateError::Unavailable)?;
                driver()?
                    .trust_maintainer_add(&k.key_id, &public_key)
                    .map_err(infra_call)?;
            }
            let adopt = !adoption.adopted.is_empty();
            let imported = driver()?
                .archive_import(&archive, adopt)
                .map_err(archive_call)?;
            let verdict = driver()?.verify().map_err(infra_call)?;
            let every_key_adopted =
                !summary.findings.keys.is_empty() && adoption.left_out.is_empty();
            let verified = verdict.verdict.ok;
            if !verified {
                let only_trust = verdict
                    .verdict
                    .failed_conditions
                    .iter()
                    .all(|c| c.id == "publication-trust");
                if every_key_adopted || !only_trust {
                    return Err(CreateError::VerifyFailed {
                        conditions: verdict
                            .verdict
                            .failed_conditions
                            .iter()
                            .map(|c| c.id.clone())
                            .collect(),
                    });
                }
            }
            self.membership
                .grant_user(creator.clone(), id.clone(), Role::Admin)
                .map_err(|e| log_unavailable("cannot make the creator the administrator", e))?;
            drop(lease);
            Ok((imported, verified))
        })();
        let (imported, verified) = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = self.membership.forget_repo(&id);
                return Err(e);
            }
        };
        let record = match creation.finish() {
            Ok(r) => r,
            Err(e) => {
                let _ = self.membership.forget_repo(&id);
                return Err(log_unavailable("cannot mark the repository ready", e));
            }
        };
        let found = |c: &Carried| FoundKey {
            key_id: c.key_id.clone(),
            public_key: c.fingerprint.clone(),
        };
        Ok(Confirmed {
            record,
            landings_total: imported.landings.len() as u64,
            landings: imported
                .landings
                .iter()
                .take(REFS_LISTED_MAX)
                .map(|l| LandingOutcome {
                    ref_name: l.ref_name.clone(),
                    landed_as: l.landed_as.clone(),
                    published: l.adopted,
                    reason: l.remote_reason.clone(),
                })
                .collect(),
            adopted: adoption.adopted.iter().map(found).collect(),
            left_out: adoption.left_out.iter().map(found).collect(),
            verified,
            objects: imported.object_count(),
            note: imported.note.map(|n| prikk_text(&n)),
        })
    }

    // -------------------------------------------------------------------------------- sweeps

    /// Remove the stages whose day is over (their archives and their records). Called before a stage is made, at start and
    /// hourly. Returns how many were removed.
    pub fn sweep_stages(&self) -> usize {
        let Some(root) = self.stage_root.as_deref() else {
            return 0;
        };
        let expired = match self.stages.expired((self.now_unix)()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("planeter: creation: cannot list the expired stages: {e}");
                return 0;
            }
        };
        let mut removed = 0;
        for stage in expired {
            match remove_stage_dir(root, &stage.stage_id) {
                Ok(_) => {
                    if self.stages.remove(&stage.stage_id).is_ok() {
                        removed += 1;
                    }
                }
                Err(e) => eprintln!(
                    "planeter: creation: cannot remove the expired stage {}: {e}",
                    stage.stage_id
                ),
            }
        }
        removed
    }

    /// The start-up sweep (RFC 004 part 7, T1). **Run it before anything reads the lease records or serves a request.**
    ///
    /// * every repository that was `creating` is removed whole, with its grants and the lease record the killed process left;
    /// * every directory under the repository root that no record owns is removed (see
    ///   [`HostingService::sweep_unfinished`] for the guards);
    /// * every stage a killed confirmation had claimed, every expired stage, and every stage directory no record owns is removed.
    ///
    /// Each removal is logged.
    pub fn sweep_at_start(&self) -> StartupSweep {
        let mut out = StartupSweep::default();
        match self.hosting.sweep_unfinished() {
            Ok(report) => out.repositories = report,
            Err(e) => eprintln!("planeter: start-up sweep of the repositories failed: {e}"),
        }
        for id in &out.repositories.removed_creating {
            if let Err(e) = self.membership.forget_repo(id) {
                eprintln!("planeter: start-up sweep: cannot forget the grants of {id}: {e}");
            }
            if let Ok(orphans) = self.leases.orphans() {
                for o in orphans.into_iter().filter(|o| &o.repo_id == id) {
                    let _ = self.leases.clear_orphan(&o.holder_id);
                    eprintln!(
                        "planeter: start-up sweep: cleared the lease record of the unfinished creation {id}"
                    );
                }
            }
        }
        let Some(root) = self.stage_root.as_deref() else {
            return out;
        };
        let now = (self.now_unix)();
        let rows = self.stages.all().unwrap_or_default();
        for row in &rows {
            let claimed = row.state == StageState::Confirming;
            if claimed || row.expires_at <= now {
                match remove_stage_dir(root, &row.stage_id) {
                    Ok(_) => {
                        if self.stages.remove(&row.stage_id).is_ok() {
                            eprintln!(
                                "planeter: start-up sweep: removed stage {} ({})",
                                row.stage_id,
                                if claimed {
                                    "a confirmation was running when the server stopped"
                                } else {
                                    "expired"
                                }
                            );
                            out.stages_removed.push(row.stage_id.clone());
                        }
                    }
                    Err(e) => eprintln!(
                        "planeter: start-up sweep: cannot remove stage {}: {e}",
                        row.stage_id
                    ),
                }
            }
        }
        let known: std::collections::HashSet<String> = self
            .stages
            .all()
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.stage_id)
            .collect();
        if let Ok(entries) = fs::read_dir(root) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if stage_id_ok(&name) && !known.contains(&name) {
                    match remove_stage_dir(root, &name) {
                        Ok(_) => {
                            eprintln!(
                                "planeter: start-up sweep: removed the stage directory {name}, which no record owns"
                            );
                            out.stages_removed.push(name);
                        }
                        Err(e) => {
                            eprintln!("planeter: start-up sweep: cannot remove {name}: {e}");
                        }
                    }
                }
            }
        }
        out
    }
}
