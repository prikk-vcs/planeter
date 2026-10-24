//! The `extract` driver (RFC 005 part 5b, T3): prikk 0.54's `extract-report-v1` and `refusal-v1` read strictly, the destination
//! made safe by the driver (prikk 0.54.x does not refuse one inside the repository), the extracted tree never read.
//!
//! Three kinds of test: pure ones (the readers), ones against a stand-in `prikk` that records its argv and prints a document
//! (they run everywhere), and ones against the real prikk (they self-skip, printing why, without it). The real documents in the
//! fixtures were captured from prikk 0.54.1.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_prikk::extract::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES, ExtractError, ExtractOptions, ExtractPoint,
    ExtractWorkdir, NoCheckpointReason, PointKind, RefusalId, parse_refusal, parse_report,
    unknown_refusals_seen,
};
use planeter_prikk::{CliPrikkRepo, PrikkError, Sandbox};

// ------------------------------------------------------------------------------------------------------------ fixtures

const REPORT: &str = r#"{
  "schema_version": "extract-report-v1",
  "extract_repository": "/srv/r/.prikk",
  "point": "heads/main",
  "point_kind": "ref",
  "target_block_id": "414c9ed05898b53d32eabb3505d63d2f8d7090f404e2b10670e8b4c4fcfb5feb",
  "claim_parent_block_id": null,
  "provisional": false,
  "written": true,
  "file_count": 1,
  "total_bytes": 3,
  "largest_file_bytes": 3
}"#;

fn refusal_doc(id: &str, reason: Option<&str>) -> String {
    let reason = reason.map_or("null".to_owned(), |r| format!("\"{r}\""));
    format!(
        r#"{{"schema_version": "refusal-v1", "command": "extract", "refusal": "{id}", "reason": {reason}, "message": "precondition not met: text of {id}"}}"#
    )
}

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("extract-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A stand-in `prikk`: answers `--version`, logs the argv of everything else to `argv.log`, and then does what `mode` says.
fn stand_in(base: &Path, body: &str) -> PathBuf {
    let bin = base.join("prikk");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'prikk 0.54.1'; exit 0; fi\necho \"$*\" >> {log}\n{body}\n",
            log = base.join("argv.log").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    wait_until_runnable(&bin);
    bin
}

/// A script that was written a moment ago may be "busy" (ETXTBSY) if another test thread forked while our write handle was open;
/// the child holds the handle only until its own `exec`. Wait until the script can be run.
fn wait_until_runnable(bin: &Path) {
    for _ in 0..200 {
        match Command::new(bin).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => return,
        }
    }
}

