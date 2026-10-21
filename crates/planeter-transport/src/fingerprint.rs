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
/// never be treated as "unchanged".
pub fn files_digest(prikk_dir: &Path) -> io::Result<String> {
    let mut lines: Vec<String> = Vec::new();
    walk(prikk_dir, prikk_dir, &mut lines)?;
    lines.sort_unstable();
    Ok(planeter_auth::sha256_hex(lines.join("\n").as_bytes()))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(root, &path, out)?;
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
