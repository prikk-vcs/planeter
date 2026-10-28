//! IS-55: a child that prints a gigabyte does not grow the server. This file holds **one test** so that the process's high-water
//! mark (`VmHWM` of `/proc/self/status`) is the test's alone: the resident set is read before the call and after it, and the growth
//! must be bounded by the limit (a buffer that doubles up to the limit, plus one chunk), not by what the child printed.

use std::ffi::OsStr;
use std::path::PathBuf;

use planeter_prikk::sandbox::CallLimits;
use planeter_prikk::{OutputLimit, Sandbox};

const MIB: u64 = 1024 * 1024;

fn hwm_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024
}

fn run(script: &str, cap: OutputLimit) -> planeter_prikk::sandbox::Run {
    Sandbox::Unconfined
        .run_capped(
            OsStr::new("/bin/sh"),
            &PathBuf::from(env!("CARGO_TARGET_TMPDIR")),
            &["-c", script],
            CallLimits::NONE,
            cap,
        )
        .expect("spawn")
}

#[test]
fn a_gigabyte_of_output_does_not_grow_the_process_and_a_call_at_the_limit_grows_it_by_the_limit() {
    // Warm up: threads, the first allocations.
    let _ = run("printf x", OutputLimit::new(MIB, MIB));
    let limit = 16 * MIB;
    let start = hwm_bytes();

    // 1. A child that would print 1 GiB on one pipe: cut at the limit, killed. (Each pipe in turn; the other has the same room.)
    for (name, script) in [
        ("stdout", "head -c 1073741824 /dev/zero"),
        ("stderr", "head -c 1073741824 /dev/zero >&2"),
    ] {
        let flood = run(script, OutputLimit::new(limit, limit));
        assert!(
            flood.exceeded.is_some() && flood.stdout.is_empty(),
            "{name}"
        );
        let growth = hwm_bytes().saturating_sub(start);
        eprintln!(
            "1 GiB on {name}, limit {} MiB: high-water growth {} MiB",
            limit / MIB,
            growth / MIB
        );
        assert!(
            growth < 2 * limit + 8 * MIB,
            "{name}: the flood grew the process by {} MiB (limit {} MiB)",
            growth / MIB,
            limit / MIB
        );
    }

    // 2. A call that prints exactly the limit is collected whole, and costs about the limit.
    let whole = run(
        &format!("head -c {limit} /dev/zero"),
        OutputLimit::new(limit, MIB),
    );
    assert!(whole.exceeded.is_none());
    assert_eq!(whole.stdout.len() as u64, limit);
    let growth = hwm_bytes().saturating_sub(start);
    eprintln!(
        "output of exactly the limit ({} MiB): high-water growth {} MiB (since the start of the test)",
        limit / MIB,
        growth / MIB
    );
    assert!(growth < 2 * limit + 8 * MIB, "grew by {} MiB", growth / MIB);
}
