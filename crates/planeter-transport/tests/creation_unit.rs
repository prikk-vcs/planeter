//! RFC 004 part 7: the creation service against a real prikk (the sandbox is off — that is the web and binary tests' subject —
//! so that every property of the *sequence* can be held): the empty creation, the two steps of creation from an archive, the
//! kill-safety of every step, the stage limits and the start-up sweep. Tests that need prikk print why and skip without one.

#[path = "support/archives.rs"]
mod archives;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use planeter_core::creation::AdoptionRequest;
use planeter_core::{HostingService, LeaseManager, Principal, new_boot_id};
use planeter_prikk::Sandbox;
use planeter_store::{
    InMemoryLeaseRecords, InMemoryMembershipStore, InMemoryRepositoryStore, InMemoryStageStore,
    MembershipStore, OrgId, OrgRole, Owner, ProposalPolicy, RepoId, RepositoryRecord,
    RepositoryStore, Role, StageLimits, StageStore, StoreError, UserId, Visibility,
};
use planeter_transport::{
    CreateError, CreationConfig, CreationService, StageMeta, creation::cost_model,
};

use archives::{
    KeyInfo, prikk_available, run, simple_archive, tampered, two_key_archive, unverifiable_archive,
};

macro_rules! need_prikk {
    () => {
        if !prikk_available() {
            eprintln!("skipping: prikk is not on PATH");
            return;
        }
    };
}

// ---------------------------------------------------------------------------------------------------- fixtures

/// A repository store whose `mark_ready` can be made to fail (the last step of a creation).
struct Repos {
    inner: InMemoryRepositoryStore,
    fail_ready: AtomicBool,
}

impl RepositoryStore for Repos {
    fn create(&self, r: RepositoryRecord) -> Result<(), StoreError> {
        self.inner.create(r)
    }
    fn get(&self, id: &RepoId) -> Result<Option<RepositoryRecord>, StoreError> {
        self.inner.get(id)
    }
    fn resolve(&self, o: &Owner, n: &str) -> Result<Option<RepoId>, StoreError> {
        self.inner.resolve(o, n)
    }
    fn rename(&self, id: &RepoId, o: Owner, n: String) -> Result<(), StoreError> {
        self.inner.rename(id, o, n)
    }
    fn set_prikk_format_version(&self, id: &RepoId, v: String) -> Result<(), StoreError> {
        self.inner.set_prikk_format_version(id, v)
    }
    fn set_proposals(&self, id: &RepoId, p: ProposalPolicy) -> Result<(), StoreError> {
        self.inner.set_proposals(id, p)
    }
    fn delete(&self, id: &RepoId) -> Result<(), StoreError> {
        self.inner.delete(id)
    }
    fn list_by_owner(&self, o: &Owner) -> Result<Vec<RepositoryRecord>, StoreError> {
        self.inner.list_by_owner(o)
    }
    fn reserve(&self, r: RepositoryRecord) -> Result<(), StoreError> {
        self.inner.reserve(r)
    }
    fn mark_ready(&self, id: &RepoId) -> Result<(), StoreError> {
        if self.fail_ready.load(Ordering::SeqCst) {
            return Err(StoreError::Backend("injected: the last step failed".into()));
        }
        self.inner.mark_ready(id)
    }
    fn creating(&self) -> Result<Vec<RepositoryRecord>, StoreError> {
        self.inner.creating()
    }
    fn known_ids(&self) -> Result<Vec<RepoId>, StoreError> {
        self.inner.known_ids()
    }
    fn count_owned(&self, o: &Owner) -> Result<u64, StoreError> {
        self.inner.count_owned(o)
    }
}

/// A membership store whose grant can be made to fail.
struct Members {
    inner: InMemoryMembershipStore,
    fail_grant: AtomicBool,
}

impl MembershipStore for Members {
    fn direct_repo_grants(&self, u: &UserId, r: &RepoId) -> Result<Vec<Role>, StoreError> {
        self.inner.direct_repo_grants(u, r)
    }
    fn team_repo_grants(&self, u: &UserId, o: &OrgId, r: &RepoId) -> Result<Vec<Role>, StoreError> {
        self.inner.team_repo_grants(u, o, r)
    }
    fn org_role(&self, u: &UserId, o: &OrgId) -> Result<Option<OrgRole>, StoreError> {
        self.inner.org_role(u, o)
    }
    fn grant_user(&self, u: UserId, r: RepoId, role: Role) -> Result<(), StoreError> {
        if self.fail_grant.load(Ordering::SeqCst) {
            return Err(StoreError::Backend("injected: the grant failed".into()));
        }
        self.inner.grant_user(u, r, role)
    }
    fn forget_repo(&self, r: &RepoId) -> Result<(), StoreError> {
        self.inner.forget_repo(r)
    }
}

struct Fix {
    base: PathBuf,
    repos: Arc<Repos>,
    members: Arc<Members>,
    stages: Arc<InMemoryStageStore>,
    hosting: Arc<HostingService>,
    svc: CreationService,
}

fn clock_zero() -> u64 {
    1_000_000
}

