//! RFC 008 handoff 2, T4: the words of landing by delivery, in both languages. Every state and every reason of the closed lists has a
//! message and a hint of its own; the three sentences the owner accepted are the English the RFC fixed; and **nothing the person
//! reads mentions the engine's words** (*stage*, *chain*, *tip*, *adopt*, *received*, *claim*, *seal*) — a scan over every message of the
//! feature, as the review view's does. No prikk needed.

use planeter_store::DeliveryState;
use planeter_web::catalog::{EN, Key, Lang, T};
use planeter_web::deliveries::texts;

fn languages() -> Vec<(&'static str, Lang)> {
    vec![("en", EN), ("ja", Lang::from_tag("ja").unwrap())]
}

/// The ids of every message of the feature.
fn feature_keys() -> Vec<Key> {
    Key::ALL
        .iter()
        .copied()
        .filter(|k| {
            let id = k.id();
            id.starts_with("delivery.")
                || id.starts_with("error.delivery_")
                || id == "error.forbidden_deliver.message"
                || id == "error.forbidden_deliver.hint"
                || id == "repo.delivered_by"
                || id == "repo.project_branch"
                || id == "repo.trusted_keys"
                || id == "recovery.cause.delivery_unfinished"
        })
        .collect()
}

const REASONS: [&str; 16] = [
    "no_main",
    "unadopted_key",
    "not_a_successor",
    "older_or_diverged",
    "unrelated",
    "branch_missing",
    "nothing_to_deliver",
    "damaged_archive",
    "moved",
    "exchange_failed",
    "verify_failed",
    "publication_trust",
    "timed_out",
    "ceiling",
    "unavailable",
    "repository_gone",
];

#[test]
fn every_state_and_every_reason_has_its_own_message_and_hint_in_both_languages() {
    for (tag, lang) in languages() {
        let t = T::new(lang);
        let mut seen = std::collections::HashSet::new();
        for state in DeliveryState::ALL {
            if state == DeliveryState::Failed {
                continue; // a failed delivery says its reason (below)
            }
            let (m, h) = texts(&t, state, None, None, false);
            assert!(!m.is_empty() && !h.is_empty(), "{tag}: {state:?}");
            assert!(
                !m.contains('{') && !h.contains('{'),
                "{tag}: {state:?}: a placeholder left"
            );
            assert!(
                seen.insert(m),
                "{tag}: {state:?} says what another state says"
            );
        }
        // a failed delivery without a reason, and with each one
        let (m, _) = texts(&t, DeliveryState::Failed, None, None, false);
        assert!(seen.insert(m), "{tag}: failed");
        for reason in REASONS {
            let (m, h) = texts(
                &t,
                DeliveryState::Failed,
                Some(reason),
                Some("heads/dev"),
                false,
            );
            assert!(!m.is_empty() && !h.is_empty(), "{tag}: {reason}");
            assert!(
                !m.contains('{') && !h.contains('{'),
                "{tag}: {reason}: a placeholder left"
            );
            assert!(
                seen.insert(m),
                "{tag}: {reason} says what another reason says"
            );
        }
        // an unfinished install says so, whatever its state
        let (m, _) = texts(&t, DeliveryState::Reaccepting, None, None, true);
        assert!(seen.insert(m), "{tag}: needs recovery");
        // the branch of a reason that names one is in its sentence
        let (m, _) = texts(
            &t,
            DeliveryState::Failed,
            Some("branch_missing"),
            Some("heads/dev"),
            false,
        );
        assert!(m.contains("heads/dev"), "{tag}");
    }
}

#[test]
fn the_sentences_the_owner_accepted_are_the_ones_the_rfc_fixed() {
    let t = T::new(EN);
    let (m, _) = texts(&t, DeliveryState::Reported, None, None, false);
    assert_eq!(
        m,
        "Delivering replaces this repository's branch heads/main with yours. It is accepted only if yours continues from the current one; nothing is signed by this server."
    );
    assert_eq!(
        t.msg(Key::RepoDeliveredBy)
            .with("name", "Alice")
            .with("date", "2026-11-18")
            .text(),
        "Delivered by Alice on 2026-11-18: signed by a key this repository trusts and checked by prikk."
    );
    // the examples of refusals in the handoff
    let (m, h) = texts(
        &t,
        DeliveryState::Failed,
        Some("older_or_diverged"),
        None,
        false,
    );
    assert_eq!(
        m,
        "Your branch does not continue from this repository's branch."
    );
    assert_eq!(
        h,
        "Update your repository from the project's branch, then deliver again."
    );
    let (m, h) = texts(
        &t,
        DeliveryState::Failed,
        Some("unadopted_key"),
        None,
        false,
    );
    assert_eq!(
        m,
        "Some of your history is signed by a key this repository does not trust."
    );
    assert_eq!(h, "Only the keys listed here can deliver.");
    let (m, h) = texts(
        &t,
        DeliveryState::Failed,
        Some("branch_missing"),
        Some("heads/dev"),
        false,
    );
    assert_eq!(
        m,
        "This repository has a branch your delivery does not have (heads/dev)."
    );
    assert_eq!(h, "A delivery may add branches but never remove one.");
}

#[test]
fn no_message_of_the_feature_names_the_engines_words() {
    let keys = feature_keys();
    assert!(keys.len() >= 60, "{}", keys.len());
    for (tag, lang) in languages() {
        let t = T::new(lang);
        for key in &keys {
            let text = t
                .msg(*key)
                .with("ref", "heads/main")
                .with("name", "N")
                .with("date", "D")
                .text();
            let lower = text.to_lowercase();
            let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
            for bad in [
                "stage", "staged", "chain", "tip", "adopt", "adopted", "received", "claim", "seal",
                "sealed", "block",
            ] {
                assert!(
                    !words.contains(&bad),
                    "{tag}: `{}` says {bad:?}: {text}",
                    key.id()
                );
            }
            for bad in [
                "保持",
                "預か",
                "ステージ",
                "チェーン",
                "チップ",
                "先端",
                "採用",
                "受信",
                "クレーム",
                "ブロック",
            ] {
                assert!(
                    !text.contains(bad),
                    "{tag}: `{}` says {bad:?}: {text}",
                    key.id()
                );
            }
        }
    }
}

/// The table the review request quotes: key → English → 日本語 (written to `target/tmp/delivery-messages.md`; the test holds that both
/// languages have every key and that the placeholders of the two agree).
#[test]
fn the_message_table_is_written_for_the_review() {
    let (en, ja) = (T::new(EN), T::new(Lang::from_tag("ja").unwrap()));
    let mut out = String::from("| key | English | 日本語 |\n|---|---|---|\n");
    for key in feature_keys() {
        let (e, j) = (en.get(key), ja.get(key));
        let holes = |t: &str| {
            let mut v: Vec<String> = t
                .split('{')
                .skip(1)
                .filter_map(|r| r.split('}').next().map(str::to_owned))
                .collect();
            v.sort();
            v
        };
        assert_eq!(holes(&e), holes(&j), "{}", key.id());
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            key.id(),
            e.replace('|', "\\|"),
            j.replace('|', "\\|")
        ));
    }
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("delivery-messages.md");
    std::fs::write(path, out).unwrap();
}
