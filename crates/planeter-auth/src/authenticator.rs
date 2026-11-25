//! The [`Authenticator`] (RFC 002 T2 / D-2): turns a presented credential into a
//! [`Principal`]. This is the boundary `STD-2` describes — authentication
//! lives here; its *output* is the principal that `authorize()` (core) consumes, and nothing else in
//! planeter sees a password, token, or key.
//!
//! Every failure returns the coarse [`AuthError::BadCredentials`] so an attacker cannot distinguish
//! "no such account" from "wrong password" from "unknown token" (`T-7`). For passwords the cases are
//! indistinguishable **in work as well as in response** (IS-31): an unknown account, an account without
//! a password and a wrong password each perform exactly one [`PasswordHasher::verify`] (the first two
//! against a dummy hash made at construction), so neither the response time nor a load-shedding refusal
//! reveals whether an account exists. Credential **expiry** is
//! enforced both here (reject at authentication) and again structurally by `authorize()` (the scope
//! carries `expires_at`), so an expired credential cannot act even if a caller skips this check.

use std::sync::Arc;

use planeter_core::Principal;
use planeter_store::UserId;

use crate::credential::CredentialStore;
use crate::hashing::{PasswordHash, PasswordHasher, TokenHasher};
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
    /// A hash made at construction, by the same hasher and parameters, of a random password that is
    /// never stored or logged. A sign-in with no real hash to check verifies against this instead, so
    /// every password sign-in costs one verification (IS-31). Its result is always ignored.
    dummy_hash: PasswordHash,
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
        let dummy_hash = password_hasher.hash(&crate::hashing::mint_token());
        Self {
            accounts,
            credentials,
            password_hasher,
            token_hasher,
            dummy_hash,
            now_unix: default_now,
        }
    }

    /// Override the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// The name a signed-in person is shown under (their own page header). `None` when the account has
    /// none to show (no such account, an empty name, or a store that cannot answer); callers then show the
    /// account id. It reveals nothing a signed-in person does not already know about themself.
    pub fn display_name(&self, user: &UserId) -> Option<String> {
        let account = self.accounts.get(user).ok().flatten()?;
        let name = account.display_name.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }

    /// The unauthenticated principal.
    pub fn anonymous(&self) -> Principal {
        Principal::Anonymous
    }

    /// Interactive sign-in: verify a local password → a `User` principal (a session).
    ///
    /// **The presented password is normalized (Unicode NFKC) before it is verified**, exactly as a password is normalized when it is set
    /// (`password::check_new_password`), so a password set through one input method verifies through any other; for ASCII nothing changes,
    /// so every hash made before the rule existed still verifies. A presented password too long to have been set under the rule is a wrong
    /// password and costs the same single verification (against the dummy). After a correct sign-in a hash made with older Argon2id
    /// parameters is replaced by one made with today's (`PasswordHasher::needs_rehash`).
    pub fn authenticate_password(&self, user: &str, password: &str) -> Result<Principal> {
        let user = UserId::new(user);
        // A store failure is a backend error, not "no such account", and is returned as such. Beyond
        // that there is no early return: the account's absence must not shorten the work.
        let account = self.accounts.get(&user)?;
        let presented = crate::password::normalize_presented(password);
        match (
            account.as_ref().and_then(|a| a.password.as_ref()),
            &presented,
        ) {
            (Some(hash), Some(presented)) => {
                if self.password_hasher.verify(presented.as_str(), hash) {
                    // A disabled account's password is checked like any other (the same one verification) and then refused as a wrong
                    // password is: the answer and the work are the same (RFC 014 D-6, IS-31).
                    if account.as_ref().is_some_and(|a| a.disabled.is_some()) {
                        return Err(AuthError::BadCredentials);
                    }
                    if self.password_hasher.needs_rehash(hash) {
                        // The password is right and was hashed with older parameters: store a fresh hash. A failure to store it
                        // is not a failed sign-in (the next one tries again).
                        let fresh = self.password_hasher.hash(presented.as_str());
                        let _ = self.accounts.set_password_hash(&user, fresh);
                    }
                    Ok(Principal::User(user))
                } else {
                    Err(AuthError::BadCredentials)
                }
            }
            _ => {
                // No real hash (or an input no password could be): do the same work against the dummy and refuse whatever it says.
                let text = presented.as_ref().map_or("", |p| p.as_str());
                let _ = self.password_hasher.verify(text, &self.dummy_hash);
                Err(AuthError::BadCredentials)
            }
        }
    }

    /// A verified OIDC identity → the **linked** local account → a `User` principal. No
    /// auto-provisioning: an unlinked `(issuer, subject)` is refused like any bad credential.
    pub fn authenticate_oidc(&self, identity: &VerifiedIdentity) -> Result<Principal> {
        let user = self
            .accounts
            .lookup_oidc(&identity.issuer, &identity.subject)?
            .ok_or(AuthError::BadCredentials)?;
        let account = self.accounts.get(&user)?.ok_or(AuthError::BadCredentials)?;
        if account.disabled.is_some() {
            // The provider has just proved who this is, so the person is told plainly (RFC 014 D-6): nothing is revealed to anyone else.
            return Err(AuthError::Disabled);
        }
        Ok(Principal::User(user))
    }

    /// Whether `user` may act at all: `false` when the account is **disabled** (or the store cannot say: it fails closed). An account that
    /// has no row at all is not disabled (a token or a session can name a handle that was never made an account row). Every path that
    /// turns a stored credential into a principal asks this at each use: a token, a session, a key.
    pub fn is_active(&self, user: &UserId) -> bool {
        self.accounts
            .get(user)
            .is_ok_and(|a| a.is_none_or(|a| a.disabled.is_none()))
    }

    /// `Err(BadCredentials)` when [`Authenticator::is_active`] says no (and for a store failure).
    fn require_active(&self, user: &UserId) -> Result<()> {
        if self.is_active(user) {
            Ok(())
        } else {
            Err(AuthError::BadCredentials)
        }
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
        // At each use, not only at creation: the tokens of a disabled account are refused, as an unknown token is.
        self.require_active(&record.user)?;
        // The time of this use, written at most once an hour per token (the record says when it was last written, so a token used
        // again within the hour costs no write and not even a second read). A failure to write is not a failed sign-in.
        let now = (self.now_unix)();
        if crate::credential::touch_due(record.last_used_at, now) {
            let _ = self.credentials.touch_token(&hash, now);
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
        self.require_active(&record.user)?;
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
        self.require_active(&record.registrant)?;
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
    fn the_display_name_is_the_account_s_trimmed_name_or_nothing() {
        let (auth, accounts, _) = fixture();
        for (id, name) in [("a", "Alice  "), ("b", ""), ("c", " \t ")] {
            accounts
                .upsert(Account {
                    created_at: None,
                    disabled: None,
                    password_changed_at: None,
                    user: UserId::new(id),
                    display_name: name.into(),
                    email: None,
                    password: None,
                })
                .unwrap();
        }
        assert_eq!(
            auth.display_name(&UserId::new("a")).as_deref(),
            Some("Alice")
        );
        assert_eq!(auth.display_name(&UserId::new("b")), None, "an empty name");
        assert_eq!(auth.display_name(&UserId::new("c")), None, "a blank name");
        assert_eq!(
            auth.display_name(&UserId::new("ghost")),
            None,
            "no such account"
        );
    }

    #[test]
    fn password_sign_in_yields_a_user_principal() {
        let (auth, accounts, _) = fixture();
        let hasher = InsecureStubHasher;
        accounts
            .upsert(Account {
                created_at: None,
                disabled: None,
                password_changed_at: None,
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
                id: String::new(),
                name: String::new(),
                created_at: None,
                last_used_at: None,
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
                id: String::new(),
                name: String::new(),
                created_at: None,
                last_used_at: None,
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
