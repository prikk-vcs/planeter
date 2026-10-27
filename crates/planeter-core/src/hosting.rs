//! The hosting model (RFC 001 D-4/D-6): creating and opening hosted repositories.
//!
//! planeter hosts *anonymous* prikk repositories (RFC 145 §7): prikk owns the bytes and the history,
//! planeter owns the identity (`owner/name`), the placement on disk, and the metadata record. This
//! module is where the two meet — it allocates a [`RepoId`], lays the repository out under the D-4
//! sharded path, runs `prikk init` **through the sandboxed boundary** ([`planeter_prikk`], the only
//! crate that spawns prikk), and records it in the [`RepositoryStore`]. Opening resolves a record to
//! its path and hands back a version-pinned driver.
//!
//! prikk stays the source of truth (ENF-5): the store holds only identity/placement metadata, never a
//! copy of history. `open` re-derives every repository fact from the on-disk `.prikk` via the driver.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use planeter_prikk::{CallLimits, CliPrikkRepo, PrikkError};
use planeter_store::{
    Owner, ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore, StoreError, Visibility,
};

use crate::layout::{DefaultRepoIdAllocator, RepoIdAllocator, RepoLayout};

/// A hosting-layer failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum HostingError {
    /// The target `owner/name` is already taken.
    NameTaken { owner: String, name: String },
    /// A persistence failure.
    Store(StoreError),
    /// A prikk-boundary failure (spawn, non-zero exit, version pin, parse).
    Prikk(PrikkError),
    /// A filesystem failure laying out the repository directory.
    Io(std::io::Error),
    /// No repository for the given id.
    NotFound,
}

impl std::fmt::Display for HostingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostingError::NameTaken { owner, name } => {
                write!(f, "repository {owner}/{name} already exists")
            }
            HostingError::Store(e) => write!(f, "store: {e}"),
            HostingError::Prikk(e) => write!(f, "prikk: {e}"),
            HostingError::Io(e) => write!(f, "filesystem: {e}"),
            HostingError::NotFound => f.write_str("repository not found"),
        }
    }
}

