//! RFC 005 D-9: the repository's `proposals` setting — the validated parser, the default, setting it, the
//! idempotent migration of a database made before the column existed, and that a stored value that is not one
//! of the two words is refused rather than defaulted.

use planeter_store::{
    Owner, ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore, SqliteDb,
    SqliteRepositoryStore, StoreError, UnknownProposalPolicy, Visibility,
};
use std::path::PathBuf;

fn record(id: &str, owner: Owner, name: &str) -> RepositoryRecord {
    RepositoryRecord {
        repo_id: RepoId::new(id),
        owner,
        name: name.to_owned(),
        visibility: Visibility::Private,
        created_at: 42,
        prikk_format_version: None,
        proposals: Default::default(),
        path: PathBuf::from(format!("/repos/{id}")),
    }
}

#[test]
fn the_proposals_parser_accepts_the_two_words_and_refuses_everything_else() {
    assert_eq!(
        ProposalPolicy::parse("members"),
        Ok(ProposalPolicy::Members)
    );
    assert_eq!(
        ProposalPolicy::parse("any-authenticated"),
        Ok(ProposalPolicy::AnyAuthenticated)
    );
    for bad in [
        "",
        "Members",
        "MEMBERS",
        "any",
        "anyone",
        "any_authenticated",
        " members",
        "members ",
        "none",
        "true",
    ] {
        assert_eq!(
            ProposalPolicy::parse(bad),
            Err(UnknownProposalPolicy(bad.to_owned())),
            "{bad:?} must be refused, not defaulted"
        );
    }
    // The text form round-trips, and the default is `members`.
    for p in [ProposalPolicy::Members, ProposalPolicy::AnyAuthenticated] {
        assert_eq!(ProposalPolicy::parse(p.as_str()), Ok(p));
    }
    assert_eq!(ProposalPolicy::default(), ProposalPolicy::Members);
}

#[test]
fn the_proposals_setting_defaults_to_members_is_settable_and_survives_reopen() {
    let dir = std::env::temp_dir().join(format!("planeter-proposals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    {
        let s = SqliteRepositoryStore::new(SqliteDb::open(&path).unwrap());
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        assert_eq!(
            s.get(&RepoId::new("r1")).unwrap().unwrap().proposals,
            ProposalPolicy::Members
        );
        s.set_proposals(&RepoId::new("r1"), ProposalPolicy::AnyAuthenticated)
            .unwrap();
        assert!(matches!(
            s.set_proposals(&RepoId::new("nope"), ProposalPolicy::Members),
            Err(StoreError::NotFound)
        ));
    }
    let s = SqliteRepositoryStore::new(SqliteDb::open(&path).unwrap());
    assert_eq!(
        s.get(&RepoId::new("r1")).unwrap().unwrap().proposals,
        ProposalPolicy::AnyAuthenticated
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_database_made_before_the_column_existed_gets_it_once_and_keeps_its_rows() {
    let dir = std::env::temp_dir().join(format!("planeter-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.db");
    // The 0.4.0 shape: no `proposals` column, one row.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE repositories (
                repo_id TEXT PRIMARY KEY, owner_kind TEXT NOT NULL, owner TEXT NOT NULL, name TEXT NOT NULL,
                visibility TEXT NOT NULL, created_at INTEGER NOT NULL, prikk_format_version TEXT,
                path TEXT NOT NULL, UNIQUE (owner_kind, owner, name));
             INSERT INTO repositories VALUES ('old1', 'user', 'alice', 'legacy', 'public', 7, NULL, '/repos/old1');",
        )
        .unwrap();
    }
    for round in 0..3 {
        // Opening applies the schema and the migration; a second and third open change nothing.
        let db = SqliteDb::open(&path).unwrap();
        let s = SqliteRepositoryStore::new(db);
        let got = s.get(&RepoId::new("old1")).unwrap().unwrap();
        assert_eq!(got.name, "legacy", "round {round}");
        assert_eq!(
            got.proposals,
            ProposalPolicy::Members,
            "the default for an old row, round {round}"
        );
    }
    // Exactly one `proposals` column.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('repositories') WHERE name = 'proposals'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stored_value_that_is_not_a_proposals_setting_is_refused_not_defaulted() {
    let db = SqliteDb::open_in_memory().unwrap();
    let s = SqliteRepositoryStore::new(db.clone());
    s.create(record("r1", Owner::User("alice".into()), "app"))
        .unwrap();
    db.lock()
        .unwrap()
        .execute("UPDATE repositories SET proposals = 'anyone'", [])
        .unwrap();
    assert!(
        matches!(s.get(&RepoId::new("r1")), Err(StoreError::Backend(m)) if m.contains("unknown proposals setting"))
    );
}

#[test]
fn the_in_memory_store_behaves_the_same() {
    let s = planeter_store::InMemoryRepositoryStore::new();
    s.create(record("r1", Owner::User("alice".into()), "app"))
        .unwrap();
    assert_eq!(
        s.get(&RepoId::new("r1")).unwrap().unwrap().proposals,
        ProposalPolicy::Members
    );
    s.set_proposals(&RepoId::new("r1"), ProposalPolicy::AnyAuthenticated)
        .unwrap();
    assert_eq!(
        s.get(&RepoId::new("r1")).unwrap().unwrap().proposals,
        ProposalPolicy::AnyAuthenticated
    );
    assert!(matches!(
        s.set_proposals(&RepoId::new("nope"), ProposalPolicy::Members),
        Err(StoreError::NotFound)
    ));
}
