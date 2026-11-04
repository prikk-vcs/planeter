//! The messages of `planeter-client`, over **every language of the registry** (RFC 004 part 9 T4; handoff `language-registry-handoff-v1`
//! T6, T9): each registered language has a catalog here, the catalogs hold exactly the keys the program names with the same
//! placeholders, the language's own rules (`crates/planeter-web/tests/i18n/<tag>.rules`: the retired words, the forbidden terms, the
//! style conventions, the wording that must stay — one set of rules for both programs) hold for the program's sentences, no markup or
//! colour is in a sentence, and the language choice follows `--lang`, then `LC_ALL`, `LC_MESSAGES`, `LANG`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use planeter_client::catalog::{
    CATALOGS, Key, Lang, baseline_text, placeholders, raw, size, t, unreviewed_count,
};
use planeter_i18n::languages::{LANGUAGES, check_registry};
use planeter_i18n::rules::{self, Program};

fn rules_text(tag: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../planeter-web/tests/i18n")
        .join(format!("{tag}.rules"));
    std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("the language `{tag}` has no rules file: add {} — docs/messages-and-languages.md, \"Adding a language\"", path.display())
    })
}

fn rules_for(tag: &str) -> Vec<rules::Rule> {
    let source = planeter_i18n::languages::source().tag;
    let mut all = Vec::new();
    for t in if tag == source {
        vec![tag]
    } else {
        vec![source, tag]
    } {
        all.extend(rules::parse(&rules_text(t)).unwrap_or_else(|e| panic!("{t}.rules: {e}")));
    }
    all
}

fn other() -> Lang {
    Lang::shipped()
        .into_iter()
        .find(|l| *l != Lang::SOURCE)
        .expect("a language besides the source ships")
}

#[test]
fn the_registry_and_this_programs_catalogs_are_the_same_list() {
    let registry: Vec<&str> = LANGUAGES.iter().map(|l| l.tag).collect();
    let catalogs: Vec<&str> = CATALOGS.iter().map(|d| d.tag).collect();
    assert_eq!(catalogs, registry);
    for (def, lang) in CATALOGS.iter().zip(LANGUAGES) {
        assert!(std::ptr::eq(def.lang, *lang), "{}", def.tag);
    }
    assert_eq!(check_registry(LANGUAGES), Vec::<String>::new());
}

#[test]
fn every_key_has_a_message_in_every_language_with_the_same_placeholders_and_no_stray_one() {
    let want: Vec<&'static str> = Key::ALL.iter().map(|k| k.id()).collect();
    let catalogs: Vec<planeter_i18n::Catalog<'_>> = CATALOGS.iter().map(|d| d.catalog()).collect();
    assert_eq!(
        planeter_i18n::audit_with(&catalogs, &want, &[], planeter_i18n::Options::PLAIN),
        vec![]
    );
    let set: BTreeSet<&str> = want.iter().copied().collect();
    for lang in Lang::all() {
        let ids: Vec<&str> = lang.entries().iter().map(|e| e.key).collect();
        assert_eq!(
            ids.len(),
            ids.iter().copied().collect::<BTreeSet<_>>().len(),
            "{}: a key is listed twice",
            lang.tag()
        );
        assert_eq!(
            ids.iter().copied().collect::<BTreeSet<_>>(),
            set,
            "{}: the table and the keys differ",
            lang.tag()
        );
    }
    for key in Key::ALL {
        let mut a = placeholders(raw(Lang::SOURCE, *key).unwrap());
        for lang in Lang::all() {
            let mut b = placeholders(raw(lang, *key).unwrap());
            let mut a2 = a.clone();
            a2.sort_unstable();
            a2.dedup();
            b.sort_unstable();
            b.dedup();
            assert_eq!(a2, b, "`{}` in {}", key.id(), lang.tag());
        }
        a.clear();
    }
}

