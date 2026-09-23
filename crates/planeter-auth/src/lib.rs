#![forbid(unsafe_code)]
//! `planeter-auth` — planeter core (identity) layer. See RFC 001/002.
//!
//! Authentication: turns a presented credential (password, OIDC token, access token, SSH/deploy key)
//! into a [`planeter_core::Principal`], which is all `authorize()` (core) ever sees (RFC 002 D-2,
//! `STD-2`). The account model ([`identity`]), the machine-credential model ([`credential`]), the OIDC
//! seam ([`oidc`]), and the [`authenticator`] that ties them together live here.
//!
//! **Crypto is behind seams, and (as of the 2026-09-23 supply-chain review) wired:** [`hashing`]
//! ships the production **Argon2id** password hasher and the constant-time SHA-256 token hasher (plus
//! [`hashing::mint_token`]); [`sshkey`] parses OpenSSH **ed25519** public keys to fingerprints. The
//! [`hashing::InsecureStubHasher`] remains only as a fast test double. **OIDC** is implemented (2026-09-24,
//! owner-ruled): [`oidc::OidcProvider`] runs the PKCE code flow and verifies ID tokens against the
//! provider's JWKS with `jsonwebtoken`'s RustCrypto backend, fetching only through the egress guard +
//! confined `curl`; accounts are linked to `(issuer, subject)` administratively — no auto-provisioning.

pub mod authenticator;
pub mod credential;
pub mod hashing;
pub mod identity;
pub mod oidc;
pub mod session;
pub mod sqlite;
pub mod sshkey;
pub mod throttle;

pub use authenticator::Authenticator;
pub use credential::{
    CredentialStore, DeployKeyRecord, InMemoryCredentialStore, SshKeyRecord, TokenRecord,
};
pub use hashing::{
    Argon2idHasher, InsecureStubHasher, PasswordHash, PasswordHasher, Sha256TokenHasher, TokenHash,
    TokenHasher, mint_token,
};
pub use identity::{Account, AccountStore, AuthError, InMemoryAccountStore};
pub use oidc::{
    BeginLogin, OidcConfig, OidcError, OidcProvider, OidcVerifier, PENDING_TTL_SECS,
    UnconfiguredOidc, VerifiedIdentity,
};
pub use session::{
    InMemorySessionStore, SESSION_TTL_SECS, Session, SessionId, SessionStore, random_token,
    tokens_match,
};
pub use sqlite::{SqliteAccountStore, SqliteCredentialStore, SqliteSessionStore};
pub use sshkey::{SshKeyError, SshPublicKey};
pub use throttle::{DEFAULT_LOCK_SECS, DEFAULT_MAX_FAILURES, LoginThrottle};
