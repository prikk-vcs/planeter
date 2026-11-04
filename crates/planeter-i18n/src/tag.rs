//! BCP 47 negotiation: which of the available languages answers a request for `zh-TW`, `pt-PT`, `ja-JP`, `en-GB`, `de_DE.UTF-8`.
//!
//! The order of matching (handoff `language-registry-handoff-v1` T4): **exact** (case-insensitive) → **same language and script** →
//! **same language** (the first registered) → none, and the caller answers in the source language. A request without a script gets
//! the script its region implies where that is settled (`zh-TW`, `zh-HK`, `zh-MO` → `Hant`; `zh-CN`, `zh-SG` → `Hans`).

use crate::Language;

/// A parsed language tag: the parts negotiation looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// Lower-case primary language subtag (`zh`).
    pub language: String,
    /// Title-case script subtag (`Hant`), as written or implied by the region.
    pub script: Option<String>,
    /// Upper-case region subtag (`TW`, `419`).
    pub region: Option<String>,
}

/// Parse a language tag. Variants, extensions and private use are ignored; `_` is accepted for `-` (POSIX locales).
/// `None` for text that is not a language tag (empty, `*`, a digit, a language subtag of one letter or more than eight).
pub fn parse(text: &str) -> Option<Tag> {
    let mut parts = text.trim().split(['-', '_']);
    let language = parts.next()?;
    if !(2..=8).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut tag = Tag {
        language: language.to_ascii_lowercase(),
        script: None,
        region: None,
    };
    for part in parts {
        let alpha = part.bytes().all(|b| b.is_ascii_alphabetic());
        if tag.script.is_none() && tag.region.is_none() && part.len() == 4 && alpha {
            let mut s = part.to_ascii_lowercase();
            s[..1].make_ascii_uppercase();
            tag.script = Some(s);
        } else if tag.region.is_none()
            && ((part.len() == 2 && alpha)
                || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit())))
        {
            tag.region = Some(part.to_ascii_uppercase());
        } else {
            break;
        }
    }
    Some(tag)
}

impl Tag {
    /// The script as written, or the one the region implies where that is settled.
    pub fn effective_script(&self) -> Option<String> {
        if let Some(s) = &self.script {
            return Some(s.clone());
        }
        match (self.language.as_str(), self.region.as_deref()) {
            ("zh", Some("TW" | "HK" | "MO")) => Some("Hant".to_owned()),
            ("zh", Some("CN" | "SG")) => Some("Hans".to_owned()),
            _ => None,
        }
    }
}

/// The available language that answers `requested`, or `None`.
pub fn matching<'a>(requested: &str, available: &[&'a Language]) -> Option<&'a Language> {
    let wanted = parse(requested)?;
    let canonical = |t: &Tag| {
        let mut s = t.language.clone();
        if let Some(x) = &t.script {
            s.push('-');
            s.push_str(&x.to_ascii_lowercase());
        }
        if let Some(x) = &t.region {
            s.push('-');
            s.push_str(&x.to_ascii_lowercase());
        }
        s
    };
    let wanted_exact = canonical(&Tag {
        script: wanted.script.clone(),
        ..wanted.clone()
    });
    let parsed: Vec<(&'a Language, Tag)> = available
        .iter()
        .filter_map(|l| parse(l.tag).map(|t| (*l, t)))
        .collect();
    // 1. exact
    if let Some((l, _)) = parsed.iter().find(|(_, t)| canonical(t) == wanted_exact) {
        return Some(l);
    }
    // 2. same language and script
    if let Some(script) = wanted.effective_script()
        && let Some((l, _)) = parsed.iter().find(|(_, t)| {
            t.language == wanted.language
                && t.effective_script().as_deref() == Some(script.as_str())
        })
    {
        return Some(l);
    }
    // 3. same language: the first registered
    parsed
        .iter()
        .find(|(_, t)| t.language == wanted.language)
        .map(|(l, _)| *l)
}

