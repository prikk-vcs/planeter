//! RFC 014 D-6 / T4: a disabled account is refused on every path that turns a stored credential into a principal, **at each use**, and the
//! refusal is the coarse one everywhere except for a sign-in the provider has just verified.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use planeter_auth::{
    Account, AccountStore, AuthError, Authenticator, DeployKeyRecord, InMemoryAccountStore,
    InMemoryCredentialStore, PasswordHash, PasswordHasher, SessionStore, Sha256TokenHasher,
    SqliteAccountStore, SqliteCredentialStore, SqliteSessionStore, SshKeyRecord, TokenHasher,
    TokenRecord, VerifiedIdentity,
};
use planeter_core::{Principal, RepoScope, Scope, ScopeAccess};
use planeter_store::{RepoId, SqliteDb, UserId};

/// `h:<password>` hashes; counts verifications.
#[derive(Default)]
struct Counting {
    verifies: AtomicUsize,
}
impl PasswordHasher for Counting {
    fn hash(&self, password: &str) -> PasswordHash {
        PasswordHash(format!("h:{password}"))
    }
    fn verify(&self, password: &str, hash: &PasswordHash) -> bool {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        hash.0 == format!("h:{password}")
    }
}

fn scope(expires_at: Option<u64>) -> Scope {
    Scope {
        access: ScopeAccess::Full,
        repos: RepoScope::All,
        expires_at,
    }
}

struct World {
    auth: Authenticator,
    accounts: Arc<dyn AccountStore>,
    hasher: Arc<Counting>,
}

fn world(accounts: Arc<dyn AccountStore>, creds: Arc<InMemoryCredentialStore>) -> World {
    let hasher = Arc::new(Counting::default());
    for user in ["alice", "bob"] {
        accounts
            .upsert(Account::local(user, user, Some(hasher.hash("pw"))))
            .unwrap();
    }
    accounts
        .link_oidc("https://idp.example", "sub-alice", UserId::new("alice"))
        .unwrap();
    let t = Sha256TokenHasher;
    for (secret, user) in [("tok-alice", "alice"), ("tok-bob", "bob")] {
        creds
            .add_token(TokenRecord {
                hash: t.hash(secret),
                user: UserId::new(user),
                scope: scope(Some(5000)),
            })
            .unwrap();
    }
    creds
        .add_ssh_key(SshKeyRecord {
            fingerprint: "SHA256:alice".into(),
            user: UserId::new("alice"),
            scope: scope(None),
        })
        .unwrap();
    creds
        .add_deploy_key(DeployKeyRecord {
            fingerprint: "SHA256:deploy-alice".into(),
            registrant: UserId::new("alice"),
            repo: RepoId::new("r1"),
            write: true,
        })
        .unwrap();
    let auth = Authenticator::new(accounts.clone(), creds, hasher.clone(), Arc::new(t))
        .with_clock(|| 1000);
    World {
        auth,
        accounts,
        hasher,
    }
}

fn identity(subject: &str) -> VerifiedIdentity {
    VerifiedIdentity {
        issuer: "https://idp.example".into(),
        subject: subject.into(),
        email: None,
    }
}

fn alice() -> UserId {
    UserId::new("alice")
}

fn each_store(check: impl Fn(&World)) {
    check(&world(
        Arc::new(InMemoryAccountStore::new()),
        Arc::new(InMemoryCredentialStore::new()),
    ));
}

#[test]
fn every_path_works_before_and_is_refused_after_disable_and_works_again_after_enable() {
    each_store(|w| {
        let paths = |w: &World| -> [bool; 6] {
            [
                w.auth.authenticate_password("alice", "pw").is_ok(),
                w.auth.authenticate_oidc(&identity("sub-alice")).is_ok(),
                w.auth.authenticate_token("tok-alice").is_ok(),
                w.auth.authenticate_ssh_key("SHA256:alice").is_ok(),
                w.auth
                    .authenticate_deploy_key("SHA256:deploy-alice")
                    .is_ok(),
                w.auth.is_active(&alice()),
            ]
        };
        assert_eq!(paths(w), [true; 6], "before");
        assert!(
            w.accounts
                .disable(&alice(), "left the project", 900)
                .unwrap()
        );
        assert_eq!(paths(w), [false; 6], "after disable: every path, at once");
        // Bob is untouched.
        assert!(w.auth.authenticate_password("bob", "pw").is_ok());
        assert!(w.auth.authenticate_token("tok-bob").is_ok());
        assert!(w.accounts.enable(&alice()).unwrap());
        assert_eq!(
            paths(w),
            [true; 6],
            "after enable (a token until its own expiry)"
        );
    });
}

