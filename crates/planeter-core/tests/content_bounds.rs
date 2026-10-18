//! RFC 011 D-3 / D-4 and the handoff-1 follow-up (T6, T8): the shared content bound and the honest mapping
//! of failures on the raw read.
//!
//! * `file` and raw refuse a file above `CONTENT_MAX_BYTES` with `TooLarge` — `file` before reading any
//!   content, raw from the metadata before `cat` runs;
//! * a **timeout** and a **ceiling hit** on raw's metadata read are `Unavailable`, never `NotFound`; only
//!   prikk's own "no such path" is `NotFound`.
//!
//! Tests with a real prikk self-skip (printed reason) without it; the timeout test also needs bubblewrap.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use planeter_core::admission::CONTENT_MAX_BYTES;
use planeter_core::{
    CallLimits, HostingService, NullAuditSink, Principal, RawTarget, ReadError, ReadService,
};
use planeter_prikk::Sandbox;
use planeter_prikk::sandbox::BwrapConfig;
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("content-bounds-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn service(path: PathBuf, sandbox: Sandbox, binary: Option<PathBuf>) -> (ReadService, Owner) {
    let store = Arc::new(InMemoryRepositoryStore::new());
    let owner = Owner::User("alice".into());
    store
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: owner.clone(),
            name: "repo".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            path,
        })
        .unwrap();
    let mut hosting =
        HostingService::new(scratch("hosting-root"), store as Arc<dyn RepositoryStore>)
            .with_sandbox(sandbox);
    if let Some(b) = binary {
        hosting = hosting.with_binary(b);
    }
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    (
        ReadService::new(Arc::new(hosting), membership, Arc::new(NullAuditSink)),
        owner,
    )
}

/// A sealed repository with a 1 MiB file, a 5 MiB file (above the 4 MiB bound) and a README.
fn sized_repo(name: &str) -> PathBuf {
    let base = scratch(name);
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
            .unwrap();
        assert!(ok.success(), "prikk {args:?}");
    };
    prikk(&["setup"]);
    let mut x: u64 = 0xDEAD_BEEF_CAFE_F00D;
    let mut bytes = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    };
    std::fs::write(repo.join("small.bin"), bytes(1 << 20)).unwrap();
    std::fs::write(repo.join("big.bin"), bytes(5 << 20)).unwrap();
    std::fs::write(repo.join("README.md"), b"hello\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

const NONE: CallLimits = CallLimits::NONE;

#[test]
fn file_and_raw_refuse_above_the_shared_bound_and_serve_below_it() {
    if !have("prikk") {
        eprintln!("skipping: prikk is not on PATH");
        return;
    }
    let (svc, owner) = service(sized_repo("bounds"), Sandbox::Unconfined, None);
    let anon = Principal::Anonymous;
    let cap = Some(CONTENT_MAX_BYTES);
    let raw = |path: &'static str, max: Option<u64>| {
        svc.raw_file(
            &anon,
            &owner,
            "repo",
            RawTarget {
                point: None,
                path,
                max_bytes: max,
            },
            NONE,
        )
    };

    // `file`
    assert!(
        svc.file(&anon, &owner, "repo", None, "small.bin", NONE)
            .is_ok()
    );
    assert!(
        svc.file(&anon, &owner, "repo", None, "README.md", NONE)
            .is_ok()
    );
    assert!(matches!(
        svc.file(&anon, &owner, "repo", None, "big.bin", NONE),
        Err(ReadError::TooLarge)
    ));
    assert!(matches!(
        svc.file(&anon, &owner, "repo", None, "no-such-file", NONE),
        Err(ReadError::NotFound)
    ));

    // raw: the same constant; an over-cap file is `TooLarge`, not `Unavailable`.
    assert_eq!(raw("small.bin", cap).expect("small").bytes.len(), 1 << 20);
    assert!(matches!(raw("big.bin", cap), Err(ReadError::TooLarge)));
    assert!(matches!(raw("no-such-file", cap), Err(ReadError::NotFound)));
    // No cap: the file is served whole (the UI/API choose their cap; the read does not invent one).
    assert_eq!(raw("big.bin", None).expect("uncapped").bytes.len(), 5 << 20);
}

#[test]
fn a_timeout_on_raws_metadata_read_is_unavailable_not_notfound() {
    // A stand-in prikk that answers the version pin and then hangs; the deadline fires after 1 s.
    let sandbox = Sandbox::Bwrap(BwrapConfig {
        timeout_secs: 1,
        ..BwrapConfig::default()
    });
    let probe = scratch("probe");
    let sandbox_ok = matches!(
        sandbox.run(OsStr::new("/usr/bin/true"), &probe, &[], CallLimits::NONE),
        Ok(r) if r.status.success()
    );
    if !sandbox_ok {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    let dir = scratch("standin");
    let script = dir.join("prikk");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.52.0';; *) sleep 30;; esac\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let (svc, owner) = service(repo, sandbox, Some(script));
    let r = svc.raw_file(
        &Principal::Anonymous,
        &owner,
        "repo",
        RawTarget {
            point: None,
            path: "x",
            max_bytes: Some(CONTENT_MAX_BYTES),
        },
        NONE,
    );
    match r {
        Err(ReadError::Unavailable(m)) => assert!(m.contains("124"), "the timeout status: {m}"),
        other => panic!("a timeout must be Unavailable, got {other:?}"),
    }
}