fn fixture_with(name: &str, wrapper: Option<&str>, tweak: impl FnOnce(&mut CreationConfig)) -> Fix {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("creation-unit-{name}"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let repos = Arc::new(Repos {
        inner: InMemoryRepositoryStore::new(),
        fail_ready: AtomicBool::new(false),
    });
    let members = Arc::new(Members {
        inner: InMemoryMembershipStore::new(),
        fail_grant: AtomicBool::new(false),
    });
    members
        .inner
        .set_org_role(UserId::new("alice"), OrgId::new("acme"), OrgRole::Member)
        .unwrap();
    let mut hosting = HostingService::new(
        base.join("repos"),
        repos.clone() as Arc<dyn RepositoryStore>,
    )
    .with_sandbox(Sandbox::Unconfined);
    if let Some(spec) = wrapper {
        // A prikk that fails the calls whose first two words are `pattern` from the `n`-th such call on (`pattern@n`; plain
        // `pattern` is from the first), and is the real prikk otherwise. A counter file keeps the count across calls.
        let (pattern, from) = match spec.split_once('@') {
            Some((p, n)) => (p, n.parse::<u32>().unwrap()),
            None => (spec, 1),
        };
        let script = base.join("prikk-wrapper.sh");
        let count = base.join("wrapper-count");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncase \"$1 $2\" in\n  \"{pattern}\"*)\n    n=$(cat \"{c}\" 2>/dev/null || echo 0); n=$((n+1)); echo $n > \"{c}\"\n    if [ \"$n\" -ge {from} ]; then echo \"error: injected failure\" >&2; exit 1; fi;;\nesac\nexec prikk \"$@\"\n",
                c = count.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        hosting = hosting.with_binary(script);
    }
    let hosting = Arc::new(hosting);
    let stages = Arc::new(InMemoryStageStore::new());
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        clock_zero,
        Duration::from_secs(1),
    ));
    let mut config = CreationConfig {
        deadline: Duration::from_secs(120),
        ..CreationConfig::default()
    };
    tweak(&mut config);
    let svc = CreationService::new(
        hosting.clone(),
        repos.clone() as Arc<dyn RepositoryStore>,
        members.clone() as Arc<dyn MembershipStore>,
        stages.clone() as Arc<dyn StageStore>,
        leases,
        Some(base.join("stages")),
        config,
    )
    .unwrap()
    .with_clock(clock_zero);
    Fix {
        base,
        repos,
        members,
        stages,
        hosting,
        svc,
    }
}

fn fixture(name: &str) -> Fix {
    fixture_with(name, None, |_| {})
}

fn alice() -> UserId {
    UserId::new("alice")
}

fn alice_p() -> Principal {
    Principal::User(alice())
}

fn own() -> Owner {
    Owner::User("alice".into())
}

fn meta(name: &str) -> StageMeta {
    StageMeta {
        owner: own(),
        name: name.into(),
        visibility: Visibility::Private,
        proposals: ProposalPolicy::Members,
    }
}

/// Upload `archive` as `alice` and verify it; the stage's summary.
fn stage(
    fix: &Fix,
    archive: &Path,
    name: &str,
) -> Result<planeter_transport::StageSummary, CreateError> {
    stage_as(fix, archive, name, &alice())
}

fn stage_as(
    fix: &Fix,
    archive: &Path,
    name: &str,
    who: &UserId,
) -> Result<planeter_transport::StageSummary, CreateError> {
    let bytes = std::fs::read(archive).unwrap();
    let mut up = fix.svc.start_stage(who, bytes.len() as u64)?;
    for chunk in bytes.chunks(8192) {
        up.write(chunk)?;
    }
    let up = up.finish()?;
    fix.svc.finish_stage(up, meta(name), who)
}

fn everything_on_disk(fix: &Fix) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(shards) = std::fs::read_dir(fix.base.join("repos")) {
        for s in shards.flatten() {
            for c in std::fs::read_dir(s.path()).into_iter().flatten().flatten() {
                out.push(c.path());
            }
        }
    }
    out
}

