//! RFC 012 D-5, the catalog half: checks that fail the build. Each is also shown able to fail (the planted
//! cases below; the review request lists the source mutations that were applied and restored).
//!
//! * the catalogs: every key in every shipped language, no orphan or duplicate key, the same placeholders
//!   everywhere, plural forms that fit the language, no markup, entity or URL in a message (`audit`);
//! * every key is used by a template or by Rust outside the catalog;
//! * no template or source file contains user-visible literal text outside `tests/literals.allow`;
//! * `|safe` is never applied to catalog output.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use planeter_web::catalog::{self, Dir, Entry, Key, LangDef, Plural, Problem, audit, shipped};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == ext))
        .collect();
    out.sort();
    out
}

/// The Rust sources that can hold messages and literals: `src/*.rs` and the UI's sub-modules `src/ui/*.rs`.
fn rust_sources() -> Vec<PathBuf> {
    let mut out = files(&root().join("src"), "rs");
    out.extend(files(&root().join("src/ui"), "rs"));
    out
}

// ---------------------------------------------------------------- the catalogs

/// Every message is translated until the owner decides otherwise (`tests/english_only.rs` shows the mechanism with a planted list).
#[test]
fn no_message_is_english_only_by_default() {
    assert!(
        catalog::ENGLISH_ONLY_KEYS.is_empty(),
        "ENGLISH_ONLY_KEYS is empty until the owner decides that a group stays English"
    );
    assert_eq!(catalog::english_only_keys(), catalog::ENGLISH_ONLY_KEYS);
}

#[test]
fn the_shipped_catalogs_are_complete_consistent_and_free_of_markup() {
    assert_eq!(
        audit(shipped()),
        vec![],
        "the shipped catalogs have problems"
    );
}

// A language built for the planted cases: all of English, then one thing wrong.
fn leaked(entries: Vec<Entry>) -> &'static [Entry] {
    Box::leak(entries.into_boxed_slice())
}
fn derived(
    tag: &'static str,
    f: impl Fn(&Entry) -> Option<Entry>,
    plural: fn(u64) -> Plural,
) -> &'static LangDef {
    let entries: Vec<Entry> = shipped()[0].entries.iter().filter_map(f).collect();
    Box::leak(Box::new(LangDef {
        tag,
        endonym: tag,
        dir: Dir::Ltr,
        plural,
        group_separator: ',',
        decimal_separator: '.',
        entries: leaked(entries),
    }))
}
fn copy(e: &Entry) -> Entry {
    Entry {
        key: e.key,
        one: e.one,
        other: e.other,
    }
}
fn en_plural(n: u64) -> Plural {
    if n == 1 { Plural::One } else { Plural::Other }
}
fn only_other(_: u64) -> Plural {
    Plural::Other
}

#[test]
fn a_key_missing_in_a_language_is_found() {
    let l = derived(
        "t1",
        |e| (e.key != "login.failed").then(|| copy(e)),
        en_plural,
    );
    let p = audit(&[shipped()[0], l]);
    assert!(
        p.contains(&Problem::Missing {
            lang: "t1",
            key: "login.failed"
        }),
        "{p:?}"
    );
}

#[test]
fn a_key_no_enum_variant_names_is_an_orphan() {
    let l = derived("t2", |e| Some(copy(e)), en_plural);
    let mut v: Vec<Entry> = l.entries.iter().map(copy).collect();
    v.push(Entry {
        key: "spare.unused",
        one: "x",
        other: "x",
    });
    let l2: &'static LangDef = Box::leak(Box::new(LangDef {
        entries: leaked(v),
        ..clone_def(l)
    }));
    let p = audit(&[shipped()[0], l2]);
    assert!(
        p.contains(&Problem::Orphan {
            lang: "t2",
            key: "spare.unused"
        }),
        "{p:?}"
    );
}

fn clone_def(d: &LangDef) -> LangDef {
    LangDef {
        tag: d.tag,
        endonym: d.endonym,
        dir: d.dir,
        plural: d.plural,
        group_separator: d.group_separator,
        decimal_separator: d.decimal_separator,
        entries: d.entries,
    }
}

