//! RFC 004 part 7, T1: a repository is made in the order *reserve, lay out, … , mark ready*, and anything that does not reach
//! *ready* is removed whole — by the guard when the process is alive, by the start-up sweep when it was killed. The sweep deletes
//! directories, so its guards are tested with hostile layouts, among them a symbolic link planted in the repository root.
//!
//! These tests need no prikk: a creation is exercised up to the point where prikk would run.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use planeter_core::HostingService;
use planeter_prikk::Sandbox;
use planeter_store::{
    InMemoryRepositoryStore, Owner, ProposalPolicy, RepoId, RepositoryRecord, RepositoryStore,
    Visibility,
};

struct Fixture {
    root: PathBuf,
    store: Arc<InMemoryRepositoryStore>,
    svc: HostingService,
}

fn fixture(name: &str) -> Fixture {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("creation-sweep-{name}"));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("repos");
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(InMemoryRepositoryStore::new());
    let svc = HostingService::new(&root, store.clone() as Arc<dyn RepositoryStore>)
        .with_sandbox(Sandbox::Unconfined);
    Fixture { root, store, svc }
}

fn alice() -> Owner {
    Owner::User("alice".into())
}

fn begin(f: &Fixture, name: &str) -> RepositoryRecord {
    let c = f
        .svc
        .begin_creation(
            alice(),
            name.into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .expect("begin");
    let record = c.record().clone();
    std::mem::forget(c); // simulate a kill: the guard never runs
    record
}

fn dirs_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for shard in std::fs::read_dir(root).unwrap().flatten() {
        if shard.path().is_dir() && !shard.file_type().unwrap().is_symlink() {
            for child in std::fs::read_dir(shard.path()).unwrap().flatten() {
                out.push(child.path());
            }
        }
    }
    out
}

