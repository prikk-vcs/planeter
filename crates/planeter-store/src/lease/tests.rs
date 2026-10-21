use super::*;

fn rec(id: &str, repo: &str, class: &str, since: u64) -> HolderRecord {
    HolderRecord {
        repo_id: RepoId::new(repo),
        class: class.to_owned(),
        operation: "test".to_owned(),
        holder_id: id.to_owned(),
        boot_id: "boot".to_owned(),
        since_unix: since,
    }
}

fn exercise(store: &dyn LeaseRecordStore) {
    assert!(store.list().unwrap().is_empty());
    store.record(&rec("b", "r2", "X", 20)).unwrap();
    store.record(&rec("a", "r1", "W", 10)).unwrap();
    // Oldest first.
    let ids: Vec<String> = store
        .list()
        .unwrap()
        .into_iter()
        .map(|r| r.holder_id)
        .collect();
    assert_eq!(ids, ["a", "b"]);
    // Recording the same holder again replaces, it does not duplicate.
    store.record(&rec("a", "r1", "W", 11)).unwrap();
    assert_eq!(store.list().unwrap().len(), 2);
    // Clearing is exact, and clearing what is gone is fine.
    store.clear("a").unwrap();
    store.clear("a").unwrap();
    store.clear("never-was").unwrap();
    let left = store.list().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0], rec("b", "r2", "X", 20));
}

#[test]
fn the_in_memory_records_behave() {
    exercise(&InMemoryLeaseRecords::new());
}

#[test]
fn the_sqlite_records_behave_and_the_table_creation_is_idempotent() {
    let db = SqliteDb::open_in_memory().unwrap();
    exercise(&SqliteLeaseRecords::new(db.clone()).unwrap());
    // A second open over the same database neither fails nor loses rows.
    let again = SqliteLeaseRecords::new(db).unwrap();
    assert_eq!(again.list().unwrap().len(), 1);
}
