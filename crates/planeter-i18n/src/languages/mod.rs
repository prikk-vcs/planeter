//! The registry: **the only list of languages**. A language is a row (`Language`); adding one is a row here, a catalog in each
//! program, a rules file and a glossary file — not a code change elsewhere (`docs/messages-and-languages.md`, "Adding a language").

use crate::Language;

pub mod en;
pub mod ja;

/// Every language, the source language first. The order is the order of the switcher and of "the first registered".
pub static LANGUAGES: &[&Language] = &[&en::EN, &ja::JA];

/// The source language: the one the messages are written in and the fallback of negotiation.
pub fn source() -> &'static Language {
    LANGUAGES
        .iter()
        .copied()
        .find(|l| l.is_source())
        .expect("the registry has a source language")
}

/// The languages a build ships without any operator setting: the source language and the signed-off ones.
pub fn shipped_of<'a>(list: &[&'a Language]) -> Vec<&'a Language> {
    list.iter().copied().filter(|l| l.is_shipped()).collect()
}

/// The languages that exist as drafts and can be previewed by the operator.
pub fn previewable_of<'a>(list: &[&'a Language]) -> Vec<&'a Language> {
    list.iter().copied().filter(|l| l.is_draft()).collect()
}

/// The language with this tag (case-insensitive, exact).
pub fn by_tag(tag: &str) -> Option<&'static Language> {
    LANGUAGES
        .iter()
        .copied()
        .find(|l| l.tag.eq_ignore_ascii_case(tag))
}

