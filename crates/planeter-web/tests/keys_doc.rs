//! RFC 013 handoff 3, T4: `docs/keys.md` says only what the catalog and the ledger hold. The sentences it quotes are the catalog's, word for
//! word; the ledger entries it cites exist; the README and `docs/deliveries.md` point to it; and it never says the forbidden words except in the
//! sentence that states the limit.

use std::path::PathBuf;

use planeter_web::catalog::shipped;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn english(key: &str) -> String {
    shipped()
        .iter()
        .find(|l| l.tag == "en")
        .unwrap()
        .entries
        .iter()
        .find(|e| e.key == key)
        .unwrap_or_else(|| panic!("no message {key}"))
        .one()
        .to_owned()
}

#[test]
fn the_page_quotes_the_catalogs_sentences_word_for_word() {
    let doc = read("docs/keys.md");
    for key in [
        "repo.keys.source.creation",
        "repo.keys.source.creation_recorded",
        "repo.keys.source.operator",
        "repo.keys.source.delivery",
        "repo.keys.source.not_recorded",
        "repo.keys.unavailable",
        "operator.done",
        "operator.error.running",
        "operator.error.needs_recovery",
        "operator.error.already_trusted",
        "operator.error.record",
        "operator.start",
    ] {
        let sentence = english(key);
        assert!(
            doc.contains(&sentence),
            "{key}: {sentence:?} is not in docs/keys.md"
        );
    }
    // sentences with a number in them: the doc says the number the code uses
    let bound = english("operator.error.bound")
        .replace("{max}", &planeter_web::keys::keys_max().to_string());
    assert!(doc.contains(&bound), "{bound:?}");
    // the verify failure, quoted in two parts
    assert!(doc.contains("The key was added, but the repository no longer passes its checks, and a key cannot be taken out again"));
    assert!(english("operator.error.verify").contains("The key was added, but the repository no longer passes its checks, and a key cannot be taken out again."));
    assert!(doc.contains("The keys this repository trusts cannot be read now."));
    assert_eq!(planeter_web::keys::keys_max(), 64);
}

#[test]
fn the_ledger_entries_it_cites_exist_and_the_other_pages_point_to_it() {
    let doc = read("docs/keys.md");
    let ledger = read("docs/src/planeter-prikk-dependency-ledger.md");
    for pk in ["PK-90", "PK-91", "PK-92", "PK-93"] {
        assert!(doc.contains(pk), "{pk} is cited");
        assert!(
            ledger.contains(&format!("| {pk} |")),
            "{pk} is in the ledger"
        );
    }
    // the measurements the page quotes are the ledger's
    for fact in ["7, 10, 29 and 553 ms", "13.8 MB"] {
        assert!(doc.contains(fact), "{fact}");
    }
    assert!(ledger.contains("**7 / 10 / 29 / 553 ms**") && ledger.contains("13.8 MB"));
    assert!(read("README.md").contains("docs/keys.md"));
    assert!(read("docs/deliveries.md").contains("keys.md"));
}

#[test]
fn it_uses_the_words_retired_revoked_and_expired_only_to_say_they_are_not_possible() {
    // The page names what cannot be done, so the words appear: only under "What you cannot do", and in the line that says the pages never use them.
    let doc = read("docs/keys.md");
    let mut heading = String::new();
    for line in doc.lines() {
        if line.starts_with('#') {
            heading = line.to_owned();
        }
        let l = line.to_lowercase();
        for word in ["retired", "revoked", "expired", "rotated"] {
            if l.contains(word) {
                assert!(
                    heading.contains("What you cannot do") || l.contains("never says"),
                    "{word} under {heading:?}: {line}"
                );
            }
        }
    }
}
