//! The generated sheet cannot drift from the catalogs, for **every language of the registry besides the source**: it has every message
//! of both programs exactly once, in the group the maintained context table gives, with the catalogs' English and target text
//! character for character; for a signed-off language every row says where it stands against the sign-off; and the check that says
//! so is shown able to fail (planted differences, a missing row, an extra key, a repeated row, a row in the wrong group, a stale status).

use std::collections::{BTreeMap, BTreeSet};

use planeter_i18n::baseline::ReviewState;
use planeter_i18n::{Entry, Language, Status, msg, plural_msg};
use planeter_i18n_sheet::{
    CONTEXTS, Contexts, Drift, Error, Group, HardTerm, LANGUAGE_DATA, Program, Row, Stamp,
    check_against, civil_date, counts, escape_cell, fingerprint, hard_terms, instructions,
    live_rows, live_sources, notes_of, parse_notes, parse_sheet, render, rows_of, sheet_language,
    state_counts, status_lines, unescape_cell,
};

fn stamp() -> Stamp {
    Stamp {
        commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        dirty: false,
        date: "2026-11-04".to_owned(),
    }
}

/// The languages with a sheet: every registry language besides the source.
fn languages() -> Vec<&'static Language> {
    planeter_i18n::languages::LANGUAGES
        .iter()
        .copied()
        .filter(|l| !l.is_source())
        .collect()
}

fn glossary(tag: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../docs/glossary/{tag}.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn instr(lang: &Language) -> (String, String) {
    let (_, text, _) = LANGUAGE_DATA.iter().find(|(t, _, _)| *t == lang.tag).unwrap_or_else(|| {
        panic!("the language `{}` has no data/{}/instructions: add it, and a line in LANGUAGE_DATA", lang.tag, lang.tag)
    });
    instructions(text, lang).unwrap()
}

fn sheet(lang: &'static Language) -> (String, Vec<Row>) {
    let rows = live_rows(lang.tag).unwrap();
    let hard = hard_terms(&glossary(lang.tag));
    (render(&rows, lang, &instr(lang), &stamp(), &hard), rows)
}

// ---------------------------------------------------------------- every message, once, in a group

#[test]
fn the_sheet_has_every_message_of_both_catalogs_exactly_once() {
    for lang in languages() {
        let (sheet, rows) = sheet(lang);
        let seen = parse_sheet(&sheet);
        let web = planeter_web::catalog::catalog_of(lang.tag)
            .unwrap()
            .entries
            .iter()
            .map(|e| e.key)
            .collect::<Vec<_>>();
        let cli = planeter_client::catalog::CATALOGS
            .iter()
            .find(|d| d.tag == lang.tag)
            .unwrap()
            .entries
            .iter()
            .map(|e| e.key)
            .collect::<Vec<_>>();
        assert_eq!(
            seen.len(),
            web.len() + cli.len(),
            "{}: one row per message",
            lang.tag
        );
        assert_eq!(rows.len(), seen.len());
        let keys: BTreeSet<&str> = seen.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys.len(), seen.len(), "{}: no key twice", lang.tag);
        for k in web.iter().chain(&cli) {
            assert!(
                keys.contains(k),
                "{}: `{k}` is missing from the sheet",
                lang.tag
            );
        }
        assert_eq!(
            check_against(&sheet, &rows),
            vec![],
            "{}: a fresh sheet has no drift",
            lang.tag
        );
        assert_eq!(sheet_language(&sheet).as_deref(), Some(lang.tag));
    }
}

#[test]
fn the_header_counts_per_group_add_up_and_say_the_commit_the_date_and_the_status() {
    for lang in languages() {
        let (sheet, rows) = sheet(lang);
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
                "group {}",
                g.letter()
            );
        }
        assert!(sheet.contains("commit `0123456789abcdef0123456789abcdef01234567`"));
        assert!(sheet.contains("on 2026-11-04"));
        assert!(sheet.contains(&format!("{} messages", rows.len())));
        assert!(sheet.contains(&format!("fingerprint `{}`", fingerprint(&rows))));
        assert!(sheet.contains(&format!("**Language `{}`", lang.tag)));
        let dirty = render(
            &rows,
            lang,
            &instr(lang),
            &Stamp {
                dirty: true,
                ..stamp()
            },
            &hard_terms(&glossary(lang.tag)),
        );
        assert!(
            dirty.contains("(+ uncommitted changes)"),
            "a sheet from a dirty tree says so"
        );
    }
}

