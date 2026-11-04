//! A language's own rules as data (`<tag>.rules`), so that a rule is a line in a file and not Rust in a test, and so that a new
//! language brings its rules the way it brings its catalog.
//!
//! One rule per line, six tab-separated columns (`#` starts a comment; a blank line is ignored):
//!
//! ```text
//! kind    scope    pattern    reason    bad examples    good examples
//! ```
//!
//! * **`forbidden`** — the pattern may not appear in a message (outside its `{placeholders}`). The pattern is a substring, or with a
//!   mode prefix `word:` / `phrase:` (case-insensitive, whole words: only ASCII letters, digits and `_` join a word), or `label:` (the
//!   whole message is exactly this). `A!B` means "A, not as a part of B" (B is removed first).
//! * **`required-pair`** — the message with exactly this key must contain the pattern (a pair of key and text that must stay).
//! * **`style`** — a named check; arguments follow after colons: `fullwidth-brackets`, `counter-space:件,時間`,
//!   `qualified:WORD:QUALIFIER`, `only-in:WORD:KEY,KEY`, `no-ascii-punct-after-wide`, and
//!   `needs-context:WORDS:CONTEXT:EXEMPT_KEY_PREFIXES:CONTEXT_KEY_PREFIX` (WORDS may not be said about CONTEXT outside the exempt keys).
//!   Style checks look at the raw text, except `needs-context`, which looks outside the placeholders.
//!
//! `scope` is `*` (every key), a key prefix (`error.`), or for `required-pair` the exact key; a `client:` or `server:` before it limits
//! the rule to that program's catalog. **Every rule carries planted examples** (`|`-separated; an example is a text, or `key::text` for a
//! rule that depends on the key): each *bad* example must break the rule and each *good* one must not, so that a rule is shown able to
//! fail ([`selftest`]).

use crate::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Forbidden,
    RequiredPair,
    Style,
}

/// Which program's catalog is being checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Server,
    Client,
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// The line in the rules file (1-based).
    pub line: usize,
    pub kind: Kind,
    /// The scope without a program marker.
    pub scope: String,
    pub program: Option<Program>,
    pub pattern: String,
    pub reason: String,
    pub bad: Vec<String>,
    pub good: Vec<String>,
}

impl Rule {
    /// The term an allowlist entry names: the pattern without its mode prefix and without its exceptions.
    pub fn term(&self) -> &str {
        let p = ["word:", "phrase:", "label:"]
            .iter()
            .find_map(|m| self.pattern.strip_prefix(m))
            .unwrap_or(&self.pattern);
        p.split('!').next().unwrap_or(p)
    }

    fn applies(&self, program: Program, key: &str) -> bool {
        if self.program.is_some_and(|p| p != program) {
            return false;
        }
        match self.kind {
            Kind::RequiredPair => key == self.scope,
            _ => self.scope == "*" || key.starts_with(&self.scope),
        }
    }
}

/// Read a rules file. A malformed line, an unknown kind or style name, a rule without a reason or without a bad and a good example
/// are errors.
pub fn parse(text: &str) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |m: &str| format!("line {}: {m}", n + 1);
        let cols: Vec<&str> = line.split('\t').collect();
        let [kind, scope, pattern, reason, bad, good] = cols[..] else {
            return Err(at(
                "expected six tab-separated columns: kind, scope, pattern, reason, bad examples, good examples",
            ));
        };
        let kind = match kind {
            "forbidden" => Kind::Forbidden,
            "required-pair" => Kind::RequiredPair,
            "style" => Kind::Style,
            other => {
                return Err(at(&format!(
                    "unknown kind {other:?} (forbidden, required-pair, style)"
                )));
            }
        };
        let (program, scope) = if let Some(s) = scope.strip_prefix("client:") {
            (Some(Program::Client), s)
        } else if let Some(s) = scope.strip_prefix("server:") {
            (Some(Program::Server), s)
        } else {
            (None, scope)
        };
        if scope.is_empty() || pattern.is_empty() || reason.trim().is_empty() {
            return Err(at("scope, pattern and reason must not be empty"));
        }
        let split = |s: &str| -> Vec<String> {
            s.split('|')
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let (bad, good) = (split(bad), split(good));
        if bad.is_empty() || good.is_empty() {
            return Err(at(
                "a rule needs at least one bad and one good planted example",
            ));
        }
        if kind == Kind::RequiredPair && scope == "*" {
            return Err(at("a required-pair names one key"));
        }
        if kind == Kind::Style && style_check(pattern, "k", "").is_none() {
            return Err(at(&format!("unknown style check {pattern:?}")));
        }
        rules.push(Rule {
            line: n + 1,
            kind,
            scope: scope.to_owned(),
            program,
            pattern: pattern.to_owned(),
            reason: reason.to_owned(),
            bad,
            good,
        });
    }
    Ok(rules)
}

