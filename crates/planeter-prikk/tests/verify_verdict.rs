//! IS-39: `verify` reads **exit 1 with a failing `verify-report-v1`** as the answer it is, and nothing else
//! as a verdict. A stand-in `prikk` plays each shape (no prikk binary needed); real prikk plays the three
//! real corruptions of the accessibility baseline (self-skips without `prikk`).
//!
//! The stand-in is written once and then only executed (the write-then-exec `ETXTBSY` race, as in
//! `trust_check.rs`); every case lives in its own directory as three small files the script prints.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_prikk::sandbox::PRLIMIT;
use planeter_prikk::{CallLimits, CliPrikkRepo, PrikkError, PrikkRepo, Sandbox};

const SCRIPT: &str = "#!/bin/sh\ncat \"$PWD/stdout\" 2>/dev/null\ncat \"$PWD/stderr\" >&2 2>/dev/null\nexit \"$(cat \"$PWD/exit\")\"\n";

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A report with the given verdict, conditions and stages.
fn report(ok: bool, conditions: &[(&str, &str)], stages: &str) -> String {
    let conds = conditions
        .iter()
        .map(|(i, m)| format!(r#"{{"id":"{i}","message":"{m}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"schema_version":"verify-report-v1","verdict":{{"ok":{ok},"failed_conditions":[{conds}]}},"active_sessions":{{"count":1,"verified_count":1}},"stages":[{stages}]}}"#
    )
}

const EVALUATED: &str =
    r#"{"stage":"objects","status":"evaluated"},{"stage":"refs","status":"evaluated"}"#;

/// What the damaged trust-policy container made real prikk 0.52.0 say: one stage failed with a finding.
const FAILED_INTEGRITY: &str = r#"{"stage":"objects","status":"failed","message":"integrity error: trust policy container has a damaged entry; run doctor for diagnosis"},{"stage":"refs","status":"evaluated"}"#;

/// What a memory ceiling made real prikk 0.52.0 say (sweep over five repositories, see the review request):
/// the same shape, a different message.
const FAILED_OOM: &str = r#"{"stage":"objects","status":"failed","message":"i/o error: out of memory"},{"stage":"refs","status":"evaluated"}"#;

const BLOCKED: &str = r#"{"stage":"refs","status":"failed","message":"integrity error: ref pointer index has a damaged entry"},{"stage":"ref-update-schema-trust","status":"not_evaluated","blocked_by":"refs"}"#;
const HALTED: &str = r#"{"stage":"objects","status":"evaluated"},{"stage":"refs","status":"halted","after":"objects"}"#;

struct Case {
    name: &'static str,
    stdout: String,
    stderr: &'static str,
    exit: i32,
}

fn cases() -> Vec<Case> {
    let item = report(
        false,
        &[("item-failure", "found a failed object")],
        EVALUATED,
    );
    let stage = report(
        false,
        &[
            ("stage-failure", "did not complete every stage"),
            ("item-failure", "found a failed object"),
        ],
        FAILED_INTEGRITY,
    );
    let c = |name, stdout: String, stderr, exit| Case {
        name,
        stdout,
        stderr,
        exit,
    };
    vec![
        c("ok-exit0", report(true, &[], EVALUATED), "", 0),
        c(
            "fail-items",
            item.clone(),
            "error: repository verification found at least one failed object, block, or ref",
            1,
        ),
        c(
            "fail-integrity-stage",
            stage.clone(),
            "error: repository verification did not complete every stage; see stage outcomes above",
            1,
        ),
        c(
            "fail-oom-stage",
            report(
                false,
                &[("stage-failure", "did not complete every stage")],
                FAILED_OOM,
            ),
            "error: repository verification did not complete every stage; see stage outcomes above",
            1,
        ),
        c(
            "fail-blocked-stage",
            report(false, &[("stage-failure", "x")], BLOCKED),
            "error: repository verification did not complete every stage",
            1,
        ),
        c(
            "fail-halted-stage",
            report(false, &[("stage-failure", "x")], HALTED),
            "error: repository verification did not complete every stage",
            1,
        ),
        c("garbage", "not json".into(), "", 1),
        c("empty", String::new(), "error: something", 1),
        c(
            "other-schema",
            report(false, &[("x", "y")], EVALUATED).replace("verify-report-v1", "verify-report-v2"),
            "",
            1,
        ),
        c("exit1-says-ok", report(true, &[], EVALUATED), "", 1),
        c("exit2-with-report", item.clone(), "usage", 2),
        c("exit124-with-report", item.clone(), "", 124),
        c(
            "exit134-abort",
            String::new(),
            "memory allocation of 1048663 bytes failed",
            134,
        ),
        c(
            "exit134-with-report",
            item,
            "memory allocation of 8 bytes failed",
            134,
        ),
    ]
}

fn dir_of(base: &Path, name: &str) -> PathBuf {
    base.join(name)
}

#[test]
fn only_exit_one_with_a_complete_failing_report_is_a_verdict() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("verify-verdict-stand-in");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let script = base.join("prikk");
    std::fs::write(&script, SCRIPT).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    for c in cases() {
        let d = dir_of(&base, c.name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("stdout"), &c.stdout).unwrap();
        std::fs::write(d.join("stderr"), c.stderr).unwrap();
        std::fs::write(d.join("exit"), c.exit.to_string()).unwrap();
    }
    const CEILING: u64 = 512 << 20;
    // Both modes matter: without a ceiling the classifier is idle; with one it is armed, as in the server.
    let mut modes: Vec<(&str, Option<u64>)> = vec![("no ceiling", None)];
    if have(PRLIMIT) {
        modes.push(("ceiling set", Some(CEILING)));
    }

    for (label, ceiling) in modes {
        let repo_for = |name: &str| {
            let r = CliPrikkRepo::with_binary(dir_of(&base, name), &script)
                .with_sandbox(Sandbox::Unconfined);
            match ceiling {
                Some(c) => r.with_limits(CallLimits::data_ceiling(c)),
                None => r,
            }
        };
        let verdict = |name: &str| repo_for(name).verify();
        let expect_command = |name: &str, code: i32| match verdict(name) {
            Err(PrikkError::Command { code: Some(c), .. }) if c == code => {}
            other => panic!("{label}: {name}: expected Command({code}), got {other:?}"),
        };

        // exit 0 and ok: a verified repository.
        let r = verdict("ok-exit0").unwrap();
        assert!(r.verdict.ok, "{label}");

        // exit 1 + a complete failing report: the answer, with or without a ceiling armed. This includes
        // the trust-policy shape, whose stderr says "did not complete every stage" like a ceiling hit does.
        for name in ["fail-items", "fail-integrity-stage"] {
            let r = verdict(name).unwrap_or_else(|e| panic!("{label}: {name}: {e:?}"));
            assert!(
                !r.verdict.ok && !r.verdict.failed_conditions.is_empty(),
                "{label}: {name}"
            );
        }

        // A partial report is never a verdict: a ceiling hit when a ceiling is set, a plain failure otherwise.
        for name in ["fail-oom-stage", "fail-blocked-stage", "fail-halted-stage"] {
            match (ceiling, verdict(name)) {
                (
                    Some(c),
                    Err(PrikkError::CeilingExceeded {
                        verb,
                        ceiling_bytes,
                    }),
                ) => {
                    assert_eq!((verb.as_str(), ceiling_bytes), ("verify", c), "{name}");
                }
                (None, Err(PrikkError::Command { code: Some(1), .. })) => {}
                (_, other) => panic!("{label}: {name}: got {other:?}"),
            }
        }

        // Garbage, empty output, another schema, a contradictory "ok" on exit 1: errors, not verdicts.
        for name in ["garbage", "empty", "other-schema", "exit1-says-ok"] {
            assert!(
                matches!(
                    verdict(name),
                    Err(PrikkError::Command { code: Some(1), .. })
                ),
                "{label}: {name}: {:?}",
                verdict(name)
            );
        }

        // Only exit 1 is an answer: 2 and 124 carrying a perfectly good failing report are errors.
        expect_command("exit2-with-report", 2);
        expect_command("exit124-with-report", 124);

        // exit 134 (abort on allocation failure): a ceiling hit when a ceiling is set — even with a report
        // on stdout — and a plain failure otherwise.
        for name in ["exit134-abort", "exit134-with-report"] {
            match (ceiling, verdict(name)) {
                (Some(c), Err(PrikkError::CeilingExceeded { ceiling_bytes, .. })) => {
                    assert_eq!(ceiling_bytes, c, "{name}")
                }
                (
                    None,
                    Err(PrikkError::Command {
                        code: Some(134), ..
                    }),
                ) => {}
                (_, other) => panic!("{label}: {name}: got {other:?}"),
            }
        }
    }
}

