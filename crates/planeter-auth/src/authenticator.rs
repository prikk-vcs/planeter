//! The [`Authenticator`] (RFC 002 T2 / D-2): turns a presented credential into a
//! [`Principal`]. This is the boundary `STD-2` describes — authentication
//! lives here; its *output* is the principal that `authorize()` (core) consumes, and nothing else in
//! planeter sees a password, token, or key.
//!
//! Every failure returns the coarse [`AuthError::BadCredentials`] so an attacker cannot distinguish
//! "no such account" from "wrong password" from "unknown token" (`T-7`). Credential **expiry** is
//! enforced both here (reject at authentication) and again structurally by `authorize()` (the scope
//! carries `expires_at`), so an expired credential cannot act even if a caller skips this check.

use std::sync::Arc;

use planeter_core::Principal;
use planeter_store::UserId;

use crate::credential::CredentialStore;
use crate::hashing::{PasswordHasher, TokenHasher};
use crate::identity::{AccountStore, AuthError, Result};
use crate::oidc::VerifiedIdentity;

/// Authenticates presented credentials into principals. Holds the identity/credential stores and the
/// hashing seams; a deployment wires the production Argon2id/token hashers here (owner-ruled: seams now,
/// vetted crypto later).
pub struct Authenticator {
    accounts: Arc<dyn AccountStore>,
    credentials: Arc<dyn CredentialStore>,
    password_hasher: Arc<dyn PasswordHasher>,
    token_hasher: Arc<dyn TokenHasher>,
    /// Current Unix seconds, injected for testable expiry checks.
    now_unix: fn() -> u64,
}

impl Authenticator {
    pub fn new(
        accounts: Arc<dyn AccountStore>,
        credentials: Arc<dyn CredentialStore>,
        password_hasher: Arc<dyn PasswordHasher>,
        token_hasher: Arc<dyn TokenHasher>,
    ) -> Self {
        Self {
            accounts,
            credentials,
            password_hasher,
            token_hasher,
            now_unix: default_now,
        }
    }

    /// Override the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// The unauthenticated principal.
    pub fn anonymous(&self) -> Principal {
        Principal::Anonymous
    }

    /// Interactive sign-in: verify a local password → a `User` principal (a session).
    pub fn authenticate_password(&self, user: &str, password: &str) -> Result<Principal> {
        let user = UserId::new(user);
        let account = self.accounts.get(&user)?.ok_or(AuthError::BadCredentials)?;
        let hash = account.password.as_ref().ok_or(AuthError::BadCredentials)?;
        if self.password_hasher.verify(password, hash) {
            Ok(Principal::User(user))
        } else {
            Err(AuthError::BadCredentials)
        }
    }

    /// A verified OIDC identity → the **linked** local account → a `User` principal. No
    /// auto-provisioning: an unlinked `(issuer, subject)` is refused like any bad credential.
    pub fn authenticate_oidc(&self, identity: &VerifiedIdentity) -> Result<Principal> {
        let user = self
            .accounts
            .lookup_oidc(&identity.issuer, &identity.subject)?
            .ok_or(AuthError::BadCredentials)?;
        self.accounts.get(&user)?.ok_or(AuthError::BadCredentials)?;
        Ok(Principal::User(user))
    }

    /// A scoped access/OAuth token → a `MachineAsUser` principal. Rejects an expired token.
    pub fn authenticate_token(&self, presented: &str) -> Result<Principal> {
        let hash = self.token_hasher.hash(presented);
        let record = self
            .credentials
            .find_token(&hash)?
            .ok_or(AuthError::BadCredentials)?;
        if record.scope.is_expired((self.now_unix)()) {
            return Err(AuthError::BadCredentials);
        }
        Ok(Principal::MachineAsUser {
            user: record.user,
            scope: record.scope,
        })
    }

    /// A registered SSH public-key fingerprint → a `MachineAsUser` principal (RFC 004 transport).
    pub fn authenticate_ssh_key(&self, fingerprint: &str) -> Result<Principal> {
        let record = self
            .credentials
            .find_ssh_key(fingerprint)?
            .ok_or(AuthError::BadCredentials)?;
        if record.scope.is_expired((self.now_unix)()) {
            return Err(AuthError::BadCredentials);
        }
        Ok(Principal::MachineAsUser {
            user: record.user,
            scope: record.scope,
        })
    }

    /// A deploy-key fingerprint → a `MachineAsUser` principal scoped to its one repository.
    pub fn authenticate_deploy_key(&self, fingerprint: &str) -> Result<Principal> {
        let record = self
            .credentials
            .find_deploy_key(fingerprint)?
            .ok_or(AuthError::BadCredentials)?;
        Ok(Principal::MachineAsUser {
            user: record.registrant.clone(),
            scope: record.scope(),
        })
    }
}

