//! The program: it writes the sheet, checks one, and says by its exit code (0 clean, 1 drift, 2 unusable) whether the sheet is what the
//! catalogs say.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_planeter-ja-sheet"))
        .args(args)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn a_written_sheet_checks_clean_an_edited_one_does_not_and_bad_input_is_refused() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("ja-sheet-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("sheet.md");
    let path = file.to_str().unwrap();
    let (code, out, err) = run(&["--out", path, "--commit", "abc123", "--date", "2026-10-31"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.starts_with("wrote ") && out.contains("messages"),
        "{out}"
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("commit `abc123`") && text.contains("on 2026-10-31"));
    let (code, out, _) = run(&["--check", path]);
    assert_eq!(code, 0, "{out}");
    // Edit one Japanese cell by hand: the check names the key and exits 1.
    let edited = text.replacen("| Verified | 検証済み |", "| Verified | 検証ずみ |", 1);
    assert_ne!(edited, text);
    std::fs::write(&file, edited).unwrap();
    let (code, out, err) = run(&["--check", path]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        out.contains("`assurance.verified`") && out.contains("Japanese"),
        "{out}"
    );
    // A file that is not there, an unknown option, a missing value.
    assert_eq!(
        run(&["--check", dir.join("nope.md").to_str().unwrap()]).0,
        2
    );
    assert_eq!(run(&["--frobnicate"]).0, 2);
    assert_eq!(run(&["--out"]).0, 2);
    assert_eq!(
        run(&["--glossary", dir.join("no-glossary.md").to_str().unwrap()]).0,
        2
    );
    let _ = std::fs::remove_dir_all(&dir);
}