/// The honesty invariant at the boundary, property-style over every small combination: whatever prikk
/// exits with and prints, `Ok(report)` with `ok == false` appears only for exit 1 and a complete report,
/// and `Ok(report)` with `ok == true` only for exit 0.
#[test]
fn no_combination_of_exit_status_and_output_yields_ok_unless_prikk_said_ok() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("verify-verdict-property");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let script = base.join("prikk");
    std::fs::write(&script, SCRIPT).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let stage_sets = [EVALUATED, FAILED_INTEGRITY, FAILED_OOM, BLOCKED, HALTED, ""];
    let mut n = 0;
    for exit in [0, 1, 2, 124, 134, 137] {
        for ok in [true, false] {
            for stages in stage_sets {
                let d = base.join(format!("c{n}"));
                n += 1;
                std::fs::create_dir_all(&d).unwrap();
                std::fs::write(d.join("stdout"), report(ok, &[("c", "m")], stages)).unwrap();
                std::fs::write(d.join("stderr"), "").unwrap();
                std::fs::write(d.join("exit"), exit.to_string()).unwrap();
                let repo = CliPrikkRepo::with_binary(&d, &script).with_sandbox(Sandbox::Unconfined);
                if let Ok(r) = repo.verify() {
                    let complete = !stages.contains("not_evaluated")
                        && !stages.contains("halted")
                        && !stages.contains("out of memory");
                    match (r.verdict.ok, exit) {
                        (true, 0) => {}
                        (false, 1) => assert!(
                            complete,
                            "exit 1 / {stages}: a partial report became a verdict"
                        ),
                        (false, 0) => {} // exit 0 is what it always was: parse and return
                        other => panic!("exit {exit}, ok {ok}, stages {stages}: Ok but {other:?}"),
                    }
                } else {
                    // An error is always acceptable; it must also never be the verified state.
                }
            }
        }
    }
    assert_eq!(n, 72);
}

