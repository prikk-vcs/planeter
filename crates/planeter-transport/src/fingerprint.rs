//! The repository **state fingerprint** (RFC 004 D-2): what a fetch's cache key and `ETag` are made of.
//!
//! Two signals, both in it. The primary one is the repository's **revision counter**, bumped under the W lease
//! whenever a write completes ([`planeter_store::RevisionStore`]). The cross-check is a digest of the lengths and
//! modification times of the files under the repository's `.prikk` directory (excluding the caches and lock files,
//! which change without the content changing), so that a write made out of band — by a person, a restored backup,
//! a tool that is not planeter — changes the fingerprint too and a stale artifact is never served.
//!
//! **Limit, plainly:** the file cross-check can be fooled by an out-of-band write that keeps every length and
//! lands in the same modification time (the filesystem's resolution); the revision counter is the primary signal,
//! and anything that writes through planeter bumps it. Two exports of one state are byte-identical (RFC 004 U-15),
//! so the fingerprint is a sound `ETag`.

use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Directories (by name) whose files change without the repository's content changing.
const IGNORED_DIRS: &[&str] = &["cache"];

/// A digest of the files under `prikk_dir` (their relative paths, lengths and modification times, in path
/// order), as lowercase hex. An unreadable directory is an error: a fingerprint that could not be computed must
/// never be treated as "unchanged" — **except that an entry that disappears between being listed and being looked at is not in the digest: it is
/// gone** (IS-75, the same rule as the store's size, IS-74: prikk creates and removes files while it writes, and an export or a changes page must not be
/// refused for a file the next walk would not show). That is `NotFound` from the `lstat` of a listed entry and `NotFound` from the listing of a
/// **sub**directory that was listed and is gone; every other error stays an error, and so does a missing `prikk_dir` itself.
///
/// **Why that is right for a cache key.** The digest is of what was there. A file that vanished is a change of the store; the next walk, which does not
/// show it, gives the digest this walk gave (it left the file out too), and a walk that still shows the file's siblings with changed lengths or times
/// gives another. No walk can give an equal digest for a store whose files differ: every file that is listed and present is in it with its length and
/// its time, and the only file left out is one that was not there when it was looked at.
pub fn files_digest(prikk_dir: &Path) -> io::Result<String> {
    files_digest_walk(prikk_dir, &mut |_, _| {})
}

/// The two places of the walk where something can change under it: a test (and only a test) acts there, to make a file vanish deterministically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkStep {
    /// A directory is about to be listed.
    BeforeList,
    /// An entry was listed and is about to be looked at.
    BeforeStat,
}

fn files_digest_walk<F: FnMut(WalkStep, &Path)>(
    prikk_dir: &Path,
    hook: &mut F,
) -> io::Result<String> {
    let mut lines: Vec<String> = Vec::new();
    walk(prikk_dir, prikk_dir, &mut lines, hook)?;
    lines.sort_unstable();
    Ok(planeter_auth::sha256_hex(lines.join("\n").as_bytes()))
}

fn walk<F: FnMut(WalkStep, &Path)>(
    root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
    hook: &mut F,
) -> io::Result<()> {
    hook(WalkStep::BeforeList, dir);
    let listing = match fs::read_dir(dir) {
        Ok(l) => l,
        // A subdirectory that was listed and is gone: nothing in it. The root is not a subdirectory: its absence is an error.
        Err(e) if e.kind() == io::ErrorKind::NotFound && dir != root => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in listing {
        let entry = match entry {
            Ok(entry) => entry,
            // The directory vanished while it was being read: what was listed before is in, the rest is gone.
            Err(e) if e.kind() == io::ErrorKind::NotFound && dir != root => break,
            Err(e) => return Err(e),
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        hook(WalkStep::BeforeStat, &path);
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            // Listed and gone: not in the digest.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if meta.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(root, &path, out, hook)?;
        } else if !name.ends_with(".lock") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let nanos = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            out.push(format!("{rel}\0{}\0{nanos}", meta.len()));
        }
    }
    Ok(())
}

/// The fingerprint: a short, specified digest of the revision counter and the files digest (32 hex characters).
pub fn fingerprint(revision: u64, files: &str) -> String {
    let digest = planeter_auth::sha256_hex(
        format!("planeter-state-v1\nrevision={revision}\nfiles={files}\n").as_bytes(),
    );
    digest[..32].to_owned()
}

#[cfg(test)]
mod walk_tests {
    //! IS-75: a file that vanishes between the listing and the stat is not in the digest; every other failure still refuses. Deterministic: the hook of
    //! `files_digest_walk` makes the change at the exact step.
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn store() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "planeter-digest-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn plain(dir: &Path) -> String {
        files_digest(dir).unwrap()
    }

    #[test]
    fn a_file_removed_between_the_listing_and_the_stat_is_not_in_the_digest() {
        let d = store();
        put(&d.join("keep"), 10);
        put(&d.join("goes"), 1000);
        // The digest of what is left, taken afterwards on the same store without the file.
        let with = plain(&d);
        let mut hits = 0;
        let during = files_digest_walk(&d, &mut |step, path| {
            if step == WalkStep::BeforeStat && path.ends_with("goes") {
                fs::remove_file(path).unwrap();
                hits += 1;
            }
        })
        .unwrap();
        assert_eq!(hits, 1, "the hook ran at the step");
        let after = plain(&d);
        assert_eq!(
            during, after,
            "the digest taken while it vanished is the digest of the store afterwards"
        );
        assert_ne!(
            during, with,
            "...and not the digest of the store while the file was there"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_subdirectory_removed_after_it_was_listed_is_not_in_the_digest() {
        let d = store();
        put(&d.join("sub").join("big"), 5000);
        put(&d.join("top"), 7);
        let during = files_digest_walk(&d, &mut |step, path| {
            if step == WalkStep::BeforeList && path.ends_with("sub") {
                fs::remove_dir_all(path).unwrap();
            }
        })
        .unwrap();
        assert_eq!(during, plain(&d));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_directory_that_cannot_be_read_is_still_an_error() {
        let d = store();
        put(&d.join("locked").join("f"), 3);
        fs::set_permissions(d.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(d.join("locked")).is_ok() {
            fs::set_permissions(d.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
            fs::remove_dir_all(&d).unwrap();
            eprintln!("skipping: this user can read a directory with no permissions");
            return;
        }
        let r = files_digest(&d);
        fs::set_permissions(d.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_missing_store_is_still_an_error() {
        let none =
            std::env::temp_dir().join(format!("planeter-digest-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&none);
        assert_eq!(
            files_digest(&none).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        // ...also when the root vanishes after the walk began (the root is not a subdirectory).
        let d = store();
        let r = files_digest_walk(&d, &mut |step, path| {
            if step == WalkStep::BeforeList && path == d.as_path() {
                fs::remove_dir_all(path).unwrap();
            }
        });
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn the_ignored_directory_and_the_lock_files_are_still_left_out() {
        let d = store();
        put(&d.join("a"), 1);
        let base = plain(&d);
        put(&d.join("cache").join("x"), 99);
        put(&d.join("some.lock"), 5);
        assert_eq!(plain(&d), base);
        fs::remove_dir_all(&d).unwrap();
    }
}