#[test]
fn a_creation_reserves_first_and_is_invisible_until_it_is_finished() {
    let f = fixture("invisible");
    let c = f
        .svc
        .begin_creation(
            alice(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap();
    assert!(c.path().is_dir(), "the directory is laid out");
    assert!(
        f.store.resolve(&alice(), "app").unwrap().is_none(),
        "resolves to nothing"
    );
    assert!(f.store.get(&c.record().repo_id).unwrap().is_none());
    assert!(f.svc.record_by_name(&alice(), "app").unwrap().is_none());
    assert!(f.svc.open(&c.record().repo_id).is_err(), "unroutable");
    assert_eq!(f.store.creating().unwrap().len(), 1);
    let record = c.finish().unwrap();
    assert!(f.svc.record_by_name(&alice(), "app").unwrap().is_some());
    assert_eq!(record.name, "app");
    assert!(f.store.creating().unwrap().is_empty());
}

#[test]
fn the_name_is_taken_from_the_reservation_and_checked_before_a_byte_is_written() {
    let f = fixture("taken");
    let _held = f
        .svc
        .begin_creation(
            alice(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap();
    let before = dirs_under(&f.root).len();
    let err = f
        .svc
        .begin_creation(
            alice(),
            "app".into(),
            Visibility::Public,
            ProposalPolicy::Members,
        )
        .expect_err("taken");
    assert!(
        matches!(err, planeter_core::HostingError::NameTaken { .. }),
        "{err}"
    );
    assert_eq!(
        dirs_under(&f.root).len(),
        before,
        "no second directory was made"
    );
}

#[test]
fn a_creation_that_is_dropped_removes_its_directory_and_its_record_and_frees_the_name() {
    let f = fixture("dropped");
    {
        let c = f
            .svc
            .begin_creation(
                alice(),
                "app".into(),
                Visibility::Private,
                ProposalPolicy::Members,
            )
            .unwrap();
        std::fs::create_dir_all(c.path().join(".prikk/objects")).unwrap();
        std::fs::write(c.path().join(".prikk/objects/x"), b"half an import").unwrap();
    }
    assert!(dirs_under(&f.root).is_empty(), "nothing on disk");
    assert!(
        f.store.known_ids().unwrap().is_empty(),
        "nothing in the store"
    );
    // The name is free again.
    f.svc
        .begin_creation(
            alice(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap()
        .finish()
        .unwrap();
}

#[test]
fn an_explicit_abort_removes_everything_and_says_so() {
    let f = fixture("abort");
    let c = f
        .svc
        .begin_creation(
            alice(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap();
    c.abort().unwrap();
    assert!(dirs_under(&f.root).is_empty());
    assert!(f.store.known_ids().unwrap().is_empty());
}

#[test]
fn a_prikk_that_cannot_run_removes_the_creation_and_leaves_the_name_free() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("creation-sweep-noprikk");
    let _ = std::fs::remove_dir_all(&base);
    let store = Arc::new(InMemoryRepositoryStore::new());
    let svc = HostingService::new(
        base.join("repos"),
        store.clone() as Arc<dyn RepositoryStore>,
    )
    .with_sandbox(Sandbox::Unconfined)
    .with_binary("/nonexistent/prikk");
    let err = svc
        .create_repo(alice(), "app", Visibility::Private)
        .expect_err("no prikk");
    assert!(
        matches!(err, planeter_core::HostingError::Prikk(_)),
        "{err}"
    );
    assert!(store.known_ids().unwrap().is_empty(), "no record");
    let leftover = std::fs::read_dir(base.join("repos"))
        .map(|d| {
            d.flatten()
                .flat_map(|s| std::fs::read_dir(s.path()).unwrap().flatten())
                .count()
        })
        .unwrap_or(0);
    assert_eq!(leftover, 0, "no directory");
}

#[test]
fn the_sweep_removes_every_unfinished_creation_directory_and_record_and_keeps_the_finished() {
    let f = fixture("sweep");
    let done = f
        .svc
        .begin_creation(
            alice(),
            "done".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap()
        .finish()
        .unwrap();
    let a = begin(&f, "half-a");
    let b = begin(&f, "half-b");
    std::fs::create_dir_all(a.path.join(".prikk")).unwrap();
    std::fs::write(a.path.join(".prikk/x"), b"partial").unwrap();
    // A reservation whose directory was never made.
    std::fs::remove_dir_all(&b.path).unwrap();

    let report = f.svc.sweep_unfinished().unwrap();
    assert_eq!(report.removed_creating.len(), 2);
    assert!(report.refused.is_empty(), "{:?}", report.refused);
    assert!(!a.path.exists() && !b.path.exists());
    assert!(f.store.creating().unwrap().is_empty());
    assert!(done.path.is_dir(), "a finished repository is not touched");
    assert!(f.svc.record_by_name(&alice(), "done").unwrap().is_some());
    // The names are free again.
    f.svc
        .begin_creation(
            alice(),
            "half-a".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap()
        .finish()
        .unwrap();
}

#[test]
fn the_sweep_removes_a_directory_no_record_owns_but_only_one_that_looks_like_ours() {
    let f = fixture("orphans");
    let keep = f
        .svc
        .begin_creation(
            alice(),
            "keep".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap()
        .finish()
        .unwrap();
    // An orphan: shard `zz`, id `zzorphan1`.
    let orphan = f.root.join("zz").join("zzorphan1");
    std::fs::create_dir_all(orphan.join(".prikk")).unwrap();
    // Not ours: a directory that is not named like a shard, a stray file, a name with a capital, an id outside its shard.
    let foreign_dir = f.root.join("lost+found").join("abc123");
    std::fs::create_dir_all(&foreign_dir).unwrap();
    let stray_file = f.root.join("notes.txt");
    std::fs::write(&stray_file, b"mine").unwrap();
    let capital = f.root.join("zz").join("Zzcapital");
    std::fs::create_dir_all(&capital).unwrap();
    let wrong_shard = f.root.join("zz").join("qqwrongshard");
    std::fs::create_dir_all(&wrong_shard).unwrap();

    let report = f.svc.sweep_unfinished().unwrap();
    assert_eq!(report.removed_orphans, vec![orphan.clone()]);
    assert!(!orphan.exists());
    for kept in [
        &foreign_dir,
        &stray_file,
        &capital,
        &wrong_shard,
        &keep.path,
    ] {
        assert!(kept.exists(), "{} must stay", kept.display());
    }
}

#[test]
fn a_symbolic_link_planted_in_the_repository_root_is_never_followed() {
    let f = fixture("link");
    let _keep = f
        .svc
        .begin_creation(
            alice(),
            "keep".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap()
        .finish()
        .unwrap();
    // Somewhere that must survive: outside the root, with a child that looks like an orphan repository.
    let outside = f.root.parent().unwrap().join("outside");
    let victim = outside.join("zzvictim1");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("precious"), b"do not delete").unwrap();

    // 1. A shard that is a link to the outside: its children must not be treated as ours.
    std::os::unix::fs::symlink(&outside, f.root.join("zz")).unwrap();
    // 2. A repository id under a real shard that is a link to the outside.
    std::fs::create_dir_all(f.root.join("yy")).unwrap();
    std::os::unix::fs::symlink(&victim, f.root.join("yy").join("yyvictim")).unwrap();

    let report = f.svc.sweep_unfinished().unwrap();
    assert!(
        victim.join("precious").exists(),
        "outside the root, nothing was touched"
    );
    assert!(outside.is_dir());
    assert!(
        report
            .refused
            .iter()
            .any(|(p, why)| p.ends_with("zz") && why.contains("link")),
        "the link shard was refused: {:?}",
        report.refused
    );
    assert!(
        report.refused.iter().any(|(p, _)| p.ends_with("yyvictim")),
        "the link child was refused: {:?}",
        report.refused
    );
    assert!(report.removed_orphans.is_empty());
}

#[test]
fn a_creating_repository_whose_directory_is_a_link_loses_the_link_and_not_its_target() {
    let f = fixture("creating-link");
    let rec = begin(&f, "app");
    let outside = f.root.parent().unwrap().join("outside2");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), b"x").unwrap();
    std::fs::remove_dir_all(&rec.path).unwrap();
    std::os::unix::fs::symlink(&outside, &rec.path).unwrap();

    let report = f.svc.sweep_unfinished().unwrap();
    assert_eq!(report.removed_creating, vec![rec.repo_id.clone()]);
    assert!(outside.join("precious").exists(), "the target survives");
    assert!(
        std::fs::symlink_metadata(&rec.path).is_err(),
        "the link is gone"
    );
    assert!(f.store.known_ids().unwrap().is_empty());
}

#[test]
fn a_creating_repository_whose_shard_is_a_link_out_of_the_root_is_refused_and_stays_recorded() {
    let f = fixture("creating-shard-link");
    let rec = begin(&f, "app");
    let shard = rec.path.parent().unwrap().to_path_buf();
    let outside = f.root.parent().unwrap().join("outside3");
    std::fs::create_dir_all(outside.join(rec.repo_id.as_str())).unwrap();
    std::fs::write(outside.join(rec.repo_id.as_str()).join("precious"), b"x").unwrap();
    std::fs::remove_dir_all(&shard).unwrap();
    std::os::unix::fs::symlink(&outside, &shard).unwrap();

    let report = f.svc.sweep_unfinished().unwrap();
    assert!(outside.join(rec.repo_id.as_str()).join("precious").exists());
    assert!(report.removed_creating.is_empty());
    // Refused twice: the record's directory (it resolves outside the root) and the shard that is a link.
    assert!(
        report
            .refused
            .iter()
            .any(|(_, why)| why.contains("not inside")),
        "{:?}",
        report.refused
    );
    assert!(
        report
            .refused
            .iter()
            .any(|(p, why)| p == &shard && why.contains("link")),
        "{:?}",
        report.refused
    );
    // The record stays, so the next start (after the operator looks) tries again.
    assert_eq!(f.store.creating().unwrap().len(), 1);
}

#[test]
fn a_record_whose_path_is_not_where_the_layout_puts_it_is_not_removed() {
    let f = fixture("wrong-path");
    let outside = f.root.parent().unwrap().join("precious-dir");
    std::fs::create_dir_all(&outside).unwrap();
    f.store
        .reserve(RepositoryRecord {
            repo_id: RepoId::new("abcdef1"),
            owner: alice(),
            name: "app".into(),
            visibility: Visibility::Private,
            created_at: 1,
            prikk_format_version: None,
            proposals: ProposalPolicy::Members,
            path: outside.clone(),
        })
        .unwrap();
    let report = f.svc.sweep_unfinished().unwrap();
    assert!(
        outside.exists(),
        "a path the record names but the layout does not is left alone"
    );
    assert_eq!(report.refused.len(), 1);
    assert!(report.removed_creating.is_empty());
}

#[test]
fn an_empty_store_with_directories_on_disk_is_a_wrong_database_not_an_unfinished_creation() {
    let f = fixture("empty-store");
    let dir = f.root.join("ab").join("abrepo1");
    std::fs::create_dir_all(dir.join(".prikk")).unwrap();
    let report = f.svc.sweep_unfinished().unwrap();
    assert!(
        dir.exists(),
        "nothing is removed when the store holds no repository at all"
    );
    assert!(report.removed_orphans.is_empty());
    assert_eq!(report.kept_because_store_is_empty, vec![dir]);
}

#[test]
fn a_sweep_of_a_clean_server_removes_nothing_and_a_second_sweep_finds_nothing() {
    let f = fixture("clean");
    f.svc
        .begin_creation(
            alice(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap()
        .finish()
        .unwrap();
    for _ in 0..2 {
        let report = f.svc.sweep_unfinished().unwrap();
        assert_eq!(report, planeter_core::SweepReport::default());
    }
    let missing_root = HostingService::new(
        "/nonexistent-root-for-sweep",
        f.store.clone() as Arc<dyn RepositoryStore>,
    );
    assert_eq!(
        missing_root.sweep_unfinished().unwrap(),
        planeter_core::SweepReport::default()
    );
}
