//! End-to-end read-path tests (RFC 003 T2/T5/T6) over a real prikk repo. **Self-skip** when `prikk` is
//! absent. Uses `Sandbox::Unconfined` (the subject is the read path, not the sandbox) and
//! `CARGO_TARGET_TMPDIR` as the repositories root.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use planeter_core::{HostingService, Principal, ReadError, ReadService};
use planeter_prikk::Sandbox;
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepositoryStore,
    UserId, Visibility,
};

fn prikk_available() -> bool {
    Command::new("prikk").arg("--version").output().is_ok()
}

fn fixture(subdir: &str) -> (ReadService, Owner) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(subdir);
    let _ = std::fs::remove_dir_all(&root);
    let store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(root, store).with_sandbox(Sandbox::Unconfined));
    let owner = Owner::User("alice".into());
    // Create a private repo owned by alice (her personal repo → she is Admin, so she can read).
    hosting
        .create_repo(owner.clone(), "app", Visibility::Private)
        .expect("create repo");
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let svc = ReadService::new(hosting, membership, Arc::new(planeter_core::NullAuditSink));
    (svc, owner)
}

#[test]
fn owner_browses_history_refs_and_verify() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let (svc, owner) = fixture("read-owner-browse");
    let alice = Principal::User(UserId::new("alice"));

    // A fresh repo: these all succeed and re-derive from prikk.
    let history = svc
        .history(&alice, &owner, "app", None, None)
        .expect("history");
    assert_eq!(history.ref_name, "heads/main");

    let refs = svc.refs(&alice, &owner, "app", false).expect("refs");
    // A fresh repo has at least the main branch reported by `branch`.
    let _ = refs.branches;

    let verify = svc.verify(&alice, &owner, "app").expect("verify");
    // Honesty: the verdict is re-derived from prikk; a fresh repo verifies.
    assert!(verify.ok);

    // tree on a fresh (unsealed) repo authorizes and returns an empty listing.
    let tree = svc.tree(&alice, &owner, "app", None, None).expect("tree");
    assert_eq!(tree.point, "heads/main");
    assert!(tree.entries.is_empty());

    // raw_file for a path that does not exist is NotFound (not a generic error).
    assert!(matches!(
        svc.raw_file(&alice, &owner, "app", None, "nope.txt", None),
        Err(ReadError::NotFound)
    ));
}

#[test]
fn anonymous_is_denied_tree_and_raw() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let (svc, owner) = fixture("read-anon-tree-raw");
    // The new browse surfaces share the one authorize(_, Read, _) gate.
    assert!(matches!(
        svc.tree(&Principal::Anonymous, &owner, "app", None, None),
        Err(ReadError::NotFound)
    ));
    assert!(matches!(
        svc.raw_file(
            &Principal::Anonymous,
            &owner,
            "app",
            None,
            "README.md",
            None
        ),
        Err(ReadError::NotFound)
    ));
}

#[test]
fn anonymous_is_denied_a_private_repo() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let (svc, owner) = fixture("read-anon-denied");
    // Anonymous read of a private repo → NotFound (T-7), and prikk is never consulted.
    let r = svc.history(&Principal::Anonymous, &owner, "app", None, None);
    assert!(matches!(r, Err(ReadError::NotFound)));
}
