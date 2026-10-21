use super::*;
use std::io::Cursor;
use std::thread;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("planeter-artifacts-{name}-{}", std::process::id()));
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
    let kept: Vec<Digest> = (0..5u8).map(|i| put_bytes(&store, &[i; 150])).collect();
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
