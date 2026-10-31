//! RFC 012 D-3 / D-5 for Japanese (handoff `japanese-and-switcher-handoff-v1` T1, T6): the draft catalog is
//! complete and consistent with English, the checks are shown able to fail for it, and **a language that is not
//! signed off is not in `shipped()`**.

use planeter_web::catalog::{
    Dir, EN, EN_DEF, Entry, JA_DEF, JA_REVIEWED, JA_SIGNOFF, Key, Lang, LangDef, Plural, Problem,
    T, audit, enable_preview_languages, english_only_keys, previewable, shipped,
};

fn leaked(entries: Vec<Entry>) -> &'static [Entry] {
    Box::leak(entries.into_boxed_slice())
}

/// Japanese with one thing changed: a copy of the catalog for the planted cases.
fn japanese_with(f: impl Fn(&Entry) -> Option<Entry>) -> &'static LangDef {
    let entries: Vec<Entry> = JA_DEF.entries.iter().filter_map(f).collect();
    Box::leak(Box::new(LangDef {
        tag: "ja",
        endonym: "日本語",
        dir: Dir::Ltr,
        plural: ja_plural,
        group_separator: ',',
        decimal_separator: '.',
        entries: leaked(entries),
    }))
}
fn ja_plural(_: u64) -> Plural {
    Plural::Other
}
fn copy(e: &Entry) -> Entry {
    Entry {
        key: e.key,
        one: e.one,
        other: e.other,
    }
}

#[test]
fn the_japanese_draft_is_complete_consistent_and_free_of_markup() {
    assert_eq!(
        audit(&[&EN_DEF, &JA_DEF]),
        vec![],
        "the Japanese catalog has problems"
    );
    // Every English key, in the English order: the reviewers' sheet and a correction map back mechanically.
    // ... except exactly the keys that are English in every language (`ENGLISH_ONLY_KEYS`, empty unless the owner has decided otherwise).
    let only = english_only_keys();
    let en: Vec<&str> = EN_DEF
        .entries
        .iter()
        .map(|e| e.key)
        .filter(|k| !only.contains(k))
        .collect();
    let ja: Vec<&str> = JA_DEF.entries.iter().map(|e| e.key).collect();
    assert_eq!(en, ja, "the Japanese catalog follows the English key order");
    assert_eq!(Key::ALL.len() - only.len(), JA_DEF.entries.len());
}

#[test]
fn japanese_has_one_form_for_every_message_and_keeps_the_grouping_and_the_units() {
    for e in JA_DEF.entries {
        assert_eq!(e.one, e.other, "`{}`: Japanese has no plural", e.key);
    }
    let ja = Lang::from_def(&JA_DEF);
    assert_eq!(ja.number(1200), "1,200");
    assert_eq!(ja.number(1_234_567), "1,234,567");
    let t = T::new(ja);
    assert_eq!(t.size(223), "223 B");
    assert_eq!(t.size(2048), "2.0 KiB");
    assert_eq!(t.size(5 * 1024 * 1024), "5.0 MiB");
    assert_eq!(
        t.msg(Key::HistoryPatchCount).n(1200).text(),
        t.msg(Key::HistoryPatchCount)
            .n(1)
            .text()
            .replace('1', "1,200"),
        "one and many read the same"
    );
    assert_eq!(t.msg(Key::HistoryPatchCount).n(3).text(), "パッチ 3 件");
    // The placeholders are filled in a real sentence.
    assert_eq!(
        t.msg(Key::RepoLocationBranch)
            .with("point", "heads/main")
            .with("branch", "heads/main")
            .text(),
        "heads/main を表示中 · ブランチ heads/main"
    );
}

#[test]
fn the_five_trust_labels_are_five_different_non_empty_labels() {
    let t = T::new(Lang::from_def(&JA_DEF));
    let labels = [
        t.get(Key::AssuranceVerified),
        t.get(Key::AssuranceSealed),
        t.get(Key::AssuranceQueued),
        t.get(Key::AssuranceUnverified),
        t.get(Key::AssuranceVerifyUnavailable),
    ];
    for l in &labels {
        assert!(!l.trim().is_empty());
    }
    let mut sorted = labels.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        5,
        "two trust labels read the same: {labels:?}"
    );
    // The three words that must never blur: the queued label does not claim a signature, the signed label does
    // not claim a verification that ran just now, the verified label does not claim a signature.
    assert!(!labels[2].contains("署名済み") || labels[2].contains("待ち"));
}

// -------------------------------------------------------------------------- the checks can fail (planted)

#[test]
fn a_key_missing_in_japanese_is_found() {
    let l = japanese_with(|e| (e.key != "login.failed").then(|| copy(e)));
    let p = audit(&[&EN_DEF, l]);
    assert!(
        p.contains(&Problem::Missing {
            lang: "ja",
            key: "login.failed"
        }),
        "{p:?}"
    );
}