#[test]
fn a_disabled_account_is_refused_with_the_coarse_error_everywhere_but_for_the_provider() {
    each_store(|w| {
        w.accounts.disable(&alice(), "x", 1).unwrap();
        // The coarse refusal: the same value as a wrong password, an unknown token, an unknown key.
        assert_eq!(
            w.auth.authenticate_password("alice", "pw"),
            w.auth.authenticate_password("alice", "wrong")
        );
        assert_eq!(
            w.auth.authenticate_token("tok-alice"),
            w.auth.authenticate_token("tok-unknown")
        );
        assert_eq!(
            w.auth.authenticate_ssh_key("SHA256:alice"),
            w.auth.authenticate_ssh_key("SHA256:unknown")
        );
        assert_eq!(
            w.auth.authenticate_deploy_key("SHA256:deploy-alice"),
            w.auth.authenticate_deploy_key("SHA256:unknown")
        );
        assert_eq!(
            w.auth.authenticate_password("alice", "pw"),
            Err(AuthError::BadCredentials)
        );
        // The provider's verified identity is told: the one path with its own error.
        assert_eq!(
            w.auth.authenticate_oidc(&identity("sub-alice")),
            Err(AuthError::Disabled)
        );
        // An identity that is not linked stays "bad credentials" (the page says "not linked").
        assert_eq!(
            w.auth.authenticate_oidc(&identity("sub-nobody")),
            Err(AuthError::BadCredentials)
        );
    });
}

#[test]
fn a_disabled_account_s_password_costs_exactly_one_verify_like_a_wrong_one() {
    each_store(|w| {
        let count = || w.hasher.verifies.load(Ordering::SeqCst);
        w.accounts.disable(&alice(), "x", 1).unwrap();
        for (user, pw) in [("alice", "pw"), ("alice", "wrong"), ("ghost", "pw")] {
            let before = count();
            assert!(w.auth.authenticate_password(user, pw).is_err());
            assert_eq!(count() - before, 1, "{user}/{pw}");
        }
    });
}

#[test]
fn a_token_is_checked_at_each_use_not_only_when_it_was_made() {
    each_store(|w| {
        // The same token string, used again and again: the answer follows the account's state at that moment.
        enum Step {
            Use,
            Disable,
            Enable,
        }
        let script = [
            (Step::Use, true),
            (Step::Disable, false),
            (Step::Use, false),
            (Step::Enable, true),
            (Step::Use, true),
            (Step::Disable, false),
            (Step::Use, false),
        ];
        for (n, (step, expected)) in script.into_iter().enumerate() {
            match step {
                Step::Use => {}
                Step::Disable => {
                    w.accounts.disable(&alice(), "x", 1).unwrap();
                }
                Step::Enable => {
                    w.accounts.enable(&alice()).unwrap();
                }
            }
            assert_eq!(
                w.auth.authenticate_token("tok-alice").is_ok(),
                expected,
                "step {n}"
            );
        }
    });
}

#[test]
fn an_account_with_no_row_is_not_disabled_and_a_store_that_cannot_answer_fails_closed() {
    let accounts = Arc::new(InMemoryAccountStore::new());
    let creds = Arc::new(InMemoryCredentialStore::new());
    creds
        .add_token(TokenRecord {
            hash: Sha256TokenHasher.hash("tok-orphan"),
            user: UserId::new("orphan"),
            scope: scope(None),
        })
        .unwrap();
    let w = world(accounts, creds);
    // A handle that was never made an account row keeps working exactly as before (existing deployments and fixtures).
    assert!(w.auth.authenticate_token("tok-orphan").is_ok());
    assert!(w.auth.is_active(&UserId::new("orphan")));
}