#[test]
fn a_duplicate_key_is_found() {
    let mut v: Vec<Entry> = shipped()[0].entries.iter().map(copy).collect();
    v.push(copy(&shipped()[0].entries[0]));
    let l: &'static LangDef = Box::leak(Box::new(LangDef {
        tag: "t3",
        entries: leaked(v),
        ..clone_def(shipped()[0])
    }));
    let p = audit(&[l]);
    assert!(
        p.iter()
            .any(|x| matches!(x, Problem::Duplicate { lang: "t3", .. })),
        "{p:?}"
    );
}

#[test]
fn a_placeholder_set_that_differs_from_english_is_found() {
    let l = derived(
        "t4",
        |e| {
            let mut c = copy(e);
            if e.key == "history.heading" {
                c.one = "History {refs}";
                c.other = "History {refs}";
            }
            Some(c)
        },
        only_other,
    );
    let p = audit(&[shipped()[0], l]);
    assert!(
        p.contains(&Problem::PlaceholdersDiffer {
            key: "history.heading",
            lang: "t4"
        }),
        "{p:?}"
    );
}

#[test]
fn plural_forms_that_do_not_fit_the_language_are_found() {
    // A language with only `other` that still defines two forms of a counted message…
    let two_forms = derived("t5", |e| Some(copy(e)), only_other);
    let p = audit(&[shipped()[0], two_forms]);
    assert!(
        p.contains(&Problem::PluralFormsDisagree {
            key: "history.patch_count",
            lang: "t5"
        }),
        "{p:?}"
    );
    // …and a language with a singular that forgot to define it.
    let forgot = derived(
        "t6",
        |e| {
            let mut c = copy(e);
            if e.key == "history.patch_count" {
                c.one = c.other;
            }
            Some(c)
        },
        en_plural,
    );
    let p = audit(&[shipped()[0], forgot]);
    assert!(
        p.contains(&Problem::PluralFormsDisagree {
            key: "history.patch_count",
            lang: "t6"
        }),
        "{p:?}"
    );
}

#[test]
fn markup_an_entity_or_a_url_in_a_message_is_found() {
    for (tag, text) in [
        ("t7", "Sign in <b>now</b>."),
        ("t8", "Fish &amp; chips."),
        ("t9", "See https://example.invalid/."),
    ] {
        let l = derived(
            tag,
            |e| {
                let mut c = copy(e);
                if e.key == "login.failed" {
                    c.one = text;
                    c.other = text;
                }
                Some(c)
            },
            en_plural,
        );
        let p = audit(&[shipped()[0], l]);
        assert!(
            p.contains(&Problem::Markup {
                lang: tag,
                key: "login.failed"
            }),
            "{text}: {p:?}"
        );
    }
}

// ---------------------------------------------------------------- use of keys

fn sources_outside_the_catalog() -> String {
    let mut all = String::new();
    for p in files(&root().join("templates"), "html") {
        all += &read(&p);
    }
    for p in rust_sources() {
        all += &read(&p);
    }
    // The catalog module uses a few keys itself (sizes); its key table (`Variant => "id"`) never says `Key::`.
    all += &read(&root().join("src/catalog/mod.rs"));
    all
}

#[test]
fn every_key_is_used_by_a_template_or_by_rust_outside_the_catalog() {
    let text = sources_outside_the_catalog();
    let unused: Vec<&str> = Key::ALL
        .iter()
        .filter(|k| !text.contains(&format!("Key::{}", k.variant())))
        .map(|k| k.id())
        .collect();
    assert!(
        unused.is_empty(),
        "keys no template or Rust path uses: {unused:?}"
    );
}

// ---------------------------------------------------------------- literal text

struct Allow {
    used: std::cell::RefCell<BTreeSet<String>>,
    exact: BTreeSet<(String, String)>,
    re: Vec<(String, regex_lite::Re, String)>,
    safe: BTreeSet<(String, String)>,
    scope: BTreeSet<String>,
}

