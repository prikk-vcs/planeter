//! The Japanese draft's consistency rules (D-50, the architect's pre-review): one word for one thing, so a reader of the
//! Japanese cannot mistake who is meant. The reviewers may overrule the words; the rules keep the draft consistent with itself
//! until they do. (The forbidden words `サーバーの管理者` and `検査` are in `tests/retired_words.rs`, with their planted cases.)
//!
//! * `管理者` is the administrator of a repository and never stands alone: it always follows `リポジトリの`.
//! * `点検` is only what prikk's `doctor` does: it occurs in exactly the two operator messages about it.

use planeter_web::catalog::JA_DEF;

/// Every `管理者` in `text` that is not directly preceded by `リポジトリの`.
fn bare_administrators(text: &str) -> usize {
    text.match_indices("管理者")
        .filter(|(at, _)| !text[..*at].ends_with("リポジトリの"))
        .count()
}

#[test]
fn administrator_is_only_ever_the_administrator_of_a_repository() {
    let mut bare = Vec::new();
    for e in JA_DEF.entries {
        for form in [e.one, e.other] {
            if bare_administrators(form) > 0 {
                bare.push(format!("`{}`: {form}", e.key));
            }
        }
    }
    assert_eq!(bare, Vec::<String>::new());
}

#[test]
fn the_administrator_rule_finds_a_bare_word_and_lets_a_qualified_one_through() {
    for text in [
        "コメントを非表示にできるのは管理者だけです。",
        "このサーバーの管理者に問い合わせてください。",
        "あなたがその管理者になり、読めるようになります。",
        "リポジトリの管理者と管理者に依頼してください。",
    ] {
        assert!(bare_administrators(text) > 0, "{text:?}");
    }
    for text in [
        "できるのはリポジトリの管理者だけです。",
        "あなたはこのリポジトリの管理者なので読めます。",
        "このサーバーを運用している人に問い合わせてください。",
    ] {
        assert_eq!(bare_administrators(text), 0, "{text:?}");
    }
}

#[test]
fn doctors_word_is_used_only_for_what_doctor_does() {
    let mut keys: Vec<&str> = JA_DEF
        .entries
        .iter()
        .filter(|e| e.one.contains("点検") || e.other.contains("点検"))
        .map(|e| e.key)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "recovery.blocker.doctor_problems",
            "recovery.note.unreached_objects"
        ]
    );
}

// ---------------------------------------------------------------- the final check (2026-10-31): one wording, one standard

/// The rules of `glossary.md` section 5.3, one message at a time: the name of every rule the text breaks.
fn standard_findings(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    // Sign in is ログイン; シングルサインオン keeps its own name.
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
    // The verb change is 変わる / 変える; 変更 is the noun.
    if text.contains("変更されていません") || text.contains("何も変更していません")
    {
        found.push("変更 as the verb change (変わる / 変える)");
    }
    if text.contains("確認待ち") {
        found.push("確認待ち (confirm is 確定)");
    }
    // Full-width brackets next to Japanese.
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if matches!(c, '(' | ')') {
            let next_to_japanese = [i.checked_sub(1), Some(i + 1)]
                .into_iter()
                .flatten()
                .filter_map(|j| chars.get(j))
                .any(|n| !n.is_ascii());
            if next_to_japanese {
                found.push("a half-width bracket next to Japanese");
                break;
            }
        }
    }
    // A space between a numeral or placeholder and its counter.
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
    for e in JA_DEF.entries {
        for form in [e.one, e.other] {
            for rule in standard_findings(form) {
                broken.push(format!("`{}`: {rule}", e.key));
            }
        }
        if e.key == "explain.queued" && (e.one.contains("まだ") || e.other.contains("まだ")) {
            broken.push(
                "`explain.queued` promises a signature (まだ); the English has no \"yet\" (D-50)"
                    .to_owned(),
            );
        }
    }
    assert_eq!(broken, Vec::<String>::new());
}

