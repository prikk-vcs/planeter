//! RFC 012 D-4, T5: `docs/glossary.md` and the English catalog say the same thing. Every row of the glossary's
//! message tables (section 2) names a catalog key; its *Current* cell must be exactly the catalog text (both plural
//! forms, if the key has two). So the page cannot drift from what the pages say, and a catalog edit that skips
//! the glossary fails here.

use std::collections::BTreeMap;
use std::path::PathBuf;

use planeter_web::catalog::shipped;

fn glossary() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/glossary.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct Row {
    key: String,
    was: String,
    current: String,
    status: String,
}

/// The rows of the tables under `## 2.`: `| `key` | Was | Current | Status |`.
fn rows(src: &str) -> Vec<Row> {
    let mut in_messages = false;
    let mut out = Vec::new();
    for line in src.lines() {
        if line.starts_with("## ") {
            in_messages = line.starts_with("## 2.");
            continue;
        }
        if !in_messages || !line.starts_with("| `") {
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split(" | ")
            .map(str::trim)
            .collect();
        assert_eq!(
            cells.len(),
            4,
            "a message row has key, was, current, status: {line}"
        );
        out.push(Row {
            key: cells[0].trim_matches('`').to_owned(),
            was: cells[1].to_owned(),
            current: cells[2].to_owned(),
            status: cells[3].to_owned(),
        });
    }
    out
}

/// A catalog entry as the glossary writes it: `"text"`, or `"one form" / "other form"`.
fn as_cell(one: &str, other: &str) -> String {
    if one == other {
        format!("\"{one}\"")
    } else {
        format!("\"{one}\" / \"{other}\"")
    }
}

fn catalog() -> BTreeMap<&'static str, (&'static str, &'static str)> {
    shipped()[0]
        .entries
        .iter()
        .map(|e| (e.key, (e.one, e.other)))
        .collect()
}

#[test]
fn every_current_cell_is_the_catalog_text() {
    let src = glossary();
    let catalog = catalog();
    let rows = rows(&src);
    assert!(
        rows.len() > 50,
        "the message tables were not read: {} rows",
        rows.len()
    );
    let mut disagreements = Vec::new();
    for r in &rows {
        match catalog.get(r.key.as_str()) {
            None => disagreements.push(format!("`{}` is not a catalog key", r.key)),
            Some((one, other)) => {
                let want = as_cell(one, other);
                if r.current != want {
                    disagreements.push(format!(
                        "`{}`: the glossary says {} but the catalog says {want}",
                        r.key, r.current
                    ));
                }
            }
        }
    }
    assert_eq!(disagreements, Vec::<String>::new());
}

#[test]
fn a_key_is_named_once_with_a_known_status() {
    let rows = rows(&glossary());
    let mut seen = std::collections::BTreeSet::new();
    for r in &rows {
        assert!(seen.insert(r.key.clone()), "`{}` is listed twice", r.key);
        assert!(
            ["Approved", "Derived", "Held for prikk's confirmation"].contains(&r.status.as_str()),
            "`{}`: unknown status {:?}",
            r.key,
            r.status
        );
    }
}

#[test]
fn the_words_the_handoff_names_are_all_on_the_page() {
    let named: Vec<String> = rows(&glossary()).into_iter().map(|r| r.key).collect();
    for key in [
        "nav.code",
        "nav.history",
        "nav.refs",
        "nav.verify",
        "title.history",
        "title.change",
        "title.refs",
        "title.verify",
        "home.intro",
        "home.open",
        "login.failed",
        "login.sso",
        "login.user",
        "not_found.body",
        "unavailable.body",
        "too_large.heading",
        "too_large.body",
        "repo.location",
        "tree.at",
        "file.binary",
        "history.empty",
        "assurance.verified",
        "assurance.sealed",
        "assurance.queued",
        "assurance.unverified",
        "assurance.verify_unavailable",
        "explain.summary",
        "explain.verified",
        "explain.signed",
        "explain.queued",
        "explain.failed",
        "explain.failed_here",
        "explain.unavailable",
        "explain.patch",
        "explain.rollback",
        "verify.intro",
        "verify.findings",
        "verify.col.stage",
        "verify.status.evaluated",
        "verify.status.failed",
        "verify.status.not_evaluated",
        "verify.status.halted",
        "verify.status.halted_after",
        "verify.status.not_run",
        "repo.location_branch",
        "title.too_large",
        "verify.sessions",
        "page.csrf.body",
        "page.form_incomplete.body",
        "page.server_error.body",
        "page.sso_off.body",
        "page.sso_unlinked.body",
    ] {
        assert!(
            named.iter().any(|n| n == key),
            "the glossary does not list `{key}`"
        );
    }
}

#[test]
fn the_held_message_is_listed_as_held_and_is_exactly_what_it_was() {
    let rows = rows(&glossary());
    let held: Vec<&Row> = rows
        .iter()
        .filter(|r| r.status.starts_with("Held"))
        .collect();
    assert_eq!(held.len(), 1, "exactly one row is held");
    let r = held[0];
    assert_eq!(r.key, "verify.sessions");
    assert_eq!(r.was, r.current, "a held row does not change");
    // The words and the plural forms, as they were before the plain-words work (RFC 012 handoff T1 (a)).
    let (one, other) = catalog()["verify.sessions"];
    assert_eq!(one, "{verified} of {count} session verified.");
    assert_eq!(other, "{verified} of {count} sessions verified.");
}

#[test]
fn the_tables_use_the_new_column_names() {
    let src = glossary();
    for line in src
        .lines()
        .filter(|l| l.starts_with("| ") && !l.contains("---"))
    {
        let first = line.trim().trim_matches('|').trim();
        // Only header rows are checked: the cell that starts a table is "Concept …" or "Key".
        if first.starts_with("Concept") || first.starts_with("Key") {
            assert!(line.contains(" Was "), "{line}");
            assert!(line.contains(" Current "), "{line}");
            assert!(
                !line.contains("Today") && !line.contains("Proposed"),
                "{line}"
            );
        }
    }
    assert!(src.contains("Certainty"), "the Certainty column stays");
    // The case rule (the architect's answer to Q8) is written on the page.
    assert!(
        src.contains("In-cell tags and status words are lowercase"),
        "the case rule is not on the page"
    );
    assert!(
        !src.to_lowercase().contains("awaiting the owner's reading"),
        "no 'proposal' status left"
    );
}
