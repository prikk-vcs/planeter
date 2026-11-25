//! Machine credentials (RFC 002 D-7): scoped access tokens, SSH keys, and deploy keys — the things a
//! non-interactive client presents. Each resolves to a [`Principal::MachineAsUser`](planeter_core::Principal)
//! carrying a [`Scope`], so `authorize()` enforces `user-permissions ∩ credential-scope`.
//!
//! Tokens are stored **hashed** (never plaintext) via the [`TokenHasher`](crate::hashing::TokenHasher)
//! seam; the store is keyed by the token hash. SSH/deploy keys are keyed by public-key fingerprint (the
//! signature *verification* itself is transport's concern, RFC 004 — here a fingerprint maps to a
//! registered credential).

use std::collections::HashMap;
use std::sync::RwLock;

use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{RepoId, UserId};

use crate::hashing::TokenHash;
use crate::identity::{AuthError, Result};

/// The longest name of a token, in characters (RFC 014 D-8).
pub const TOKEN_NAME_MAX: usize = 60;
/// The longest a token may live from the moment it is made: 365 days (RFC 014 D-8). A token without an end is not handed out.
pub const TOKEN_LIFETIME_MAX_SECS: u64 = 365 * 24 * 3600;
/// The most tokens an account may hold that have not expired, and the most [`CredentialStore::list_tokens`] returns.
pub const TOKENS_MAX: usize = 200;
/// A token's `last_used_at` is written at most this often (once an hour): reading a token must not be a write on every request.
pub const TOKEN_TOUCH_INTERVAL_SECS: u64 = 3600;
/// The name given to a token made before tokens had names.
pub const LEGACY_TOKEN_NAME: &str = "(created before 0.10.0)";

/// A scoped personal-access / OAuth token, stored by its hash.
///
/// `id` is the token's **public** identifier (16 lowercase hexadecimal characters): it names the token to its holder and to the operator
/// (`token revoke HANDLE ID`) and **cannot be used to present it**. A record built by hand with an empty `id` or `name` (a fixture, an
/// older caller) is given a fresh id and the placeholder name when it is stored with [`add_token`](InMemoryCredentialStore::add_token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    pub hash: TokenHash,
    pub user: UserId,
    pub scope: Scope,
    pub id: String,
    pub name: String,
    /// Unix seconds; `None` for a token made before this was recorded.
    pub created_at: Option<u64>,
    /// Unix seconds of the last use, **at hour resolution at best**; `None` when it was never used since this was recorded.
    pub last_used_at: Option<u64>,
}

/// A token as its holder is shown it: **no hash, no secret**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenListing {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub created_at: Option<u64>,
    pub last_used_at: Option<u64>,
}

/// A token about to be made (RFC 014 D-8): its hash (the secret was minted by [`mint_token`](crate::hashing::mint_token) and is shown
/// once by the caller; only the hash is stored), its holder, a name and a scope **with an expiry**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewToken {
    pub hash: TokenHash,
    pub user: UserId,
    pub name: String,
    pub scope: Scope,
}

/// Why a token was not made. Closed; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// The name is empty.
    NameEmpty,
    /// The name is longer than [`TOKEN_NAME_MAX`] characters.
    NameTooLong,
    /// The name carries a control, bidirectional or zero-width character.
    NameUnsafe,
    /// No expiry was given: a token must end.
    ExpiryRequired,
    /// The expiry is not in the future.
    ExpiryPast,
    /// The expiry is more than [`TOKEN_LIFETIME_MAX_SECS`] from now.
    ExpiryTooFar,
    /// The account already holds [`TOKENS_MAX`] tokens that have not expired.
    TooMany,
    Backend(String),
}

impl TokenError {
    /// A stable word for a message key and a log.
    pub fn reason(&self) -> &'static str {
        match self {
            TokenError::NameEmpty => "name_empty",
            TokenError::NameTooLong => "name_too_long",
            TokenError::NameUnsafe => "name_unsafe",
            TokenError::ExpiryRequired => "expiry_required",
            TokenError::ExpiryPast => "expiry_past",
            TokenError::ExpiryTooFar => "expiry_too_far",
            TokenError::TooMany => "too_many",
            TokenError::Backend(_) => "backend",
        }
    }
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the token was not made: {}", self.reason())
    }
}

