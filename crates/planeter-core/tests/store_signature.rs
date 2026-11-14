//! The store signature (re-baseline v2, D1): the sorted `(path, size, mtime in ns)` of every regular file under `.prikk`, leaving out
//! `*.lock`. Equal signatures around a failed accept are the evidence that the store did not change.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use planeter_core::admission::{StoreSizeError, store_signature, store_signature_limited};

fn repo(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("store-signature-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".prikk/containers/patch")).unwrap();
    std::fs::create_dir_all(dir.join(".prikk/active/default")).unwrap();
    std::fs::write(dir.join(".prikk/FORMAT"), "8\n").unwrap();
    std::fs::write(dir.join(".prikk/containers/patch/a.container"), b"abc").unwrap();
    dir
}

#[test]
fn it_lists_the_regular_files_sorted_with_size_and_mtime_and_is_stable_between_calls() {
    let dir = repo("list");
    let a = store_signature(&dir).unwrap();
    let paths: Vec<&str> = a.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(paths, ["FORMAT", "containers/patch/a.container"]);
    assert_eq!(a[0].1, 2);
    assert_eq!(a[1].1, 3);
    assert!(a.iter().all(|f| f.2 > 0));
    assert_eq!(store_signature(&dir).unwrap(), a, "reading changes nothing");
}

#[test]
fn locks_are_left_out_so_a_clean_refusal_that_takes_and_drops_them_changes_nothing() {
    let dir = repo("locks");
    let before = store_signature(&dir).unwrap();
    std::fs::write(dir.join(".prikk/active/default/active.lock"), b"pid").unwrap();
    std::fs::write(dir.join(".prikk/containers/objects.lock"), b"pid").unwrap();
    assert_eq!(store_signature(&dir).unwrap(), before);
    std::fs::remove_file(dir.join(".prikk/containers/objects.lock")).unwrap();
    assert_eq!(store_signature(&dir).unwrap(), before);
}

#[test]
fn a_changed_size_or_a_changed_mtime_or_a_new_or_missing_file_changes_it() {
    let dir = repo("changes");
    let base = store_signature(&dir).unwrap();
    let f = dir.join(".prikk/containers/patch/a.container");
    // Same size, new mtime.
    let file = std::fs::File::options().write(true).open(&f).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    drop(file);
    let touched = store_signature(&dir).unwrap();
    assert_ne!(touched, base);
    assert_eq!(touched[1].1, base[1].1);
    // New size.
    std::fs::write(&f, b"abcdef").unwrap();
    assert_ne!(store_signature(&dir).unwrap(), touched);
    // A new file, then a missing one.
    let before = store_signature(&dir).unwrap();
    std::fs::write(dir.join(".prikk/containers/patch/b.container"), b"x").unwrap();
    assert_ne!(store_signature(&dir).unwrap(), before);
    std::fs::remove_file(dir.join(".prikk/containers/patch/b.container")).unwrap();
    std::fs::remove_file(&f).unwrap();
    assert_eq!(store_signature(&dir).unwrap().len(), 1);
}

#[test]
fn the_walk_is_bounded_and_a_missing_store_is_an_error_never_a_guess() {
    let dir = repo("bounds");
    assert!(matches!(
        store_signature_limited(&dir, 2),
        Err(StoreSizeError::TooManyEntries)
    ));
    assert!(store_signature_limited(&dir, 100).is_ok());
    let none = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("store-signature-nowhere");
    let _ = std::fs::remove_dir_all(&none);
    assert!(matches!(store_signature(&none), Err(StoreSizeError::Io(_))));
}

#[test]
fn symlinks_are_not_followed_and_add_nothing() {
    let dir = repo("symlink");
    let before = store_signature(&dir).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", dir.join(".prikk/link")).unwrap();
    std::os::unix::fs::symlink("/tmp", dir.join(".prikk/dirlink")).unwrap();
    assert_eq!(store_signature(&dir).unwrap(), before);
}