fn default_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::InMemoryCredentialStore;
    use crate::credential::{DeployKeyRecord, SshKeyRecord, TokenRecord};
    use crate::hashing::{InsecureStubHasher, PasswordHasher, TokenHasher};
    use crate::identity::{Account, InMemoryAccountStore};
    use planeter_core::{RepoScope, Scope, ScopeAccess};
    use planeter_store::{RepoId, UserId};

    fn fixture() -> (
        Authenticator,
        Arc<InMemoryAccountStore>,
        Arc<InMemoryCredentialStore>,
    ) {
        let accounts = Arc::new(InMemoryAccountStore::new());
        let creds = Arc::new(InMemoryCredentialStore::new());
        let hasher = Arc::new(InsecureStubHasher);
        let auth = Authenticator::new(
            accounts.clone(),
            creds.clone(),
            hasher.clone(),
            hasher.clone(),
        )
        .with_clock(|| 1000);
        (auth, accounts, creds)
    }

    #[test]
    fn password_sign_in_yields_a_user_principal() {
        let (auth, accounts, _) = fixture();
        let hasher = InsecureStubHasher;
        accounts
            .upsert(Account {
                user: UserId::new("alice"),
                display_name: "Alice".into(),
                email: None,
                password: Some(PasswordHasher::hash(&hasher, "s3cret")),
            })
            .unwrap();

        assert_eq!(
            auth.authenticate_password("alice", "s3cret").unwrap(),
            Principal::User(UserId::new("alice"))
        );
        // Wrong password and unknown user both fail identically.
        assert_eq!(
            auth.authenticate_password("alice", "nope"),
            Err(AuthError::BadCredentials)
        );
        assert_eq!(
            auth.authenticate_password("ghost", "whatever"),
            Err(AuthError::BadCredentials)
        );
    }

    #[test]
    fn token_yields_machine_principal_and_expiry_is_enforced() {
        let (auth, _, creds) = fixture();
        let hasher = InsecureStubHasher;
        // A live token.
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&hasher, "tok-live"),
                user: UserId::new("bob"),
                scope: Scope {
                    access: ScopeAccess::Write,
                    repos: RepoScope::All,
                    expires_at: Some(2000),
                },
            })
            .unwrap();
        match auth.authenticate_token("tok-live").unwrap() {
            Principal::MachineAsUser { user, .. } => assert_eq!(user, UserId::new("bob")),
            other => panic!("expected MachineAsUser, got {other:?}"),
        }
        // An expired token (expires 500, clock 1000) is refused.
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&hasher, "tok-dead"),
                user: UserId::new("bob"),
                scope: Scope {
                    access: ScopeAccess::Write,
                    repos: RepoScope::All,
                    expires_at: Some(500),
                },
            })
            .unwrap();
        assert_eq!(
            auth.authenticate_token("tok-dead"),
            Err(AuthError::BadCredentials)
        );
        // An unknown token is refused.
        assert_eq!(
            auth.authenticate_token("tok-unknown"),
            Err(AuthError::BadCredentials)
        );
    }

    #[test]
    fn deploy_key_is_scoped_to_one_repo() {
        let (auth, _, creds) = fixture();
        creds
            .add_deploy_key(DeployKeyRecord {
                fingerprint: "SHA256:abc".into(),
                registrant: UserId::new("carol"),
                repo: RepoId::new("r1"),
                write: true,
            })
            .unwrap();
        match auth.authenticate_deploy_key("SHA256:abc").unwrap() {
            Principal::MachineAsUser { user, scope } => {
                assert_eq!(user, UserId::new("carol"));
                assert_eq!(scope.access, ScopeAccess::Write);
                assert_eq!(scope.repos, RepoScope::Only(vec![RepoId::new("r1")]));
            }
            other => panic!("expected MachineAsUser, got {other:?}"),
        }
    }

    #[test]
    fn ssh_key_maps_to_its_user() {
        let (auth, _, creds) = fixture();
        creds
            .add_ssh_key(SshKeyRecord {
                fingerprint: "SHA256:def".into(),
                user: UserId::new("dave"),
                scope: Scope {
                    access: ScopeAccess::Full,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
        match auth.authenticate_ssh_key("SHA256:def").unwrap() {
            Principal::MachineAsUser { user, .. } => assert_eq!(user, UserId::new("dave")),
            other => panic!("expected MachineAsUser, got {other:?}"),
        }
        assert_eq!(
            auth.authenticate_ssh_key("SHA256:unknown"),
            Err(AuthError::BadCredentials)
        );
    }
}
