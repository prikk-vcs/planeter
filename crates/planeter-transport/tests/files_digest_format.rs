//! IS-75: the digest of a store that does not change during the walk is **byte-identical** to the one the walk gave before the vanished-entry rule: the
//! cache key's format did not move. A fixed store (fixed contents, fixed modification times, a `cache/` directory and a lock file that are left out),
//! and the digest the code gave for it **before** this change, as a constant.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use planeter_transport::fingerprint::files_digest;

fn put(path: &Path, bytes: usize, secs: u64, nanos: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::new(secs, nanos))
        .unwrap();
}

fn fixed_store(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("files-digest-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let store = dir.join(".prikk");
    put(&store.join("a"), 3, 1_700_000_000, 123_456_789);
    put(&store.join("d").join("b"), 5, 1_700_000_001, 0);
    put(
        &store.join("d").join("e").join("c"),
        7,
        1_700_000_002,
        999_999_999,
    );
    put(&store.join("cache").join("ignored"), 11, 1_700_000_003, 1);
    put(&store.join("x.lock"), 2, 1_700_000_004, 2);
    store
}

/// The digest the code before IS-75 gave for `fixed_store` (computed by running that code on it).
const BEFORE: &str = "357f58b0fa790ed5407c1bf585eac47de430fb89ccd555827e61d51c62afbaae";

#[test]
fn the_digest_of_a_fixed_store_is_what_it_was_before_the_vanished_entry_rule() {
    let store = fixed_store("fixed");
    let got = files_digest(&store).unwrap();
    eprintln!("digest = {got}");
    assert_eq!(got, BEFORE);
    // And it is stable: a second walk of the unchanged store gives the same bytes.
    assert_eq!(files_digest(&store).unwrap(), got);
}