#[test]
fn the_groups_are_what_the_handoff_says() {
    for lang in languages() {
        let rows = live_rows(lang.tag).unwrap();
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
        // The pages of group B come in the order a person meets them, and within a page the catalog's order; a page is one block.
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
        assert!(
            first("Creating a repository") < first("Not found, unavailable and other plain pages")
        );
        let distinct: BTreeSet<&str> = pages.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            pages.len(),
            "a page is one block, never split"
        );
    }
}

// ---------------------------------------------------------------- the status against the sign-off

#[test]
fn a_signed_off_language_marks_every_row_against_the_baseline_and_a_draft_has_no_status_column() {
    for lang in languages() {
        let (sheet, rows) = sheet(lang);
        let signed = lang.sign_off().is_some();
        assert_eq!(
            rows.iter().all(|r| r.state.is_some()),
            signed,
            "{}",
            lang.tag
        );
        assert_eq!(sheet.contains("| since sign-off |"), signed, "{}", lang.tag);
        if signed {
            let (u, c, n) = state_counts(&rows);
            assert_eq!(u + c + n, rows.len());
            assert!(
                sheet.contains(&format!(
                    "**Since then: {u} unchanged, {c} changed, {n} new**"
                )),
                "{}",
                lang.tag
            );
            assert!(sheet.contains("**Signed off** by "), "{}", lang.tag);
            for r in &rows {
                let cell = r.state.unwrap().label();
                assert!(
                    sheet
                        .lines()
                        .any(|l| l.starts_with(&format!("| `{}` |", r.key))
                            && l.contains(&format!("| {cell} |"))),
                    "`{}`",
                    r.key
                );
            }
            // The count is the programs' own (what the server logs at start).
            let web = planeter_web::catalog::unreviewed_count(lang.tag).unwrap();
            let cli = planeter_client::catalog::unreviewed_count(lang.tag).unwrap();
            assert_eq!(c + n, web + cli, "{}", lang.tag);
            let lines = status_lines().unwrap();
            assert!(
                lines.iter().any(|l| l.starts_with(lang.tag)
                    && l.contains(&format!("{} message(s) newer than the sign-off", web + cli))),
                "{lines:?}"
            );
        }
    }
}

