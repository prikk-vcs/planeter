//! The rule for a local password (RFC 014 D-4), and its Unicode normalization.
//!
//! **A password is normalized to Unicode NFKC before anything else happens to it: before it is counted, before it is checked and before it
//! is hashed, and a password that is presented to sign in is normalized the same way before it is verified.** The reason is a person's
//! keyboard: the same Japanese password typed through two input methods, or on two systems, is not always the same sequence of code
//! points (`が` is U+304C on one and U+304B U+3099 on another), and without normalization a person would be locked out of their own
//! password by the way they typed it. NFKC also folds compatibility forms (a fullwidth `ａｂｃ` becomes `abc`): that is what NFKC does, and
//! it is the same for setting and for signing in, so no password is locked out by it. **For ASCII, NFKC changes nothing**, so every hash
//! made before this rule existed still verifies.
//!
//! The rule counts **Unicode scalar values after normalization**: at least [`PASSWORD_MIN`] and at most [`PASSWORD_MAX`]; not equal to the
//! account's handle (compared normalized and lowercased); not in the embedded list of common passwords (`data/common-passwords.txt`,
//! compiled in, no network). There are no composition rules (they teach people worse passwords). Every error says *which* rule failed
//! and **never contains the password**.

use std::collections::HashSet;
use std::sync::OnceLock;

use icu_normalizer::ComposingNormalizerBorrowed;

/// The fewest characters (Unicode scalar values, after normalization) a new password has.
pub const PASSWORD_MIN: usize = 12;
/// The most characters (Unicode scalar values, after normalization) a new password has.
pub const PASSWORD_MAX: usize = 128;
/// The most characters a *presented* password is normalized and verified at. A longer input cannot be a password set under this rule
/// (the longest is [`PASSWORD_MAX`] after normalization, and normalization can expand a character at most about eighteen-fold), so it is
/// refused without normalizing it: sign-in does the same work as for any wrong password (see the authenticator).
pub const PRESENTED_MAX_CHARS: usize = 4096;

/// The embedded list of common passwords (see its header for the source and the licence).
const COMMON_LIST: &str = include_str!("../data/common-passwords.txt");

// The list is bounded at compile time: a refresh that made it much larger is a reviewed change of this line too.
const _: () = assert!(COMMON_LIST.len() <= 128 * 1024);

/// Unicode NFKC of `s`.
pub fn nfkc(s: &str) -> String {
    ComposingNormalizerBorrowed::new_nfkc()
        .normalize(s)
        .into_owned()
}

/// A password in the form that is counted, checked and hashed. Obtained only from [`check_new_password`] (a password that passed the
/// rule) or [`normalize_presented`] (one presented to sign in). **Its `Debug` and `Display` never show the text.**
#[derive(Clone, PartialEq, Eq)]
pub struct NormalizedPassword(String);

impl NormalizedPassword {
    /// The normalized text: what is hashed and verified.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for NormalizedPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NormalizedPassword(..)")
    }
}

/// Which rule a new password failed. Closed; each has a stable word. **No variant carries the password, and none of the words contains it.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordError {
    /// Fewer than [`PASSWORD_MIN`] characters.
    TooShort,
    /// More than [`PASSWORD_MAX`] characters.
    TooLong,
    /// The same as the account's handle.
    IsHandle,
    /// In the list of common passwords.
    TooCommon,
}

impl PasswordError {
    /// A stable word for a message key, a response and a log.
    pub fn reason(self) -> &'static str {
        match self {
            PasswordError::TooShort => "too_short",
            PasswordError::TooLong => "too_long",
            PasswordError::IsHandle => "is_handle",
            PasswordError::TooCommon => "too_common",
        }
    }
}

impl std::fmt::Display for PasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PasswordError::TooShort => "the password is too short",
            PasswordError::TooLong => "the password is too long",
            PasswordError::IsHandle => "the password is the same as the handle",
            PasswordError::TooCommon => "the password is one of the most common passwords",
        })
    }
}

impl std::error::Error for PasswordError {}

fn common() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        COMMON_LIST
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    })
}

/// How many entries the embedded list has (for the tests and the ledger).
pub fn common_list_len() -> usize {
    common().len()
}

/// The key a password and a handle are compared by: normalized, then lowercased.
fn compare_form(s: &str) -> String {
    nfkc(s).to_lowercase()
}

/// Check a **new** password for `handle` (RFC 014 D-4) and return it normalized. The input is bounded before it is normalized, so a
/// huge input costs nothing.
pub fn check_new_password(
    password: &str,
    handle: &str,
) -> Result<NormalizedPassword, PasswordError> {
    // Normalization can only make a string longer by a bounded factor; refuse an input that is plainly beyond the limit first.
    if password.chars().count() > PRESENTED_MAX_CHARS {
        return Err(PasswordError::TooLong);
    }
    let normalized = nfkc(password);
    let length = normalized.chars().count();
    if length < PASSWORD_MIN {
        return Err(PasswordError::TooShort);
    }
    if length > PASSWORD_MAX {
        return Err(PasswordError::TooLong);
    }
    let folded = normalized.to_lowercase();
    if folded == compare_form(handle) {
        return Err(PasswordError::IsHandle);
    }
    if common().contains(folded.as_str()) {
        return Err(PasswordError::TooCommon);
    }
    Ok(NormalizedPassword(normalized))
}

/// A password **presented to sign in**, normalized the same way as one that was set. `None` when it is longer than
/// [`PRESENTED_MAX_CHARS`] characters: no password set under the rule is that long, so the caller treats it as a wrong password (and does
/// the same work as for any wrong password).
pub fn normalize_presented(password: &str) -> Option<NormalizedPassword> {
    if password.chars().count() > PRESENTED_MAX_CHARS {
        return None;
    }
    Some(NormalizedPassword(nfkc(password)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_what_its_header_says() {
        assert!(common_list_len() >= 1000, "{}", common_list_len());
        assert!(common_list_len() <= 10_000);
        // Every entry is already normalized and lowercase, and long enough to matter.
        for w in common() {
            assert_eq!(nfkc(w).to_lowercase(), *w, "{w:?}");
            assert!(w.chars().count() >= PASSWORD_MIN, "{w:?}");
        }
        assert!(COMMON_LIST.lines().next().unwrap().starts_with('#'));
        assert!(
            COMMON_LIST.contains("MIT")
                && COMMON_LIST.contains("1a7bb9127eca9e6ff2fc0301c597fe6e16a0cb56")
        );
    }
}
