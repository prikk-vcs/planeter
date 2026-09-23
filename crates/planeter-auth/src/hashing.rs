//! Password/token hashing (RFC 002 T2, `STD-2`) — the seams and their production implementations.
//!
//! - [`Argon2idHasher`] — **Argon2id** (the `argon2` crate, PHC string format, OS-random salt) for local
//!   passwords. This is the production [`PasswordHasher`].
//! - [`Sha256TokenHasher`] — SHA-256 of a high-entropy access token, compared in **constant time**
//!   (`subtle`). Tokens are random secrets, not human passwords, so a KDF adds nothing; a fast hash with a
//!   constant-time compare is the standard scheme. This is the production [`TokenHasher`].
//! - [`mint_token`] — mints a fresh high-entropy token (32 OS-random bytes, base64url), the only way a
//!   token should ever be created.
//! - [`InsecureStubHasher`] — a **non-cryptographic** test double kept for fast unit tests. Never install
//!   it in a deployment.
//!
//! The crates entered through the owner-approved supply-chain review of 2026-09-23 (cargo-deny/audit
//! clean; +15 crates). Verification is a boolean; the hasher never reveals which part failed.

use std::hash::{Hash, Hasher};

/// An opaque stored password verifier (never the plaintext). Produced by a [`PasswordHasher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordHash(pub String);

/// An opaque stored token verifier (never the plaintext token). Produced by a [`TokenHasher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHash(pub String);

/// One-way hashing of a user password. The real implementation is **Argon2id** (`STD-2`); this trait is
/// the seam it installs behind.
pub trait PasswordHasher: Send + Sync {
    /// Hash a password for storage.
    fn hash(&self, password: &str) -> PasswordHash;
    /// Verify a presented password against a stored hash (constant-time in the real impl).
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool;
}

/// One-way hashing of an access/OAuth token. Tokens are stored hashed and compared in **constant time**
/// in the real implementation; this is the seam.
pub trait TokenHasher: Send + Sync {
    fn hash(&self, token: &str) -> TokenHash;
    fn verify(&self, token: &str, hash: &TokenHash) -> bool;
}

/// A **non-cryptographic** reference hasher for tests and local development. It uses the std hasher with
/// a fixed domain-separating prefix — enough to exercise the seams and to avoid storing plaintext, but
/// **trivially reversible/brute-forceable and NOT constant-time**. Never install this in a deployment.
#[derive(Debug, Default, Clone, Copy)]
pub struct InsecureStubHasher;

impl InsecureStubHasher {
    fn digest(domain: &str, secret: &str) -> String {
        // Two rounds with a domain tag so a password hash and a token hash of the same input differ.
        // This is deliberately simple; it is NOT a KDF.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        domain.hash(&mut h);
        secret.hash(&mut h);
        let a = h.finish();
        let mut h2 = std::collections::hash_map::DefaultHasher::new();
        a.hash(&mut h2);
        secret.hash(&mut h2);
        format!("stub${domain}${:016x}{:016x}", a, h2.finish())
    }
}

impl PasswordHasher for InsecureStubHasher {
    fn hash(&self, password: &str) -> PasswordHash {
        PasswordHash(Self::digest("pw", password))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        Self::digest("pw", password) == hash.0
    }
}

impl TokenHasher for InsecureStubHasher {
    fn hash(&self, token: &str) -> TokenHash {
        TokenHash(Self::digest("tok", token))
    }
    fn verify(&self, token: &str, hash: &TokenHash) -> bool {
        Self::digest("tok", token) == hash.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_hash_is_not_plaintext_and_round_trips() {
        let h = InsecureStubHasher;
        let stored = PasswordHasher::hash(&h, "correct horse");
        assert_ne!(stored.0, "correct horse");
        assert!(PasswordHasher::verify(&h, "correct horse", &stored));
        assert!(!PasswordHasher::verify(&h, "wrong", &stored));
    }

    #[test]
    fn password_and_token_domains_are_separated() {
        let h = InsecureStubHasher;
        let pw = PasswordHasher::hash(&h, "same-input");
        let tok = TokenHasher::hash(&h, "same-input");
        assert_ne!(pw.0, tok.0);
    }
}

// ----------------------------------------------------------------------------
// Production implementations
// ----------------------------------------------------------------------------

/// Argon2id password hashing (PHC string format). Default `argon2` parameters (Argon2id v19,
/// m=19456 KiB, t=2, p=1 — the OWASP-recommended baseline); the salt is 16 OS-random bytes.
#[derive(Debug, Default, Clone, Copy)]
pub struct Argon2idHasher;

impl PasswordHasher for Argon2idHasher {
    fn hash(&self, password: &str) -> PasswordHash {
        use argon2::PasswordHasher as _;
        use argon2::password_hash::{SaltString, rand_core::OsRng};
        let salt = SaltString::generate(&mut OsRng);
        // hash_password fails only on malformed parameters, which the defaults are not.
        let phc = argon2::Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .expect("argon2 default parameters are valid");
        PasswordHash(phc.to_string())
    }

    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        use argon2::PasswordVerifier as _;
        let Ok(parsed) = argon2::password_hash::PasswordHash::new(&hash.0) else {
            return false;
        };
        argon2::Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }
}

