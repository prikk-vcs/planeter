//! RFC 004 part 7, T1: the `creating | ready` state of a repository record, and the stage store of an archive creation.
//! Every property is held on both backends (in memory and SQLite), and the migration of a database made before the column
//! existed is tested with a real old-shaped table.

use std::path::PathBuf;
use std::sync::Arc;

use planeter_store::{
    ClaimError, InMemoryMembershipStore, InMemoryRepositoryStore, InMemoryStageStore,
    MembershipStore, Owner, ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore, Role,
    SqliteDb, SqliteMembershipStore, SqliteRepositoryStore, SqliteStageStore, StageLimits,
    StageRecord, StageRefusal, StageState, StageStore, StoreError, UserId, Visibility,
};

fn record(id: &str, owner: Owner, name: &str) -> RepositoryRecord {
    RepositoryRecord {
        repo_id: RepoId::new(id),
        owner,
        name: name.to_owned(),
        visibility: Visibility::Private,
        created_at: 100,
        prikk_format_version: None,
        proposals: ProposalPolicy::Members,
        path: PathBuf::from(format!("/repos/{id}")),
    }
}

fn stores() -> Vec<(&'static str, Arc<dyn RepositoryStore>)> {
    vec![
        ("memory", Arc::new(InMemoryRepositoryStore::new())),
        (
            "sqlite",
            Arc::new(SqliteRepositoryStore::new(
                SqliteDb::open_in_memory().unwrap(),
            )),
        ),
    ]
}

#[test]
fn a_creating_repository_is_invisible_to_every_read() {
    for (which, s) in stores() {
        let alice = Owner::User("alice".into());
        s.reserve(record("r1", alice.clone(), "app")).unwrap();
        assert!(s.get(&RepoId::new("r1")).unwrap().is_none(), "{which}: get");
        assert!(
            s.resolve(&alice, "app").unwrap().is_none(),
            "{which}: resolve"
        );
        assert!(
            s.list_by_owner(&alice).unwrap().is_empty(),
            "{which}: list_by_owner"
        );
        assert_eq!(s.creating().unwrap().len(), 1, "{which}: creating lists it");
        assert_eq!(
            s.known_ids().unwrap(),
            vec![RepoId::new("r1")],
            "{which}: known_ids"
        );
        // It cannot be renamed while it is being created.
        assert!(
            matches!(
                s.rename(&RepoId::new("r1"), alice.clone(), "other".into()),
                Err(StoreError::NotFound)
            ),
            "{which}: rename"
        );
    }
}

#[test]
fn a_reservation_holds_the_name_and_ready_makes_the_repository_visible() {
    for (which, s) in stores() {
        let alice = Owner::User("alice".into());
        s.reserve(record("r1", alice.clone(), "app")).unwrap();
        // Taken: by a second reservation, and by a plain create, in either state.
        assert!(
            matches!(
                s.reserve(record("r2", alice.clone(), "app")),
                Err(StoreError::Conflict(_))
            ),
            "{which}: second reservation"
        );
        assert!(
            matches!(
                s.create(record("r3", alice.clone(), "app")),
                Err(StoreError::Conflict(_))
            ),
            "{which}: create over a reservation"
        );
        s.mark_ready(&RepoId::new("r1")).unwrap();
        assert!(
            s.get(&RepoId::new("r1")).unwrap().is_some(),
            "{which}: visible once ready"
        );
        assert_eq!(
            s.resolve(&alice, "app").unwrap(),
            Some(RepoId::new("r1")),
            "{which}"
        );
        assert_eq!(s.list_by_owner(&alice).unwrap().len(), 1, "{which}");
        assert!(
            s.creating().unwrap().is_empty(),
            "{which}: nothing left creating"
        );
        // Ready exactly once.
        assert!(
            matches!(s.mark_ready(&RepoId::new("r1")), Err(StoreError::NotFound)),
            "{which}: a second mark_ready"
        );
        assert!(
            matches!(
                s.mark_ready(&RepoId::new("nope")),
                Err(StoreError::NotFound)
            ),
            "{which}: an unknown id"
        );
    }
}

#[test]
fn delete_removes_a_record_in_either_state_and_frees_the_name() {
    for (which, s) in stores() {
        let alice = Owner::User("alice".into());
        s.reserve(record("r1", alice.clone(), "app")).unwrap();
        s.delete(&RepoId::new("r1")).unwrap();
        assert!(s.creating().unwrap().is_empty(), "{which}");
        s.create(record("r2", alice.clone(), "app")).unwrap();
        s.delete(&RepoId::new("r2")).unwrap();
        assert!(s.known_ids().unwrap().is_empty(), "{which}");
        assert!(
            matches!(s.delete(&RepoId::new("r2")), Err(StoreError::NotFound)),
            "{which}"
        );
    }
}

