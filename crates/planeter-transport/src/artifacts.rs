//! The artifact store and the export spool (RFC 004 D-2).
//!
//! Two directories, both **content-addressed** by the SHA-256 of their bytes, both written **atomically**
//! (`<name>.partial` → fsync → rename), both under operator-configured paths outside every repository:
//!
//! * the [`ArtifactStore`] keeps **accepted** artifacts. It has no way to delete or evict: nothing here removes
//!   an object (the type has no such method), so retained artifacts cannot be lost to a disk-budget decision;
//! * the [`Spool`] keeps **exports** (what a client will download). It has a disk **budget** and evicts its
//!   **least recently used** objects to stay inside it; it cannot touch the store, because it only ever
//!   operates in its own directory and the configuration refuses overlapping directories.
//!
//! Layout inside a directory: `objects/<first two hex>/<64 hex>` for finished objects; `incoming/<unique>.partial`
//! for writes in progress. A crash can leave only the second kind; opening a directory removes them. There is no
//! export or accept logic here: this is the primitive those use.
//!
//! **Limits, plainly.** Digests are checked on write, not on every read. The spool's recency order survives a
//! restart through file modification times (best effort); two spools on one directory are not supported (the
//! server is one process, RFC 004 D-7). The tests simulate a crash by stopping before the rename and reopening;
//! they do not cut power.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use planeter_auth::Sha256Stream;

const CHUNK: usize = 64 * 1024;

// ---------------------------------------------------------------------------------------------------- Digest