#[test]
fn a_changed_and_a_new_message_are_marked_and_the_status_is_part_of_the_check() {
    let lang = languages()
        .into_iter()
        .find(|l| l.sign_off().is_some())
        .expect("a signed-off language");
    let src = live_sources(lang.tag).unwrap();
    let baseline = src.web_baseline.clone().unwrap();
    // Planted catalogs: one signed message changed, one added.
    let mut target: Vec<Entry> = src.web_target.to_vec();
    let at = target
        .iter()
        .position(|e| {
            baseline.contains_key(e.key)
                && planeter_i18n::baseline::message_hash(e) == baseline[e.key]
        })
        .unwrap();
    let changed_key = target[at].key;
    target[at] = msg(changed_key, "changed after the sign-off");
    let mut source: Vec<Entry> = src.web_source.to_vec();
    source.push(msg("error.not_found.planted", "x"));
    target.push(msg("error.not_found.planted", "y"));
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let rows = rows_of(
        Program::Web,
        &source,
        &target,
        &contexts,
        &BTreeMap::new(),
        Some(&baseline),
    )
    .unwrap();
    let state = |k: &str| rows.iter().find(|r| r.key == k).unwrap().state;
    assert_eq!(state(changed_key), Some(ReviewState::Changed));
    assert_eq!(state("error.not_found.planted"), Some(ReviewState::New));
    assert_eq!(state("login.user"), Some(ReviewState::Unchanged));
    // A stale status cell is drift.
    let (sheet, live) = sheet(lang);
    let victim = live
        .iter()
        .find(|r| r.state == Some(ReviewState::Unchanged))
        .unwrap();
    let stale: String = sheet
        .lines()
        .map(|l| {
            if l.starts_with(&format!("| `{}` |", victim.key)) {
                l.replace("| unchanged |", "| new since sign-off |")
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        check_against(&stale, &live),
        vec![Drift::StateDiffers {
            key: victim.key.clone()
        }]
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
fn a_changed_target_cell_and_a_changed_english_cell_are_found() {
    for lang in languages() {
        let (sheet, rows) = sheet(lang);
        let target = rows
            .iter()
            .find(|r| r.key == "assurance.verified")
            .unwrap()
            .target
            .clone();
        let planted = replace_row_cell(
            &sheet,
            "assurance.verified",
            &escape_cell(&target),
            "planted text",
        );
        assert_eq!(
            check_against(&planted, &rows),
            vec![Drift::TargetDiffers {
                key: "assurance.verified".into()
            }]
        );
        let en = rows
            .iter()
            .find(|r| r.key == "login.submit")
            .unwrap()
            .english
            .clone();
        let planted = replace_row_cell(&sheet, "login.submit", &escape_cell(&en), "Log in!");
        assert_eq!(
            check_against(&planted, &rows),
            vec![Drift::EnglishDiffers {
                key: "login.submit".into()
            }]
        );
        // A change in a client message is found too (the client's catalog is checked the same way).
        let client_en = rows.iter().find(|r| r.key == "prikk.missing").unwrap();
        let planted = replace_row_cell(
            &sheet,
            "prikk.missing",
            &escape_cell(&client_en.english)[..10],
            "prikk not found",
        );
        assert_eq!(
            check_against(&planted, &rows),
            vec![Drift::EnglishDiffers {
                key: "prikk.missing".into()
            }]
        );
    }
}

#[test]
fn a_missing_row_an_unknown_key_a_repeated_row_and_a_row_in_the_wrong_group_are_found() {
    let lang = languages()[0];
    let (sheet, rows) = sheet(lang);
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
fn a_truncated_or_empty_sheet_is_not_clean() {
    let lang = languages()[0];
    let (sheet, rows) = sheet(lang);
    // Cut in the middle, at a character boundary (a sheet may be mostly non-ASCII).
    let mut cut = sheet.len() / 2;
    while !sheet.is_char_boundary(cut) {
        cut -= 1;
    }
    let drift = check_against(&sheet[..cut], &rows);
    assert!(
        drift
            .iter()
            .any(|d| matches!(d, Drift::MissingFromSheet { .. }))
    );
    assert!(
        !check_against("", &rows).is_empty(),
        "an empty sheet has every message missing"
    );
    assert_eq!(check_against("", &rows).len(), rows.len());
}

// ---------------------------------------------------------------- the table escaping

#[test]
fn every_catalog_text_survives_the_cell_escaping_unchanged() {
    for lang in languages() {
        for r in live_rows(lang.tag).unwrap() {
            for text in [&r.english, &r.target, &r.note, &r.situation] {
                assert_eq!(&unescape_cell(&escape_cell(text)), text, "`{}`", r.key);
                assert!(
                    !escape_cell(text).contains('\n'),
                    "a cell is one line (`{}`)",
                    r.key
                );
            }
        }
    }
    let nasty = "a | b \\ c <dir> & *x* _y_ `z` [w]\nsecond\r\nthird <br> &lt;";
    assert_eq!(unescape_cell(&escape_cell(nasty)), nasty);
}

#[test]
fn a_cell_with_a_bar_does_not_split_the_row() {
    let lang = languages()[0];
    let rows = vec![Row {
        program: Program::Web,
        key: "home.open".into(),
        group: Group::B,
        page: "Home".into(),
        situation: "x".into(),
        english: "a | b".into(),
        target: "あ | い".into(),
        placeholders: vec![],
        note: String::new(),
        state: None,
    }];
    let text = render(&rows, lang, &instr(lang), &stamp(), &[]);
    let seen = parse_sheet(&text);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].english, "a | b");
    assert_eq!(seen[0].target, "あ | い");
}

// ---------------------------------------------------------------- the maintained tables

#[test]
fn every_message_has_a_context_and_a_bad_table_is_refused() {
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let en = [msg("brand.new.key", "x")];
    let notes = BTreeMap::new();
    let err = rows_of(Program::Web, &en, &en, &contexts, &notes, None).unwrap_err();
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
    let group = |k: &'static str| {
        rows_of(
            Program::Web,
            &[msg(k, "a")],
            &[msg(k, "a")],
            &contexts,
            &BTreeMap::new(),
            None,
        )
        .unwrap()[0]
            .group
    };
    assert_eq!(group("review.heading"), Group::B);
    assert_eq!(group("review.line.awaiting"), Group::A);
    assert_eq!(group("review.line.x"), Group::D);
}

#[test]
fn a_note_for_a_key_no_catalog_has_is_an_error_and_the_notes_tables_are_clean() {
    for lang in languages() {
        let notes = notes_of(lang.tag).unwrap();
        let live: BTreeSet<String> = live_rows(lang.tag)
            .unwrap()
            .into_iter()
            .map(|r| r.key)
            .collect();
        for k in notes.keys() {
            assert!(live.contains(k), "the note for `{k}` has no message");
        }
    }
    assert!(
        notes_of("ja").unwrap().len() > 50,
        "the notes of the old sheet and addenda are carried over"
    );
    for malformed in ["no tab here", "a.b\t", "a.b\tx\na.b\ty"] {
        assert!(
            matches!(parse_notes(malformed), Err(Error::BadNotes(_))),
            "{malformed:?}"
        );
    }
    // A note for a key that is not in any catalog fails the rows.
    let lang = languages()[0];
    let src = live_sources(lang.tag).unwrap();
    let mut bad = BTreeMap::new();
    bad.insert("home.opened".to_owned(), "x".to_owned());
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    assert_eq!(
        planeter_i18n_sheet::rows_for(&src, &contexts, &bad).unwrap_err(),
        Error::NoteForUnknownKey("home.opened".into())
    );
}

#[test]
fn a_target_message_without_a_source_message_is_an_error_and_an_english_only_key_is_not_a_row() {
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let ja = [msg("home.open", "あ")];
    assert_eq!(
        rows_of(Program::Web, &[], &ja, &contexts, &BTreeMap::new(), None).unwrap_err(),
        Error::NoEnglish {
            key: "home.open".into()
        }
    );
    // English-only: the source table has two messages, the target one only the first; the sheet has one row.
    let en = [
        msg("home.open", "Open"),
        msg("error.not_found.message", "No such resource."),
    ];
    let rows = rows_of(Program::Web, &en, &ja, &contexts, &BTreeMap::new(), None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "home.open");
}

#[test]
fn a_counted_message_shows_every_form_labelled_and_the_placeholders_are_listed() {
    let contexts = Contexts::parse(CONTEXTS).unwrap();
    let en = [plural_msg(
        "history.patch_count",
        "{count} patch",
        "{count} patches",
    )];
    let ja = [msg("history.patch_count", "{count} 件")];
    let rows = rows_of(Program::Web, &en, &ja, &contexts, &BTreeMap::new(), None).unwrap();
    assert_eq!(
        rows[0].english,
        "[one] {count} patch [other] {count} patches"
    );
    assert_eq!(rows[0].target, "{count} 件");
    assert_eq!(rows[0].placeholders, vec!["count".to_owned()]);
}

// ---------------------------------------------------------------- the first page

#[test]
fn the_hard_terms_come_from_the_glossary_file_and_the_first_page_has_both_languages_and_a_status_aware_wording()
 {
    for lang in languages() {
        let hard = hard_terms(&glossary(lang.tag));
        assert!(hard.len() > 25, "{} {}", lang.tag, hard.len());
        assert!(
            hard.iter()
                .all(|t: &HardTerm| t.status.starts_with("Hard term"))
        );
        assert!(hard.iter().any(|t| t.english == "repository"));
        assert!(
            hard.iter()
                .any(|t| t.english.starts_with("Verified (badge)"))
        );
        assert!(
            !hard.iter().any(|t| t.english == "tag"),
            "a Proposed term is not on the list"
        );
        let (sheet, _) = sheet(lang);
        for t in &hard {
            assert!(
                sheet.contains(&format!(
                    "| {} | {} |",
                    t.english.replace('|', "\\|"),
                    t.target.replace('|', "\\|")
                )),
                "{}",
                t.english
            );
        }
        for needle in [
            format!("## What to do ({})", lang.endonym),
            "## What to do (English)".to_owned(),
            "## Terms to decide first".to_owned(),
            "## What happens next".to_owned(),
            "reviewer's correction".to_owned(),
            "one commit per group".to_owned(),
        ] {
            assert!(sheet.contains(&needle), "{needle}");
        }
        // The first page comes before the first group.
        assert!(sheet.find("## What happens next").unwrap() < sheet.find("## Group A.").unwrap());
        assert!(
            sheet.find("## Terms to decide first").unwrap() < sheet.find("## Group A.").unwrap()
        );
    }
}

#[test]
fn the_instruction_wording_follows_the_status_of_the_language() {
    let lang = languages()[0];
    let text = LANGUAGE_DATA
        .iter()
        .find(|(t, _, _)| *t == lang.tag)
        .unwrap()
        .1;
    let draft = Language {
        status: Status::Draft,
        ..clone_of(lang)
    };
    let signed = instructions(text, lang).unwrap();
    let as_draft = instructions(text, &draft).unwrap();
    assert!(
        as_draft.0.contains("a draft") && !as_draft.0.contains("the current text"),
        "{}",
        as_draft.0
    );
    assert!(
        signed.0.contains("the current text") && !signed.0.contains("a draft"),
        "{}",
        signed.0
    );
    assert_ne!(
        as_draft.1, signed.1,
        "the language's own section follows the status too"
    );
    for bad in [
        "=== english\nonly one section",
        "=== english\na {{x\n=== local\nb",
        "=== english\na {{x}}\n=== local\nb",
    ] {
        assert!(
            matches!(instructions(bad, lang), Err(Error::BadInstructions(_))),
            "{bad:?}"
        );
    }
}

fn clone_of(l: &'static Language) -> Language {
    Language {
        tag: l.tag,
        endonym: l.endonym,
        dir: l.dir,
        categories: l.categories,
        plural: l.plural,
        group_separator: l.group_separator,
        decimal_separator: l.decimal_separator,
        status: Status::Draft,
    }
}

#[test]
fn the_fingerprint_changes_with_any_shown_text_and_not_with_a_note_or_a_status() {
    let lang = languages()[0];
    let mut rows = live_rows(lang.tag).unwrap();
    let before = fingerprint(&rows);
    assert_eq!(before.len(), 16);
    rows[0].note.push('!');
    rows[0].state = Some(ReviewState::Changed);
    assert_eq!(
        fingerprint(&rows),
        before,
        "a note and a status are not the wording"
    );
    rows[0].target.push('。');
    assert_ne!(fingerprint(&rows), before);
}

#[test]
fn an_unknown_language_is_refused_and_dates_are_civil_dates_in_utc() {
    assert!(matches!(live_rows("zz"), Err(Error::UnknownLanguage(_))));
    assert!(
        matches!(live_rows("en"), Ok(_) | Err(_)),
        "the source language is not a target, but asking is not a crash"
    );
    assert_eq!(civil_date(0), "1970-01-01");
    assert_eq!(civil_date(59), "1970-03-01");
    assert_eq!(civil_date(11_016), "2000-02-29");
    assert_eq!(civil_date(20_757), "2026-10-31");
    assert_eq!(civil_date(-1), "1969-12-31");
}
