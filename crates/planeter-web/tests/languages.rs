//! RFC 012 D-3 / D-5 over **every language of the registry** (handoff `language-registry-handoff-v1` T2, T3, T9): each registered
//! language has a catalog in the server, the catalogs are complete and consistent with the source language, the checks are shown able
//! to fail for them, a language is shipped exactly when its status says so, a signed-off language carries a sound record and a baseline,
//! and the operator's preview names only drafts. Nothing here names a language.

use planeter_i18n::languages::{LANGUAGES, check_registry, previewable_of, shipped_of};
use planeter_web::catalog::{
    CATALOGS, Category, EN_DEF, Entry, Key, Lang, LangDef, Problem, T, audit, baseline_text,
    enable_preview_languages, english_only_keys, msg, planted_language, previewable, shipped,
    unreviewed_count,
};

fn others() -> Vec<&'static LangDef> {
    CATALOGS
        .iter()
        .copied()
        .filter(|d| !d.is_source())
        .collect()
}

/// A copy of a language's catalog with one thing changed, for the planted cases.
fn derived(def: &'static LangDef, f: impl Fn(&Entry) -> Option<Entry>) -> &'static LangDef {
    let entries: Vec<Entry> = def.entries.iter().filter_map(f).collect();
    planted_language(
        def.tag,
        def.endonym,
        def.dir,
        def.categories,
        def.plural,
        def.group_separator,
        def.decimal_separator,
        Box::leak(entries.into_boxed_slice()),
    )
}

#[test]
fn the_registry_and_the_servers_catalogs_are_the_same_list_and_the_registry_is_sound() {
    let registry: Vec<&str> = LANGUAGES.iter().map(|l| l.tag).collect();
    let catalogs: Vec<&str> = CATALOGS.iter().map(|d| d.tag).collect();
    assert_eq!(
        catalogs, registry,
        "a registered language has a catalog here, in the registry's order, and the other way round"
    );
    for (def, lang) in CATALOGS.iter().zip(LANGUAGES) {
        assert!(
            std::ptr::eq(def.lang, *lang),
            "{}: the catalog is for the registry's row",
            def.tag
        );
    }
    assert_eq!(check_registry(LANGUAGES), Vec::<String>::new());
}

#[test]
fn every_catalog_is_complete_consistent_and_free_of_markup_and_follows_the_source_key_order() {
    assert_eq!(audit(&CATALOGS), vec![], "the catalogs have problems");
    // Every source key, in the source order: the reviewers' sheet and a correction map back mechanically. Except exactly the keys that
    // are English in every language (`ENGLISH_ONLY_KEYS`, empty unless the owner has decided otherwise).
    let only = english_only_keys();
    let source: Vec<&str> = EN_DEF
        .entries
        .iter()
        .map(|e| e.key)
        .filter(|k| !only.contains(k))
        .collect();
    for def in others() {
        let keys: Vec<&str> = def.entries.iter().map(|e| e.key).collect();
        assert_eq!(
            keys, source,
            "{}: the catalog follows the source key order",
            def.tag
        );
        assert_eq!(
            Key::ALL.len() - only.len(),
            def.entries.len(),
            "{}",
            def.tag
        );
    }
}

#[test]
fn a_language_has_exactly_the_plural_forms_it_declares_and_numbers_use_its_separators() {
    for def in CATALOGS.iter().copied() {
        for e in def.entries {
            let have: Vec<Category> = e.forms().map(|(c, _)| c).collect();
            assert!(
                have.iter().all(|c| def.categories.contains(c)),
                "{} `{}` has a form the language does not declare",
                def.tag,
                e.key
            );
            assert!(
                have.contains(&Category::Other),
                "{} `{}`: `other` is required",
                def.tag,
                e.key
            );
        }
        let lang = Lang::from_def(def);
        let g = def.group_separator;
        assert_eq!(lang.number(1200), format!("1{g}200"), "{}", def.tag);
        assert_eq!(
            lang.number(1_234_567),
            format!("1{g}234{g}567"),
            "{}",
            def.tag
        );
        assert_eq!(lang.number(12), "12");
        // A counted message gives the same text for 1 and many only if the language has no singular.
        let t = T::new(lang);
        let one = t.msg(Key::HistoryPatchCount).n(1).text();
        let many = t.msg(Key::HistoryPatchCount).n(2).text();
        assert_eq!(
            one == many.replace('2', "1"),
            !def.categories.contains(&Category::One),
            "{}: {one:?} / {many:?}",
            def.tag
        );
        // The units are the same in every language.
        assert_eq!(t.size(223), "223 B");
        assert_eq!(
            t.size(2048),
            "2.0 KiB".replace('.', &def.decimal_separator.to_string())
        );
    }
}

#[test]
fn the_five_trust_labels_are_five_different_non_empty_labels_in_every_language() {
    for def in CATALOGS.iter().copied() {
        let t = T::new(Lang::from_def(def));
        let labels = [
            t.get(Key::AssuranceVerified),
            t.get(Key::AssuranceSealed),
            t.get(Key::AssuranceQueued),
            t.get(Key::AssuranceUnverified),
            t.get(Key::AssuranceVerifyUnavailable),
        ];
        assert!(labels.iter().all(|l| !l.trim().is_empty()), "{}", def.tag);
        let mut sorted = labels.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            5,
            "{}: two trust labels read the same: {labels:?}",
            def.tag
        );
    }
}