/// Negotiate a request's language: the cookie when it names an available language, else the best `Accept-Language` entry (q-values,
/// `q=0` and `*` refused; on a tie the earlier entry), else `None` (the caller uses the source language).
pub fn negotiate<'a>(
    accept_language: Option<&str>,
    cookie: Option<&str>,
    available: &[&'a Language],
) -> Option<&'a Language> {
    if let Some(l) = cookie.and_then(|c| matching(c, available)) {
        return Some(l);
    }
    let mut best: Option<(f32, usize, &'a Language)> = None;
    for (i, part) in accept_language.unwrap_or("").split(',').enumerate() {
        let mut it = part.trim().split(';');
        let tag = it.next().unwrap_or("").trim();
        let q = it
            .find_map(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|v| v.parse::<f32>().ok())
            })
            .unwrap_or(1.0);
        if q <= 0.0 || tag == "*" {
            continue;
        }
        if let Some(l) = matching(tag, available)
            && best.is_none_or(|(bq, bi, _)| q > bq || (q == bq && i < bi))
        {
            best = Some((q, i, l));
        }
    }
    best.map(|(_, _, l)| l)
}

/// The language a POSIX locale names: the first of `LC_ALL`, `LC_MESSAGES`, `LANG` that is set and not empty, by its tag
/// (`de_DE.UTF-8`, `zh_TW.Big5`, `ja_JP@mod` → `de-DE`, `zh-TW`, `ja-JP`). `C` and `POSIX` name nothing.
pub fn from_environment<'a>(
    var: impl Fn(&str) -> Option<String>,
    available: &[&'a Language],
) -> Option<&'a Language> {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| var(name))
        .find(|v| !v.trim().is_empty())?;
    let tag = value.split(['.', '@']).next().unwrap_or("");
    if tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
        return None;
    }
    matching(tag, available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plural;
    use crate::{Dir, Status};

    const fn lang(tag: &'static str) -> Language {
        Language {
            tag,
            endonym: tag,
            dir: Dir::Ltr,
            categories: plural::OTHER_ONLY,
            plural: plural::ja,
            group_separator: ',',
            decimal_separator: '.',
            status: Status::Draft,
        }
    }

    static EN: Language = lang("en");
    static JA: Language = lang("ja");
    static HANS: Language = lang("zh-Hans");
    static HANT: Language = lang("zh-Hant");
    static PT_BR: Language = lang("pt-BR");
    static DE: Language = lang("de");

    fn all() -> Vec<&'static Language> {
        vec![&EN, &JA, &HANS, &HANT, &PT_BR, &DE]
    }

    fn got(requested: &str) -> Option<&'static str> {
        matching(requested, &all()).map(|l| l.tag)
    }

    /// The negotiation table (request → answer), as printed in the review request.
    #[test]
    fn the_negotiation_table() {
        for (request, answer) in [
            ("en", Some("en")),
            ("EN", Some("en")),
            ("en-GB", Some("en")),
            ("en_US", Some("en")),
            ("ja", Some("ja")),
            ("ja-JP", Some("ja")),
            ("ja_JP", Some("ja")),
            ("zh-Hant", Some("zh-Hant")),
            ("zh-hans", Some("zh-Hans")),
            ("zh-TW", Some("zh-Hant")),
            ("zh-HK", Some("zh-Hant")),
            ("zh-CN", Some("zh-Hans")),
            ("zh-SG", Some("zh-Hans")),
            ("zh-Hant-TW", Some("zh-Hant")),
            ("zh", Some("zh-Hans")),
            ("pt-BR", Some("pt-BR")),
            ("pt-PT", Some("pt-BR")),
            ("pt", Some("pt-BR")),
            ("de-AT", Some("de")),
            ("fr", None),
            ("fr-CA", None),
            ("*", None),
            ("", None),
            ("x", None),
            ("1", None),
        ] {
            assert_eq!(got(request), answer, "{request:?}");
        }
    }

    #[test]
    fn a_script_beats_the_first_registered_and_an_unsettled_region_falls_back_to_the_first_of_the_language()
     {
        // Hant is registered after Hans, but a request for Hant gets Hant.
        assert_eq!(got("zh-Hant-HK"), Some("zh-Hant"));
        // Only the other script ships: same language, the first registered.
        let only_hans: Vec<&Language> = vec![&EN, &HANS];
        assert_eq!(
            matching("zh-TW", &only_hans).map(|l| l.tag),
            Some("zh-Hans")
        );
        // No Chinese at all.
        let none: Vec<&Language> = vec![&EN, &JA];
        assert!(matching("zh-TW", &none).is_none());
    }

    #[test]
    fn accept_language_uses_q_values_skips_q0_and_the_star_and_prefers_the_earlier_on_a_tie() {
        let a = all();
        let tag = |h: &str, c: Option<&str>| negotiate(Some(h), c, &a).map(|l| l.tag);
        assert_eq!(tag("ja,en;q=0.5", None), Some("ja"));
        assert_eq!(tag("en;q=0.5,ja;q=0.9", None), Some("ja"));
        assert_eq!(tag("ja;q=0.8,de;q=0.8", None), Some("ja"));
        assert_eq!(tag("de;q=0.8,ja;q=0.8", None), Some("de"));
        assert_eq!(tag("ja;q=0,de", None), Some("de"));
        assert_eq!(tag("*,fr", None), None);
        assert_eq!(tag("fr,zh-TW;q=0.7", None), Some("zh-Hant"));
        assert_eq!(tag("pt-PT", None), Some("pt-BR"));
        // The cookie beats the header; an unknown cookie is ignored.
        assert_eq!(tag("en", Some("ja")), Some("ja"));
        assert_eq!(tag("de", Some("xx")), Some("de"));
        assert!(negotiate(None, None, &a).is_none());
    }

    #[test]
    fn the_environment_names_a_language_by_the_first_variable_that_is_set() {
        let a = all();
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let from = |pairs: &'static [(&'static str, &'static str)]| {
            from_environment(env(pairs), &a).map(|l| l.tag)
        };
        assert_eq!(from(&[("LANG", "de_DE.UTF-8")]), Some("de"));
        assert_eq!(from(&[("LANG", "zh_TW.Big5")]), Some("zh-Hant"));
        assert_eq!(from(&[("LANG", "zh_CN.UTF-8")]), Some("zh-Hans"));
        assert_eq!(from(&[("LC_ALL", "ja_JP@mod")]), Some("ja"));
        assert_eq!(from(&[("LC_MESSAGES", "pt_PT.UTF-8")]), Some("pt-BR"));
        assert_eq!(
            from(&[("LC_ALL", ""), ("LANG", "ja_JP.UTF-8")]),
            Some("ja"),
            "an empty variable is skipped"
        );
        assert_eq!(
            from(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "ja_JP.UTF-8")]),
            Some("en"),
            "the first one set decides"
        );
        for none in ["C", "POSIX", "c", "fr_FR.UTF-8", "jam_XX", ""] {
            let pairs: &'static [(&'static str, &'static str)] =
                Box::leak(vec![("LANG", none)].into_boxed_slice());
            assert_eq!(from(pairs), None, "{none:?}");
        }
        assert_eq!(from(&[]), None);
    }

    #[test]
    fn parsing_reads_script_and_region_and_refuses_what_is_not_a_tag() {
        let t = parse("zh-hant-tw").unwrap();
        assert_eq!(
            (
                t.language.as_str(),
                t.script.as_deref(),
                t.region.as_deref()
            ),
            ("zh", Some("Hant"), Some("TW"))
        );
        let t = parse("es-419").unwrap();
        assert_eq!(t.region.as_deref(), Some("419"));
        let t = parse("en-GB-oxendict").unwrap();
        assert_eq!((t.script, t.region.as_deref()), (None, Some("GB")));
        assert!(
            parse("").is_none()
                && parse("*").is_none()
                && parse("e").is_none()
                && parse("123").is_none()
        );
        assert!(parse("abcdefghi").is_none());
    }
}
