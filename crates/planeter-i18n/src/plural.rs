//! CLDR plural categories and the rules that pick one (<https://cldr.unicode.org/index/cldr-spec/plural-rules>), for whole numbers.
//!
//! A message with a count has one form per category its language uses. English and Japanese need two and one; Russian, Polish, Arabic
//! and Welsh are here as tested rule functions so that the machinery is proven before the first catalog that needs it. The rules are
//! written by hand from CLDR (integers only: the program counts patches, files and proposals); revisit a library when five or more
//! languages ship.

/// A CLDR plural category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Zero,
        Category::One,
        Category::Two,
        Category::Few,
        Category::Many,
        Category::Other,
    ];

    pub fn index(self) -> usize {
        match self {
            Category::Zero => 0,
            Category::One => 1,
            Category::Two => 2,
            Category::Few => 3,
            Category::Many => 4,
            Category::Other => 5,
        }
    }

    /// CLDR's name for it.
    pub fn label(self) -> &'static str {
        match self {
            Category::Zero => "zero",
            Category::One => "one",
            Category::Two => "two",
            Category::Few => "few",
            Category::Many => "many",
            Category::Other => "other",
        }
    }
}

/// The categories of a language that has only `other` (Japanese, Chinese, Korean, Vietnamese…).
pub const OTHER_ONLY: &[Category] = &[Category::Other];
/// `one` and `other` (English, German, Spanish, Italian…).
pub const ONE_OTHER: &[Category] = &[Category::One, Category::Other];
/// Russian, Ukrainian: `one`, `few`, `many`, and `other` (fractions).
pub const ONE_FEW_MANY_OTHER: &[Category] = &[
    Category::One,
    Category::Few,
    Category::Many,
    Category::Other,
];
/// Arabic and Welsh: all six.
pub const ALL_SIX: &[Category] = &Category::ALL;

/// English: `one` for exactly 1, `other` for everything else.
pub fn en(n: u64) -> Category {
    if n == 1 {
        Category::One
    } else {
        Category::Other
    }
}

/// Japanese: no grammatical plural; every count takes `other`.
pub fn ja(_: u64) -> Category {
    Category::Other
}

/// Russian: `one` for 1, 21, 31… (not 11); `few` for 2–4, 22–24… (not 12–14); `many` for 0, 5–20, 25–30…
pub fn ru(n: u64) -> Category {
    let (m10, m100) = (n % 10, n % 100);
    if m10 == 1 && m100 != 11 {
        Category::One
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        Category::Few
    } else {
        Category::Many
    }
}

/// Polish: `one` for exactly 1; `few` for 2–4, 22–24… (not 12–14); `many` for everything else (0, 5–21, 25–31…).
pub fn pl(n: u64) -> Category {
    let (m10, m100) = (n % 10, n % 100);
    if n == 1 {
        Category::One
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        Category::Few
    } else {
        Category::Many
    }
}

/// Arabic: `zero` 0; `one` 1; `two` 2; `few` for n mod 100 in 3–10; `many` for n mod 100 in 11–99; `other` for 100–102, 200–202…
pub fn ar(n: u64) -> Category {
    let m100 = n % 100;
    match n {
        0 => Category::Zero,
        1 => Category::One,
        2 => Category::Two,
        _ if (3..=10).contains(&m100) => Category::Few,
        _ if (11..=99).contains(&m100) => Category::Many,
        _ => Category::Other,
    }
}

