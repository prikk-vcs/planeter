//! RFC 012 D-4, T4: the words the plain-word rewrite retired do not come back. A user-facing message of any
//! shipped language must not contain them, except for the entries of `tests/retired_words.allow` (each with a
//! reason). The check is shown able to fail below (planted cases); the review request lists the source
//! mutation that was applied and restored.

use std::path::PathBuf;

use planeter_web::catalog::{Dir, Entry, LangDef, Plural, shipped};

/// Words that may not appear in a message at all (matched as whole words, ignoring case).
const WORDS: &[&str] = &[
    "unsealed",
    "forge-approved",
    "prikk-verified",
    "not_evaluated",
    "evaluated",
    "refs",
    "SSO",
    // IS-49: the verification page's "N of M sessions verified" is gone; a user message does not say "session"
    // (the sign-in session never appears in a message either).
    "session",
    "sessions",
];
/// RFC 005 part 5 T0 (decision D-38): **change** is the glossary's word for a recorded unit of history (prikk's block). What a person
/// proposes is a **proposal**. These phrases are the old uses of "change" for a proposal; a message may not contain them. Whole
/// phrases, not the word: "a recorded change" in the history pages is right, "open changes" is not.
const CHANGE_MEANING_PROPOSAL: &[&str] = &[
    "open change",
    "open changes",
    "new change",
    "new changes",
    "proposed change",
    "proposed changes",
    "propose changes",
    "close a change",
    "finished changes",
    "change of your own",
    "change with that id",
    "your change",
    "your changes",
    "that change has landed",
    "an existing change",
];

/// Old navigation words that may not be a whole message (a label); `Verification` and `error code` are fine.
const LABELS: &[&str] = &["code", "history", "verify"];

#[derive(Debug, PartialEq, Eq)]
struct Allowed {
    key: String,
    term: String,
}

fn parse_allow(src: &str) -> Result<Vec<Allowed>, String> {
    let mut out = Vec::new();
    for line in src.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, " :: ").collect();
        if parts.len() != 3 || parts.iter().any(|p| p.trim().is_empty()) {
            return Err(format!("needs key :: term :: reason: {line}"));
        }
        out.push(Allowed {
            key: parts[0].trim().to_owned(),
            term: parts[1].trim().to_owned(),
        });
    }
    Ok(out)
}

/// The text without its `{placeholders}` (their names are code, not words).
fn without_placeholders(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn contains_word(text: &str, word: &str) -> bool {
    let (text, word) = (text.to_lowercase(), word.to_lowercase());
    // Only ASCII letters and digits join a word: Japanese text has no spaces, so `SSOでログイン` still contains `SSO`.
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

/// The retired terms one message form uses.
fn terms_in(text: &str) -> Vec<&'static str> {
    let text = without_placeholders(text);
    let mut out: Vec<&'static str> = WORDS
        .iter()
        .copied()
        .filter(|w| contains_word(&text, w))
        .collect();
    out.extend(LABELS.iter().copied().filter(|l| text.trim() == *l));
    out.extend(
        CHANGE_MEANING_PROPOSAL
            .iter()
            .copied()
            .filter(|p| contains_word(&text, p)),
    );
    out
}

/// (problems, stale allowlist entries) for the given languages.
fn check(langs: &[&LangDef], allow: &[Allowed]) -> (Vec<String>, Vec<String>) {
    let mut problems = Vec::new();
    let mut used = vec![false; allow.len()];
    for lang in langs {
        for e in lang.entries {
            let mut seen: Vec<&str> = Vec::new();
            for form in [e.one, e.other] {
                for term in terms_in(form) {
                    if seen.contains(&term) {
                        continue;
                    }
                    seen.push(term);
                    match allow.iter().position(|a| a.key == e.key && a.term == term) {
                        Some(i) => used[i] = true,
                        None => problems.push(format!(
                            "{} `{}` uses the retired {term:?}: {form:?}",
                            lang.tag, e.key
                        )),
                    }
                }
            }
        }
    }
    let stale = allow
        .iter()
        .zip(used)
        .filter(|(_, u)| !u)
        .map(|(a, _)| {
            format!(
                "`{}` :: {:?} is allowed but no message uses it",
                a.key, a.term
            )
        })
        .collect();
    (problems, stale)
}

fn planted(entries: Vec<Entry>) -> &'static LangDef {
    fn one_other(n: u64) -> Plural {
        if n == 1 { Plural::One } else { Plural::Other }
    }
    Box::leak(Box::new(LangDef {
        tag: "planted",
        endonym: "planted",
        dir: Dir::Ltr,
        plural: one_other,
        group_separator: ',',
        decimal_separator: '.',
        entries: Box::leak(entries.into_boxed_slice()),
    }))
}

