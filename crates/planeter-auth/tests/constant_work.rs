//! IS-31: a password sign-in does the same work whether or not the account exists. A counting
//! stand-in hasher makes "the same work" deterministic (no wall-clock assertions): each of the three
//! failure cases performs exactly one `verify`, and the dummy never authenticates anyone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use planeter_auth::{
    Account, AccountStore, AuthError, Authenticator, InMemoryAccountStore, InMemoryCredentialStore,
    PasswordHash, PasswordHasher, Sha256TokenHasher,
};
use planeter_core::Principal;
use planeter_store::UserId;

/// Hashes to `count:<password>`; counts calls; can be told to accept everything.
#[derive(Default)]
struct CountingHasher {
    hashes: AtomicUsize,
    verifies: AtomicUsize,
    accept_all: AtomicBool,
}

impl PasswordHasher for CountingHasher {
    fn hash(&self, password: &str) -> PasswordHash {
        self.hashes.fetch_add(1, Ordering::SeqCst);
        PasswordHash(format!("count:{password}"))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        self.accept_all.load(Ordering::SeqCst) || hash.0 == format!("count:{password}")
    }
}

fn setup() -> (Authenticator, Arc<CountingHasher>) {
    let hasher = Arc::new(CountingHasher::default());
    let accounts = Arc::new(InMemoryAccountStore::new());
    accounts
        .upsert(Account {
            user: UserId::new("alice"),
            display_name: "Alice".into(),
            email: None,
            password: Some(hasher.hash("s3cret")),
        })
        .unwrap();
    accounts
        .upsert(Account {
            user: UserId::new("sso-only"),
            display_name: "SSO only".into(),
            email: None,
            password: None,
        })
        .unwrap();
    let auth = Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        Arc::new(Sha256TokenHasher),
    );
    (auth, hasher)
}

fn verifies_for(
    auth: &Authenticator,
    hasher: &CountingHasher,
    user: &str,
    pw: &str,
) -> (bool, usize) {
    let before = hasher.verifies.load(Ordering::SeqCst);
    let ok = matches!(auth.authenticate_password(user, pw), Ok(Principal::User(_)));
    (ok, hasher.verifies.load(Ordering::SeqCst) - before)
}

#[test]
fn construction_makes_one_dummy_hash_and_verifies_nothing() {
    let hasher = Arc::new(CountingHasher::default());
    let _ = Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        Arc::new(Sha256TokenHasher),
    );
    assert_eq!(hasher.hashes.load(Ordering::SeqCst), 1);
    assert_eq!(hasher.verifies.load(Ordering::SeqCst), 0);
}

#[test]
fn every_password_case_performs_exactly_one_verify() {
    let (auth, hasher) = setup();
    assert_eq!(
        verifies_for(&auth, &hasher, "alice", "s3cret"),
        (true, 1),
        "right password"
    );
    assert_eq!(
        verifies_for(&auth, &hasher, "alice", "wrong"),
        (false, 1),
        "wrong password"
    );
    assert_eq!(
        verifies_for(&auth, &hasher, "ghost", "whatever"),
        (false, 1),
        "unknown user"
    );
    assert_eq!(
        verifies_for(&auth, &hasher, "sso-only", "whatever"),
        (false, 1),
        "no password"
    );
}

#[test]
fn the_failures_are_the_same_error() {
    let (auth, _) = setup();
    for (u, p) in [("alice", "wrong"), ("ghost", "x"), ("sso-only", "x")] {
        assert!(
            matches!(
                auth.authenticate_password(u, p),
                Err(AuthError::BadCredentials)
            ),
            "{u}"
        );
    }
}

#[test]
fn the_dummy_never_authenticates_even_if_its_verification_says_yes() {
    let (auth, hasher) = setup();
    hasher.accept_all.store(true, Ordering::SeqCst);
    // An accept-everything hasher says yes to the dummy; the result must still be ignored.
    for (u, p) in [("ghost", "anything"), ("sso-only", "anything"), ("", "")] {
        let (ok, n) = verifies_for(&auth, &hasher, u, p);
        assert!(!ok, "{u:?} must not authenticate against the dummy");
        assert_eq!(n, 1, "{u:?}");
    }
}
