//! RFC 012 D-4 for Japanese (handoff `japanese-and-switcher-handoff-v1` T2, T6): section 5 of `docs/glossary.md` and
//! the two catalogs say the same thing. Every row of 5.2 names a key; its *English* cell is the English catalog text
//! and its *Japanese* cell the Japanese catalog text, character for character. Section 5.1 lists the terms, flags
//! the hard ones for the reviewers, and every term it says is shown is really in the Japanese messages.

use std::path::PathBuf;

use planeter_web::catalog::{EN_DEF, JA_DEF};

fn glossary() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/glossary.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split(" | ")
        .map(|c| c.trim().to_owned())
        .collect()
}

/// The data rows of the table under `### <heading>` of section 5.
fn table(src: &str, heading: &str) -> Vec<Vec<String>> {
    let mut on = false;
    let mut seen_header = false;
    let mut out = Vec::new();
    for line in src.lines() {
        if line.starts_with("## ") && !line.starts_with("## 5.") {
            on = false;
        }
        if line.starts_with("### ") {
            on = line.starts_with(heading);
            seen_header = false;
            continue;
        }
        if !on || !line.starts_with('|') {
            continue;
        }
        if line.contains("---") {
            seen_header = true;
            continue;
        }
        if seen_header {
            out.push(cells(line));
        }
    }
    out
}

fn as_cell(one: &str, other: &str) -> String {
    if one == other {
        format!("\"{one}\"")
    } else {
        format!("\"{one}\" / \"{other}\"")
    }
}

fn entry(
    def: &'static planeter_web::catalog::LangDef,
    key: &str,
) -> Option<(&'static str, &'static str)> {
    def.entries
        .iter()
        .find(|e| e.key == key)
        .map(|e| (e.one, e.other))
}

#[test]
fn every_message_row_of_the_japanese_section_is_the_catalog_text_in_both_languages() {
    let rows = table(&glossary(), "### 5.2");
    assert!(
        rows.len() >= 40,
        "the 5.2 table was not read: {} rows",
        rows.len()
    );
    let mut problems = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for r in &rows {
        assert_eq!(
            r.len(),
            3,
            "a 5.2 row has message, English, Japanese: {r:?}"
        );
        let key = r[0].trim_matches('`');
        if !seen.insert(key.to_owned()) {
            problems.push(format!("`{key}` is listed twice"));
        }
        match (entry(&EN_DEF, key), entry(&JA_DEF, key)) {
            (Some((eo, et)), Some((jo, jt))) => {
                if r[1] != as_cell(eo, et) {
                    problems.push(format!(
                        "`{key}`: the English cell is {} but the catalog says {}",
                        r[1],
                        as_cell(eo, et)
                    ));
                }
                if r[2] != as_cell(jo, jt) {
                    problems.push(format!(
                        "`{key}`: the Japanese cell is {} but the catalog says {}",
                        r[2],
                        as_cell(jo, jt)
                    ));
                }
            }
            _ => problems.push(format!("`{key}` is not a key of both catalogs")),
        }
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn the_five_trust_labels_and_their_explanations_are_all_in_the_japanese_section() {
    let rows = table(&glossary(), "### 5.2");
    let named: Vec<&str> = rows.iter().map(|r| r[0].trim_matches('`')).collect();
    for key in [
        "assurance.verified",
        "assurance.sealed",
        "assurance.queued",
        "assurance.unverified",
        "assurance.verify_unavailable",
        "explain.verified",
        "explain.signed",
        "explain.queued",
        "explain.failed",
        "explain.failed_here",
        "explain.unavailable",
        "explain.patch",
        "explain.rollback",
        "login.sso",
        "nav.code",
        "nav.history",
        "nav.refs",
        "nav.verify",
    ] {
        assert!(
            named.contains(&key),
            "the Japanese glossary does not list `{key}`"
        );
    }
}

#[test]
fn the_hard_terms_are_listed_flagged_and_the_shown_terms_are_really_used() {
    let rows = table(&glossary(), "### 5.1");
    assert!(
        rows.len() >= 15,
        "the 5.1 table was not read: {} rows",
        rows.len()
    );
    for r in &rows {
        assert_eq!(
            r.len(),
            5,
            "a 5.1 row has term, Japanese, shown, why, status: {r:?}"
        );
        assert!(["yes", "no"].contains(&r[2].as_str()), "{r:?}");
        assert!(!r[3].is_empty());
        assert!(
            r[4] == "Proposed" || r[4].starts_with("Hard term"),
            "unknown status {:?}",
            r[4]
        );
    }
    // The terms the handoff names for the reviewers' decision are all there, flagged.
    for term in [
        "seal / sealed / unsealed",
        "verify / verification",
        "branch",
        "patch",
        "change",
        "rollback",
        "Signed into the history (badge)",
        "single sign-on",
        "repository",
    ] {
        let row = rows
            .iter()
            .find(|r| r[0] == term)
            .unwrap_or_else(|| panic!("the term {term:?} is not in 5.1"));
        assert!(
            row[4].starts_with("Hard term"),
            "{term:?} must be flagged: {:?}",
            row[4]
        );
    }
    // A term that says it is shown appears in some Japanese message (the first form, before any ` / `).
    let all_japanese: String = JA_DEF
        .entries
        .iter()
        .flat_map(|e| [e.one, e.other])
        .collect::<Vec<_>>()
        .join("\n");
    for r in &rows {
        let shown = r[2] == "yes";
        for form in r[1].split(" / ") {
            if shown {
                assert!(
                    all_japanese.contains(form),
                    "{:?}: the Japanese term {form:?} is said to be shown but no message has it",
                    r[0]
                );
            }
        }
    }
    // A term that is not shown is not used by any message (the interface does not say it).
    let sealed = rows
        .iter()
        .find(|r| r[0] == "seal / sealed / unsealed")
        .unwrap();
    assert_eq!(sealed[2], "no");
    for word in ["封印", "シール"] {
        assert!(
            !all_japanese.contains(word),
            "{word:?} is in a Japanese message"
        );
    }
}