#[test]
fn count_owned_counts_ready_and_creating_and_only_the_owners() {
    for (which, s) in stores() {
        let alice = Owner::User("alice".into());
        let acme = Owner::Org("acme".into());
        s.create(record("r1", alice.clone(), "a")).unwrap();
        s.reserve(record("r2", alice.clone(), "b")).unwrap();
        s.create(record("r3", acme.clone(), "c")).unwrap();
        s.reserve(record("r4", acme.clone(), "d")).unwrap();
        assert_eq!(
            s.count_owned(&alice).unwrap(),
            2,
            "{which}: a reservation holds its place"
        );
        assert_eq!(s.count_owned(&acme).unwrap(), 2, "{which}");
        assert_eq!(
            s.count_owned(&Owner::User("bob".into())).unwrap(),
            0,
            "{which}"
        );
        // A user and an organization with one handle are two owners.
        assert_eq!(
            s.count_owned(&Owner::Org("alice".into())).unwrap(),
            0,
            "{which}"
        );
        // Another owner's listing never shows these.
        assert_eq!(
            s.list_by_owner(&alice).unwrap().len(),
            1,
            "{which}: only the ready one"
        );
    }
}

#[test]
fn the_state_survives_a_reopen_of_the_database() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("creation-store-reopen");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("db.sqlite");
    let alice = Owner::User("alice".into());
    {
        let s = SqliteRepositoryStore::new(SqliteDb::open(&file).unwrap());
        s.reserve(record("r1", alice.clone(), "app")).unwrap();
        s.create(record("r2", alice.clone(), "done")).unwrap();
    }
    let s = SqliteRepositoryStore::new(SqliteDb::open(&file).unwrap());
    assert_eq!(s.creating().unwrap().len(), 1);
    assert!(s.resolve(&alice, "app").unwrap().is_none());
    assert!(s.resolve(&alice, "done").unwrap().is_some());
}

#[test]
fn a_database_made_before_the_state_column_gets_it_once_and_every_old_row_is_ready() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("creation-store-migrate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("old.sqlite");
    {
        // The table as part 5 left it: a `proposals` column and no `state`.
        let conn = rusqlite::Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE repositories (
                 repo_id TEXT PRIMARY KEY, owner_kind TEXT NOT NULL, owner TEXT NOT NULL, name TEXT NOT NULL,
                 visibility TEXT NOT NULL, created_at INTEGER NOT NULL, prikk_format_version TEXT, path TEXT NOT NULL,
                 proposals TEXT NOT NULL DEFAULT 'members', UNIQUE (owner_kind, owner, name));
             INSERT INTO repositories VALUES ('old1', 'user', 'alice', 'legacy', 'public', 7, NULL, '/repos/old1', 'members');",
        )
        .unwrap();
    }
    for pass in 0..2 {
        let s = SqliteRepositoryStore::new(SqliteDb::open(&file).unwrap());
        let alice = Owner::User("alice".into());
        assert_eq!(
            s.resolve(&alice, "legacy").unwrap(),
            Some(RepoId::new("old1")),
            "pass {pass}: an old row is ready"
        );
        assert!(s.creating().unwrap().is_empty(), "pass {pass}");
    }
}

#[test]
fn the_database_refuses_a_state_it_does_not_know() {
    let db = SqliteDb::open_in_memory().unwrap();
    let _store = SqliteRepositoryStore::new(db.clone());
    let conn = db.lock().unwrap();
    let r = conn.execute(
        "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, path, state) \
         VALUES ('x', 'user', 'a', 'b', 'public', 1, '/x', 'half-made')",
        [],
    );
    assert!(r.is_err(), "the CHECK constraint holds");
}

#[test]
fn grants_follow_the_repository_and_forgetting_it_removes_them() {
    let sqlite = SqliteMembershipStore::new(SqliteDb::open_in_memory().unwrap());
    let memory = InMemoryMembershipStore::new();
    let stores: Vec<(&str, &dyn MembershipStore)> = vec![("sqlite", &sqlite), ("memory", &memory)];
    for (which, s) in stores {
        let (u, r) = (UserId::new("alice"), RepoId::new("r1"));
        s.grant_user(u.clone(), r.clone(), Role::Admin).unwrap();
        s.grant_user(u.clone(), RepoId::new("r2"), Role::Read)
            .unwrap();
        assert_eq!(
            s.direct_repo_grants(&u, &r).unwrap(),
            vec![Role::Admin],
            "{which}"
        );
        s.forget_repo(&r).unwrap();
        assert!(
            s.direct_repo_grants(&u, &r).unwrap().is_empty(),
            "{which}: gone"
        );
        assert_eq!(
            s.direct_repo_grants(&u, &RepoId::new("r2")).unwrap(),
            vec![Role::Read],
            "{which}: another repository's grants stay"
        );
    }
}

// ---------------------------------------------------------------------------------------------------- stages

fn stage(id: &str, principal: &str, bytes: u64, expires: u64) -> StageRecord {
    StageRecord {
        stage_id: id.to_owned(),
        principal: principal.to_owned(),
        owner: Owner::User(principal.to_owned()),
        name: "app".into(),
        visibility: Visibility::Private,
        proposals: ProposalPolicy::Members,
        archive_digest: "a".repeat(64),
        archive_bytes: bytes,
        created_at: 1_000,
        expires_at: expires,
        state: StageState::Staged,
        report: "{}".into(),
    }
}

