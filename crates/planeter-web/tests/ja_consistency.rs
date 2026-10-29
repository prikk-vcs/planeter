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