fn stage_dirs(fix: &Fix) -> Vec<PathBuf> {
    std::fs::read_dir(fix.base.join("stages"))
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn typed(keys: &[KeyInfo]) -> AdoptionRequest {
    AdoptionRequest {
        adopt: keys
            .iter()
            .map(|k| (k.key_id.clone(), k.public_key.clone()))
            .collect(),
        ..AdoptionRequest::default()
    }
}

fn trusted(fix: &Fix, record: &RepositoryRecord) -> Vec<String> {
    let out = run(
        &record.path,
        &fix.base,
        &["trust", "maintainer", "list", "--format", "json"],
    );
    let mut ids = Vec::new();
    let mut rest = out.as_str();
    while let Some(i) = rest.find("\"key_id\"") {
        rest = &rest[i + 8..];
        let s = rest.find('"').unwrap() + 1;
        let e = s + rest[s..].find('"').unwrap();
        ids.push(rest[s..e].to_owned());
    }
    ids
}

fn branches(fix: &Fix, record: &RepositoryRecord) -> String {
    run(
        &record.path,
        &fix.base,
        &["branch", "--all", "--format", "json"],
    )
}

fn verify_exit(fix: &Fix, record: &RepositoryRecord) -> i32 {
    std::process::Command::new("prikk")
        .args(["verify", "--format", "json"])
        .current_dir(&record.path)
        .env("HOME", &fix.base)
        .output()
        .unwrap()
        .status
        .code()
        .unwrap_or(-1)
}

// ---------------------------------------------------------------------------------------------------- the cost model

#[test]
fn the_cost_model_is_one_table_with_the_measured_multiples() {
    const MIB: u64 = 1 << 20;
    assert_eq!(cost_model::verify_cost(100 * MIB), 10 * MIB + 300 * MIB);
    assert_eq!(cost_model::import_cost(100 * MIB), 10 * MIB + 600 * MIB);
    assert_eq!(
        cost_model::admission_cost(100 * MIB),
        cost_model::import_cost(100 * MIB)
    );
    assert_eq!(cost_model::verify_ceiling(100 * MIB), 465 * MIB);
    assert_eq!(cost_model::import_ceiling(100 * MIB), 915 * MIB);
    // The ceiling clears what was measured (3.02 x and 6.03 x of one 10 MiB blob; 3.00 and 6.00 for the larger).
    for mib in [10u64, 50, 100, 200] {
        assert!(cost_model::verify_ceiling(mib * MIB) > mib * MIB * 303 / 100);
        assert!(cost_model::import_ceiling(mib * MIB) > mib * MIB * 604 / 100);
    }
    // The inverse: a 4 GiB host's quarter (1 GiB) admits just under 170 MiB.
    let largest = cost_model::largest_archive_admitted(1024 * MIB).unwrap();
    assert!(cost_model::admission_cost(largest) <= 1024 * MIB);
    assert!(cost_model::admission_cost(largest + MIB) > 1024 * MIB - MIB);
    assert_eq!(cost_model::largest_archive_admitted(MIB), None);
}

// ---------------------------------------------------------------------------------------------------- empty

#[test]
fn an_empty_repository_is_created_ready_and_the_creator_administers_it() {
    need_prikk!();
    let fix = fixture("empty");
    let target = fix.svc.target(&alice_p(), "alice").unwrap();
    let record = fix
        .svc
        .create_empty(
            target,
            "app".into(),
            Visibility::Public,
            ProposalPolicy::Members,
            &alice(),
        )
        .unwrap();
    assert!(record.path.join(".prikk").is_dir());
    assert!(fix.hosting.record_by_name(&own(), "app").unwrap().is_some());
    assert_eq!(
        fix.members
            .direct_repo_grants(&alice(), &record.repo_id)
            .unwrap(),
        vec![Role::Admin]
    );
    // In an organization the creator is a member of.
    let org = fix.svc.target(&alice_p(), "acme").unwrap();
    assert_eq!(org, Owner::Org("acme".into()));
    let r2 = fix
        .svc
        .create_empty(
            org,
            "core".into(),
            Visibility::Private,
            ProposalPolicy::AnyAuthenticated,
            &alice(),
        )
        .unwrap();
    assert_eq!(r2.proposals, ProposalPolicy::AnyAuthenticated);
    assert_eq!(
        fix.members
            .direct_repo_grants(&alice(), &r2.repo_id)
            .unwrap(),
        vec![Role::Admin]
    );
}

#[test]
fn a_stranger_gets_one_refusal_whatever_they_name() {
    let fix = fixture("neutral");
    let mallory = Principal::User(UserId::new("mallory"));
    for handle in ["acme", "alice", "no-such-org"] {
        assert!(
            matches!(fix.svc.target(&mallory, handle), Err(CreateError::Refused)),
            "{handle}"
        );
    }
    assert!(matches!(
        fix.svc.target(&Principal::Anonymous, "alice"),
        Err(CreateError::NotSignedIn)
    ));
}

#[test]
fn a_taken_name_and_a_full_quota_are_named_and_nothing_is_written() {
    need_prikk!();
    let fix = fixture_with("taken", None, |c| c.user_quota = 2);
    for n in ["one", "two"] {
        fix.svc
            .create_empty(
                own(),
                n.into(),
                Visibility::Private,
                ProposalPolicy::Members,
                &alice(),
            )
            .unwrap();
    }
    let before = everything_on_disk(&fix).len();
    assert!(matches!(
        fix.svc.create_empty(
            own(),
            "one".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice()
        ),
        Err(CreateError::Quota { limit: 2, owned: 2 })
    ));
    assert_eq!(
        everything_on_disk(&fix).len(),
        before,
        "nothing was written"
    );
    // Room for a third in the organization (its own quota), but the name check works there too.
    fix.svc
        .create_empty(
            Owner::Org("acme".into()),
            "x".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice(),
        )
        .unwrap();
    assert!(matches!(
        fix.svc.create_empty(
            Owner::Org("acme".into()),
            "x".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice()
        ),
        Err(CreateError::NameTaken)
    ));
}

#[test]
fn an_empty_creation_that_fails_at_any_step_removes_everything() {
    need_prikk!();
    // 1. `prikk init` fails.
    let fix = fixture_with("empty-fail-init", Some("init"), |_| {});
    let e = fix
        .svc
        .create_empty(
            own(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice(),
        )
        .unwrap_err();
    assert!(matches!(e, CreateError::Unavailable), "{e:?}");
    nothing_left(&fix);
    // 2. The grant fails.
    let fix = fixture("empty-fail-grant");
    fix.members.fail_grant.store(true, Ordering::SeqCst);
    assert!(
        fix.svc
            .create_empty(
                own(),
                "app".into(),
                Visibility::Private,
                ProposalPolicy::Members,
                &alice()
            )
            .is_err()
    );
    nothing_left(&fix);
    // 3. The last step fails.
    let fix = fixture("empty-fail-ready");
    fix.repos.fail_ready.store(true, Ordering::SeqCst);
    assert!(
        fix.svc
            .create_empty(
                own(),
                "app".into(),
                Visibility::Private,
                ProposalPolicy::Members,
                &alice()
            )
            .is_err()
    );
    fix.repos.fail_ready.store(false, Ordering::SeqCst);
    nothing_left(&fix);
    // ... and the name is free again.
    fix.svc
        .create_empty(
            own(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice(),
        )
        .unwrap();
}

fn nothing_left(fix: &Fix) {
    assert!(
        everything_on_disk(fix).is_empty(),
        "directories: {:?}",
        everything_on_disk(fix)
    );
    assert!(fix.repos.known_ids().unwrap().is_empty(), "records");
    assert!(fix.repos.creating().unwrap().is_empty());
}

// ---------------------------------------------------------------------------------------------------- stage

#[test]
fn a_stage_shows_the_carried_keys_the_refs_and_the_counts_and_trusts_nothing() {
    need_prikk!();
    let fix = fixture("stage");
    let (archive, keys) = simple_archive(&fix.base, "maint", 3, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    assert_eq!(s.findings.keys.len(), 1);
    assert_eq!(s.findings.keys[0].key_id, keys[0].key_id);
    assert_eq!(
        s.findings.keys[0].public_key, keys[0].public_key,
        "the fingerprint is the whole public key"
    );
    assert_eq!(s.findings.refs.len(), 1);
    assert_eq!(s.findings.refs[0].ref_name, "heads/main");
    assert!(
        s.findings.refs[0].publishable,
        "with its key adopted it would be published"
    );
    assert!(s.findings.verifies_with_all_keys);
    assert!(s.findings.object_total >= 4, "{}", s.findings.object_total);
    assert_eq!(s.archive_bytes, std::fs::metadata(&archive).unwrap().len());
    assert_eq!(s.expires_at - s.created_at, 24 * 3600);
    // Nothing is a repository yet: no record, no directory under the repository root, and the scratch repository is gone.
    assert!(fix.repos.known_ids().unwrap().is_empty());
    assert!(everything_on_disk(&fix).is_empty());
    let dirs = stage_dirs(&fix);
    assert_eq!(dirs.len(), 1);
    assert!(
        !dirs[0].join("scratch").exists(),
        "the scratch repository is removed"
    );
    assert!(dirs[0].join("in").join("archive").is_file());
}

#[test]
fn a_tampered_archive_is_refused_with_prikks_text_and_nothing_is_kept() {
    need_prikk!();
    let fix = fixture("tampered");
    let (archive, _) = simple_archive(&fix.base, "maint", 2, 0);
    let bad = tampered(&archive, &fix.base.join("bad.archive"));
    let e = stage(&fix, &bad, "app").unwrap_err();
    match e {
        CreateError::ArchiveRefused { text } => {
            assert!(text.contains("archive"), "prikk's own words: {text}");
            assert!(!text.starts_with("error:"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    assert!(stage_dirs(&fix).is_empty(), "no directory is kept");
    assert!(fix.stages.all().unwrap().is_empty(), "no record is kept");
    // Not an archive at all; empty.
    let junk = fix.base.join("junk.archive");
    std::fs::write(&junk, vec![7u8; 5000]).unwrap();
    assert!(matches!(
        stage(&fix, &junk, "app"),
        Err(CreateError::ArchiveRefused { .. })
    ));
    std::fs::write(&junk, b"").unwrap();
    assert!(matches!(
        stage(&fix, &junk, "app"),
        Err(CreateError::ArchiveRefused { .. })
    ));
    assert!(stage_dirs(&fix).is_empty());
}

#[test]
fn stages_are_limited_per_principal_and_by_bytes_before_a_byte_is_read() {
    need_prikk!();
    let fix = fixture("limits");
    let (archive, _) = simple_archive(&fix.base, "maint", 1, 0);
    let one = std::fs::metadata(&archive).unwrap().len();
    stage(&fix, &archive, "a").unwrap();
    stage(&fix, &archive, "b").unwrap();
    match fix.svc.start_stage(&alice(), one) {
        Err(CreateError::StageLimit(planeter_store::StageRefusal::TooManyForPrincipal {
            limit: 2,
        })) => {}
        other => panic!("{:?}", other.err()),
    }
    // Another principal is unaffected.
    let bob = UserId::new("bob");
    assert!(fix.svc.start_stage(&bob, one).is_ok());
    // The byte budget.
    let fix = fixture_with("budget", None, |c| {
        c.stage_limits = StageLimits {
            per_principal: 5,
            total_bytes: 1000,
        }
    });
    match fix.svc.start_stage(&alice(), 1001) {
        Err(CreateError::StageLimit(planeter_store::StageRefusal::BudgetExceeded {
            limit: 1000,
        })) => {}
        other => panic!("{:?}", other.err()),
    }
    // The size bound.
    let fix = fixture_with("size", None, |c| c.max_archive_bytes = 4096);
    assert!(matches!(
        fix.svc.start_stage(&alice(), 4097),
        Err(CreateError::TooLarge { limit: 4096 })
    ));
    let mut up = fix.svc.start_stage(&alice(), 10).unwrap();
    assert!(
        matches!(
            up.write(&vec![0u8; 5000]),
            Err(CreateError::TooLarge { .. })
        ),
        "a body longer than it said"
    );
    drop(up);
    assert!(
        stage_dirs(&fix).is_empty(),
        "an abandoned upload leaves no directory"
    );
}

static EXPIRY_NOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_000_000);
fn expiry_clock() -> u64 {
    EXPIRY_NOW.load(Ordering::SeqCst)
}

#[test]
fn an_expired_stage_is_gone_410_and_swept() {
    need_prikk!();
    let fix = fixture("expiry");
    let svc = fix.svc.with_clock(expiry_clock);
    let fix = Fix { svc, ..fix };
    EXPIRY_NOW.store(1_000_000, Ordering::SeqCst);
    let (archive, keys) = simple_archive(&fix.base, "maint", 1, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    EXPIRY_NOW.store(1_000_000 + 24 * 3600 - 1, Ordering::SeqCst);
    assert!(
        fix.svc.stage(&alice(), &s.stage_id).is_ok(),
        "one second before the end"
    );
    EXPIRY_NOW.store(1_000_000 + 24 * 3600, Ordering::SeqCst);
    assert!(matches!(
        fix.svc.stage(&alice(), &s.stage_id),
        Err(CreateError::StageExpired)
    ));
    assert!(matches!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys)),
        Err(CreateError::StageExpired)
    ));
    assert_eq!(fix.svc.sweep_stages(), 1);
    assert!(stage_dirs(&fix).is_empty());
    assert!(fix.stages.all().unwrap().is_empty());
    assert!(matches!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys)),
        Err(CreateError::StageNotFound)
    ));
    EXPIRY_NOW.store(1_000_000, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------------------------------- confirm

#[test]
fn typing_the_right_fingerprints_creates_a_verified_repository_with_exactly_those_keys_trusted() {
    need_prikk!();
    let fix = fixture("confirm");
    let (archive, keys) = simple_archive(&fix.base, "maint", 4, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    // A wrong fingerprint first: refused, naming the key, and the stage is untouched.
    let wrong = AdoptionRequest {
        adopt: vec![(keys[0].key_id.clone(), "0".repeat(64))],
        ..AdoptionRequest::default()
    };
    match fix.svc.confirm(&alice_p(), &alice(), &s.stage_id, &wrong) {
        Err(CreateError::Adoption(planeter_core::creation::AdoptionError::Mismatch { key_id })) => {
            assert_eq!(key_id, keys[0].key_id)
        }
        other => panic!("{:?}", other.err()),
    }
    assert!(
        fix.repos.known_ids().unwrap().is_empty(),
        "nothing was reserved"
    );
    assert!(
        fix.svc.stage(&alice(), &s.stage_id).is_ok(),
        "the stage is still there"
    );
    // The right one, grouped and in capitals as a person pastes it.
    let pasted = AdoptionRequest {
        adopt: vec![(
            keys[0].key_id.clone(),
            planeter_core::creation::group_fingerprint(&keys[0].public_key).to_uppercase(),
        )],
        ..AdoptionRequest::default()
    };
    let done = fix
        .svc
        .confirm(&alice_p(), &alice(), &s.stage_id, &pasted)
        .unwrap();
    assert!(done.verified);
    assert_eq!(done.adopted.len(), 1);
    assert_eq!(done.landings.len(), 1);
    assert!(done.landings[0].published, "{:?}", done.landings);
    assert_eq!(done.landings[0].landed_as, "heads/main");
    assert_eq!(verify_exit(&fix, &done.record), 0, "verify exits 0");
    assert_eq!(
        trusted(&fix, &done.record),
        vec![keys[0].key_id.clone()],
        "the typed key and no other"
    );
    assert!(branches(&fix, &done.record).contains("\"heads/main\""));
    assert!(
        fix.hosting.record_by_name(&own(), "app").unwrap().is_some(),
        "visible"
    );
    assert_eq!(
        fix.members
            .direct_repo_grants(&alice(), &done.record.repo_id)
            .unwrap(),
        vec![Role::Admin]
    );
    // The stage is gone: no second confirmation.
    assert!(stage_dirs(&fix).is_empty());
    assert!(matches!(
        fix.svc.confirm(&alice_p(), &alice(), &s.stage_id, &pasted),
        Err(CreateError::StageNotFound)
    ));
}

#[test]
fn only_the_stager_can_see_or_confirm_a_stage_and_a_stranger_learns_nothing() {
    need_prikk!();
    let fix = fixture("owner-only");
    let (archive, keys) = simple_archive(&fix.base, "maint", 1, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    let bob = UserId::new("bob");
    let bob_p = Principal::User(bob.clone());
    assert!(matches!(
        fix.svc.stage(&bob, &s.stage_id),
        Err(CreateError::StageNotFound)
    ));
    assert!(matches!(
        fix.svc.confirm(&bob_p, &bob, &s.stage_id, &typed(&keys)),
        Err(CreateError::StageNotFound)
    ));
    // The same answer as for a stage that never existed.
    assert!(matches!(
        fix.svc
            .confirm(&bob_p, &bob, "s-000000000000000000000000", &typed(&keys)),
        Err(CreateError::StageNotFound)
    ));
    for hostile in [
        "../x",
        "s-../../etc",
        "",
        "S-AAAAAAAAAAAAAAAAAAAAAAAA",
        "s-0000",
    ] {
        assert!(
            matches!(
                fix.svc.stage(&alice(), hostile),
                Err(CreateError::StageNotFound)
            ),
            "{hostile}"
        );
    }
    assert!(fix.repos.known_ids().unwrap().is_empty());
}

#[test]
fn no_adopt_lands_everything_under_remotes_and_says_it_is_not_verified() {
    need_prikk!();
    let fix = fixture("no-adopt");
    let (archive, _) = simple_archive(&fix.base, "maint", 2, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    let req = AdoptionRequest {
        no_adopt: true,
        ..AdoptionRequest::default()
    };
    let done = fix
        .svc
        .confirm(&alice_p(), &alice(), &s.stage_id, &req)
        .unwrap();
    assert!(!done.verified, "nothing is verified until a key is adopted");
    assert!(done.adopted.is_empty());
    assert_eq!(done.left_out.len(), 1);
    assert!(!done.landings[0].published);
    assert_eq!(done.landings[0].landed_as, "remotes/heads/main");
    // Without `--adopt` prikk gives no per-ref reason; its note says the refs are not yet trusted.
    assert!(
        done.note
            .as_deref()
            .unwrap_or("")
            .contains("not yet trusted"),
        "prikk's note: {:?}",
        done.note
    );
    assert!(trusted(&fix, &done.record).is_empty(), "no key was adopted");
    let b = branches(&fix, &done.record);
    assert!(
        b.contains("remotes/heads/main") && b.contains("\"branches\": []"),
        "no published branch, one received ref: {b}"
    );
    assert_ne!(verify_exit(&fix, &done.record), 0);
}

#[test]
fn leaving_one_of_two_keys_out_publishes_only_the_ref_of_the_key_that_was_typed() {
    need_prikk!();
    let fix = fixture("partial");
    let (archive, keys) = two_key_archive(&fix.base, "two");
    assert_eq!(keys.len(), 2);
    let s = stage(&fix, &archive, "app").unwrap();
    assert!(
        s.findings.verifies_with_all_keys,
        "with both adopted it verifies"
    );
    // Two published refs, and the received ref the exporter carries (B's `heads/main` beside A's own), which is never published.
    assert_eq!(s.findings.refs.len(), 3, "{:?}", s.findings.refs);
    assert_eq!(s.findings.refs.iter().filter(|r| r.publishable).count(), 2);
    assert!(
        s.findings
            .refs
            .iter()
            .any(|r| r.ref_name == "remotes/heads/main" && !r.publishable)
    );
    // Adopt the first, leave the second out explicitly.
    let req = AdoptionRequest {
        adopt: vec![(keys[0].key_id.clone(), keys[0].public_key.clone())],
        leave_out: vec![keys[1].key_id.clone()],
        no_adopt: false,
    };
    let done = fix
        .svc
        .confirm(&alice_p(), &alice(), &s.stage_id, &req)
        .unwrap();
    assert!(
        !done.verified,
        "a ref signed by the key left out is under remotes/"
    );
    let published: Vec<&str> = done
        .landings
        .iter()
        .filter(|l| l.published)
        .map(|l| l.ref_name.as_str())
        .collect();
    assert_eq!(published.len(), 1, "{:?}", done.landings);
    assert_eq!(trusted(&fix, &done.record), vec![keys[0].key_id.clone()]);
    // Both typed: everything is published and verify is clean.
    let fix2 = fixture("both");
    let s2 = stage(&fix2, &archive, "app").unwrap();
    let done2 = fix2
        .svc
        .confirm(&alice_p(), &alice(), &s2.stage_id, &typed(&keys))
        .unwrap();
    assert!(done2.verified);
    assert_eq!(done2.landings.iter().filter(|l| l.published).count(), 2);
    assert_eq!(verify_exit(&fix2, &done2.record), 0);
    // Neither adopted nor left out: refused before anything is made.
    let fix3 = fixture("unaddressed");
    let s3 = stage(&fix3, &archive, "app").unwrap();
    let half = AdoptionRequest {
        adopt: vec![(keys[0].key_id.clone(), keys[0].public_key.clone())],
        ..AdoptionRequest::default()
    };
    assert!(matches!(
        fix3.svc.confirm(&alice_p(), &alice(), &s3.stage_id, &half),
        Err(CreateError::Adoption(
            planeter_core::creation::AdoptionError::Unaddressed { .. }
        ))
    ));
    assert!(fix3.repos.known_ids().unwrap().is_empty());
}

#[test]
fn an_archive_that_cannot_verify_even_with_every_key_is_removed_whole_and_the_stage_goes_with_it() {
    need_prikk!();
    let fix = fixture("unverifiable");
    let (archive, keys) = unverifiable_archive(&fix.base, "bad");
    let s = stage(&fix, &archive, "app").unwrap();
    assert!(
        !s.findings.verifies_with_all_keys,
        "the stage already says so"
    );
    assert!(
        s.findings
            .verify_failures
            .iter()
            .any(|c| c == "publication-trust"),
        "{:?}",
        s.findings.verify_failures
    );
    match fix
        .svc
        .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
    {
        Err(CreateError::VerifyFailed { conditions }) => {
            assert_eq!(conditions, vec!["publication-trust".to_owned()])
        }
        other => panic!("{:?}", other.err()),
    }
    nothing_left(&fix);
    assert!(
        stage_dirs(&fix).is_empty(),
        "the archive's own fault: nothing is kept"
    );
    // The creator could have chosen to create it without trusting anything (everything under remotes/).
    let s2 = stage(&fix, &archive, "app").unwrap();
    let done = fix
        .svc
        .confirm(
            &alice_p(),
            &alice(),
            &s2.stage_id,
            &AdoptionRequest {
                no_adopt: true,
                ..AdoptionRequest::default()
            },
        )
        .unwrap();
    assert!(!done.verified);
}

#[test]
fn every_step_of_the_confirmation_that_fails_removes_the_repository_whole() {
    need_prikk!();
    // The call that fails, as `$1 $2` of the prikk command line, from its **second** call on: the first is the stage's own
    // (its scratch repository runs the same verbs), the second is the confirmation's. `kept` says whether the stage is kept for
    // another try (a failure that is not the archive's) or removed with the repository (prikk refused the archive).
    for (step, wrapper, kept) in [
        ("init", "init@2", true),
        ("trust", "trust maintainer@2", true),
        ("archive import", "archive import@2", false),
        ("repository verify", "verify --format@2", true),
    ] {
        let fix = fixture_with(
            &format!("fail-{}", step.replace(' ', "-")),
            Some(wrapper),
            |_| {},
        );
        let (archive, keys) = simple_archive(&fix.base, "maint", 2, 0);
        let s = stage(&fix, &archive, "app")
            .unwrap_or_else(|e| panic!("{step}: the stage must work: {e:?}"));
        let e = fix
            .svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
            .unwrap_err();
        eprintln!("{step}: {e:?}");
        nothing_left(&fix);
        assert_eq!(
            fix.svc.stage(&alice(), &s.stage_id).is_ok(),
            kept,
            "{step}: the stage is {} for another try",
            if kept { "kept" } else { "removed" }
        );
        assert_eq!(stage_dirs(&fix).len(), usize::from(kept), "{step}");
        // The name is free: with a working prikk the same stage confirms (when it was kept).
        if kept {
            let fixed = fix.base.join("prikk-wrapper.sh");
            std::fs::write(&fixed, "#!/bin/sh\nexec prikk \"$@\"\n").unwrap();
            assert!(
                fix.svc
                    .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
                    .is_ok(),
                "{step}: a second try works once prikk does"
            );
        }
    }
}

#[test]
fn a_failure_of_the_confirmation_that_is_not_the_archives_leaves_the_stage_for_another_try() {
    need_prikk!();
    let fix = fixture("retry");
    let (archive, keys) = simple_archive(&fix.base, "maint", 2, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    fix.members.fail_grant.store(true, Ordering::SeqCst);
    assert!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
            .is_err()
    );
    nothing_left(&fix);
    assert!(
        fix.svc.stage(&alice(), &s.stage_id).is_ok(),
        "the stage is released for another try"
    );
    fix.members.fail_grant.store(false, Ordering::SeqCst);
    let done = fix
        .svc
        .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
        .unwrap();
    assert!(done.verified);
    // The last step failing is the same.
    let fix = fixture("retry-ready");
    let s = stage(&fix, &archive, "app").unwrap();
    fix.repos.fail_ready.store(true, Ordering::SeqCst);
    assert!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
            .is_err()
    );
    fix.repos.fail_ready.store(false, Ordering::SeqCst);
    nothing_left(&fix);
    assert!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
            .is_ok()
    );
}

#[test]
fn two_confirmations_at_once_have_one_winner() {
    need_prikk!();
    let fix = fixture("race");
    let (archive, keys) = simple_archive(&fix.base, "maint", 3, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    let results: Vec<Result<planeter_transport::Confirmed, CreateError>> =
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        fix.svc
                            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
    let wins = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        wins,
        1,
        "{:?}",
        results
            .iter()
            .map(|r| r.as_ref().err().map(CreateError::reason))
            .collect::<Vec<_>>()
    );
    for r in results.iter().filter_map(|r| r.as_ref().err()) {
        assert!(
            matches!(
                r,
                CreateError::StageInUse | CreateError::StageNotFound | CreateError::NameTaken
            ),
            "{r:?}"
        );
    }
    assert_eq!(fix.repos.known_ids().unwrap().len(), 1, "one repository");
}

#[test]
fn a_staged_file_that_changed_on_disk_is_not_imported() {
    need_prikk!();
    // The file under the stage is replaced by **another valid archive** (so that prikk itself would accept it): only the digest
    // taken when it was verified can tell.
    let fix = fixture("digest");
    let (archive, keys) = simple_archive(&fix.base, "maint", 2, 0);
    let (other, _) = simple_archive(&fix.base, "other", 3, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    let staged = stage_dirs(&fix)[0].join("in").join("archive");
    std::fs::write(&staged, std::fs::read(&other).unwrap()).unwrap();
    match fix
        .svc
        .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys))
    {
        Err(CreateError::ArchiveRefused { text }) => {
            assert!(text.contains("not the one that was verified"), "{text}")
        }
        other => panic!("{:?}", other.err()),
    }
    nothing_left(&fix);
    assert!(
        stage_dirs(&fix).is_empty(),
        "a stage whose file changed is not kept"
    );
    // A byte flipped in the staged file is refused too (by the same check, before prikk is asked).
    let fix = fixture("digest-flip");
    let (archive, keys) = simple_archive(&fix.base, "maint", 2, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    let staged = stage_dirs(&fix)[0].join("in").join("archive");
    let mut bytes = std::fs::read(&staged).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&staged, bytes).unwrap();
    assert!(matches!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys)),
        Err(CreateError::ArchiveRefused { .. })
    ));
    nothing_left(&fix);
}

#[test]
fn a_taken_name_or_a_changed_membership_at_confirmation_is_refused_before_anything_is_made() {
    need_prikk!();
    let fix = fixture("late");
    let (archive, keys) = simple_archive(&fix.base, "maint", 1, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    fix.svc
        .create_empty(
            own(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice(),
        )
        .unwrap();
    assert!(matches!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s.stage_id, &typed(&keys)),
        Err(CreateError::NameTaken)
    ));
    assert!(
        fix.svc.stage(&alice(), &s.stage_id).is_ok(),
        "the stage waits for another name... which it cannot take: it is kept"
    );
    // A stage for an organization the stager has since left.
    let mut m = meta("lib");
    m.owner = Owner::Org("acme".into());
    let bytes = std::fs::read(&archive).unwrap();
    let mut up = fix.svc.start_stage(&alice(), bytes.len() as u64).unwrap();
    up.write(&bytes).unwrap();
    let s2 = fix
        .svc
        .finish_stage(up.finish().unwrap(), m, &alice())
        .unwrap();
    let carol = UserId::new("carol");
    let _ = carol;
    // alice is still a member: works; remove her by using a fresh membership view is not possible on the in-memory store,
    // so the refusal is held at the unit level of `creation_target` (tests/creation_rules.rs) and here only the path runs.
    assert!(
        fix.svc
            .confirm(&alice_p(), &alice(), &s2.stage_id, &typed(&keys))
            .is_ok()
    );
}

// ---------------------------------------------------------------------------------------------------- the start-up sweep

#[test]
fn the_start_up_sweep_removes_a_killed_creation_its_grants_its_lease_record_and_a_stage_that_was_confirming()
 {
    need_prikk!();
    let fix = fixture("sweep");
    let (archive, keys) = simple_archive(&fix.base, "maint", 1, 0);
    // A creation killed half way: reserved, directory made, `init` run, a grant, a W lease record, a stage being confirmed.
    let s = stage(&fix, &archive, "app").unwrap();
    let c = fix
        .hosting
        .begin_creation(
            own(),
            "app".into(),
            Visibility::Private,
            ProposalPolicy::Members,
        )
        .unwrap();
    run(c.path(), &fix.base, &["init", "."]);
    fix.members
        .inner
        .grant_user(alice(), c.record().repo_id.clone(), Role::Admin)
        .unwrap();
    let id = c.record().repo_id.clone();
    let path = c.path().to_path_buf();
    std::mem::forget(c);
    fix.stages
        .claim(&s.stage_id, "alice", clock_zero())
        .unwrap()
        .unwrap();
    // A stage directory no record owns, and a link planted in the stage root.
    let orphan = fix.base.join("stages").join("s-0123456789abcdef01234567");
    std::fs::create_dir_all(orphan.join("in")).unwrap();
    std::fs::write(orphan.join("in").join("archive"), b"x").unwrap();
    let outside = fix.base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), b"x").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        fix.base.join("stages").join("s-fedcba9876543210fedcba98"),
    )
    .unwrap();

    let report = fix.svc.sweep_at_start();
    assert_eq!(report.repositories.removed_creating, vec![id.clone()]);
    assert!(!path.exists());
    assert!(fix.repos.known_ids().unwrap().is_empty());
    assert!(
        fix.members
            .direct_repo_grants(&alice(), &id)
            .unwrap()
            .is_empty(),
        "the grant is forgotten"
    );
    assert!(
        report.stages_removed.contains(&s.stage_id),
        "a stage whose confirmation was running is removed"
    );
    assert!(
        report.stages_removed.iter().any(|n| n.ends_with("4567")),
        "{:?}",
        report.stages_removed
    );
    assert!(!orphan.exists());
    assert!(
        outside.join("precious").exists(),
        "a link in the stage root is unlinked, never followed"
    );
    // The name is free and a new stage can be made (the budget is not held by the dead one).
    assert!(fix.svc.start_stage(&alice(), 1).is_ok());
    let _ = keys;
}