fn stage_stores() -> Vec<(&'static str, Arc<dyn StageStore>)> {
    vec![
        ("memory", Arc::new(InMemoryStageStore::new())),
        (
            "sqlite",
            Arc::new(SqliteStageStore::new(SqliteDb::open_in_memory().unwrap()).unwrap()),
        ),
    ]
}

const LIMITS: StageLimits = StageLimits {
    per_principal: 2,
    total_bytes: 100,
};

#[test]
fn stages_obey_the_per_principal_and_the_byte_limits_atomically() {
    for (which, s) in stage_stores() {
        assert_eq!(
            s.put(stage("s1", "alice", 30, 9_000), LIMITS).unwrap(),
            Ok(()),
            "{which}"
        );
        assert_eq!(
            s.put(stage("s2", "alice", 30, 9_000), LIMITS).unwrap(),
            Ok(()),
            "{which}"
        );
        assert_eq!(
            s.put(stage("s3", "alice", 1, 9_000), LIMITS).unwrap(),
            Err(StageRefusal::TooManyForPrincipal { limit: 2 }),
            "{which}: a third stage"
        );
        assert_eq!(
            s.put(stage("s4", "bob", 41, 9_000), LIMITS).unwrap(),
            Err(StageRefusal::BudgetExceeded { limit: 100 }),
            "{which}: 60 held + 41 > 100"
        );
        assert_eq!(
            s.put(stage("s5", "bob", 40, 9_000), LIMITS).unwrap(),
            Ok(()),
            "{which}: exactly the budget"
        );
        assert_eq!(
            s.put(stage("s7", "carol", 1, 9_000), LIMITS).unwrap(),
            Err(StageRefusal::BudgetExceeded { limit: 100 }),
            "{which}: no room left"
        );
        s.remove("s1").unwrap();
        assert_eq!(
            s.put(stage("s6", "alice", 30, 9_000), LIMITS).unwrap(),
            Ok(()),
            "{which}: room after a removal"
        );
        assert_eq!(
            s.put(stage("s6", "alice", 0, 9_000), LIMITS).unwrap(),
            Err(StageRefusal::IdTaken),
            "{which}: an id is used once"
        );
    }
}

#[test]
fn only_the_stager_can_claim_a_stage_and_only_once_at_a_time() {
    for (which, s) in stage_stores() {
        s.put(stage("s1", "alice", 10, 5_000), LIMITS)
            .unwrap()
            .unwrap();
        // Another principal and an unknown id are one answer.
        assert_eq!(
            s.claim("s1", "bob", 2_000).unwrap(),
            Err(ClaimError::NotFound),
            "{which}: a stranger"
        );
        assert_eq!(
            s.claim("zzz", "alice", 2_000).unwrap(),
            Err(ClaimError::NotFound),
            "{which}: unknown"
        );
        let claimed = s.claim("s1", "alice", 2_000).unwrap().unwrap();
        assert_eq!(claimed.state, StageState::Confirming, "{which}");
        assert_eq!(
            s.claim("s1", "alice", 2_000).unwrap(),
            Err(ClaimError::InUse),
            "{which}: a second claim"
        );
        // A claimed stage is not swept as expired while its confirmation runs.
        assert!(s.expired(9_999).unwrap().is_empty(), "{which}");
        s.release("s1").unwrap();
        assert_eq!(
            s.get("s1").unwrap().unwrap().state,
            StageState::Staged,
            "{which}: released"
        );
        // Expired: told as such to the stager (410), and listed for the sweep.
        assert_eq!(
            s.claim("s1", "alice", 5_000).unwrap(),
            Err(ClaimError::Expired),
            "{which}"
        );
        assert_eq!(s.expired(5_000).unwrap().len(), 1, "{which}");
        assert!(s.expired(4_999).unwrap().is_empty(), "{which}");
    }
}

#[test]
fn one_hundred_claims_at_once_have_one_winner() {
    for (which, s) in stage_stores() {
        s.put(stage("s1", "alice", 10, 5_000), LIMITS)
            .unwrap()
            .unwrap();
        let wins = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..100 {
                scope.spawn(|| {
                    if s.claim("s1", "alice", 2_000).unwrap().is_ok() {
                        wins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(wins.load(std::sync::atomic::Ordering::SeqCst), 1, "{which}");
    }
}

#[test]
fn stages_survive_a_reopen() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("creation-stage-reopen");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("db.sqlite");
    {
        let s = SqliteStageStore::new(SqliteDb::open(&file).unwrap()).unwrap();
        s.put(stage("s1", "alice", 10, 5_000), LIMITS)
            .unwrap()
            .unwrap();
        s.claim("s1", "alice", 1_500).unwrap().unwrap();
    }
    let s = SqliteStageStore::new(SqliteDb::open(&file).unwrap()).unwrap();
    let got = s.get("s1").unwrap().unwrap();
    assert_eq!(
        got.state,
        StageState::Confirming,
        "a claim is durable (the start-up sweep decides what it means)"
    );
    assert_eq!(s.all().unwrap().len(), 1);
}
