//! The account model (RFC 002 D-2): a human user account and its store. An account is the identity a
//! [`Principal::User`](planeter_core::Principal) refers to; authentication (password/OIDC/key) resolves
//! *to* an account, and `authorize()` (core) sees only the resulting principal.

use std::collections::HashMap;
use std::sync::RwLock;

use planeter_store::UserId;

use crate::hashing::PasswordHash;

/// A user account. `password` is present only for local password sign-in (Argon2id-hashed via the
/// [`PasswordHasher`](crate::hashing::PasswordHasher) seam); OIDC/SSO accounts may have none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub user: UserId,
    pub display_name: String,
    pub email: Option<String>,
    pub password: Option<PasswordHash>,
}

/// An authentication failure (kept coarse so it never reveals whether an account exists — `T-7`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthError {
    /// The credential did not authenticate (no such account, wrong password, unknown/expired token).
    /// Deliberately indistinguishable across those cases.
    BadCredentials,
    /// A backend failure reading identity/credential state.
    Backend(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::BadCredentials => f.write_str("authentication failed"),
            AuthError::Backend(e) => write!(f, "auth backend error: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

pub type Result<T> = std::result::Result<T, AuthError>;

/// Persistence of accounts. A trait so the backend is swappable (in-memory now, SQLite later).
pub trait AccountStore: Send + Sync {
    fn get(&self, user: &UserId) -> Result<Option<Account>>;
    fn upsert(&self, account: Account) -> Result<()>;
}

/// An in-memory [`AccountStore`] for tests/dev.
#[derive(Default)]
pub struct InMemoryAccountStore {
    accounts: RwLock<HashMap<UserId, Account>>,
}

impl InMemoryAccountStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AccountStore for InMemoryAccountStore {
    fn get(&self, user: &UserId) -> Result<Option<Account>> {
        self.accounts
            .read()
            .map(|m| m.get(user).cloned())
            .map_err(|_| AuthError::Backend("account lock poisoned".to_owned()))
    }

    fn upsert(&self, account: Account) -> Result<()> {
        self.accounts
            .write()
            .map(|mut m| {
                m.insert(account.user.clone(), account);
            })
            .map_err(|_| AuthError::Backend("account lock poisoned".to_owned()))
    }
}
