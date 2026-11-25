//! RFC 013 D-1 held in the words: planeter never removes a key, so nothing it says about keys says a key is retired, revoked, expired, rotated or
//! removed. The messages of the keys' pages (`repo.keys.*`) and of the operator's command (`operator.*`) are scanned in English and in Japanese;
//! the one fixed phrase that states the limit ("Keys cannot be removed yet") is the single exception, listed by key. (The rendered pages are
//! scanned in `deliveries_http.rs`.)

use planeter_web::catalog::shipped;

const FORBIDDEN_EN: &[&str] = &["retire", "revoke", "revoked", "expire", "rotat", "remov"];
const FORBIDDEN_JA: &[&str] = &["失効", "無効", "取り消", "退役", "削除"];

/// (the key, the English phrase, the Japanese phrase): where the message states the limit that keys cannot be removed yet.
const THE_LIMIT: &[(&str, &str, &str)] = &[(
    "operator.error.bound",
    "Keys cannot be removed yet",
    "鍵はまだ取り除けない",
)];

fn scanned(key: &str) -> bool {
    // The operator's account commands (RFC 014) are about accounts, not keys: `revoke` is a command word there and 無効 is "disabled".
    key.starts_with("repo.keys.")
        || (key.starts_with("operator.") && !key.starts_with("operator.account."))
}

#[test]
fn no_message_about_keys_says_retired_revoked_expired_rotated_or_removed() {
    let mut scanned_count = 0;
    for lang in shipped() {
        let forbidden = if lang.tag == "en" {
            FORBIDDEN_EN
        } else {
            FORBIDDEN_JA
        };
        for e in lang.entries.iter().filter(|e| scanned(e.key)) {
            scanned_count += 1;
            let mut text = e.one().to_lowercase();
            if let Some((_, en, ja)) = THE_LIMIT.iter().find(|(k, _, _)| *k == e.key) {
                text = text.replace(&en.to_lowercase(), "").replace(ja, "");
            }
            for w in forbidden {
                assert!(
                    !text.contains(w),
                    "{} ({}): `{w}` in {:?}",
                    e.key,
                    lang.tag,
                    e.one()
                );
            }
        }
    }
    assert!(
        scanned_count >= 2 * (27 + 8),
        "the scan found the messages: {scanned_count}"
    );
}

#[test]
fn the_one_exception_is_where_it_states_the_limit_and_nowhere_else() {
    // the phrase is in the message listed for it, in both languages, and in no other scanned message
    for lang in shipped() {
        let phrase = |(_, en, ja): &(&str, &str, &str)| {
            if lang.tag == "en" {
                en.to_lowercase()
            } else {
                (*ja).to_owned()
            }
        };
        for e in lang.entries.iter().filter(|e| scanned(e.key)) {
            for l in THE_LIMIT {
                let has = e.one().to_lowercase().contains(&phrase(l));
                assert_eq!(has, e.key == l.0, "{} ({})", e.key, lang.tag);
            }
        }
    }
}