fn argv_log(base: &Path) -> Vec<String> {
    std::fs::read_to_string(base.join("argv.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

struct World {
    base: PathBuf,
    repo: PathBuf,
    work: PathBuf,
}

fn world(name: &str, body: &str) -> (World, CliPrikkRepo) {
    let base = scratch(name);
    let repo = base.join("repo");
    let work = base.join("area");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let bin = stand_in(&base, body);
    let driver = CliPrikkRepo::with_binary(&repo, &bin).with_sandbox(Sandbox::Unconfined);
    (World { base, repo, work }, driver)
}

fn point() -> ExtractPoint {
    ExtractPoint::Ref("heads/main".to_owned())
}

// ------------------------------------------------------------------------------------------------------------ the readers

#[test]
fn the_real_report_is_read_and_every_field_is_required() {
    let r = parse_report(REPORT).unwrap();
    assert_eq!(r.point, "heads/main");
    assert_eq!(r.point_kind, PointKind::Ref);
    assert_eq!(r.claim_parent_block_id, None);
    assert!(!r.provisional && r.written);
    assert_eq!(
        (r.file_count, r.total_bytes, r.largest_file_bytes),
        (1, 3, 3)
    );
    for field in [
        "point",
        "target_block_id",
        "claim_parent_block_id",
        "provisional",
        "written",
        "file_count",
        "total_bytes",
        "largest_file_bytes",
        "extract_repository",
        "point_kind",
    ] {
        let mut v: serde_json::Value = serde_json::from_str(REPORT).unwrap();
        v.as_object_mut().unwrap().remove(field);
        assert!(
            parse_report(&v.to_string()).is_err(),
            "a report without `{field}` is refused"
        );
    }
    // A field of the wrong type, another schema, and a field prikk adds later (tolerated).
    let mut v: serde_json::Value = serde_json::from_str(REPORT).unwrap();
    v["file_count"] = serde_json::json!("1");
    assert!(parse_report(&v.to_string()).is_err());
    let mut v: serde_json::Value = serde_json::from_str(REPORT).unwrap();
    v["schema_version"] = serde_json::json!("extract-report-v2");
    assert!(matches!(
        parse_report(&v.to_string()),
        Err(PrikkError::Parse(_))
    ));
    let mut v: serde_json::Value = serde_json::from_str(REPORT).unwrap();
    v["a_later_field"] = serde_json::json!(1);
    assert!(parse_report(&v.to_string()).is_ok());
}

#[test]
fn all_thirteen_refusal_ids_read_to_their_own_value_and_never_to_another() {
    let expected = [
        ("over-max-bytes", RefusalId::OverMaxBytes),
        ("over-max-files", RefusalId::OverMaxFiles),
        ("destination-exists", RefusalId::DestinationExists),
        (
            "temporary-sibling-exists",
            RefusalId::TemporarySiblingExists,
        ),
        ("point-not-found", RefusalId::PointNotFound),
        ("claim-not-found", RefusalId::ClaimNotFound),
        ("not-a-claim", RefusalId::NotAClaim),
        (
            "claim-has-several-parents",
            RefusalId::ClaimHasSeveralParents,
        ),
        ("case-collision", RefusalId::CaseCollision),
        ("no-checkpoint-in-reach", RefusalId::NoCheckpointInReach),
        ("damaged-repository", RefusalId::DamagedRepository),
        ("io-failure", RefusalId::IoFailure),
        ("not-a-repository", RefusalId::NotARepository),
    ];
    assert_eq!(expected.len(), 13);
    assert_eq!(RefusalId::KNOWN.len(), 13);
    for (id, want) in expected {
        let r = parse_refusal(&refusal_doc(id, None)).unwrap();
        assert_eq!(r.refusal, want, "{id}");
        assert_eq!(r.refusal.id(), id);
        assert!(RefusalId::KNOWN.contains(&id));
        assert_eq!(r.command, "extract");
        assert_eq!(r.reason, None);
    }
}

#[test]
fn the_three_reasons_are_a_closed_set_and_an_unknown_one_is_kept_and_counted() {
    for (s, want) in [
        ("signer-not-adopted", NoCheckpointReason::SignerNotAdopted),
        ("not-recorded", NoCheckpointReason::NotRecorded),
        ("no-checkpoint", NoCheckpointReason::NoCheckpoint),
    ] {
        let r = parse_refusal(&refusal_doc("no-checkpoint-in-reach", Some(s))).unwrap();
        assert_eq!(r.reason, Some(want));
    }
    let before = unknown_refusals_seen();
    let r = parse_refusal(&refusal_doc("no-checkpoint-in-reach", Some("a-new-reason"))).unwrap();
    assert_eq!(
        r.reason,
        Some(NoCheckpointReason::Other("a-new-reason".to_owned()))
    );
    assert!(
        unknown_refusals_seen() > before,
        "the catch-all is counted (and logged)"
    );
}

#[test]
fn an_unknown_refusal_id_is_the_named_catch_all_never_a_known_id() {
    let before = unknown_refusals_seen();
    let r = parse_refusal(&refusal_doc("a-fourteenth-id", None)).unwrap();
    assert_eq!(r.refusal, RefusalId::Other("a-fourteenth-id".to_owned()));
    assert_ne!(r.refusal, RefusalId::DamagedRepository);
    assert!(unknown_refusals_seen() > before);
    // Near misses are not the known ids.
    for near in [
        "Damaged-Repository",
        "damaged_repository",
        "over-max-byte",
        "",
    ] {
        assert!(
            matches!(
                parse_refusal(&refusal_doc(near, None)).unwrap().refusal,
                RefusalId::Other(_)
            ),
            "{near:?}"
        );
    }
}

#[test]
fn a_refusal_missing_a_field_or_of_another_schema_is_not_read() {
    for field in ["command", "refusal", "reason", "message"] {
        let mut v: serde_json::Value =
            serde_json::from_str(&refusal_doc("point-not-found", None)).unwrap();
        v.as_object_mut().unwrap().remove(field);
        assert!(parse_refusal(&v.to_string()).is_err(), "`{field}`");
    }
    let mut v: serde_json::Value =
        serde_json::from_str(&refusal_doc("point-not-found", None)).unwrap();
    v["schema_version"] = serde_json::json!("refusal-v2");
    assert!(parse_refusal(&v.to_string()).is_err());
    assert!(parse_refusal("not json").is_err());
}

#[test]
fn the_default_bounds_are_64_mib_and_10_000_files_in_one_place() {
    let o = ExtractOptions::default();
    assert_eq!((o.max_bytes, o.max_files), (Some(64 << 20), Some(10_000)));
    assert_eq!((DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES), (64 << 20, 10_000));
    assert!(!o.plan_only && !o.allow_full_replay);
}

// ------------------------------------------------------------------------------------------------------------ the destination

#[test]
fn a_working_area_inside_the_repository_or_through_a_link_to_it_or_around_it_is_refused() {
    let base = scratch("guard");
    let repo = base.join("repo");
    std::fs::create_dir_all(repo.join(".prikk")).unwrap();
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    // the repository itself, a directory inside it, `.prikk`
    for inside in [repo.clone(), repo.join("sub"), repo.join(".prikk")] {
        let e = ExtractWorkdir::create(&inside, &repo).unwrap_err();
        assert!(
            matches!(e, ExtractError::UnsafeDestination(_)),
            "{}: {e}",
            inside.display()
        );
    }
    // a link that lives outside and points into the repository
    let link = base.join("link-to-repo");
    std::os::unix::fs::symlink(&repo, &link).unwrap();
    let e = ExtractWorkdir::create(&link, &repo).unwrap_err();
    assert!(matches!(e, ExtractError::UnsafeDestination(_)), "{e}");
    let link2 = base.join("link-into-repo");
    std::os::unix::fs::symlink(repo.join("sub"), &link2).unwrap();
    assert!(ExtractWorkdir::create(&link2, &repo).is_err());
    // a directory that contains the repository (the sandbox would see the repository through both mounts)
    assert!(ExtractWorkdir::create(&base, &repo).is_err());
    // a relative path, and one that does not exist
    assert!(ExtractWorkdir::create(Path::new("area"), &repo).is_err());
    assert!(ExtractWorkdir::create(&base.join("nowhere"), &repo).is_err());
    // refused means nothing was created
    assert!(!repo.join("work").exists() && !base.join("work").exists());
    // a `work` that is a link someone put there
    let area = base.join("area");
    std::fs::create_dir_all(&area).unwrap();
    std::os::unix::fs::symlink(&repo, area.join("work")).unwrap();
    assert!(ExtractWorkdir::create(&area, &repo).is_err());
    assert!(
        std::fs::read_dir(repo.join("sub"))
            .unwrap()
            .next()
            .is_none(),
        "nothing was written into the repository"
    );
}

#[test]
fn a_good_working_directory_has_a_fresh_destination_and_is_removed_whole_when_dropped() {
    let (w, _) = world("workdir", "exit 0");
    let dir;
    {
        let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
        dir = wd.out_dir().parent().unwrap().to_path_buf();
        assert!(wd.out_dir().is_dir());
        assert_eq!(wd.destination(), wd.out_dir().join("tree"));
        assert!(!wd.destination().exists(), "prikk makes it");
        assert!(
            wd.destination()
                .starts_with(w.work.canonicalize().unwrap().join("work"))
        );
        assert!(!wd.destination().starts_with(w.repo.canonicalize().unwrap()));
        std::fs::create_dir_all(wd.destination().join("deep")).unwrap();
        std::fs::write(wd.destination().join("deep/f"), b"x").unwrap();
        std::os::unix::fs::symlink("/", wd.destination().join("deep/link-out")).unwrap();
        let other = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
        assert_ne!(
            other.destination(),
            wd.destination(),
            "every call has its own"
        );
    }
    assert!(
        !dir.exists(),
        "everything under it is gone, links included, and what they pointed at is untouched"
    );
    assert!(Path::new("/usr").exists());
}

#[test]
fn a_working_directory_made_for_another_repository_that_lies_inside_this_one_is_refused_before_any_prikk_call()
 {
    let (w, driver) = world(
        "other-repo",
        &format!("echo '{}'", REPORT.replace('\n', " ")),
    );
    // A repository A elsewhere; the working area is under it; the driver is bound to a repository B that contains A's area.
    let outer = w.base.join("outer");
    let area = outer.join("inner-area");
    let other_repo = w.base.join("other-repo");
    std::fs::create_dir_all(&area).unwrap();
    std::fs::create_dir_all(&other_repo).unwrap();
    let wd = ExtractWorkdir::create(&area, &other_repo).unwrap();
    let inside =
        CliPrikkRepo::with_binary(&outer, w.base.join("prikk")).with_sandbox(Sandbox::Unconfined);
    let e = inside
        .extract(&point(), &wd, &ExtractOptions::default())
        .unwrap_err();
    assert!(matches!(e, ExtractError::UnsafeDestination(_)), "{e}");
    assert!(
        argv_log(&w.base).is_empty(),
        "prikk was never called: {:?}",
        argv_log(&w.base)
    );
    // And a destination that already exists is refused before the call as well.
    let wd2 = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    std::fs::create_dir_all(wd2.destination()).unwrap();
    let e = driver
        .extract(&point(), &wd2, &ExtractOptions::default())
        .unwrap_err();
    assert!(matches!(e, ExtractError::UnsafeDestination(_)), "{e}");
    assert!(argv_log(&w.base).is_empty());
}

// ------------------------------------------------------------------------------------------------------------ the call

#[test]
fn the_call_is_the_documented_argv_and_the_destination_it_names_is_ours_and_outside_the_repository()
{
    let (w, driver) = world("argv", &format!("echo '{}'", REPORT.replace('\n', " ")));
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    let out = driver
        .extract(&point(), &wd, &ExtractOptions::default())
        .unwrap();
    assert_eq!(out.tree.as_deref(), Some(wd.destination()));
    assert!(out.report.written);
    let log = argv_log(&w.base);
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0],
        format!(
            "extract --ref heads/main {} --format json --max-bytes 67108864 --max-files 10000",
            wd.destination().display()
        )
    );
    let repo_real = w.repo.canonicalize().unwrap();
    assert!(!wd.destination().starts_with(&repo_real));
    // The other points and options.
    let plan = ExtractOptions {
        plan_only: true,
        allow_full_replay: true,
        max_bytes: Some(5),
        max_files: None,
    };
    let rep = REPORT.replace("\"written\": true", "\"written\": false");
    let (w2, d2) = world("argv2", &format!("echo '{}'", rep.replace('\n', " ")));
    let wd2 = ExtractWorkdir::create(&w2.work, &w2.repo).unwrap();
    let out = d2
        .extract(&ExtractPoint::Block("ab".repeat(32)), &wd2, &plan)
        .unwrap();
    assert_eq!(out.tree, None, "a plan has no tree");
    assert_eq!(
        argv_log(&w2.base)[0],
        format!(
            "extract --ref {} {} --format json --plan-only --allow-full-replay --max-bytes 5",
            "ab".repeat(32),
            wd2.destination().display()
        )
    );
}

#[test]
fn a_report_that_disagrees_with_the_call_is_not_trusted() {
    // `written` says true for a plan; a claim point answered as a ref; a claim parent for a ref point; the largest above the total.
    let plan = ExtractOptions {
        plan_only: true,
        ..ExtractOptions::default()
    };
    for (opts, pt, doc) in [
        (plan.clone(), point(), REPORT.to_owned()),
        (
            ExtractOptions::default(),
            ExtractPoint::Claim("cd".repeat(32)),
            REPORT.to_owned(),
        ),
        (
            ExtractOptions::default(),
            point(),
            REPORT.replace(
                "\"claim_parent_block_id\": null",
                &format!("\"claim_parent_block_id\": \"{}\"", "ef".repeat(32)),
            ),
        ),
        (
            ExtractOptions::default(),
            point(),
            REPORT.replace("\"largest_file_bytes\": 3", "\"largest_file_bytes\": 4"),
        ),
    ] {
        let (w, d) = world("mismatch", &format!("echo '{}'", doc.replace('\n', " ")));
        let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
        let e = d.extract(&pt, &wd, &opts).unwrap_err();
        assert!(
            matches!(e, ExtractError::Prikk(PrikkError::Parse(_))),
            "{e}"
        );
    }
}

#[test]
fn a_refusal_on_stdout_with_exit_one_is_read_and_a_failure_without_one_is_the_usual_typed_error() {
    let doc = refusal_doc("over-max-files", None);
    let (w, d) = world(
        "refused",
        &format!("echo '{doc}'; echo 'error: x' >&2; exit 1"),
    );
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    match d
        .extract(&point(), &wd, &ExtractOptions::default())
        .unwrap_err()
    {
        ExtractError::Refused(r) => assert_eq!(r.refusal, RefusalId::OverMaxFiles),
        e => panic!("{e}"),
    }
    // exit 1 and garbage on stdout: a plain command failure with prikk's stderr.
    let (w, d) = world("garbage", "echo 'not a document'; echo 'boom' >&2; exit 1");
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    match d
        .extract(&point(), &wd, &ExtractOptions::default())
        .unwrap_err()
    {
        ExtractError::Prikk(PrikkError::Command { code, stderr }) => {
            assert_eq!(code, Some(1));
            assert!(stderr.contains("boom"));
        }
        e => panic!("{e}"),
    }
    // exit 2 (usage) with a refusal document on stdout is not a refusal.
    let (w, d) = world("usage", &format!("echo '{doc}'; exit 2"));
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    assert!(matches!(
        d.extract(&point(), &wd, &ExtractOptions::default())
            .unwrap_err(),
        ExtractError::Prikk(PrikkError::Command { code: Some(2), .. })
    ));
    // a refusal of another command
    let other = doc.replace("\"command\": \"extract\"", "\"command\": \"verify\"");
    let (w, d) = world("othercmd", &format!("echo '{other}'; exit 1"));
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    assert!(matches!(
        d.extract(&point(), &wd, &ExtractOptions::default())
            .unwrap_err(),
        ExtractError::Prikk(PrikkError::Parse(_))
    ));
}

#[test]
fn an_extract_that_hits_the_memory_ceiling_is_a_ceiling_error_not_a_refusal() {
    let (w, d) = world(
        "ceiling",
        "echo 'memory allocation of 99 bytes failed' >&2; kill -ABRT $$",
    );
    let d = d.with_limits(planeter_prikk::CallLimits::data_ceiling(1 << 30));
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    assert!(matches!(
        d.extract(&point(), &wd, &ExtractOptions::default())
            .unwrap_err(),
        ExtractError::Prikk(PrikkError::CeilingExceeded { .. })
    ));
}

// ------------------------------------------------------------------------------------------------------------ (f) the pending parent

#[test]
fn damaged_repository_for_a_claim_is_not_reported_as_damage_but_for_a_ref_it_is_the_refusal_it_says()
 {
    let doc = r#"{"schema_version": "refusal-v1", "command": "extract", "refusal": "damaged-repository", "reason": null, "message": "integrity error: missing Block 24ed83908b30253b204668051d9a1170ea3a8799f727b825d5278c291ae5120f"}"#;
    let (w, d) = world("damaged", &format!("echo '{doc}'; exit 1"));
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    let e = d
        .extract(
            &ExtractPoint::Claim("63".repeat(32)),
            &wd,
            &ExtractOptions::default(),
        )
        .unwrap_err();
    let ExtractError::ClaimParentNotSealedOrDamaged(r) = &e else {
        panic!("{e}")
    };
    assert_eq!(r.refusal, RefusalId::DamagedRepository);
    let text = e.to_string();
    assert!(
        text.contains("still pending") && text.contains("damaged"),
        "both possibilities are said: {text}"
    );
    // The same document for a ref or a block is the refusal, as named.
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    assert!(
        matches!(d.extract(&point(), &wd, &ExtractOptions::default()).unwrap_err(), ExtractError::Refused(r) if r.refusal == RefusalId::DamagedRepository)
    );
}

// ------------------------------------------------------------------------------------------------------------ (d) the tree is data

#[test]
fn a_symlink_in_the_extracted_tree_is_reported_where_it_is_and_nothing_the_driver_does_follows_it()
{
    // The stand-in "extracts" a tree holding a link to a FIFO that has no writer: opening it for reading would block forever.
    // The driver returns, so it did not open it; the link is still a link and its target is untouched.
    let fifo_dir = scratch("fifo");
    let fifo = fifo_dir.join("trap");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let body = format!(
        "mkdir -p \"$4\"; ln -s {fifo} \"$4/innocent.txt\"; ln -s /etc \"$4/etc\"; echo '{report}'",
        fifo = fifo.display(),
        report = REPORT.replace('\n', " ")
    );
    let (w, d) = world("symlink", &body);
    let wd = ExtractWorkdir::create(&w.work, &w.repo).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let tree_path = wd.destination().to_path_buf();
    let handle = std::thread::spawn(move || {
        let r = d.extract(&point(), &wd, &ExtractOptions::default());
        let ok = r.is_ok();
        let tree = r.ok().and_then(|x| x.tree);
        // the checks, while the guard (moved in here) is still alive
        let link = tree.as_ref().map(|t| t.join("innocent.txt"));
        let still_a_link = link.as_ref().is_some_and(|l| {
            l.symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        });
        let etc_is_a_link = tree.as_ref().is_some_and(|t| {
            t.join("etc")
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        });
        let _ = tx.send((ok, still_a_link, etc_is_a_link));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the driver returned: it did not open the link");
    assert_eq!(got, (true, true, true));
    handle.join().unwrap();
    assert!(
        fifo.exists() && !tree_path.exists(),
        "the guard removed the tree; the target of the link is still there"
    );
}

// ------------------------------------------------------------------------------------------------------------ real prikk

fn have_prikk() -> bool {
    Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() >= "prikk 0.54.0")
        .unwrap_or(false)
}

fn bwrap_works() -> bool {
    Command::new("bwrap")
        .args([
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/lib",
            "/lib64",
            "--symlink",
            "usr/bin",
            "/bin",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--unshare-all",
            "/usr/bin/true",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn prikk_in(dir: &Path, keys: &Path, args: &[&str]) -> String {
    let out = Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A keyed client with `blocks` sealed commits (one file each); returns (repo, keys).
fn client(base: &Path, blocks: usize) -> (PathBuf, PathBuf) {
    let (repo, keys) = (base.join("client"), base.join("client-keys"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    prikk_in(&repo, &keys, &["setup"]);
    for i in 0..blocks {
        std::fs::write(repo.join(format!("f{i}.txt")), format!("file {i}\n")).unwrap();
        prikk_in(
            &repo,
            &keys,
            &["commit", "--from-worktree", "-m", &format!("c{i}")],
        );
        prikk_in(&repo, &keys, &["seal", "--allow-no-audit"]);
    }
    (repo, keys)
}

#[test]
fn real_prikk_extracts_a_ref_into_the_working_directory_and_a_plan_writes_nothing() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk >= 0.54.0");
        return;
    }
    let base = scratch("real-ref");
    let (repo, _) = client(&base, 3);
    let area = base.join("area");
    std::fs::create_dir_all(&area).unwrap();
    let driver = CliPrikkRepo::open(&repo)
        .unwrap()
        .with_sandbox(Sandbox::Unconfined);
    let wd = ExtractWorkdir::create(&area, &repo).unwrap();
    let out = driver
        .extract(&point(), &wd, &ExtractOptions::default())
        .unwrap();
    let r = &out.report;
    assert_eq!(
        (
            r.point.as_str(),
            r.point_kind,
            r.file_count,
            r.total_bytes,
            r.largest_file_bytes,
            r.provisional,
            r.written
        ),
        ("heads/main", PointKind::Ref, 3, 3 * 7, 7, false, true)
    );
    assert_eq!(
        std::fs::read_to_string(wd.destination().join("f2.txt")).unwrap(),
        "file 2\n"
    );
    // the repository was not touched: no extract directory, no worktree file of ours
    assert!(!repo.join("tree").exists() && !repo.join("work").exists());
    // a plan: nothing written, the same numbers
    let wd2 = ExtractWorkdir::create(&area, &repo).unwrap();
    let plan = driver
        .extract(
            &point(),
            &wd2,
            &ExtractOptions {
                plan_only: true,
                ..ExtractOptions::default()
            },
        )
        .unwrap();
    assert!(!plan.report.written && plan.tree.is_none() && !wd2.destination().exists());
    assert_eq!(
        (
            plan.report.file_count,
            plan.report.total_bytes,
            plan.report.largest_file_bytes
        ),
        (3, 21, 7)
    );
    // a block id is a point too (through --ref)
    let tip = out.report.target_block_id.clone();
    let wd3 = ExtractWorkdir::create(&area, &repo).unwrap();
    let by_block = driver
        .extract(&ExtractPoint::Block(tip), &wd3, &ExtractOptions::default())
        .unwrap();
    assert_eq!(by_block.report.point_kind, PointKind::Block);
}

#[test]
fn real_prikk_extracts_under_the_production_sandbox_with_only_the_working_directory_writable() {
    if !have_prikk() || !bwrap_works() {
        eprintln!("skipping: needs prikk >= 0.54.0 and bubblewrap");
        return;
    }
    let base = scratch("real-bwrap");
    let (repo, _) = client(&base, 2);
    let area = base.join("area");
    std::fs::create_dir_all(&area).unwrap();
    let driver = CliPrikkRepo::open(&repo).unwrap();
    let wd = ExtractWorkdir::create(&area, &repo).unwrap();
    let out = driver
        .extract(&point(), &wd, &ExtractOptions::default())
        .expect("the sandboxed extract");
    assert_eq!(out.report.file_count, 2);
    assert!(wd.destination().join("f0.txt").is_file());
}

/// Each refusal that can be provoked with a real repository, as the driver reads it. (The rest are from prikk's source.)
#[test]
fn real_prikk_refusals_provoked_here_read_to_their_ids() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk >= 0.54.0");
        return;
    }
    let base = scratch("real-refusals");
    let (repo, keys) = client(&base, 2);
    let area = base.join("area");
    std::fs::create_dir_all(&area).unwrap();
    let driver = CliPrikkRepo::open(&repo)
        .unwrap()
        .with_sandbox(Sandbox::Unconfined);
    let refused = |p: ExtractPoint, o: ExtractOptions| -> planeter_prikk::extract::Refusal {
        let wd = ExtractWorkdir::create(&area, &repo).unwrap();
        match driver.extract(&p, &wd, &o).unwrap_err() {
            ExtractError::Refused(r) => r,
            e => panic!("{e}"),
        }
    };
    let id = |p, o| refused(p, o).refusal;
    assert_eq!(
        id(
            ExtractPoint::Ref("heads/nope".into()),
            ExtractOptions::default()
        ),
        RefusalId::PointNotFound
    );
    assert_eq!(
        id(
            point(),
            ExtractOptions {
                max_files: Some(1),
                ..ExtractOptions::default()
            }
        ),
        RefusalId::OverMaxFiles
    );
    assert_eq!(
        id(
            point(),
            ExtractOptions {
                max_bytes: Some(1),
                ..ExtractOptions::default()
            }
        ),
        RefusalId::OverMaxBytes
    );
    assert_eq!(
        id(
            ExtractPoint::Claim("00".repeat(32)),
            ExtractOptions::default()
        ),
        RefusalId::ClaimNotFound
    );
    let tip = driver
        .extract(
            &point(),
            &ExtractWorkdir::create(&area, &repo).unwrap(),
            &ExtractOptions {
                plan_only: true,
                ..ExtractOptions::default()
            },
        )
        .unwrap()
        .report
        .target_block_id;
    assert_eq!(
        id(ExtractPoint::Claim(tip), ExtractOptions::default()),
        RefusalId::NotAClaim
    );
    // not a repository: the driver bound to a directory with no `.prikk`
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let d = CliPrikkRepo::with_binary(&empty, "prikk").with_sandbox(Sandbox::Unconfined);
    let wd = ExtractWorkdir::create(&area, &empty).unwrap();
    assert!(
        matches!(d.extract(&point(), &wd, &ExtractOptions::default()).unwrap_err(), ExtractError::Refused(r) if r.refusal == RefusalId::NotARepository)
    );
    // The ones the driver cannot reach (it makes its own destination) are read from prikk's own raw documents.
    let raw = |dest: &Path, extra: &[&str]| -> String {
        let mut args = vec![
            "extract",
            "--ref",
            "heads/main",
            dest.to_str().unwrap(),
            "--format",
            "json",
        ];
        args.extend(extra);
        let out = Command::new("prikk")
            .args(&args)
            .current_dir(&repo)
            .env("HOME", &keys)
            .env("XDG_CONFIG_HOME", &keys)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let exists = base.join("exists");
    std::fs::create_dir_all(&exists).unwrap();
    assert_eq!(
        parse_refusal(&raw(&exists, &[])).unwrap().refusal,
        RefusalId::DestinationExists
    );
    let sibling = base.join("sib");
    std::fs::create_dir_all(base.join("sib.prikk-extract-tmp")).unwrap();
    assert_eq!(
        parse_refusal(&raw(&sibling, &[])).unwrap().refusal,
        RefusalId::TemporarySiblingExists
    );
    assert_eq!(
        parse_refusal(&raw(&base.join("no/such/parent/x"), &[]))
            .unwrap()
            .refusal,
        RefusalId::IoFailure
    );
}

/// No admitted checkpoint in reach: a history of 70 blocks received from a client whose key the receiver has not adopted.
/// prikk says `not-recorded` here (a block with a snapshot is in reach; the receiver never recorded it); after `verify` it extracts.
#[test]
fn real_prikk_no_checkpoint_in_reach_carries_its_reason_and_the_full_replay_is_provisional() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk >= 0.54.0");
        return;
    }
    let base = scratch("real-nocp");
    let (crepo, ckeys) = client(&base, 70);
    let archive = base.join("c.archive");
    prikk_in(
        &crepo,
        &ckeys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    let (forge, fkeys) = (base.join("forge"), base.join("forge-keys"));
    std::fs::create_dir_all(&forge).unwrap();
    std::fs::create_dir_all(&fkeys).unwrap();
    prikk_in(&forge, &fkeys, &["init", "."]);
    prikk_in(
        &forge,
        &fkeys,
        &["archive", "import", archive.to_str().unwrap()],
    );
    let area = base.join("area");
    std::fs::create_dir_all(&area).unwrap();
    // the forge's driver reads with the forge's own (empty) key directory
    let driver = CliPrikkRepo::with_binary(&forge, "prikk").with_sandbox(Sandbox::Unconfined);
    let p = ExtractPoint::Ref("remotes/heads/main".to_owned());
    let wd = ExtractWorkdir::create(&area, &forge).unwrap();
    match driver
        .extract(&p, &wd, &ExtractOptions::default())
        .unwrap_err()
    {
        ExtractError::Refused(r) => {
            assert_eq!(r.refusal, RefusalId::NoCheckpointInReach);
            assert_eq!(r.reason, Some(NoCheckpointReason::NotRecorded));
        }
        e => panic!("{e}"),
    }
    let wd = ExtractWorkdir::create(&area, &forge).unwrap();
    let full = driver
        .extract(
            &p,
            &wd,
            &ExtractOptions {
                allow_full_replay: true,
                ..ExtractOptions::default()
            },
        )
        .unwrap();
    assert!(full.report.provisional, "a full replay is provisional");
    assert_eq!(full.report.file_count, 70);
}

/// (f) A claim whose parent is still pending: prikk 0.54.x refuses the second claim of an accepted artifact with
/// `damaged-repository` ("missing Block …"), although nothing is damaged. The driver says both possibilities.
#[test]
fn real_prikk_a_claim_whose_parent_is_pending_is_not_called_damage() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk >= 0.54.0");
        return;
    }
    let base = scratch("real-claims");
    let (crepo, ckeys) = client(&base, 1);
    let archive = base.join("c.archive");
    prikk_in(
        &crepo,
        &ckeys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    let (forge, fkeys) = (base.join("forge"), base.join("forge-keys"));
    std::fs::create_dir_all(&forge).unwrap();
    std::fs::create_dir_all(&fkeys).unwrap();
    prikk_in(&forge, &fkeys, &["init", "."]);
    let info: serde_json::Value = serde_json::from_str(&prikk_in(
        &forge,
        &fkeys,
        &[
            "archive",
            "verify",
            archive.to_str().unwrap(),
            "--format",
            "json",
        ],
    ))
    .unwrap();
    let key = &info["maintainer_keys"][0];
    prikk_in(
        &forge,
        &fkeys,
        &[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            key["key_id"].as_str().unwrap(),
            "--public-key",
            key["public_key"].as_str().unwrap(),
        ],
    );
    prikk_in(
        &forge,
        &fkeys,
        &["archive", "import", archive.to_str().unwrap(), "--adopt"],
    );
    let have = base.join("have");
    prikk_in(
        &forge,
        &fkeys,
        &[
            "sync",
            "have",
            "heads/main",
            "--output",
            have.to_str().unwrap(),
        ],
    );
    for name in ["two", "three"] {
        std::fs::write(crepo.join(format!("{name}.txt")), name).unwrap();
        prikk_in(&crepo, &ckeys, &["commit", "--from-worktree", "-m", name]);
        prikk_in(&crepo, &ckeys, &["seal", "--allow-no-audit"]);
    }
    let art = base.join("art");
    prikk_in(
        &crepo,
        &ckeys,
        &[
            "sync",
            "build",
            "heads/main",
            "--have",
            have.to_str().unwrap(),
            "--output",
            art.to_str().unwrap(),
        ],
    );
    let claims = base.join("claims");
    prikk_in(
        &forge,
        &fkeys,
        &[
            "sync",
            "accept",
            art.to_str().unwrap(),
            "--claims-out",
            claims.to_str().unwrap(),
        ],
    );
    let ids: Vec<String> = std::fs::read_to_string(&claims)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(ids.len(), 2);
    let area = base.join("area");
    std::fs::create_dir_all(&area).unwrap();
    let driver = CliPrikkRepo::with_binary(&forge, "prikk").with_sandbox(Sandbox::Unconfined);
    let mut outcomes = Vec::new();
    for id in &ids {
        let wd = ExtractWorkdir::create(&area, &forge).unwrap();
        outcomes.push(driver.extract(
            &ExtractPoint::Claim(id.clone()),
            &wd,
            &ExtractOptions::default(),
        ));
    }
    let ok = outcomes.iter().filter(|o| o.is_ok()).count();
    let pending = outcomes
        .iter()
        .filter(|o| matches!(o, Err(ExtractError::ClaimParentNotSealedOrDamaged(_))))
        .count();
    assert_eq!((ok, pending), (1, 1), "{outcomes:?}");
    let good = outcomes.iter().find_map(|o| o.as_ref().ok()).unwrap();
    assert_eq!(good.report.point_kind, PointKind::Claim);
    assert!(
        good.report.claim_parent_block_id.is_some(),
        "a claim names its parent"
    );
}