/// Welsh: `zero` 0; `one` 1; `two` 2; `few` 3; `many` 6; `other` for everything else.
pub fn cy(n: u64) -> Category {
    match n {
        0 => Category::Zero,
        1 => Category::One,
        2 => Category::Two,
        3 => Category::Few,
        6 => Category::Many,
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::Category::*;
    use super::*;

    /// (count, expected) vectors from CLDR's plural rules for integers.
    fn check(rule: fn(u64) -> Category, name: &str, vectors: &[(u64, Category)]) {
        for (n, want) in vectors {
            assert_eq!(rule(*n), *want, "{name}({n})");
        }
    }

    #[test]
    fn english_and_japanese_keep_their_rules() {
        check(
            en,
            "en",
            &[
                (0, Other),
                (1, One),
                (2, Other),
                (11, Other),
                (21, Other),
                (100, Other),
                (1_000_000, Other),
            ],
        );
        for n in [0, 1, 2, 3, 5, 11, 21, 100, 101, 1_000_000] {
            assert_eq!(ja(n), Other, "ja({n})");
        }
    }

    #[test]
    fn russian() {
        check(
            ru,
            "ru",
            &[
                (0, Many),
                (1, One),
                (2, Few),
                (3, Few),
                (4, Few),
                (5, Many),
                (11, Many),
                (12, Many),
                (13, Many),
                (14, Many),
                (15, Many),
                (20, Many),
                (21, One),
                (22, Few),
                (25, Many),
                (31, One),
                (100, Many),
                (101, One),
                (102, Few),
                (111, Many),
                (112, Many),
                (121, One),
                (1000, Many),
                (1001, One),
                (1_000_000, Many),
            ],
        );
    }

    #[test]
    fn polish() {
        check(
            pl,
            "pl",
            &[
                (0, Many),
                (1, One),
                (2, Few),
                (3, Few),
                (4, Few),
                (5, Many),
                (11, Many),
                (12, Many),
                (14, Many),
                (21, Many),
                (22, Few),
                (24, Few),
                (25, Many),
                (31, Many),
                (100, Many),
                (101, Many),
                (102, Few),
                (111, Many),
                (112, Many),
                (1000, Many),
                (1002, Few),
                (1_000_000, Many),
            ],
        );
    }

    #[test]
    fn arabic() {
        check(
            ar,
            "ar",
            &[
                (0, Zero),
                (1, One),
                (2, Two),
                (3, Few),
                (5, Few),
                (10, Few),
                (11, Many),
                (21, Many),
                (99, Many),
                (100, Other),
                (101, Other),
                (102, Other),
                (103, Few),
                (110, Few),
                (111, Many),
                (199, Many),
                (200, Other),
                (1000, Other),
                (1003, Few),
                (1011, Many),
                (1_000_000, Other),
            ],
        );
    }

    #[test]
    fn welsh() {
        check(
            cy,
            "cy",
            &[
                (0, Zero),
                (1, One),
                (2, Two),
                (3, Few),
                (4, Other),
                (5, Other),
                (6, Many),
                (7, Other),
                (11, Other),
                (21, Other),
                (100, Other),
                (101, Other),
                (1_000_000, Other),
            ],
        );
    }

    #[test]
    fn every_rule_returns_only_a_category_its_set_declares_and_every_set_ends_with_other() {
        type Row<'a> = (&'a str, fn(u64) -> Category, &'a [Category]);
        let sets: [Row; 6] = [
            ("en", en, ONE_OTHER),
            ("ja", ja, OTHER_ONLY),
            ("ru", ru, ONE_FEW_MANY_OTHER),
            ("pl", pl, ONE_FEW_MANY_OTHER),
            ("ar", ar, ALL_SIX),
            ("cy", cy, ALL_SIX),
        ];
        for (name, rule, categories) in sets {
            assert_eq!(categories.last(), Some(&Other), "{name}");
            for n in (0..=1200).chain([10_000, 1_000_000, u64::MAX]) {
                assert!(categories.contains(&rule(n)), "{name}({n}) = {:?}", rule(n));
            }
        }
    }

    #[test]
    fn indexes_and_labels_are_distinct() {
        let mut seen = [false; 6];
        for c in Category::ALL {
            assert!(!seen[c.index()]);
            seen[c.index()] = true;
        }
        assert_eq!(
            Category::ALL.map(Category::label),
            ["zero", "one", "two", "few", "many", "other"]
        );
    }
}