impl std::error::Error for HostingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HostingError::Store(e) => Some(e),
            HostingError::Prikk(e) => Some(e),
            HostingError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StoreError> for HostingError {
    fn from(e: StoreError) -> Self {
        HostingError::Store(e)
    }
}
impl From<PrikkError> for HostingError {
    fn from(e: PrikkError) -> Self {
        HostingError::Prikk(e)
    }
}
impl From<std::io::Error> for HostingError {
    fn from(e: std::io::Error) -> Self {
        HostingError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, HostingError>;

/// The forge's repository-hosting service. Ties together the on-disk [`RepoLayout`], the id
/// [`RepoIdAllocator`], the metadata [`RepositoryStore`], and the sandboxed prikk boundary.
pub struct HostingService {
    layout: RepoLayout,
    store: Arc<dyn RepositoryStore>,
    allocator: Arc<dyn RepoIdAllocator>,
    /// prikk binary override (default: `prikk` on `PATH`).
    binary: Option<OsString>,
    /// Confinement for every prikk invocation (default: bubblewrap, via `CliPrikkRepo`).
    sandbox: planeter_prikk::Sandbox,
}

impl HostingService {
    /// A service rooted at `repos_root`, persisting to `store`, with the default id allocator and the
    /// default (bubblewrap) sandbox.
    pub fn new(repos_root: impl Into<std::path::PathBuf>, store: Arc<dyn RepositoryStore>) -> Self {
        Self {
            layout: RepoLayout::new(repos_root),
            store,
            allocator: Arc::new(DefaultRepoIdAllocator::new()),
            binary: None,
            sandbox: planeter_prikk::Sandbox::default(),
        }
    }

    /// Override the id allocator (e.g. a ULID/UUID scheme).
    #[must_use]
    pub fn with_allocator(mut self, allocator: Arc<dyn RepoIdAllocator>) -> Self {
        self.allocator = allocator;
        self
    }

    /// Override the prikk binary.
    #[must_use]
    pub fn with_binary(mut self, binary: impl Into<OsString>) -> Self {
        self.binary = Some(binary.into());
        self
    }

    /// Override the sandbox policy (use [`planeter_prikk::Sandbox::Unconfined`] only in dev).
    #[must_use]
    pub fn with_sandbox(mut self, sandbox: planeter_prikk::Sandbox) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// Build a driver bound to `root` with this service's binary and sandbox (no version pin yet).
    fn driver_at(&self, root: &std::path::Path) -> CliPrikkRepo {
        let repo = match &self.binary {
            Some(b) => CliPrikkRepo::with_binary(root, b.clone()),
            None => CliPrikkRepo::new(root),
        };
        repo.with_sandbox(self.sandbox.clone())
    }

    /// Create a new hosted repository under `owner/name`. It is made the way every creation is (RFC 004 part 7, T1): the
    /// name is **reserved** first (a record in the state `creating`, invisible to every read), the directory is laid out
    /// at `<repos-root>/<shard>/<repo_id>` (D-4), `prikk init` runs there through the sandbox, and only then does the
    /// record become `ready` — the **last** step. Fails [`HostingError::NameTaken`] if the identity is taken, checked
    /// **before** any byte is written. A failure at any step removes the repository whole (directory first, then record);
    /// a kill leaves what [`HostingService::sweep_unfinished`] removes at the next start.
    pub fn create_repo(
        &self,
        owner: Owner,
        name: impl Into<String>,
        visibility: Visibility,
    ) -> Result<RepositoryRecord> {
        let creation =
            self.begin_creation(owner, name.into(), visibility, ProposalPolicy::default())?;
        creation.driver().init_configured()?;
        creation.finish()
    }

    /// Begin a creation: reserve the name (checked before anything is written), then lay out the empty directory. The
    /// returned [`Creation`] removes everything it made unless [`Creation::finish`] is called, whatever way it is left
    /// (an error, a panic, an early return).
    pub fn begin_creation(
        &self,
        owner: Owner,
        name: String,
        visibility: Visibility,
        proposals: ProposalPolicy,
    ) -> Result<Creation<'_>> {
        // Reject a taken identity before touching the disk (a reservation counts as taken).
        if self.store.resolve(&owner, &name)?.is_some() {
            return Err(HostingError::NameTaken {
                owner: owner.handle().to_owned(),
                name,
            });
        }
        let repo_id = self.allocator.allocate();
        let path = self.layout.path_for(&repo_id);
        let record = RepositoryRecord {
            repo_id,
            owner: owner.clone(),
            name: name.clone(),
            visibility,
            created_at: now_unix_secs(),
            prikk_format_version: None,
            proposals,
            path,
        };
        match self.store.reserve(record.clone()) {
            Ok(()) => {}
            Err(StoreError::Conflict(_)) => {
                return Err(HostingError::NameTaken {
                    owner: owner.handle().to_owned(),
                    name,
                });
            }
            Err(e) => return Err(e.into()),
        }
        let creation = Creation {
            svc: self,
            record,
            finished: false,
        };
        // On failure here `creation` is dropped and removes the reservation.
        std::fs::create_dir_all(&creation.record.path)?;
        Ok(creation)
    }

    /// Remove a repository that was being created: its directory first (only ever inside the repository root, never through a
    /// link), then its record. Returns whether a directory was there.
    fn remove_unfinished(&self, record: &RepositoryRecord) -> Result<bool> {
        let expected = self.layout.path_for(&record.repo_id);
        if record.path != expected {
            return Err(HostingError::Io(std::io::Error::other(format!(
                "refusing to remove {}: not where repository {} lives",
                record.path.display(),
                record.repo_id
            ))));
        }
        let existed =
            remove_inside(self.layout.repos_root(), &expected).map_err(HostingError::Io)?;
        match self.store.delete(&record.repo_id) {
            Ok(()) | Err(StoreError::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(existed)
    }

    /// The start-up sweep (RFC 004 part 7, T1). A server that was killed while it was creating a repository leaves a record in
    /// the state `creating`, and perhaps a directory. This removes **every** `creating` repository (directory, then record)
    /// and every directory under the repository root that has no record at all, and **logs each removal**.
    ///
    /// The guards, because this deletes directories:
    ///
    /// * it only ever removes a path **inside** the repository root, resolved through the file system, and a link is never
    ///   followed (a link is unlinked for a `creating` repository and left alone otherwise);
    /// * it descends only into entries named like a shard (two characters of `0-9a-z_`) and removes only entries named like a
    ///   repository id (`0-9a-z`, starting with the shard): anything else in the root is not planeter's and is left;
    /// * it removes a record's directory only where the layout puts that record's id;
    /// * **an empty store with directories on disk is not swept** (that is a wrong database, not an unfinished creation).
    pub fn sweep_unfinished(&self) -> Result<SweepReport> {
        let mut report = SweepReport::default();
        for record in self.store.creating()? {
            match self.remove_unfinished(&record) {
                Ok(existed) => {
                    eprintln!(
                        "planeter: start-up sweep: removed the unfinished creation of {}/{} ({}){}",
                        record.owner.handle(),
                        record.name,
                        record.repo_id,
                        if existed { " and its directory" } else { "" }
                    );
                    report.removed_creating.push(record.repo_id.clone());
                }
                Err(e) => {
                    eprintln!(
                        "planeter: start-up sweep: could not remove the unfinished creation {}: {e}",
                        record.repo_id
                    );
                    report
                        .refused
                        .push((self.layout.path_for(&record.repo_id), e.to_string()));
                }
            }
        }
        let known: std::collections::HashSet<String> = self
            .store
            .known_ids()?
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        let root = self.layout.repos_root();
        let Ok(shards) = std::fs::read_dir(root) else {
            return Ok(report);
        };
        let mut orphans: Vec<PathBuf> = Vec::new();
        for shard in shards.flatten() {
            let name = shard.file_name().to_string_lossy().into_owned();
            let Ok(kind) = shard.file_type() else {
                continue;
            };
            if !is_shard_name(&name) {
                continue;
            }
            if !kind.is_dir() {
                report.refused.push((
                    shard.path(),
                    "not a directory (a link or a file is left alone)".to_owned(),
                ));
                continue;
            }
            let Ok(children) = std::fs::read_dir(shard.path()) else {
                continue;
            };
            for child in children.flatten() {
                let id = child.file_name().to_string_lossy().into_owned();
                if known.contains(&id) || !is_id_name(&id) || !id.starts_with(name.as_str()) {
                    continue;
                }
                let Ok(kind) = child.file_type() else {
                    continue;
                };
                if !kind.is_dir() {
                    report.refused.push((
                        child.path(),
                        "not a directory (a link or a file is left alone)".to_owned(),
                    ));
                    continue;
                }
                orphans.push(child.path());
            }
        }
        if known.is_empty() && !orphans.is_empty() {
            eprintln!(
                "planeter: start-up sweep: {} director{} under {} belong to no repository, but the store holds none at all; \
                 that looks like a different database, so nothing was removed",
                orphans.len(),
                if orphans.len() == 1 { "y" } else { "ies" },
                root.display()
            );
            report.kept_because_store_is_empty = orphans;
            return Ok(report);
        }
        for dir in orphans {
            match remove_inside(root, &dir) {
                Ok(_) => {
                    eprintln!(
                        "planeter: start-up sweep: removed {}, a directory no repository record owns",
                        dir.display()
                    );
                    report.removed_orphans.push(dir);
                }
                Err(e) => {
                    eprintln!(
                        "planeter: start-up sweep: could not remove {}: {e}",
                        dir.display()
                    );
                    report.refused.push((dir, e.to_string()));
                }
            }
        }
        Ok(report)
    }

    /// Open a hosted repository by id: resolve its record, then hand back a **version-pinned** driver
    /// bound to its on-disk `.prikk` (RFC 001 D-3). Every fact the caller reads comes from prikk, not
    /// the store (ENF-5).
    pub fn open(&self, repo_id: &RepoId) -> Result<CliPrikkRepo> {
        let record = self.store.get(repo_id)?.ok_or(HostingError::NotFound)?;
        Ok(self.driver_at(&record.path).checked()?)
    }

    /// Open by `owner/name` (resolve → open).
    pub fn open_by_name(&self, owner: &Owner, name: &str) -> Result<CliPrikkRepo> {
        let repo_id = self
            .store
            .resolve(owner, name)?
            .ok_or(HostingError::NotFound)?;
        self.open(&repo_id)
    }

    /// The record for a repository id (metadata only; no prikk read).
    pub fn record(&self, repo_id: &RepoId) -> Result<Option<RepositoryRecord>> {
        Ok(self.store.get(repo_id)?)
    }

    /// The record for an `owner/name` (metadata only; no prikk read). Lets a caller authorize on the
    /// repository's visibility **before** opening prikk (RFC 003 read gate).
    pub fn record_by_name(&self, owner: &Owner, name: &str) -> Result<Option<RepositoryRecord>> {
        match self.store.resolve(owner, name)? {
            Some(repo_id) => Ok(self.store.get(&repo_id)?),
            None => Ok(None),
        }
    }

    /// A driver bound to `root` — a directory that is **not** a hosted repository (the scratch repository of a staged
    /// archive) — with this service's prikk binary, under `sandbox` and `limits`. Not version-pinned: the caller's first
    /// call (`init_configured`) pins it.
    pub fn driver_in(
        &self,
        root: &std::path::Path,
        sandbox: planeter_prikk::Sandbox,
        limits: CallLimits,
    ) -> CliPrikkRepo {
        self.driver_at(root)
            .with_sandbox(sandbox)
            .with_limits(limits)
    }

    /// The sandbox policy every prikk call of this service runs under.
    pub fn sandbox(&self) -> &planeter_prikk::Sandbox {
        &self.sandbox
    }

    /// [`HostingService::open_record_with`] under a **different sandbox** (the transport's exports mount the
    /// spool read-write and have their own deadline).
    pub fn open_record_sandboxed(
        &self,
        record: &RepositoryRecord,
        limits: CallLimits,
        sandbox: planeter_prikk::Sandbox,
    ) -> Result<CliPrikkRepo> {
        let repo = match &self.binary {
            Some(b) => CliPrikkRepo::with_binary(&record.path, b.clone()),
            None => CliPrikkRepo::new(&record.path),
        };
        Ok(repo.with_sandbox(sandbox).with_limits(limits).checked()?)
    }

    /// Open a driver bound to a record's path with this service's binary/sandbox, version-pinned. Lets
    /// the read path open a repository it has already fetched and authorized (avoids a second lookup).
    pub fn open_record(&self, record: &RepositoryRecord) -> Result<CliPrikkRepo> {
        self.open_record_with(record, CallLimits::NONE)
    }

    /// [`HostingService::open_record`] with per-call `limits` (a memory ceiling, RFC 011 D-2) applied to
    /// every prikk invocation made through the returned driver — including the version pin.
    pub fn open_record_with(
        &self,
        record: &RepositoryRecord,
        limits: CallLimits,
    ) -> Result<CliPrikkRepo> {
        Ok(self.driver_at(&record.path).with_limits(limits).checked()?)
    }
}

/// A repository being created: its name reserved and its directory laid out, not yet visible to anyone. **Unless
/// [`Creation::finish`] is called it is removed whole when dropped** — directory, then record — so no error path, early
/// return or panic can leave a half-made repository behind. (A kill leaves it to [`HostingService::sweep_unfinished`].)
pub struct Creation<'a> {
    svc: &'a HostingService,
    record: RepositoryRecord,
    finished: bool,
}

impl Creation<'_> {
    /// The record (state `creating`: it resolves to nothing for any reader).
    pub fn record(&self) -> &RepositoryRecord {
        &self.record
    }

    /// The repository's directory (it exists; `.prikk` is not there until `init` runs).
    pub fn path(&self) -> &Path {
        &self.record.path
    }

    /// A driver bound to the new directory, with this service's binary and sandbox (no version pin yet).
    pub fn driver(&self) -> CliPrikkRepo {
        self.svc.driver_at(&self.record.path)
    }

    /// The same under another sandbox and per-call limits (an import mounts the archive's directory, and has a deadline and a
    /// memory ceiling of its own).
    pub fn driver_sandboxed(
        &self,
        sandbox: planeter_prikk::Sandbox,
        limits: CallLimits,
    ) -> CliPrikkRepo {
        self.svc
            .driver_at(&self.record.path)
            .with_sandbox(sandbox)
            .with_limits(limits)
    }

    /// The last step: the repository becomes `ready` and visible. On failure nothing is left (the drop removes it).
    pub fn finish(mut self) -> Result<RepositoryRecord> {
        self.svc.store.mark_ready(&self.record.repo_id)?;
        self.finished = true;
        Ok(self.record.clone())
    }

    /// Remove everything now and say whether it worked (a drop does the same and cannot say).
    pub fn abort(mut self) -> Result<()> {
        self.finished = true;
        self.svc.remove_unfinished(&self.record).map(drop)
    }
}

impl std::fmt::Debug for Creation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Creation")
            .field("record", &self.record)
            .field("finished", &self.finished)
            .finish()
    }
}