fn allowlist() -> Allow {
    let mut a = Allow {
        used: Default::default(),
        exact: BTreeSet::new(),
        re: Vec::new(),
        safe: BTreeSet::new(),
        scope: BTreeSet::new(),
    };
    for line in read(&root().join("tests/literals.allow")).lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, " :: ").collect();
        assert_eq!(
            parts.len(),
            4,
            "allowlist line needs kind :: file :: literal :: reason: {line}"
        );
        assert!(
            !parts[3].trim().is_empty(),
            "allowlist line without a reason: {line}"
        );
        match parts[0] {
            "exact" => {
                a.exact.insert((parts[1].to_owned(), parts[2].to_owned()));
            }
            "re" => a.re.push((
                parts[1].to_owned(),
                regex_lite::Re::new(parts[2]),
                parts[2].to_owned(),
            )),
            "safe" => {
                a.safe.insert((parts[1].to_owned(), parts[2].to_owned()));
            }
            "scope" => {
                a.scope.insert(parts[1].to_owned());
            }
            other => panic!("unknown allowlist kind {other}"),
        }
    }
    a
}

impl Allow {
    fn literal_ok(&self, file: &str, lit: &str) -> bool {
        if self.exact.contains(&(file.to_owned(), lit.to_owned())) {
            self.used
                .borrow_mut()
                .insert(format!("exact::{file}::{lit}"));
            return true;
        }
        if let Some((_, _, src)) = self
            .re
            .iter()
            .find(|(f, r, _)| (f == "*" || f == file) && r.is_match(lit))
        {
            self.used.borrow_mut().insert(format!("re::{src}"));
            return true;
        }
        false
    }

    /// Entries nothing matched: an allowlist that only grows hides what it once excused.
    fn unused(&self) -> Vec<String> {
        let used = self.used.borrow();
        let mut v: Vec<String> = self
            .exact
            .iter()
            .map(|(f, l)| format!("exact::{f}::{l}"))
            .chain(self.re.iter().map(|(_, _, s)| format!("re::{s}")))
            .filter(|k| !used.contains(k))
            .collect();
        v.sort();
        v
    }
}

