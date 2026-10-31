//! T4: the messages. Both languages hold exactly the keys the program names, with the same placeholders; no retired word is said;
//! no markup or colour is in a sentence; the language choice follows `--lang`, then `LC_ALL`, `LC_MESSAGES`, `LANG`, and Japanese
//! (a draft) is never chosen from the environment until its reviewers have signed it off.

use std::collections::BTreeSet;

use planeter_client::catalog::{JA_REVIEWED, Key, Lang, placeholders, raw, size, t};

#[test]
fn every_key_has_a_message_in_every_language_and_no_language_has_a_stray_one() {
    for lang in Lang::ALL {
        let ids: Vec<&str> = lang.entries().iter().map(|e| e.key).collect();
        let unique: BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "{}: a key is listed twice",
            lang.tag()
        );
        let want: BTreeSet<&str> = Key::ALL.iter().map(|k| k.id()).collect();
        assert_eq!(
            unique,
            want,
            "{}: the table and the keys differ",
            lang.tag()
        );
    }
}

#[test]
fn a_message_uses_the_same_placeholders_in_every_language() {
    for key in Key::ALL {
        let en = raw(Lang::En, *key).unwrap();
        let ja = raw(Lang::Ja, *key).unwrap();
        let (mut a, mut b) = (placeholders(en), placeholders(ja));
        a.sort_unstable();
        a.dedup();
        b.sort_unstable();
        b.dedup();
        assert_eq!(a, b, "`{}`: English {a:?}, Japanese {b:?}", key.id());
    }
}

/// Whole-word, case-insensitive match; `-` and `_` are not word characters.
fn has_word(text: &str, word: &str) -> bool {
    let (text, word) = (text.to_lowercase(), word.to_lowercase());
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
    })
}

/// The words the project retired (docs/glossary.md): the CLI says *proposal*, *revision*, *held*, *trust a key* — never these.
/// The option `--no-adopt` is named by decision (RFC 004 Q-2) and is taken out before the scan: it is a flag, not a sentence.
const RETIRED: &[&str] = &[
    "seal",
    "sealed",
    "unsealed",
    "adopt",
    "adopted",
    "intent",
    "forge-approved",
    "evaluated",
    "session",
    "sessions",
    "change set",
];

fn retired_found(text: &str) -> Vec<&'static str> {
    let text = text.replace("--no-adopt", "");
    RETIRED
        .iter()
        .copied()
        .filter(|w| has_word(&text, w))
        .collect()
}

#[test]
fn no_message_says_a_retired_word() {
    for lang in [Lang::En] {
        let mut problems = Vec::new();
        for e in lang.entries() {
            for w in retired_found(e.text) {
                problems.push(format!("{} `{}` says {w:?}", lang.tag(), e.key));
            }
        }
        assert_eq!(problems, Vec::<String>::new());
    }
}

#[test]
fn the_retired_word_check_finds_a_planted_word_and_lets_the_flag_through() {
    assert_eq!(
        retired_found("The work was sealed in the history."),
        ["sealed"]
    );
    assert_eq!(
        retired_found("It adopts nothing; adopt it later."),
        ["adopt"]
    );
    assert_eq!(retired_found("An intent is open."), ["intent"]);
    assert!(retired_found("Use --no-adopt to trust nothing.").is_empty());
    assert!(retired_found("The proposal is held for review; trust a key.").is_empty());
}

/// Japanese terms the draft must not use (the server's glossary, §5): a word with two roles, the retired private words.
const JA_FORBIDDEN: &[&str] = &[
    "管理者",
    "シール",
    "封印",
    "未封印",
    "フォージ",
    "評価済み",
    "ブロック",
    "セッション",
    "検査",
];