/// The text without its `{placeholders}` (their names are code, not words).
pub fn without_placeholders(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether `word` occurs in `text` as a whole word, ignoring case. Only ASCII letters, digits and `_` join a word: Japanese text has
/// no spaces, so `SSOでログイン` still contains `SSO`.
pub fn contains_word(text: &str, word: &str) -> bool {
    let (text, word) = (text.to_lowercase(), word.to_lowercase());
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(&word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

/// Run a style check on a text. `None` when the check's name or arguments are not known; `Some(true)` when the text breaks it.
fn style_check(spec: &str, key: &str, text: &str) -> Option<bool> {
    let mut parts = spec.split(':');
    let name = parts.next()?;
    let args: Vec<&str> = parts.collect();
    let wide = |c: char| !c.is_ascii();
    let chars: Vec<char> = text.chars().collect();
    match (name, args.as_slice()) {
        ("fullwidth-brackets", []) => Some(chars.iter().enumerate().any(|(i, c)| {
            matches!(c, '(' | ')')
                && [i.checked_sub(1), Some(i + 1)]
                    .into_iter()
                    .flatten()
                    .filter_map(|j| chars.get(j))
                    .any(|n| wide(*n))
        })),
        ("counter-space", [counters]) => {
            let counters: Vec<&str> = counters.split(',').filter(|c| !c.is_empty()).collect();
            if counters.is_empty() {
                return None;
            }
            Some(text.char_indices().any(|(at, _)| {
                counters.iter().any(|counter| {
                    text[at..].starts_with(counter)
                        && text[..at]
                            .chars()
                            .next_back()
                            .is_some_and(|p| p.is_ascii_digit() || p == '}')
                })
            }))
        }
        ("qualified", [word, qualifier]) => Some(
            text.match_indices(word)
                .any(|(at, _)| !text[..at].ends_with(qualifier)),
        ),
        ("only-in", [word, keys]) => {
            let allowed: Vec<&str> = keys.split(',').collect();
            Some(text.contains(word) && !allowed.contains(&key))
        }
        // WORDS may not be said in a message about CONTEXT (a context word in the text, or a key starting with CONTEXT_KEY) unless the
        // key starts with one of the EXEMPT prefixes. ASCII words are whole words, ignoring case; other words are substrings.
        ("needs-context", [words, context, exempt, context_key]) => {
            let hit = |list: &str, t: &str| {
                list.split(',').filter(|w| !w.is_empty()).any(|w| {
                    if w.is_ascii() {
                        contains_word(t, w)
                    } else {
                        t.contains(w)
                    }
                })
            };
            let text = without_placeholders(text);
            let exempt_key = exempt
                .split(',')
                .filter(|p| !p.is_empty())
                .any(|p| key.starts_with(p));
            let in_context =
                hit(context, &text) || (!context_key.is_empty() && key.starts_with(context_key));
            Some(!exempt_key && in_context && hit(words, &text))
        }
        ("no-ascii-punct-after-wide", []) => Some(
            chars
                .iter()
                .enumerate()
                .any(|(i, c)| matches!(c, ',' | '?' | '!') && i > 0 && wide(chars[i - 1])),
        ),
        _ => None,
    }
}

/// Whether one message form breaks one rule (the key matters for `required-pair` and `only-in`).
pub fn violates(rule: &Rule, key: &str, form: &str) -> bool {
    match rule.kind {
        Kind::Forbidden => {
            let text = without_placeholders(form);
            let (head, excepts) = match rule.pattern.split_once('!') {
                Some((h, e)) => (h, e),
                None => (rule.pattern.as_str(), ""),
            };
            let mut text = text;
            for except in excepts.split('!').filter(|e| !e.is_empty()) {
                text = text.replace(except, "");
            }
            if let Some(w) = head
                .strip_prefix("word:")
                .or_else(|| head.strip_prefix("phrase:"))
            {
                contains_word(&text, w)
            } else if let Some(l) = head.strip_prefix("label:") {
                text.trim() == l
            } else {
                text.contains(head)
            }
        }
        Kind::RequiredPair => !form.contains(&rule.pattern),
        Kind::Style => style_check(&rule.pattern, key, form).unwrap_or(false),
    }
}

/// An allowlist entry: this message may break the rule for this term (with a reason in the file).
#[derive(Debug, PartialEq, Eq)]
pub struct Allowed {
    pub key: String,
    pub term: String,
}

/// Read an allowlist: `key :: term :: reason` per line.
pub fn parse_allow(src: &str) -> Result<Vec<Allowed>, String> {
    let mut out = Vec::new();
    for line in src.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, " :: ").collect();
        if parts.len() != 3 || parts.iter().any(|p| p.trim().is_empty()) {
            return Err(format!("needs key :: term :: reason: {line}"));
        }
        out.push(Allowed {
            key: parts[0].trim().to_owned(),
            term: parts[1].trim().to_owned(),
        });
    }
    Ok(out)
}

/// What [`check`] found.
#[derive(Debug, Default)]
pub struct Report {
    pub problems: Vec<String>,
    /// For each allowlist entry, whether a message used it.
    pub allow_used: Vec<bool>,
}

impl Report {
    /// The allowlist entries nothing needed.
    pub fn stale<'a>(&self, allow: &'a [Allowed]) -> Vec<&'a Allowed> {
        allow
            .iter()
            .zip(&self.allow_used)
            .filter(|(_, u)| !**u)
            .map(|(a, _)| a)
            .collect()
    }
}

