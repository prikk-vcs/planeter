//! Display-safe text for the account records (RFC 014 D-7): a reason, an event's detail, a name. **The same set of characters is a
//! disguise here as in the file-by-file view** (`planeter-transport`'s `changes::display::is_disguise`, which a test of that crate holds
//! equal to this one over every character): controls, separators, the bidirectional controls and the zero-width and format characters.
//!
//! Two uses: a **record** (an event's detail, a disable reason) is *escaped and cut*, never refused ([`escape_display`]): what is written
//! is what the operator typed, made visible; a **name** (a display name, a token's name) is *refused* when it carries one
//! ([`is_display_safe`]): a name is an identity, not a log line.

/// Whether `c` could hide, reorder or break what a reader reads: C0 and C1 controls (tab and the line terminators included: an account
/// record is one line), DEL, the line and paragraph separators, the bidirectional controls and the zero-width and format characters.
pub fn is_disguise(c: char) -> bool {
    matches!(
        c,
        '\u{0000}'..='\u{001F}'
            | '\u{007F}'
            | '\u{0080}'..='\u{009F}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}'
            | '\u{200B}'..='\u{200D}'
            | '\u{2060}'
            | '\u{FEFF}'
    )
}

/// `s` with every disguise character shown as a visible escape (`\u{202E}`), **cut to at most `max_chars` characters of output**. The cut
/// is at a character boundary of the output and **never inside an escape** (an escape that would not fit is left out whole). Nothing is
/// refused: the result is always shown as it is.
pub fn escape_display(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for c in s.chars() {
        let piece = if is_disguise(c) {
            format!("\\u{{{:X}}}", u32::from(c))
        } else {
            c.to_string()
        };
        let n = piece.chars().count();
        if used + n > max_chars {
            break;
        }
        out.push_str(&piece);
        used += n;
    }
    out
}

/// Whether `s` carries no disguise character.
pub fn is_display_safe(s: &str) -> bool {
    !s.chars().any(is_disguise)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_are_visible_and_never_cut_in_the_middle() {
        assert_eq!(escape_display("a\u{202E}b", 100), "a\\u{202E}b");
        assert_eq!(
            escape_display("line\nbreak\ttab", 100),
            "line\\u{A}break\\u{9}tab"
        );
        // 200 disguise characters: each escape is longer than one character, so the text is cut at whole escapes.
        let all = "\u{202E}".repeat(200);
        let cut = escape_display(&all, 200);
        assert!(cut.chars().count() <= 200);
        assert!(cut.ends_with('}'));
        assert_eq!(cut.chars().count() % "\\u{202E}".chars().count(), 0);
        // Plain text is cut at a character boundary.
        assert_eq!(
            escape_display("\u{3042}\u{3044}\u{3046}\u{3048}\u{304a}", 3),
            "\u{3042}\u{3044}\u{3046}"
        );
        assert_eq!(escape_display("", 3), "");
    }

    #[test]
    fn display_safety_is_the_same_set() {
        assert!(is_display_safe(
            "Alice Smith, \u{5c71}\u{7530} \u{592a}\u{90ce}"
        ));
        for c in [
            '\n', '\t', '\u{202E}', '\u{200B}', '\u{FEFF}', '\u{7F}', '\u{85}',
        ] {
            assert!(!is_display_safe(&format!("a{c}b")), "{c:?}");
        }
    }
}
