//! T0: the binary exists, runs, and names itself. (`--version` grows its build commit and the prikk floor in T4.)

use std::process::Command;

#[test]
fn the_binary_runs_and_says_what_it_is() {
    let out = Command::new(env!("CARGO_BIN_EXE_planeter-client"))
        .output()
        .expect("the binary runs");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with("planeter-client "),
        "unexpected output: {text:?}"
    );
}
