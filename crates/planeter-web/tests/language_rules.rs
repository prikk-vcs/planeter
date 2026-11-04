//! A language's own rules, as data (handoff `language-registry-handoff-v1` T9): for **every language in the registry** the rules in
//! `tests/i18n/<tag>.rules` hold for the server's messages. The source language's retired words apply to every language (a retired
//! English word may not come back inside a Japanese sentence either); a language's own file adds its own forbidden terms, its style
//! conventions and the wording that must stay. Every rule carries planted examples and is shown to fail by them (`selftest`). A new
//! language without a rules file fails the build with a sentence that says to add one; an empty file is allowed and says so.
//! The allowlist `tests/retired_words.allow` silences exactly one message and term, with a reason, and a dead line fails.

use std::path::PathBuf;

use planeter_i18n::rules::{self, Program, Rule};
use planeter_web::catalog::CATALOGS;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// The text of a language's rules file, or the sentence that says what to do.
fn rules_text(tag: &str) -> Result<String, String> {
    let path = tests_dir().join("i18n").join(format!("{tag}.rules"));
    std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "the language `{tag}` has no rules file: add {} (an empty file, with a comment saying so, is allowed) — docs/messages-and-languages.md, \"Adding a language\"",
            path.display()
        )
    })
}

/// The rules that hold for a language: the source language's, and its own.
fn rules_for(tag: &str, source_tag: &str) -> Vec<Rule> {
    let mut all = Vec::new();
    for t in if tag == source_tag {
        vec![tag]
    } else {
        vec![source_tag, tag]
    } {
        let text = rules_text(t).unwrap_or_else(|e| panic!("{e}"));
        all.extend(rules::parse(&text).unwrap_or_else(|e| panic!("tests/i18n/{t}.rules: {e}")));
    }
    all
}

fn allowlist() -> Vec<rules::Allowed> {
    let path = tests_dir().join("retired_words.allow");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    rules::parse_allow(&src).unwrap()
}

#[test]
fn every_language_of_the_registry_has_a_rules_file_and_every_rule_is_shown_able_to_fail() {
    for lang in planeter_i18n::languages::LANGUAGES {
        let rules = rules_for(lang.tag, planeter_i18n::languages::source().tag);
        assert_eq!(
            rules::selftest(&rules),
            Vec::<String>::new(),
            "{}",
            lang.tag
        );
        if !lang.is_source() {
            assert!(
                rules_text(lang.tag)
                    .unwrap()
                    .lines()
                    .any(|l| !l.trim().is_empty()),
                "{}: an empty rules file must say so in a comment",
                lang.tag
            );
        }
    }
}

#[test]
fn a_language_without_a_rules_file_fails_with_a_sentence_that_says_to_add_one() {
    let e = rules_text("zz-planted").unwrap_err();
    assert!(
        e.contains("has no rules file")
            && e.contains("zz-planted.rules")
            && e.contains("Adding a language"),
        "{e}"
    );
}

#[test]
fn no_message_of_any_language_breaks_a_rule_of_its_language_and_the_allowlist_has_no_dead_lines() {
    let allow = allowlist();
    let source_tag = planeter_i18n::languages::source().tag;
    let mut problems = Vec::new();
    let mut used = vec![false; allow.len()];
    for def in CATALOGS.iter().copied() {
        let rules = rules_for(def.tag, source_tag);
        let report = rules::check(&rules, Program::Server, def.tag, def.entries, &allow);
        problems.extend(report.problems);
        for (u, r) in used.iter_mut().zip(&report.allow_used) {
            *u |= *r;
        }
        // The pairs a language says must stay name keys that exist.
        for key in rules::required_keys(&rules, Program::Server) {
            if def.tag != source_tag {
                assert!(
                    def.entries.iter().any(|e| e.key == key),
                    "{}: the rule names `{key}`, which the catalog lacks",
                    def.tag
                );
            }
        }
    }
    assert_eq!(problems, Vec::<String>::new());
    let stale: Vec<String> = allow
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(a, _)| format!("`{}` :: {:?}", a.key, a.term))
        .collect();
    assert_eq!(
        stale,
        Vec::<String>::new(),
        "allowlist lines no message needs"
    );
}

/// The rules have teeth on the real catalogs: planting a breach in a copy of each shipped catalog is found.
#[test]
fn a_planted_breach_in_a_copy_of_a_real_catalog_is_found_for_every_language() {
    let source_tag = planeter_i18n::languages::source().tag;
    for def in CATALOGS.iter().copied() {
        let rules = rules_for(def.tag, source_tag);
        // English retired words apply to every language.
        let mut entries: Vec<planeter_i18n::Entry> = def.entries.to_vec();
        entries.push(planeter_i18n::msg("planted.key", "Sign in with SSO"));
        let report = rules::check(&rules, Program::Server, def.tag, &entries, &[]);
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("`planted.key`") && p.contains("SSO")),
            "{}: {:?}",
            def.tag,
            report.problems
        );
    }
}
