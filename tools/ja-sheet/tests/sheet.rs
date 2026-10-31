//! The generated sheet cannot drift from the catalogs: it has every message of both programs exactly once, in the group the maintained
//! context table gives, with the catalogs' English and Japanese character for character; and the check that says so is shown able to
//! fail (planted differences, a missing row, an extra key, a repeated row, a row in the wrong group).

use std::collections::{BTreeMap, BTreeSet};

use planeter_ja_sheet::{
    CONTEXTS, Contexts, Drift, Error, Group, HardTerm, Msg, NOTES, Program, Row, Stamp, assemble,
    check_against, civil_date, counts, escape_cell, fingerprint, hard_terms, live_rows,
    parse_notes, parse_sheet, render, unescape_cell,
};

fn stamp() -> Stamp {
    Stamp {
        commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        dirty: false,
        date: "2026-10-31".to_owned(),
    }
}

fn glossary() -> String {
    std::fs::read_to_string(format!(
        "{}/../../docs/glossary.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn sheet() -> (String, Vec<Row>) {
    let rows = live_rows().unwrap();
    let hard = hard_terms(&glossary());
    (render(&rows, &stamp(), &hard), rows)
}

// ---------------------------------------------------------------- every message, once, in a group

#[test]
fn the_sheet_has_every_message_of_both_catalogs_exactly_once() {
    let (sheet, rows) = sheet();
    let seen = parse_sheet(&sheet);
    let web: Vec<&str> = planeter_web::catalog::JA_DEF
        .entries
        .iter()
        .map(|e| e.key)
        .collect();
    let cli: Vec<&str> = planeter_client::catalog::Lang::Ja
        .entries()
        .iter()
        .map(|e| e.key)
        .collect();
    assert_eq!(seen.len(), web.len() + cli.len(), "one row per message");
    assert_eq!(rows.len(), seen.len());
    let keys: BTreeSet<&str> = seen.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys.len(), seen.len(), "no key twice");
    for k in web.iter().chain(&cli) {
        assert!(keys.contains(k), "`{k}` is missing from the sheet");
    }
    assert_eq!(
        check_against(&sheet, &rows),
        vec![],
        "a fresh sheet has no drift"
    );
}

#[test]
fn the_header_counts_per_group_add_up_and_say_the_commit_and_the_date() {
    let (sheet, rows) = sheet();
    let c = counts(&rows);
    assert_eq!(c.values().sum::<usize>(), rows.len());
    for g in Group::ALL {
        assert!(
            sheet.contains(&format!(
                "| **{}. {}** | {} |",
                g.letter(),
                g.title(),
                c[&g]
            )),
            "group {}",
            g.letter()
        );
        assert!(
            sheet.contains(&format!(
                "## Group {}. {} — {} rows",
                g.letter(),
                g.title(),
                c[&g]
            )),
            "the section of group {} says its count",
            g.letter()
        );
    }
    assert!(sheet.contains("commit `0123456789abcdef0123456789abcdef01234567`"));
    assert!(sheet.contains("on 2026-10-31"));
    assert!(sheet.contains(&format!("{} messages", rows.len())));
    assert!(sheet.contains(&format!("fingerprint `{}`", fingerprint(&rows))));
    let dirty = render(
        &rows,
        &Stamp {
            dirty: true,
            ..stamp()
        },
        &hard_terms(&glossary()),
    );
    assert!(
        dirty.contains("(+ uncommitted changes)"),
        "a sheet from a dirty tree says so"
    );
}

#[test]
fn the_groups_are_what_the_handoff_says() {
    let rows = live_rows().unwrap();
    let group_of = |key: &str| rows.iter().find(|r| r.key == key).unwrap().group;
    for key in [
        "assurance.verified",
        "explain.queued",
        "review.word.approved",
        "review.means.approved",
        "decision.form.approve",
        "proposal.state.landed",
        "proposal.what.open",
        "stage.key.fingerprint",
        "stage.none.button",
        "created.unverified",
    ] {
        assert_eq!(group_of(key), Group::A, "{key}");
    }
    for key in [
        "home.open",
        "login.submit",
        "history.heading",
        "proposals.heading",
        "comment.form.body",
        "new.heading",
        "not_found.heading",
        "layout.sign_in",
    ] {
        assert_eq!(group_of(key), Group::B, "{key}");
    }
    for key in [
        "help.main",
        "clone.done",
        "fetch.done",
        "propose.step.build",
        "tls.expired",
        "prikk.missing",
    ] {
        assert_eq!(group_of(key), Group::C, "{key}");
    }
    for r in &rows {
        let d = r.key.starts_with("error.")
            || r.key.starts_with("quota.")
            || r.key.starts_with("recovery.")
            || (r.program == Program::Web && r.key.starts_with("propose."));
        assert_eq!(r.group == Group::D, d, "{}", r.key);
        assert_eq!(
            r.program == Program::Client,
            r.group == Group::C,
            "{}",
            r.key
        );
    }
    // The pages of group B come in the order a person meets them, and within a page the catalog's order.
    let pages: Vec<&str> = rows
        .iter()
        .filter(|r| r.group == Group::B)
        .map(|r| r.page.as_str())
        .fold(Vec::new(), |mut v, p| {
            if v.last() != Some(&p) {
                v.push(p);
            }
            v
        });
    let first = |p: &str| pages.iter().position(|x| *x == p).unwrap();
    assert!(first("Home") < first("Files"));
    assert!(first("Files") < first("History and a change"));
    assert!(first("History and a change") < first("Branches and tags"));
    assert!(first("Branches and tags") < first("Verification"));
    assert!(first("Verification") < first("Signing in"));
    assert!(first("Signing in") < first("Proposals: the list"));
    assert!(first("Comments") < first("Creating a repository"));
    assert!(first("Creating a repository") < first("Not found, unavailable and other plain pages"));
    let mut seen_pages = BTreeSet::new();
    for r in rows.iter().filter(|r| r.group == Group::B) {
        let _ = seen_pages.insert(&r.page);
    }
    assert_eq!(
        seen_pages.len(),
        pages.len(),
        "a page is one block, never split"
    );
}

// ---------------------------------------------------------------- the check can fail

fn replace_row_cell(sheet: &str, key: &str, from: &str, to: &str) -> String {
    let marker = format!("| `{key}` |");
    sheet
        .lines()
        .map(|l| {
            if l.starts_with(&marker) {
                assert!(l.contains(from), "{key}: {from:?} not in the row");
                l.replacen(from, to, 1)
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_changed_japanese_cell_and_a_changed_english_cell_are_found() {
    let (sheet, rows) = sheet();
    let ja = rows
        .iter()
        .find(|r| r.key == "assurance.verified")
        .unwrap()
        .japanese
        .clone();
    let planted = replace_row_cell(&sheet, "assurance.verified", &ja, "検証ずみ");
    assert_eq!(
        check_against(&planted, &rows),
        vec![Drift::JapaneseDiffers {
            key: "assurance.verified".into()
        }]
    );
    let en = rows
        .iter()
        .find(|r| r.key == "login.submit")
        .unwrap()
        .english
        .clone();
    let planted = replace_row_cell(&sheet, "login.submit", &en, "Log in!");
    assert_eq!(
        check_against(&planted, &rows),
        vec![Drift::EnglishDiffers {
            key: "login.submit".into()
        }]
    );
    // A change in a client message is found too (the client's catalog is checked the same way).
    let en = rows.iter().find(|r| r.key == "prikk.missing").unwrap();
    let planted = replace_row_cell(
        &sheet,
        "prikk.missing",
        &escape_cell(&en.english)[..10],
        "prikk not found",
    );
    assert_eq!(
        check_against(&planted, &rows),
        vec![Drift::EnglishDiffers {
            key: "prikk.missing".into()
        }]
    );
}

#[test]
fn a_missing_row_an_unknown_key_a_repeated_row_and_a_row_in_the_wrong_group_are_found() {
    let (sheet, rows) = sheet();
    // A row removed.
    let without: String = sheet
        .lines()
        .filter(|l| !l.starts_with("| `home.open` |"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        check_against(&without, &rows),
        vec![Drift::MissingFromSheet {
            key: "home.open".into()
        }]
    );
    // A key the catalogs do not have.
    let line = sheet
        .lines()
        .find(|l| l.starts_with("| `home.open` |"))
        .unwrap();
    let extra = sheet.replacen(
        line,
        &format!(
            "{line}\n{}",
            line.replacen("`home.open`", "`home.opened`", 1)
        ),
        1,
    );
    assert_eq!(
        check_against(&extra, &rows),
        vec![Drift::NotInCatalog {
            key: "home.opened".into()
        }]
    );
    // A row twice.
    let twice = sheet.replacen(line, &format!("{line}\n{line}"), 1);
    assert_eq!(
        check_against(&twice, &rows),
        vec![Drift::Duplicate {
            key: "home.open".into()
        }]
    );
    // A row moved to another group (taken out of B and put under the heading of group D).
    let moved_out: String = sheet
        .lines()
        .filter(|l| *l != line)
        .collect::<Vec<_>>()
        .join("\n");
    let at = moved_out.find("## Group D.").unwrap();
    let after_heading = moved_out[at..].find("\n| key |").map(|i| at + i).unwrap();
    let moved = format!(
        "{}\n{line}{}",
        &moved_out[..after_heading],
        &moved_out[after_heading..]
    );
    assert_eq!(
        check_against(&moved, &rows),
        vec![Drift::WrongGroup {
            key: "home.open".into(),
            found: Group::D,
            expected: Group::B
        }]
    );
}

#[test]
fn a_hand_edit_of_the_key_cell_or_a_truncated_sheet_is_not_clean() {
    let (sheet, rows) = sheet();
    // Cut in the middle, at a character boundary (the sheet is mostly Japanese).
    let mut cut = sheet.len() / 2;
    while !sheet.is_char_boundary(cut) {
        cut -= 1;
    }
    let truncated = &sheet[..cut];
    let drift = check_against(truncated, &rows);
    assert!(
        drift
            .iter()
            .any(|d| matches!(d, Drift::MissingFromSheet { .. }))
    );
    assert!(!drift.is_empty());
    assert!(
        !check_against("", &rows).is_empty(),
        "an empty sheet has every message missing"
    );
    assert_eq!(check_against("", &rows).len(), rows.len());
}

// ---------------------------------------------------------------- the table escaping

#[test]
fn every_catalog_text_survives_the_cell_escaping_unchanged() {
    for r in live_rows().unwrap() {
        for text in [&r.english, &r.japanese, &r.note, &r.situation] {
            assert_eq!(&unescape_cell(&escape_cell(text)), text, "`{}`", r.key);
            assert!(
                !escape_cell(text).contains('\n'),
                "a cell is one line (`{}`)",
                r.key
            );
        }
    }
    let nasty = "a | b \\ c <dir> & *x* _y_ `z` [w]\nsecond\r\nthird <br> &lt;";
    assert_eq!(unescape_cell(&escape_cell(nasty)), nasty);
}

#[test]
fn a_cell_with_a_bar_does_not_split_the_row() {
    let rows = vec![Row {
        program: Program::Web,
        key: "home.open".into(),
        group: Group::B,
        page: "Home".into(),
        situation: "x".into(),
        english: "a | b".into(),
        japanese: "あ | い".into(),
        placeholders: vec![],
        note: String::new(),
    }];
    let text = render(&rows, &stamp(), &[]);
    let seen = parse_sheet(&text);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].english, "a | b");
    assert_eq!(seen[0].japanese, "あ | い");
}

// ---------------------------------------------------------------- the maintained tables

#[test]
fn every_message_has_a_context_and_a_bad_table_is_refused() {
    // (live_rows fails with NoContext for a message no line covers: it succeeded above.)
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let en = [Msg {
        key: "brand.new.key",
        one: "x",
        other: "x",
    }];
    let notes = BTreeMap::new();
    let err = assemble(&en, &en, &[], &[], &contexts, &notes).unwrap_err();
    assert_eq!(
        err,
        Error::NoContext {
            program: Program::Web,
            key: "brand.new.key".into()
        }
    );
    for bad in [
        "web\thome.*\tZ\tHome\tx",
        "web\thome.*\tB\tHome",
        "mobile\thome.*\tB\tHome\tx",
        "web\thome.*.x\tB\tHome\tx",
        "web\thome.*\tB\tHome\tx\nweb\thome.*\tB\tHome\ty",
        "web\ta.*\tA\tOne page\tx\nweb\tb.*\tB\tOne page\ty",
        "web\thome.*\tB\t\tx",
    ] {
        assert!(
            matches!(Contexts::parse(bad), Err(Error::BadContexts(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn the_longest_pattern_wins_and_an_exact_key_beats_a_prefix() {
    let contexts = Contexts::parse("web\treview.*\tB\tReview\tgeneral\nweb\treview.line.*\tA\tLine\tline\nweb\treview.line.x\tD\tExact\texact\n").unwrap();
    let msg = |k: &'static str| {
        [Msg {
            key: k,
            one: "a",
            other: "a",
        }]
    };
    let group = |k: &'static str| {
        assemble(&msg(k), &msg(k), &[], &[], &contexts, &BTreeMap::new()).unwrap()[0].group
    };
    assert_eq!(group("review.heading"), Group::B);
    assert_eq!(group("review.line.awaiting"), Group::A);
    assert_eq!(group("review.line.x"), Group::D);
}

#[test]
fn a_note_for_a_key_no_catalog_has_is_an_error_and_the_notes_table_is_clean() {
    let notes = parse_notes(NOTES).unwrap();
    assert!(
        notes.len() > 50,
        "the notes of the old sheet and addenda are carried over"
    );
    let live: BTreeSet<String> = live_rows().unwrap().into_iter().map(|r| r.key).collect();
    for k in notes.keys() {
        assert!(live.contains(k), "the note for `{k}` has no message");
    }
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let m = [Msg {
        key: "home.open",
        one: "a",
        other: "a",
    }];
    let mut bad = BTreeMap::new();
    bad.insert("home.opened".to_owned(), "x".to_owned());
    assert_eq!(
        assemble(&m, &m, &[], &[], &contexts, &bad).unwrap_err(),
        Error::NoteForUnknownKey("home.opened".into())
    );
    for malformed in ["no tab here", "a.b\t", "a.b\tx\na.b\ty"] {
        assert!(
            matches!(parse_notes(malformed), Err(Error::BadNotes(_))),
            "{malformed:?}"
        );
    }
}

#[test]
fn a_japanese_message_without_an_english_one_is_an_error_and_an_english_only_key_is_not_a_row() {
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let ja = [Msg {
        key: "home.open",
        one: "あ",
        other: "あ",
    }];
    assert_eq!(
        assemble(&[], &ja, &[], &[], &contexts, &BTreeMap::new()).unwrap_err(),
        Error::NoEnglish {
            key: "home.open".into()
        }
    );
    // English-only: the English table has two messages, the Japanese one only the first; the sheet has one row.
    let en = [
        Msg {
            key: "home.open",
            one: "Open",
            other: "Open",
        },
        Msg {
            key: "error.not_found.message",
            one: "No such resource.",
            other: "No such resource.",
        },
    ];
    let rows = assemble(&en, &ja, &[], &[], &contexts, &BTreeMap::new()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "home.open");
}

#[test]
fn a_counted_message_shows_both_forms_and_the_placeholders_are_listed() {
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let en = [Msg {
        key: "history.patch_count",
        one: "{count} patch",
        other: "{count} patches",
    }];
    let ja = [Msg {
        key: "history.patch_count",
        one: "{count} 件",
        other: "{count} 件",
    }];
    let rows = assemble(&en, &ja, &[], &[], &contexts, &BTreeMap::new()).unwrap();
    assert_eq!(
        rows[0].english,
        "[one] {count} patch [other] {count} patches"
    );
    assert_eq!(rows[0].japanese, "{count} 件");
    assert_eq!(rows[0].placeholders, vec!["count".to_owned()]);
}

// ---------------------------------------------------------------- the rest of the first page

#[test]
fn the_hard_terms_come_from_the_glossary_and_the_first_page_has_both_languages() {
    let hard = hard_terms(&glossary());
    assert!(hard.len() > 25, "{}", hard.len());
    assert!(
        hard.iter()
            .all(|t: &HardTerm| t.status.starts_with("Hard term"))
    );
    assert!(
        hard.iter()
            .any(|t| t.english == "repository" && t.japanese == "リポジトリ")
    );
    assert!(
        hard.iter()
            .any(|t| t.english.starts_with("Verified (badge)"))
    );
    assert!(
        !hard.iter().any(|t| t.english == "tag"),
        "a Proposed term is not on the list"
    );
    let (sheet, _) = sheet();
    for t in &hard {
        assert!(
            sheet.contains(&format!(
                "| {} | {} |",
                t.english.replace('|', "\\|"),
                t.japanese.replace('|', "\\|")
            )),
            "{}",
            t.english
        );
    }
    for needle in [
        "## 何をするか（日本語）",
        "## What to do (English)",
        "## Terms to decide first",
        "## What happens next",
        "reviewer's correction",
        "JA_SIGNOFF",
        "hide the lone English button",
        "one commit per group",
    ] {
        assert!(sheet.contains(needle), "{needle}");
    }
    // The first page comes before the first group.
    assert!(sheet.find("## What happens next").unwrap() < sheet.find("## Group A.").unwrap());
    assert!(sheet.find("## Terms to decide first").unwrap() < sheet.find("## Group A.").unwrap());
}

#[test]
fn the_fingerprint_changes_with_any_shown_text_and_not_with_a_note() {
    let mut rows = live_rows().unwrap();
    let before = fingerprint(&rows);
    assert_eq!(before.len(), 16);
    rows[0].note.push('!');
    assert_eq!(fingerprint(&rows), before, "a note is not the wording");
    rows[0].japanese.push('。');
    assert_ne!(fingerprint(&rows), before);
}

#[test]
fn dates_are_civil_dates_in_utc() {
    assert_eq!(civil_date(0), "1970-01-01");
    assert_eq!(civil_date(59), "1970-03-01");
    assert_eq!(civil_date(11_016), "2000-02-29");
    assert_eq!(civil_date(20_757), "2026-10-31");
    assert_eq!(civil_date(-1), "1969-12-31");
}