#[test]
fn the_rules_of_each_language_hold_for_the_programs_sentences_and_are_shown_able_to_fail() {
    for def in CATALOGS.iter().copied() {
        let rules = rules_for(def.tag);
        let report = rules::check(&rules, Program::Client, def.tag, def.entries, &[]);
        assert_eq!(report.problems, Vec::<String>::new(), "{}", def.tag);
        // The wording that must stay names keys that exist in this program.
        if !def.is_source() {
            for key in rules::required_keys(&rules, Program::Client) {
                assert!(
                    def.entries.iter().any(|e| e.key == key),
                    "{}: the rules name `{key}`, which the catalog lacks",
                    def.tag
                );
            }
        }
    }
    // A planted retired word is found in the source language and in another one.
    for def in CATALOGS.iter().copied() {
        let mut entries = def.entries.to_vec();
        entries.push(planeter_i18n::msg(
            "planted.key",
            "The work was sealed in the history.",
        ));
        let report = rules::check(&rules_for(def.tag), Program::Client, def.tag, &entries, &[]);
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("`planted.key`") && p.contains("sealed")),
            "{}: {:?}",
            def.tag,
            report.problems
        );
    }
    // ... and the flag `--no-adopt` is let through.
    let mut entries = Lang::SOURCE.entries().to_vec();
    entries.push(planeter_i18n::msg(
        "planted.key",
        "Use --no-adopt to trust nothing.",
    ));
    assert_eq!(
        rules::check(&rules_for("en"), Program::Client, "en", &entries, &[]).problems,
        Vec::<String>::new()
    );
}

#[test]
fn no_message_carries_markup_or_colour() {
    for lang in Lang::all() {
        for e in lang.entries() {
            let text = e.other();
            assert!(
                !text.contains('\u{1b}'),
                "{} `{}`: an escape sequence",
                lang.tag(),
                e.key
            );
            assert!(
                !text.contains("<b>") && !text.contains("</") && !text.contains("<a "),
                "{} `{}`: markup",
                lang.tag(),
                e.key
            );
        }
    }
}

#[test]
fn a_sentence_the_person_reads_ends_like_a_sentence() {
    // Help texts, progress lines and the key prompts are not single sentences; every other English message ends with a full stop,
    // a question mark, a colon or an ellipsis.
    let exempt =
        |id: &str| id.starts_with("help.") || id.starts_with("progress.") || id == "version.line";
    for e in Lang::SOURCE.entries().iter().filter(|e| !exempt(e.key)) {
        let last = e.other().trim_end().chars().last().unwrap();
        assert!(
            matches!(last, '.' | '…' | ':' | '?' | '}'),
            "`{}` ends with {last:?}: {}",
            e.key,
            e.other()
        );
    }
}

#[test]
fn a_filled_message_has_no_placeholder_left_and_a_missing_key_is_a_visible_marker() {
    let line = t(Lang::SOURCE, Key::ProposeDoneNew)
        .with("id", "c-123")
        .with("revision", 1)
        .with("url", "https://forge.example/alice/app/proposals/c-123")
        .text();
    assert!(!line.contains('{'), "{line}");
    assert!(
        line.contains("held for review") && line.contains("no branch has changed"),
        "{line}"
    );
    // An unfilled placeholder stays visible (it is the caller's bug, not a silent blank).
    assert!(t(Lang::SOURCE, Key::ProposeDoneNew).text().contains("{id}"));
}