/// Check a catalog (`lang_tag` for the messages) against the rules of its language and program, with an allowlist.
pub fn check(
    rules: &[Rule],
    program: Program,
    lang_tag: &str,
    entries: &[Entry],
    allow: &[Allowed],
) -> Report {
    let mut report = Report {
        problems: Vec::new(),
        allow_used: vec![false; allow.len()],
    };
    for e in entries {
        for rule in rules.iter().filter(|r| r.applies(program, e.key)) {
            let mut forms: Vec<&'static str> = e.forms().map(|(_, t)| t).collect();
            if rule.kind == Kind::RequiredPair {
                forms = vec![e.other()];
            }
            for form in forms {
                if !violates(rule, e.key, form) {
                    continue;
                }
                let term = rule.term().to_owned();
                match allow.iter().position(|a| a.key == e.key && a.term == term) {
                    Some(i) => report.allow_used[i] = true,
                    None => report.problems.push(format!(
                        "{lang_tag} `{}` breaks the rule {:?} (line {}: {}): {form:?}",
                        e.key, rule.pattern, rule.line, rule.reason
                    )),
                }
                break;
            }
        }
    }
    report
}

/// The keys the `required-pair` rules of a program name (a test checks each exists in that program's catalog).
pub fn required_keys(rules: &[Rule], program: Program) -> Vec<&str> {
    rules
        .iter()
        .filter(|r| r.kind == Kind::RequiredPair && r.program.is_none_or(|p| p == program))
        .map(|r| r.scope.as_str())
        .collect()
}

