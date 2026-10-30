//! A stress run of the `flows` suite (part 11, T3): it is ignored by default and run by hand, because it takes minutes and burns CPU.
//!
//! ```text
//! cargo test -p planeter-client --test stress -- --ignored --nocapture
//! PLANETER_STRESS_RUNS=50 PLANETER_STRESS_CPUS=0,1 cargo test -p planeter-client --test stress -- --ignored --nocapture
//! ```
//!
//! It runs `cargo test -p planeter-client --test flows` `PLANETER_STRESS_RUNS` times (default 10), one after the other (the suites keep
//! their scratch directories by name), and fails on the first run that fails, printing the failing tests and their texts. With
//! `PLANETER_STRESS_CPUS` (a CPU list such as `0,1`; Linux, needs `taskset`) every run is confined to those CPUs and one busy loop per
//! CPU is started on them; the busy loops are children of this test, stopped through their handles (never by name), also when the
//! test panics.

use std::process::{Child, Command, Stdio};

/// Busy loops pinned to the CPUs; killed and waited for when dropped.
struct Burners(Vec<Child>);

impl Drop for Burners {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn burn(cpus: &str) -> Burners {
    let mut children = Vec::new();
    for cpu in cpus.split(',').filter(|c| !c.trim().is_empty()) {
        let child = Command::new("taskset")
            .args(["-c", cpu.trim(), "sh", "-c", "while :; do :; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("taskset (util-linux) is needed for PLANETER_STRESS_CPUS");
        children.push(child);
    }
    Burners(children)
}

#[test]
#[ignore = "a stress run: minutes of CPU; run by hand (see the module documentation)"]
fn the_flows_suite_passes_every_time_under_load() {
    let runs: u32 = std::env::var("PLANETER_STRESS_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let cpus = std::env::var("PLANETER_STRESS_CPUS").ok();
    let _burners = cpus.as_deref().map(burn);
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut failed = Vec::new();
    for n in 1..=runs {
        let mut cmd = match &cpus {
            Some(list) => {
                let mut c = Command::new("taskset");
                c.args(["-c", list]).arg(&cargo);
                c
            }
            None => Command::new(&cargo),
        };
        let out = cmd
            .args([
                "test",
                "--locked",
                "-p",
                "planeter-client",
                "--test",
                "flows",
            ])
            .output()
            .expect("cargo test");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() {
            println!("run {n}/{runs}: ok");
        } else {
            println!("run {n}/{runs}: FAILED");
            for line in text.lines().filter(|l| {
                l.contains("FAILED")
                    || l.contains("panicked")
                    || l.contains("left:")
                    || l.contains("right:")
            }) {
                println!("    {line}");
            }
            failed.push(n);
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {runs} runs failed: runs {failed:?}",
        failed.len()
    );
}
