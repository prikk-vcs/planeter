//! IS-45 at the service level: `NoSuchPath` exists only **after** authorization, `tree_existing` says a folder is
//! missing only where prikk can tell, and prikk's own behaviour that makes it possible is pinned: prikk stores
//! files, not directories. Real prikk; **self-skips** without it. Keys live in per-test directories.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use planeter_core::{CallLimits, HostingService, Principal, RawTarget, ReadError, ReadService};
use planeter_prikk::Sandbox;
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};

fn prikk_available() -> bool {
    Command::new("prikk").arg("--version").output().is_ok()
}

/// A repository with `src/main.rs`, `docs/deep/a.txt`, a README and an **empty directory** in the worktree
/// (`src/empty_dir`), sealed (`sealed`) or left queued.
fn repo(name: &str, seal: bool) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("core-missing-{name}"));
    let _ = std::fs::remove_dir_all(&base);
    let (repo, keys) = (base.join("repo"), base.join("keys"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    let prikk = |args: &[&str]| {
        let ok = Command::new("prikk")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &keys)
            .env("XDG_CONFIG_HOME", &keys)
            .status()
            .expect("spawn prikk");
        assert!(ok.success(), "prikk {args:?} failed");
    };
    prikk(&["setup"]);
    std::fs::create_dir_all(repo.join("src/empty_dir")).unwrap();
    std::fs::create_dir_all(repo.join("docs/deep")).unwrap();
    std::fs::write(repo.join("src/main.rs"), b"fn main() {}\n").unwrap();
    std::fs::write(repo.join("docs/deep/a.txt"), b"a\n").unwrap();
    std::fs::write(repo.join("README.md"), b"hello\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    if seal {
        prikk(&["seal", "--allow-no-audit"]);
    }
    repo
}

fn service(repos: Vec<(&str, Visibility, PathBuf)>) -> (ReadService, Owner) {
    let store = Arc::new(InMemoryRepositoryStore::new());
    for (i, (name, visibility, path)) in repos.into_iter().enumerate() {
        store
            .create(RepositoryRecord {
                repo_id: RepoId::new(format!("r{i}")),
                owner: Owner::User("alice".into()),
                name: name.into(),
                visibility,
                created_at: 0,
                prikk_format_version: None,
                path,
            })
            .unwrap();
    }
    let hosting = Arc::new(
        HostingService::new("/unused", store as Arc<dyn RepositoryStore>)
            .with_sandbox(Sandbox::Unconfined),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    (
        ReadService::new(hosting, membership, Arc::new(planeter_core::NullAuditSink)),
        Owner::User("alice".into()),
    )
}

const NONE: CallLimits = CallLimits::NONE;

fn raw<'a>(path: &'a str) -> RawTarget<'a> {
    RawTarget {
        point: None,
        path,
        max_bytes: None,
    }
}

#[test]
fn prikk_stores_files_not_directories_so_an_empty_listing_under_a_sealed_point_is_a_missing_path() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let (svc, owner) = service(vec![("open", Visibility::Public, repo("sealed", true))]);
    let anon = Principal::Anonymous;
    let tree = |prefix: Option<&str>| svc.tree_existing(&anon, &owner, "open", None, prefix, NONE);
    // What is there: a directory, a nested one, a file, the root (no prefix; prikk refuses an empty one).
    for (prefix, entries) in [
        (Some("src"), 1),
        (Some("src/"), 1),
        (Some("docs"), 1),
        (Some("docs/deep"), 1),
        (Some("src/main.rs"), 1),
        (None, 3),
    ] {
        let v = tree(prefix).unwrap_or_else(|e| panic!("{prefix:?}: {e:?}"));
        assert_eq!(v.entries.len(), entries, "{prefix:?}");
    }
    // What is not: a missing folder, a half-typed name, a path below a file, and — the measured fact — a
    // directory that existed **only in the worktree** (an empty directory leaves no trace in prikk).
    for prefix in [
        "nope",
        "nope/x",
        "src/ma",
        "docs/de",
        "src/main.rs/x",
        "src/empty_dir",
    ] {
        assert!(
            matches!(tree(Some(prefix)), Err(ReadError::NoSuchPath)),
            "{prefix}: {:?}",
            tree(Some(prefix))
        );
    }
    // The plain `tree` (the API's) keeps answering an empty listing for the same prefixes.
    for prefix in ["nope", "src/empty_dir"] {
        let v = svc
            .tree(&anon, &owner, "open", None, Some(prefix), NONE)
            .unwrap();
        assert!(v.entries.is_empty(), "{prefix}");
    }
}

#[test]
fn a_queued_only_repository_says_nothing_about_a_path_in_a_listing() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let (svc, owner) = service(vec![("q", Visibility::Public, repo("queued", false))]);
    let anon = Principal::Anonymous;
    // No sealed block: an empty listing, whatever the prefix (there is nothing to be missing from).
    for prefix in [None, Some("src"), Some("nope")] {
        let v = svc
            .tree_existing(&anon, &owner, "q", None, prefix, NONE)
            .unwrap_or_else(|e| panic!("{prefix:?}: {e:?}"));
        assert!(v.entries.is_empty(), "{prefix:?}");
    }
    // `file` and `raw`: the path is not in the sealed history.
    assert!(matches!(
        svc.file(&anon, &owner, "q", None, "README.md", NONE),
        Err(ReadError::NoSuchPath)
    ));
    assert!(matches!(
        svc.raw_file(&anon, &owner, "q", raw("README.md"), NONE),
        Err(ReadError::NoSuchPath)
    ));
}

#[test]
fn no_such_path_is_never_raised_before_authorization() {
    if !prikk_available() {
        eprintln!("skipping: prikk not on PATH");
        return;
    }
    let (svc, owner) = service(vec![
        ("open", Visibility::Public, repo("auth-open", true)),
        ("secret", Visibility::Private, repo("auth-secret", true)),
    ]);
    let anon = Principal::Anonymous;
    // A stranger of a private repository, and a repository that is not there: `NotFound` for every read, for
    // a path that exists and for one that does not.
    for name in ["secret", "ghost"] {
        for path in ["README.md", "no-such-file"] {
            assert!(
                matches!(
                    svc.file(&anon, &owner, name, None, path, NONE),
                    Err(ReadError::NotFound)
                ),
                "{name} file {path}"
            );
            assert!(
                matches!(
                    svc.raw_file(&anon, &owner, name, raw(path), NONE),
                    Err(ReadError::NotFound)
                ),
                "{name} raw {path}"
            );
            assert!(
                matches!(
                    svc.tree_existing(&anon, &owner, name, None, Some(path), NONE),
                    Err(ReadError::NotFound)
                ),
                "{name} tree {path}"
            );
        }
    }
    // The reader of a public repository is told.
    assert!(matches!(
        svc.file(&anon, &owner, "open", None, "no-such-file", NONE),
        Err(ReadError::NoSuchPath)
    ));
    // The owner of the private one is told.
    let alice = Principal::User(planeter_store::UserId::new("alice"));
    assert!(matches!(
        svc.file(&alice, &owner, "secret", None, "no-such-file", NONE),
        Err(ReadError::NoSuchPath)
    ));
    assert!(
        svc.file(&alice, &owner, "secret", None, "README.md", NONE)
            .is_ok()
    );
}
