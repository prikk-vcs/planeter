//! The completeness and consistency checks over a set of catalogs (RFC 012 D-5): every key in every language, no key a catalog has
//! and the program's key enum does not, no duplicates, the same placeholder names everywhere, **exactly the plural forms a language
//! declares** (each containing `{count}`), no markup, entity or URL in any message, and the "English in every language" list
//! (`english_only`): a listed key is in the first (source) catalog and in no other.

use crate::{Category, Entry, Language, placeholders};

/// A language's messages for one program.
#[derive(Clone, Copy)]
pub struct Catalog<'a> {
    pub lang: &'a Language,
    pub entries: &'a [Entry],
}

/// One thing wrong with a set of catalogs.
#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    Missing {
        lang: &'static str,
        key: &'static str,
    },
    Orphan {
        lang: &'static str,
        key: &'static str,
    },
    Duplicate {
        lang: &'static str,
        key: &'static str,
    },
    PlaceholdersDiffer {
        key: &'static str,
        lang: &'static str,
    },
    /// The entry does not have exactly the forms its language declares, or a form of a counted message lacks `{count}`.
    PluralFormsDisagree {
        key: &'static str,
        lang: &'static str,
    },
    Markup {
        lang: &'static str,
        key: &'static str,
    },
    /// The English-only list names a key the source catalog (or the key enum) does not have.
    EnglishOnlyUnknown { key: &'static str },
    /// A key that is English in every language is also in another language's table.
    EnglishOnlyTranslated {
        lang: &'static str,
        key: &'static str,
    },
}

fn all_placeholders(e: &Entry) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = e.forms().flat_map(|(_, t)| placeholders(t)).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn has_markup(text: &str) -> bool {
    text.contains(['<', '>', '&']) || text.contains("http://") || text.contains("https://")
}

/// What a program's catalog is checked for beyond the shared rules.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// No markup, entity or URL in a message (the server renders its messages into HTML; a command-line program's help legitimately
    /// shows `<https://host/owner/name>`).
    pub forbid_markup: bool,
    /// The program uses plural forms (the server does); otherwise every message has `other` only, a counted sentence being written so
    /// that it reads right for one and for many.
    pub plurals: bool,
}

impl Options {
    /// The server's: HTML-safe messages with plural forms.
    pub const WEB: Options = Options {
        forbid_markup: true,
        plurals: true,
    };
    /// The command-line program's: plain text, no plural forms.
    pub const PLAIN: Options = Options {
        forbid_markup: false,
        plurals: false,
    };
}

/// Whether the entry has exactly the forms its language declares, with `{count}` in each when it is counted.
fn forms_agree(lang: &Language, e: &Entry, plurals: bool) -> bool {
    let counted = plurals && e.forms().any(|(_, t)| placeholders(t).contains(&"count"));
    let have: Vec<Category> = e.forms().map(|(c, _)| c).collect();
    if !counted {
        return have == [Category::Other];
    }
    let want: Vec<Category> = Category::ALL
        .into_iter()
        .filter(|c| lang.categories.contains(c))
        .collect();
    have == want && e.forms().all(|(_, t)| placeholders(t).contains(&"count"))
}

