//! The shell tools of `tools/` have tests of their own in `tools/tests/` (no build, no cap, fake steps in a throw-away repository): this runs
//! them with the workspace's tests, so that a change to `tools/run-capped.sh` or `tools/gates.sh` is judged by the same `cargo test` as the rest
//! (RFC 010 Amendment 1). They need `sh`, `git` and the usual coreutils; a machine without `git` skips, saying so.

use std::path::Path;
use std::process::Command;

fn run(script: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping {script}: needs git");
        return;
    }
    let out = Command::new("sh")
        .arg(root.join("tools/tests").join(script))
        .current_dir(&root)
        .output()
        .expect("run sh");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "tools/tests/{script} failed:\n{}",
        text.lines()
            .filter(|l| !l.starts_with("ok "))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn run_capped_derives_its_limits_from_the_machine_and_says_what_it_did() {
    run("run-capped.sh");
}

#[test]
fn gates_clean_up_on_every_way_out_and_refuse_what_they_must() {
    run("gates.sh");
}