/// What is wrong with a list of languages as a registry: the rules every language row must meet. Empty when it is sound.
/// A `SignedOff` language needs reviewers, a date, a commit and a 16-hex fingerprint; the list needs exactly one source language,
/// first; tags are unique; a language declares `other` last and its rule returns only categories it declares.
pub fn check_registry(list: &[&Language]) -> Vec<String> {
    use crate::Status;
    let mut problems = Vec::new();
    let sources = list.iter().filter(|l| l.is_source()).count();
    if sources != 1 || !list.first().is_some_and(|l| l.is_source()) {
        problems.push(format!(
            "the registry needs exactly one source language, first (it has {sources})"
        ));
    }
    for (i, l) in list.iter().enumerate() {
        let tag = l.tag;
        if crate::tag::parse(tag).is_none() {
            problems.push(format!("`{tag}` is not a language tag"));
        }
        if list[..i].iter().any(|p| p.tag.eq_ignore_ascii_case(tag)) {
            problems.push(format!("`{tag}` is registered twice"));
        }
        if l.endonym.trim().is_empty() {
            problems.push(format!("`{tag}` has no endonym"));
        }
        if l.categories.last() != Some(&crate::Category::Other) {
            problems.push(format!(
                "`{tag}`: the plural categories must end with `other`"
            ));
        }
        for n in (0..=1200u64).chain([10_000, 1_000_000]) {
            let c = (l.plural)(n);
            if !l.categories.contains(&c) {
                problems.push(format!("`{tag}`: the plural rule gives {} for {n}, which the language does not declare", c.label()));
                break;
            }
        }
        if let Status::SignedOff(s) = &l.status {
            if s.reviewers.is_empty() || s.reviewers.iter().any(|r| r.trim().is_empty()) {
                problems.push(format!("`{tag}` is signed off without a named reviewer"));
            }
            let d = s.date.as_bytes();
            let date_ok = d.len() == 10
                && d.iter().enumerate().all(|(i, b)| {
                    if i == 4 || i == 7 {
                        *b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                });
            if !date_ok {
                problems.push(format!(
                    "`{tag}`: the sign-off date {:?} is not YYYY-MM-DD",
                    s.date
                ));
            }
            if s.commit.len() < 7
                || !s
                    .commit
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                problems.push(format!(
                    "`{tag}`: the sign-off commit {:?} is not a commit hash",
                    s.commit
                ));
            }
            if s.sheet_fingerprint.len() != 16
                || !s
                    .sheet_fingerprint
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                problems.push(format!(
                    "`{tag}`: the sign-off fingerprint {:?} is not 16 hex digits",
                    s.sheet_fingerprint
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dir, SignOff, Status, plural};

    fn row(tag: &'static str, status: Status) -> Language {
        Language {
            tag,
            endonym: tag,
            dir: Dir::Ltr,
            categories: plural::ONE_OTHER,
            plural: plural::en,
            group_separator: ',',
            decimal_separator: '.',
            status,
        }
    }

    fn signed(
        reviewers: &'static [&'static str],
        date: &'static str,
        commit: &'static str,
        fp: &'static str,
    ) -> Status {
        Status::SignedOff(SignOff {
            reviewers,
            date,
            commit,
            sheet_fingerprint: fp,
            note: "",
        })
    }

    /// The one test over every language in the registry.
    #[test]
    fn every_language_in_the_registry_is_sound() {
        assert_eq!(check_registry(LANGUAGES), Vec::<String>::new());
        assert_eq!(source().tag, "en");
        assert_eq!(
            shipped_of(LANGUAGES)
                .iter()
                .map(|l| l.tag)
                .collect::<Vec<_>>(),
            ["en", "ja"]
        );
        assert!(
            previewable_of(LANGUAGES).is_empty(),
            "no draft language exists at present"
        );
        assert_eq!(by_tag("JA").map(|l| l.tag), Some("ja"));
    }

    #[test]
    fn the_registry_check_fails_each_way_a_language_can_be_unsound() {
        let en = row("en", Status::Source);
        let good = row(
            "xx",
            signed(
                &["a fluent reader"],
                "2026-11-04",
                "0123456789abcdef",
                "0123456789abcdef",
            ),
        );
        assert_eq!(check_registry(&[&en, &good]), Vec::<String>::new());
        for (bad, needle) in [
            (
                row(
                    "xx",
                    signed(&[], "2026-11-04", "0123456789abcdef", "0123456789abcdef"),
                ),
                "without a named reviewer",
            ),
            (
                row(
                    "xx",
                    signed(&[" "], "2026-11-04", "0123456789abcdef", "0123456789abcdef"),
                ),
                "without a named reviewer",
            ),
            (
                row(
                    "xx",
                    signed(&["r"], "", "0123456789abcdef", "0123456789abcdef"),
                ),
                "not YYYY-MM-DD",
            ),
            (
                row(
                    "xx",
                    signed(&["r"], "04/11/2026", "0123456789abcdef", "0123456789abcdef"),
                ),
                "not YYYY-MM-DD",
            ),
            (
                row("xx", signed(&["r"], "2026-11-04", "", "0123456789abcdef")),
                "not a commit",
            ),
            (
                row(
                    "xx",
                    signed(&["r"], "2026-11-04", "xyz", "0123456789abcdef"),
                ),
                "not a commit",
            ),
            (
                row("xx", signed(&["r"], "2026-11-04", "0123456789abcdef", "")),
                "not 16 hex",
            ),
            (
                row(
                    "xx",
                    signed(&["r"], "2026-11-04", "0123456789abcdef", "0123456789ABCDEF"),
                ),
                "not 16 hex",
            ),
            (
                row(
                    "xx",
                    signed(&["r"], "2026-11-04", "0123456789abcdef", "0123"),
                ),
                "not 16 hex",
            ),
            (row("e", Status::Draft), "not a language tag"),
        ] {
            let problems = check_registry(&[&en, &bad]);
            assert!(
                problems.iter().any(|p| p.contains(needle)),
                "{needle}: {problems:?}"
            );
        }
        // A draft is not shipped; two source languages and a duplicate tag are refused.
        let draft = row("xx", Status::Draft);
        assert!(!draft.is_shipped() && draft.is_draft());
        assert_eq!(shipped_of(&[&en, &draft]).len(), 1);
        assert_eq!(previewable_of(&[&en, &draft]).len(), 1);
        let en2 = row("en2", Status::Source);
        assert!(
            check_registry(&[&en, &en2])
                .iter()
                .any(|p| p.contains("exactly one source"))
        );
        assert!(
            check_registry(&[&en, &row("EN", Status::Draft)])
                .iter()
                .any(|p| p.contains("twice"))
        );
        assert!(
            check_registry(&[&draft, &en])
                .iter()
                .any(|p| p.contains("exactly one source"))
        );
    }

    #[test]
    fn a_plural_rule_that_returns_an_undeclared_category_is_refused() {
        let en = row("en", Status::Source);
        let mut odd = row("xx", Status::Draft);
        odd.plural = plural::ru; // gives Few and Many, which ONE_OTHER does not declare
        assert!(
            check_registry(&[&en, &odd])
                .iter()
                .any(|p| p.contains("does not declare"))
        );
        let mut no_other = row("xx", Status::Draft);
        no_other.categories = &[crate::Category::One];
        assert!(
            check_registry(&[&en, &no_other])
                .iter()
                .any(|p| p.contains("end with `other`"))
        );
    }
}
