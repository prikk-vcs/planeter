//! The disposable cache of change sets in the spool (RFC 005 part 6). A change set is a pure function of content-addressed inputs
//! (the foot claim id and the root's parent block id), so an entry is correct for ever; it is only ever **evicted** (oldest first, past a byte
//! budget), never invalidated. Every file is written to a temporary name and renamed, so a reader sees a whole file or none.
//!
//! Layout under `<spool>/changes-cache/`: `rev-<revision id>.json` (a pointer to the key of the revision's change set, or the cached failure),
//! `k-<key>/list.json` (the change set) and `k-<key>/f-<file id>.json` (a file's diff).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::{ChangeSet, FileDiff};

/// The version of what an entry holds, the first component of every key. **Bump it when the format of a stored change set changes**: a
/// pointer to a key of another version is a miss (the view is computed again), and the old entries are only evicted, oldest first.
/// Version 2: cut text is data (`cut_chars`, `path_cut_bytes`), no marker written into it.
pub const KEY_SCHEMA: &str = "v2";

/// A failure that is a fact about immutable inputs, so it may be remembered (a busy or killed run may not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Remembered {
    NotOneChange,
    TreeTooLarge,
}

/// What the cache knows about a revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevEntry {
    Key(String),
    Failure(Remembered),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListEntry {
    Set(Box<ChangeSet>),
    Failure(Remembered),
}

pub struct Cache {
    dir: PathBuf,
    budget: u64,
}

fn hex_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Cache {
    pub fn open(spool: &Path, budget: u64) -> std::io::Result<Cache> {
        let dir = spool.join("changes-cache");
        fs::create_dir_all(&dir)?;
        // a run that was killed left temporary names: they are not entries
        for e in fs::read_dir(&dir)?.flatten() {
            if e.file_name().to_string_lossy().contains(".tmp-") {
                let _ = fs::remove_dir_all(e.path());
                let _ = fs::remove_file(e.path());
            }
        }
        Ok(Cache { dir, budget })
    }

    fn rev_path(&self, revision_id: &str) -> Option<PathBuf> {
        hex_ok(revision_id).then(|| self.dir.join(format!("rev-{revision_id}.json")))
    }

    fn key_dir(&self, key: &str) -> Option<PathBuf> {
        hex_ok(key).then(|| self.dir.join(format!("k-{key}")))
    }

    fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }

    fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn touch(path: &Path) {
        if let Ok(f) = fs::OpenOptions::new().write(true).open(path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
    }

    pub fn rev(&self, revision_id: &str) -> Option<RevEntry> {
        match Self::read_json(&self.rev_path(revision_id)?)? {
            // a pointer written by an earlier version of the format is not read
            RevEntry::Key(k) if !k.starts_with(&format!("{KEY_SCHEMA}-")) => None,
            e => Some(e),
        }
    }

    pub fn put_rev(&self, revision_id: &str, entry: &RevEntry) {
        if let Some(p) = self.rev_path(revision_id)
            && let Ok(body) = serde_json::to_vec(entry)
        {
            let _ = Self::write_atomic(&p, &body);
        }
    }

    pub fn list(&self, key: &str) -> Option<ListEntry> {
        let p = self.key_dir(key)?.join("list.json");
        let got = Self::read_json(&p);
        if got.is_some() {
            Self::touch(&p);
        }
        got
    }

    pub fn file(&self, key: &str, file_id: &str) -> Option<FileDiff> {
        if !hex_ok(file_id) {
            return None;
        }
        let p = self.key_dir(key)?.join(format!("f-{file_id}.json"));
        let got = Self::read_json(&p);
        if got.is_some()
            && let Some(l) = self.key_dir(key)
        {
            Self::touch(&l.join("list.json"));
        }
        got
    }

    /// Start writing the entry of `key`: diffs are written one by one into a temporary directory that [`Writer::finish`] renames into place.
    pub fn begin(&self, key: &str) -> Option<Writer<'_>> {
        let final_dir = self.key_dir(key)?;
        let tmp = self.dir.join(format!("k-{key}.tmp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).ok()?;
        Some(Writer {
            cache: self,
            tmp,
            final_dir,
            done: false,
        })
    }

    /// Write one file's diff into an entry that already exists (its other files are there; this one was evicted or never kept).
    pub fn put_file(&self, key: &str, diff: &FileDiff) {
        if let Some(d) = self.key_dir(key)
            && d.is_dir()
            && let Ok(body) = serde_json::to_vec(diff)
        {
            let _ = Self::write_atomic(&d.join(format!("f-{}.json", diff.entry.file_id)), &body);
        }
    }

    pub fn put_failure(&self, key: &str, why: Remembered) {
        if let Some(w) = self.begin(key) {
            w.finish(&ListEntry::Failure(why));
        }
    }

    fn size_of(path: &Path) -> u64 {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return 0;
        };
        if meta.is_dir() {
            fs::read_dir(path)
                .map(|it| it.flatten().map(|e| Self::size_of(&e.path())).sum())
                .unwrap_or(0)
        } else {
            meta.len()
        }
    }

    /// Past the budget, remove the least recently used entries (by the modification time of `list.json`, touched on every read) until under it.
    pub fn evict(&self) {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("k-") || name.contains(".tmp-") {
                continue;
            }
            let used = fs::metadata(e.path().join("list.json"))
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            entries.push((used, Self::size_of(&e.path()), e.path()));
        }
        let mut total: u64 = entries.iter().map(|x| x.1).sum();
        entries.sort();
        for (_, size, path) in entries {
            if total <= self.budget {
                break;
            }
            let _ = fs::remove_dir_all(&path);
            total = total.saturating_sub(size);
        }
    }

    /// The bytes held now (for tests).
    pub fn bytes(&self) -> u64 {
        Self::size_of(&self.dir)
    }
}

pub struct Writer<'a> {
    cache: &'a Cache,
    tmp: PathBuf,
    final_dir: PathBuf,
    done: bool,
}

impl Writer<'_> {
    pub fn put(&self, diff: &FileDiff) {
        if let Ok(body) = serde_json::to_vec(diff) {
            let _ = fs::write(
                self.tmp.join(format!("f-{}.json", diff.entry.file_id)),
                body,
            );
        }
    }

    pub fn finish(mut self, list: &ListEntry) {
        self.done = true;
        let Ok(body) = serde_json::to_vec(list) else {
            let _ = fs::remove_dir_all(&self.tmp);
            return;
        };
        if fs::write(self.tmp.join("list.json"), body).is_err() {
            let _ = fs::remove_dir_all(&self.tmp);
            return;
        }
        if self.final_dir.exists() {
            // the same inputs gave the same entry already (another revision with these claims): keep it
            let _ = fs::remove_dir_all(&self.tmp);
        } else if fs::rename(&self.tmp, &self.final_dir).is_err() {
            let _ = fs::remove_dir_all(&self.tmp);
        }
        self.cache.evict();
    }
}

impl Drop for Writer<'_> {
    fn drop(&mut self) {
        if !self.done {
            // nothing is cached for a run that did not finish (a busy, killed or timed-out run)
            let _ = fs::remove_dir_all(&self.tmp);
        }
    }
}