// -------------------------------------------------------------------------- the checks can fail (planted)

#[test]
fn a_key_missing_in_a_language_is_found() {
    for def in others() {
        let l = derived(def, |e| (e.key != "login.failed").then_some(*e));
        let p = audit(&[&EN_DEF, l]);
        assert!(
            p.contains(&Problem::Missing {
                lang: def.tag,
                key: "login.failed"
            }),
            "{p:?}"
        );
    }
}

#[test]
fn a_changed_placeholder_in_a_language_is_found() {
    for def in others() {
        // `title.change` takes {target}, {repo}, {product}: rename one.
        let l = derived(def, |e| {
            Some(if e.key == "title.change" {
                msg(e.key, "{goal} · {repo} · {product}")
            } else {
                *e
            })
        });
        let p = audit(&[&EN_DEF, l]);
        assert!(
            p.contains(&Problem::PlaceholdersDiffer {
                key: "title.change",
                lang: def.tag
            }),
            "{p:?}"
        );
    }
}

#[test]
fn a_form_the_language_does_not_declare_and_a_missing_one_are_found() {
    for def in others() {
        if def.categories.contains(&Category::One) {
            continue;
        }
        let l = derived(def, |e| {
            Some(if e.key == "history.patch_count" {
                Entry::with_forms(
                    e.key,
                    &[(Category::One, "{count} a"), (Category::Other, "{count} b")],
                )
            } else {
                *e
            })
        });
        let p = audit(&[&EN_DEF, l]);
        assert!(
            p.contains(&Problem::PluralFormsDisagree {
                key: "history.patch_count",
                lang: def.tag
            }),
            "{p:?}"
        );
    }
    // A language that declares `one` but a counted message that has `other` only.
    let l = derived(&EN_DEF, |e| {
        Some(if e.key == "history.patch_count" {
            msg(e.key, "{count} x")
        } else {
            *e
        })
    });
    let p = audit(&[&EN_DEF, l]);
    assert!(
        p.contains(&Problem::PluralFormsDisagree {
            key: "history.patch_count",
            lang: "en"
        }),
        "{p:?}"
    );
}

#[test]
fn markup_in_a_message_of_a_language_is_found() {
    for def in others() {
        let l = derived(def, |e| {
            Some(if e.key == "login.user" {
                msg(e.key, "<b>x</b>")
            } else {
                *e
            })
        });
        let p = audit(&[&EN_DEF, l]);
        assert!(
            p.contains(&Problem::Markup {
                lang: def.tag,
                key: "login.user"
            }),
            "{p:?}"
        );
    }
}

// -------------------------------------------------------------------------- status, shipping, sign-off

