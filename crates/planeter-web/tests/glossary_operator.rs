//! RFC 004 part 4a, T4: the words the person who runs the server meets in the recovery and proposal sentences are
//! defined, in plain words, in section 6 of `docs/glossary.md`. Every one of these words that an operator sentence of
//! the English catalog uses must be defined there, and the section must define nothing it has no use for twice or
//! leave a definition empty.

use std::path::PathBuf;

use planeter_web::catalog::shipped;

/// (the word as the glossary defines it, the stem that finds it in a sentence)
const OPERATOR_WORDS: [(&str, &str); 6] = [
    ("flag", "flag"),
    ("lease", "lease"),
    ("accept", "accept"),
    ("intent", "intent"),
    ("lock", "lock"),
    ("recovery", "recover"),
];

fn section_six() -> Vec<(String, String, String)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/glossary.md");
    let src = std::fs::read_to_string(&path).unwrap();
    let mut in_section = false;
    let mut rows = Vec::new();
    for line in src.lines() {
        if line.starts_with("## ") {
            in_section = line.starts_with("## 6.");
            continue;
        }
        if !in_section || !line.starts_with("| **") {
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split(" | ")
            .map(str::trim)
            .collect();
        assert_eq!(
            cells.len(),
            3,
            "a row has the word, its definition, where it is met: {line}"
        );
        rows.push((
            cells[0].trim_matches('*').to_owned(),
            cells[1].to_owned(),
            cells[2].to_owned(),
        ));
    }
    rows
}

/// The sentences an operator reads: the recovery plan, and the proposal and administration errors and texts.
fn operator_sentences() -> Vec<(&'static str, &'static str)> {
    shipped()[0]
        .entries
        .iter()
        .filter(|e| {
            e.key.starts_with("recovery.")
                || e.key.starts_with("error.")
                || e.key.starts_with("quota.")
                || e.key.starts_with("propose.")
        })
        .map(|e| (e.key, e.one))
        .collect()
}

fn uses(sentence: &str, stem: &str) -> bool {
    sentence
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(stem))
}

#[test]
fn the_section_defines_each_operator_word_once_in_plain_words() {
    let rows = section_six();
    let words: Vec<&str> = rows.iter().map(|(w, _, _)| w.as_str()).collect();
    let expected: Vec<&str> = OPERATOR_WORDS.iter().map(|(w, _)| *w).collect();
    assert_eq!(
        words, expected,
        "section 6 defines exactly the operator words, in order"
    );
    for (word, meaning, met) in &rows {
        assert!(
            meaning.len() > 40,
            "{word}: a definition is a sentence or two, not a label"
        );
        assert!(meaning.ends_with('.'), "{word}: {meaning}");
        assert!(!met.is_empty(), "{word}: where it is met");
        assert!(
            !meaning
                .to_lowercase()
                .contains(&format!("a {word} is a {word}")),
            "{word}: a definition does not use itself as its only content"
        );
    }
}

#[test]
fn every_operator_word_that_an_operator_sentence_uses_is_defined() {
    let defined: Vec<String> = section_six().into_iter().map(|(w, _, _)| w).collect();
    let mut seen_in_use = std::collections::BTreeSet::new();
    for (key, sentence) in operator_sentences() {
        for (word, stem) in OPERATOR_WORDS {
            if uses(sentence, stem) {
                seen_in_use.insert(word);
                assert!(
                    defined.iter().any(|d| d == word),
                    "`{key}` uses the word `{word}` and section 6 of docs/glossary.md does not define it: {sentence:?}"
                );
            }
        }
    }
    // The check is not vacuous: the sentences do use these words.
    for word in ["flag", "lease", "accept", "lock", "recovery"] {
        assert!(
            seen_in_use.contains(word),
            "no operator sentence uses `{word}`: the list is stale"
        );
    }
}

#[test]
fn a_new_jargon_word_in_an_operator_sentence_is_found() {
    // The detection works on text it has not seen: a sentence that says `lease` is detected, one that does not is not.
    assert!(uses("The repository's lease is held right now.", "lease"));
    assert!(uses("Recovery stopped.", "recover"));
    assert!(uses("It was recovered.", "recover"));
    assert!(!uses("The repository is busy.", "lease"));
    assert!(
        !uses("A blocker.", "lock"),
        "a word that merely contains the stem is not the word"
    );
}