impl std::error::Error for TokenError {}

/// The rules of a new token that need no store: the name (1 to [`TOKEN_NAME_MAX`] characters, display-safe: **refused**, not escaped, a
/// name is what a person tells tokens apart by) and the expiry (present, in the future, at most [`TOKEN_LIFETIME_MAX_SECS`] from `now`).
pub fn check_new_token(new: &NewToken, now: u64) -> std::result::Result<(), TokenError> {
    let name = new.name.as_str();
    if name.trim().is_empty() {
        return Err(TokenError::NameEmpty);
    }
    if name.chars().count() > TOKEN_NAME_MAX {
        return Err(TokenError::NameTooLong);
    }
    if !crate::text::is_display_safe(name) {
        return Err(TokenError::NameUnsafe);
    }
    match new.scope.expires_at {
        None => Err(TokenError::ExpiryRequired),
        Some(at) if at <= now => Err(TokenError::ExpiryPast),
        Some(at) if at > now.saturating_add(TOKEN_LIFETIME_MAX_SECS) => {
            Err(TokenError::ExpiryTooFar)
        }
        Some(_) => Ok(()),
    }
}

/// Whether a token last used at `last` is due to be written again at `now`.
pub fn touch_due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|t| now >= t.saturating_add(TOKEN_TOUCH_INTERVAL_SECS))
}

/// A registered SSH public key mapping to a user (used by RFC 004 transport). Its scope defaults to the
/// user's full capability but may be narrowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshKeyRecord {
    pub fingerprint: String,
    pub user: UserId,
    pub scope: Scope,
}

/// A deploy key: a machine credential scoped to exactly one repository. Recorded against the user who
/// registered it (so decisions and audit tie to a real identity), with an access ceiling of read or
/// write — never merge-seal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployKeyRecord {
    pub fingerprint: String,
    pub registrant: UserId,
    pub repo: RepoId,
    /// Read-only or write. (Anything above `Write` is meaningless for a deploy key.)
    pub write: bool,
}

impl DeployKeyRecord {
    /// The scope a deploy key confers: its one repository, read or write.
    pub fn scope(&self) -> Scope {
        Scope {
            access: if self.write {
                ScopeAccess::Write
            } else {
                ScopeAccess::ReadOnly
            },
            repos: RepoScope::Only(vec![self.repo.clone()]),
            expires_at: None,
        }
    }
}

/// Persistence + lookup of machine credentials. Lookups are by the presented secret's hash/fingerprint.
pub trait CredentialStore: Send + Sync {
    fn find_token(&self, hash: &TokenHash) -> Result<Option<TokenRecord>>;
    /// The tokens of `user`, newest first, **at most [`TOKENS_MAX`]**. No hash, no secret.
    fn list_tokens(&self, user: &UserId) -> Result<Vec<TokenListing>>;
    /// Revoke the token with this public `id` **that belongs to `user`**: a person's id never reaches another's token (`Ok(false)`, and
    /// nothing changes, when `user` has no such token). The record is removed; the account's event log is the history.
    fn revoke_token(&self, user: &UserId, id: &str) -> Result<bool>;
    /// Record that the token with this hash was used at `now`. **Written at most once an hour per token**: the stored time is compared
    /// first, and a token used again within the hour writes nothing. `Ok(true)` when a write was made.
    fn touch_token(&self, hash: &TokenHash, now: u64) -> Result<bool>;
    fn find_ssh_key(&self, fingerprint: &str) -> Result<Option<SshKeyRecord>>;
    fn find_deploy_key(&self, fingerprint: &str) -> Result<Option<DeployKeyRecord>>;
}

/// An in-memory [`CredentialStore`] for tests/dev.
#[derive(Default)]
pub struct InMemoryCredentialStore {
    inner: RwLock<CredentialData>,
}

