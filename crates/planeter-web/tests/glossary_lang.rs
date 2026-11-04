//! RFC 012 D-4 for every language of the registry other than the source (handoff `language-registry-handoff-v1` T9): the language's
//! glossary file `docs/glossary/<tag>.md` and the catalog say the same thing. Every row of its "Messages that carry a term or a trust
//! label" table names a key; its *English* cell is the English catalog text and its *language* cell that language's catalog text,
//! character for character. Its "Terms" table lists the terms, flags the hard ones for the reviewers, and every term it says is shown
//! is really in the language's messages. A language without a glossary file fails the build with a sentence that says to add one.

use std::path::PathBuf;

use planeter_web::catalog::{CATALOGS, EN_DEF, LangDef};

fn glossary_path(tag: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/glossary")
        .join(format!("{tag}.md"))
}

fn glossary(tag: &str) -> String {
    let path = glossary_path(tag);
    std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "the language `{tag}` has no glossary file: add {} (Status, Terms, Messages that carry a term or a trust label) — docs/messages-and-languages.md, \"Adding a language\"",
            path.display()
        )
    })
}

/// The languages with a glossary file to check: every registry language but the source.
fn languages() -> Vec<&'static LangDef> {
    CATALOGS
        .iter()
        .copied()
        .filter(|d| !d.is_source())
        .collect()
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split(" | ")
        .map(|c| c.trim().to_owned())
        .collect()
}

/// The data rows of the table under `## <heading>`.
fn table(src: &str, heading: &str) -> Vec<Vec<String>> {
    let mut on = false;
    let mut seen_header = false;
    let mut out = Vec::new();
    for line in src.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            on = line.trim_start_matches('#').trim() == heading;
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

const MESSAGES: &str = "Messages that carry a term or a trust label";
const TERMS: &str = "Terms";

fn as_cell(one: &str, other: &str) -> String {
    if one == other {
        format!("\"{one}\"")
    } else {
        format!("\"{one}\" / \"{other}\"")
    }
}

fn entry(def: &'static LangDef, key: &str) -> Option<(&'static str, &'static str)> {
    def.entries
        .iter()
        .find(|e| e.key == key)
        .map(|e| (e.one(), e.other()))
}

#[test]
fn every_language_but_the_source_has_a_glossary_file() {
    assert!(!languages().is_empty());
    for d in languages() {
        let g = glossary(d.tag);
        for part in [
            "## Status",
            "## Terms",
            "## Messages that carry a term or a trust label",
        ] {
            assert!(
                g.lines().any(|l| l.trim_end() == part),
                "{}: the glossary file has no `{part}` part",
                d.tag
            );
        }
    }
}

#[test]
fn every_message_row_of_a_glossary_file_is_the_catalog_text_in_both_languages() {
    for def in languages() {
        let rows = table(&glossary(def.tag), MESSAGES);
        assert!(
            rows.len() >= 40,
            "{}: the messages table was not read: {} rows",
            def.tag,
            rows.len()
        );
        let mut problems = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for r in &rows {
            assert_eq!(
                r.len(),
                3,
                "a messages row has message, English, {}: {r:?}",
                def.tag
            );
            let key = r[0].trim_matches('`');
            if !seen.insert(key.to_owned()) {
                problems.push(format!("`{key}` is listed twice"));
            }
            match (entry(&EN_DEF, key), entry(def, key)) {
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
                            "`{key}`: the {} cell is {} but the catalog says {}",
                            def.tag,
                            r[2],
                            as_cell(jo, jt)
                        ));
                    }
                }
                _ => problems.push(format!("`{key}` is not a key of both catalogs")),
            }
        }
        assert_eq!(problems, Vec::<String>::new(), "{}", def.tag);
    }
}

#[test]
fn the_five_trust_labels_and_their_explanations_are_all_in_every_glossary_file() {
    for def in languages() {
        let rows = table(&glossary(def.tag), MESSAGES);
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
                "{}: the glossary file does not list `{key}`",
                def.tag
            );
        }
    }
}

#[test]
fn the_hard_terms_are_listed_flagged_and_the_shown_terms_are_really_used() {
    for def in languages() {
        let rows = table(&glossary(def.tag), TERMS);
        assert!(
            rows.len() >= 15,
            "{}: the terms table was not read: {} rows",
            def.tag,
            rows.len()
        );
        for r in &rows {
            assert_eq!(
                r.len(),
                5,
                "a terms row has term, {}, shown, why, status: {r:?}",
                def.tag
            );
            assert!(["yes", "no"].contains(&r[2].as_str()), "{r:?}");
            assert!(!r[3].is_empty());
            assert!(
                r[4] == "Proposed" || r[4].starts_with("Hard term"),
                "unknown status {:?}",
                r[4]
            );
        }
        // The terms the reviewers decide are all there, flagged.
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
            let row = rows.iter().find(|r| r[0] == term).unwrap_or_else(|| {
                panic!("{}: the term {term:?} is not in the terms table", def.tag)
            });
            assert!(
                row[4].starts_with("Hard term"),
                "{term:?} must be flagged: {:?}",
                row[4]
            );
        }
        // A term that says it is shown appears in some message of the language (the first form, before any ` / `).
        let all_text: String = def
            .entries
            .iter()
            .flat_map(|e| e.forms().map(|(_, t)| t))
            .collect::<Vec<_>>()
            .join("\n");
        for r in &rows {
            if r[2] == "yes" {
                for form in r[1].split(" / ") {
                    assert!(
                        all_text.contains(form),
                        "{} {:?}: the term {form:?} is said to be shown but no message has it",
                        def.tag,
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
        assert_eq!(sealed[2], "no", "{}", def.tag);
    }
}
