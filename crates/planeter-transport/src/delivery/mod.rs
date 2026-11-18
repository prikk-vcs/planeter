//! **The delivery engine** (RFC 008, landing by delivery). A maintainer delivers a repository archive; the forge accepts it only as the
//! *successor* of the branch it holds, checks it in a scratch repository that trusts nothing but the forge's own adopted keys, tells the
//! maintainer what would happen, and — on their confirmation — replaces the repository by the stage. **Nothing here is reachable from a
//! route yet** (handoffs 2 and 3 are the API and the client).
//!
//! ```text
//!   start ──upload──► finish ─► stage ─► decide ─► report ──► confirm ─► install ─► done
//!   (busy if one is in flight)   (the stage is the safety argument)    (W lease, intent, exchange, re-accept)
//! ```
//!
//! * **Storage** is [`planeter_store::DeliveryStore`]: one row per delivery, one in flight per repository. **The state is the intent**: every
//!   step that moves anything is preceded by its state, so a restart knows what was done ([`DeliveryService::reconcile_at_start`]).
//! * **The stage** (RFC 008 D-2) is a fresh repository in the delivery's own directory, on the repositories' own filesystem. It trusts
//!   exactly the live repository's adopted keys, **never a key the archive carries**; `archive verify`, `archive import --adopt`,
//!   `branch --all`, `verify` and `log` of every ref are read from it, every call sandboxed under the creation cost model's ceilings. A call
//!   that is killed is never resumed: the stage is thrown away and the delivery is `failed`.
//! * **The rule** is [`rule::decide`], one pure function over those facts (RFC 008 D-3).
//! * **The install** (RFC 008 D-7), under the repository's write lease: re-check, write the intent (`confirmed`), put the stage in the
//!   live directory's place, bump the revision counter, re-accept the retained artifact of every revision of every open proposal in
//!   order, `verify`, finish. **Nothing after the exchange is rolled back**: a failure there leaves the state, flags the repository
//!   `needs-recovery` (`delivery_unfinished`) and is completed at the next start.
//!
//! # How the stage takes the live directory's place — and the 503 window
//!
//! The workspace forbids `unsafe`, and `renameat2(RENAME_EXCHANGE)` (one atomic step, no moment without the repository) is reachable
//! only through `libc` (an `unsafe` call) or a crate (`rustix`, `nix`) that is not in the lock. **This version therefore uses two ordinary
//! renames** — the live directory to `<delivery>/previous`, then the stage to the live path — which leaves a **window** (milliseconds:
//! two renames on one filesystem) in which the repository's directory does not exist; a request that opens it then is answered
//! `unavailable` (a 503) and retried by the client. The window is **measured and counted** (`exchange_window` in [`crate::Metrics`], its
//! length in the result of [`DeliveryService::confirm`]), and a kill inside it is recovered (the second rename is completed from the
//! delivery's directory). Replacing the private function `exchange` by `renameat2` removes the window and nothing else.
//!
//! # What this module never does
//!
//! It signs nothing (no `seal`, no `merge`, no `branch close`, no `--allow-non-fast-forward`: a source test holds this), takes no
//! bare path (every entry point takes the repository record and the user), and logs no file, branch or key name and no content.

pub mod rule;

use std::collections::BTreeSet;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use planeter_auth::Sha256Stream;
use planeter_core::change::{Landing, current_patch_set, observe_landing};
use planeter_core::{
    CallLimits, HostingService, Lease, LeaseClass, LeaseError, LeaseManager,
    creation::group_fingerprint,
};
use planeter_prikk::model::TrustedKey;
use planeter_prikk::sandbox::{IncomingBounds, TIMEOUT_EXIT_CODE};
use planeter_prikk::{CliPrikkRepo, PrikkError, PrikkRepo, Sandbox};
use planeter_store::{
    ChangeState, ChangeStore, DeliveryRecord, DeliveryState, DeliveryStore, DeliveryUpdate, RepoId,
    RepositoryRecord, StageLimits, StartRefusal, TransitionError, UserId,
};
use serde::{Deserialize, Serialize};

use crate::artifacts::{ArtifactStore, Digest};
use crate::creation::cost_model;
use crate::export::ExportService;
use crate::metrics::Metrics;
use crate::proposals::FOLLOWED_REF;
use crate::propose::{ProposeConfig, SlotGuard, Slots};
use crate::recovery::RecoveryService;

pub use rule::{Acceptable, LiveFacts, RefFacts, Refusal, StagedFacts, decide};

const MIB: u64 = 1 << 20;

/// The name of the archive inside a delivery's read-only `in` directory.
const ARCHIVE_FILE: &str = "archive";
/// The delivery's scratch repository: the stage. After the exchange it is the repository.
const STAGE_DIR: &str = "repo";
/// Where the previous generation waits between the exchange and the end.
const PREVIOUS_DIR: &str = "previous";
/// The directory, beside the repositories, that holds every delivery's own directory.
pub const DELIVERIES_DIR: &str = ".deliveries";
/// The most blocks of one ref the engine reads (a history longer than this cannot be bounded: `damaged_archive`).
pub const CHAIN_BLOCKS_MAX: usize = 200_000;
/// The flag a repository gets when an install stopped after its exchange.
pub const FLAG_REASON: &str = "a write did not finish (delivery_unfinished)";

// ------------------------------------------------------------------------------------------------ settings and errors

/// Operator settings of the engine: the creation settings it reuses (the archive bound `PLANETER_IMPORT_MAX_MB`, the stage budget, the
/// lifetime, the deadline) and the propose settings its re-accepts run under. **No new setting.**
#[derive(Debug, Clone)]
pub struct DeliveryConfig {
    pub max_archive_bytes: u64,
    /// The wall-time bound of one staging and of one install's prikk calls.
    pub deadline: Duration,
    /// How long a staged delivery lives (24 h).
    pub stage_ttl: Duration,
    pub stage_limits: StageLimits,
    /// How long a start waits for a slot and a confirmation for the write lease before it is `busy`.
    pub lease_wait: Duration,
    pub max_concurrent: usize,
    /// The bounds and the ceiling of the re-accept of one retained artifact (the accept's own).
    pub propose: ProposeConfig,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        let c = crate::creation::CreationConfig::default();
        DeliveryConfig {
            max_archive_bytes: c.max_archive_bytes,
            deadline: c.deadline,
            stage_ttl: c.stage_ttl,
            stage_limits: c.stage_limits,
            lease_wait: c.lease_wait,
            max_concurrent: c.max_concurrent,
            propose: ProposeConfig::default(),
        }
    }
}

/// Why a step of a delivery did not happen. A **rule** refusal is not an error: it is a [`Report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryError {
    /// A slot or the write lease did not come free in time, or a delivery is in flight for the repository (`busy`).
    Busy { retry_after: Duration },
    /// The archive is larger than the bound (`too_large`).
    TooLarge { limit: u64 },
    /// Too many deliveries for this principal, or the byte budget is spent (`stage_limit`).
    StageLimit(StartRefusal),
    /// The repository is flagged `needs-recovery`: writes are refused.
    NeedsRecovery { reason: String },
    /// No such delivery of this user (also: someone else's).
    NotFound,
    /// The delivery's day is over.
    Expired,
    /// The delivery is not in a state that can be confirmed (or reported).
    NotConfirmable(DeliveryState),
    /// The report was a refusal, or the repository moved (its reason is the closed word).
    Failed(String),
    /// A prikk call hit its deadline.
    TimedOut,
    /// A prikk call hit its memory ceiling.
    CeilingExceeded,
    /// The install stopped after its exchange: the state is kept, the repository is flagged, the next start completes it.
    Incomplete { delivery_id: String },
    /// Something the caller cannot fix (logged).
    Unavailable,
}

impl DeliveryError {
    /// A stable word for logs and counters.
    pub fn reason(&self) -> &'static str {
        match self {
            DeliveryError::Busy { .. } => "busy",
            DeliveryError::TooLarge { .. } => "too_large",
            DeliveryError::StageLimit(_) => "stage_limit",
            DeliveryError::NeedsRecovery { .. } => "needs_recovery",
            DeliveryError::NotFound => "not_found",
            DeliveryError::Expired => "expired",
            DeliveryError::NotConfirmable(_) => "not_confirmable",
            DeliveryError::Failed(_) => "failed",
            DeliveryError::TimedOut => "timed_out",
            DeliveryError::CeilingExceeded => "ceiling",
            DeliveryError::Incomplete { .. } => "delivery_unfinished",
            DeliveryError::Unavailable => "unavailable",
        }
    }
}

