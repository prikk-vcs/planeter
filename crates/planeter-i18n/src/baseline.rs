//! What a sign-off covered. A signed-off language keeps, next to its catalog, a file `locales/<tag>.signed` that lists
//! `key<TAB>hash` for every message **as it was when the reviewers signed it off**. A message written or changed afterwards ships
//! (the dev team writes it, the glossary's rules apply) **and is flagged until a reviewer has read it**: nothing is silently counted
//! as reviewed. [`review_states`] says, for every message of a catalog, whether it is unchanged, changed or new since the sign-off.

use std::collections::BTreeMap;

use crate::Entry;

/// Where a message stands against the sign-off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    /// The text is the one that was signed off.
    Unchanged,
    /// The key was signed off with other text.
    Changed,
    /// The key did not exist at the sign-off.
    New,
}

impl ReviewState {
    pub fn label(self) -> &'static str {
        match self {
            ReviewState::Unchanged => "unchanged",
            ReviewState::Changed => "changed since sign-off",
            ReviewState::New => "new since sign-off",
        }
    }
}

/// A 64-bit FNV-1a hash (16 hex digits) of every form of a message, each with its category's label: it changes when any shown text
/// changes. It identifies wording; it is not a cryptographic hash.
pub fn message_hash(entry: &Entry) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |s: &str| {
        for b in s.bytes().chain(std::iter::once(0u8)) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (category, text) in entry.forms() {
        feed(category.label());
        feed(text);
    }
    format!("{h:016x}")
}

/// The baseline file for a catalog as it stands: one `key<TAB>hash` line per message, in catalog order, after a comment header.
pub fn render(entries: &[Entry], tag: &str) -> String {
    let mut out = format!(
        "# The messages of `{tag}` as signed off: key<TAB>hash (planeter-i18n baseline::message_hash). Regenerated only by a new sign-off.\n"
    );
    for e in entries {
        out.push_str(&format!("{}\t{}\n", e.key, message_hash(e)));
    }
    out
}

/// Read a baseline file. A malformed line, a hash that is not 16 hex digits and a key twice are errors.
pub fn parse(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, hash)) = line.split_once('\t') else {
            return Err(format!("line {}: expected key<TAB>hash", n + 1));
        };
        if key.is_empty()
            || hash.len() != 16
            || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(format!(
                "line {}: `{key}` needs a 16-digit lower-case hex hash",
                n + 1
            ));
        }
        if map.insert(key.to_owned(), hash.to_owned()).is_some() {
            return Err(format!("line {}: `{key}` twice", n + 1));
        }
    }
    Ok(map)
}

/// The state of every message of `entries` against the baseline, in catalog order.
pub fn review_states(
    entries: &[Entry],
    baseline: &BTreeMap<String, String>,
) -> Vec<(&'static str, ReviewState)> {
    entries
        .iter()
        .map(|e| {
            let state = match baseline.get(e.key) {
                None => ReviewState::New,
                Some(h) if *h == message_hash(e) => ReviewState::Unchanged,
                Some(_) => ReviewState::Changed,
            };
            (e.key, state)
        })
        .collect()
}

/// How many messages are changed or new since the sign-off.
pub fn unreviewed_count(entries: &[Entry], baseline: &BTreeMap<String, String>) -> usize {
    review_states(entries, baseline)
        .into_iter()
        .filter(|(_, s)| *s != ReviewState::Unchanged)
        .count()
}

/// The counts `(unchanged, changed, new)`.
pub fn counts(entries: &[Entry], baseline: &BTreeMap<String, String>) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for (_, s) in review_states(entries, baseline) {
        match s {
            ReviewState::Unchanged => c.0 += 1,
            ReviewState::Changed => c.1 += 1,
            ReviewState::New => c.2 += 1,
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{msg, plural_msg};

    #[test]
    fn a_rendered_baseline_reads_back_and_every_message_is_unchanged() {
        let entries = [
            msg("a.b", "x"),
            plural_msg("a.c", "{count} one", "{count} many"),
        ];
        let text = render(&entries, "xx");
        let map = parse(&text).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(counts(&entries, &map), (2, 0, 0));
        assert_eq!(unreviewed_count(&entries, &map), 0);
    }

    #[test]
    fn a_changed_form_a_new_key_and_a_removed_key_are_told_apart() {
        let before = [
            msg("a.b", "x"),
            plural_msg("a.c", "{count} one", "{count} many"),
            msg("a.gone", "y"),
        ];
        let map = parse(&render(&before, "xx")).unwrap();
        let after = [
            msg("a.b", "x"),
            plural_msg("a.c", "{count} one", "{count} lots"),
            msg("a.new", "z"),
        ];
        let states = review_states(&after, &map);
        assert_eq!(
            states,
            vec![
                ("a.b", ReviewState::Unchanged),
                ("a.c", ReviewState::Changed),
                ("a.new", ReviewState::New)
            ]
        );
        assert_eq!(unreviewed_count(&after, &map), 2);
        // A message whose only difference is which category carries the text is a different message.
        let moved = [
            msg("a.b", "x"),
            plural_msg("a.c", "{count} many", "{count} one"),
            msg("a.new", "z"),
        ];
        assert_eq!(review_states(&moved, &map)[1].1, ReviewState::Changed);
        assert_ne!(
            message_hash(&msg("k", "t")),
            message_hash(&plural_msg("k", "t", "t"))
        );
    }

    #[test]
    fn a_malformed_baseline_is_refused() {
        for bad in [
            "no tab here",
            "a\tshort",
            "a\tXXXXXXXXXXXXXXXX",
            "a\t0123456789abcdef\na\t0123456789abcdef",
            "\t0123456789abcdef",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert!(parse("# comment\n\nok.key\t0123456789abcdef\n").is_ok());
    }
}
