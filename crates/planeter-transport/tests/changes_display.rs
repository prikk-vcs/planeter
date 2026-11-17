//! Display-safe paths and text (RFC 005 part 6): one function family, every disguise shown as a visible escape.

use planeter_transport::changes::display::{
    LINE_MAX_CHARS, NAME_MAX_BYTES, display_line, display_line_bytes, display_path, is_disguise,
};

#[test]
fn an_ordinary_utf8_path_is_kept_and_is_not_marked() {
    for p in [
        "src/main.rs",
        "docs/日本語/ファイル.md",
        "a b/c",
        "emoji-🎉.txt",
        "back\\slash",
    ] {
        assert_eq!(display_path(p.as_bytes()), (p.to_owned(), false, 0), "{p}");
    }
}

#[test]
fn invalid_utf8_is_shown_byte_by_byte_as_x_escapes() {
    assert_eq!(
        display_path(b"ok\xFF\xFEz"),
        ("ok\\xFF\\xFEz".to_owned(), true, 0)
    );
    // a Shift_JIS name
    assert_eq!(
        display_path(b"\x83e\x83X\x83g"),
        ("\\x83e\\x83X\\x83g".to_owned(), true, 0)
    );
    // a truncated multibyte sequence at the end
    assert_eq!(
        display_path(b"a\xE3\x81"),
        ("a\\xE3\\x81".to_owned(), true, 0)
    );
}

#[test]
fn controls_separators_bidi_and_zero_width_characters_are_visible_escapes_in_a_path() {
    let cases: &[(&str, &str)] = &[
        ("a\nb", "a\\u{A}b"),
        ("a\tb", "a\\u{9}b"),
        ("a\rb", "a\\u{D}b"),
        ("a\u{0}b", "a\\u{0}b"),
        ("a\u{7F}b", "a\\u{7F}b"),
        ("a\u{85}b", "a\\u{85}b"),
        ("a\u{9F}b", "a\\u{9F}b"),
        ("a\u{2028}b", "a\\u{2028}b"),
        ("a\u{2029}b", "a\\u{2029}b"),
        ("fdp.\u{202E}txt", "fdp.\\u{202E}txt"),
        (
            "a\u{202A}b\u{202B}c\u{202C}d\u{202D}e",
            "a\\u{202A}b\\u{202B}c\\u{202C}d\\u{202D}e",
        ),
        (
            "a\u{2066}b\u{2067}c\u{2068}d\u{2069}e",
            "a\\u{2066}b\\u{2067}c\\u{2068}d\\u{2069}e",
        ),
        (
            "a\u{200E}b\u{200F}c\u{061C}d",
            "a\\u{200E}b\\u{200F}c\\u{61C}d",
        ),
        (
            "a\u{200B}b\u{200C}c\u{200D}d\u{2060}e\u{FEFF}f",
            "a\\u{200B}b\\u{200C}c\\u{200D}d\\u{2060}e\\u{FEFF}f",
        ),
    ];
    for (raw, shown) in cases {
        assert_eq!(
            display_path(raw.as_bytes()),
            ((*shown).to_owned(), true, 0),
            "{raw:?}"
        );
    }
    // the disguise set itself
    for c in [
        '\u{202E}', '\u{2066}', '\u{200B}', '\u{FEFF}', '\u{061C}', '\u{85}',
    ] {
        assert!(is_disguise(c));
    }
    for c in ['a', ' ', 'é', '日', '🎉', '\u{3000}'] {
        assert!(!is_disguise(c), "{c:?}");
    }
}

#[test]
fn a_name_with_html_in_it_is_left_for_the_page_to_escape() {
    assert_eq!(
        display_path(b"<script>alert(1)</script>").0,
        "<script>alert(1)</script>"
    );
}

#[test]
fn a_long_name_is_its_head_with_no_marker_and_the_count_of_the_bytes_left_out_beside_it() {
    let long = vec![b'a'; NAME_MAX_BYTES + 77];
    let (shown, escaped, cut) = display_path(&long);
    assert!(escaped);
    assert_eq!(
        shown,
        "a".repeat(NAME_MAX_BYTES),
        "the head, and nothing else"
    );
    assert_eq!(cut, 77);
    // exactly at the limit: not cut
    assert_eq!(
        display_path(&vec![b'a'; NAME_MAX_BYTES]),
        ("a".repeat(NAME_MAX_BYTES), false, 0)
    );
}

#[test]
fn a_name_is_cut_at_a_character_boundary_never_inside_a_character() {
    // 1,023 ASCII bytes, then a three-byte character that straddles the limit
    let mut name = "a".repeat(NAME_MAX_BYTES - 1).into_bytes();
    name.extend("あい".as_bytes());
    let (shown, _, cut) = display_path(&name);
    assert_eq!(
        shown,
        "a".repeat(NAME_MAX_BYTES - 1),
        "the half character is not shown"
    );
    assert_eq!(cut as usize, name.len() - (NAME_MAX_BYTES - 1));
    // a character that ends exactly at the limit stays
    let mut exact = "a".repeat(NAME_MAX_BYTES - 3).into_bytes();
    exact.extend("あい".as_bytes());
    let (shown, _, cut) = display_path(&exact);
    assert_eq!(shown, format!("{}あ", "a".repeat(NAME_MAX_BYTES - 3)));
    assert_eq!(cut, 3);
    assert!(!shown.contains("\\x"), "no escape from a cut character");
}

#[test]
fn the_head_of_a_cut_name_is_still_display_safe() {
    let mut name = b"x\xFFy\x1B".to_vec();
    name.extend(vec![b'a'; NAME_MAX_BYTES]);
    let (shown, escaped, cut) = display_path(&name);
    assert!(escaped);
    assert!(shown.starts_with("x\\xFFy\\u{1B}"), "{shown}");
    assert!(cut > 0);
    assert!(!shown.contains("more bytes"));
}

#[test]
fn text_keeps_tab_shows_a_stray_cr_as_its_symbol_and_escapes_the_rest() {
    assert_eq!(display_line("a\tb"), ("a\tb".to_owned(), 0));
    assert_eq!(display_line("a\rb\r").0, "a\u{240D}b\u{240D}");
    assert_eq!(
        display_line("x\u{202E}y\u{0}z\u{1B}[31m").0,
        "x\\u{202E}y\\u{0}z\\u{1B}[31m"
    );
    assert_eq!(display_line("日本語 text").0, "日本語 text");
    assert_eq!(display_line_bytes(b"/etc/passwd").0, "/etc/passwd");
    assert_eq!(
        display_line_bytes(b"to\xFFtarget\n").0,
        "to\\xFFtarget\\u{A}"
    );
}

#[test]
fn a_long_line_is_its_head_with_no_marker_and_the_count_of_the_characters_left_out_beside_it() {
    let long = "x".repeat(LINE_MAX_CHARS + 5);
    assert_eq!(display_line(&long), ("x".repeat(LINE_MAX_CHARS), 5));
    let exact = "y".repeat(LINE_MAX_CHARS);
    assert_eq!(display_line(&exact), (exact, 0));
    // multibyte characters are counted as characters, not bytes
    let jp = "あ".repeat(LINE_MAX_CHARS + 1);
    assert_eq!(display_line(&jp), ("あ".repeat(LINE_MAX_CHARS), 1));
    // the old English marker is just text now: a line that ends with it is shown as itself, uncut
    let looks = "see \u{2026}(500 more characters)";
    assert_eq!(display_line(looks), (looks.to_owned(), 0));
}