fn message(key: &'static str, text: &'static str) -> Entry {
    Entry {
        key,
        one: text,
        other: text,
    }
}

fn allow_file() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/retired_words.allow");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn no_shipped_message_uses_a_retired_word_and_the_allowlist_has_no_dead_lines() {
    let allow = parse_allow(&allow_file()).unwrap();
    let (problems, stale) = check(shipped(), &allow);
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(stale, Vec::<String>::new());
}

#[test]
fn every_retired_word_is_found_wherever_it_hides() {
    for (text, term) in [
        ("There are no unsealed changes.", "unsealed"),
        ("Unsealed work", "unsealed"),
        ("forge-approved, unsealed", "forge-approved"),
        ("prikk-verified", "prikk-verified"),
        ("The stage is not_evaluated.", "not_evaluated"),
        ("evaluated", "evaluated"),
        ("Show refs · {repo}", "refs"),
        ("Sign in with SSO", "SSO"),
        ("3 of 4 sessions verified.", "sessions"),
        ("1 of 1 session verified.", "session"),
        ("sign in with sso", "SSO"),
        // "change" for a proposal (D-38)
        ("You already have 3 open changes.", "open changes"),
        ("Open a new change instead.", "new change"),
        ("Close one of your open changes.", "open changes"),
        ("You may not propose changes to it.", "propose changes"),
        // a whole message that is an old navigation label
        ("code", "code"),
        ("history", "history"),
        ("verify", "verify"),
    ] {
        let l = planted(vec![message("planted.key", text)]);
        let (problems, _) = check(&[l], &[]);
        assert!(
            problems.iter().any(|p| p.contains(&format!("{term:?}"))),
            "{text:?} should be found as {term:?}: {problems:?}"
        );
    }
    // Hidden in the other plural form only.
    let l = planted(vec![Entry {
        key: "planted.count",
        one: "{count} change",
        other: "{count} unsealed changes",
    }]);
    assert_eq!(check(&[l], &[]).0.len(), 1);
}

#[test]
fn ordinary_words_and_placeholders_are_not_mistaken_for_retired_ones() {
    for text in [
        "History",
        "Verification",
        "Files",
        "Branches and tags",
        "The error code is shown.",
        "Use {refs} here",
        "Your prefs are saved.",
        "Sign in with single sign-on",
        "The checks were reevaluated by hand",
        "This history is empty.",
        // "change" in its glossary sense, a recorded unit of history, is not a proposal
        "A recorded change.",
        "There are no signed changes on this branch yet.",
        "This change was approved on this server.",
        "A rollback is a change that undoes an earlier change.",
        "No branch has changed.",
    ] {
        let l = planted(vec![message("planted.key", text)]);
        let (problems, _) = check(&[l], &[]);
        // A whole-word match, not a substring match ("reevaluated", "prefs"), and placeholder names are code.
        assert_eq!(problems, Vec::<String>::new(), "{text:?}");
    }
}

#[test]
fn an_allowlist_entry_silences_exactly_its_message_and_term_and_a_dead_one_fails() {
    let l = planted(vec![
        message("planted.a", "The command is prikk-verified."),
        message("planted.b", "Also prikk-verified."),
    ]);
    let allow =
        parse_allow("planted.a :: prikk-verified :: the name of prikk's own data\n").unwrap();
    let (problems, stale) = check(&[l], &allow);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("planted.b"));
    assert!(stale.is_empty());
    // An entry nothing needs.
    let allow = parse_allow("planted.zzz :: refs :: gone\n").unwrap();
    let (_, stale) = check(&[planted(vec![message("planted.ok", "Fine.")])], &allow);
    assert_eq!(stale.len(), 1, "{stale:?}");
    // An entry without a reason is refused.
    assert!(parse_allow("planted.a :: refs\n").is_err());
    assert!(parse_allow("planted.a :: refs :: \n").is_err());
}

// ---------------------------------------------------------------------------------------------------------
// Japanese (handoff `japanese-and-switcher-handoff-v1` T6): the English retired words may not come back inside a
// Japanese message either, and a short list of Japanese terms is forbidden — the draft's own proposal, for the
// reviewers to amend. Japanese has no word boundaries, so these are substring matches outside placeholders.

