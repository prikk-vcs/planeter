//! The one bound on the maintainer keys a repository may trust (RFC 013 D-6).
//!
//! **64**, and it is the same number on every path (creation, a delivery, the operator's add), so that a person never meets two rules.
//! It is not a guess. Creation already keeps an archive's findings only up to 64 carried keys, a repository made with 40 would have been
//! over any lower bound on its first addition, and — because planeter never removes a key (RFC 013 D-1, ledger PK-90: removing a key that
//! signed anything makes `verify` fail) — every addition uses a slot for the repository's whole life, so a low bound is a cliff with no way
//! down. The cost at 64 was measured on prikk 0.58.0 (ledger PK-92): trusted keys 17 / 65 / 257 / 1,025 → `verify` 7 / 10 / 29 / 553 ms,
//! the trust list 2.2 / 8.2 / 32 / 129 KB, an archive of the repository 10 KB / 70 KB / 0.9 MB / 13.8 MB (every key added leaves a stale
//! trust record the size of the whole list, so the archive grows with the square of the number of additions until `prikk compact
//! --trust-policy` reclaims it). At 64 nothing is noticeable and no compaction is needed; far beyond it the cost leaves the linear. The
//! output cap of `trust maintainer list` was measured for 64 keys (6.9 KB) too.

use std::fmt;

/// The most maintainer keys a repository may trust, and the most an archive may carry for creation to keep its findings.
pub const KEYS_MAX: usize = 64;

/// Whether `text` is a public key as prikk writes it: 64 lowercase hexadecimal characters.
pub fn is_public_key(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The id prikk derives from a public key: `ed25519-` and the **first 16 characters of the key** (measured on prikk 0.58.0 over the keys
/// the tests make: the id of `ca34871b374be635…` is `ed25519-ca34871b374be635`). `None` when `public_key` is not a public key. The
/// operator's command (RFC 013 D-2) refuses an id that is not this one, and shows both; creation never needed it (prikk reports each carried
/// key as a pair). A test over real keys holds that it is what prikk says.
pub fn key_id_of(public_key: &str) -> Option<String> {
    is_public_key(public_key).then(|| format!("ed25519-{}", &public_key[..16]))
}

/// Adding would take a repository over [`KEYS_MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeysBound {
    /// How many keys the repository trusts now.
    pub trusted: usize,
    /// How many were to be added.
    pub adding: usize,
    /// The bound, [`KEYS_MAX`].
    pub max: usize,
}

impl fmt::Display for KeysBound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} keys are trusted and {} were to be added; at most {} are allowed",
            self.trusted, self.adding, self.max
        )
    }
}

impl std::error::Error for KeysBound {}

/// Whether `adding` more keys fit beside the `trusted` ones: `Ok` when the total is at most [`KEYS_MAX`]. The arithmetic cannot overflow
/// (a total that does not fit a `usize` does not fit the bound either).
pub fn room_for(trusted: usize, adding: usize) -> Result<(), KeysBound> {
    match trusted.checked_add(adding) {
        Some(total) if total <= KEYS_MAX => Ok(()),
        _ => Err(KeysBound {
            trusted,
            adding,
            max: KEYS_MAX,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_the_first_sixteen_characters_of_the_key() {
        let pk = "ca34871b374be63553636fa321a7c39a7e28ee79f4c32a437628b34fe0f9f199";
        assert!(is_public_key(pk));
        assert_eq!(key_id_of(pk).as_deref(), Some("ed25519-ca34871b374be635"));
        for bad in [
            "",
            "ab",
            &pk.to_uppercase(),
            &pk[..63],
            &format!("{pk}0"),
            &"g".repeat(64),
        ] {
            assert!(!is_public_key(bad), "{bad}");
            assert_eq!(key_id_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_bound_is_sixty_four() {
        assert_eq!(KEYS_MAX, 64);
    }

    #[test]
    fn the_boundaries_are_zero_sixty_four_and_sixty_five() {
        assert_eq!(room_for(0, 0), Ok(()));
        assert_eq!(room_for(0, 64), Ok(()));
        assert_eq!(room_for(64, 0), Ok(()));
        assert_eq!(room_for(63, 1), Ok(()));
        assert_eq!(
            room_for(0, 65),
            Err(KeysBound {
                trusted: 0,
                adding: 65,
                max: 64
            })
        );
        assert_eq!(
            room_for(64, 1),
            Err(KeysBound {
                trusted: 64,
                adding: 1,
                max: 64
            })
        );
        assert_eq!(
            room_for(65, 0),
            Err(KeysBound {
                trusted: 65,
                adding: 0,
                max: 64
            }),
            "a repository already over the bound has no room even for nothing"
        );
    }

    #[test]
    fn adding_nothing_to_a_full_repository_is_fine() {
        assert!(room_for(KEYS_MAX, 0).is_ok());
    }

    #[test]
    fn the_arithmetic_cannot_overflow() {
        assert!(room_for(usize::MAX, 1).is_err());
        assert!(room_for(1, usize::MAX).is_err());
        assert!(room_for(usize::MAX, usize::MAX).is_err());
    }

    #[test]
    fn the_bound_says_its_numbers() {
        let e = room_for(60, 10).unwrap_err();
        assert_eq!(
            e.to_string(),
            "60 keys are trusted and 10 were to be added; at most 64 are allowed"
        );
    }
}
