//! RFC 004 part 3b, the parts of the propose path that need no prikk: the metadata rules, the admission arithmetic,
//! the proposal's working area and upload in the artifact store, and the upload slots.

use std::sync::Arc;
use std::time::Duration;

use planeter_core::{HostingService, LeaseManager, new_boot_id};
use planeter_store::{
    ChangeStore, InMemoryChangeStore, InMemoryLeaseRecords, InMemoryRepositoryStore,
    InMemoryRevisions, RepositoryStore, RevisionStore,
};
use planeter_transport::{
    ArtifactStore, ExportConfig, ExportService, MetaError, Metrics, ProposeConfig, ProposeError,
    ProposeMeta, ProposeService, Spool, StoreError,
};

fn dir(name: &str) -> std::path::PathBuf {
    let d =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("propose-unit-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn title(t: &str) -> ProposeMeta {
    ProposeMeta {
        title: Some(t.to_owned()),
        ..ProposeMeta::default()
    }
}

#[test]
fn the_metadata_rules_each_have_a_named_reason() {
    assert_eq!(title("A change").validate(), Ok(()));
    assert_eq!(
        ProposeMeta::default().validate(),
        Err(MetaError::TitleRequired)
    );
    assert_eq!(title("   ").validate(), Err(MetaError::TitleRequired));
    assert_eq!(
        title("a\nb").validate(),
        Err(MetaError::TitleRequired),
        "a control character is not a title"
    );
    assert_eq!(title(&"x".repeat(200)).validate(), Ok(()));
    assert_eq!(
        title(&"x".repeat(201)).validate(),
        Err(MetaError::TitleTooLong)
    );
    assert_eq!(
        title(&"é".repeat(200)).validate(),
        Ok(()),
        "200 characters, not 200 bytes"
    );
    let mut m = title("t");
    m.description = Some("d".repeat(64 * 1024));
    assert_eq!(m.validate(), Ok(()));
    m.description = Some("d".repeat(64 * 1024 + 1));
    assert_eq!(m.validate(), Err(MetaError::DescriptionTooLong));
    let mut m = title("t");
    for bad in ["", "a b", "a..b", "a\tb"] {
        m.target_ref = Some(bad.to_owned());
        assert_eq!(m.validate(), Err(MetaError::TargetRefInvalid), "{bad:?}");
    }
    m.target_ref = Some("heads/main".to_owned());
    assert_eq!(m.validate(), Ok(()));
    let hex = "a".repeat(64);
    m.base = Some(hex.clone());
    assert_eq!(m.validate(), Ok(()));
    for bad in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        m.base = Some(bad);
        assert_eq!(m.validate(), Err(MetaError::BaseInvalid));
    }
    m.base = None;
    m.declared_patches = Some(vec![hex.clone(), "b".repeat(64)]);
    assert_eq!(m.validate(), Ok(()));
    m.declared_patches = Some(vec![hex.clone(), "nope".to_owned()]);
    assert_eq!(m.validate(), Err(MetaError::PatchIdInvalid));
    m.declared_patches = Some(vec![hex; 20_001]);
    assert_eq!(m.validate(), Err(MetaError::TooManyPatchIds));
    m.declared_patches = None;
    m.client = Some("c".repeat(201));
    assert_eq!(m.validate(), Err(MetaError::ClientInvalid));
    // An existing change takes no title, description or target ref, and its id is plain.
    let existing = ProposeMeta {
        change: Some("c-0123456789abcdef".to_owned()),
        ..ProposeMeta::default()
    };
    assert_eq!(existing.validate(), Ok(()));
    for bad in ["", "has space", &"x".repeat(65), "semi;colon"] {
        let m = ProposeMeta {
            change: Some(bad.to_owned()),
            ..ProposeMeta::default()
        };
        assert_eq!(m.validate(), Err(MetaError::ChangeIdInvalid), "{bad:?}");
    }
    let m = ProposeMeta {
        title: Some("t".to_owned()),
        ..existing
    };
    assert_eq!(m.validate(), Err(MetaError::FieldsForExistingChange));
}

#[test]
fn the_admission_cost_of_an_accept_is_ten_mib_plus_four_point_two_times_the_artifact() {
    const MIB: u64 = 1 << 20;
    assert_eq!(ProposeConfig::accept_cost(0), 10 * MIB);
    assert_eq!(ProposeConfig::accept_cost(100 * MIB), 10 * MIB + 420 * MIB);
    // At the default bound (256 MiB): about 1.1 GiB of the budget, and a ceiling of 1.5 times that.
    let cost = ProposeConfig::accept_cost(256 * MIB);
    assert_eq!(cost, 10 * MIB + 256 * MIB * 42 / 10);
    assert!(cost > 1000 * MIB && cost < 1100 * MIB, "{} MiB", cost / MIB);
    assert_eq!(ProposeConfig::accept_ceiling(256 * MIB), cost * 15 / 10);
    // No overflow at the extremes.
    let _ = ProposeConfig::accept_cost(u64::MAX);
    let _ = ProposeConfig::accept_ceiling(u64::MAX);
}

