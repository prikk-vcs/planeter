//! The repository/identity records planeter keeps *above* anonymous prikk (RFC 001 D-5): a prikk
//! repository has no name, owner or URL of its own (RFC 145 §7), so planeter assigns them here. The
//! bytes on disk are keyed by the stable [`RepoId`]; owner/name are metadata a rename/transfer changes
//! without moving anything (RFC 001 D-4/D-5).

use std::path::PathBuf;

/// A stable, opaque internal id for a hosted repository. The on-disk `.prikk` is laid out by this id,
/// so renaming or transferring a repository never moves bytes. Allocated by the hosting layer
/// (planeter-core, RFC 001 D-6); the store only persists it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoId(String);

impl RepoId {
    /// Wrap an already-allocated id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The owner of a repository: a user or an organization, referenced by its forge-level handle. (The
/// full account/org model is RFC 002; here an owner is just the reference the URL is built from.)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Owner {
    User(String),
    Org(String),
}

impl Owner {
    /// The owner's handle (the `<owner>` segment of `owner/name`).
    pub fn handle(&self) -> &str {
        match self {
            Owner::User(h) | Owner::Org(h) => h,
        }
    }
}

/// Repository visibility. Stored here as data; *enforced* by the authorization service (RFC 002 D-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Internal,
    Private,
}

/// A hosted repository's forge-level record. `repo_id` and `path` are stable for the repository's life;
/// `owner`/`name` may change (rename/transfer) without touching either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRecord {
    pub repo_id: RepoId,
    pub owner: Owner,
    pub name: String,
    pub visibility: Visibility,
    /// Unix seconds at creation.
    pub created_at: u64,
    /// The prikk on-disk format version this repo is hosted in (RFC 009 durability; may be unknown
    /// until the repo is first read).
    pub prikk_format_version: Option<String>,
    /// The directory containing this repository's `.prikk`. Keyed by `repo_id` (RFC 001 D-4), so it is
    /// invariant under rename/transfer.
    pub path: PathBuf,
}