/// The checks over `catalogs` (the source language first; the others are compared with it), against `keys` — every identifier the
/// program's key enum knows — and the `english_only` list.
pub fn audit(
    catalogs: &[Catalog<'_>],
    keys: &[&'static str],
    english_only: &[&'static str],
) -> Vec<Problem> {
    audit_with(catalogs, keys, english_only, Options::WEB)
}

/// [`audit`] with the program's [`Options`].
pub fn audit_with(
    catalogs: &[Catalog<'_>],
    keys: &[&'static str],
    english_only: &[&'static str],
    options: Options,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let base = catalogs.first();
    if let Some(b) = base {
        for key in english_only {
            if !keys.contains(key) || !b.entries.iter().any(|e| e.key == *key) {
                problems.push(Problem::EnglishOnlyUnknown { key });
            }
        }
    }
    for (index, c) in catalogs.iter().enumerate() {
        let def = c.lang;
        let is_base = index == 0;
        let ids: Vec<&'static str> = c.entries.iter().map(|e| e.key).collect();
        for key in english_only.iter().filter(|_| !is_base) {
            if ids.contains(key) {
                problems.push(Problem::EnglishOnlyTranslated { lang: def.tag, key });
            }
        }
        for k in keys {
            // A key that is English in every language is rightly absent from the other tables.
            if !is_base && english_only.contains(k) {
                continue;
            }
            if !ids.contains(k) {
                problems.push(Problem::Missing {
                    lang: def.tag,
                    key: k,
                });
            }
        }
        for (i, e) in c.entries.iter().enumerate() {
            if !keys.contains(&e.key) {
                problems.push(Problem::Orphan {
                    lang: def.tag,
                    key: e.key,
                });
            }
            if c.entries[..i].iter().any(|p| p.key == e.key) {
                problems.push(Problem::Duplicate {
                    lang: def.tag,
                    key: e.key,
                });
            }
            if options.forbid_markup && e.forms().any(|(_, t)| has_markup(t)) {
                problems.push(Problem::Markup {
                    lang: def.tag,
                    key: e.key,
                });
            }
            if !forms_agree(def, e, options.plurals) {
                problems.push(Problem::PluralFormsDisagree {
                    key: e.key,
                    lang: def.tag,
                });
            }
            if let Some(b) = base.filter(|b| b.lang.tag != def.tag)
                && let Some(be) = b.entries.iter().find(|x| x.key == e.key)
                && all_placeholders(be) != all_placeholders(e)
            {
                problems.push(Problem::PlaceholdersDiffer {
                    key: e.key,
                    lang: def.tag,
                });
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::{en::EN, ja::JA};
    use crate::{msg, plural, plural_msg};

    const KEYS: &[&str] = &["a.plain", "a.counted", "a.named"];

    fn en() -> Vec<Entry> {
        vec![
            msg("a.plain", "Hello"),
            plural_msg("a.counted", "{count} file", "{count} files"),
            msg("a.named", "Hi {name}"),
        ]
    }

    fn ja_ok() -> Vec<Entry> {
        vec![
            msg("a.plain", "こんにちは"),
            msg("a.counted", "{count} 件"),
            msg("a.named", "{name} さん"),
        ]
    }

    fn run(second: &[Entry]) -> Vec<Problem> {
        let en = en();
        audit(
            &[
                Catalog {
                    lang: &EN,
                    entries: &en,
                },
                Catalog {
                    lang: &JA,
                    entries: second,
                },
            ],
            KEYS,
            &[],
        )
    }

    #[test]
    fn a_consistent_pair_has_no_problem() {
        assert_eq!(run(&ja_ok()), vec![]);
    }

    #[test]
    fn missing_orphan_duplicate_markup_and_placeholder_problems_are_found_by_name() {
        let mut v = ja_ok();
        v.pop();
        assert_eq!(
            run(&v),
            vec![Problem::Missing {
                lang: "ja",
                key: "a.named"
            }]
        );
        let mut v = ja_ok();
        v.push(msg("a.extra", "x"));
        assert_eq!(
            run(&v),
            vec![Problem::Orphan {
                lang: "ja",
                key: "a.extra"
            }]
        );
        let mut v = ja_ok();
        v.push(msg("a.plain", "again"));
        assert_eq!(
            run(&v),
            vec![Problem::Duplicate {
                lang: "ja",
                key: "a.plain"
            }]
        );
        let mut v = ja_ok();
        v[0] = msg("a.plain", "<b>x</b>");
        assert_eq!(
            run(&v),
            vec![Problem::Markup {
                lang: "ja",
                key: "a.plain"
            }]
        );
        let mut v = ja_ok();
        v[2] = msg("a.named", "{who} さん");
        assert_eq!(
            run(&v),
            vec![Problem::PlaceholdersDiffer {
                key: "a.named",
                lang: "ja"
            }]
        );
    }

    #[test]
    fn a_language_has_exactly_the_forms_it_declares_and_each_counted_form_has_the_count() {
        // Japanese declares only `other`: a counted message with a singular form disagrees, and so does an uncounted one with two.
        let mut v = ja_ok();
        v[1] = plural_msg("a.counted", "{count} 件", "{count} 件");
        assert_eq!(
            run(&v),
            vec![Problem::PluralFormsDisagree {
                key: "a.counted",
                lang: "ja"
            }]
        );
        let mut v = ja_ok();
        v[0] = plural_msg("a.plain", "こんにちは", "こんにちは");
        assert_eq!(
            run(&v),
            vec![Problem::PluralFormsDisagree {
                key: "a.plain",
                lang: "ja"
            }]
        );
        // English declares one and other: a counted message with `other` only disagrees; so does a form without the count.
        let bad_en = vec![
            msg("a.plain", "Hello"),
            msg("a.counted", "{count} files"),
            msg("a.named", "Hi {name}"),
        ];
        let ja = ja_ok();
        let problems = audit(
            &[
                Catalog {
                    lang: &EN,
                    entries: &bad_en,
                },
                Catalog {
                    lang: &JA,
                    entries: &ja,
                },
            ],
            KEYS,
            &[],
        );
        assert_eq!(
            problems,
            vec![Problem::PluralFormsDisagree {
                key: "a.counted",
                lang: "en"
            }]
        );
        let no_count = vec![
            msg("a.plain", "Hello"),
            plural_msg("a.counted", "one file", "{count} files"),
            msg("a.named", "Hi {name}"),
        ];
        let problems = audit(
            &[
                Catalog {
                    lang: &EN,
                    entries: &no_count,
                },
                Catalog {
                    lang: &JA,
                    entries: &ja,
                },
            ],
            KEYS,
            &[],
        );
        assert!(
            problems.contains(&Problem::PluralFormsDisagree {
                key: "a.counted",
                lang: "en"
            }),
            "{problems:?}"
        );
    }

    /// A language with four categories needs four forms, `other` among them; the machinery is proven before a catalog needs it.
    #[test]
    fn a_four_category_language_needs_one_few_many_and_other() {
        static RU: Language = Language {
            tag: "ru",
            endonym: "русский",
            dir: crate::Dir::Ltr,
            categories: plural::ONE_FEW_MANY_OTHER,
            plural: plural::ru,
            group_separator: ' ',
            decimal_separator: ',',
            status: crate::Status::Draft,
        };
        let ru_ok = vec![
            msg("a.plain", "Привет"),
            Entry::with_forms(
                "a.counted",
                &[
                    (Category::One, "{count} файл"),
                    (Category::Few, "{count} файла"),
                    (Category::Many, "{count} файлов"),
                    (Category::Other, "{count} файла"),
                ],
            ),
            msg("a.named", "Привет, {name}"),
        ];
        let en = en();
        let run_ru = |ru: &[Entry]| {
            audit(
                &[
                    Catalog {
                        lang: &EN,
                        entries: &en,
                    },
                    Catalog {
                        lang: &RU,
                        entries: ru,
                    },
                ],
                KEYS,
                &[],
            )
        };
        assert_eq!(run_ru(&ru_ok), vec![]);
        let mut two = ru_ok.clone();
        two[1] = plural_msg("a.counted", "{count} файл", "{count} файлов");
        assert_eq!(
            run_ru(&two),
            vec![Problem::PluralFormsDisagree {
                key: "a.counted",
                lang: "ru"
            }]
        );
    }

    #[test]
    fn the_english_only_list_is_expected_exactly() {
        let en = en();
        let ja_without: Vec<Entry> = ja_ok().into_iter().filter(|e| e.key != "a.plain").collect();
        let run = |ja: &[Entry], only: &[&'static str]| {
            audit(
                &[
                    Catalog {
                        lang: &EN,
                        entries: &en,
                    },
                    Catalog {
                        lang: &JA,
                        entries: ja,
                    },
                ],
                KEYS,
                only,
            )
        };
        assert_eq!(run(&ja_without, &["a.plain"]), vec![]);
        assert_eq!(
            run(&ja_ok(), &["a.plain"]),
            vec![Problem::EnglishOnlyTranslated {
                lang: "ja",
                key: "a.plain"
            }]
        );
        assert_eq!(
            run(&ja_without, &[]),
            vec![Problem::Missing {
                lang: "ja",
                key: "a.plain"
            }]
        );
        assert_eq!(
            run(&ja_ok(), &["no.such"]),
            vec![Problem::EnglishOnlyUnknown { key: "no.such" }]
        );
    }

    #[test]
    fn a_plain_text_program_may_show_angle_brackets_and_counts_without_plural_forms() {
        let keys = ["a.help", "a.count"];
        let en = vec![
            msg("a.help", "usage: tool <https://host/x>"),
            msg("a.count", "{count} files"),
        ];
        let ja = vec![
            msg("a.help", "使い方: tool <https://host/x>"),
            msg("a.count", "{count} 件"),
        ];
        let cats = [
            Catalog {
                lang: &EN,
                entries: &en,
            },
            Catalog {
                lang: &JA,
                entries: &ja,
            },
        ];
        assert_eq!(audit_with(&cats, &keys, &[], Options::PLAIN), vec![]);
        // The same catalogs fail the server's options: markup, and a counted message with `other` only in a language with `one`.
        let web = audit_with(&cats, &keys, &[], Options::WEB);
        assert!(
            web.contains(&Problem::Markup {
                lang: "en",
                key: "a.help"
            }),
            "{web:?}"
        );
        assert!(
            web.contains(&Problem::PluralFormsDisagree {
                key: "a.count",
                lang: "en"
            }),
            "{web:?}"
        );
        // A plain program has no plural forms at all.
        let two = vec![
            msg("a.help", "h"),
            plural_msg("a.count", "{count} file", "{count} files"),
        ];
        let p = audit_with(
            &[Catalog {
                lang: &EN,
                entries: &two,
            }],
            &keys,
            &[],
            Options::PLAIN,
        );
        assert_eq!(
            p,
            vec![Problem::PluralFormsDisagree {
                key: "a.count",
                lang: "en"
            }]
        );
    }
}