impl Drop for Creation<'_> {
    fn drop(&mut self) {
        if !self.finished
            && let Err(e) = self.svc.remove_unfinished(&self.record)
        {
            eprintln!(
                "planeter: could not remove the unfinished repository {}: {e}; the next start removes it",
                self.record.repo_id
            );
        }
    }
}

/// What the start-up sweep did ([`HostingService::sweep_unfinished`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Repositories that were `creating`, removed (directory and record).
    pub removed_creating: Vec<RepoId>,
    /// Directories under the repository root that no record owns, removed.
    pub removed_orphans: Vec<PathBuf>,
    /// Paths the sweep would not touch, with the reason (a link, a file, a directory out of place, a removal that failed).
    pub refused: Vec<(PathBuf, String)>,
    /// Directories no record owns that were **kept** because the store holds no repository at all.
    pub kept_because_store_is_empty: Vec<PathBuf>,
}

fn is_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_id_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Remove the directory `target` if it exists, **only if it lies inside `root`**. The parent is resolved through the file
/// system (a shard that is a link to somewhere else resolves outside the root and is refused); the last component is looked
/// at without following it: a link is unlinked, never entered; a directory is removed with `remove_dir_all`, which does not
/// follow links either. Returns whether something was removed.
fn remove_inside(root: &Path, target: &Path) -> std::io::Result<bool> {
    let Ok(root_real) = std::fs::canonicalize(root) else {
        return Ok(false);
    };
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(std::io::Error::other("a path without a parent"));
    };
    let parent_real = match std::fs::canonicalize(parent) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    // Ours always lives at `<root>/<shard>/<id>`: the parent must be strictly inside the root (a shard that is a link to somewhere
    // else resolves outside it; a path directly under the root is not a repository).
    if !parent_real.starts_with(&root_real) || parent_real == root_real {
        return Err(std::io::Error::other(format!(
            "refusing to remove {}: it is not inside {}",
            target.display(),
            root.display()
        )));
    }
    let victim = parent_real.join(name);
    match std::fs::symlink_metadata(&victim) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(&victim).map(|()| true),
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&victim).map(|()| true),
        Ok(_) => Err(std::io::Error::other(format!(
            "refusing to remove {}: not a directory",
            victim.display()
        ))),
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