#[test]
fn a_proposals_working_area_holds_its_upload_and_is_removed_whole() {
    let root = dir("area");
    let store = ArtifactStore::open(&root).unwrap();
    let area = store.new_accept_dir().unwrap();
    assert!(area.in_dir().is_dir() && area.out_dir().is_dir());
    let mut up = area.begin_upload(100).unwrap();
    up.write(b"hello ").unwrap();
    up.write(b"world").unwrap();
    assert_eq!(up.written(), 11);
    let (digest, size) = up.finish().unwrap();
    assert_eq!(size, 11);
    assert_eq!(digest.as_str(), planeter_auth::sha256_hex(b"hello world"));
    assert_eq!(std::fs::read(area.artifact()).unwrap(), b"hello world");
    // The claims file does not exist yet (prikk refuses to overwrite one), and the area is not an object.
    assert!(!area.claims().exists());
    assert!(!store.contains(&digest));
    // Retaining moves the file into the store, once; keeping bytes already there changes nothing.
    assert!(store.retain(&area, &digest).unwrap());
    assert!(store.contains(&digest));
    assert!(!area.artifact().exists());
    let again = store.new_accept_dir().unwrap();
    let mut up = again.begin_upload(100).unwrap();
    up.write(b"hello world").unwrap();
    let (d2, _) = up.finish().unwrap();
    assert_eq!(d2, digest);
    assert!(!store.retain(&again, &d2).unwrap(), "already there");
    let path = area.in_dir();
    drop(area);
    assert!(!path.exists(), "the working area is removed with its guard");
    drop(again);
    assert_eq!(std::fs::read_dir(root.join("work")).unwrap().count(), 0);
}

#[test]
fn an_upload_over_its_limit_is_refused_and_a_crash_leaves_nothing_after_the_next_open() {
    let root = dir("limit");
    let store = ArtifactStore::open(&root).unwrap();
    let area = store.new_accept_dir().unwrap();
    let mut up = area.begin_upload(10).unwrap();
    up.write(b"0123456789").unwrap();
    assert!(matches!(
        up.write(b"x"),
        Err(StoreError::TooLarge { limit: 10 })
    ));
    // A crash: the guard never runs. Opening the store again removes the working area.
    let leftover = area.in_dir();
    std::mem::forget(area);
    assert!(leftover.exists());
    drop(store);
    let reopened = ArtifactStore::open(&root).unwrap();
    assert!(
        !leftover.exists(),
        "an orphaned working area is removed at open"
    );
    assert!(reopened.cleaned_partials() >= 1);
    assert!(reopened.list().unwrap().is_empty());
}

#[test]
fn only_so_many_uploads_run_at_once_and_the_rest_are_told_to_retry() {
    let root = dir("slots");
    let hosting = Arc::new(HostingService::new(
        root.join("repos"),
        Arc::new(InMemoryRepositoryStore::new()) as Arc<dyn RepositoryStore>,
    ));
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        || 0,
        Duration::from_secs(1),
    ));
    let metrics = Arc::new(Metrics::new());
    let export = Arc::new(ExportService::new(
        hosting.clone(),
        Spool::open(root.join("spool"), 1 << 20).unwrap(),
        leases.clone(),
        Arc::new(InMemoryRevisions::new()) as Arc<dyn RevisionStore>,
        ExportConfig::default(),
        metrics.clone(),
    ));
    let svc = ProposeService::new(
        hosting,
        Arc::new(ArtifactStore::open(root.join("artifacts")).unwrap()),
        leases,
        Arc::new(InMemoryChangeStore::new()) as Arc<dyn ChangeStore>,
        export,
        ProposeConfig {
            max_concurrent: 2,
            lease_wait: Duration::from_millis(100),
            ..ProposeConfig::default()
        },
        metrics,
    );
    let a = svc.start_upload(10).unwrap();
    let b = svc.start_upload(10).unwrap();
    let started = std::time::Instant::now();
    match svc.start_upload(10) {
        Err(ProposeError::Busy { retry_after }) => {
            assert!(retry_after >= Duration::from_millis(100))
        }
        other => panic!("expected busy, got {:?}", other.map(|_| ())),
    }
    assert!(
        started.elapsed() >= Duration::from_millis(100),
        "it waited its bound first"
    );
    // A slot comes back when an upload ends (finished or dropped).
    drop(a);
    let c = svc.start_upload(10).unwrap();
    drop((b, c));
    assert!(svc.start_upload(10).is_ok());
}
