//! Display-safe paths and text (RFC 005 part 6). **One function family, with its own tests.**
//!
//! A path in a tree is bytes; a line of a file is whatever the proposer wrote. Neither is shown as it is. The display form keeps
//! valid UTF-8, and shows as a **visible escape** everything that could hide, reorder or break what a reviewer reads:
//!
//! * bytes that are not valid UTF-8 as `\xNN`, one per byte;
//! * C0 controls, DEL, C1 controls, the line and paragraph separators (U+2028, U+2029), the **bidirectional controls** (U+202A–202E,
//!   U+2066–2069, U+200E, U+200F, U+061C) and the zero-width and format characters (U+200B–200D, U+2060, U+FEFF) as `\u{…}`;
//! * in a line of text, tab is kept (it is layout, not a disguise) and a stray carriage return is `␍` (U+240D).
//!
//! A name longer than [`NAME_MAX_BYTES`] bytes is cut, and a line longer than [`LINE_MAX_CHARS`] characters is cut. **The cut is data, not
//! words**: the text is the head and nothing else (no marker is written into it), and how much was left out is a number beside it
//! (`path_cut_bytes`, `cut_chars`), which each reader turns into a sentence in its own language.

/// A path or a symbolic link's target is shown up to this many bytes.
pub const NAME_MAX_BYTES: usize = 1024;
/// A line of a diff is shown up to this many characters (after escaping each escape counts as its own characters).
pub const LINE_MAX_CHARS: usize = 2000;

/// The characters that are shown as `\u{…}` in a path or in text: controls, separators, bidirectional controls, zero-width and
/// format characters. (Tab and the line terminator are decided by the caller.)
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

fn push_escaped(out: &mut String, c: char, keep_tab: bool) {
    if c == '\t' && keep_tab {
        out.push('\t');
    } else if c == '\r' && keep_tab {
        // text mode: a stray CR is made visible with its own symbol
        out.push('\u{240D}');
    } else if is_disguise(c) {
        out.push_str(&format!("\\u{{{:X}}}", u32::from(c)));
    } else {
        out.push(c);
    }
}

fn escape_bytes(bytes: &[u8], text_mode: bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            push_escaped(&mut out, c, text_mode);
        }
        for b in chunk.invalid() {
            out.push_str(&format!("\\x{b:02X}"));
        }
    }
    out
}

/// The display form of a path (or of a link target), whether it differs from the bytes, and how many bytes were cut off: valid UTF-8
/// kept, everything else escaped; a name over [`NAME_MAX_BYTES`] bytes is cut **at a character boundary** (no marker in the text; the
/// number of bytes left out is the third value, 0 when nothing was cut).
pub fn display_path(bytes: &[u8]) -> (String, bool, u64) {
    let mut keep = bytes.len().min(NAME_MAX_BYTES);
    if keep < bytes.len() {
        // never in the middle of a character: step back over continuation bytes (at most three)
        while keep > 0 && bytes[keep] & 0xC0 == 0x80 {
            keep -= 1;
        }
    }
    let shown = escape_bytes(&bytes[..keep], false);
    let cut = (bytes.len() - keep) as u64;
    let same = cut == 0 && std::str::from_utf8(bytes).is_ok_and(|s| s == shown);
    (shown, !same, cut)
}

/// The display form of one line of text (no line terminator in it), given as bytes that may not be UTF-8 (a link's target): tab kept, a
/// stray CR shown as `␍`; a line over [`LINE_MAX_CHARS`] characters is cut to that many (no marker in the text) and the second value
/// is the number of characters left out (0 when nothing was cut).
pub fn display_line_bytes(bytes: &[u8]) -> (String, u64) {
    cut_line(escape_bytes(bytes, true))
}

/// [`display_line_bytes`] for a line already known to be UTF-8.
pub fn display_line(line: &str) -> (String, u64) {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        push_escaped(&mut out, c, true);
    }
    cut_line(out)
}

fn cut_line(s: String) -> (String, u64) {
    let n = s.chars().count();
    if n <= LINE_MAX_CHARS {
        return (s, 0);
    }
    (
        s.chars().take(LINE_MAX_CHARS).collect(),
        (n - LINE_MAX_CHARS) as u64,
    )
}