#[test]
fn a_changed_placeholder_in_japanese_is_found() {
    // `title.change` takes {target}, {repo}, {product}: rename one.
    let l = japanese_with(|e| {
        Some(if e.key == "title.change" {
            Entry {
                key: e.key,
                one: "変更 {goal} · {repo} · {product}",
                other: "変更 {goal} · {repo} · {product}",
            }
        } else {
            copy(e)
        })
    });
    let p = audit(&[&EN_DEF, l]);
    assert!(
        p.contains(&Problem::PlaceholdersDiffer {
            key: "title.change",
            lang: "ja"
        }),
        "{p:?}"
    );
}

#[test]
fn two_plural_forms_in_japanese_are_found() {
    let l = japanese_with(|e| {
        Some(if e.key == "history.patch_count" {
            Entry {
                key: e.key,
                one: "パッチ {count} 件目",
                other: "パッチ {count} 件",
            }
        } else {
            copy(e)
        })
    });
    let p = audit(&[&EN_DEF, l]);
    assert!(
        p.contains(&Problem::PluralFormsDisagree {
            key: "history.patch_count",
            lang: "ja"
        }),
        "{p:?}"
    );
}

#[test]
fn markup_in_a_japanese_message_is_found() {
    let l = japanese_with(|e| {
        Some(if e.key == "login.user" {
            Entry {
                key: e.key,
                one: "<b>ユーザー名</b>",
                other: "<b>ユーザー名</b>",
            }
        } else {
            copy(e)
        })
    });
    let p = audit(&[&EN_DEF, l]);
    assert!(
        p.contains(&Problem::Markup {
            lang: "ja",
            key: "login.user"
        }),
        "{p:?}"
    );
}

// -------------------------------------------------------------------------- signed off, shipped

/// The rule: a language not signed off is not in `shipped()`. `JA_REVIEWED` is the sign-off; it is `true` only
/// together with a recorded sign-off (`JA_SIGNOFF`), and `shipped()` lists Japanese exactly then.
#[test]
fn a_language_is_shipped_exactly_when_it_is_signed_off_and_the_record_is_kept_with_it() {
    let listed = shipped().iter().any(|d| d.tag == "ja");
    assert_eq!(
        listed, JA_REVIEWED,
        "shipped() must list Japanese if and only if JA_REVIEWED"
    );
    assert_eq!(
        JA_REVIEWED,
        JA_SIGNOFF.is_some(),
        "JA_REVIEWED and the recorded sign-off are set together"
    );
    assert_eq!(shipped()[0].tag, EN.tag(), "English is always first");
    assert_eq!(shipped().len(), if JA_REVIEWED { 2 } else { 1 });
}

/// The record names who, when, which commit and which sheet (the fingerprint of the wording that was reviewed).
#[test]
fn the_signoff_record_names_who_when_the_commit_and_the_sheet() {
    let record = JA_SIGNOFF.expect("Japanese is signed off");
    for needle in ["owner", "2026-10-31", "commit ", "fingerprint "] {
        assert!(
            record.contains(needle),
            "{needle:?} missing from {record:?}"
        );
    }
    // The fingerprint is sixteen hex digits, and the record does not carry a placeholder.
    let fp = record.split("fingerprint ").last().unwrap();
    let hex: String = fp.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(hex.len(), 16, "{record:?}");
    assert!(!record.contains("PLACEHOLDER"), "{record:?}");
}

#[test]
fn the_preview_setting_accepts_a_shipped_language_refuses_a_typo_and_names_no_draft() {
    assert!(
        previewable().is_empty(),
        "no draft language exists at present"
    );
    assert_eq!(enable_preview_languages(""), Ok(vec![]));
    assert_eq!(enable_preview_languages(" , "), Ok(vec![]));
    // An earlier setting keeps working: a shipped language is accepted and changes nothing (no second switcher button).
    assert_eq!(enable_preview_languages("ja"), Ok(vec!["ja"]));
    assert_eq!(enable_preview_languages("JA, en"), Ok(vec!["ja", "en"]));
    assert_eq!(Lang::from_tag("ja").unwrap().tag(), "ja");
    let buttons = T::new(EN).languages();
    assert_eq!(
        buttons.iter().filter(|l| l.tag == "ja").count(),
        1,
        "{buttons:?}"
    );
    let err = enable_preview_languages("jp").unwrap_err();
    assert!(
        err.contains("\"jp\"") && err.contains("none at present"),
        "{err}"
    );
    let err = enable_preview_languages("ja,xx").unwrap_err();
    assert!(err.contains("\"xx\""), "{err}");
}