#[test]
fn disable_is_idempotent_cuts_the_reason_and_makes_it_display_safe() {
    let db = SqliteDb::open_in_memory().unwrap();
    let store = SqliteAccountStore::new(db).unwrap();
    store
        .upsert(Account::local("alice", "Alice", None))
        .unwrap();
    assert!(
        !store.disable(&UserId::new("ghost"), "x", 1).unwrap(),
        "no such account"
    );
    let long = format!("a\u{202E}b{}", "z".repeat(500));
    assert!(store.disable(&alice(), &long, 77).unwrap());
    assert!(
        !store.disable(&alice(), "second reason", 99).unwrap(),
        "already disabled: nothing changes"
    );
    let a = store.get(&alice()).unwrap().unwrap();
    let d = a.disabled.unwrap();
    assert_eq!(d.at, 77, "the first time stands");
    assert!(
        d.reason.starts_with("a\\u{202E}b") && d.reason.chars().count() <= 200,
        "{}",
        d.reason
    );
    assert!(!store.enable(&UserId::new("ghost")).unwrap());
    assert!(store.enable(&alice()).unwrap());
    assert!(!store.enable(&alice()).unwrap(), "already enabled");
    assert!(store.get(&alice()).unwrap().unwrap().disabled.is_none());
}

#[test]
fn disabling_ends_the_accounts_sessions_in_the_same_transaction_and_a_failure_changes_nothing() {
    let db = SqliteDb::open_in_memory().unwrap();
    let accounts = SqliteAccountStore::new(db.clone()).unwrap();
    let sessions = SqliteSessionStore::new(db.clone()).unwrap();
    let _ = SqliteCredentialStore::new(db.clone()).unwrap();
    for u in ["alice", "bob"] {
        accounts.upsert(Account::local(u, u, None)).unwrap();
    }
    let a1 = sessions.create(alice(), 10, 5000).unwrap();
    let a2 = sessions.create(alice(), 11, 5000).unwrap();
    let b1 = sessions.create(UserId::new("bob"), 12, 5000).unwrap();

    // A failure inside the transaction (a trigger refuses the deletion of sessions) leaves the account **enabled** and every session.
    db.lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_session_delete BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    assert!(accounts.disable(&alice(), "x", 1).is_err());
    assert!(
        accounts.get(&alice()).unwrap().unwrap().disabled.is_none(),
        "rolled back: not disabled"
    );
    assert!(
        sessions.get(&a1.id, 20).unwrap().is_some() && sessions.get(&a2.id, 20).unwrap().is_some()
    );
    db.lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_session_delete;")
        .unwrap();

    assert!(accounts.disable(&alice(), "x", 1).unwrap());
    assert!(sessions.get(&a1.id, 20).unwrap().is_none());
    assert!(sessions.get(&a2.id, 20).unwrap().is_none());
    assert!(
        sessions.get(&b1.id, 20).unwrap().is_some(),
        "another account's session stays"
    );
    // Enabling does not bring an ended session back.
    accounts.enable(&alice()).unwrap();
    assert!(sessions.get(&a1.id, 20).unwrap().is_none());
}

#[test]
fn the_state_columns_survive_a_round_trip_and_a_sqlite_world_behaves_like_the_memory_one() {
    let db = SqliteDb::open_in_memory().unwrap();
    let accounts = Arc::new(SqliteAccountStore::new(db.clone()).unwrap());
    let creds = Arc::new(InMemoryCredentialStore::new());
    let w = world(accounts.clone(), creds);
    assert!(w.accounts.disable(&alice(), "reason here", 321).unwrap());
    let a = accounts.get(&alice()).unwrap().unwrap();
    assert_eq!(
        a.disabled.as_ref().map(|d| (d.at, d.reason.as_str())),
        Some((321, "reason here"))
    );
    assert_eq!(
        w.auth.authenticate_oidc(&identity("sub-alice")),
        Err(AuthError::Disabled)
    );
    assert!(matches!(
        w.auth.authenticate_password("bob", "pw"),
        Ok(Principal::User(_))
    ));
}
