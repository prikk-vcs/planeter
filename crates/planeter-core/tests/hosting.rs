//! Integration tests for the hosting model (RFC 001 T6), run against a real prikk binary. They
//! **self-skip** when `prikk` is not on `PATH`. These use [`Sandbox::Unconfined`] deliberately: the
//! subject here is the hosting/layout logic and the "prikk is the source of truth" invariant (ENF-5),
//! not the sandbox — the sandbox itself is covered by `planeter-prikk`'s driver tests. Uses
//! `CARGO_TARGET_TMPDIR` as the repositories root.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use planeter_core::HostingService;
use planeter_prikk::{PrikkRepo, Sandbox};
use planeter_store::{InMemoryRepositoryStore, Owner, RepositoryStore, Visibility};

fn prikk_available() -> bool {
    Command::new("prikk").arg("--version").output().is_ok()
}

fn service(subdir: &str) -> HostingService {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(subdir);
    let _ = std::fs::remove_dir_all(&root);
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    HostingService::new(root, store).with_sandbox(Sandbox::Unconfined)
}

#[test]
fn create_lays_out_by_id_and_prikk_is_the_source_of_truth() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let svc = service("hosting-create");
    let record = svc
        .create_repo(Owner::User("alice".into()), "app", Visibility::Private)
        .expect("create repo");

    // D-4: the on-disk path is <repos-root>/<shard>/<repo_id> and holds a real .prikk.
    assert!(record.path.join(".prikk").is_dir(), "prikk init ran");
    assert!(
        record.path.ends_with(record.repo_id.as_str()),
        "path keyed by repo_id: {:?}",
        record.path
    );

    // ENF-5: re-derive a fact straight from the on-disk repository, WITHOUT the forge store — proving
    // prikk, not planeter's metadata, is the source of truth. A fresh driver bound only to the path.
    let direct = planeter_prikk::CliPrikkRepo::new(&record.path).with_sandbox(Sandbox::Unconfined);
    let status = direct.status().expect("status from disk");
    assert_eq!(status.schema_version, "status-report-v1");
}

#[test]
fn open_by_id_and_by_name_return_a_working_driver() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let svc = service("hosting-open");
    let record = svc
        .create_repo(Owner::Org("acme".into()), "core", Visibility::Public)
        .expect("create repo");

    let by_id = svc.open(&record.repo_id).expect("open by id");
    assert!(by_id.verify().is_ok(), "verify a freshly hosted repo");

    let by_name = svc
        .open_by_name(&Owner::Org("acme".into()), "core")
        .expect("open by name");
    assert!(by_name.status().is_ok());
}

#[test]
fn duplicate_name_is_refused_before_touching_disk() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let svc = service("hosting-dup");
    svc.create_repo(Owner::User("alice".into()), "app", Visibility::Private)
        .expect("first create");
    let err = svc
        .create_repo(Owner::User("alice".into()), "app", Visibility::Private)
        .expect_err("second create must fail");
    assert!(
        matches!(err, planeter_core::HostingError::NameTaken { .. }),
        "got {err:?}"
    );
}
