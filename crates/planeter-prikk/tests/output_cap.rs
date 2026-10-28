//! IS-55: what the server collects from a prikk call is bounded. The mechanism (a limit per pipe; the child killed at the limit; the
//! output dropped; the error named) is held here with stand-in programs (`/bin/sh`), so that every edge — exactly the limit, one
//! byte more, a flood on either pipe, a child that ignores what it is told, a grandchild that holds the pipe — can be made on
//! purpose. The table of what each verb gets, and the real prikk, are in `output_cap_table.rs`; the memory in `output_cap_memory.rs`.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use planeter_prikk::sandbox::{
    BwrapConfig, CallLimits, OVER_LIMIT_TAIL_BYTES, Run, run_command_capped,
};
use planeter_prikk::{OutputLimit, OutputStream, Sandbox};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn sh(script: &str, cap: OutputLimit) -> (Run, Duration) {
    let t = Instant::now();
    let run = Sandbox::Unconfined
        .run_capped(
            OsStr::new("/bin/sh"),
            &root(),
            &["-c", script],
            CallLimits::NONE,
            cap,
        )
        .expect("spawn");
    (run, t.elapsed())
}

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

#[test]
fn a_quiet_call_is_collected_whole() {
    let (run, _) = sh(
        "printf hello; printf oops >&2",
        OutputLimit::new(1024, 1024),
    );
    assert!(run.exceeded.is_none() && !run.timed_out);
    assert_eq!(
        (run.stdout.as_slice(), run.stderr.as_slice()),
        (&b"hello"[..], &b"oops"[..])
    );
    assert!(run.status.success());
}

#[test]
fn exactly_the_limit_is_collected_and_one_byte_more_is_not() {
    // Exactly at the limit: collected, every byte.
    let (run, _) = sh(
        "head -c 1000 /dev/zero | tr '\\0' x",
        OutputLimit::new(1000, 1024),
    );
    assert!(run.exceeded.is_none());
    assert_eq!(run.stdout.len(), 1000);
    // One byte more: killed, reported, and **nothing of the output is kept**.
    let (run, _) = sh(
        "head -c 1001 /dev/zero | tr '\\0' x; sleep 20",
        OutputLimit::new(1000, 1024),
    );
    let over = run.exceeded.expect("one byte over");
    assert_eq!((over.stream, over.limit), (OutputStream::Stdout, 1000));
    assert!(
        run.stdout.is_empty(),
        "a truncated report is never handed on"
    );
    assert!(!run.status.success());
}

#[test]
fn stderr_has_its_own_limit_exactly_and_one_more() {
    let (run, _) = sh(
        "head -c 500 /dev/zero | tr '\\0' e >&2",
        OutputLimit::new(1024, 500),
    );
    assert!(run.exceeded.is_none());
    assert_eq!(run.stderr.len(), 500);
    let (run, _) = sh(
        "head -c 501 /dev/zero | tr '\\0' e >&2; sleep 20",
        OutputLimit::new(1024, 500),
    );
    let over = run.exceeded.expect("one byte over on stderr");
    assert_eq!((over.stream, over.limit), (OutputStream::Stderr, 500));
    assert!(run.stdout.is_empty());
}

#[test]
fn a_flood_on_stdout_is_cut_the_child_is_killed_and_the_call_is_quick() {
    let (run, took) = sh("yes", OutputLimit::new(4 * MIB, 64 * KIB));
    assert_eq!(run.exceeded.map(|e| e.stream), Some(OutputStream::Stdout));
    assert!(run.stdout.is_empty());
    assert!(took < Duration::from_secs(10), "{took:?}");
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(run.status.signal(), Some(9), "killed, not left to finish");
}