/// (term, why it is forbidden). Proposed by the dev team; the reviewers amend it with their sheet.
const JA_FORBIDDEN: &[(&str, &str)] = &[
    // The retired private words, in their Japanese forms: the interface does not say "seal".
    (
        "シール",
        "the loan word for seal / sealed: the glossary retired `sealed`; say \"signed into the history\" (署名して履歴に記録)",
    ),
    ("封印", "the literal translation of seal: same reason"),
    ("未封印", "the literal translation of unsealed: same reason"),
    (
        "フォージ",
        "forge-approved is retired; say \"approved on this server\" (このサーバーで承認)",
    ),
    (
        "評価済み",
        "`evaluated` is retired because it does not mean \"passed\"; the status is 完了 (completed)",
    ),
    ("ブロック", "a block is a \"change\" (変更) for the reader"),
    (
        "リファレンス",
        "refs are \"branches and tags\" (ブランチとタグ)",
    ),
    (
        "セッション",
        "IS-49: the sessions count was removed; no message says session",
    ),
    // RFC 012 D-6: no blaming, no gendered wording.
    (
        "あなたの間違い",
        "blames the reader: say what happened and what to do",
    ),
    ("あなたが間違", "blames the reader"),
    ("ユーザーの過ち", "blames the reader"),
    (
        "不正な",
        "implies wrongdoing; say \"not correct\" / \"not recognised\"",
    ),
    // RFC 005 part 5 T0 (D-38): 変更 is the one word for a recorded unit of history; a proposal is 提案.
    (
        "開いている変更",
        "a proposal is 提案; 変更 is a recorded unit of history (D-38)",
    ),
    ("新しい変更", "a proposal is 提案 (D-38)"),
    ("提案された変更", "say 提案 (D-38)"),
    ("変更を提案", "say 提案する (D-38)"),
    ("変更を閉じ", "a proposal is closed: 提案を閉じ (D-38)"),
    ("変更を開", "a proposal is opened: 提案を開 (D-38)"),
    ("自分の変更", "say 自分の提案 (D-38)"),
    ("完了した変更", "say 完了した提案 (D-38)"),
    ("その変更は", "a proposal is 提案 (D-38)"),
    ("ID の変更", "a proposal is 提案 (D-38)"),
    ("彼女", "gendered wording (D-6)"),
    ("彼は", "gendered wording (D-6)"),
    ("男性", "gendered wording (D-6)"),
    ("女性", "gendered wording (D-6)"),
];

/// The forbidden Japanese terms one message form uses.
fn ja_terms_in(text: &str) -> Vec<&'static str> {
    let text = without_placeholders(text);
    JA_FORBIDDEN
        .iter()
        .filter(|(term, _)| text.contains(term))
        .map(|(term, _)| *term)
        .collect()
}

fn ja_check(langs: &[&LangDef]) -> Vec<String> {
    let mut problems = Vec::new();
    for lang in langs {
        for e in lang.entries {
            for form in [e.one, e.other] {
                for term in ja_terms_in(form) {
                    problems.push(format!(
                        "{} `{}` uses the forbidden {term:?}: {form:?}",
                        lang.tag, e.key
                    ));
                }
            }
        }
    }
    problems
}

#[test]
fn the_japanese_draft_uses_no_retired_english_word_and_no_forbidden_japanese_term() {
    let (problems, _) = check(&[&planeter_web::catalog::JA_DEF], &[]);
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(
        ja_check(&[&planeter_web::catalog::JA_DEF]),
        Vec::<String>::new()
    );
}

#[test]
fn every_forbidden_japanese_term_is_found_and_ordinary_words_are_not_mistaken_for_one() {
    for (term, reason) in JA_FORBIDDEN {
        assert!(!reason.trim().is_empty());
        let l = planted(vec![message(
            "planted.key",
            Box::leak(format!("これは{term}です。").into_boxed_str()),
        )]);
        let found = ja_check(&[l]);
        assert!(
            found.iter().any(|p| p.contains(&format!("{term:?}"))),
            "{term:?} should be found: {found:?}"
        );
    }
    // The words the glossary does use, and placeholder names, are fine.
    for text in [
        "検証済み",
        "署名して履歴に記録済み",
        "承認済み・署名待ち",
        "完了",
        "変更 {target}",
        "{ref}",
        "ブランチとタグ",
        "シングルサインオンでログイン",
        "このサーバーで承認されました。",
        "記録された変更です。",
        "この変更は、このサーバーで承認されました。",
        "ロールバックとは、以前の変更を取り消す変更のことです。",
        "このブランチには、署名済みの変更がまだありません。",
        "開いている提案がすでに{used}件あり、上限は{limit}件です。",
    ] {
        let l = planted(vec![message("planted.key", text)]);
        assert_eq!(ja_check(&[l]), Vec::<String>::new(), "{text:?}");
    }
    // A term hidden in the other form only.
    let l = planted(vec![Entry {
        key: "planted.count",
        one: "{count} 件",
        other: "{count} 件の封印",
    }]);
    assert_eq!(ja_check(&[l]).len(), 1);
}

