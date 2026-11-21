//! RFC 013 D-4: the table of where each trusted key came from — one contract, run over both stores.

use planeter_store::{
    DETAIL_MAX_CHARS, InMemoryKeyEventStore, KeyEvent, KeyEventStore, KeySource, RepoId, SqliteDb,
    SqliteKeyEventStore,
};

fn hex(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn event(repo: &str, n: u8, source: KeySource, at: u64) -> KeyEvent {
    KeyEvent {
        repo_id: RepoId::new(repo),
        key_id: format!("ed25519-{n:02x}"),
        public_key: hex(n),
        source,
        detail: String::new(),
        delivery_id: None,
        recorded_at: at,
        backfilled: false,
    }
}

fn contract(s: &dyn KeyEventStore) {
    let r = RepoId::new("r1");
    assert!(!s.has_any(&r).unwrap());
    assert!(s.events(&r).unwrap().is_empty());

    // one row per (repo, key): the first record wins, and a later one never rewrites it
    assert!(s.record(&event("r1", 1, KeySource::Creation, 100)).unwrap());
    let mut again = event("r1", 1, KeySource::Operator, 200);
    again.detail = "a later claim".to_owned();
    assert!(
        !s.record(&again).unwrap(),
        "the key already has its row: nothing is written"
    );
    let rows = s.events(&r).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, KeySource::Creation);
    assert_eq!(rows[0].recorded_at, 100);
    assert_eq!(rows[0].detail, "");
    assert!(s.has_any(&r).unwrap());

    // the same key in another repository is its own row
    assert!(s.record(&event("r2", 1, KeySource::Creation, 100)).unwrap());

    // the order: recorded_at, then key id
    assert!(s.record(&event("r1", 3, KeySource::Creation, 100)).unwrap());
    assert!(s.record(&event("r1", 2, KeySource::Delivery, 100)).unwrap());
    assert!(s.record(&event("r1", 9, KeySource::Operator, 50)).unwrap());
    let ids: Vec<String> = s
        .events(&r)
        .unwrap()
        .into_iter()
        .map(|e| e.key_id)
        .collect();
    assert_eq!(
        ids,
        ["ed25519-09", "ed25519-01", "ed25519-02", "ed25519-03"]
    );
    // the other repository is untouched
    assert_eq!(s.events(&RepoId::new("r2")).unwrap().len(), 1);

    // every field survives
    let mut full = event("r3", 7, KeySource::Delivery, 777);
    full.delivery_id = Some("dl-0123".to_owned());
    full.backfilled = true;
    assert!(s.record(&full).unwrap());
    assert_eq!(s.events(&RepoId::new("r3")).unwrap(), vec![full]);
}

fn long_and_hostile_details_are_cut_not_refused(s: &dyn KeyEventStore) {
    let r = RepoId::new("rd");
    let mut e = event("rd", 1, KeySource::Operator, 1);
    // 300 characters, multi-byte ones included: cut at a character boundary, never refused
    e.detail = "鍵".repeat(150) + &"x".repeat(150);
    assert!(s.record(&e).unwrap());
    let kept = &s.events(&r).unwrap()[0].detail;
    assert_eq!(kept.chars().count(), DETAIL_MAX_CHARS);
    assert!(kept.starts_with('鍵'));
    // hostile text is kept as text (the reader makes it display-safe) and counts as characters
    let mut h = event("rd", 2, KeySource::Operator, 2);
    h.detail = "\u{202E}evil\u{0}\n".repeat(100);
    assert!(s.record(&h).unwrap());
    let kept = s
        .events(&r)
        .unwrap()
        .into_iter()
        .find(|x| x.key_id == "ed25519-02")
        .unwrap();
    assert_eq!(kept.detail.chars().count(), DETAIL_MAX_CHARS);
}

fn a_malformed_key_is_refused(s: &dyn KeyEventStore) {
    for bad in ["", "ab", &"A".repeat(64), &"g".repeat(64), &"a".repeat(65)] {
        let mut e = event("rm", 1, KeySource::Creation, 1);
        e.public_key = bad.to_owned();
        assert!(s.record(&e).is_err(), "{bad:?}");
    }
    assert!(!s.has_any(&RepoId::new("rm")).unwrap());
}

fn the_fill_is_all_or_none_and_only_for_a_repository_with_no_row(s: &dyn KeyEventStore) {
    let r = RepoId::new("rf");
    let fill: Vec<KeyEvent> = (1..=3)
        .map(|n| {
            let mut e = event("rf", n, KeySource::Creation, 10);
            e.backfilled = true;
            e
        })
        .collect();
    assert!(s.record_all_if_none(&r, &fill).unwrap());
    assert_eq!(s.events(&r).unwrap().len(), 3);
    assert!(s.events(&r).unwrap().iter().all(|e| e.backfilled));
    // a second fill does nothing, even with other keys
    let more = vec![event("rf", 9, KeySource::Creation, 20)];
    assert!(!s.record_all_if_none(&r, &more).unwrap());
    assert_eq!(s.events(&r).unwrap().len(), 3);
    // a repository with one row of its own (a key added later) is not filled
    let r2 = RepoId::new("rg");
    assert!(s.record(&event("rg", 5, KeySource::Operator, 5)).unwrap());
    assert!(
        !s.record_all_if_none(&r2, &[event("rg", 6, KeySource::Creation, 6)])
            .unwrap()
    );
    assert_eq!(s.events(&r2).unwrap().len(), 1);
    // one bad event refuses the whole fill: nothing is written
    let r3 = RepoId::new("rh");
    let mut bad = event("rh", 2, KeySource::Creation, 1);
    bad.public_key = "nope".to_owned();
    assert!(
        s.record_all_if_none(&r3, &[event("rh", 1, KeySource::Creation, 1), bad])
            .is_err()
    );
    assert!(!s.has_any(&r3).unwrap());
    // an event of another repository is refused
    assert!(
        s.record_all_if_none(&r3, &[event("other", 1, KeySource::Creation, 1)])
            .is_err()
    );
    // an empty fill writes nothing and the repository still has no row
    assert!(s.record_all_if_none(&r3, &[]).unwrap());
    assert!(!s.has_any(&r3).unwrap());
}

fn run_all(make: impl Fn() -> Box<dyn KeyEventStore>) {
    contract(&*make());
    long_and_hostile_details_are_cut_not_refused(&*make());
    a_malformed_key_is_refused(&*make());
    the_fill_is_all_or_none_and_only_for_a_repository_with_no_row(&*make());
}

#[test]
fn the_in_memory_store_keeps_the_contract() {
    run_all(|| Box::new(InMemoryKeyEventStore::new()));
}

#[test]
fn the_sqlite_store_keeps_the_same_contract() {
    run_all(|| Box::new(SqliteKeyEventStore::new(SqliteDb::open_in_memory().unwrap()).unwrap()));
}

#[test]
fn the_table_is_created_again_without_harm_and_keeps_its_rows() {
    let dir = std::env::temp_dir().join(format!("planeter-keyev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.sqlite");
    {
        let s = SqliteKeyEventStore::new(SqliteDb::open(&path).unwrap()).unwrap();
        assert!(s.record(&event("r1", 1, KeySource::Creation, 1)).unwrap());
    }
    let s = SqliteKeyEventStore::new(SqliteDb::open(&path).unwrap()).unwrap();
    assert_eq!(s.events(&RepoId::new("r1")).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
