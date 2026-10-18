//! RFC 011 D-2 (T3): a call that hits its memory ceiling is **never a verdict**. Through the read service
//! it is `ReadError::Unavailable` — not `NotFound` (the repository exists and is readable), not a
//! verification result, never "unverified" — and one line names the verb and the ceiling.
//!
//! Self-skips (with a printed reason) when `prikk` or `prlimit` is not on `PATH`. Uses
//! `Sandbox::Unconfined` plus `prlimit`: the subject is the ceiling and its mapping, not bwrap.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use planeter_core::{
    CallLimits, HostingService, NullAuditSink, Principal, RawTarget, ReadError, ReadService,
};
use planeter_prikk::Sandbox;
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A sealed repository with one 16 MiB binary file; an 8 MiB data ceiling is below what its reads need.
fn big_repo(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let keys = base.join("keys");
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
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let bytes: Vec<u8> = (0..16 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    std::fs::write(repo.join("big.bin"), bytes).unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

fn service(path: PathBuf) -> (ReadService, Owner) {
    let store = Arc::new(InMemoryRepositoryStore::new());
    let owner = Owner::User("alice".into());
    store
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: owner.clone(),
            name: "big".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            path,
        })
        .unwrap();
    let hosting = Arc::new(
        HostingService::new(
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ceiling-mapping-root"),
            store as Arc<dyn RepositoryStore>,
        )
        .with_sandbox(Sandbox::Unconfined),
    );
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    (
        ReadService::new(hosting, membership, Arc::new(NullAuditSink)),
        owner,
    )
}

const LOW: CallLimits = CallLimits::data_ceiling(8 << 20);
const BIG: RawTarget<'static> = RawTarget {
    point: None,
    path: "big.bin",
    max_bytes: Some(32 << 20),
};

fn unavailable_naming_the_ceiling<T: std::fmt::Debug>(r: Result<T, ReadError>, verb: &str) {
    match r {
        Err(ReadError::Unavailable(m)) => {
            assert!(
                m.contains("memory ceiling") && m.contains(verb),
                "{verb}: {m}"
            );
            assert!(m.contains(&(8u64 << 20).to_string()), "{m}");
        }
        other => panic!("{verb}: expected Unavailable (ceiling), got {other:?}"),
    }
}

#[test]
fn a_ceiling_hit_is_unavailable_never_notfound_and_never_a_verification_result() {
    if !have("prikk") || !have("prlimit") {
        eprintln!("skipping: needs prikk and prlimit on PATH");
        return;
    }
    let (svc, owner) = service(big_repo("ceiling-mapping"));
    let anon = Principal::Anonymous;

    // The repository is readable and complete: with no ceiling every read succeeds.
    let free = CallLimits::NONE;
    assert!(svc.verify(&anon, &owner, "big", free).expect("verify").ok);
    assert!(svc.tree(&anon, &owner, "big", None, None, free).is_ok());
    assert!(svc.raw_file(&anon, &owner, "big", BIG, free).is_ok());
    // `file` refuses a 16 MiB file (above the 4 MiB bound) before reading any content (RFC 011 D-3).
    assert!(matches!(
        svc.file(&anon, &owner, "big", None, "big.bin", free),
        Err(ReadError::TooLarge)
    ));

    // Under a ceiling below the need each one is `Unavailable`, naming the verb and the ceiling.
    unavailable_naming_the_ceiling(svc.verify(&anon, &owner, "big", LOW), "verify");
    unavailable_naming_the_ceiling(svc.tree(&anon, &owner, "big", None, None, LOW), "tree");
    // `file` learns the size from a tree pre-check first, so the pre-check is the call that trips.
    unavailable_naming_the_ceiling(svc.file(&anon, &owner, "big", None, "big.bin", LOW), "tree");
    // `raw_file` asks `cat --format json` first and turns a command failure into `NotFound` ("no such
    // path"): a ceiling hit there must NOT be read as a missing path.
    unavailable_naming_the_ceiling(svc.raw_file(&anon, &owner, "big", BIG, LOW), "cat");
}