/// SHA-256 token hashing with constant-time verification. Stored form: `sha256$<hex digest>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sha256TokenHasher;

impl Sha256TokenHasher {
    fn digest(token: &str) -> [u8; 32] {
        use sha2::Digest as _;
        sha2::Sha256::digest(token.as_bytes()).into()
    }
}

impl TokenHasher for Sha256TokenHasher {
    fn hash(&self, token: &str) -> TokenHash {
        let d = Self::digest(token);
        let mut hex = String::with_capacity(7 + 64);
        hex.push_str("sha256$");
        for b in d {
            hex.push_str(&format!("{b:02x}"));
        }
        TokenHash(hex)
    }

    fn verify(&self, token: &str, hash: &TokenHash) -> bool {
        use subtle::ConstantTimeEq as _;
        let Some(stored_hex) = hash.0.strip_prefix("sha256$") else {
            return false;
        };
        let Some(stored) = decode_hex_32(stored_hex) else {
            return false;
        };
        Self::digest(token).ct_eq(&stored).into()
    }
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// Mint a fresh access token: 32 OS-random bytes, base64url (unpadded), prefixed `plt_` so a leaked
/// token is recognisable to secret scanners. The caller stores only [`TokenHasher::hash`] of it.
pub fn mint_token() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore as _};
    use base64ct::{Base64UrlUnpadded, Encoding as _};
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("plt_{}", Base64UrlUnpadded::encode_string(&bytes))
}

#[cfg(test)]
mod production_tests {
    use super::*;

    #[test]
    fn argon2id_round_trips_and_rejects_wrong_password() {
        let h = Argon2idHasher;
        let stored = PasswordHasher::hash(&h, "correct horse battery staple");
        assert!(
            stored.0.starts_with("$argon2id$"),
            "PHC string: {}",
            stored.0
        );
        assert!(PasswordHasher::verify(
            &h,
            "correct horse battery staple",
            &stored
        ));
        assert!(!PasswordHasher::verify(&h, "wrong", &stored));
        // Two hashes of the same password differ (fresh salt) yet both verify.
        let again = PasswordHasher::hash(&h, "correct horse battery staple");
        assert_ne!(stored.0, again.0);
        assert!(PasswordHasher::verify(
            &h,
            "correct horse battery staple",
            &again
        ));
        // A malformed stored hash never verifies.
        assert!(!PasswordHasher::verify(
            &h,
            "x",
            &PasswordHash("garbage".into())
        ));
    }

    #[test]
    fn sha256_token_hash_verifies_and_is_not_plaintext() {
        let h = Sha256TokenHasher;
        let tok = mint_token();
        assert!(tok.starts_with("plt_"));
        assert!(tok.len() > 40);
        let stored = TokenHasher::hash(&h, &tok);
        assert!(stored.0.starts_with("sha256$"));
        assert!(!stored.0.contains(&tok));
        assert!(TokenHasher::verify(&h, &tok, &stored));
        assert!(!TokenHasher::verify(&h, "plt_other", &stored));
        assert!(!TokenHasher::verify(
            &h,
            &tok,
            &TokenHash("sha256$zz".into())
        ));
        assert!(!TokenHasher::verify(&h, &tok, &TokenHash("md5$00".into())));
        // Two mints differ.
        assert_ne!(mint_token(), mint_token());
    }
}