#[test]
fn the_english_retired_words_are_found_in_a_japanese_message_too() {
    for (text, term) in [
        ("これは unsealed です", "unsealed"),
        ("SSO でログイン", "SSO"),
        ("SSOでログイン", "SSO"),
        ("prikk-verified の変更", "prikk-verified"),
    ] {
        let l = planted(vec![message("planted.key", text)]);
        let (problems, _) = check(&[l], &[]);
        assert!(
            problems.iter().any(|p| p.contains(&format!("{term:?}"))),
            "{text:?}: {problems:?}"
        );
    }
}

// ------------------------------------------------------------------------------------------------ "approved" (RFC 005 part 6, T5)

/// *Approve* belongs to a **reviewer's decision**. A sentence about a proposal that says it without being about a decision —
/// "Approved, awaiting signature" beside a proposal — would tell a reader that someone approved the proposal when nobody
/// did: that badge is about work held on this server, and the owner is asked about its wording separately.
const APPROVE_WORDS: &[&str] = &[
    "approve",
    "approved",
    "approves",
    "approval",
    "approvals",
    "承認",
];

/// The messages whose subject is a reviewer's decision, by key prefix.
const DECISION_KEYS: &[&str] = &[
    "review.",
    "decision.",
    "error.self_approval",
    "error.forbidden_decision",
    "error.decisions_closed",
    "error.revision_not_current",
    "error.rate_decisions",
    "error.invalid_request.decision_hint",
];

/// Messages that say *approve* about a proposal outside the decision messages: `lang tag key`.
fn approve_outside_decisions(langs: &[&LangDef]) -> Vec<String> {
    let mut out = Vec::new();
    for lang in langs {
        for e in lang.entries {
            if DECISION_KEYS.iter().any(|p| e.key.starts_with(p)) {
                continue;
            }
            for form in [e.one, e.other] {
                let text = without_placeholders(form);
                let about_a_proposal = e.key.starts_with("proposal")
                    || contains_word(&text, "proposal")
                    || text.contains("提案");
                let says_approve = APPROVE_WORDS.iter().any(|w| {
                    if w.is_ascii() {
                        contains_word(&text, w)
                    } else {
                        text.contains(w)
                    }
                });
                if about_a_proposal && says_approve {
                    out.push(format!(
                        "{} `{}` says approve about a proposal: {form:?}",
                        lang.tag, e.key
                    ));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn no_shipped_sentence_about_a_proposal_says_approved_unless_it_is_about_a_reviewers_decision() {
    assert_eq!(approve_outside_decisions(shipped()), Vec::<String>::new());
}

#[test]
fn the_approved_rule_finds_the_planted_badge_beside_a_proposal_and_lets_decisions_through() {
    for text in [
        "Approved, awaiting signature",
        "This proposal was approved on this server.",
        "Your proposal has approval.",
        "この提案は承認されました。",
    ] {
        let l = planted(vec![message(
            "proposal.planted",
            Box::leak(text.to_owned().into_boxed_str()),
        )]);
        assert_eq!(approve_outside_decisions(&[l]).len(), 1, "{text:?}");
    }
    // A sentence that is not about a proposal is not this rule's (the owner-approved badge on a patch stays as it is).
    let badge = planted(vec![message(
        "assurance.queued",
        "Approved, awaiting signature",
    )]);
    assert_eq!(approve_outside_decisions(&[badge]), Vec::<String>::new());
    // Decisions are exactly what the word is for.
    for (key, text) in [
        ("review.line.approved", "Approved by 2 people"),
        (
            "review.decision.approve",
            "{who} approved revision {revision}",
        ),
        ("decision.form.approve", "Approve"),
        (
            "error.self_approval.message",
            "You cannot approve work that you sent yourself.",
        ),
        (
            "review.means.approved",
            "Approved here means reviewers said they are happy with revision {revision}.",
        ),
        (
            "review.decision.approve",
            "{who} さんがリビジョン {revision} を承認しました",
        ),
    ] {
        let l = planted(vec![message(
            key,
            Box::leak(text.to_owned().into_boxed_str()),
        )]);
        assert_eq!(
            approve_outside_decisions(&[l]),
            Vec::<String>::new(),
            "{key}"
        );
    }
}
