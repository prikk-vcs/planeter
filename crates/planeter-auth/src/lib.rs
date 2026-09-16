#![forbid(unsafe_code)]
//! `planeter-auth` — planeter core (identity) layer. See RFC 001/002.
//!
//! Authentication: turns a presented credential (password, OIDC token, access token, SSH/deploy key)
//! into a [`planeter_core::Principal`], which is all `authorize()` (core) ever sees (RFC 002 D-2,
//! `STD-2`). The account model ([`identity`]), the machine-credential model ([`credential`]), the OIDC
//! seam ([`oidc`]), and the [`authenticator`] that ties them together live here.
//!
//! **Crypto is behind seams, not yet wired** (owner-ruled: model + seams now, vetted crates later).
//! [`hashing`] defines the `PasswordHasher`/`TokenHasher` traits the production **Argon2id** password
//! hasher and constant-time token hasher install behind; it ships only a clearly-marked
//! [`hashing::InsecureStubHasher`] for tests. No real secret should pass through this crate until the
//! production hashers land.

pub mod authenticator;
pub mod credential;
pub mod hashing;
pub mod identity;
pub mod oidc;

pub use authenticator::Authenticator;
pub use credential::{
    CredentialStore, DeployKeyRecord, InMemoryCredentialStore, SshKeyRecord, TokenRecord,
};
pub use hashing::{InsecureStubHasher, PasswordHash, PasswordHasher, TokenHash, TokenHasher};
pub use identity::{Account, AccountStore, AuthError, InMemoryAccountStore};
pub use oidc::{OidcError, OidcVerifier, UnconfiguredOidc, VerifiedIdentity};