/// A SHA-256 digest: 64 lowercase hexadecimal characters. Anything else is refused, so a digest taken from
/// outside can never name a path outside the store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    /// Validate a digest string.
    pub fn parse(s: &str) -> Result<Digest, StoreError> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Digest(s.to_owned()))
        } else {
            Err(StoreError::BadDigest(s.chars().take(80).collect()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------------------------------- errors

/// What can go wrong in the store or the spool.
#[derive(Debug)]
pub enum StoreError {
    /// The string is not a SHA-256 hex digest.
    BadDigest(String),
    /// The object is larger than the whole spool budget (or than the limit given to `put`).
    TooLarge { limit: u64 },
    /// A file operation failed.
    Io {
        context: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::BadDigest(s) => write!(f, "not a SHA-256 digest: {s:?}"),
            StoreError::TooLarge { limit } => {
                write!(f, "the object is larger than the allowed {limit} bytes")
            }
            StoreError::Io { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for StoreError {}

fn io_err(context: &'static str) -> impl FnOnce(io::Error) -> StoreError {
    move |source| StoreError::Io { context, source }
}

// ---------------------------------------------------------------------------------------------------- shared

/// What both kinds of directory share: the layout, the atomic write, the lookup.
struct Dir {
    root: PathBuf,
}

static UNIQUE: AtomicU64 = AtomicU64::new(0);

impl Dir {
    /// Create the layout and remove the leftovers of a crashed write (`*.partial`). Returns the directory and
    /// how many partials were removed.
    fn open(root: &Path) -> Result<(Dir, usize), StoreError> {
        fs::create_dir_all(root.join("objects"))
            .map_err(io_err("cannot create the objects directory"))?;
        fs::create_dir_all(root.join("incoming"))
            .map_err(io_err("cannot create the incoming directory"))?;
        let mut removed = 0;
        for e in fs::read_dir(root.join("incoming"))
            .map_err(io_err("cannot read the incoming directory"))?
        {
            let e = e.map_err(io_err("cannot read the incoming directory"))?;
            if e.file_name().to_string_lossy().ends_with(".partial") {
                fs::remove_file(e.path())
                    .map_err(io_err("cannot remove an orphaned partial file"))?;
                removed += 1;
            }
        }
        Ok((
            Dir {
                root: root.to_path_buf(),
            },
            removed,
        ))
    }

    fn object_path(&self, d: &Digest) -> PathBuf {
        self.root.join("objects").join(&d.0[..2]).join(&d.0)
    }

    fn partial_path(&self) -> PathBuf {
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        self.root
            .join("incoming")
            .join(format!("{}-{n}.partial", std::process::id()))
    }

    /// Stream `reader` into a partial file, hashing as it goes; at most `limit` bytes. Returns the partial's
    /// path, the digest and the size. On any error the partial is removed.
    fn write_partial(
        &self,
        reader: &mut dyn Read,
        limit: u64,
    ) -> Result<(PathBuf, Digest, u64), StoreError> {
        let path = self.partial_path();
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(io_err("cannot create the partial file"))?;
            let mut hash = Sha256Stream::new();
            let mut size: u64 = 0;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = reader
                    .read(&mut buf)
                    .map_err(io_err("cannot read the artifact"))?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > limit {
                    return Err(StoreError::TooLarge { limit });
                }
                hash.update(&buf[..n]);
                file.write_all(&buf[..n])
                    .map_err(io_err("cannot write the partial file"))?;
            }
            file.sync_all()
                .map_err(io_err("cannot sync the partial file"))?;
            Ok((hash.finish_hex(), size))
        })();
        match result {
            Ok((hex, size)) => {
                let digest = Digest(hex);
                Ok((path, digest, size))
            }
            Err(e) => {
                let _ = fs::remove_file(&path);
                Err(e)
            }
        }
    }

    /// Make the partial the object `digest`. If the object already exists the partial is dropped (the bytes
    /// are the same by construction): two writers of one digest converge on one file. Returns whether this
    /// call created the object.
    fn publish(&self, partial: &Path, digest: &Digest) -> Result<bool, StoreError> {
        let target = self.object_path(digest);
        let shard = target.parent().expect("an object path has a parent");
        fs::create_dir_all(shard).map_err(io_err("cannot create the object directory"))?;
        if target.exists() {
            let _ = fs::remove_file(partial);
            return Ok(false);
        }
        // rename(2) replaces atomically; a concurrent writer of the same digest installs identical bytes.
        fs::rename(partial, &target).map_err(io_err("cannot move the object into place"))?;
        // Make the rename itself durable.
        if let Ok(dir) = File::open(shard) {
            let _ = dir.sync_all();
        }
        Ok(true)
    }

    fn open_object(&self, d: &Digest) -> Option<File> {
        File::open(self.object_path(d)).ok()
    }

    fn contains(&self, d: &Digest) -> bool {
        self.object_path(d).is_file()
    }

    /// Every finished object: (digest, size, modified).
    fn scan(&self) -> Result<Vec<(Digest, u64, SystemTime)>, StoreError> {
        let mut out = Vec::new();
        for shard in fs::read_dir(self.root.join("objects"))
            .map_err(io_err("cannot read the objects directory"))?
        {
            let shard = shard.map_err(io_err("cannot read the objects directory"))?;
            if !shard.path().is_dir() {
                continue;
            }
            for obj in
                fs::read_dir(shard.path()).map_err(io_err("cannot read an object directory"))?
            {
                let obj = obj.map_err(io_err("cannot read an object directory"))?;
                let name = obj.file_name().to_string_lossy().into_owned();
                let Ok(digest) = Digest::parse(&name) else {
                    continue;
                };
                let meta = obj.metadata().map_err(io_err("cannot stat an object"))?;
                out.push((
                    digest,
                    meta.len(),
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                ));
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------------- store

/// The accepted-artifact store: content-addressed, atomic, **never evicting**. It has no delete method.
pub struct ArtifactStore {
    dir: Dir,
    /// Partial files removed when this store was opened (a crash's leftovers).
    cleaned: usize,
}

impl ArtifactStore {
    /// Open (creating) the store at `root`, removing orphaned `.partial` files.
    pub fn open(root: impl AsRef<Path>) -> Result<ArtifactStore, StoreError> {
        let (dir, cleaned) = Dir::open(root.as_ref())?;
        Ok(ArtifactStore { dir, cleaned })
    }

    /// How many orphaned `.partial` files opening removed.
    pub fn cleaned_partials(&self) -> usize {
        self.cleaned
    }

    /// Store the bytes of `reader`; returns their digest. Storing bytes that are already there changes nothing.
    pub fn put(&self, reader: &mut dyn Read) -> Result<Digest, StoreError> {
        let (partial, digest, _) = self.dir.write_partial(reader, u64::MAX)?;
        self.dir.publish(&partial, &digest)?;
        Ok(digest)
    }

    /// Open an object for reading, or `None` if it is not stored.
    pub fn get(&self, digest: &Digest) -> Option<File> {
        self.dir.open_object(digest)
    }

    pub fn contains(&self, digest: &Digest) -> bool {
        self.dir.contains(digest)
    }

    /// The digests stored, in no particular order.
    pub fn list(&self) -> Result<Vec<Digest>, StoreError> {
        Ok(self.dir.scan()?.into_iter().map(|(d, _, _)| d).collect())
    }

    /// The directory (for the sandbox's read-only `artifact-in` mount).
    pub fn root(&self) -> &Path {
        &self.dir.root
    }

    /// Test seam: write the bytes but stop before the rename, as a crash would.
    #[cfg(test)]
    fn put_until_rename(&self, reader: &mut dyn Read) -> Result<PathBuf, StoreError> {
        Ok(self.dir.write_partial(reader, u64::MAX)?.0)
    }
}

// ---------------------------------------------------------------------------------------------------- spool

struct SpoolState {
    /// size and recency tick (higher = more recently used) of every object.
    entries: HashMap<Digest, (u64, u64)>,
    total: u64,
    tick: u64,
}

/// The export spool: content-addressed, atomic, bounded by a disk budget, evicting **least recently used**.
pub struct Spool {
    dir: Dir,
    budget: u64,
    state: Mutex<SpoolState>,
    cleaned: usize,
}

impl Spool {
    /// Open (creating) the spool at `root` with a budget of `budget_bytes`. Orphaned `.partial` files are
    /// removed; objects already there are counted, in the order of their modification times, and the spool is
    /// brought within the budget.
    pub fn open(root: impl AsRef<Path>, budget_bytes: u64) -> Result<Arc<Spool>, StoreError> {
        let (dir, cleaned) = Dir::open(root.as_ref())?;
        let mut found = dir.scan()?;
        found.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
        let mut state = SpoolState {
            entries: HashMap::new(),
            total: 0,
            tick: 0,
        };
        for (digest, size, _) in found {
            state.tick += 1;
            state.total += size;
            state.entries.insert(digest, (size, state.tick));
        }
        let spool = Arc::new(Spool {
            dir,
            budget: budget_bytes,
            state: Mutex::new(state),
            cleaned,
        });
        spool.evict_to_budget(None)?;
        Ok(spool)
    }

    pub fn cleaned_partials(&self) -> usize {
        self.cleaned
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Bytes held now.
    pub fn used(&self) -> u64 {
        self.lock().total
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SpoolState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Spool the bytes of `reader`; returns their digest. An object larger than the whole budget is refused
    /// (`TooLarge`). Otherwise least-recently-used objects are evicted until the spool is within its budget; the
    /// object just written is never the one evicted.
    pub fn put(&self, reader: &mut dyn Read) -> Result<Digest, StoreError> {
        let (partial, digest, size) = self.dir.write_partial(reader, self.budget)?;
        self.dir.publish(&partial, &digest)?;
        {
            let mut st = self.lock();
            st.tick += 1;
            let tick = st.tick;
            match st.entries.get_mut(&digest) {
                Some(entry) => entry.1 = tick, // converged on an existing object: only its recency changes
                None => {
                    st.entries.insert(digest.clone(), (size, tick));
                    st.total += size;
                }
            }
        }
        self.evict_to_budget(Some(&digest))?;
        Ok(digest)
    }

    /// Open an object for reading and mark it recently used. The returned file stays readable even if the object
    /// is evicted afterwards.
    pub fn get(&self, digest: &Digest) -> Option<File> {
        let file = self.dir.open_object(digest)?;
        let mut st = self.lock();
        if st.entries.contains_key(digest) {
            st.tick += 1;
            let tick = st.tick;
            if let Some(e) = st.entries.get_mut(digest) {
                e.1 = tick;
            }
            // Keep the order across a restart (best effort).
            let _ = file.set_modified(SystemTime::now());
        }
        Some(file)
    }

    pub fn contains(&self, digest: &Digest) -> bool {
        self.lock().entries.contains_key(digest)
    }

    /// Digests in eviction order: the next to go first.
    pub fn eviction_order(&self) -> Vec<Digest> {
        let st = self.lock();
        let mut v: Vec<(&Digest, u64)> = st.entries.iter().map(|(d, (_, t))| (d, *t)).collect();
        v.sort_by_key(|(_, t)| *t);
        v.into_iter().map(|(d, _)| d.clone()).collect()
    }

    pub fn root(&self) -> &Path {
        &self.dir.root
    }

    fn evict_to_budget(&self, keep: Option<&Digest>) -> Result<(), StoreError> {
        loop {
            let victim = {
                let st = self.lock();
                if st.total <= self.budget {
                    return Ok(());
                }
                st.entries
                    .iter()
                    .filter(|(d, _)| Some(*d) != keep)
                    .min_by_key(|(_, (_, tick))| *tick)
                    .map(|(d, (size, _))| (d.clone(), *size))
            };
            let Some((digest, size)) = victim else {
                return Ok(());
            };
            match fs::remove_file(self.dir.object_path(&digest)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(StoreError::Io {
                        context: "cannot evict a spool object",
                        source: e,
                    });
                }
            }
            let mut st = self.lock();
            if st.entries.remove(&digest).is_some() {
                st.total = st.total.saturating_sub(size);
            }
        }
    }

    #[cfg(test)]
    fn put_until_rename(&self, reader: &mut dyn Read) -> Result<PathBuf, StoreError> {
        Ok(self.dir.write_partial(reader, self.budget)?.0)
    }
}

// ---------------------------------------------------------------------------------------------------- config

/// Why the configuration was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum DirsConfigError {
    /// A directory setting is not an absolute path.
    NotAbsolute { var: &'static str, value: String },
    /// A path contains `..` or cannot be normalised.
    Unclear { var: &'static str, value: String },
    /// The spool or the store would sit inside (or around) a repository root: a client-visible export
    /// directory must never be a place where repositories live.
    InsideRepositoryRoot {
        var: &'static str,
        dir: PathBuf,
        repos_root: PathBuf,
    },
    /// The store and the spool overlap: the spool evicts, the store never may.
    Overlap { artifact: PathBuf, spool: PathBuf },
    /// The budget is not a positive number of MiB.
    BadBudget(String),
    /// The spool is configured without the store or the reverse is fine, but a budget needs a spool.
    BudgetWithoutSpool,
}

impl fmt::Display for DirsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DirsConfigError::NotAbsolute { var, value } => {
                write!(f, "{var} must be an absolute path (got {value:?})")
            }
            DirsConfigError::Unclear { var, value } => {
                write!(
                    f,
                    "{var} must not contain `..` or other unclear parts (got {value:?})"
                )
            }
            DirsConfigError::InsideRepositoryRoot {
                var,
                dir,
                repos_root,
            } => write!(
                f,
                "{var} ({}) overlaps the repository root ({}); choose a directory outside the repositories",
                dir.display(),
                repos_root.display()
            ),
            DirsConfigError::Overlap { artifact, spool } => write!(
                f,
                "the artifact directory ({}) and the spool directory ({}) must not overlap: the spool evicts old \
                 objects, the artifact store never does",
                artifact.display(),
                spool.display()
            ),
            DirsConfigError::BadBudget(v) => write!(
                f,
                "PLANETER_SPOOL_BUDGET_MB must be a whole number of MiB greater than zero (got {v:?})"
            ),
            DirsConfigError::BudgetWithoutSpool => {
                write!(
                    f,
                    "PLANETER_SPOOL_BUDGET_MB is set but PLANETER_SPOOL_DIR is not"
                )
            }
        }
    }
}

impl std::error::Error for DirsConfigError {}

/// The default spool budget when a spool directory is set and no budget is: 1,024 MiB.
pub const DEFAULT_SPOOL_BUDGET_MB: u64 = 1024;

/// The validated directory settings. Nothing is created until [`TransportDirs::open`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirsConfig {
    pub artifact_dir: Option<PathBuf>,
    pub spool_dir: Option<PathBuf>,
    pub spool_budget_bytes: u64,
}

impl DirsConfig {
    /// Read and validate the three settings (`PLANETER_ARTIFACT_DIR`, `PLANETER_SPOOL_DIR`,
    /// `PLANETER_SPOOL_BUDGET_MB`) against the repository root. Unset directories mean "not configured".
    pub fn from_vars(
        artifact_dir: Option<&str>,
        spool_dir: Option<&str>,
        spool_budget_mb: Option<&str>,
        repos_root: &Path,
    ) -> Result<DirsConfig, DirsConfigError> {
        fn non_empty(v: Option<&str>) -> Option<&str> {
            v.filter(|s| !s.is_empty())
        }
        let artifact = non_empty(artifact_dir)
            .map(|v| normalise("PLANETER_ARTIFACT_DIR", v))
            .transpose()?;
        let spool = non_empty(spool_dir)
            .map(|v| normalise("PLANETER_SPOOL_DIR", v))
            .transpose()?;
        let budget_mb = match non_empty(spool_budget_mb) {
            None => DEFAULT_SPOOL_BUDGET_MB,
            Some(v) => {
                if spool.is_none() {
                    return Err(DirsConfigError::BudgetWithoutSpool);
                }
                match v.parse::<u64>() {
                    Ok(n) if n > 0 && n <= (u64::MAX >> 20) => n,
                    _ => return Err(DirsConfigError::BadBudget(v.to_owned())),
                }
            }
        };
        // The repository root may be given relative to the working directory.
        let root =
            lexical(&std::path::absolute(repos_root).unwrap_or_else(|_| repos_root.to_path_buf()));
        for (var, dir) in [
            ("PLANETER_ARTIFACT_DIR", &artifact),
            ("PLANETER_SPOOL_DIR", &spool),
        ] {
            if let Some(d) = dir
                && (d.starts_with(&root) || root.starts_with(d))
            {
                return Err(DirsConfigError::InsideRepositoryRoot {
                    var,
                    dir: d.clone(),
                    repos_root: root.clone(),
                });
            }
        }
        if let (Some(a), Some(s)) = (&artifact, &spool)
            && (a.starts_with(s) || s.starts_with(a))
        {
            return Err(DirsConfigError::Overlap {
                artifact: a.clone(),
                spool: s.clone(),
            });
        }
        Ok(DirsConfig {
            artifact_dir: artifact,
            spool_dir: spool,
            spool_budget_bytes: budget_mb << 20,
        })
    }
}

/// An absolute path without `.` parts, symlinks resolved for the part that exists (so a symlink into a
/// repository root is seen for what it is).
fn normalise(var: &'static str, value: &str) -> Result<PathBuf, DirsConfigError> {
    let p = Path::new(value);
    if !p.is_absolute() {
        return Err(DirsConfigError::NotAbsolute {
            var,
            value: value.to_owned(),
        });
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(DirsConfigError::Unclear {
            var,
            value: value.to_owned(),
        });
    }
    Ok(lexical(p))
}

/// Resolve the longest existing prefix with the file system (symlinks), keep the rest as written.
fn lexical(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(&existing) {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_owned()),
            existing.parent().map(Path::to_path_buf),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// The opened store and spool of a running server.
pub struct TransportDirs {
    pub artifacts: Option<ArtifactStore>,
    pub spool: Option<Arc<Spool>>,
}

impl TransportDirs {
    /// Create the directories and open them, removing orphaned partial files.
    pub fn open(config: &DirsConfig) -> Result<TransportDirs, StoreError> {
        let artifacts = config
            .artifact_dir
            .as_deref()
            .map(ArtifactStore::open)
            .transpose()?;
        let spool = config
            .spool_dir
            .as_deref()
            .map(|d| Spool::open(d, config.spool_budget_bytes))
            .transpose()?;
        Ok(TransportDirs { artifacts, spool })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::thread;

    fn dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("planeter-artifacts-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn put_bytes(store: &ArtifactStore, b: &[u8]) -> Digest {
        store.put(&mut Cursor::new(b.to_vec())).unwrap()
    }

    fn bytes_of(mut f: File) -> Vec<u8> {
        let mut v = Vec::new();
        f.read_to_end(&mut v).unwrap();
        v
    }

    fn partials(root: &Path) -> usize {
        fs::read_dir(root.join("incoming")).unwrap().count()
    }

    #[test]
    fn a_digest_is_validated_and_cannot_name_another_path() {
        assert!(Digest::parse(&"a".repeat(64)).is_ok());
        for bad in [
            "",
            "abc",
            &"A".repeat(64),
            &"g".repeat(64),
            &"a".repeat(63),
            &"a".repeat(65),
            "../../etc/passwd",
        ] {
            assert!(
                matches!(Digest::parse(bad), Err(StoreError::BadDigest(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_object_is_stored_by_its_digest_and_read_back() {
        let d = dir("roundtrip");
        let store = ArtifactStore::open(&d).unwrap();
        let digest = put_bytes(&store, b"hello artifact");
        // The well-known SHA-256 of the bytes, computed independently.
        assert_eq!(
            digest.as_str(),
            planeter_auth::sha256_hex(b"hello artifact")
        );
        assert!(store.contains(&digest));
        assert_eq!(bytes_of(store.get(&digest).unwrap()), b"hello artifact");
        assert_eq!(store.list().unwrap(), vec![digest.clone()]);
        // Storing it again changes nothing.
        assert_eq!(put_bytes(&store, b"hello artifact"), digest);
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(partials(&d), 0);
        // A large object streams through in chunks.
        let big = vec![7u8; 5 * CHUNK + 13];
        let dg = put_bytes(&store, &big);
        assert_eq!(dg.as_str(), planeter_auth::sha256_hex(&big));
        assert_eq!(bytes_of(store.get(&dg).unwrap()), big);
        let _ = fs::remove_dir_all(&d);
    }

    /// A crash between the write and the rename leaves no visible object, and opening again cleans up.
    #[test]
    fn a_crash_between_write_and_rename_leaves_no_visible_object() {
        let d = dir("crash-store");
        let store = ArtifactStore::open(&d).unwrap();
        let partial = store
            .put_until_rename(&mut Cursor::new(b"half way".to_vec()))
            .unwrap();
        assert!(partial.to_string_lossy().ends_with(".partial"));
        assert!(partial.exists());
        let digest = Digest::parse(&planeter_auth::sha256_hex(b"half way")).unwrap();
        assert!(
            !store.contains(&digest),
            "an unfinished write is not an object"
        );
        assert!(store.list().unwrap().is_empty());
        drop(store); // "the process died"
        let reopened = ArtifactStore::open(&d).unwrap();
        assert_eq!(reopened.cleaned_partials(), 1);
        assert!(!partial.exists());
        assert!(reopened.list().unwrap().is_empty());
        assert!(!reopened.contains(&digest));
        // And the same bytes can then be stored normally.
        assert_eq!(put_bytes(&reopened, b"half way"), digest);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_read_error_mid_stream_leaves_nothing_behind() {
        struct Failing(usize);
        impl Read for Failing {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    return Err(io::Error::other("the source broke"));
                }
                self.0 -= 1;
                buf[..4].copy_from_slice(b"data");
                Ok(4)
            }
        }
        let d = dir("failing");
        let store = ArtifactStore::open(&d).unwrap();
        assert!(matches!(
            store.put(&mut Failing(3)),
            Err(StoreError::Io { .. })
        ));
        assert_eq!(partials(&d), 0);
        assert!(store.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn writers_of_the_same_digest_converge_on_one_object() {
        let d = dir("converge");
        let store = Arc::new(ArtifactStore::open(&d).unwrap());
        let payload = vec![42u8; 3 * CHUNK + 5];
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (store, payload) = (store.clone(), payload.clone());
                thread::spawn(move || store.put(&mut Cursor::new(payload)).unwrap())
            })
            .collect();
        let digests: Vec<Digest> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(digests.windows(2).all(|w| w[0] == w[1]));
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(partials(&d), 0, "no partial survives");
        assert_eq!(bytes_of(store.get(&digests[0]).unwrap()), payload);
        // The same holds for the spool, and it counts the object once.
        let s = Spool::open(dir("converge-spool"), 100 * CHUNK as u64).unwrap();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (s, payload) = (s.clone(), payload.clone());
                thread::spawn(move || s.put(&mut Cursor::new(payload)).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(s.used(), payload.len() as u64);
        assert_eq!(s.eviction_order().len(), 1);
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(s.root());
    }

    fn spool_put(s: &Spool, n: usize, fill: u8) -> Digest {
        s.put(&mut Cursor::new(vec![fill; n])).unwrap()
    }

    #[test]
    fn the_spool_evicts_least_recently_used_first_and_keeps_within_budget() {
        let d = dir("lru");
        let s = Spool::open(&d, 300).unwrap();
        let a = spool_put(&s, 100, 1);
        let b = spool_put(&s, 100, 2);
        let c = spool_put(&s, 100, 3);
        assert_eq!(s.used(), 300);
        assert_eq!(s.eviction_order(), vec![a.clone(), b.clone(), c.clone()]);
        // Use `a`: it is now the most recent, so `b` is next to go.
        assert!(s.get(&a).is_some());
        assert_eq!(s.eviction_order(), vec![b.clone(), c.clone(), a.clone()]);
        let e = spool_put(&s, 100, 4);
        assert!(
            !s.contains(&b),
            "the least recently used object was evicted"
        );
        assert!(s.contains(&a) && s.contains(&c) && s.contains(&e));
        assert!(s.used() <= 300);
        // A bigger object evicts as many as it needs, oldest first, and is itself never evicted.
        let f = spool_put(&s, 250, 5);
        assert!(s.contains(&f));
        assert!(!s.contains(&c) && !s.contains(&a) && !s.contains(&e));
        assert!(s.used() <= 300, "{}", s.used());
        assert!(!s.dir.object_path(&b).exists(), "evicted from disk too");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_object_larger_than_the_whole_budget_is_refused_and_nothing_is_evicted() {
        let d = dir("toolarge");
        let s = Spool::open(&d, 300).unwrap();
        let a = spool_put(&s, 100, 1);
        let err = s.put(&mut Cursor::new(vec![9u8; 301])).unwrap_err();
        assert!(matches!(err, StoreError::TooLarge { limit: 300 }), "{err}");
        assert!(s.contains(&a), "a refused object evicts nothing");
        assert_eq!(partials(&d), 0);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_reader_keeps_a_file_that_is_evicted_after_it_was_opened() {
        let d = dir("evicted-open");
        let s = Spool::open(&d, 200).unwrap();
        let a = spool_put(&s, 100, 1);
        let file = s.get(&a).unwrap();
        spool_put(&s, 100, 2);
        spool_put(&s, 100, 3);
        assert!(!s.contains(&a));
        assert_eq!(
            bytes_of(file).len(),
            100,
            "the open handle still reads the evicted object"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_spool_after_a_restart_counts_what_is_there_cleans_partials_and_keeps_within_budget() {
        let d = dir("restart");
        {
            let s = Spool::open(&d, 1000).unwrap();
            spool_put(&s, 400, 1);
            spool_put(&s, 400, 2);
            s.put_until_rename(&mut Cursor::new(vec![3u8; 100]))
                .unwrap();
        }
        let s = Spool::open(&d, 1000).unwrap();
        assert_eq!(s.cleaned_partials(), 1);
        assert_eq!(s.used(), 800);
        assert_eq!(partials(&d), 0);
        // A smaller budget on the next start evicts down to it.
        drop(s);
        let s = Spool::open(&d, 500).unwrap();
        assert!(s.used() <= 500);
        assert_eq!(s.eviction_order().len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    /// The spool's eviction never reaches the store, even when the spool is under pressure and both live in
    /// one parent directory.
    #[test]
    fn eviction_never_touches_the_artifact_store() {
        let parent = dir("both");
        let store = ArtifactStore::open(parent.join("artifacts")).unwrap();
        let kept: Vec<Digest> = (0..5u8).map(|i| put_bytes(&store, &vec![i; 150])).collect();
        let s = Spool::open(parent.join("spool"), 300).unwrap();
        for i in 0..40u8 {
            spool_put(&s, 150, i);
            assert!(s.used() <= 300);
        }
        assert_eq!(s.eviction_order().len(), 2);
        for d in &kept {
            assert!(store.contains(d), "a retained artifact was lost");
            assert_eq!(bytes_of(store.get(d).unwrap()).len(), 150);
        }
        assert_eq!(store.list().unwrap().len(), 5);
        let _ = fs::remove_dir_all(&parent);
    }

    // ---- configuration

    fn cfg(
        a: Option<&str>,
        s: Option<&str>,
        b: Option<&str>,
        root: &Path,
    ) -> Result<DirsConfig, DirsConfigError> {
        DirsConfig::from_vars(a, s, b, root)
    }

    #[test]
    fn the_configuration_is_validated() {
        let base = dir("cfg");
        let repos = base.join("repos");
        fs::create_dir_all(&repos).unwrap();
        let ok = |p: &str| base.join(p).to_string_lossy().into_owned();
        // Nothing configured: fine, nothing happens.
        let none = cfg(None, None, None, &repos).unwrap();
        assert_eq!((none.artifact_dir, none.spool_dir), (None, None));
        // Both configured, outside the repositories.
        let both = cfg(Some(&ok("art")), Some(&ok("spool")), Some("64"), &repos).unwrap();
        assert_eq!(both.spool_budget_bytes, 64 << 20);
        let default = cfg(None, Some(&ok("spool")), None, &repos).unwrap();
        assert_eq!(default.spool_budget_bytes, DEFAULT_SPOOL_BUDGET_MB << 20);
        // The spool inside a repository root, around it, or equal to it.
        for bad in [
            repos.join("alice").join("spool"),
            repos.clone(),
            base.clone(),
        ] {
            let e = cfg(None, Some(&bad.to_string_lossy()), None, &repos).unwrap_err();
            assert!(
                matches!(e, DirsConfigError::InsideRepositoryRoot { .. }),
                "{bad:?}: {e}"
            );
            assert!(e.to_string().contains("outside the repositories"));
        }
        // The store has the same rule.
        assert!(matches!(
            cfg(Some(&repos.join("x").to_string_lossy()), None, None, &repos),
            Err(DirsConfigError::InsideRepositoryRoot {
                var: "PLANETER_ARTIFACT_DIR",
                ..
            })
        ));
        // A symlink into a repository root is seen through.
        let link = base.join("link");
        std::os::unix::fs::symlink(&repos, &link).unwrap();
        assert!(matches!(
            cfg(
                None,
                Some(&link.join("spool").to_string_lossy()),
                None,
                &repos
            ),
            Err(DirsConfigError::InsideRepositoryRoot { .. })
        ));
        // Relative, `..`, overlap, bad budget.
        assert!(matches!(
            cfg(None, Some("spool"), None, &repos),
            Err(DirsConfigError::NotAbsolute { .. })
        ));
        assert!(matches!(
            cfg(
                None,
                Some(&format!("{}/a/../b", base.display())),
                None,
                &repos
            ),
            Err(DirsConfigError::Unclear { .. })
        ));
        assert!(matches!(
            cfg(Some(&ok("x")), Some(&ok("x/inner")), None, &repos),
            Err(DirsConfigError::Overlap { .. })
        ));
        assert!(matches!(
            cfg(Some(&ok("x")), Some(&ok("x")), None, &repos),
            Err(DirsConfigError::Overlap { .. })
        ));
        for b in ["0", "-1", "ten", "1.5", "99999999999999999999"] {
            assert!(
                matches!(
                    cfg(None, Some(&ok("s")), Some(b), &repos),
                    Err(DirsConfigError::BadBudget(_))
                ),
                "{b}"
            );
        }
        assert!(matches!(
            cfg(Some(&ok("a")), None, Some("10"), &repos),
            Err(DirsConfigError::BudgetWithoutSpool)
        ));
        // Opening creates the directories.
        let opened = TransportDirs::open(&both).unwrap();
        assert!(opened.artifacts.unwrap().root().join("objects").is_dir());
        assert!(opened.spool.unwrap().root().join("incoming").is_dir());
        let _ = fs::remove_dir_all(&base);
    }
}
