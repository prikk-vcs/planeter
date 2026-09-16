//! The password/token hashing **seams** (RFC 002 T2, `STD-2`). Authentication needs one-way hashing —
//! Argon2id for local passwords, a constant-time compare for access-token hashes — but the concrete
//! crypto is a deferred supply-chain decision (owner-ruled: model + seams now, vetted crates later). So
//! hashing lives behind these traits, and the real implementations drop in without touching the account,
//! credential, or authenticator code.
//!
//! This module ships only a **reference stub** ([`InsecureStubHasher`]) so the model is testable. It is
//! **NOT cryptographic and NEVER for production** — it exists solely to exercise the seams. The
//! production build will install an Argon2id password hasher and a constant-time token hasher behind
//! [`PasswordHasher`]/[`TokenHasher`]; until then no real secret should pass through this crate.

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
