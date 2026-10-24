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

/// An object taken out of the store and not yet gone: in the trash, to be unlinked or put back.
#[derive(Debug)]
pub struct Removal {
    trashed: PathBuf,
    original: PathBuf,
}

/// The accepted-artifact store: content-addressed, atomic, **never evicting**. The only way an object leaves is
/// [`ArtifactStore::remove`], which the retention sweep (RFC 005 part 5) calls for an artifact nothing needs any more.
pub struct ArtifactStore {
    dir: Dir,
    /// Partial files removed when this store was opened (a crash's leftovers).
    cleaned: usize,
}

impl ArtifactStore {
    /// Open (creating) the store at `root`, removing orphaned `.partial` files.
    pub fn open(root: impl AsRef<Path>) -> Result<ArtifactStore, StoreError> {
        let (dir, mut cleaned) = Dir::open(root.as_ref())?;
        // The trash: objects renamed out of `objects/` by `remove` and not yet unlinked when a kill came. They are not
        // objects any more (nothing finds them by digest); the start-up finishes the removal.
        let trash = dir.root.join("trash");
        fs::create_dir_all(&trash).map_err(io_err("cannot create the trash directory"))?;
        for e in fs::read_dir(&trash).map_err(io_err("cannot read the trash directory"))? {
            let e = e.map_err(io_err("cannot read the trash directory"))?;
            let _ = fs::remove_file(e.path());
            cleaned += 1;
        }
        // Working directories left by a crashed proposal: removed whole (they hold only uploads and outputs that
        // were never recorded; whatever was retained is in `objects/`).
        let work = dir.root.join("work");
        fs::create_dir_all(&work).map_err(io_err("cannot create the work directory"))?;
        for e in fs::read_dir(&work).map_err(io_err("cannot read the work directory"))? {
            let e = e.map_err(io_err("cannot read the work directory"))?;
            let _ = fs::remove_dir_all(e.path());
            cleaned += 1;
        }
        Ok(ArtifactStore { dir, cleaned })
    }

    /// A fresh **working area for one proposal** (`<root>/work/<id>/{in,out}`), removed with everything in it when
    /// the guard is dropped. The upload is written to `in/artifact`; prikk is given `in` read-only and `out`
    /// read-write, and nothing else of the store (RFC 004 D-6: no whole-directory mount).
    pub fn new_accept_dir(&self) -> Result<AcceptDir, StoreError> {
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let path = self
            .dir
            .root
            .join("work")
            .join(format!("{}-{n}", std::process::id()));
        fs::create_dir(&path).map_err(io_err("cannot create a proposal working directory"))?;
        let dir = AcceptDir { path };
        fs::create_dir(dir.in_dir()).map_err(io_err("cannot create a proposal input directory"))?;
        fs::create_dir(dir.out_dir())
            .map_err(io_err("cannot create a proposal output directory"))?;
        Ok(dir)
    }