fn unavailable(what: &str, e: impl std::fmt::Display) -> DeliveryError {
    eprintln!("planeter: delivery: {what}: {e}");
    DeliveryError::Unavailable
}

/// The text of prikk's refusal, trimmed and bounded (it is logged behind our own sentence; never shown as a name list).
fn prikk_text(stderr: &str) -> String {
    let t = stderr.trim();
    let t = t.strip_prefix("error:").map_or(t, str::trim);
    t.chars().take(300).collect()
}

/// How a stage call failed.
#[derive(Debug)]
enum StageFail {
    /// prikk refused the archive (or a report on it could not be bounded): `damaged_archive`.
    Damaged,
    TimedOut,
    Ceiling,
    Unavailable,
}

impl StageFail {
    fn word(&self) -> &'static str {
        match self {
            StageFail::Damaged => "damaged_archive",
            StageFail::TimedOut => "timed_out",
            StageFail::Ceiling => "ceiling",
            StageFail::Unavailable => "unavailable",
        }
    }
}

/// A prikk call that reads the **archive**: exit 1 is the archive's fault.
fn archive_call(id: &str, e: PrikkError) -> StageFail {
    match e {
        PrikkError::Command {
            code: Some(TIMEOUT_EXIT_CODE),
            ..
        } => StageFail::TimedOut,
        PrikkError::Command {
            code: Some(1),
            stderr,
        } => {
            eprintln!(
                "planeter: delivery {id}: prikk refused the archive: {}",
                prikk_text(&stderr)
            );
            StageFail::Damaged
        }
        PrikkError::CeilingExceeded { .. } => StageFail::Ceiling,
        PrikkError::OutputTooLarge { .. } => {
            eprintln!(
                "planeter: delivery {id}: prikk's report on the archive is larger than this server reads"
            );
            StageFail::Damaged
        }
        other => {
            eprintln!("planeter: delivery {id}: an archive call failed: {other}");
            StageFail::Unavailable
        }
    }
}

/// A prikk call that does not read the archive.
fn infra_call(id: &str, e: PrikkError) -> StageFail {
    match e {
        PrikkError::Command {
            code: Some(TIMEOUT_EXIT_CODE),
            ..
        } => StageFail::TimedOut,
        PrikkError::CeilingExceeded { .. } => StageFail::Ceiling,
        other => {
            eprintln!("planeter: delivery {id}: a prikk call failed: {other}");
            StageFail::Unavailable
        }
    }
}

// ------------------------------------------------------------------------------------------------ the report

/// What the report says of the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Acceptable,
    Refused,
}

/// One key the repository trusts, as the report shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyView {
    pub key_id: String,
    pub fingerprint: String,
}

/// The delivery report (RFC 008 D-5). Serialised **once** (into the delivery's row) and read back from there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub verdict: Verdict,
    /// The closed reason of a refusal.
    pub reason: Option<String>,
    pub old_tip: Option<String>,
    pub new_tip: Option<String>,
    pub new_blocks: u64,
    pub refs_added: Vec<String>,
    /// Open proposals that stay open.
    pub proposals_stay: Vec<String>,
    /// Open proposals whose every patch is in a block of the staged `heads/main` chain.
    pub proposals_land: Vec<String>,
    /// Open proposals with a revision whose retained artifact is gone (D-8): they stay readable, their changes can no longer be shown.
    pub proposals_cannot_be_kept: Vec<String>,
    pub trusted_keys: Vec<KeyView>,
    pub archive_bytes: u64,
}

impl Report {
    fn refused(reason: Refusal, keys: Vec<KeyView>, bytes: u64) -> Report {
        Report {
            verdict: Verdict::Refused,
            reason: Some(reason.code().to_owned()),
            old_tip: None,
            new_tip: None,
            new_blocks: 0,
            refs_added: Vec::new(),
            proposals_stay: Vec::new(),
            proposals_land: Vec::new(),
            proposals_cannot_be_kept: Vec::new(),
            trusted_keys: keys,
            archive_bytes: bytes,
        }
    }

    pub fn is_acceptable(&self) -> bool {
        self.verdict == Verdict::Acceptable
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// A delivery as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryView {
    pub delivery_id: String,
    pub repo_id: RepoId,
    pub state: DeliveryState,
    pub reason: Option<String>,
    pub report: Option<Report>,
    pub created_at: u64,
    pub expires_at: u64,
}

/// What a finished install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub delivery_id: String,
    pub old_tip: Option<String>,
    pub new_tip: String,
    /// Revisions accepted again, and revisions skipped because their retained artifact is gone.
    pub reaccepted: u64,
    pub skipped: u64,
    /// How long the repository's directory did not exist (the two renames).
    pub window: Duration,
}

/// What [`DeliveryService::reconcile_at_start`] found and did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// Staged or reported deliveries whose stage was discarded.
    pub discarded: Vec<String>,
    /// Confirmations that had exchanged nothing: abandoned.
    pub abandoned: Vec<String>,
    /// Installs completed from their intent.
    pub completed: Vec<String>,
    /// Installs that could not be completed now (the repository stays flagged).
    pub left: Vec<String>,
    /// Directories no row owns, removed.
    pub swept: Vec<String>,
}

// ------------------------------------------------------------------------------------------------ small pieces

struct Deadline(Instant);

impl Deadline {
    fn after(d: Duration) -> Self {
        Deadline(Instant::now() + d)
    }

    fn remaining_secs(&self) -> Result<u64, StageFail> {
        let left = self.0.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(StageFail::TimedOut)
        } else {
            Ok(left.as_secs().max(1))
        }
    }
}

