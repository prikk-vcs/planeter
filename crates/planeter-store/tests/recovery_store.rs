//! RFC 004 part 4: the recovery records. One suite, run against both implementations.

use planeter_store::{
    DETAIL_MAX_BYTES, InMemoryRecoveries, RecoveryStore, RepoId, SqliteDb, SqliteRecoveries,
    StoreError,
};

fn repo() -> RepoId {
    RepoId::new("r1")
}

macro_rules! suite {
    ($($name:ident),* $(,)?) => {
        mod in_memory { $( #[test] fn $name() { super::$name(&planeter_store::InMemoryRecoveries::new()); } )* }
        mod sqlite { $( #[test] fn $name() { super::$name(&super::sqlite()); } )* }
    };
}

fn sqlite() -> SqliteRecoveries {
    SqliteRecoveries::new(SqliteDb::open_in_memory().unwrap()).unwrap()
}

suite!(
    a_run_is_written_step_by_step_and_read_back,
    a_step_written_before_it_is_taken_shows_where_an_interrupted_run_stopped,
    ids_and_step_numbers_are_used_once,
    unknown_runs_and_steps_are_not_found,
    recent_runs_are_newest_first_per_repository_and_limited,
    long_text_is_bounded_on_a_character_boundary,
);

fn a_run_is_written_step_by_step_and_read_back(s: &dyn RecoveryStore) {
    s.begin("rec-1", &repo(), "automatic", 10).unwrap();
    s.begin_step("rec-1", 1, "witnesses", 10).unwrap();
    s.end_step("rec-1", 1, "ok", "flag set; no live holder", 11)
        .unwrap();
    s.begin_step("rec-1", 2, "unlock-force", 12).unwrap();
    s.end_step(
        "rec-1",
        2,
        "ok",
        "/srv/r/.prikk/active/default/active.lock cleared",
        13,
    )
    .unwrap();
    s.end("rec-1", "recovered", 14).unwrap();
    let r = s.get("rec-1").unwrap().unwrap();
    assert_eq!(
        (
            r.actor.as_str(),
            r.started_at,
            r.ended_at,
            r.outcome.as_deref()
        ),
        ("automatic", 10, Some(14), Some("recovered"))
    );
    assert_eq!(r.steps.len(), 2);
    assert_eq!(
        (
            r.steps[1].name.as_str(),
            r.steps[1].outcome.as_deref(),
            r.steps[1].ended_at
        ),
        ("unlock-force", Some("ok"), Some(13))
    );
    assert!(r.steps[1].detail.contains("active.lock"));
}

fn a_step_written_before_it_is_taken_shows_where_an_interrupted_run_stopped(s: &dyn RecoveryStore) {
    s.begin("rec-1", &repo(), "alice", 10).unwrap();
    s.begin_step("rec-1", 1, "witnesses", 10).unwrap();
    s.end_step("rec-1", 1, "ok", "", 11).unwrap();
    s.begin_step("rec-1", 2, "unlock-force", 12).unwrap();
    // The process dies here: no end_step, no end.
    let r = s.get("rec-1").unwrap().unwrap();
    assert_eq!(r.outcome, None);
    assert_eq!(r.ended_at, None);
    assert_eq!(
        (
            r.steps[1].name.as_str(),
            r.steps[1].ended_at,
            r.steps[1].outcome.clone()
        ),
        ("unlock-force", None, None)
    );
}

fn ids_and_step_numbers_are_used_once(s: &dyn RecoveryStore) {
    s.begin("rec-1", &repo(), "a", 1).unwrap();
    assert!(matches!(
        s.begin("rec-1", &repo(), "a", 2),
        Err(StoreError::Conflict(_))
    ));
    s.begin_step("rec-1", 1, "x", 1).unwrap();
    assert!(matches!(
        s.begin_step("rec-1", 1, "y", 2),
        Err(StoreError::Conflict(_))
    ));
}

fn unknown_runs_and_steps_are_not_found(s: &dyn RecoveryStore) {
    assert_eq!(s.get("nope").unwrap(), None);
    assert!(matches!(
        s.begin_step("nope", 1, "x", 1),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.end_step("nope", 1, "ok", "", 1),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.end("nope", "recovered", 1),
        Err(StoreError::NotFound)
    ));
    s.begin("rec-1", &repo(), "a", 1).unwrap();
    assert!(matches!(
        s.end_step("rec-1", 9, "ok", "", 1),
        Err(StoreError::NotFound)
    ));
}

fn recent_runs_are_newest_first_per_repository_and_limited(s: &dyn RecoveryStore) {
    for (i, repo_id) in [(1u64, "r1"), (2, "r1"), (3, "r2"), (4, "r1")] {
        s.begin(&format!("rec-{i}"), &RepoId::new(repo_id), "a", i * 10)
            .unwrap();
        s.begin_step(&format!("rec-{i}"), 1, "x", i * 10).unwrap();
    }
    let ids = |v: Vec<planeter_store::RecoveryRecord>| {
        v.into_iter().map(|r| r.recovery_id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(s.recent(&repo(), 10).unwrap()),
        vec!["rec-4", "rec-2", "rec-1"]
    );
    assert_eq!(ids(s.recent(&repo(), 2).unwrap()), vec!["rec-4", "rec-2"]);
    assert_eq!(
        ids(s.recent(&RepoId::new("r2"), 10).unwrap()),
        vec!["rec-3"]
    );
    assert!(
        s.recent(&RepoId::new("none"), 10)
            .unwrap()
            .iter()
            .next()
            .is_none()
    );
    assert_eq!(
        s.recent(&repo(), 1).unwrap()[0].steps.len(),
        1,
        "steps come with their run"
    );
}

fn long_text_is_bounded_on_a_character_boundary(s: &dyn RecoveryStore) {
    s.begin("rec-1", &repo(), "a", 1).unwrap();
    s.begin_step("rec-1", 1, "doctor", 1).unwrap();
    let text = "é".repeat(DETAIL_MAX_BYTES); // two bytes a character
    s.end_step("rec-1", 1, "ok", &text, 2).unwrap();
    let detail = s.get("rec-1").unwrap().unwrap().steps[0].detail.clone();
    assert!(detail.len() < DETAIL_MAX_BYTES + 64, "{}", detail.len());
    assert!(detail.contains("bytes more]"));
    assert!(detail.starts_with("éé"));
    let _ = InMemoryRecoveries::new();
}

#[test]
fn the_tables_survive_a_reopen_and_creating_them_twice_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("planeter-recoveries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.db");
    for round in 0..3 {
        let s = SqliteRecoveries::new(SqliteDb::open(&path).unwrap()).unwrap();
        if round == 0 {
            s.begin("rec-1", &repo(), "a", 1).unwrap();
            s.begin_step("rec-1", 1, "x", 1).unwrap();
        }
        assert_eq!(
            s.get("rec-1").unwrap().unwrap().steps.len(),
            1,
            "round {round}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
