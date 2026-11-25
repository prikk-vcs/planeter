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
    /// When the account was made (Unix seconds); `None` for an account that existed before this was recorded.
    pub created_at: Option<u64>,
    /// `Some` when the account is **disabled** (RFC 014 D-6): it cannot sign in, its sessions have ended and every token it holds is
    /// refused, at each use. An account is never deleted.
    pub disabled: Option<DisabledState>,
    /// When the password was last set (Unix seconds); `None` when it never was, or was before this was recorded.
    pub password_changed_at: Option<u64>,
}

/// Why and when an account was disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisabledState {
    /// Unix seconds.
    pub at: u64,
    /// The operator's reason: display-safe, at most 200 characters (escaped and cut when it was written).
    pub reason: String,
}

impl Account {
    /// An account with a handle, a name and (maybe) a password hash, and nothing else set. For tests and for the code that builds the
    /// account it is about to create.
    pub fn local(user: &str, display_name: &str, password: Option<PasswordHash>) -> Self {
        Self {
            user: UserId::new(user),
            display_name: display_name.to_owned(),
            email: None,
            password,
            created_at: None,
            disabled: None,
            password_changed_at: None,
        }
    }
}

/// An authentication failure (kept coarse so it never reveals whether an account exists — `T-7`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthError {
    /// The credential did not authenticate (no such account, wrong password, unknown/expired token).
    /// Deliberately indistinguishable across those cases.
    BadCredentials,
    /// **Only** [`Authenticator::authenticate_oidc`](crate::Authenticator::authenticate_oidc) returns this: the identity was verified by
    /// the provider and its linked account is disabled. The person is told plainly (RFC 014 D-6), because the provider has just proved who
    /// they are and a coarse refusal would only leave them asking why. A password, a token, a key and a session never say it: they answer
    /// as for any bad credential.
    Disabled,
    /// A backend failure reading identity/credential state.
    Backend(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::BadCredentials => f.write_str("authentication failed"),
            AuthError::Disabled => f.write_str("the account is disabled"),
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
    /// Replace an account's password hash with another made of **the same password** (the rehash after a sign-in with older parameters).
    /// It changes nothing else: no event, no session ends. An account that does not exist is left as it is.
    fn set_password_hash(&self, user: &UserId, hash: PasswordHash) -> Result<()>;
    /// Disable an account (RFC 014 D-6) and **end all its sessions in the same transaction** where the store holds them. `Ok(true)` when
    /// the account was disabled now, `Ok(false)` when there is no such account or it was disabled already (nothing changes then). The
    /// reason is made display-safe and cut at 200 characters.
    fn disable(&self, user: &UserId, reason: &str, at: u64) -> Result<bool>;
    /// Enable a disabled account again. `Ok(true)` when it was disabled and is not now; `Ok(false)` otherwise. A token it holds works
    /// again until its own expiry; a session that ended stays ended.
    fn enable(&self, user: &UserId) -> Result<bool>;
    /// Link an OIDC `(issuer, subject)` to a local account (an administrative act; no auto-provisioning).
    fn link_oidc(&self, issuer: &str, subject: &str, user: UserId) -> Result<()>;
    /// The account linked to an OIDC `(issuer, subject)`, if any.
    fn lookup_oidc(&self, issuer: &str, subject: &str) -> Result<Option<UserId>>;
}

/// An in-memory [`AccountStore`] for tests/dev.
#[derive(Default)]
pub struct InMemoryAccountStore {
    accounts: RwLock<HashMap<UserId, Account>>,
    oidc_links: RwLock<HashMap<(String, String), UserId>>,
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

    fn disable(&self, user: &UserId, reason: &str, at: u64) -> Result<bool> {
        // Sessions live in a separate in-memory store here, so they are not ended by this call; a session of a disabled account is
        // refused at resolution all the same (the authenticator checks), which is what the guarantee rests on. The SQLite store ends them.
        self.accounts
            .write()
            .map(|mut m| match m.get_mut(user) {
                Some(a) if a.disabled.is_none() => {
                    a.disabled = Some(DisabledState {
                        at,
                        reason: crate::text::escape_display(reason, 200),
                    });
                    true
                }
                _ => false,
            })
            .map_err(|_| AuthError::Backend("account lock poisoned".to_owned()))
    }

    fn enable(&self, user: &UserId) -> Result<bool> {
        self.accounts
            .write()
            .map(|mut m| match m.get_mut(user) {
                Some(a) if a.disabled.is_some() => {
                    a.disabled = None;
                    true
                }
                _ => false,
            })
            .map_err(|_| AuthError::Backend("account lock poisoned".to_owned()))
    }

    fn set_password_hash(&self, user: &UserId, hash: PasswordHash) -> Result<()> {
        self.accounts
            .write()
            .map(|mut m| {
                if let Some(a) = m.get_mut(user) {
                    a.password = Some(hash);
                }
            })
            .map_err(|_| AuthError::Backend("account lock poisoned".to_owned()))
    }

    fn link_oidc(&self, issuer: &str, subject: &str, user: UserId) -> Result<()> {
        self.oidc_links
            .write()
            .map(|mut m| {
                m.insert((issuer.to_owned(), subject.to_owned()), user);
            })
            .map_err(|_| AuthError::Backend("oidc link lock poisoned".to_owned()))
    }

    fn lookup_oidc(&self, issuer: &str, subject: &str) -> Result<Option<UserId>> {
        self.oidc_links
            .read()
            .map(|m| m.get(&(issuer.to_owned(), subject.to_owned())).cloned())
            .map_err(|_| AuthError::Backend("oidc link lock poisoned".to_owned()))
    }
}
