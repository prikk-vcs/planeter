//! IS-55 on the read path: a prikk call that prints more than its verb's limit is `Unavailable` on every read (history, refs, tree,
//! raw file, verify), never an empty listing and never `NoSuchPath`. A stand-in `prikk` prints 40 MB for each verb (the limits are
//! 8 MiB for a default `log`, 16 MiB for `branch`, 1 MiB for `cat` metadata, 32 MiB for `tree` and `verify`).

use std::path::PathBuf;
use std::sync::Arc;

use planeter_core::{
    CallLimits, HostingService, NullAuditSink, Principal, RawTarget, ReadError, ReadService,
};
use planeter_prikk::Sandbox;
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, Owner, RepoId,
    RepositoryRecord, RepositoryStore, Visibility,
};

const NONE: CallLimits = CallLimits::NONE;

#[test]
fn a_read_whose_prikk_call_prints_too_much_is_unavailable_never_empty() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("output-cap-read");
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let script = base.join("prikk");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'prikk 0.54.1' ;;\n  *) head -c 40000000 /dev/zero | tr '\\0' ' ' ;;\nesac\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
            proposals: Default::default(),
            path: repo,
        })
        .unwrap();
    let hosting = HostingService::new(base.join("root"), store as Arc<dyn RepositoryStore>)
        .with_sandbox(Sandbox::Unconfined)
        .with_binary(script);
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let svc = ReadService::new(Arc::new(hosting), membership, Arc::new(NullAuditSink));
    let anon = Principal::Anonymous;

    let unavailable = |what: &str, r: Result<(), ReadError>| match r {
        Err(ReadError::Unavailable(text)) => {
            assert!(text.contains("more than"), "{what}: {text}")
        }
        other => panic!("{what}: expected Unavailable, got {other:?}"),
    };
    unavailable(
        "history",
        svc.history(&anon, &owner, "repo", None, None, NONE)
            .map(drop),
    );
    unavailable(
        "refs",
        svc.refs(&anon, &owner, "repo", true, NONE).map(drop),
    );
    unavailable(
        "tree",
        svc.tree(&anon, &owner, "repo", None, None, NONE).map(drop),
    );
    unavailable(
        "raw file",
        svc.raw_file(
            &anon,
            &owner,
            "repo",
            RawTarget {
                point: None,
                path: "x",
                max_bytes: Some(1000),
            },
            NONE,
        )
        .map(drop),
    );
    unavailable("verify", svc.verify(&anon, &owner, "repo", NONE).map(drop));
}
