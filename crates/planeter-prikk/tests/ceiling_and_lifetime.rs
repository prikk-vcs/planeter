//! RFC 011 D-2 and D-5 at the prikk boundary: the memory ceiling and the child's lifetime.
//!
//! Pure tests (the argument vector, the probe) never skip. Tests that need `bwrap`, `prlimit` or `prikk`
//! **self-skip with a printed reason** when the tool or the user namespace is unavailable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use planeter_prikk::sandbox::{BwrapConfig, CallLimits, PRLIMIT, TIMEOUT_EXIT_CODE, probe_prlimit};
use planeter_prikk::{CliPrikkRepo, PrikkError, PrikkRepo, Sandbox};

fn args_of(c: &Command) -> Vec<String> {
    c.get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Whether the production sandbox can run here (bwrap present and user namespaces allowed).
fn sandbox_works() -> bool {
    // Not `scratch(..)`: tests run in parallel and `scratch` deletes and recreates its directory.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    matches!(
        Sandbox::default().run(OsStr::new("/usr/bin/true"), &root, &[], CallLimits::NONE),
        Ok(r) if r.status.success()
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Processes that belong to a call run against `root`: the sandbox's cmdline names it, or the process
/// sits in it (the sandboxed program's working directory).
fn procs_of(root: &Path) -> Vec<u32> {
    let root_s = root.to_string_lossy().into_owned();
    let me = std::process::id();
    let mut found = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == me {
            continue;
        }
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let cmd = String::from_utf8_lossy(&cmd).replace('\0', " ");
        let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap_or_default();
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let ours = matches!(
            comm.trim(),
            "bwrap" | "sleep" | "prlimit" | "timeout" | "prikk"
        );
        if ours && (cmd.contains(&root_s) || cwd == root) {
            found.push(pid);
        }
    }
    found
}

// ---------------------------------------------------------------- the vector (never skips)

#[test]
fn bwrap_is_the_program_and_timeout_is_gone() {
    let c = Sandbox::default().command(OsStr::new("prikk"), Path::new("/repo"), &["status"]);
    assert_eq!(
        c.get_program(),
        OsStr::new("bwrap"),
        "no `timeout` hop in front of bwrap"
    );
    let args = args_of(&c);
    assert!(
        !args
            .iter()
            .any(|a| a == "timeout" || a.starts_with("--kill-after")),
        "{args:?}"
    );
    assert!(
        args.contains(&"--die-with-parent".to_owned()),
        "the tie to the server"
    );
    assert!(args.contains(&"--unshare-all".to_owned()));
}

#[test]
fn the_prlimit_prefix_is_present_only_when_a_ceiling_is_set() {
    let sb = Sandbox::default();
    let none = sb.command_with(
        OsStr::new("prikk"),
        Path::new("/repo"),
        &["status"],
        CallLimits::NONE,
    );
    assert_eq!(none.get_program(), OsStr::new("bwrap"));
    assert!(!args_of(&none).iter().any(|a| a.starts_with("--data=")));

    let capped = sb.command_with(
        OsStr::new("prikk"),
        Path::new("/repo"),
        &["status"],
        CallLimits::data_ceiling(1_073_741_824),
    );
    assert_eq!(capped.get_program(), OsStr::new(PRLIMIT));
    let args = args_of(&capped);
    assert_eq!(&args[..3], ["--data=1073741824", "--", "bwrap"], "{args:?}");
    // …and the rest of the vector is exactly the unlimited one.
    assert_eq!(args[3..], args_of(&none)[..]);
}

#[test]
fn the_prefix_also_wraps_the_unconfined_dev_mode_and_keeps_its_directory() {
    let c = Sandbox::Unconfined.command_with(
        OsStr::new("prikk"),
        Path::new("/repo"),
        &["verify"],
        CallLimits::data_ceiling(7),
    );
    assert_eq!(c.get_program(), OsStr::new(PRLIMIT));
    assert_eq!(args_of(&c), ["--data=7", "--", "prikk", "verify"]);
    assert_eq!(c.get_current_dir(), Some(Path::new("/repo")));
}

#[test]
fn the_ceiling_prefix_uses_the_configured_prlimit_binary() {
    let cfg = BwrapConfig {
        prlimit: "/opt/util-linux/prlimit".into(),
        ..BwrapConfig::default()
    };
    let c = Sandbox::Bwrap(cfg).command_with(
        OsStr::new("prikk"),
        Path::new("/repo"),
        &["log"],
        CallLimits::data_ceiling(5),
    );
    assert_eq!(c.get_program(), OsStr::new("/opt/util-linux/prlimit"));
}

#[test]
fn the_wall_time_bound_is_the_configured_setting_and_unconfined_has_none() {
    let cfg = BwrapConfig {
        timeout_secs: 7,
        ..BwrapConfig::default()
    };
    assert_eq!(Sandbox::Bwrap(cfg).deadline(), Some(Duration::from_secs(7)));
    assert_eq!(
        Sandbox::default().deadline(),
        Some(Duration::from_secs(120))
    );
    assert_eq!(Sandbox::Unconfined.deadline(), None);
}

// ---------------------------------------------------------------- the probe (never skips)

#[test]
fn the_probe_names_a_missing_binary_and_accepts_a_working_one() {
    let err = probe_prlimit(OsStr::new("/nonexistent/prlimit")).unwrap_err();
    assert!(err.contains("No such file or directory"), "{err}");
    if have(PRLIMIT) {
        probe_prlimit(OsStr::new(PRLIMIT)).expect("prlimit is on PATH");
    } else {
        eprintln!("note: prlimit is not installed here; the positive probe was not run");
    }
    // A program that runs but fails `--version` is also refused.
    let err = probe_prlimit(OsStr::new("/usr/bin/false")).unwrap_err();
    assert!(err.contains("--version"), "{err}");
}

// ---------------------------------------------------------------- the lifetime (needs bwrap)

#[test]
fn a_call_past_the_deadline_is_killed_by_the_server_and_leaves_no_process() {
    if !sandbox_works() {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    let root = scratch("deadline-root");
    let sb = Sandbox::Bwrap(BwrapConfig {
        timeout_secs: 1,
        ..BwrapConfig::default()
    });
    let t = Instant::now();
    let run = sb
        .run(
            OsStr::new("/usr/bin/sleep"),
            &root,
            &["30"],
            CallLimits::NONE,
        )
        .expect("spawn");
    let took = t.elapsed();
    assert!(run.timed_out, "the bound must fire");
    assert!(
        took >= Duration::from_secs(1) && took < Duration::from_secs(4),
        "{took:?}"
    );
    // No process of the call remains (bounded settle time).
    let end = Instant::now() + Duration::from_secs(3);
    while !procs_of(&root).is_empty() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        procs_of(&root),
        Vec::<u32>::new(),
        "a process of the killed call survived"
    );
}

#[test]
fn a_normal_call_completes_and_is_not_marked_timed_out() {
    if !sandbox_works() {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    let root = scratch("normal-root");
    let run = Sandbox::default()
        .run(
            OsStr::new("/usr/bin/sleep"),
            &root,
            &["1"],
            CallLimits::NONE,
        )
        .expect("spawn");
    assert!(run.status.success() && !run.timed_out);
    assert!(procs_of(&root).is_empty());
}

#[test]
fn the_driver_reports_a_deadline_kill_as_the_status_timeout_used_to() {
    if !sandbox_works() {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    // A stand-in "prikk" that answers `--version` and then hangs, driven through the real vector.
    let dir = scratch("standin-bin");
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
    let root = scratch("standin-root");
    let repo =
        CliPrikkRepo::with_binary(&root, &script).with_sandbox(Sandbox::Bwrap(BwrapConfig {
            timeout_secs: 1,
            ..BwrapConfig::default()
        }));
    match repo.status() {
        Err(PrikkError::Command { code, stderr }) => {
            assert_eq!(code, Some(TIMEOUT_EXIT_CODE));
            assert!(stderr.is_empty());
        }
        other => panic!("expected the timeout status, got {other:?}"),
    }
}

// ---------------------------------------------------------------- the ceiling hit (needs prikk, prlimit)

fn prikk_available() -> bool {
    have("prikk")
}

/// A sealed repository holding one 16 MiB binary file, so that an 8 MiB data ceiling is below what
/// `tree`, `cat` and `verify` need.
fn big_repo(name: &str) -> PathBuf {
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
            .expect("spawn prikk");
        assert!(ok.success(), "prikk {args:?} failed");
    };
    prikk(&["setup"]);
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let bytes: Vec<u8> = (0..16 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    std::fs::write(repo.join("big.bin"), bytes).unwrap();
    std::fs::write(repo.join("README.md"), b"hello\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    repo
}

const LOW: u64 = 8 << 20;

fn capped(root: &Path) -> CliPrikkRepo {
    CliPrikkRepo::new(root)
        .with_sandbox(Sandbox::Unconfined)
        .with_limits(CallLimits::data_ceiling(LOW))
}

#[test]
fn a_ceiling_hit_is_a_typed_error_never_a_verdict_for_verify_tree_and_cat() {
    if !prikk_available() || !have(PRLIMIT) {
        eprintln!("skipping: needs prikk and prlimit on PATH");
        return;
    }
    let root = big_repo("ceiling-hit");

    // Without a ceiling all three succeed on this repository.
    let free = CliPrikkRepo::new(&root).with_sandbox(Sandbox::Unconfined);
    assert!(free.verify().is_ok());
    assert!(free.tree(None, None).is_ok());
    assert!(free.cat_bytes(None, "big.bin", Some(32 << 20)).is_ok());

    // With a ceiling below the need, each is `CeilingExceeded` naming the verb and the ceiling.
    let low = capped(&root);
    for (verb, result) in [
        ("verify", low.verify().err()),
        ("tree", low.tree(None, None).err()),
        ("cat", low.cat_bytes(None, "big.bin", Some(32 << 20)).err()),
    ] {
        match result {
            Some(PrikkError::CeilingExceeded {
                verb: v,
                ceiling_bytes,
            }) => {
                assert_eq!(v, verb);
                assert_eq!(ceiling_bytes, LOW);
            }
            other => panic!("{verb}: expected CeilingExceeded, got {other:?}"),
        }
    }

    // The message carries no path and no tool output.
    let msg = low.verify().unwrap_err().to_string();
    assert!(
        msg.contains("verify") && msg.contains(&LOW.to_string()),
        "{msg}"
    );
    assert!(!msg.contains(root.to_string_lossy().as_ref()), "{msg}");
}

#[test]
fn a_genuine_verification_failure_is_not_mistaken_for_a_ceiling_hit() {
    if !prikk_available() || !have(PRLIMIT) {
        eprintln!("skipping: needs prikk and prlimit on PATH");
        return;
    }
    let root = big_repo("genuine-failure");
    // Damage one byte of the blob container: prikk reports a failed object (exit 1), all stages evaluated.
    let container = root.join(".prikk/containers/blob/a.container");
    let mut data = std::fs::read(&container).unwrap();
    let mid = data.len() / 2;
    data[mid] ^= 0xFF;
    std::fs::write(&container, data).unwrap();

    // A generous ceiling is set, so the classifier is armed; the damage must still be a plain failure.
    let repo = CliPrikkRepo::new(&root)
        .with_sandbox(Sandbox::Unconfined)
        .with_limits(CallLimits::data_ceiling(512 << 20));
    match repo.verify() {
        Err(PrikkError::Command {
            code: Some(1),
            stderr,
        }) => {
            assert!(stderr.contains("failed object"), "{stderr}");
        }
        other => panic!("expected a plain exit-1 failure, got {other:?}"),
    }
}