#[test]
fn the_language_is_the_flag_then_the_environment_and_the_source_language_when_neither_names_a_shipped_one()
 {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    let other = other();
    let tag: &'static str = other.tag();
    let locale =
        |name: &'static str, region: &'static str| -> &'static [(&'static str, &'static str)] {
            Box::leak(
                vec![(
                    name,
                    Box::leak(format!("{tag}_{region}.UTF-8").into_boxed_str()) as &'static str,
                )]
                .into_boxed_slice(),
            )
        };
    assert_eq!(Lang::from_environment(env(&[])), Lang::SOURCE);
    // A shipped language is chosen from any of the three variables, the first one set deciding.
    for name in ["LANG", "LC_MESSAGES", "LC_ALL"] {
        assert_eq!(
            Lang::from_environment(env(locale(name, "XX"))),
            other,
            "{name}"
        );
    }
    let both: &'static [(&'static str, &'static str)] = Box::leak(
        vec![
            (
                "LC_ALL",
                Box::leak(format!("{tag}_XX").into_boxed_str()) as &'static str,
            ),
            ("LANG", "en_US.UTF-8"),
        ]
        .into_boxed_slice(),
    );
    assert_eq!(Lang::from_environment(env(both)), other);
    let swapped: &'static [(&'static str, &'static str)] = Box::leak(
        vec![
            ("LC_ALL", "en_US.UTF-8"),
            (
                "LANG",
                Box::leak(format!("{tag}_XX").into_boxed_str()) as &'static str,
            ),
        ]
        .into_boxed_slice(),
    );
    assert_eq!(Lang::from_environment(env(swapped)), Lang::SOURCE);
    let skipped: &'static [(&'static str, &'static str)] = Box::leak(
        vec![
            ("LC_ALL", ""),
            (
                "LANG",
                Box::leak(format!("{tag}_XX").into_boxed_str()) as &'static str,
            ),
        ]
        .into_boxed_slice(),
    );
    assert_eq!(
        Lang::from_environment(env(skipped)),
        other,
        "an empty variable is skipped"
    );
    // Anything else is the source language: another language, the C locale, a name that only starts like a tag.
    for none in ["C", "POSIX", "fr_FR.UTF-8", "jam_XX", "de"] {
        let pairs: &'static [(&'static str, &'static str)] =
            Box::leak(vec![("LANG", none)].into_boxed_slice());
        assert_eq!(Lang::from_environment(env(pairs)), Lang::SOURCE, "{none}");
    }
    // --lang is the flag's own function; it matches a shipped tag as a BCP 47 tag and always wins over the environment.
    assert_eq!(Lang::from_flag(tag), Some(other));
    assert_eq!(Lang::from_flag(&format!("{tag}-XX")), Some(other));
    assert_eq!(
        Lang::from_flag(&Lang::SOURCE.tag().to_uppercase()),
        Some(Lang::SOURCE)
    );
    assert_eq!(Lang::from_flag("zz"), None);
}

#[test]
fn sizes_are_written_for_a_person() {
    assert_eq!(size(12), "12 B");
    assert_eq!(size(1536), "1.5 KiB");
    assert_eq!(size(5 << 20), "5.0 MiB");
    assert_eq!(size(3 << 30), "3.0 GiB");
}

#[test]
fn the_help_and_the_refusal_of_a_wrong_language_name_the_shipped_languages_from_the_registry() {
    let tags = Lang::shipped_tags("|");
    assert!(
        tags.split('|').count() >= 2 && tags.split('|').next() == Some(Lang::SOURCE.tag()),
        "{tags}"
    );
    for lang in Lang::shipped() {
        let help = t(lang, Key::HelpMain).with("languages", &tags).text();
        assert!(
            help.contains(&format!("--lang {tags}")),
            "{}: {help}",
            lang.tag()
        );
        assert!(!help.contains('{'), "{}: {help}", lang.tag());
        let bad = t(lang, Key::UsageBadLang)
            .with("value", "zz")
            .with("languages", Lang::shipped_tags(", "))
            .text();
        for tag in Lang::shipped().iter().map(|l| l.tag()) {
            assert!(bad.contains(tag), "{}: {bad}", lang.tag());
        }
    }
}

#[test]
fn a_signed_off_language_has_the_messages_as_signed_and_the_program_counts_what_is_newer() {
    for def in CATALOGS.iter().copied().filter(|d| d.sign_off().is_some()) {
        let text = baseline_text(def.tag).unwrap_or_else(|| {
            panic!(
                "{}: signed off without a baseline file (locales/{}.signed)",
                def.tag, def.tag
            )
        });
        let baseline = planeter_i18n::baseline::parse(text).unwrap();
        assert!(!baseline.is_empty());
        let n = unreviewed_count(def.tag).expect("a count");
        assert_eq!(
            n,
            planeter_i18n::baseline::unreviewed_count(def.entries, &baseline)
        );
    }
    assert!(unreviewed_count("en").is_none());
}