    /// Keep the uploaded artifact of `accept` as the object `digest` (never evicted): the file is **moved** into
    /// place (an atomic rename on the same volume). Keeping bytes that are already there changes nothing. The
    /// caller vouches for the digest, which [`UploadWriter::finish`] computed while the bytes were written.
    pub fn retain(&self, accept: &AcceptDir, digest: &Digest) -> Result<bool, StoreError> {
        self.dir.publish(&accept.artifact(), digest)
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

    /// Remove the object `digest`: **rename it into the trash, then unlink it**, so a kill between the two leaves
    /// nothing half-removed (the object is already gone for every reader that looks it up; the next start-up
    /// unlinks what the trash still holds). Returns whether an object was there. A reader that opened the file
    /// before keeps reading it (the file lives until the last handle closes).
    ///
    /// The caller decides that nothing needs the artifact: this method knows nothing of proposals. A caller that has
    /// to look again after the object is out of reach (the retention sweep: a proposal may take the same artifact in
    /// between) uses [`ArtifactStore::begin_remove`] and then `finish_remove` or `undo_remove`.
    pub fn remove(&self, digest: &Digest) -> Result<bool, StoreError> {
        match self.begin_remove(digest)? {
            Some(removal) => self.finish_remove(removal).map(|()| true),
            None => Ok(false),
        }
    }

    /// The first half of [`ArtifactStore::remove`]: the object is renamed into the trash and so is gone for every
    /// lookup. `None` if it was not there.
    pub fn begin_remove(&self, digest: &Digest) -> Result<Option<Removal>, StoreError> {
        let from = self.dir.object_path(digest);
        if !from.is_file() {
            return Ok(None);
        }
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let to = self.dir.root.join("trash").join(format!(
            "{}.{}-{n}.trash",
            digest.0,
            std::process::id()
        ));
        fs::rename(&from, &to).map_err(io_err("cannot move an object into the trash"))?;
        if let Some(shard) = from.parent()
            && let Ok(d) = File::open(shard)
        {
            let _ = d.sync_all();
        }
        Ok(Some(Removal {
            trashed: to,
            original: from,
        }))
    }

    /// The second half: unlink what [`ArtifactStore::begin_remove`] put in the trash.
    pub fn finish_remove(&self, removal: Removal) -> Result<(), StoreError> {
        fs::remove_file(&removal.trashed).map_err(io_err("cannot unlink a removed object"))
    }

    /// Put back what [`ArtifactStore::begin_remove`] took out, because something needs it after all. If the object has
    /// been stored again in the meantime (same digest: the same bytes), the trashed copy is simply unlinked.
    pub fn undo_remove(&self, removal: Removal) -> Result<(), StoreError> {
        if removal.original.exists() {
            return fs::remove_file(&removal.trashed)
                .map_err(io_err("cannot unlink a removed object"));
        }
        fs::rename(&removal.trashed, &removal.original).map_err(io_err("cannot put an object back"))
    }

    /// Test seam: remove as far as a kill would stop it (renamed, not unlinked).
    #[cfg(test)]
    fn remove_until_unlink(&self, digest: &Digest) -> Result<Option<PathBuf>, StoreError> {
        Ok(self.begin_remove(digest)?.map(|r| r.trashed))
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

/// One proposal's working area (see [`ArtifactStore::new_accept_dir`]); removed, with everything in it, on drop.
#[derive(Debug)]
pub struct AcceptDir {
    path: PathBuf,
}

impl AcceptDir {
    /// The directory prikk reads (mounted read-only): it holds the artifact.
    pub fn in_dir(&self) -> PathBuf {
        self.path.join("in")
    }

    /// The directory prikk writes (mounted read-write): the claims file.
    pub fn out_dir(&self) -> PathBuf {
        self.path.join("out")
    }

    /// The uploaded artifact.
    pub fn artifact(&self) -> PathBuf {
        self.in_dir().join("artifact")
    }

    /// The file `sync accept --claims-out` writes (it does not exist yet: prikk refuses to overwrite one).
    pub fn claims(&self) -> PathBuf {
        self.out_dir().join("claims")
    }

    /// Start writing the upload to [`AcceptDir::artifact`]: at most `limit` bytes, hashed as they are written.
    pub fn begin_upload(&self, limit: u64) -> Result<UploadWriter, StoreError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.artifact())
            .map_err(io_err("cannot create the upload file"))?;
        Ok(UploadWriter {
            file,
            hash: Sha256Stream::new(),
            size: 0,
            limit,
        })
    }
}

impl Drop for AcceptDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The streaming writer of an upload: bytes are hashed as they are written and never held in memory.
pub struct UploadWriter {
    file: File,
    hash: Sha256Stream,
    size: u64,
    limit: u64,
}

impl UploadWriter {
    /// Append `chunk`. More than the limit in all is [`StoreError::TooLarge`].
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        self.size += chunk.len() as u64;
        if self.size > self.limit {
            return Err(StoreError::TooLarge { limit: self.limit });
        }
        self.hash.update(chunk);
        self.file
            .write_all(chunk)
            .map_err(io_err("cannot write the upload file"))
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.size
    }

    /// Flush to disk and return the digest and size of everything written.
    pub fn finish(self) -> Result<(Digest, u64), StoreError> {
        self.file
            .sync_all()
            .map_err(io_err("cannot sync the upload file"))?;
        Ok((Digest(self.hash.finish_hex()), self.size))
    }
}

