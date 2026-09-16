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

/// A scoped personal-access / OAuth token, stored by its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    pub hash: TokenHash,
    pub user: UserId,
    pub scope: Scope,
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

    pub fn add_token(&self, record: TokenRecord) -> Result<()> {
        self.write()?.tokens.insert(record.hash.0.clone(), record);
        Ok(())
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
    fn find_ssh_key(&self, fingerprint: &str) -> Result<Option<SshKeyRecord>> {
        Ok(self.read()?.ssh_keys.get(fingerprint).cloned())
    }
    fn find_deploy_key(&self, fingerprint: &str) -> Result<Option<DeployKeyRecord>> {
        Ok(self.read()?.deploy_keys.get(fingerprint).cloned())
    }
}