/// A tiny anchored matcher for the handful of regex shapes the allowlist uses, so that this test needs no
/// new dependency: `^…$` alternations of literal words, `^/…`, `^\?…`, `.*` tails and `[a-z_]+` classes.
mod regex_lite {
    pub struct Re(String);
    impl Re {
        pub fn new(p: &str) -> Re {
            Re(p.to_owned())
        }
        pub fn is_match(&self, s: &str) -> bool {
            matches(self.0.trim_start_matches('^'), s, self.0.starts_with('^'))
        }
    }
    // Supported: literals, `\x` escapes, `.`, `.*`, `[a-z_]` with `+`/`*`/`?`, `\{ \}`, groups with `|`,
    // and a final `$`. Enough for literals.allow; a pattern outside this fails loudly.
    fn matches(p: &str, s: &str, anchored: bool) -> bool {
        let chars: Vec<char> = s.chars().collect();
        let pat: Vec<char> = p.chars().collect();
        let starts: Box<dyn Iterator<Item = usize>> = if anchored {
            Box::new(0..1)
        } else {
            Box::new(0..=chars.len())
        };
        for st in starts {
            if go(&pat, 0, &chars, st).is_some() {
                return true;
            }
        }
        false
    }
    fn go(p: &[char], pi: usize, s: &[char], si: usize) -> Option<()> {
        if pi >= p.len() {
            return Some(());
        }
        if p[pi] == '$' && pi + 1 == p.len() {
            return (si == s.len()).then_some(());
        }
        // group: ( a | b | c ) followed by an optional quantifier
        if p[pi] == '(' {
            let mut depth = 0;
            let mut end = pi;
            for (i, c) in p.iter().enumerate().skip(pi) {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let inner: String = p[pi + 1..end].iter().collect();
            for alt in inner.split('|') {
                let mut sub: Vec<char> = alt.chars().collect();
                sub.extend_from_slice(&p[end + 1..]);
                if go(&sub, 0, s, si).is_some() {
                    return Some(());
                }
            }
            return None;
        }
        // an atom
        let (atom_len, test): (usize, Box<dyn Fn(char) -> bool>) = match p[pi] {
            '\\' => {
                let c = p[pi + 1];
                (2, Box::new(move |x| x == c))
            }
            '.' => (1, Box::new(|_| true)),
            '[' => {
                let close = p[pi..].iter().position(|c| *c == ']').unwrap() + pi;
                let set: String = p[pi + 1..close].iter().collect();
                assert_eq!(set, "a-z_", "unsupported class {set}");
                (
                    close - pi + 1,
                    Box::new(|x: char| x.is_ascii_lowercase() || x == '_'),
                )
            }
            c => (1, Box::new(move |x| x == c)),
        };
        let next = pi + atom_len;
        match p.get(next) {
            Some('*') | Some('+') | Some('?') => {
                let q = p[next];
                let (min, max) = match q {
                    '*' => (0, usize::MAX),
                    '+' => (1, usize::MAX),
                    _ => (0, 1),
                };
                let mut n = 0;
                while si + n < s.len() && n < max && test(s[si + n]) {
                    n += 1;
                }
                loop {
                    if n >= min && go(p, next + 1, s, si + n).is_some() {
                        return Some(());
                    }
                    if n == 0 {
                        return None;
                    }
                    n -= 1;
                }
            }
            _ => {
                if si < s.len() && test(s[si]) {
                    go(p, next, s, si + 1)
                } else {
                    None
                }
            }
        }
    }
}

/// The string literals of a Rust source file that could reach a person: outside comments, outside the test
/// module, outside the macros that only log or abort (`eprintln!`, `.expect(`, `panic!`), with at least
/// three consecutive ASCII letters.
fn rust_literals(src: &str) -> Vec<(usize, String)> {
    let cut = src.find("#[cfg(test)]").unwrap_or(src.len());
    let mut out = Vec::new();
    for (i, line) in src[..cut].lines().enumerate() {
        let t = line.trim();
        if t.starts_with("//")
            || t.contains("eprintln!")
            || t.contains(".expect(")
            || t.contains("panic!")
        {
            continue;
        }
        let bytes: Vec<char> = line.chars().collect();
        let mut j = 0;
        while j < bytes.len() {
            if bytes[j] == '"' {
                let mut lit = String::new();
                j += 1;
                while j < bytes.len() && bytes[j] != '"' {
                    if bytes[j] == '\\' && j + 1 < bytes.len() {
                        lit.push(bytes[j + 1]);
                        j += 2;
                        continue;
                    }
                    lit.push(bytes[j]);
                    j += 1;
                }
                let letters = lit.chars().fold((0, 0), |(run, best), c| {
                    let run = if c.is_ascii_alphabetic() { run + 1 } else { 0 };
                    (run, best.max(run))
                });
                if letters.1 >= 3 {
                    out.push((i + 1, lit));
                }
            }
            j += 1;
        }
    }
    out
}

#[test]
fn no_source_file_holds_user_visible_literal_text_outside_the_allowlist() {
    let allow = allowlist();
    let mut found = Vec::new();
    for p in rust_sources() {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if allow.scope.contains(&name) {
            continue;
        }
        for (line, lit) in rust_literals(&read(&p)) {
            if !allow.literal_ok(&name, &lit) {
                found.push(format!("{name}:{line}: {lit:?}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "literal text outside the catalog and the allowlist:\n{}",
        found.join("\n")
    );
    assert_eq!(
        allow.unused(),
        Vec::<String>::new(),
        "allowlist entries nothing matches (remove them)"
    );
}

/// The pieces of a template: its text nodes (outside tags and askama tags), attribute values that are
/// words a person reads, and the string literals inside `{{ }}` expressions that are not placeholder names.
fn template_findings(name: &str, src: &str, allow: &Allow) -> Vec<String> {
    let mut found = Vec::new();
    // askama expressions and tags first: literals inside `{{ }}`, `|safe` uses
    let mut text = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("{{").into_iter().chain(rest.find("{%")).min() {
        text.push_str(&rest[..i]);
        text.push('\u{1}');
        let close = if rest[i..].starts_with("{{") {
            "}}"
        } else {
            "%}"
        };
        let j = rest[i..].find(close).expect("unterminated askama tag") + i;
        let inner = &rest[i + 2..j];
        if close == "}}" {
            if let Some(k) = inner.find("|safe") {
                let expr = inner[..k].trim();
                if !allow.safe.contains(&(name.to_owned(), expr.to_owned())) {
                    found.push(format!("{name}: `{expr}|safe` is not allowlisted (never use |safe on catalog output)"));
                }
            }
            // string literals: only placeholder names (the first argument of with/code/muted/link) and nothing else
            let b: Vec<char> = inner.chars().collect();
            let mut m = 0;
            while m < b.len() {
                if b[m] == '"' {
                    let start = m;
                    m += 1;
                    let mut lit = String::new();
                    while m < b.len() && b[m] != '"' {
                        lit.push(b[m]);
                        m += 1;
                    }
                    let before: String = b[..start].iter().collect();
                    let is_name = ["with(", "code(", "muted(", "link("]
                        .iter()
                        .any(|c| before.trim_end().ends_with(c));
                    if !is_name && lit.chars().any(|c| c.is_alphabetic()) {
                        found.push(format!("{name}: a literal {lit:?} inside an expression"));
                    }
                }
                m += 1;
            }
        }
        rest = &rest[j + 2..];
    }
    text.push_str(rest);
    // attributes that carry words
    for attr in ["placeholder", "title", "alt", "aria-label", "label"] {
        let needle = format!("{attr}=\"");
        let mut t = text.as_str();
        while let Some(i) = t.find(&needle) {
            let after = &t[i + needle.len()..];
            let val = &after[..after.find('"').unwrap()];
            if val.chars().any(|c| c.is_alphabetic()) && !val.contains('\u{1}') {
                found.push(format!("{name}: {attr}=\"{val}\" is literal text"));
            }
            t = after;
        }
    }
    // text nodes: strip tags
    let mut nodes = Vec::new();
    let mut cur = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                nodes.push(std::mem::take(&mut cur));
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => cur.push(c),
            _ => {}
        }
    }
    nodes.push(cur);
    for n in nodes {
        for piece in n.split('\u{1}') {
            let t = piece.trim();
            if t.chars().any(|c| c.is_alphabetic()) && !allow.literal_ok(name, t) {
                found.push(format!("{name}: literal text {t:?}"));
            }
        }
    }
    found
}

#[test]
fn no_template_holds_user_visible_literal_text_and_none_prints_catalog_output_with_safe() {
    let allow = allowlist();
    let mut found = Vec::new();
    for p in files(&root().join("templates"), "html") {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        found.extend(template_findings(&name, &read(&p), &allow));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_scanners_can_fail() {
    let allow = allowlist();
    // A hard-coded sentence in a template, a literal attribute, a literal inside an expression, |safe on a
    // catalog call: each is found.
    let bad = "<h1>Hello there</h1><input placeholder=\"owner/name\"><p>{{ t.get(Key::X) }} {{ \"Oops\" }}</p>{{ t.msg(Key::Y).html()|safe }}";
    let f = template_findings("x.html", bad, &allow);
    assert!(
        f.iter().any(|l| l.contains("literal text \"Hello there\"")),
        "{f:?}"
    );
    assert!(
        f.iter().any(|l| l.contains("placeholder=\"owner/name\"")),
        "{f:?}"
    );
    assert!(
        f.iter()
            .any(|l| l.contains("a literal \"Oops\" inside an expression")),
        "{f:?}"
    );
    assert!(f.iter().any(|l| l.contains("|safe")), "{f:?}");
    // …and a clean template is not flagged (separators and icons are not words).
    let ok = "<p>{{ a }} · <a href=\"/x\">{{ t.get(Key::X) }}</a> / 📁</p>";
    assert_eq!(
        template_findings("y.html", ok, &allow),
        Vec::<String>::new()
    );
    // A hard-coded sentence in Rust is found; a log line and a test module are not.
    let rs = "fn f() {\n respond(\"Something went wrong here\");\n eprintln!(\"planeter: a log line\");\n}\n#[cfg(test)]\nmod t { fn g() { x(\"Only in tests\"); } }\n";
    let lits = rust_literals(rs);
    assert_eq!(lits.len(), 1, "{lits:?}");
    assert_eq!(lits[0].1, "Something went wrong here");
    // The allowlist matcher: routes and templates pass, prose does not.
    assert!(allow.literal_ok("ui.rs", "/{owner}/{name}/refs"));
    assert!(allow.literal_ok("ui.rs", "repo.html"));
    assert!(!allow.literal_ok("ui.rs", "Something went wrong here"));
}

#[test]
fn the_default_language_is_english_and_only_signed_off_languages_are_shipped() {
    // English first; Japanese after its sign-off; nothing else (a test language registered by another test is not shipped).
    let tags: Vec<&str> = shipped().iter().map(|d| d.tag).collect();
    assert_eq!(
        tags,
        if catalog::JA_REVIEWED {
            vec!["en", "ja"]
        } else {
            vec!["en"]
        }
    );
    assert_eq!(catalog::EN.tag(), "en");
}