#[test]
fn each_standard_finds_a_planted_breach_and_lets_the_right_form_through() {
    for (bad, rule) in [
        ("サインインしてください。", "サインイン"),
        ("クローズ済み", "クローズ"),
        ("サーバーが断りました。", "断る"),
        ("断られた値です。", "断る"),
        ("どのブランチも変更されていません。", "変更 as the verb"),
        ("何も変更していません。", "変更 as the verb"),
        ("確認待ちのアーカイブ", "確認待ち"),
        ("履歴が長いためです(読む範囲より)。", "half-width bracket"),
        ("提案が3件あります。", "counter"),
        ("上限は{limit}件です。", "counter"),
        ("この1時間に、何度も送りました。", "counter"),
    ] {
        let found = standard_findings(bad);
        assert!(
            found.iter().any(|f| f.contains(rule)),
            "{bad:?} should break {rule}: {found:?}"
        );
    }
    for ok in [
        "ログインしてください。",
        "シングルサインオンでログイン",
        "閉じている",
        "サーバーが拒否しました。",
        "どのブランチも変わっていません。",
        "変更を依頼する",
        "確定待ちのアーカイブ",
        "履歴が長いためです（読む範囲より）。",
        "提案が 3 件あります。",
        "上限は {limit} 件です。",
        "この 1 時間に、何度も送りました。",
        "コメント 2 件",
    ] {
        assert_eq!(standard_findings(ok), Vec::<&str>::new(), "{ok:?}");
    }
}

/// The corrections of the final handoff (T1) in the server's catalog (23 of the 25; the other two are the client's): each key says exactly this.
#[test]
fn the_final_corrections_of_the_server_catalog_are_in_it() {
    let after: [(&str, &str); 23] = [
        (
            "propose.held",
            "あなたの作業はこのサーバーでレビュー待ちとして保持されており、どのブランチも変わっていません。",
        ),
        (
            "propose.already_recorded",
            "このアーティファクトはすでに記録済みのため何も追加されておらず、どのブランチも変わっていません。",
        ),
        (
            "recovery.blocker.output_unreadable",
            "リポジトリのツールの出力を読み取れなかったため、何も変えていません。",
        ),
        (
            "recovery.outcome.stopped",
            "復旧は停止し、これ以上は何も変えていません。フラグはまだ立っています。",
        ),
        ("hide.refused.heading", "コメントは変わりませんでした"),
        (
            "decision.form.consequence",
            "これはリビジョン {revision} だけに記録され、このリポジトリを読める全員に見えます。この判断は、提案が開いている間は変えられます。あとのリビジョンには引き継がれず、何もマージしません。",
        ),
        (
            "error.forbidden.hint",
            "メンテナーに書き込み権限を依頼するか、ログインしている全員に提案を開放するよう依頼してください。",
        ),
        (
            "error.forbidden_comment.message",
            "このリポジトリは読めますが、コメントするにはログインが必要です。",
        ),
        (
            "error.forbidden_comment.hint",
            "ログインするか、アクセストークンを付けてリクエストを送ってください。",
        ),
        (
            "new.visibility.public",
            "誰でも（ログインの有無を問わない）",
        ),
        ("new.visibility.internal", "ログインしている人なら誰でも"),
        ("new.proposals.any", "読めるログイン済みの人なら誰でも"),
        (
            "new.signin",
            "リポジトリを作成するには、ログインしてください。",
        ),
        (
            "error.invalid_request.name_hint",
            "この応答の field と reason に、拒否された値と理由が示されています。API ドキュメントの項目の通りに直して、もう一度送ってください。",
        ),
        ("refs.closed", "閉じている"),
        (
            "error.stage_limit.message",
            "確定待ちのアーカイブがすでに {limit} 件あります。",
        ),
        (
            "explain.queued",
            "このパッチは、このサーバーで保持されています。メンテナーがプロジェクトの履歴に署名していないため、検証済みの記録には含まれていません。",
        ),
        (
            "proposal.patch.unknown_explain",
            "このサーバーは、このパッチが署名されて履歴に入ったかどうかを判断できません（履歴が、サーバーが読む範囲より長いためです）。",
        ),
        (
            "explain.patch",
            "パッチとは、記録された編集 1 件のことです。",
        ),
        (
            "quota.open_proposals.message",
            "このリポジトリで開いている提案がすでに {used} 件あり、上限は {limit} 件です。",
        ),
        (
            "quota.revisions.message",
            "この提案にはすでに {used} 件のリビジョンがあり、上限は {limit} 件です。",
        ),
        (
            "error.rate_comments.message",
            "この 1 時間に、この提案へ何度もコメントしています。",
        ),
        (
            "error.rate_decisions.message",
            "この 1 時間に、この提案へ何度も判断を記録しています。",
        ),
    ];
    for (key, want) in after {
        let e = JA_DEF
            .entries
            .iter()
            .find(|e| e.key == key)
            .unwrap_or_else(|| panic!("no `{key}`"));
        assert_eq!(e.other, want, "`{key}`");
    }
}
