//! `--version` prints the name, the version, the build commit and the prikk floor, and needs no network (the RFC 010 D-6 pattern);
//! `--help` and an unknown command exit as documented.

use std::process::Command;

fn client() -> Command {
    Command::new(env!("CARGO_BIN_EXE_planeter-client"))
}

#[test]
fn version_names_the_program_the_build_and_the_prikk_floor() {
    let out = client().arg("--version").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with(&format!(
            "planeter-client {} (build ",
            env!("CARGO_PKG_VERSION")
        )),
        "{text}"
    );
    assert!(text.contains("prikk 0.54.0 or later"), "{text}");
}

#[test]
fn help_lists_the_commands_and_an_unknown_command_is_exit_2() {
    let out = client().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for word in [
        "propose",
        "clone",
        "fetch",
        "--token-file",
        "--cacert",
        "Exit codes",
        "Example",
    ] {
        assert!(text.contains(word), "{word}: {text}");
    }
    let bad = client().arg("frobnicate").output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("frobnicate"));
}

#[test]
fn a_plain_http_address_is_refused_before_anything_is_sent_and_a_missing_token_is_a_sentence() {
    let out = client()
        .args(["clone", "http://127.0.0.1:1/a/b"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("only talks to servers over https"), "{err}");
    let dir = std::env::temp_dir();
    let out = client()
        .args(["propose", "https://127.0.0.1:1/a/b", "--title", "T"])
        .current_dir(&dir)
        .env_remove("PLANETER_TOKEN")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