// ---------------------------------------------------------------- real prikk

fn sealed(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("verify-verdict-{name}"));
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
            .unwrap();
        assert!(ok.success(), "prikk {args:?}");
    };
    prikk(&["setup"]);
    std::fs::write(repo.join("a.txt"), b"hello\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

fn corrupt(repo: &Path, rel: &str) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(repo.join(rel))
        .unwrap();
    f.seek(SeekFrom::Start(40)).unwrap();
    f.write_all(b"CORRUPT").unwrap();
}

#[test]
fn the_three_real_corruptions_are_unverified_verdicts_with_and_without_a_ceiling() {
    if !have("prikk") {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let intact = sealed("intact");
    assert!(
        CliPrikkRepo::new(&intact)
            .with_sandbox(Sandbox::Unconfined)
            .verify()
            .unwrap()
            .verdict
            .ok
    );
    for (name, rel, condition) in [
        ("policy", ".prikk/trust/policy-a.container", "stage-failure"),
        (
            "patch",
            ".prikk/containers/patch/a.container",
            "item-failure",
        ),
        (
            "block",
            ".prikk/containers/block/a.container",
            "item-failure",
        ),
    ] {
        let repo = sealed(name);
        corrupt(&repo, rel);
        let mut handles = vec![(
            "no ceiling",
            CliPrikkRepo::new(&repo).with_sandbox(Sandbox::Unconfined),
        )];
        if have(PRLIMIT) {
            handles.push((
                "512 MiB ceiling",
                CliPrikkRepo::new(&repo)
                    .with_sandbox(Sandbox::Unconfined)
                    .with_limits(CallLimits::data_ceiling(512 << 20)),
            ));
        }
        for (label, h) in handles {
            let r = h
                .verify()
                .unwrap_or_else(|e| panic!("{name} ({label}): {e:?}"));
            assert!(!r.verdict.ok, "{name} ({label})");
            assert!(
                r.verdict
                    .failed_conditions
                    .iter()
                    .any(|c| c.id == condition),
                "{name} ({label}): {:?}",
                r.verdict.failed_conditions
            );
        }
    }
}
