//! The words of a quota refusal (RFC 005 D-9): each refusal says which quota, the limit, the current use and what
//! the person can do, and every key the core model can ask for exists in every language.

use planeter_core::change::{Quota, QuotaRefusal};
use planeter_web::catalog::{EN_DEF, JA_DEF, Key, Lang};
use planeter_web::quota::{keys, texts};

const MIB: u64 = 1024 * 1024;

fn all() -> [QuotaRefusal; 4] {
    [
        QuotaRefusal {
            quota: Quota::OpenChanges,
            limit: 10,
            used: 10,
            requested: 1,
        },
        QuotaRefusal {
            quota: Quota::AcceptedBytes,
            limit: 512 * MIB,
            used: 500 * MIB,
            requested: 20 * MIB,
        },
        QuotaRefusal {
            quota: Quota::RevisionsPerChange,
            limit: 20,
            used: 20,
            requested: 1,
        },
        QuotaRefusal {
            quota: Quota::RepositoryCapacity,
            limit: 4096 * MIB,
            used: 4090 * MIB,
            requested: 10 * MIB,
        },
    ]
}

#[test]
fn the_keys_the_core_model_asks_for_are_the_keys_the_web_maps_to() {
    for r in all() {
        let (m, h) = keys(&r);
        assert_eq!(m.id(), r.message_key());
        assert_eq!(h.id(), r.hint_key());
    }
    let ids: Vec<&str> = Key::ALL
        .iter()
        .map(|k| k.id())
        .filter(|id| id.starts_with("quota."))
        .collect();
    let mut expected = QuotaRefusal::ALL_KEYS.to_vec();
    let mut got = ids;
    expected.sort_unstable();
    got.sort_unstable();
    assert_eq!(
        got, expected,
        "the catalog has exactly the quota keys the model can ask for"
    );
}

#[test]
fn each_english_refusal_names_the_limit_and_the_use_and_says_what_to_do() {
    let lang = Lang::from_def(&EN_DEF);
    let said: Vec<(String, String)> = all().iter().map(|r| texts(r, lang)).collect();
    assert!(
        said[0].0.contains("10 open changes") && said[0].0.contains("limit is 10"),
        "{:?}",
        said[0]
    );
    assert!(
        said[1].0.contains("500 MiB") && said[1].0.contains("512 MiB"),
        "{:?}",
        said[1]
    );
    assert!(
        said[2].0.contains("20 revisions") && said[2].0.contains("limit is 20"),
        "{:?}",
        said[2]
    );
    assert!(
        said[3].0.contains("4090 MiB")
            && said[3].0.contains("4096 MiB")
            && said[3].0.contains("capacity"),
        "{:?}",
        said[3]
    );
    for (m, h) in &said {
        assert!(
            !m.contains('{') && !h.contains('{'),
            "no placeholder is left: {m} / {h}"
        );
        assert!(
            h.contains("ask ") || h.contains("Ask ") || h.contains("Close ") || h.contains("Open "),
            "the hint says what to do: {h}"
        );
    }
}

#[test]
fn the_japanese_draft_fills_the_same_placeholders() {
    let lang = Lang::from_def(&JA_DEF);
    for r in all() {
        let (m, h) = texts(&r, lang);
        assert!(!m.contains('{') && !h.contains('{'), "{m} / {h}");
        let (limit, used) = r.figures();
        assert!(
            m.contains(&limit.to_string()) && m.contains(&used.to_string()),
            "{m}"
        );
    }
}
