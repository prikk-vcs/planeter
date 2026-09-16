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
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use planeter_prikk::{CliPrikkRepo, PrikkError};
use planeter_store::{Owner, RepoId, RepositoryRecord, RepositoryStore, StoreError, Visibility};

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

    /// Create a new hosted repository under `owner/name`. Allocates a stable id, lays the repository
    /// out at `<repos-root>/<shard>/<repo_id>` (D-4), runs `prikk init` there through the sandbox, and
    /// records it. Fails [`HostingError::NameTaken`] if the identity is taken — checked **before** any
    /// bytes are written. On a later failure the freshly-created directory is removed (best effort) so a
    /// failed create leaves no orphan.
    pub fn create_repo(
        &self,
        owner: Owner,
        name: impl Into<String>,
        visibility: Visibility,
    ) -> Result<RepositoryRecord> {
        let name = name.into();

        // Reject a taken identity before touching the disk.
        if self.store.resolve(&owner, &name)?.is_some() {
            return Err(HostingError::NameTaken {
                owner: owner.handle().to_owned(),
                name,
            });
        }

        let repo_id = self.allocator.allocate();
        let path = self.layout.path_for(&repo_id);

        std::fs::create_dir_all(&path)?;

        // `prikk init` through the boundary (version-pinned + sandboxed). Clean up on failure.
        if let Err(e) = self.driver_at(&path).init_configured() {
            let _ = std::fs::remove_dir_all(&path);
            return Err(e.into());
        }

        let record = RepositoryRecord {
            repo_id,
            owner,
            name,
            visibility,
            created_at: now_unix_secs(),
            prikk_format_version: None,
            path: path.clone(),
        };

        if let Err(e) = self.store.create(record.clone()) {
            let _ = std::fs::remove_dir_all(&path);
            return Err(e.into());
        }

        Ok(record)
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
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