/// The rule: a language not signed off is not shipped; a signed-off one carries a sound record (`check_registry`) and the messages as
/// they were signed (a baseline), so that nothing written afterwards is silently counted as reviewed.
#[test]
fn a_language_is_shipped_exactly_when_its_status_says_so_and_a_signed_off_one_has_a_baseline() {
    let want: Vec<&str> = shipped_of(LANGUAGES).iter().map(|l| l.tag).collect();
    let got: Vec<&str> = shipped().iter().map(|d| d.tag).collect();
    assert_eq!(got, want);
    assert!(
        shipped()[0].is_source(),
        "the source language is always first"
    );
    let drafts: Vec<&str> = previewable_of(LANGUAGES).iter().map(|l| l.tag).collect();
    assert_eq!(
        previewable().iter().map(|d| d.tag).collect::<Vec<_>>(),
        drafts
    );
    for def in CATALOGS.iter().copied().filter(|d| d.sign_off().is_some()) {
        let text = baseline_text(def.tag).unwrap_or_else(|| {
            panic!(
                "{}: signed off without a baseline file (locales/{}.signed)",
                def.tag, def.tag
            )
        });
        let baseline =
            planeter_i18n::baseline::parse(text).unwrap_or_else(|e| panic!("{}: {e}", def.tag));
        assert!(!baseline.is_empty(), "{}", def.tag);
        assert!(unreviewed_count(def.tag).is_some(), "{}", def.tag);
        // Every key the baseline lists is a key the language had: none is invented.
        let keys: Vec<&str> = EN_DEF.entries.iter().map(|e| e.key).collect();
        for k in baseline.keys() {
            assert!(
                keys.contains(&k.as_str()),
                "{}: the baseline names `{k}`, which is not a key",
                def.tag
            );
        }
    }
    assert!(
        unreviewed_count("en").is_none(),
        "the source language has no sign-off to count against"
    );
    assert!(unreviewed_count("zz").is_none());
}

#[test]
fn the_baseline_counts_what_is_new_or_changed_since_the_signing_and_nothing_is_counted_as_read() {
    use planeter_i18n::baseline;
    for def in CATALOGS.iter().copied().filter(|d| d.sign_off().is_some()) {
        let signed = baseline::parse(baseline_text(def.tag).unwrap()).unwrap();
        let (unchanged, changed, new) = baseline::counts(def.entries, &signed);
        assert_eq!(unchanged + changed + new, def.entries.len(), "{}", def.tag);
        assert_eq!(
            unreviewed_count(def.tag),
            Some(changed + new),
            "{}",
            def.tag
        );
        // Planted: change one message and add one; both are counted, the rest stay as they were.
        let mut entries: Vec<Entry> = def.entries.to_vec();
        let victim = entries
            .iter()
            .position(|e| {
                signed
                    .get(e.key)
                    .is_some_and(|h| *h == baseline::message_hash(e))
            })
            .expect("an unchanged message");
        entries[victim] = msg(entries[victim].key, "changed after the sign-off");
        entries.push(msg("planted.new", "new after the sign-off"));
        let (u2, c2, n2) = baseline::counts(&entries, &signed);
        assert_eq!(
            (u2, c2, n2),
            (unchanged - 1, changed + 1, new + 1),
            "{}",
            def.tag
        );
    }
}

#[test]
fn the_preview_setting_accepts_a_shipped_language_refuses_a_typo_and_names_no_draft() {
    assert_eq!(previewable().len(), previewable_of(LANGUAGES).len());
    assert_eq!(enable_preview_languages(""), Ok(vec![]));
    assert_eq!(enable_preview_languages(" , "), Ok(vec![]));
    // An earlier setting keeps working: a shipped language is accepted and changes nothing (no second switcher button).
    let other = others()
        .into_iter()
        .find(|d| d.is_shipped())
        .expect("a shipped language besides the source");
    assert_eq!(enable_preview_languages(other.tag), Ok(vec![other.tag]));
    assert_eq!(
        enable_preview_languages(&other.tag.to_uppercase())
            .unwrap()
            .len(),
        1
    );
    let buttons = T::new(Lang::from_def(&EN_DEF)).languages();
    assert_eq!(
        buttons.iter().filter(|l| l.tag == other.tag).count(),
        1,
        "{buttons:?}"
    );
    let err = enable_preview_languages("jp").unwrap_err();
    assert!(
        err.contains("\"jp\"")
            && (err.contains("none at present") || err.contains("can be previewed")),
        "{err}"
    );
    let err = enable_preview_languages(&format!("{},xx", other.tag)).unwrap_err();
    assert!(err.contains("\"xx\""), "{err}");
}
