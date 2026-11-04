//! The program: it writes the sheet of a language, checks one, writes a baseline, prints the release-notes status, and says by its exit
//! code (0 clean, 1 drift, 2 unusable) whether the sheet is what the catalogs say.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_planeter-i18n-sheet"))
        .args(args)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// The first language of the registry besides the source.
fn tag() -> &'static str {
    planeter_i18n::languages::LANGUAGES
        .iter()
        .find(|l| !l.is_source())
        .unwrap()
        .tag
}

#[test]
fn a_written_sheet_checks_clean_an_edited_one_does_not_and_bad_input_is_refused() {
    let tag = tag();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("i18n-sheet-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("sheet.md");
    let path = file.to_str().unwrap();
    let (code, out, err) = run(&[
        "--lang",
        tag,
        "--out",
        path,
        "--commit",
        "abc123",
        "--date",
        "2026-11-04",
    ]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.starts_with("wrote ") && out.contains("messages"),
        "{out}"
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("commit `abc123`")
            && text.contains("on 2026-11-04")
            && text.contains(&format!("**Language `{tag}`"))
    );
    // The sheet says its language, so --check needs no --lang; the counts since the sign-off are information.
    let (code, out, _) = run(&["--check", path]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("since the sign-off:"), "{out}");
    // With no --lang the one language besides the source is the default.
    let (code, _, _) = run(&["--out", path, "--commit", "abc123", "--date", "2026-11-04"]);
    assert_eq!(code, 0);
    // Edit one target cell by hand: the check names the key and exits 1.
    let row = text
        .lines()
        .find(|l| l.starts_with("| `assurance.verified` |"))
        .unwrap()
        .to_owned();
    let target = row.split(" | ").nth(3).unwrap().to_owned();
    let edited = text.replacen(
        &row,
        &row.replacen(&format!(" | {target} | "), " | planted by hand | ", 1),
        1,
    );
    assert_ne!(edited, text);
    std::fs::write(&file, edited).unwrap();
    let (code, out, err) = run(&["--check", path]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        out.contains("`assurance.verified`") && out.contains("target-language cell"),
        "{out}"
    );
    // A file that is not there, an unknown option, a missing value, a glossary that is not there, an unknown language.
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
    assert_eq!(run(&["--lang", "zz"]).0, 2);
    assert_eq!(
        run(&["--lang", "en"]).0,
        2,
        "the source language has no sheet"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_baseline_of_a_catalog_as_it_stands_is_all_unchanged_and_the_status_line_names_the_counts() {
    let tag = tag();
    for program in ["web", "client"] {
        let (code, out, err) = run(&["--baseline", tag, program]);
        assert_eq!(code, 0, "{err}");
        let map = planeter_i18n::baseline::parse(&out).unwrap();
        assert!(map.len() > 100, "{program}: {}", map.len());
    }
    assert_eq!(run(&["--baseline", tag, "mobile"]).0, 2);
    assert_eq!(run(&["--baseline", "zz", "web"]).0, 2);
    let (code, out, _) = run(&["--status"]);
    assert_eq!(code, 0);
    assert!(
        out.lines().any(|l| l.starts_with(tag)
            && l.contains("signed off by")
            && l.contains("newer than the sign-off")),
        "{out}"
    );
}
