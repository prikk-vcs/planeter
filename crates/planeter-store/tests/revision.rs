//! RFC 004 D-2: the revision counter — starts at 0, bumps by one, is per repository, and survives a reopen.

use planeter_store::{InMemoryRevisions, RepoId, RevisionStore, SqliteDb, SqliteRevisions};

fn exercise(s: &dyn RevisionStore) {
    let (a, b) = (RepoId::new("a"), RepoId::new("b"));
    assert_eq!(s.get(&a).unwrap(), 0);
    assert_eq!(s.bump(&a).unwrap(), 1);
    assert_eq!(s.bump(&a).unwrap(), 2);
    assert_eq!(s.get(&a).unwrap(), 2);
    assert_eq!(s.get(&b).unwrap(), 0, "another repository is untouched");
    assert_eq!(s.bump(&b).unwrap(), 1);
}

#[test]
fn the_in_memory_counter_behaves() {
    exercise(&InMemoryRevisions::new());
}

#[test]
fn the_sqlite_counter_behaves_survives_a_reopen_and_never_repeats_a_value_under_threads() {
    exercise(&SqliteRevisions::new(SqliteDb::open_in_memory().unwrap()).unwrap());
    let dir = std::env::temp_dir().join(format!("planeter-revisions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("r.db");
    {
        let s = SqliteRevisions::new(SqliteDb::open(&path).unwrap()).unwrap();
        s.bump(&RepoId::new("a")).unwrap();
        s.bump(&RepoId::new("a")).unwrap();
    }
    let s = std::sync::Arc::new(SqliteRevisions::new(SqliteDb::open(&path).unwrap()).unwrap());
    assert_eq!(s.get(&RepoId::new("a")).unwrap(), 2);
    let seen = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..8 {
            sc.spawn(|| {
                for _ in 0..20 {
                    let n = s.bump(&RepoId::new("a")).unwrap();
                    seen.lock().unwrap().push(n);
                }
            });
        }
    });
    assert_eq!(s.get(&RepoId::new("a")).unwrap(), 162);
    let mut v = seen.into_inner().unwrap();
    v.sort_unstable();
    v.dedup();
    assert_eq!(v.len(), 160, "no two bumps returned the same value");
    let _ = std::fs::remove_dir_all(&dir);
}
