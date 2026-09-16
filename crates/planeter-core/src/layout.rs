//! Repository-id allocation and the on-disk layout (RFC 001 D-4/D-6).
//!
//! A hosted repository lives at `<repos-root>/<shard>/<repo_id>/` (the directory that holds prikk's
//! `.prikk`). The path is keyed by the stable, opaque [`RepoId`], never by `owner/name` — so a
//! rename/transfer changes only the metadata record and never moves bytes (D-4/D-5). The `<shard>`
//! (a short prefix of the id) keeps any single directory from accumulating unboundedly many entries.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use planeter_store::RepoId;

/// Allocates stable, opaque repository ids (RFC 001 D-6). A trait so the scheme is swappable (a
/// production deployment may prefer ULIDs/UUIDs); the store only persists whatever this yields.
pub trait RepoIdAllocator: Send + Sync {
    /// Produce a fresh id. Must be unique within a deployment and contain only lowercase
    /// `[0-9a-z]` (so it is safe as a path component and URL-opaque).
    fn allocate(&self) -> RepoId;
}

/// The default allocator: a lowercase base-36 render of `<unix-nanos><pid><counter>`. Unique within a
/// process (the monotonic counter), and across processes/restarts (nanos + pid) with negligible
/// collision risk. Opaque and path-safe. Swap it behind [`RepoIdAllocator`] for a ULID/UUID scheme.
#[derive(Debug, Default)]
pub struct DefaultRepoIdAllocator {
    counter: AtomicU64,
}

impl DefaultRepoIdAllocator {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RepoIdAllocator for DefaultRepoIdAllocator {
    fn allocate(&self) -> RepoId {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = u128::from(std::process::id());
        let n = u128::from(self.counter.fetch_add(1, Ordering::Relaxed));
        // A fixed-width blend so ids sort roughly by time yet stay collision-safe across processes.
        let blended = nanos
            .wrapping_mul(1_000_003)
            .wrapping_add(pid.wrapping_mul(1_000_033))
            .wrapping_add(n);
        RepoId::new(to_base36(blended))
    }
}

/// Render a `u128` as lowercase base-36 (`0-9a-z`).
fn to_base36(mut v: u128) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if v == 0 {
        return "0".to_owned();
    }
    let mut buf = Vec::new();
    while v > 0 {
        buf.push(DIGITS[(v % 36) as usize]);
        v /= 36;
    }
    buf.reverse();
    // Safe: DIGITS is ASCII.
    String::from_utf8(buf).expect("base36 is ASCII")
}

/// Maps a [`RepoId`] to its on-disk directory under a fixed repositories root (RFC 001 D-4).
#[derive(Debug, Clone)]
pub struct RepoLayout {
    repos_root: PathBuf,
}

impl RepoLayout {
    /// Layout rooted at `repos_root` (the parent directory of all hosted repositories).
    pub fn new(repos_root: impl Into<PathBuf>) -> Self {
        Self {
            repos_root: repos_root.into(),
        }
    }

    pub fn repos_root(&self) -> &Path {
        &self.repos_root
    }

    /// The directory that holds this repository's `.prikk`: `<repos-root>/<shard>/<repo_id>`.
    pub fn path_for(&self, repo_id: &RepoId) -> PathBuf {
        self.repos_root
            .join(shard_of(repo_id))
            .join(repo_id.as_str())
    }
}

/// The shard segment for an id: its first two characters, or `"__"` for a shorter id. Opaque ids from
/// [`DefaultRepoIdAllocator`] are always longer, so `"__"` is only a defensive fallback.
fn shard_of(repo_id: &RepoId) -> String {
    let s = repo_id.as_str();
    if s.len() >= 2 {
        s[..2].to_owned()
    } else {
        "__".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocated_ids_are_unique_lowercase_alnum() {
        let alloc = DefaultRepoIdAllocator::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = alloc.allocate();
            let s = id.as_str().to_owned();
            assert!(!s.is_empty());
            assert!(
                s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
                "id not path-safe base36: {s:?}"
            );
            assert!(seen.insert(s.clone()), "duplicate id {s:?}");
        }
    }

    #[test]
    fn path_is_sharded_by_id_prefix() {
        let layout = RepoLayout::new("/srv/repos");
        let id = RepoId::new("abcd1234");
        assert_eq!(
            layout.path_for(&id),
            PathBuf::from("/srv/repos/ab/abcd1234")
        );
    }

    #[test]
    fn path_depends_only_on_id_not_owner_or_name() {
        // The invariant behind D-4/D-5: the path is a pure function of the id.
        let layout = RepoLayout::new("/srv/repos");
        let id = RepoId::new("zzzzzz");
        assert_eq!(layout.path_for(&id), layout.path_for(&id));
    }

    #[test]
    fn base36_renders_known_values() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }
}
