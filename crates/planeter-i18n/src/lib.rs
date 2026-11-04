#![forbid(unsafe_code)]
//! planeter's languages as data (RFC 012 D-3, amended 2026-11-04; handoff `language-registry-handoff-v1`).
//!
//! A **language is a row of data** ([`Language`]): its BCP 47 tag, its name in itself, its direction, the plural categories it uses
//! and the function that picks one, its number separators, and its [`Status`] — the source language, a draft, or **signed off**, with
//! the record of who signed what, when. The registry ([`languages::LANGUAGES`]) is the only list of languages; the server and the
//! command-line program each keep only their own message tables ([`Entry`]) and key enums, and read everything else from here:
//!
//! * [`plural`] — the CLDR categories and the rule functions (English, Japanese, and Russian, Polish, Arabic and Welsh as tested
//!   functions with no catalogs, to prove the machinery before the first real language that needs it);
//! * [`tag`] — BCP 47 negotiation: exact, then same language and script, then same language, else the source language;
//! * [`mod@audit`] — completeness and consistency of a set of catalogs (the same keys, the same placeholders, exactly the forms a language
//!   declares, no markup);
//! * [`baseline`] — what a sign-off covered: a hash of every message as signed, and the count of messages newer than it;
//! * [`rules`] — a language's own rules as data (`<tag>.rules`), each shown to fail by its own planted examples.
//!
//! No external dependency, no `unsafe`.

pub mod audit;
pub mod baseline;
pub mod languages;
pub mod plural;
pub mod rules;
pub mod tag;

pub use audit::{Catalog, Options, Problem, audit, audit_with};
pub use plural::Category;

/// Text direction of a language (`dir` on `<html>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Ltr,
    Rtl,
}

/// What a sign-off recorded: who read the language's messages, when, which commit and which review sheet (its fingerprint), and
/// anything the record must say plainly.
#[derive(Debug, PartialEq, Eq)]
pub struct SignOff {
    /// Names or roles of the people who read it (at least one; at least one a fluent reader).
    pub reviewers: &'static [&'static str],
    /// `YYYY-MM-DD`.
    pub date: &'static str,
    /// The commit of the corrections the sign-off covers.
    pub commit: &'static str,
    /// The review sheet's catalog fingerprint: sixteen lower-case hex digits.
    pub sheet_fingerprint: &'static str,
    pub note: &'static str,
}

/// Where a language stands.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// The language the messages are written in; the fallback of negotiation. Shipped.
    Source,
    /// Compiled in and checked, not signed off: offered only when the operator turns the preview on.
    Draft,
    /// Reviewed and signed off: shipped like the source language.
    SignedOff(SignOff),
}

/// A language, as data.
#[derive(Debug)]
pub struct Language {
    /// The BCP 47 tag: `en`, `ja`, `zh-Hant`, `pt-BR`. Also the cookie value and `<html lang>`.
    pub tag: &'static str,
    /// The language's name in itself, for the switcher.
    pub endonym: &'static str,
    pub dir: Dir,
    /// The plural categories the language uses, in order, ending with [`Category::Other`] (required).
    pub categories: &'static [Category],
    /// The category of a count.
    pub plural: fn(u64) -> Category,
    pub group_separator: char,
    pub decimal_separator: char,
    pub status: Status,
}

impl Language {
    /// Whether a build ships it without any operator setting: the source language and the signed-off ones.
    pub fn is_shipped(&self) -> bool {
        matches!(self.status, Status::Source | Status::SignedOff(_))
    }

    pub fn is_draft(&self) -> bool {
        matches!(self.status, Status::Draft)
    }

    pub fn is_source(&self) -> bool {
        matches!(self.status, Status::Source)
    }

    pub fn sign_off(&self) -> Option<&SignOff> {
        match &self.status {
            Status::SignedOff(s) => Some(s),
            _ => None,
        }
    }

    /// The BCP 47 primary language subtag (`zh` of `zh-Hant`).
    pub fn language_subtag(&self) -> &str {
        self.tag.split('-').next().unwrap_or(self.tag)
    }
}

/// A language together with one program's messages for it (the server's catalog, or the command-line program's). The language's
/// fields are reached through `Deref`.
#[derive(Debug)]
pub struct LangDef {
    pub lang: &'static Language,
    pub entries: &'static [Entry],
}

impl LangDef {
    pub const fn new(lang: &'static Language, entries: &'static [Entry]) -> LangDef {
        LangDef { lang, entries }
    }

    /// The catalog view the audit takes.
    pub fn catalog(&self) -> Catalog<'_> {
        Catalog {
            lang: self.lang,
            entries: self.entries,
        }
    }
}

impl std::ops::Deref for LangDef {
    type Target = Language;
    fn deref(&self) -> &Language {
        self.lang
    }
}

/// One message: the key and its text for each plural category the language uses (`other` always).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: &'static str,
    forms: [Option<&'static str>; 6],
}

/// A message with no plural forms.
pub const fn msg(key: &'static str, text: &'static str) -> Entry {
    Entry {
        key,
        forms: [None, None, None, None, None, Some(text)],
    }
}

/// A message with a singular and an other form (it must contain `{count}`).
pub const fn plural_msg(key: &'static str, one: &'static str, other: &'static str) -> Entry {
    Entry {
        key,
        forms: [None, Some(one), None, None, None, Some(other)],
    }
}

impl Entry {
    /// A message with the given forms (`Other` is required; the audit checks the rest against the language).
    pub fn with_forms(key: &'static str, forms: &[(Category, &'static str)]) -> Entry {
        let mut all = [None; 6];
        for (category, text) in forms {
            all[category.index()] = Some(*text);
        }
        Entry { key, forms: all }
    }

    /// The text of one category, if the entry has that form.
    pub fn form(&self, category: Category) -> Option<&'static str> {
        self.forms[category.index()]
    }

    /// Every form the entry has, in category order.
    pub fn forms(&self) -> impl Iterator<Item = (Category, &'static str)> + '_ {
        Category::ALL
            .into_iter()
            .filter_map(|c| self.form(c).map(|t| (c, t)))
    }

    /// The `other` form (every entry has it; an entry without one reads as empty, and the audit reports it).
    pub fn other(&self) -> &'static str {
        self.form(Category::Other).unwrap_or("")
    }

    /// The singular form, or the `other` form for a language with none.
    pub fn one(&self) -> &'static str {
        self.form(Category::One).unwrap_or_else(|| self.other())
    }

    /// The text for a category, falling back to `other`.
    pub fn text_for(&self, category: Category) -> &'static str {
        self.form(category).unwrap_or_else(|| self.other())
    }

    /// The same entry with every form passed through `f` (for a planted or wrapped catalog).
    pub fn map_forms(&self, f: impl Fn(&'static str) -> &'static str) -> Entry {
        let mut forms = self.forms;
        for text in forms.iter_mut().flatten() {
            *text = f(text);
        }
        Entry {
            key: self.key,
            forms,
        }
    }
}

/// Every placeholder name in a message text, sorted and de-duplicated.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        match rest[i..].find('}') {
            Some(j) => {
                v.push(&rest[i + 1..i + j]);
                rest = &rest[i + j + 1..];
            }
            None => break,
        }
    }
    v.sort_unstable();
    v.dedup();
    v
}