/// `dl-` and 24 hexadecimal characters: the only shape of a delivery id, so one taken from a request can never name a path.
fn delivery_id_ok(id: &str) -> bool {
    id.len() == 27
        && id.starts_with("dl-")
        && id[3..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn new_delivery_id() -> String {
    let token = planeter_auth::random_token();
    format!("dl-{}", &planeter_auth::sha256_hex(token.as_bytes())[..24])
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    DirBuilder::new().mode(0o700).create(path)
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Make a directory entry change durable: the parent is synced.
fn sync_dir(dir: &Path) {
    if let Ok(f) = File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Put `stage` in the place of `live`, the live directory ending in `previous`. **Two ordinary renames** (see the module documentation);
/// `between` is a test hook called in the window. A failure of the second rename puts the live directory back.
fn exchange(
    live: &Path,
    stage: &Path,
    previous: &Path,
    between: &dyn Fn(),
) -> std::io::Result<Duration> {
    let started = Instant::now();
    fs::rename(live, previous)?;
    between();
    if let Err(e) = fs::rename(stage, live) {
        let _ = fs::rename(previous, live);
        return Err(e);
    }
    let window = started.elapsed();
    if let Some(parent) = live.parent() {
        sync_dir(parent);
    }
    if let Some(parent) = previous.parent() {
        sync_dir(parent);
    }
    Ok(window)
}

/// Give the staged repository's directory the **mode of the live one** before the exchange (RFC 008 handoff 2, T0.2), and refuse
/// when the two directories do not belong to the same user: the stage was made by this server, so a live directory that is someone
/// else's is not one the server may replace.
fn stage_takes_live_mode(live: &Path, stage: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (l, s) = (fs::metadata(live)?, fs::metadata(stage)?);
    if l.uid() != s.uid() {
        return Err(std::io::Error::other(
            "the repository's directory belongs to another user",
        ));
    }
    fs::set_permissions(stage, fs::Permissions::from_mode(l.mode() & 0o7777))
}

fn remove_tree(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
        Ok(m) if m.file_type().is_symlink() => fs::remove_file(path).map(|()| true),
        Ok(m) if m.is_dir() => fs::remove_dir_all(path).map(|()| true),
        Ok(_) => Err(std::io::Error::other("not a directory")),
    }
}

// ------------------------------------------------------------------------------------------------ reading the repositories

/// One ref's chain with what the same `log` shows of its tip and of its patches.
struct Chain {
    facts: RefFacts,
    tip_sealers: Vec<String>,
    patches: BTreeSet<String>,
}

/// The refs of a repository, read whole.
struct RefsRead {
    refs: Vec<RefFacts>,
    tip_sealers: Vec<(String, Vec<String>)>,
    followed_patches: BTreeSet<String>,
    received: Vec<String>,
}

/// The open proposals sorted for the report.
struct Sorted {
    stay: Vec<String>,
    land: Vec<String>,
    cannot: Vec<String>,
}

/// One chain, **read whole**: the tip's `update_seq` gives the first guess of its length and the limit grows until `log` returns fewer
/// blocks than asked (a fixed limit would silently cut a longer history and answer a false "not a successor"). `None` when the length
/// cannot be bounded ([`CHAIN_BLOCKS_MAX`], or a report larger than this server reads).
fn read_chain(driver: &CliPrikkRepo, ref_name: &str) -> Result<Option<Chain>, PrikkError> {
    let first = driver.log(Some(ref_name), Some(1))?;
    let Some(tip) = first.blocks.first() else {
        return Ok(Some(Chain {
            facts: RefFacts {
                name: ref_name.to_owned(),
                blocks: Vec::new(),
            },
            tip_sealers: Vec::new(),
            patches: BTreeSet::new(),
        }));
    };
    let mut limit = (usize::try_from(tip.update_seq).unwrap_or(usize::MAX))
        .saturating_add(2)
        .clamp(64, CHAIN_BLOCKS_MAX);
    loop {
        let log = match driver.log(Some(ref_name), Some(limit)) {
            Ok(l) => l,
            Err(PrikkError::OutputTooLarge { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        if log.blocks.len() < limit {
            let tip_sealers = log
                .blocks
                .first()
                .map(|b| b.sealed_by.clone())
                .unwrap_or_default();
            let patches: BTreeSet<String> = log
                .blocks
                .iter()
                .flat_map(|b| b.patch_messages.iter().map(|p| p.patch_id.clone()))
                .collect();
            let blocks = log.blocks.into_iter().map(|b| b.block_id).collect();
            return Ok(Some(Chain {
                facts: RefFacts {
                    name: ref_name.to_owned(),
                    blocks,
                },
                tip_sealers,
                patches,
            }));
        }
        if limit >= CHAIN_BLOCKS_MAX {
            return Ok(None);
        }
        limit = limit.saturating_mul(4).min(CHAIN_BLOCKS_MAX);
    }
}

/// The published refs of a repository with their whole chains. Closed branches are published refs too. `None` when a chain cannot be bounded.
fn read_refs(driver: &CliPrikkRepo) -> Result<Option<RefsRead>, PrikkError> {
    let branches = driver.branches(true)?;
    let mut out = RefsRead {
        refs: Vec::new(),
        tip_sealers: Vec::new(),
        followed_patches: BTreeSet::new(),
        received: branches
            .received
            .iter()
            .map(|r| r.ref_name.clone())
            .collect(),
    };
    for b in &branches.branches {
        let Some(chain) = read_chain(driver, &b.ref_name)? else {
            return Ok(None);
        };
        if chain.facts.name == FOLLOWED_REF {
            out.followed_patches = chain.patches;
        }
        out.tip_sealers
            .push((chain.facts.name.clone(), chain.tip_sealers));
        out.refs.push(chain.facts);
    }
    Ok(Some(out))
}

// ------------------------------------------------------------------------------------------------ the service

type Hook = Arc<dyn Fn(&str) + Send + Sync>;

/// The delivery engine. Cheap to share (`Arc`); every method is **blocking** (call it from `spawn_blocking`).
pub struct DeliveryService {
    hosting: Arc<HostingService>,
    deliveries: Arc<dyn DeliveryStore>,
    changes: Arc<dyn ChangeStore>,
    artifacts: Arc<ArtifactStore>,
    export: Arc<ExportService>,
    leases: Arc<LeaseManager>,
    metrics: Arc<Metrics>,
    root: PathBuf,
    config: DeliveryConfig,
    slots: Arc<Slots>,
    now_unix: fn() -> u64,
    recovery: Option<Arc<RecoveryService>>,
    /// Test seam: called at named points of an install, to stop or kill the process there. Never set in production.
    hook: Option<Hook>,
}

impl DeliveryService {
    /// A service. Its directory (`.deliveries`) lives **beside the repositories**, on their filesystem, so that a stage can replace a
    /// repository by renaming.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hosting: Arc<HostingService>,
        deliveries: Arc<dyn DeliveryStore>,
        changes: Arc<dyn ChangeStore>,
        artifacts: Arc<ArtifactStore>,
        export: Arc<ExportService>,
        leases: Arc<LeaseManager>,
        metrics: Arc<Metrics>,
        config: DeliveryConfig,
    ) -> std::io::Result<Self> {
        let root = hosting.repos_root().join(DELIVERIES_DIR);
        fs::create_dir_all(hosting.repos_root())?;
        match private_dir(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let root = fs::canonicalize(&root)?;
        let slots = Slots::new(config.max_concurrent);
        Ok(DeliveryService {
            hosting,
            deliveries,
            changes,
            artifacts,
            export,
            leases,
            metrics,
            root,
            config,
            slots,
            now_unix: system_now,
            recovery: None,
            hook: None,
        })
    }

    /// Test seam: the clock.
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// The recovery service whose unattended run clears the stale locks of a killed call, after its witnesses agree (RFC 008 D-7).
    #[must_use]
    pub fn with_recovery(mut self, recovery: Arc<RecoveryService>) -> Self {
        self.recovery = Some(recovery);
        self
    }

    /// Test seam: a function called with the name of each point of an install (`after_report`, `after_intent`, `before_exchange`,
    /// `between_renames`, `after_exchange`, `exchanged`, `reaccept_first`, `reaccept_middle`, `reaccept_last`, `before_done`) and of the stage
    /// (`stage_before_import`).
    #[doc(hidden)]
    #[must_use]
    pub fn with_failpoint(mut self, hook: Hook) -> Self {
        self.hook = Some(hook);
        self
    }

    fn at(&self, point: &str) {
        if let Some(h) = &self.hook {
            h(point);
        }
    }

    pub fn config(&self) -> &DeliveryConfig {
        &self.config
    }

    /// What one staging or one install is admitted for by the host's memory pool (the creation cost model's).
    pub fn admission_cost(bytes: u64) -> u64 {
        cost_model::admission_cost(bytes)
    }

    fn dir_of(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn now(&self) -> u64 {
        (self.now_unix)()
    }

    fn advance(
        &self,
        id: &str,
        from: DeliveryState,
        to: DeliveryState,
        update: DeliveryUpdate,
    ) -> Result<DeliveryRecord, DeliveryError> {
        match self
            .deliveries
            .advance(id, from, to, update, self.now())
            .map_err(|e| unavailable("cannot move the delivery", e))?
        {
            Ok(r) => Ok(r),
            Err(TransitionError::NotFound) => Err(DeliveryError::NotFound),
            Err(TransitionError::Changed { actual }) => Err(DeliveryError::NotConfirmable(actual)),
            Err(TransitionError::Illegal { from, .. }) => Err(DeliveryError::NotConfirmable(from)),
        }
    }

    fn discard(&self, id: &str) {
        if !delivery_id_ok(id) {
            return;
        }
        if let Err(e) = remove_tree(&self.dir_of(id)) {
            eprintln!(
                "planeter: delivery {id}: cannot remove its directory: {e}; the next start removes it"
            );
        }
    }

    // -------------------------------------------------------------------------------- start

    /// Begin a delivery to `record`'s repository by `principal`: **one in flight per repository** (a second is `busy`), the principal's
    /// and the host's stage limits, the archive bound against `expected` before a byte is read, a slot, and the delivery's own
    /// directory (checked to be on the repositories' filesystem). The archive is written into the returned upload.
    pub fn start(
        &self,
        principal: &UserId,
        record: &RepositoryRecord,
        expected: u64,
    ) -> Result<DeliveryUpload, DeliveryError> {
        if expected > self.config.max_archive_bytes {
            self.metrics.delivery("too_large");
            return Err(DeliveryError::TooLarge {
                limit: self.config.max_archive_bytes,
            });
        }
        let flags = self
            .changes
            .repo_flags(&record.repo_id)
            .map_err(|e| unavailable("cannot read the repository's flag", e))?;
        if flags.needs_recovery {
            return Err(DeliveryError::NeedsRecovery {
                reason: flags.reason.unwrap_or_default(),
            });
        }
        // The exchange renames across this directory and the repository's: they must be one filesystem.
        let repo_parent = record.path.parent().unwrap_or(&record.path);
        match (fs::metadata(&self.root), fs::metadata(repo_parent)) {
            (Ok(a), Ok(b)) if a.dev() == b.dev() => {}
            _ => {
                eprintln!(
                    "planeter: delivery: the delivery directory and the repositories are not on one filesystem (or cannot be read): deliveries are unavailable"
                );
                return Err(DeliveryError::Unavailable);
            }
        }
        self.sweep_expired();
        let slot = self.slots.acquire(self.config.lease_wait).ok_or_else(|| {
            self.metrics.delivery("busy");
            DeliveryError::Busy {
                retry_after: self.config.lease_wait,
            }
        })?;
        let id = new_delivery_id();
        let now = self.now();
        let row = DeliveryRecord {
            delivery_id: id.clone(),
            repo_id: record.repo_id.clone(),
            principal: principal.as_str().to_owned(),
            state: DeliveryState::Staged,
            archive_digest: String::new(),
            archive_bytes: expected,
            stage_dir: id.clone(),
            old_tip: None,
            new_tip: None,
            report_json: None,
            reason: None,
            created_at: now,
            updated_at: now,
            finished_at: None,
        };
        match self
            .deliveries
            .start(row, self.config.stage_limits)
            .map_err(|e| unavailable("cannot keep the delivery", e))?
        {
            Ok(()) => {}
            Err(StartRefusal::RepoBusy) => {
                self.metrics.delivery("busy");
                return Err(DeliveryError::Busy {
                    retry_after: self.config.lease_wait,
                });
            }
            Err(StartRefusal::IdTaken) => return Err(DeliveryError::Unavailable),
            Err(other) => {
                self.metrics.delivery("stage_limit");
                return Err(DeliveryError::StageLimit(other));
            }
        }
        let guard = DeliveryGuard {
            store: Arc::clone(&self.deliveries),
            dir: self.dir_of(&id),
            id: id.clone(),
            now: self.now_unix,
            armed: true,
        };
        let made = (|| -> std::io::Result<File> {
            private_dir(&guard.dir)?;
            private_dir(&guard.dir.join("in"))?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(guard.dir.join("in").join(ARCHIVE_FILE))
        })();
        let file = match made {
            Ok(f) => f,
            Err(e) => return Err(unavailable("cannot make the delivery directory", e)),
        };
        Ok(DeliveryUpload {
            guard,
            file: Some(file),
            hash: Sha256Stream::new(),
            written: 0,
            limit: self.config.max_archive_bytes,
            _slot: slot,
        })
    }

    /// Expire the staged and reported deliveries whose day is over; their directories go.
    pub fn sweep_expired(&self) -> usize {
        let now = self.now();
        let rows = match self.deliveries.non_final() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("planeter: delivery: cannot list the deliveries: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for row in rows {
            let expired = matches!(row.state, DeliveryState::Staged | DeliveryState::Reported)
                && row
                    .created_at
                    .saturating_add(self.config.stage_ttl.as_secs())
                    <= now;
            if expired
                && self
                    .advance(
                        &row.delivery_id,
                        row.state,
                        DeliveryState::Expired,
                        DeliveryUpdate::default(),
                    )
                    .is_ok()
            {
                self.discard(&row.delivery_id);
                self.metrics.delivery("expired");
                n += 1;
            }
        }
        n
    }

    // -------------------------------------------------------------------------------- stage and report

    /// Stage the finished upload, apply the rule and keep the report. An acceptable delivery is `reported`; a refusal is `failed` with
    /// its closed reason (its report is kept, its directory is gone); a prikk call that was killed or hit its ceiling fails the delivery
    /// (`timed_out`, `ceiling`, `unavailable`) and is never resumed.
    pub fn finish(
        &self,
        principal: &UserId,
        record: &RepositoryRecord,
        uploaded: Uploaded,
    ) -> Result<Report, DeliveryError> {
        let Uploaded {
            mut guard,
            digest,
            size,
            _slot,
        } = uploaded;
        let id = guard.id.clone();
        let row = self
            .deliveries
            .get(&id)
            .map_err(|e| unavailable("cannot read the delivery", e))?
            .filter(|r| r.principal == principal.as_str() && r.repo_id == record.repo_id)
            .ok_or(DeliveryError::NotFound)?;
        let result = if size == 0 {
            // an empty upload is an archive prikk would refuse: the same closed answer, without a call
            Ok((
                Report::refused(Refusal::DamagedArchive, Vec::new(), 0),
                None,
            ))
        } else {
            self.stage(&row, record, size)
        };
        match result {
            Ok((report, acceptable)) => {
                let json = report.to_json();
                match acceptable {
                    Some(a) => {
                        self.advance(
                            &id,
                            DeliveryState::Staged,
                            DeliveryState::Reported,
                            DeliveryUpdate {
                                old_tip: a.old_tip.clone(),
                                new_tip: Some(a.new_tip.clone()),
                                report_json: Some(json),
                                reason: None,
                            },
                        )?;
                        guard.armed = false; // the stage is kept for the confirmation
                        self.metrics.delivery("accepted_reports");
                        self.at("after_report");
                        eprintln!(
                            "planeter: delivery {id}: acceptable ({} new blocks, {} archive bytes)",
                            report.new_blocks, size
                        );
                    }
                    None => {
                        let reason = report.reason.clone().unwrap_or_default();
                        self.advance(
                            &id,
                            DeliveryState::Staged,
                            DeliveryState::Failed,
                            DeliveryUpdate {
                                report_json: Some(json),
                                reason: Some(reason.clone()),
                                ..DeliveryUpdate::default()
                            },
                        )?;
                        guard.armed = false;
                        self.discard(&id);
                        self.metrics.delivery(&reason);
                        eprintln!("planeter: delivery {id}: refused ({reason})");
                    }
                }
                let _ = digest;
                Ok(report)
            }
            Err(fail) => {
                let word = fail.word();
                let _ = self.advance(
                    &id,
                    DeliveryState::Staged,
                    DeliveryState::Failed,
                    DeliveryUpdate {
                        reason: Some(word.to_owned()),
                        ..DeliveryUpdate::default()
                    },
                );
                guard.armed = false;
                self.discard(&id);
                self.metrics.delivery("failed");
                eprintln!("planeter: delivery {id}: failed ({word})");
                Err(match fail {
                    StageFail::TimedOut => DeliveryError::TimedOut,
                    StageFail::Ceiling => DeliveryError::CeilingExceeded,
                    _ => DeliveryError::Unavailable,
                })
            }
        }
    }

    /// The live repository as the readers show it: the adopted keys and every published ref with its whole chain.
    fn read_live(
        &self,
        record: &RepositoryRecord,
        bytes: u64,
    ) -> Result<(LiveFacts, Vec<TrustedKey>), DeliveryError> {
        let driver = self
            .hosting
            .open_record_with(
                record,
                CallLimits::data_ceiling(cost_model::import_ceiling(bytes.max(MIB))),
            )
            .map_err(|e| unavailable("cannot open the repository", e))?;
        let trust = driver
            .trust_list()
            .map_err(|e| unavailable("cannot read the trust list", e))?;
        let Some(RefsRead { refs, .. }) =
            read_refs(&driver).map_err(|e| unavailable("cannot read the refs", e))?
        else {
            eprintln!("planeter: delivery: a live history is longer than this server reads");
            return Err(DeliveryError::Unavailable);
        };
        Ok((LiveFacts { refs }, trust.keys))
    }

    fn key_views(keys: &[TrustedKey]) -> Vec<KeyView> {
        keys.iter()
            .map(|k| KeyView {
                key_id: k.key_id.clone(),
                fingerprint: group_fingerprint(&k.public_key),
            })
            .collect()
    }

    /// The sandbox of one call that reads the delivery's archive.
    fn sandbox_reading(&self, in_dir: &Path, deadline: &Deadline) -> Result<Sandbox, StageFail> {
        Ok(self
            .hosting
            .sandbox()
            .with_artifact_in(in_dir)
            .with_deadline(deadline.remaining_secs()?))
    }

    /// The staged repository's facts, read by the driver's readers only.
    fn read_staged(
        &self,
        id: &str,
        driver: &CliPrikkRepo,
        adopted: &[String],
    ) -> Result<StagedFacts, StageFail> {
        let Some(RefsRead {
            refs: local,
            tip_sealers,
            followed_patches: followed,
            received,
        }) = read_refs(driver).map_err(|e| infra_call(id, e))?
        else {
            // The length of a staged chain cannot be bounded: the archive is refused (RFC 008 handoff 1, T2).
            return Ok(StagedFacts {
                damaged: true,
                ..StagedFacts::default()
            });
        };
        let verify = driver.verify().map_err(|e| infra_call(id, e))?;
        Ok(StagedFacts {
            damaged: false,
            local,
            received,
            verify_ok: verify.verdict.ok,
            verify_failed: verify
                .verdict
                .failed_conditions
                .iter()
                .map(|c| c.id.clone())
                .collect(),
            publication_trust_issues: verify.publication_trust_issues.len(),
            tip_sealers,
            block_sealers: verify
                .block_seals
                .iter()
                .map(|b| (b.block_id.clone(), b.sealed_by.clone()))
                .collect(),
            followed_patch_ids: followed,
            adopted: adopted.to_vec(),
        })
    }

    /// The stage and the rule. `Ok((report, Some(acceptable)))` or `Ok((report, None))` for a refusal.
    fn stage(
        &self,
        row: &DeliveryRecord,
        record: &RepositoryRecord,
        size: u64,
    ) -> Result<(Report, Option<Acceptable>), StageFail> {
        let id = row.delivery_id.as_str();
        let dir = self.dir_of(id);
        let in_dir = dir.join("in");
        let archive = in_dir.join(ARCHIVE_FILE);
        let scratch = dir.join(STAGE_DIR);
        let deadline = Deadline::after(self.config.deadline);
        let (live, trusted) = self
            .read_live(record, size)
            .map_err(|_| StageFail::Unavailable)?;
        let keys = Self::key_views(&trusted);
        let adopted: Vec<String> = trusted.iter().map(|k| k.key_id.clone()).collect();
        private_dir(&scratch).map_err(|e| {
            eprintln!("planeter: delivery {id}: cannot make the scratch repository: {e}");
            StageFail::Unavailable
        })?;
        let call = |ceiling: u64| -> Result<CliPrikkRepo, StageFail> {
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
            .map_err(|e| infra_call(id, e))?;
        // The stage trusts exactly what the live repository trusts. **Never** a key the archive carries.
        for k in &trusted {
            call(import_ceiling)?
                .trust_maintainer_add(&k.key_id, &k.public_key)
                .map_err(|e| infra_call(id, e))?;
        }
        let refused = |reason: Refusal| (Report::refused(reason, keys.clone(), size), None);
        let verified = match call(verify_ceiling)?.archive_verify(&archive) {
            Ok(v) => v,
            Err(e) => match archive_call(id, e) {
                StageFail::Damaged => return Ok(refused(Refusal::DamagedArchive)),
                other => return Err(other),
            },
        };
        if !verified.passed() {
            return Ok(refused(Refusal::DamagedArchive));
        }
        self.at("stage_before_import");
        match call(import_ceiling)?.archive_import(&archive, true) {
            Ok(_) => {}
            Err(e) => match archive_call(id, e) {
                StageFail::Damaged => return Ok(refused(Refusal::DamagedArchive)),
                other => return Err(other),
            },
        }
        let staged = self.read_staged(id, &call(import_ceiling)?, &adopted)?;
        match decide(&live, &staged) {
            Err(reason) => Ok(refused(reason)),
            Ok(a) => {
                let Sorted { stay, land, cannot } = self
                    .proposals_against(&record.repo_id, &staged.followed_patch_ids)
                    .map_err(|_| StageFail::Unavailable)?;
                let report = Report {
                    verdict: Verdict::Acceptable,
                    reason: None,
                    old_tip: a.old_tip.clone(),
                    new_tip: Some(a.new_tip.clone()),
                    new_blocks: a.new_blocks,
                    refs_added: a.refs_added.clone(),
                    proposals_stay: stay,
                    proposals_land: land,
                    proposals_cannot_be_kept: cannot,
                    trusted_keys: keys,
                    archive_bytes: size,
                };
                Ok((report, Some(a)))
            }
        }
    }

    /// The open proposals sorted into those that stay, those that land and those whose retained artifact is gone. **Landing is the
    /// existing observation's own function** (`current_patch_set`, `observe_landing`) over the blocks of [`FOLLOWED_REF`]; there is no
    /// second definition of "landed".
    fn proposals_against(
        &self,
        repo: &RepoId,
        followed_patch_ids: &BTreeSet<String>,
    ) -> Result<Sorted, DeliveryError> {
        let open = self
            .changes
            .list_changes(repo, Some(ChangeState::Open))
            .map_err(|e| unavailable("cannot list the open proposals", e))?;
        let (mut stay, mut land, mut cannot) = (Vec::new(), Vec::new(), Vec::new());
        for c in open {
            let revisions = self
                .changes
                .revisions_of(&c.change_id)
                .map_err(|e| unavailable("cannot read the revisions", e))?;
            match observe_landing(&current_patch_set(&revisions), followed_patch_ids) {
                Landing::Landed => land.push(c.change_id.clone()),
                Landing::NotYet { .. } => stay.push(c.change_id.clone()),
            }
            let gone = revisions.iter().any(|r| {
                !r.artifact_digest
                    .strip_prefix("sha256:")
                    .and_then(|d| Digest::parse(d).ok())
                    .is_some_and(|d| self.artifacts.contains(&d))
            });
            if gone {
                cannot.push(c.change_id);
            }
        }
        stay.sort();
        land.sort();
        cannot.sort();
        Ok(Sorted { stay, land, cannot })
    }

    /// One delivery of `principal`, as they may read it. Someone else's and one that does not exist are one answer.
    pub fn view(
        &self,
        principal: &UserId,
        delivery_id: &str,
    ) -> Result<DeliveryView, DeliveryError> {
        if !delivery_id_ok(delivery_id) {
            return Err(DeliveryError::NotFound);
        }
        let row = self
            .deliveries
            .get(delivery_id)
            .map_err(|e| unavailable("cannot read the delivery", e))?
            .filter(|r| r.principal == principal.as_str())
            .ok_or(DeliveryError::NotFound)?;
        let report = row
            .report_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Report>(j).ok());
        Ok(DeliveryView {
            delivery_id: row.delivery_id,
            repo_id: row.repo_id,
            state: row.state,
            reason: row.reason,
            report,
            created_at: row.created_at,
            expires_at: row
                .created_at
                .saturating_add(self.config.stage_ttl.as_secs()),
        })
    }

    /// Give up a staged or reported delivery of `principal`: it is `abandoned` and its directory goes. Anything that has begun to
    /// install is not cancellable (nothing after the intent is rolled back).
    pub fn cancel(&self, principal: &UserId, delivery_id: &str) -> Result<(), DeliveryError> {
        let view = self.view(principal, delivery_id)?;
        match view.state {
            DeliveryState::Staged | DeliveryState::Reported => {
                self.advance(
                    delivery_id,
                    view.state,
                    DeliveryState::Abandoned,
                    DeliveryUpdate::default(),
                )?;
                self.discard(delivery_id);
                self.metrics.delivery("abandoned");
                Ok(())
            }
            other => Err(DeliveryError::NotConfirmable(other)),
        }
    }

    /// The last finished delivery of the repository (who, when, old and new tip).
    pub fn last_done(&self, repo: &RepoId) -> Result<Option<DeliveryRecord>, DeliveryError> {
        self.deliveries
            .last_done(repo)
            .map_err(|e| unavailable("cannot read the last delivery", e))
    }

    // -------------------------------------------------------------------------------- confirm

    /// Carry out a reported, acceptable delivery of `principal` (RFC 008 D-7). **The one place that writes.**
    pub fn confirm(
        &self,
        principal: &UserId,
        record: &RepositoryRecord,
        delivery_id: &str,
    ) -> Result<Installed, DeliveryError> {
        if !delivery_id_ok(delivery_id) {
            return Err(DeliveryError::NotFound);
        }
        let row = self
            .deliveries
            .get(delivery_id)
            .map_err(|e| unavailable("cannot read the delivery", e))?
            .filter(|r| r.principal == principal.as_str() && r.repo_id == record.repo_id)
            .ok_or(DeliveryError::NotFound)?;
        if row.state != DeliveryState::Reported {
            return Err(DeliveryError::NotConfirmable(row.state));
        }
        let report: Report = row
            .report_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .ok_or_else(|| {
                unavailable("a reported delivery has no readable report", delivery_id)
            })?;
        if !report.is_acceptable() {
            return Err(DeliveryError::NotConfirmable(row.state));
        }
        if row
            .created_at
            .saturating_add(self.config.stage_ttl.as_secs())
            <= self.now()
        {
            let _ = self.advance(
                delivery_id,
                DeliveryState::Reported,
                DeliveryState::Expired,
                DeliveryUpdate::default(),
            );
            self.discard(delivery_id);
            self.metrics.delivery("expired");
            return Err(DeliveryError::Expired);
        }
        let flags = self
            .changes
            .repo_flags(&record.repo_id)
            .map_err(|e| unavailable("cannot read the repository's flag", e))?;
        if flags.needs_recovery {
            return Err(DeliveryError::NeedsRecovery {
                reason: flags.reason.unwrap_or_default(),
            });
        }
        // The write lease: a conflict leaves the delivery `reported` (it can be confirmed again).
        let lease = match self.leases.acquire(
            &record.repo_id,
            LeaseClass::Write,
            "delivery install",
            self.config.lease_wait,
        ) {
            Ok(l) => l,
            Err(LeaseError::Busy { retry_after, .. }) => {
                self.metrics.delivery("busy");
                return Err(DeliveryError::Busy { retry_after });
            }
            Err(e) => return Err(unavailable("cannot take the write lease", e)),
        };
        let dir = self.dir_of(delivery_id);
        // Re-check under the lease: the live tip is the one the report saw, the staged archive is the one that was checked, and the
        // rule still holds. Nothing has been touched yet: a failure here is `failed` and the live repository is as it was.
        let recheck = (|| -> Result<(), String> {
            let on_disk = dir.join("in").join(ARCHIVE_FILE);
            let digest = digest_of(&on_disk).map_err(|_| "unavailable".to_owned())?;
            if digest != row.archive_digest {
                return Err("unavailable".to_owned());
            }
            let (live, trusted) = self
                .read_live(record, row.archive_bytes)
                .map_err(|_| "unavailable".to_owned())?;
            let live_tip = live
                .refs
                .iter()
                .find(|r| r.name == FOLLOWED_REF)
                .and_then(|r| r.tip().map(str::to_owned));
            if live_tip != row.old_tip {
                return Err("moved".to_owned());
            }
            let adopted: Vec<String> = trusted.iter().map(|k| k.key_id.clone()).collect();
            let driver = self.hosting.driver_in(
                &dir.join(STAGE_DIR),
                self.hosting.sandbox().clone(),
                CallLimits::data_ceiling(cost_model::import_ceiling(row.archive_bytes)),
            );
            let staged = self
                .read_staged(delivery_id, &driver, &adopted)
                .map_err(|f| f.word().to_owned())?;
            match decide(&live, &staged) {
                Ok(a) if Some(a.new_tip.as_str()) == row.new_tip.as_deref() => {
                    // the stage becomes the repository: it must look like it to everyone who opens it
                    stage_takes_live_mode(&record.path, &dir.join(STAGE_DIR)).map_err(|e| {
                        eprintln!(
                            "planeter: delivery {delivery_id}: the stage cannot take the repository's place: {e}"
                        );
                        "exchange_failed".to_owned()
                    })
                }
                Ok(_) => Err("moved".to_owned()),
                Err(r) => Err(r.code().to_owned()),
            }
        })();
        if let Err(word) = recheck {
            let _ = self.advance(
                delivery_id,
                DeliveryState::Reported,
                DeliveryState::Failed,
                DeliveryUpdate {
                    reason: Some(word.clone()),
                    ..DeliveryUpdate::default()
                },
            );
            self.discard(delivery_id);
            self.metrics.delivery("failed");
            eprintln!("planeter: delivery {delivery_id}: failed at the re-check ({word})");
            return Err(DeliveryError::Failed(word));
        }

        // The intent, durable, **before** anything moves.
        self.advance(
            delivery_id,
            DeliveryState::Reported,
            DeliveryState::Confirmed,
            DeliveryUpdate::default(),
        )?;
        self.metrics.delivery("confirmed");
        self.at("after_intent");
        self.at("before_exchange");
        let live = &record.path;
        let window = exchange(live, &dir.join(STAGE_DIR), &dir.join(PREVIOUS_DIR), &|| {
            self.at("between_renames")
        })
        .map_err(|e| {
            // Nothing was exchanged (the first rename failed, or was put back): the confirmation is abandoned, the stage discarded.
            let _ = self.advance(
                delivery_id,
                DeliveryState::Confirmed,
                DeliveryState::Failed,
                DeliveryUpdate {
                    reason: Some("exchange_failed".to_owned()),
                    ..DeliveryUpdate::default()
                },
            );
            self.discard(delivery_id);
            self.metrics.delivery("failed");
            unavailable("cannot put the stage in the repository's place", e)
        })?;
        self.metrics.delivery("exchange_window");
        self.at("after_exchange");
        let row = self.advance(
            delivery_id,
            DeliveryState::Confirmed,
            DeliveryState::Exchanged,
            DeliveryUpdate::default(),
        )?;
        self.at("exchanged");
        let (reaccepted, skipped) = self.complete_install(&lease, record, row.clone())?;
        Ok(Installed {
            delivery_id: delivery_id.to_owned(),
            old_tip: row.old_tip,
            new_tip: row.new_tip.unwrap_or_default(),
            reaccepted,
            skipped,
            window,
        })
    }

    /// From `exchanged` (or `reaccepting`) to `done`. Used by the confirmation and by the start-up completion: **idempotent**.
    fn complete_install(
        &self,
        lease: &Lease,
        record: &RepositoryRecord,
        row: DeliveryRecord,
    ) -> Result<(u64, u64), DeliveryError> {
        let id = row.delivery_id.clone();
        // The revision counter moves again (idempotent for the fingerprint's purpose): the directory under the repository changed.
        if let Err(e) = self.export.write_completed(lease) {
            return Err(self.stop(record, &id, "cannot bump the revision counter", e));
        }
        if row.state == DeliveryState::Exchanged {
            self.advance(
                &id,
                DeliveryState::Exchanged,
                DeliveryState::Reaccepting,
                DeliveryUpdate::default(),
            )?;
        }
        let (reaccepted, skipped) = match self.reaccept_all(record) {
            Ok(v) => v,
            Err(e) => return Err(self.stop(record, &id, "a re-accept failed", e)),
        };
        // The new repository must verify; a publication-trust failure is loud.
        let verdict = self
            .hosting
            .open_record_with(
                record,
                CallLimits::data_ceiling(cost_model::import_ceiling(row.archive_bytes.max(MIB))),
            )
            .and_then(|d| d.verify().map_err(Into::into));
        match verdict {
            Ok(v) if v.verdict.ok => {}
            Ok(v) => {
                let trust = v
                    .verdict
                    .failed_conditions
                    .iter()
                    .any(|c| c.id == "publication-trust");
                let word = if trust {
                    eprintln!(
                        "planeter: delivery {id}: THE DELIVERED REPOSITORY FAILS PUBLICATION TRUST — it was installed and is flagged; an administrator must look"
                    );
                    "publication_trust"
                } else {
                    "verify_failed"
                };
                let _ = self.advance(
                    &id,
                    DeliveryState::Reaccepting,
                    DeliveryState::Failed,
                    DeliveryUpdate {
                        reason: Some(word.to_owned()),
                        ..DeliveryUpdate::default()
                    },
                );
                self.flag(record);
                self.metrics.delivery("failed");
                return Err(DeliveryError::Failed(word.to_owned()));
            }
            Err(e) => {
                return Err(self.stop(record, &id, "cannot verify the delivered repository", e));
            }
        }
        self.at("before_done");
        self.advance(
            &id,
            DeliveryState::Reaccepting,
            DeliveryState::Done,
            DeliveryUpdate::default(),
        )?;
        // Previous generation and stage go; the cache entry goes.
        self.discard(&id);
        self.export.forget_listing(&record.repo_id);
        self.metrics.delivery("done");
        eprintln!(
            "planeter: delivery {id}: done ({reaccepted} revisions accepted again, {skipped} skipped)"
        );
        Ok((reaccepted, skipped))
    }

    fn flag(&self, record: &RepositoryRecord) {
        if let Err(e) = self
            .changes
            .set_needs_recovery(&record.repo_id, FLAG_REASON, self.now())
        {
            eprintln!("planeter: delivery: cannot flag the repository: {e}");
        }
    }

    /// An install stopped after its exchange: the state stays, the repository is flagged, nothing is rolled back.
    fn stop(
        &self,
        record: &RepositoryRecord,
        id: &str,
        what: &str,
        e: impl std::fmt::Display,
    ) -> DeliveryError {
        eprintln!(
            "planeter: delivery {id}: {what}: {e}; the repository is flagged and the next start completes it"
        );
        self.flag(record);
        self.metrics.delivery("incomplete");
        DeliveryError::Incomplete {
            delivery_id: id.to_owned(),
        }
    }

    /// Accept again the retained artifact of **every revision of every open proposal, in order** (idempotent: a patch already sealed
    /// by the delivery is a no-op), with the same driver call and bounds as a proposal's accept. A revision whose artifact is gone is
    /// skipped and counted. A failing accept stops the run.
    fn reaccept_all(&self, record: &RepositoryRecord) -> Result<(u64, u64), String> {
        let mut open = self
            .changes
            .list_changes(&record.repo_id, Some(ChangeState::Open))
            .map_err(|e| e.to_string())?;
        open.sort_by(|a, b| (a.opened_at, &a.change_id).cmp(&(b.opened_at, &b.change_id)));
        let mut revisions = Vec::new();
        for c in &open {
            let mut r = self
                .changes
                .revisions_of(&c.change_id)
                .map_err(|e| e.to_string())?;
            r.sort_by_key(|r| r.seq);
            revisions.extend(r);
        }
        let total = revisions.len();
        let (mut done, mut skipped) = (0u64, 0u64);
        for (i, rev) in revisions.iter().enumerate() {
            if i == 0 {
                self.at("reaccept_first");
            }
            if total > 2 && i == total / 2 {
                self.at("reaccept_middle");
            }
            if i + 1 == total {
                self.at("reaccept_last");
            }
            let Some(file) = rev
                .artifact_digest
                .strip_prefix("sha256:")
                .and_then(|d| Digest::parse(d).ok())
                .and_then(|d| self.artifacts.get(&d))
            else {
                skipped += 1;
                continue;
            };
            let dir = self.artifacts.new_accept_dir().map_err(|e| e.to_string())?;
            {
                let mut src = file;
                let mut out = File::create(dir.artifact()).map_err(|e| e.to_string())?;
                std::io::copy(&mut src, &mut out).map_err(|e| e.to_string())?;
                out.sync_all().map_err(|e| e.to_string())?;
            }
            let cfg = &self.config.propose;
            let ceiling = cfg.accept_ceiling(rev.artifact_bytes);
            let sandbox = self
                .hosting
                .sandbox()
                .with_artifact_in(dir.in_dir())
                .with_spool_out(dir.out_dir(), cfg.accept_deadline.as_secs().max(1))
                .with_incoming_bounds(IncomingBounds {
                    exchange_max_bytes: Some(cfg.max_artifact_bytes),
                    exchange_max_objects: Some(cfg.max_objects),
                    ..IncomingBounds::default()
                });
            let driver = self
                .hosting
                .open_record_sandboxed(record, CallLimits::data_ceiling(ceiling), sandbox)
                .map_err(|e| e.to_string())?;
            driver
                .sync_accept_report(&dir.artifact(), &dir.claims())
                .map_err(|e| e.to_string())?;
            done += 1;
        }
        Ok((done, skipped))
    }

    // -------------------------------------------------------------------------------- recovery

    /// What the filesystem says of an exchange of `row`'s stage and the live directory.
    fn exchange_state(&self, record: &RepositoryRecord, id: &str) -> ExchangeState {
        let dir = self.dir_of(id);
        let live = record.path.join(".prikk").is_dir();
        let stage = dir.join(STAGE_DIR).is_dir();
        let previous = dir.join(PREVIOUS_DIR).is_dir();
        match (live, stage, previous) {
            (true, true, false) => ExchangeState::NotDone,
            (false, true, true) => ExchangeState::InWindow,
            (true, false, true) => ExchangeState::Done,
            _ => ExchangeState::Unknown,
        }
    }

    /// **Run at start, before anything serves a request** (after the proposals' own `reconcile_at_start`). For each delivery not final:
    ///
    /// * `staged`, `reported`: the stage is discarded (the delivery is `abandoned`);
    /// * `confirmed` with nothing exchanged: `abandoned`, the stage discarded; with the exchange done (or half done: the second rename is
    ///   completed): continued as `exchanged`;
    /// * `exchanged`, `reaccepting`: **completed from the intent** — the stale locks of the killed call are cleared by the recovery
    ///   service (`unlock --force` after its witnesses agree, never `--yes`), every open proposal is accepted again (idempotent),
    ///   the repository is verified, the delivery finishes and its flag is cleared;
    ///
    /// then the directories no row owns are removed. Each action is logged and counted (`abandoned`, `recovered`).
    pub fn reconcile_at_start(&self) -> Reconciled {
        let mut out = Reconciled::default();
        let rows = match self.deliveries.non_final() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("planeter: delivery: start-up: cannot list the deliveries: {e}");
                return out;
            }
        };
        for row in rows {
            let id = row.delivery_id.clone();
            let Ok(Some(record)) = self.hosting.record(&row.repo_id) else {
                let _ = self.advance(
                    &id,
                    row.state,
                    DeliveryState::Failed,
                    DeliveryUpdate {
                        reason: Some("repository_gone".to_owned()),
                        ..DeliveryUpdate::default()
                    },
                );
                self.discard(&id);
                continue;
            };
            match row.state {
                DeliveryState::Staged | DeliveryState::Reported => {
                    if self
                        .advance(
                            &id,
                            row.state,
                            DeliveryState::Abandoned,
                            DeliveryUpdate::default(),
                        )
                        .is_ok()
                    {
                        self.discard(&id);
                        self.metrics.delivery("abandoned");
                        eprintln!(
                            "planeter: delivery {id}: start-up: the stage of a {} delivery was discarded",
                            row.state.as_str()
                        );
                        out.discarded.push(id);
                    }
                }
                DeliveryState::Confirmed => match self.exchange_state(&record, &id) {
                    ExchangeState::NotDone => {
                        if self
                            .advance(
                                &id,
                                row.state,
                                DeliveryState::Abandoned,
                                DeliveryUpdate::default(),
                            )
                            .is_ok()
                        {
                            self.discard(&id);
                            self.metrics.delivery("abandoned");
                            eprintln!(
                                "planeter: delivery {id}: start-up: a confirmation that exchanged nothing was abandoned"
                            );
                            out.abandoned.push(id);
                        }
                    }
                    state @ (ExchangeState::InWindow | ExchangeState::Done) => {
                        if state == ExchangeState::InWindow {
                            let dir = self.dir_of(&id);
                            if let Err(e) = fs::rename(dir.join(STAGE_DIR), &record.path) {
                                eprintln!(
                                    "planeter: delivery {id}: start-up: cannot finish the exchange: {e}"
                                );
                                self.flag(&record);
                                out.left.push(id);
                                continue;
                            }
                            if let Some(parent) = record.path.parent() {
                                sync_dir(parent);
                            }
                        }
                        match self.advance(
                            &id,
                            DeliveryState::Confirmed,
                            DeliveryState::Exchanged,
                            DeliveryUpdate::default(),
                        ) {
                            Ok(r) => self.finish_from_intent(&record, r, &mut out),
                            Err(_) => out.left.push(id),
                        }
                    }
                    ExchangeState::Unknown => {
                        eprintln!(
                            "planeter: delivery {id}: start-up: the directories do not show whether the exchange happened; left alone and flagged"
                        );
                        self.flag(&record);
                        out.left.push(id);
                    }
                },
                DeliveryState::Exchanged | DeliveryState::Reaccepting => {
                    self.finish_from_intent(&record, row, &mut out);
                }
                _ => {}
            }
        }
        self.sweep_directories(&mut out);
        out
    }

    /// Complete the unfinished install of `record`'s repository, if there is one (`exchanged` or `reaccepting`): what an administrator's
    /// run does after the recovery service has cleared the flag, and what [`DeliveryService::reconcile_at_start`] does at start.
    /// Returns the delivery's id when one was completed.
    pub fn resume(&self, record: &RepositoryRecord) -> Result<Option<String>, DeliveryError> {
        let Some(row) = self
            .deliveries
            .in_flight(&record.repo_id)
            .map_err(|e| unavailable("cannot read the delivery", e))?
            .filter(|r| {
                matches!(
                    r.state,
                    DeliveryState::Exchanged | DeliveryState::Reaccepting
                )
            })
        else {
            return Ok(None);
        };
        let id = row.delivery_id.clone();
        let lease = match self.leases.acquire(
            &record.repo_id,
            LeaseClass::Write,
            "delivery completion",
            self.config.lease_wait,
        ) {
            Ok(l) => l,
            Err(LeaseError::Busy { retry_after, .. }) => {
                return Err(DeliveryError::Busy { retry_after });
            }
            Err(e) => return Err(unavailable("cannot take the write lease", e)),
        };
        self.complete_install(&lease, record, row)?;
        let _ = self.changes.clear_needs_recovery(&record.repo_id);
        self.metrics.delivery("recovered");
        Ok(Some(id))
    }

    fn finish_from_intent(
        &self,
        record: &RepositoryRecord,
        row: DeliveryRecord,
        out: &mut Reconciled,
    ) {
        let id = row.delivery_id.clone();
        // The killed call may have left locks: flag, let the recovery service clear them if every witness agrees, then take the lease.
        self.flag(record);
        if let Some(recovery) = &self.recovery {
            let _ = recovery.auto(record);
            // the recovery run completes a delivery of its own accord (its completion hook): then there is nothing left to do here
            if self
                .deliveries
                .get(&id)
                .ok()
                .flatten()
                .is_some_and(|r| r.state.is_final())
            {
                eprintln!("planeter: delivery {id}: start-up: completed by the recovery run");
                out.completed.push(id);
                return;
            }
        }
        let lease = match self.leases.acquire(
            &record.repo_id,
            LeaseClass::Write,
            "delivery completion",
            self.config.lease_wait,
        ) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("planeter: delivery {id}: start-up: cannot take the write lease: {e}");
                out.left.push(id);
                return;
            }
        };
        match self.complete_install(&lease, record, row) {
            Ok(_) => {
                let _ = self.changes.clear_needs_recovery(&record.repo_id);
                self.metrics.delivery("recovered");
                eprintln!("planeter: delivery {id}: start-up: completed from its intent");
                out.completed.push(id);
            }
            Err(e) => {
                eprintln!(
                    "planeter: delivery {id}: start-up: not completed ({}): the repository stays flagged",
                    e.reason()
                );
                out.left.push(id);
            }
        }
    }

    /// Remove the delivery directories no non-final row owns. A failed install's directory keeps its **previous generation** (nothing
    /// is rolled back; an administrator may need it), so a directory with a `previous` is left, whatever its row says.
    fn sweep_directories(&self, out: &mut Reconciled) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        let owned: BTreeSet<String> = self
            .deliveries
            .non_final()
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.delivery_id)
            .collect();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !delivery_id_ok(&name) || owned.contains(&name) {
                continue;
            }
            if e.path().join(PREVIOUS_DIR).exists() {
                eprintln!(
                    "planeter: delivery {name}: start-up: the directory keeps a previous generation of a failed install; left for an administrator"
                );
                continue;
            }
            match remove_tree(&e.path()) {
                Ok(_) => {
                    eprintln!(
                        "planeter: delivery {name}: start-up: removed a directory no delivery owns"
                    );
                    out.swept.push(name);
                }
                Err(err) => {
                    eprintln!("planeter: delivery {name}: start-up: cannot remove it: {err}")
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExchangeState {
    NotDone,
    InWindow,
    Done,
    Unknown,
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

/// Abandons a delivery that never reached its report (an upload that was cut off, a request that went away) and removes its directory.
struct DeliveryGuard {
    store: Arc<dyn DeliveryStore>,
    dir: PathBuf,
    id: String,
    now: fn() -> u64,
    armed: bool,
}

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.store.advance(
                &self.id,
                DeliveryState::Staged,
                DeliveryState::Abandoned,
                DeliveryUpdate::default(),
                (self.now)(),
            );
            if let Err(e) = remove_tree(&self.dir) {
                eprintln!(
                    "planeter: delivery {}: could not remove its directory: {e}; the next start removes it",
                    self.id
                );
            }
        }
    }
}

/// An archive being written to a delivery's directory: bounded, hashed as it arrives, never held in memory.
pub struct DeliveryUpload {
    guard: DeliveryGuard,
    file: Option<File>,
    hash: Sha256Stream,
    written: u64,
    limit: u64,
    _slot: SlotGuard,
}

impl DeliveryUpload {
    /// The delivery's id.
    pub fn delivery_id(&self) -> &str {
        &self.guard.id
    }

    /// Append the next bytes. Past the bound is refused.
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), DeliveryError> {
        self.written = self.written.saturating_add(chunk.len() as u64);
        if self.written > self.limit {
            return Err(DeliveryError::TooLarge { limit: self.limit });
        }
        self.hash.update(chunk);
        self.file
            .as_mut()
            .expect("the file lives until finish")
            .write_all(chunk)
            .map_err(|e| unavailable("cannot write the delivered archive", e))
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    /// The upload is complete: synced, and its digest and size recorded on the delivery.
    pub fn finish(self, service: &DeliveryService) -> Result<Uploaded, DeliveryError> {
        let DeliveryUpload {
            guard,
            file,
            hash,
            written,
            _slot,
            ..
        } = self;
        let file = file.expect("the file lives until finish");
        file.sync_all()
            .map_err(|e| unavailable("cannot sync the delivered archive", e))?;
        drop(file);
        let digest = hash.finish_hex().as_str().to_owned();
        let kept = service
            .deliveries
            .set_archive(&guard.id, &digest, written)
            .map_err(|e| unavailable("cannot record the archive", e))?;
        if !kept {
            return Err(DeliveryError::NotFound);
        }
        Ok(Uploaded {
            guard,
            digest,
            size: written,
            _slot,
        })
    }
}

/// A finished upload, waiting to be staged ([`DeliveryService::finish`]).
pub struct Uploaded {
    guard: DeliveryGuard,
    pub digest: String,
    pub size: u64,
    _slot: SlotGuard,
}

impl Uploaded {
    pub fn delivery_id(&self) -> &str {
        &self.guard.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in `prikk` whose `log` answers over a history of `blocks` blocks (tip first), honouring `--limit`.
    fn standin(name: &str, blocks: u64) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("planeter-delivery-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("prikk");
        fs::write(
            &script,
            format!(
                r#"#!/bin/sh
limit=100000000
while [ $# -gt 0 ]; do case "$1" in --limit) limit=$2; shift;; esac; shift; done
awk -v n={blocks} -v limit=$limit 'BEGIN {{
  printf "{{\"schema_version\":\"log-report-v1\",\"repository\":\"x\",\"ref\":\"heads/main\",\"current_branch\":\"heads/main\",\"blocks\":[";
  c = n < limit ? n : limit;
  for (i = 0; i < c; i++) {{
    id = n - i; if (i > 0) printf ",";
    printf "{{\"block_id\":\"b%d\",\"ref_state_id\":\"b%d\",\"update_seq\":%d,\"kind\":\"Normal\",\"rollback_block\":false,\"parent_count\":1,\"patch_count\":0,\"rollback_patch_count\":0,\"required_attestation_count\":0,\"patch_messages\":[{{\"patch_id\":\"p%d\",\"message\":\"m\"}}],\"previous_ref_state_id\":null,\"sealed_by\":[\"k\"]}}", id, id, id - 1, id
  }}
  print "]}}"
}}'
"#
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn the_stage_takes_the_live_directorys_mode_and_refuses_a_directory_of_another_user() {
        use std::os::unix::fs::PermissionsExt;
        let base =
            std::env::temp_dir().join(format!("planeter-delivery-mode-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for mode in [0o750, 0o700, 0o755, 0o2750] {
            let (live, stage) = (base.join("live"), base.join("stage"));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(&live).unwrap();
            fs::create_dir_all(&stage).unwrap();
            fs::set_permissions(&live, fs::Permissions::from_mode(mode)).unwrap();
            fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
            stage_takes_live_mode(&live, &stage).unwrap();
            assert_eq!(
                fs::metadata(&stage).unwrap().mode() & 0o7777,
                fs::metadata(&live).unwrap().mode() & 0o7777,
                "{mode:o}"
            );
        }
        // a live directory of another user (`/usr` is root's; skipped when this runs as root)
        let me = fs::metadata(base.join("stage")).unwrap().uid();
        if fs::metadata("/usr").unwrap().uid() != me {
            assert!(stage_takes_live_mode(Path::new("/usr"), &base.join("stage")).is_err());
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_chain_longer_than_any_fixed_limit_is_read_whole() {
        let script = standin("six-thousand", 6000);
        let driver = CliPrikkRepo::with_binary(script.parent().unwrap(), script.clone())
            .with_sandbox(Sandbox::Unconfined);
        let chain = read_chain(&driver, "heads/main").unwrap().expect("bounded");
        assert_eq!(chain.facts.blocks.len(), 6000, "not cut at a fixed 5,000");
        assert_eq!(chain.facts.tip(), Some("b6000"));
        assert_eq!(chain.facts.blocks.last().map(String::as_str), Some("b1"));
        assert_eq!(chain.tip_sealers, vec!["k".to_owned()]);
        assert_eq!(chain.patches.len(), 6000);
        let _ = fs::remove_dir_all(script.parent().unwrap());
    }

    #[test]
    fn a_chain_that_cannot_be_bounded_is_none_not_a_prefix() {
        // more blocks than the engine reads at all
        let blocks = u64::try_from(CHAIN_BLOCKS_MAX).unwrap() + 1;
        let script = standin("unbounded", blocks);
        let driver = CliPrikkRepo::with_binary(script.parent().unwrap(), script.clone())
            .with_sandbox(Sandbox::Unconfined);
        assert!(read_chain(&driver, "heads/main").unwrap().is_none());
        let _ = fs::remove_dir_all(script.parent().unwrap());
    }
}