#[derive(Default)]
struct CredentialData {
    tokens: HashMap<String, TokenRecord>,
    ssh_keys: HashMap<String, SshKeyRecord>,
    deploy_keys: HashMap<String, DeployKeyRecord>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, CredentialData>> {
        self.inner
            .write()
            .map_err(|_| AuthError::Backend("credential lock poisoned".to_owned()))
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, CredentialData>> {
        self.inner
            .read()
            .map_err(|_| AuthError::Backend("credential lock poisoned".to_owned()))
    }

    /// Store a token record as it is (a fixture, a migration): an empty `id` is given a fresh one and an empty `name` the placeholder.
    /// **No rule of a new token is applied here**; [`create_token`](Self::create_token) is the door for a token a person asks for.
    pub fn add_token(&self, mut record: TokenRecord) -> Result<()> {
        if record.id.is_empty() {
            record.id = crate::sqlite::random_token_id();
        }
        if record.name.is_empty() {
            record.name = LEGACY_TOKEN_NAME.to_owned();
        }
        self.write()?.tokens.insert(record.hash.0.clone(), record);
        Ok(())
    }

    /// Make a token a person asked for: the rules of [`check_new_token`], and at most [`TOKENS_MAX`] unexpired tokens per account.
    pub fn create_token(
        &self,
        new: NewToken,
        now: u64,
    ) -> std::result::Result<TokenRecord, TokenError> {
        check_new_token(&new, now)?;
        let mut data = self
            .write()
            .map_err(|e| TokenError::Backend(e.to_string()))?;
        let live = data
            .tokens
            .values()
            .filter(|t| t.user == new.user && t.scope.expires_at.is_none_or(|e| e > now))
            .count();
        if live >= TOKENS_MAX {
            return Err(TokenError::TooMany);
        }
        let record = TokenRecord {
            hash: new.hash,
            user: new.user,
            scope: new.scope,
            id: crate::sqlite::random_token_id(),
            name: new.name,
            created_at: Some(now),
            last_used_at: None,
        };
        data.tokens.insert(record.hash.0.clone(), record.clone());
        Ok(record)
    }

    pub fn add_ssh_key(&self, record: SshKeyRecord) -> Result<()> {
        self.write()?
            .ssh_keys
            .insert(record.fingerprint.clone(), record);
        Ok(())
    }

    pub fn add_deploy_key(&self, record: DeployKeyRecord) -> Result<()> {
        self.write()?
            .deploy_keys
            .insert(record.fingerprint.clone(), record);
        Ok(())
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn find_token(&self, hash: &TokenHash) -> Result<Option<TokenRecord>> {
        Ok(self.read()?.tokens.get(&hash.0).cloned())
    }
    fn list_tokens(&self, user: &UserId) -> Result<Vec<TokenListing>> {
        let data = self.read()?;
        let mut mine: Vec<&TokenRecord> =
            data.tokens.values().filter(|t| &t.user == user).collect();
        // Newest first; a token with no creation time (made before it was recorded) after every dated one.
        mine.sort_by(|a, b| {
            b.created_at
                .unwrap_or(0)
                .cmp(&a.created_at.unwrap_or(0))
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(mine
            .into_iter()
            .take(TOKENS_MAX)
            .map(|t| TokenListing {
                id: t.id.clone(),
                name: t.name.clone(),
                scope: t.scope.clone(),
                created_at: t.created_at,
                last_used_at: t.last_used_at,
            })
            .collect())
    }

    fn revoke_token(&self, user: &UserId, id: &str) -> Result<bool> {
        let mut data = self.write()?;
        let key = data
            .tokens
            .iter()
            .find(|(_, t)| &t.user == user && t.id == id)
            .map(|(k, _)| k.clone());
        Ok(key.is_some_and(|k| data.tokens.remove(&k).is_some()))
    }

    fn touch_token(&self, hash: &TokenHash, now: u64) -> Result<bool> {
        let mut data = self.write()?;
        match data.tokens.get_mut(&hash.0) {
            Some(t) if touch_due(t.last_used_at, now) => {
                t.last_used_at = Some(now);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn find_ssh_key(&self, fingerprint: &str) -> Result<Option<SshKeyRecord>> {
        Ok(self.read()?.ssh_keys.get(fingerprint).cloned())
    }
    fn find_deploy_key(&self, fingerprint: &str) -> Result<Option<DeployKeyRecord>> {
        Ok(self.read()?.deploy_keys.get(fingerprint).cloned())
    }
}