#[test]
fn a_flood_on_stderr_is_cut_and_its_tail_is_kept_for_the_log() {
    let (run, took) = sh(
        "echo START >&2; yes zzzz >&2",
        OutputLimit::new(MIB, 64 * KIB),
    );
    let over = run.exceeded.expect("a stderr flood");
    assert_eq!(over.stream, OutputStream::Stderr);
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert!(run.stdout.is_empty());
    assert!(
        !run.stderr.is_empty() && run.stderr.len() <= OVER_LIMIT_TAIL_BYTES,
        "{}",
        run.stderr.len()
    );
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("zzzz"),
        "the tail is the end, not the beginning"
    );
    assert!(!String::from_utf8_lossy(&run.stderr).contains("START"));
}

#[test]
fn a_child_that_ignores_the_closing_of_the_pipe_and_the_term_signal_is_still_killed() {
    let (run, took) = sh(
        "trap '' PIPE TERM INT HUP; yes",
        OutputLimit::new(MIB, 64 * KIB),
    );
    assert!(run.exceeded.is_some());
    assert!(took < Duration::from_secs(10), "{took:?}");
}

#[test]
fn a_grandchild_that_keeps_the_pipes_open_cannot_hold_the_call() {
    // The shell is killed; a `sleep` it started keeps both pipes. The call must still return at once.
    let (run, took) = sh("(sleep 20 &) ; yes", OutputLimit::new(MIB, 64 * KIB));
    assert!(run.exceeded.is_some());
    assert!(took < Duration::from_secs(8), "{took:?}");
}

#[test]
fn the_wall_time_bound_still_fires_beside_the_output_bound() {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "sleep 30"]);
    let t = Instant::now();
    let run = run_command_capped(
        cmd,
        Some(Duration::from_secs(1)),
        OutputLimit::new(MIB, MIB),
    )
    .unwrap();
    assert!(run.timed_out && run.exceeded.is_none());
    assert!(t.elapsed() < Duration::from_secs(5));
    // A slow printer is cut by whichever bound comes first.
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "while true; do printf x; sleep 0.01; done"]);
    let run = run_command_capped(
        cmd,
        Some(Duration::from_secs(1)),
        OutputLimit::new(10 * MIB, MIB),
    )
    .unwrap();
    assert!(run.timed_out, "10 MiB would take hours at this rate");
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "while true; do printf xxxxxxxxxx; sleep 0.001; done"]);
    let run = run_command_capped(
        cmd,
        Some(Duration::from_secs(30)),
        OutputLimit::new(1000, MIB),
    )
    .unwrap();
    assert!(
        run.exceeded.is_some() && !run.timed_out,
        "the output bound came first"
    );
}

#[test]
fn a_call_in_the_real_sandbox_that_floods_is_killed_and_leaves_no_process() {
    let probe = Sandbox::default().run(OsStr::new("/usr/bin/true"), &root(), &[], CallLimits::NONE);
    if !matches!(probe, Ok(r) if r.status.success()) {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    let dir = root().join("output-cap-bwrap");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sb = Sandbox::Bwrap(BwrapConfig {
        timeout_secs: 30,
        ..BwrapConfig::default()
    });
    let t = Instant::now();
    let run = sb
        .run_capped(
            OsStr::new("/usr/bin/yes"),
            &dir,
            &[],
            CallLimits::NONE,
            OutputLimit::new(MIB, 64 * KIB),
        )
        .expect("spawn");
    assert!(run.exceeded.is_some() && !run.timed_out);
    assert!(t.elapsed() < Duration::from_secs(10));
    // Nothing of the call is left (bounded settle time).
    let me = std::process::id();
    let end = Instant::now() + Duration::from_secs(3);
    let left = |dir: &PathBuf| -> bool {
        std::fs::read_dir("/proc").unwrap().flatten().any(|e| {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
                return false;
            };
            if pid == me {
                return false;
            }
            let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            String::from_utf8_lossy(&cmd)
                .replace('\0', " ")
                .contains(&dir.to_string_lossy().into_owned())
        })
    };
    while left(&dir) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!left(&dir), "a process of the flooding call is still alive");
}