/// Every rule shown able to fail: each bad example breaks it, each good one does not. Problems, empty when the rules are sound.
pub fn selftest(rules: &[Rule]) -> Vec<String> {
    let mut problems = Vec::new();
    for rule in rules {
        let split = |example: &str| -> (String, String) {
            match example.split_once("::") {
                Some((k, t)) => (k.to_owned(), t.to_owned()),
                None => (
                    if rule.kind == Kind::RequiredPair {
                        rule.scope.clone()
                    } else {
                        "planted.key".to_owned()
                    },
                    example.to_owned(),
                ),
            }
        };
        for example in &rule.bad {
            let (key, text) = split(example);
            if !violates(rule, &key, &text) {
                problems.push(format!(
                    "line {}: the bad example {example:?} does not break {:?}",
                    rule.line, rule.pattern
                ));
            }
        }
        for example in &rule.good {
            let (key, text) = split(example);
            if violates(rule, &key, &text) {
                problems.push(format!(
                    "line {}: the good example {example:?} breaks {:?}",
                    rule.line, rule.pattern
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{msg, plural_msg};

    fn rule(line: &str) -> Rule {
        parse(line).unwrap().remove(0)
    }

    #[test]
    fn a_file_is_read_and_a_malformed_line_is_refused() {
        let ok = "# c\n\nforbidden\t*\tword:refs\twhy\tShow refs\tUse {refs}\n";
        assert_eq!(parse(ok).unwrap().len(), 1);
        for bad in [
            "forbidden\t*\tx\twhy\tb",
            "wrong\t*\tx\twhy\tb\tg",
            "forbidden\t*\tx\t\tb\tg",
            "forbidden\t\tx\twhy\tb\tg",
            "forbidden\t*\tx\twhy\t\tg",
            "forbidden\t*\tx\twhy\tb\t",
            "required-pair\t*\tx\twhy\tb\tg",
            "style\t*\tno-such-check\twhy\tb\tg",
            "style\t*\tcounter-space:\twhy\tb\tg",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn words_are_whole_ascii_words_ignoring_case_and_placeholder_names_are_code() {
        let r = rule("forbidden\t*\tword:refs\twhy\tx\ty\n");
        for bad in ["Show refs · {repo}", "Refs", "SSOでrefs"] {
            assert!(violates(&r, "k", bad), "{bad}");
        }
        for good in [
            "Use {refs} here",
            "Your prefs are saved.",
            "refsx",
            "x_refs",
        ] {
            assert!(!violates(&r, "k", good), "{good}");
        }
        let sso = rule("forbidden\t*\tword:SSO\twhy\tx\ty\n");
        assert!(violates(&sso, "k", "SSOでログイン") && violates(&sso, "k", "sign in with sso"));
        assert!(!violates(&sso, "k", "Sign in with single sign-on"));
    }

    #[test]
    fn phrases_labels_substrings_and_exceptions() {
        let phrase = rule("forbidden\t*\tphrase:open changes\twhy\tx\ty\n");
        assert!(
            violates(&phrase, "k", "You already have 3 Open Changes.")
                && !violates(&phrase, "k", "A recorded change.")
        );
        let label = rule("forbidden\t*\tlabel:code\twhy\tx\ty\n");
        assert!(
            violates(&label, "k", " code ") && !violates(&label, "k", "The error code is shown.")
        );
        let sub = rule("forbidden\t*\tサインイン!シングルサインオン\twhy\tx\ty\n");
        assert!(
            violates(&sub, "k", "サインインしてください")
                && !violates(&sub, "k", "シングルサインオンでログイン")
        );
        assert_eq!(sub.term(), "サインイン");
        assert_eq!(phrase.term(), "open changes");
    }

    #[test]
    fn style_checks_find_their_breach_and_let_the_right_form_through() {
        let cases: [(&str, &str, &str); 7] = [
            ("fullwidth-brackets", "履歴(読む範囲)", "履歴（読む範囲）"),
            (
                "counter-space:件,時間",
                "上限は{limit}件です",
                "上限は {limit} 件です",
            ),
            ("counter-space:件,時間", "この1時間に", "この 1 時間に"),
            (
                "qualified:管理者:リポジトリの",
                "管理者だけです",
                "リポジトリの管理者だけです",
            ),
            ("only-in:点検:a.k,b.k", "点検します", "a.k::点検します"),
            (
                "no-ascii-punct-after-wide",
                "いいえ,そうです",
                "いいえ、そうです。 yes, ok",
            ),
            ("fullwidth-brackets", "あ(b)", "a (b)"),
        ];
        for (spec, bad, good) in cases {
            let r = rule(&format!("style\t*\t{spec}\twhy\t{bad}\t{good}\n"));
            let (bk, bt) = bad.split_once("::").unwrap_or(("planted.key", bad));
            assert!(violates(&r, bk, bt), "{spec}: {bad}");
            let (gk, gt) = good.split_once("::").unwrap_or(("planted.key", good));
            assert!(!violates(&r, gk, gt), "{spec}: {good}");
        }
        // `only-in` is key-dependent: the same text in another key breaks it.
        let only = rule("style\t*\tonly-in:点検:a.k\twhy\tz.k::点検\ta.k::点検\n");
        assert!(violates(&only, "z.k", "点検") && !violates(&only, "a.k", "点検"));
    }

    #[test]
    fn selftest_fails_a_rule_whose_planted_examples_do_not_behave() {
        let sound = parse("forbidden\t*\tword:refs\twhy\tShow refs\tUse {refs}\n").unwrap();
        assert_eq!(selftest(&sound), Vec::<String>::new());
        let blind = parse("forbidden\t*\tword:refs\twhy\tNothing here\tUse {refs}\n").unwrap();
        assert!(
            selftest(&blind)
                .iter()
                .any(|p| p.contains("does not break"))
        );
        let hasty = parse("forbidden\t*\tword:refs\twhy\tShow refs\tShow refs too\n").unwrap();
        assert!(selftest(&hasty).iter().any(|p| p.contains("breaks")));
    }

    #[test]
    fn a_catalog_is_checked_by_scope_program_and_allowlist_and_a_dead_allowlist_line_is_found() {
        let rules = parse(
            "forbidden\terror.\tword:refs\twhy\tx refs\ty\n\
             required-pair\tkeep.this\tsigned\twhy\tnothing\tsigned it\n\
             required-pair\tclient:cli.only\tok\twhy\tno\tok\n\
             forbidden\tclient:*\tbanned\twhy\tbanned\tfine\n",
        )
        .unwrap();
        let entries = [
            msg("error.a", "Show refs"),
            msg("other.b", "Show refs"),
            msg("keep.this", "not it"),
            plural_msg("error.c", "{count} refs", "{count} thing"),
        ];
        let problems = |program, allow: &[Allowed]| check(&rules, program, "xx", &entries, allow);
        let p = problems(Program::Server, &[]).problems;
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(
            p.iter().any(|m| m.contains("`error.a`"))
                && p.iter().any(|m| m.contains("`error.c`"))
                && p.iter().any(|m| m.contains("`keep.this`"))
        );
        // The client's rules do not apply to the server's catalog, and the server's scope rules do not apply to the client's.
        let c = problems(Program::Client, &[]).problems;
        assert!(
            c.iter().all(|m| !m.contains("cli.only")) && c.iter().any(|m| m.contains("`error.a`"))
        );
        assert_eq!(required_keys(&rules, Program::Server), ["keep.this"]);
        assert_eq!(
            required_keys(&rules, Program::Client),
            ["keep.this", "cli.only"]
        );
        // An allowlist line silences exactly its message and term; a dead one is found.
        let allow = parse_allow(
            "error.a :: refs :: the name of prikk's own data\nghost.key :: refs :: gone\n",
        )
        .unwrap();
        let report = problems(Program::Server, &allow);
        assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
        assert_eq!(report.stale(&allow).len(), 1);
        assert_eq!(report.stale(&allow)[0].key, "ghost.key");
        assert!(parse_allow("a :: refs\n").is_err() && parse_allow("a :: refs :: \n").is_err());
    }

    #[test]
    fn needs_context_flags_a_word_about_a_context_outside_the_exempt_keys() {
        let r = rule(
            "style\t*\tneeds-context:approve,approved,承認:proposal,提案:review.,decision.:proposal\twhy\tproposal.x::It was approved\treview.line::Approved by 2\n",
        );
        assert!(violates(&r, "proposal.x", "It was approved"));
        assert!(violates(
            &r,
            "other.y",
            "This proposal was approved on this server."
        ));
        assert!(violates(&r, "other.y", "この提案は承認されました。"));
        assert!(
            !violates(&r, "review.line", "Approved by 2 people"),
            "exempt key"
        );
        assert!(
            !violates(&r, "assurance.queued", "Approved, awaiting signature"),
            "not about a proposal"
        );
        assert!(
            !violates(&r, "proposal.x", "The {approved} one"),
            "a placeholder name is code"
        );
        assert!(!violates(&r, "proposal.x", "Disapproved"), "whole words");
    }
}