/// One export's working directory (see [`Spool::new_work_dir`]); removed, with everything in it, on drop.
#[derive(Debug)]
pub struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// The directory: the only part of the spool the exporting process is given.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file the export writes (it does not exist yet: prikk refuses to overwrite one).
    pub fn file(&self) -> PathBuf {
        self.path.join("export")
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The export spool: content-addressed, atomic, bounded by a disk budget, evicting **least recently used**.
pub struct Spool {
    dir: Dir,
    budget: u64,
    state: Mutex<SpoolState>,
    /// Objects whose content was re-hashed, with the (length, modification time) they had then.
    checked: Mutex<HashMap<Digest, (u64, Option<SystemTime>)>>,
    cleaned: usize,
}

impl Spool {
    /// Open (creating) the spool at `root` with a budget of `budget_bytes`. Orphaned `.partial` files are
    /// removed; objects already there are counted, in the order of their modification times, and the spool is
    /// brought within the budget.
    pub fn open(root: impl AsRef<Path>, budget_bytes: u64) -> Result<Arc<Spool>, StoreError> {
        let (dir, mut cleaned) = Dir::open(root.as_ref())?;
        // Working directories left by a crashed export: removed whole.
        let work = dir.root.join("work");
        fs::create_dir_all(&work).map_err(io_err("cannot create the work directory"))?;
        for e in fs::read_dir(&work).map_err(io_err("cannot read the work directory"))? {
            let e = e.map_err(io_err("cannot read the work directory"))?;
            let _ = fs::remove_dir_all(e.path());
            cleaned += 1;
        }
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
            checked: Mutex::new(HashMap::new()),
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

    /// A fresh **working directory** for one export (`<spool>/work/<id>`), created now and removed when the returned
    /// guard is dropped, so that whatever writes there (prikk, in the spool-out mount) is given this directory and
    /// nothing else of the spool: no other export's file, no stored object. The finished file is moved into the spool
    /// by [`Spool::adopt`] (an atomic rename on the same volume) before the guard is dropped. Orphaned working
    /// directories (a crash) are removed when the spool is opened.
    pub fn new_work_dir(&self) -> Result<WorkDir, StoreError> {
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let path = self
            .dir
            .root
            .join("work")
            .join(format!("{}-{n}", std::process::id()));
        fs::create_dir(&path).map_err(io_err("cannot create an export working directory"))?;
        Ok(WorkDir { path })
    }

    /// A fresh path inside the spool for a file that is being written by someone else (prikk, in the spool-out
    /// mount): it ends in `.partial`, so a crash leaves nothing visible and opening the spool removes it. The file
    /// does not exist yet (prikk refuses to overwrite one). Pass it to [`Spool::adopt`] or [`Spool::discard`].
    pub fn partial_path(&self) -> PathBuf {
        self.dir.partial_path()
    }

    /// Remove a partial file (best effort; used on every failure path, so the spool never keeps a partial result).
    pub fn discard(&self, partial: &Path) {
        let _ = fs::remove_file(partial);
    }

    /// Take a finished partial file into the spool: hash it, refuse it if it is larger than the whole budget
    /// (`TooLarge`; the file is removed), otherwise rename it into place (atomically; the same volume) and count
    /// it. Returns its digest and size. Evicts least-recently-used objects to stay within the budget.
    pub fn adopt(&self, partial: &Path) -> Result<(Digest, u64), StoreError> {
        let result = (|| {
            let mut file = File::open(partial).map_err(io_err("cannot open the exported file"))?;
            let mut hash = Sha256Stream::new();
            let mut size: u64 = 0;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = file
                    .read(&mut buf)
                    .map_err(io_err("cannot read the exported file"))?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > self.budget {
                    return Err(StoreError::TooLarge { limit: self.budget });
                }
                hash.update(&buf[..n]);
            }
            file.sync_all()
                .map_err(io_err("cannot sync the exported file"))?;
            Ok((Digest(hash.finish_hex()), size))
        })();
        let (digest, size) = match result {
            Ok(v) => v,
            Err(e) => {
                self.discard(partial);
                return Err(e);
            }
        };
        self.dir.publish(partial, &digest)?;
        {
            let mut st = self.lock();
            st.tick += 1;
            let tick = st.tick;
            match st.entries.get_mut(&digest) {
                Some(entry) => entry.1 = tick,
                None => {
                    st.entries.insert(digest.clone(), (size, tick));
                    st.total += size;
                }
            }
        }
        self.evict_to_budget(Some(&digest))?;
        Ok((digest, size))
    }

    /// Check that an object is still what its name says: re-hash it unless it has not changed (length and
    /// modification time) since it was last checked. A mismatch removes the object (it is never served). Returns
    /// whether the object is present and intact.
    pub fn verify_object(&self, digest: &Digest) -> bool {
        let path = self.dir.object_path(digest);
        let Ok(meta) = fs::metadata(&path) else {
            self.forget(digest);
            return false;
        };
        let stamp = (meta.len(), meta.modified().ok());
        {
            let checked = self
                .checked
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if checked.get(digest) == Some(&stamp) {
                return true;
            }
        }
        let intact = (|| {
            let mut f = File::open(&path).ok()?;
            let mut hash = Sha256Stream::new();
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = f.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
            }
            Some(hash.finish_hex() == digest.0)
        })()
        .unwrap_or(false);
        if intact {
            self.checked
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(digest.clone(), stamp);
        } else {
            let _ = fs::remove_file(&path);
            self.forget(digest);
        }
        intact
    }

    /// Drop an object from the accounting (its file is gone or was removed).
    fn forget(&self, digest: &Digest) {
        let mut st = self.lock();
        if let Some((size, _)) = st.entries.remove(digest) {
            st.total = st.total.saturating_sub(size);
        }
        self.checked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(digest);
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
    pub artifacts: Option<Arc<ArtifactStore>>,
    pub spool: Option<Arc<Spool>>,
}

impl TransportDirs {
    /// Create the directories and open them, removing orphaned partial files.
    pub fn open(config: &DirsConfig) -> Result<TransportDirs, StoreError> {
        let artifacts = config
            .artifact_dir
            .as_deref()
            .map(|d| ArtifactStore::open(d).map(Arc::new))
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
mod tests;
