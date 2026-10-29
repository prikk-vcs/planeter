//! The program's exit codes: 0 pass, 1 the registry's answer says no, 2 unreadable input — and nothing printed as success on 1 or 2.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_planeter-image-identity"))
        .args(args)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn same_passes_fails_and_refuses_with_three_different_exit_codes() {
    let (code, out, _) = run(&["same", A, A]);
    assert_eq!(
        (code, out.trim()),
        (0, "the pushed image is the image that was run")
    );
    let (code, out, err) = run(&["same", A, B]);
    assert_eq!(code, 1);
    assert!(
        out.is_empty() && err.contains(A) && err.contains(B),
        "{out}{err}"
    );
    let (code, out, _) = run(&["same", A, "garbage"]);
    assert_eq!((code, out.as_str()), (2, ""));
}

#[test]
fn files_are_read_and_a_missing_file_or_a_wrong_command_is_a_usage_failure() {
    let dir = std::env::temp_dir().join(format!("identity-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let index = dir.join("index.json");
    std::fs::write(
        &index,
        format!(r#"{{"manifests":[{{"digest":"{B}","platform":{{"os":"linux","architecture":"arm64"}}}}]}}"#),
    )
    .unwrap();
    let path = index.to_str().unwrap();
    assert_eq!(run(&["platform", path, "arm64"]).0, 0);
    assert_eq!(run(&["platform", path, "arm64"]).1.trim(), B);
    assert_eq!(run(&["platform", path, "amd64"]).0, 1);
    assert_eq!(run(&["platform", "/nonexistent/index.json", "amd64"]).0, 2);
    assert_eq!(run(&["config", path]).0, 2, "an index is not a manifest");
    assert_eq!(run(&[]).0, 2);
    assert_eq!(run(&["frobnicate", "x"]).0, 2);
    let _ = std::fs::remove_dir_all(&dir);
}
