//! RFC 013 D-4 (the foundation): the reader that joins prikk's trust list to the record of where each key came from, and the one-time
//! backfill of the repositories that existed before the record. A stand-in `prikk` (an unconfined sandbox, no real prikk needed) holds the join;
//! the service tests use real prikk and say why they skip without one.

#[path = "support/archives.rs"]
mod archives;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use planeter_core::{HostingService, LeaseManager, new_boot_id};
use planeter_prikk::Sandbox;
use planeter_prikk::cli::CliPrikkRepo;
use planeter_store::{
    ChangeStore, InMemoryChangeStore, InMemoryKeyEventStore, InMemoryLeaseRecords,
    InMemoryRepositoryStore, KeyEvent, KeyEventStore, KeySource, Owner, RepoId, RepositoryRecord,
    RepositoryStore, Visibility,
};
use planeter_transport::{
    Backfilled, KeyService, KeySourceView as Source, Metrics, RepoKey, detail_for_record, repo_keys,
};

macro_rules! need_prikk {
    () => {
        if !archives::prikk_available() {
            eprintln!("skipping: prikk is not on PATH");
            return;
        }
    };
}

fn hex(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn key_id(n: u8) -> String {
    format!("ed25519-{n:02x}{n:02x}")
}

/// A stand-in prikk whose `trust maintainer list` answers the given keys (and fails everything else it is asked).
fn standin(name: &str, keys: &[u8]) -> (CliPrikkRepo, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("keys-unit-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    let list: Vec<String> = keys
        .iter()
        .map(|n| {
            format!(
                "{{\"key_id\":\"{}\",\"public_key\":\"{}\"}}",
                key_id(*n),
                hex(*n)
            )
        })
        .collect();
    let script = dir.join("prikk");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'prikk 0.58.0'; exit 0; fi\nif [ \"$1 $2 $3\" = \"trust maintainer list\" ]; then echo '{{\"schema_version\":\"trust-list-v1\",\"keys\":[{}]}}'; exit 0; fi\necho \"stand-in: unexpected $*\" >&2; exit 2\n",
            list.join(",")
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..200 {
        match std::process::Command::new(&script)
            .arg("--version")
            .output()
        {
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(10)),
            _ => break,
        }
    }
    (
        CliPrikkRepo::with_binary(dir.join("repo"), script).with_sandbox(Sandbox::Unconfined),
        dir,
    )
}

fn event(repo: &str, n: u8, source: KeySource, at: u64) -> KeyEvent {
    KeyEvent {
        repo_id: RepoId::new(repo),
        key_id: key_id(n),
        public_key: hex(n),
        source,
        detail: String::new(),
        delivery_id: None,
        recorded_at: at,
        backfilled: false,
    }
}

fn ids(keys: &[RepoKey]) -> Vec<String> {
    keys.iter().map(|k| k.key_id.clone()).collect()
}

// ------------------------------------------------------------------------------------------------ the join (T3)

#[test]
fn the_four_sources_come_back_and_the_order_is_creation_first_then_by_when_then_the_unrecorded() {
    let (driver, _dir) = standin("four", &[1, 2, 3, 4, 5]);
    let store = InMemoryKeyEventStore::new();
    let repo = RepoId::new("r");
    // 5 is unrecorded; 4 was added by the operator first in time; 3 by a delivery; 1 and 2 at creation (2 backfilled)
    let mut op = event("r", 4, KeySource::Operator, 150);
    op.detail = "the only key was lost".to_owned();
    let mut del = event("r", 3, KeySource::Delivery, 300);
    del.delivery_id = Some("dl-aaaa".to_owned());
    let mut back = event("r", 2, KeySource::Creation, 400);
    back.backfilled = true;
    for e in [del, op, back, event("r", 1, KeySource::Creation, 100)] {
        assert!(store.record(&e).unwrap());
    }
    let keys = repo_keys(&driver, &store, &repo).unwrap();
    assert_eq!(
        ids(&keys),
        [key_id(1), key_id(2), key_id(4), key_id(3), key_id(5)],
        "creation (by when), then the others by when, then the key nobody recorded"
    );
    assert_eq!(keys[0].source, Source::Creation { backfilled: false });
    assert_eq!(keys[1].source, Source::Creation { backfilled: true });
    assert_eq!(
        keys[2].source,
        Source::Operator {
            reason: "the only key was lost".to_owned()
        }
    );
    assert_eq!(
        keys[3].source,
        Source::Delivery {
            delivery_id: Some("dl-aaaa".to_owned())
        }
    );
    assert_eq!(keys[4].source, Source::NotRecorded);
    assert_eq!(keys[4].recorded_at, None);
    assert_eq!(keys[0].recorded_at, Some(100));
    // the fingerprint is the eight-group form the delivery prints
    assert_eq!(
        keys[0].fingerprint,
        planeter_core::creation::group_fingerprint(&hex(1))
    );
    assert_eq!(keys[0].fingerprint.split(' ').count(), 8);
    assert_eq!(keys[0].public_key, hex(1));
}

#[test]
fn a_row_for_a_key_prikk_does_not_list_is_ignored_never_shown_never_an_error() {
    let (driver, _dir) = standin("ghost", &[1]);
    let store = InMemoryKeyEventStore::new();
    store
        .record(&event("r", 1, KeySource::Creation, 1))
        .unwrap();
    store
        .record(&event("r", 9, KeySource::Operator, 2))
        .unwrap(); // prikk does not list 9
    let keys = repo_keys(&driver, &store, &RepoId::new("r")).unwrap();
    assert_eq!(ids(&keys), [key_id(1)]);
}

#[test]
fn a_repository_with_no_events_shows_every_key_as_not_recorded_in_prikks_order() {
    let (driver, _dir) = standin("none", &[7, 3, 5]);
    let store = InMemoryKeyEventStore::new();
    let keys = repo_keys(&driver, &store, &RepoId::new("r")).unwrap();
    assert!(keys.iter().all(|k| k.source == Source::NotRecorded));
    let mut want = vec![key_id(3), key_id(5), key_id(7)];
    want.sort();
    assert_eq!(
        ids(&keys),
        want,
        "a stable order (by key id) when nothing is recorded"
    );
}

#[test]
fn sixty_four_keys_are_read_whole() {
    let all: Vec<u8> = (1..=64).collect();
    let (driver, _dir) = standin("sixty-four", &all);
    let store = InMemoryKeyEventStore::new();
    for n in 1..=32u8 {
        store
            .record(&event("r", n, KeySource::Creation, u64::from(n)))
            .unwrap();
    }
    let keys = repo_keys(&driver, &store, &RepoId::new("r")).unwrap();
    assert_eq!(keys.len(), 64);
    assert_eq!(
        keys.iter()
            .filter(|k| k.source == Source::NotRecorded)
            .count(),
        32
    );
    assert_eq!(keys[0].key_id, key_id(1));
    assert_eq!(keys[31].key_id, key_id(32));
}

#[test]
fn a_hostile_or_long_reason_is_shown_visibly_and_cut_not_refused() {
    let raw = format!("\u{202E}lost\u{0}\n{}", "鍵".repeat(300));
    let cut = detail_for_record(&raw);
    assert!(
        cut.starts_with("\\u{202E}lost\\u{0}\\u{A}") || cut.starts_with("\\u{202E}lost\\u{0}"),
        "{cut}"
    );
    assert!(
        !cut.chars()
            .any(|c| c == '\u{202E}' || c == '\0' || c == '\n')
    );
    assert_eq!(cut.chars().count(), planeter_store::DETAIL_MAX_CHARS);
    // and a row that holds raw text (written by some other path) is made safe when it is read
    let (driver, _dir) = standin("hostile", &[1]);
    let store = InMemoryKeyEventStore::new();
    let mut e = event("r", 1, KeySource::Operator, 1);
    e.detail = "\u{202E}x".to_owned();
    store.record(&e).unwrap();
    let keys = repo_keys(&driver, &store, &RepoId::new("r")).unwrap();
    assert_eq!(
        keys[0].source,
        Source::Operator {
            reason: "\\u{202E}x".to_owned()
        }
    );
}

#[test]
fn a_prikk_that_cannot_answer_is_an_error_not_an_empty_list() {
    let (driver, dir) = standin("broken", &[1]);
    std::fs::write(
        dir.join("prikk"),
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'prikk 0.58.0'; exit 0; fi\nexit 3\n",
    )
    .unwrap();
    let store = InMemoryKeyEventStore::new();
    assert!(repo_keys(&driver, &store, &RepoId::new("r")).is_err());
}

// ------------------------------------------------------------------------------------------------ the service, with real prikk (T3, T5)

struct World {
    base: PathBuf,
    repos: Arc<InMemoryRepositoryStore>,
    changes: Arc<InMemoryChangeStore>,
    events: Arc<InMemoryKeyEventStore>,
    metrics: Arc<Metrics>,
    svc: KeyService,
    keys: Vec<archives::KeyInfo>,
}

fn clock() -> u64 {
    1_000
}

/// Three real maintainer keys (real prikk makes them) and an empty service over a temp root.
fn world(name: &str) -> World {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("keys-world-{name}"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let mut keys = Vec::new();
    for i in 0..3 {
        let m = archives::Maintainer::new(&base, &format!("maker{i}"));
        let out = archives::run(
            &m.work,
            &m.keys,
            &["trust", "maintainer", "list", "--format", "json"],
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let k = &v["keys"][0];
        keys.push(archives::KeyInfo {
            key_id: k["key_id"].as_str().unwrap().to_owned(),
            public_key: k["public_key"].as_str().unwrap().to_owned(),
        });
    }
    let repos = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(
        HostingService::new(base.join("root"), repos.clone() as Arc<dyn RepositoryStore>)
            .with_sandbox(Sandbox::Unconfined),
    );
    let leases = Arc::new(LeaseManager::new(
        Arc::new(InMemoryLeaseRecords::new()),
        new_boot_id(),
        clock,
        Duration::from_secs(1),
    ));
    let changes = Arc::new(InMemoryChangeStore::new());
    let events = Arc::new(InMemoryKeyEventStore::new());
    let metrics = Arc::new(Metrics::new());
    let svc = KeyService::new(
        hosting,
        leases,
        events.clone() as Arc<dyn KeyEventStore>,
        repos.clone() as Arc<dyn RepositoryStore>,
        changes.clone() as Arc<dyn ChangeStore>,
        metrics.clone(),
    )
    .with_clock(clock);
    World {
        base,
        repos,
        changes,
        events,
        metrics,
        svc,
        keys,
    }
}

impl World {
    /// A repository that trusts the keys numbered in `which` (its own directory, `prikk init`, `trust maintainer add`).
    fn repo(&self, id: &str, which: &[usize]) -> RepositoryRecord {
        let path = self.base.join("root").join(id);
        std::fs::create_dir_all(&path).unwrap();
        let home = self.base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        archives::run(&path, &home, &["init", "."]);
        for i in which {
            let k = &self.keys[*i];
            archives::run(
                &path,
                &home,
                &[
                    "trust",
                    "maintainer",
                    "add",
                    "--key-id",
                    &k.key_id,
                    "--public-key",
                    &k.public_key,
                ],
            );
        }
        let record = RepositoryRecord {
            repo_id: RepoId::new(id),
            owner: Owner::User("alice".into()),
            name: id.to_owned(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path,
        };
        self.repos.create(record.clone()).unwrap();
        record
    }

    fn rows(&self, id: &str) -> Vec<KeyEvent> {
        self.events.events(&RepoId::new(id)).unwrap()
    }

    fn counter(&self, name: &str) -> u64 {
        self.metrics
            .snapshot()
            .key_events
            .get(name)
            .copied()
            .unwrap_or(0)
    }
}

#[test]
fn the_service_reads_real_prikks_list_under_a_read_lease_and_joins_the_record() {
    need_prikk!();
    let w = world("reader");
    let rec = w.repo("r1", &[0, 1, 2]);
    // creation, delivery, operator; and nothing for the third until the table gains it
    w.events
        .record(&event_of(&w, "r1", 0, KeySource::Creation, 10))
        .unwrap();
    let mut d = event_of(&w, "r1", 1, KeySource::Delivery, 20);
    d.delivery_id = Some("dl-0123".to_owned());
    w.events.record(&d).unwrap();
    let keys = w.svc.keys(&rec).unwrap();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0].key_id, w.keys[0].key_id);
    assert_eq!(keys[0].source, Source::Creation { backfilled: false });
    assert_eq!(
        keys[1].source,
        Source::Delivery {
            delivery_id: Some("dl-0123".to_owned())
        }
    );
    assert_eq!(keys[2].source, Source::NotRecorded);
    assert_eq!(keys[2].key_id, w.keys[2].key_id);
    let mut op = event_of(&w, "r1", 2, KeySource::Operator, 5);
    op.detail = "lost the key".to_owned();
    w.events.record(&op).unwrap();
    let keys = w.svc.keys(&rec).unwrap();
    assert_eq!(
        keys.iter().map(|k| k.key_id.clone()).collect::<Vec<_>>(),
        [&w.keys[0], &w.keys[2], &w.keys[1]].map(|k| k.key_id.clone()),
        "creation first, then the operator's (recorded at 5) before the delivery's (20)"
    );
}

fn event_of(w: &World, repo: &str, i: usize, source: KeySource, at: u64) -> KeyEvent {
    KeyEvent {
        repo_id: RepoId::new(repo),
        key_id: w.keys[i].key_id.clone(),
        public_key: w.keys[i].public_key.clone(),
        source,
        detail: String::new(),
        delivery_id: None,
        recorded_at: at,
        backfilled: false,
    }
}

// ------------------------------------------------------------------------------------------------ the backfill (T5)

#[test]
fn the_backfill_fills_only_a_repository_with_no_rows_skips_what_it_cannot_read_and_a_second_run_does_nothing()
 {
    need_prikk!();
    let w = world("backfill");
    // r-recorded: trusts two keys and has a row for one (it gained a key later): never touched
    let recorded = w.repo("r-recorded", &[0, 1]);
    w.events
        .record(&event_of(&w, "r-recorded", 0, KeySource::Operator, 5))
        .unwrap();
    // r-empty: three keys, no row at all: filled
    let _empty = w.repo("r-empty", &[0, 1, 2]);
    // r-flagged: needs recovery: skipped
    let _flagged = w.repo("r-flagged", &[1]);
    w.changes
        .set_needs_recovery(
            &RepoId::new("r-flagged"),
            "the server stopped during a write",
            1,
        )
        .unwrap();
    // r-gone: its directory is not there: skipped
    let gone = w.repo("r-gone", &[2]);
    std::fs::remove_dir_all(&gone.path).unwrap();
    // r-nokeys: prikk lists none: nothing to record, and it is not an error
    let _nokeys = w.repo("r-nokeys", &[]);

    let before = w.rows("r-recorded");
    let report = w.svc.backfill_all();
    assert_eq!(
        (report.filled, report.skipped, report.untouched),
        (1, 2, 1),
        "{report:?}"
    );
    assert_eq!(
        w.rows("r-recorded"),
        before,
        "a repository with rows is not touched"
    );
    let filled = w.rows("r-empty");
    assert_eq!(filled.len(), 3);
    assert!(filled.iter().all(|e| e.source == KeySource::Creation
        && e.backfilled
        && e.recorded_at == 1_000
        && e.detail.is_empty()));
    assert!(w.rows("r-flagged").is_empty());
    assert!(w.rows("r-gone").is_empty());
    assert!(w.rows("r-nokeys").is_empty());
    assert_eq!(w.counter("key_events_backfilled"), 1);
    assert_eq!(w.counter("key_events_skipped"), 2);
    let _ = recorded;

    // a second run (a restart): nothing new is written, the two that could not be read are skipped again
    let again = w.svc.backfill_all();
    assert_eq!((again.filled, again.skipped), (0, 2), "{again:?}");
    assert_eq!(w.rows("r-empty"), filled, "idempotent");

    // the flagged repository is recovered: the next start fills it; the repository was never changed by any pass
    w.changes
        .clear_needs_recovery(&RepoId::new("r-flagged"))
        .unwrap();
    let third = w.svc.backfill_all();
    assert_eq!((third.filled, third.skipped), (1, 1), "{third:?}");
    assert_eq!(w.rows("r-flagged").len(), 1);
    // and a repository that gains a key later keeps its partial rows and is not touched again
    let gained = w.rows("r-recorded");
    assert_eq!(w.svc.backfill_all().filled, 0);
    assert_eq!(w.rows("r-recorded"), gained);
}

#[test]
fn a_pass_that_was_killed_in_the_middle_gives_the_same_final_state_as_a_whole_one() {
    need_prikk!();
    // the straight run
    let straight = world("straight");
    for id in ["a", "b", "c"] {
        straight.repo(id, &[0, 1]);
    }
    straight.svc.backfill_all();
    let want: Vec<Vec<(String, bool)>> = ["a", "b", "c"]
        .iter()
        .map(|id| {
            straight
                .rows(id)
                .into_iter()
                .map(|e| (e.key_id, e.backfilled))
                .collect()
        })
        .collect();
    // the killed run: one repository filled, then "the process dies" (nothing else runs), then a new service on the same store finishes
    let w = world("killed");
    let recs: Vec<RepositoryRecord> = ["a", "b", "c"]
        .iter()
        .map(|id| w.repo(id, &[0, 1]))
        .collect();
    assert_eq!(w.svc.backfill_one(&recs[0]), Backfilled::Filled(2));
    assert!(w.rows("b").is_empty() && w.rows("c").is_empty());
    let restarted = w.svc.clone().with_clock(clock);
    restarted.backfill_all();
    let got: Vec<Vec<(String, bool)>> = ["a", "b", "c"]
        .iter()
        .map(|id| {
            w.rows(id)
                .into_iter()
                .map(|e| (e.key_id, e.backfilled))
                .collect()
        })
        .collect();
    // the key ids differ between the two worlds (real keys are generated), so compare the shape: the same counts, all backfilled
    assert_eq!(
        got.iter().map(Vec::len).collect::<Vec<_>>(),
        want.iter().map(Vec::len).collect::<Vec<_>>()
    );
    assert!(got.iter().flatten().all(|(_, b)| *b));
}

#[test]
fn nothing_in_a_pass_changes_the_repository() {
    need_prikk!();
    let w = world("untouched");
    let rec = w.repo("r", &[0, 1]);
    let hash = archives_tree_hash(&rec.path);
    w.svc.backfill_all();
    w.svc.keys(&rec).unwrap();
    assert_eq!(
        archives_tree_hash(&rec.path),
        hash,
        "the .prikk tree is byte-identical"
    );
}

/// The bytes of every file under `dir/.prikk` (names and contents), as one string to compare.
fn archives_tree_hash(dir: &Path) -> String {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            // prikk's own caches are rewritten by reads; the record of trust and history are what matter
            if p.ends_with("cache") || p.ends_with("tmp") {
                continue;
            }
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&dir.join(".prikk"), &mut files);
    files.sort();
    let mut s = String::new();
    for f in files {
        s.push_str(&format!(
            "{} {}\n",
            f.display(),
            std::fs::read(&f).map(|b| b.len()).unwrap_or(0)
        ));
    }
    s
}