#[test]
fn the_japanese_draft_uses_none_of_the_terms_the_glossary_forbids() {
    let mut problems = Vec::new();
    for e in Lang::Ja.entries() {
        for w in JA_FORBIDDEN {
            if e.text.contains(w) {
                problems.push(format!("`{}` uses {w}", e.key));
            }
        }
        // Full-width punctuation follows Japanese text.
        for (i, c) in e.text.char_indices() {
            if matches!(c, ',' | '?' | '!')
                && e.text[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|p| !p.is_ascii())
            {
                problems.push(format!(
                    "`{}` has a half-width {c:?} after Japanese text",
                    e.key
                ));
            }
        }
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn no_message_carries_markup_or_colour() {
    for lang in Lang::ALL {
        for e in lang.entries() {
            assert!(
                !e.text.contains('\u{1b}'),
                "{} `{}`: an escape sequence",
                lang.tag(),
                e.key
            );
            assert!(
                !e.text.contains("<b>") && !e.text.contains("</") && !e.text.contains("<a "),
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
    for e in Lang::En.entries().iter().filter(|e| !exempt(e.key)) {
        let last = e.text.trim_end().chars().last().unwrap();
        assert!(
            matches!(last, '.' | '…' | ':' | '?' | '}'),
            "`{}` ends with {last:?}: {}",
            e.key,
            e.text
        );
    }
}

#[test]
fn a_filled_message_has_no_placeholder_left_and_a_missing_key_is_a_visible_marker() {
    let line = t(Lang::En, Key::ProposeDoneNew)
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
    assert!(t(Lang::En, Key::ProposeDoneNew).text().contains("{id}"));
}

#[test]
fn the_language_is_the_flag_then_the_environment_and_english_when_neither_says_japanese() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    // Japanese is signed off (RFC 012 D-3f): the environment can choose it.
    const { assert!(JA_REVIEWED) };
    assert_eq!(Lang::from_environment(env(&[])), Lang::En);
    assert_eq!(
        Lang::from_environment(env(&[("LANG", "ja_JP.UTF-8")])),
        Lang::Ja
    );
    assert_eq!(
        Lang::from_environment(env(&[("LC_MESSAGES", "ja_JP.UTF-8")])),
        Lang::Ja
    );
    assert_eq!(Lang::from_environment(env(&[("LC_ALL", "ja")])), Lang::Ja);
    // The first of LC_ALL, LC_MESSAGES, LANG that is set and not empty decides.
    assert_eq!(
        Lang::from_environment(env(&[("LC_ALL", "ja_JP.UTF-8"), ("LANG", "en_US.UTF-8")])),
        Lang::Ja
    );
    assert_eq!(
        Lang::from_environment(env(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "ja_JP.UTF-8")])),
        Lang::En
    );
    assert_eq!(
        Lang::from_environment(env(&[("LC_ALL", ""), ("LANG", "ja_JP.UTF-8")])),
        Lang::Ja,
        "an empty variable is skipped"
    );
    // Anything else is English: another language, the C locale, a name that only starts like ja.
    for other in ["C", "POSIX", "fr_FR.UTF-8", "jam_XX", "de"] {
        let pairs: &'static [(&'static str, &'static str)] =
            Box::leak(vec![("LANG", other)].into_boxed_slice());
        assert_eq!(Lang::from_environment(env(pairs)), Lang::En, "{other}");
    }
    // --lang is the flag's own function; it always wins over the environment (the command line applies it first).
    assert_eq!(Lang::from_flag("ja"), Some(Lang::Ja));
    assert_eq!(Lang::from_flag("EN"), Some(Lang::En));
    assert_eq!(Lang::from_flag("fr"), None);
}

#[test]
fn sizes_are_written_for_a_person() {
    assert_eq!(size(12), "12 B");
    assert_eq!(size(1536), "1.5 KiB");
    assert_eq!(size(5 << 20), "5.0 MiB");
    assert_eq!(size(3 << 30), "3.0 GiB");
}

// ---------------------------------------------------------------- the final check (2026-10-31): one wording, one standard

/// The rules of the server's `glossary.md` section 5.3 that apply to the program's sentences.
fn standard_findings(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    if text
        .replace("シングルサインオン", "")
        .contains("サインイン")
    {
        found.push("サインイン (sign in is ログイン)");
    }
    if text.contains("クローズ") {
        found.push("クローズ (closed is 閉じている)");
    }
    if text.contains("断りま") || text.contains("断られ") || text.contains("お断り") {
        found.push("断る (refused is 拒否)");
    }
    if text.contains("変更されていません") || text.contains("何も変更していません")
    {
        found.push("変更 as the verb change (変わる / 変える)");
    }
    if text.contains("確認待ち") {
        found.push("確認待ち (confirm is 確定)");
    }
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if matches!(c, '(' | ')')
            && [i.checked_sub(1), Some(i + 1)]
                .into_iter()
                .flatten()
                .filter_map(|j| chars.get(j))
                .any(|n| !n.is_ascii())
        {
            found.push("a half-width bracket next to Japanese");
            break;
        }
    }
    for (i, c) in chars.iter().enumerate() {
        let counter = *c == '件' || (*c == '時' && chars.get(i + 1) == Some(&'間'));
        if counter && i > 0 && (chars[i - 1].is_ascii_digit() || chars[i - 1] == '}') {
            found.push("no space between a numeral or placeholder and its counter");
            break;
        }
    }
    found
}

#[test]
fn the_japanese_catalog_keeps_the_final_standards() {
    let mut broken = Vec::new();
    for e in Lang::Ja.entries() {
        for rule in standard_findings(e.text) {
            broken.push(format!("`{}`: {rule}", e.key));
        }
    }
    assert_eq!(broken, Vec::<String>::new());
}

#[test]
fn each_standard_finds_a_planted_breach_and_lets_the_right_form_through() {
    for (bad, rule) in [
        ("サインインしてください。", "サインイン"),
        ("サーバーが断りました：{message}", "断る"),
        ("断られた値です。", "断る"),
        ("ブランチは変更されていません。", "変更 as the verb"),
        ("履歴(読む範囲)", "half-width bracket"),
        ("{count}件", "counter"),
    ] {
        let found = standard_findings(bad);
        assert!(
            found.iter().any(|f| f.contains(rule)),
            "{bad:?} should break {rule}: {found:?}"
        );
    }
    for ok in [
        "ログイン",
        "サーバーが拒否しました：{message}",
        "ブランチは変わっていません。",
        "{count} 件",
        "履歴（読む範囲）",
    ] {
        assert_eq!(standard_findings(ok), Vec::<&str>::new(), "{ok:?}");
    }
}

/// Two of the final handoff's corrections are the program's own: the server's refusal is 拒否, in both of its sentences.
#[test]
fn the_servers_refusal_is_reject_in_both_sentences() {
    assert_eq!(
        raw(Lang::Ja, Key::ServerRefused),
        Some("サーバーが拒否しました：{message} {hint}")
    );
    assert_eq!(
        raw(Lang::Ja, Key::ServerRefusedPlain),
        Some(
            "サーバーがリクエストを拒否しました（{status}）が、理由は示されませんでした。もう一度お試しください。続く場合は、このサーバーを運用している人に伝えてください。"
        )
    );
}

/// After the sign-off no sentence of the program calls Japanese a draft or a preview (the help line said so until 2026-10-31).
#[test]
fn no_message_calls_japanese_a_draft_and_the_help_still_names_the_language_option() {
    for lang in Lang::ALL {
        for e in lang.entries() {
            for word in ["draft", "preview", "草案", "プレビュー", "未確認"] {
                assert!(
                    !e.text.to_lowercase().contains(word),
                    "`{}` ({}) says {word:?}",
                    e.key,
                    lang.tag()
                );
            }
        }
    }
    assert!(
        raw(Lang::En, Key::HelpMain)
            .unwrap()
            .contains("--lang en|ja        the language of the messages\n")
    );
    assert!(
        raw(Lang::Ja, Key::HelpMain)
            .unwrap()
            .contains("--lang en|ja        メッセージの言語\n")
    );
}