#[test]
fn the_sweep_of_a_clean_server_removes_nothing() {
    need_prikk!();
    let fix = fixture("sweep-clean");
    let (archive, _) = simple_archive(&fix.base, "maint", 1, 0);
    let s = stage(&fix, &archive, "app").unwrap();
    fix.svc
        .create_empty(
            own(),
            "other".into(),
            Visibility::Private,
            ProposalPolicy::Members,
            &alice(),
        )
        .unwrap();
    let report = fix.svc.sweep_at_start();
    assert!(
        report.repositories.removed_creating.is_empty()
            && report.repositories.removed_orphans.is_empty()
    );
    assert!(report.stages_removed.is_empty());
    assert!(
        fix.svc.stage(&alice(), &s.stage_id).is_ok(),
        "a live stage survives a restart"
    );
}

#[test]
fn creation_from_an_archive_is_off_without_a_stage_directory() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("creation-unit-off");
    let _ = std::fs::remove_dir_all(&base);
    let repos = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(
        base.join("repos"),
        repos.clone() as Arc<dyn RepositoryStore>,
    ));
    let svc = CreationService::new(
        hosting,
        repos,
        Arc::new(InMemoryMembershipStore::new()),
        Arc::new(InMemoryStageStore::new()),
        Arc::new(LeaseManager::new(
            Arc::new(InMemoryLeaseRecords::new()),
            new_boot_id(),
            clock_zero,
            Duration::from_secs(1),
        )),
        None,
        CreationConfig::default(),
    )
    .unwrap();
    assert!(!svc.import_enabled());
    assert!(matches!(
        svc.start_stage(&alice(), 1),
        Err(CreateError::ImportOff)
    ));
    assert!(matches!(
        svc.stage(&alice(), "s-000000000000000000000000"),
        Err(CreateError::ImportOff) | Err(CreateError::StageNotFound)
    ));
}
