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
        proposal: Some("c-0123456789abcdef".to_owned()),
        ..ProposeMeta::default()
    };
    assert_eq!(existing.validate(), Ok(()));
    for bad in ["", "has space", &"x".repeat(65), "semi;colon"] {
        let m = ProposeMeta {
            proposal: Some(bad.to_owned()),
            ..ProposeMeta::default()
        };
        assert_eq!(m.validate(), Err(MetaError::ProposalIdInvalid), "{bad:?}");
    }
    let m = ProposeMeta {
        title: Some("t".to_owned()),
        ..existing
    };
    assert_eq!(m.validate(), Err(MetaError::FieldsForExistingProposal));
}

const MIB: u64 = 1 << 20;

/// What `prikk sync accept` (**0.58.0**, 2026-11-18/19, the call the server makes) was measured to hold in its **data segment** (`VmData`, what
/// `RLIMIT_DATA` limits), the worst of an empty store and one holding 10,000 pending patches (where the shape was measured on both): `(shape,
/// artifact bytes, patches, blobs, VmData in tenths of a MiB)`. The 0.58 re-baseline (D-97, PK-85); the script and the raw table are in its review
/// request. Every row is a shape somebody could propose; the model has to cover all of them. (The 0.53.0 table of part 4b held 7.0 × the artifact for a
/// single blob; 0.58.0 streams, and holds 4.0 × the largest object.) The 20 MiB file edited 50 times (a 1.4 GiB artifact) is not a row any more: it is
/// over the 256 MiB bound of the server, and the 4 MiB one is the same shape.
const MEASURED: &[(&str, u64, u64, u64, u64)] = &[
    ("one-file patches, 1,000", 511496, 1000, 1000, 112),
    ("one-file patches, 10,000", 5213564, 10000, 10000, 134),
    ("one 1 MiB blob", 1049378, 1, 1, 134),
    ("one patch of 100 files of 100 KiB", 10263864, 1, 100, 104),
    ("one 10 MiB blob (random or zeros)", 10486562, 1, 1, 503),
    ("one 25 MiB blob", 26215202, 1, 1, 1002),
    (
        "one 50 MiB blob (random, zeros or text)",
        52429602,
        1,
        1,
        2104,
    ),
    (
        "one patch of 1,000 files of 100 KiB",
        102634464,
        1,
        1000,
        12,
    ),
    ("1,000 patches of 100 KiB", 102832376, 1000, 1000, 18),
    ("one 100 MiB blob", 104858402, 1, 1, 4002),
    ("25 patches of 4 MiB", 104868700, 25, 25, 163),
    ("100 patches of 1 MiB", 104900950, 100, 100, 134),
    ("one 250 MiB blob (random or zeros)", 262144802, 1, 1, 10002),
    ("a 4 MiB file edited 50 times", 268686725, 51, 1, 411),
];

#[test]
fn the_cost_of_an_accept_is_ten_mib_plus_four_and_a_half_times_the_artifact_plus_one_and_a_half_kib_an_object()
 {
    use planeter_transport::propose::cost_model;
    assert_eq!(cost_model::cost(0, 0), 10 * MIB);
    assert_eq!(cost_model::cost(100 * MIB, 0), 10 * MIB + 450 * MIB);
    assert_eq!(cost_model::cost(0, 5000), 10 * MIB + 5000 * 1536);
    assert_eq!(
        cost_model::ceiling(100 * MIB, 0),
        (10 * MIB + 450 * MIB) * 15 / 10
    );
    // The configuration carries the object bound; the default is the provisional 5,000.
    let c = ProposeConfig::default();
    assert_eq!((c.max_objects, c.max_artifact_bytes), (5_000, 100 * MIB));
    assert_eq!(c.accept_deadline, Duration::from_secs(600));
    let cost = c.accept_cost(c.max_artifact_bytes);
    assert_eq!(cost, cost_model::cost(100 * MIB, 5_000));
    // 10 + 450 + 7.3 MiB = about 467 MiB (0.46 GiB; 720 MiB until 0.57.0)
    assert!(cost > 460 * MIB && cost < 480 * MIB, "{} MiB", cost / MIB);
    assert_eq!(c.accept_ceiling(c.max_artifact_bytes), cost * 15 / 10);
    // The operator can raise the bound: 256 MiB costs about 1.14 GiB (1.8 GiB until 0.57.0).
    let raised = cost_model::cost(256 * MIB, 5_000);
    assert!(
        raised > 1100 * MIB && raised < 1250 * MIB,
        "{} MiB",
        raised / MIB
    );
    // The inverse: what a pool admits, and that it admits no more.
    let pool = cost_model::cost(125 * MIB, 5_000);
    let a = cost_model::largest_artifact_admitted(pool, 5_000).unwrap();
    assert!((125 * MIB..125 * MIB + 2).contains(&a), "{a}");
    assert!(cost_model::cost(a + 16, 5_000) > pool);
    assert_eq!(cost_model::largest_artifact_admitted(MIB, 5_000), None);
    let more = ProposeConfig {
        max_objects: 20_000,
        ..ProposeConfig::default()
    };
    assert!(
        more.accept_cost(0) > c.accept_cost(0),
        "a larger bound costs more"
    );
    // No overflow at the extremes.
    let _ = cost_model::cost(u64::MAX, u64::MAX);
    let _ = cost_model::ceiling(u64::MAX, u64::MAX);
}

#[test]
fn the_model_admits_every_measured_shape_at_no_less_than_what_it_held_and_kills_none_of_them() {
    use planeter_transport::propose::cost_model;
    for (shape, bytes, patches, blobs, tenths) in MEASURED {
        let held = tenths * MIB / 10;
        // The object bound is at least what the artifact declares (the server refuses a larger artifact before the accept).
        let bound = 5_000.max(*patches).max(*blobs);
        let cost = cost_model::cost(*bytes, bound);
        assert!(
            cost >= held,
            "{shape}: admitted at {} MiB, held {} MiB",
            cost / MIB,
            held / MIB
        );
        let ceiling = cost_model::ceiling(*bytes, bound);
        assert!(
            ceiling >= held * 3 / 2,
            "{shape}: the ceiling {} MiB leaves less than 1.5 x the {} MiB it held",
            ceiling / MIB,
            held / MIB
        );
    }
}

#[test]
fn the_ceiling_holds_a_single_blob_of_0_58_at_every_size_and_prikks_outlier_of_five_times() {
    // 0.58.0 holds 4.0 x a single blob (1,000.2 MiB for 250 MiB, six runs); 0.57.0 held 7.0 x it. The model must hold the former with its ceiling
    // (x 1.5), and prikk's own letter names an outlier of 5.0 x (1,256 MB for 250 MiB, once in eleven runs) that the ceiling must hold too.
    let held = |bytes: u64| bytes * 4 + 2 * MIB;
    let outlier = |bytes: u64| bytes * 5;
    let c = ProposeConfig::default();
    for mib in [1, 5, 10, 25, 50, 105, 250, 256] {
        assert!(c.accept_ceiling(mib * MIB) > held(mib * MIB), "{mib} MiB");
        assert!(
            c.accept_ceiling(mib * MIB) > outlier(mib * MIB),
            "{mib} MiB: the 5.0 x outlier"
        );
        assert!(
            c.accept_cost(mib * MIB) > held(mib * MIB),
            "{mib} MiB: the cost admits what is held"
        );
    }
    // And 0.57.0's 7.0 x would NOT be held by this model's cost (the floor is what makes the smaller model true): the cost is below it from 10 MiB.
    assert!(c.accept_cost(100 * MIB) < 700 * MIB);
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
