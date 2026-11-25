//! RFC 014 D-8 / T5: the token record (an id, a name, a creation time, a last use), the rules of a new token, the bound of 200, the revocation
//! by id **and** holder, and the once-an-hour write of `last_used_at`. One contract, run over the in-memory store and the SQLite store.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use planeter_auth::{
    AuthError, Authenticator, CredentialStore, InMemoryAccountStore, InMemoryCredentialStore,
    LEGACY_TOKEN_NAME, NewToken, PasswordHash, PasswordHasher, Sha256TokenHasher,
    SqliteCredentialStore, TOKEN_NAME_MAX, TOKENS_MAX, TokenError, TokenHash, TokenHasher,
    TokenListing, TokenRecord,
};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{SqliteDb, UserId};

const NOW: u64 = 1_000_000;
const DAY: u64 = 86_400;

/// What the contract needs of a store: the trait, the raw writer of a record, and the door for a token a person asks for.
trait Store: CredentialStore {
    fn raw(&self, rec: TokenRecord);
    fn make(&self, new: NewToken, now: u64) -> Result<TokenRecord, TokenError>;
}
impl Store for InMemoryCredentialStore {
    fn raw(&self, rec: TokenRecord) {
        self.add_token(rec).unwrap();
    }
    fn make(&self, new: NewToken, now: u64) -> Result<TokenRecord, TokenError> {
        self.create_token(new, now)
    }
}
impl Store for SqliteCredentialStore {
    fn raw(&self, rec: TokenRecord) {
        self.add_token(rec).unwrap();
    }
    fn make(&self, new: NewToken, now: u64) -> Result<TokenRecord, TokenError> {
        self.create_token(new, now)
    }
}

fn each(contract: impl Fn(&dyn Store)) {
    contract(&InMemoryCredentialStore::new());
    contract(&SqliteCredentialStore::new(SqliteDb::open_in_memory().unwrap()).unwrap());
}

fn scope(expires_at: Option<u64>) -> Scope {
    Scope {
        access: ScopeAccess::Write,
        repos: RepoScope::All,
        expires_at,
    }
}

fn new_token(secret: &str, user: &str, name: &str, expires_at: Option<u64>) -> NewToken {
    NewToken {
        hash: Sha256TokenHasher.hash(secret),
        user: UserId::new(user),
        name: name.to_owned(),
        scope: scope(expires_at),
    }
}

fn alice() -> UserId {
    UserId::new("alice")
}

#[test]
fn a_new_token_needs_a_name_of_1_to_60_safe_characters_and_an_expiry_within_365_days() {
    each(|s| {
        assert_eq!(TOKEN_NAME_MAX, 60);
        let ok = |secret: &str, name: &str, exp: Option<u64>| {
            s.make(new_token(secret, "alice", name, exp), NOW)
        };
        // The name.
        assert_eq!(
            ok("t1", "", Some(NOW + DAY)).unwrap_err(),
            TokenError::NameEmpty
        );
        assert_eq!(
            ok("t1", "   ", Some(NOW + DAY)).unwrap_err(),
            TokenError::NameEmpty
        );
        assert_eq!(
            ok("t1", &"n".repeat(61), Some(NOW + DAY)).unwrap_err(),
            TokenError::NameTooLong
        );
        assert_eq!(
            ok("t1", "line\nbreak", Some(NOW + DAY)).unwrap_err(),
            TokenError::NameUnsafe
        );
        assert_eq!(
            ok("t1", "rtl\u{202E}name", Some(NOW + DAY)).unwrap_err(),
            TokenError::NameUnsafe
        );
        assert!(ok("t2", &"n".repeat(60), Some(NOW + DAY)).is_ok());
        assert!(
            ok("t3", &"\u{3042}".repeat(60), Some(NOW + DAY)).is_ok(),
            "60 characters, not 60 bytes"
        );
        // The expiry: required, in the future, at most 365 days from now.
        assert_eq!(ok("t4", "x", None).unwrap_err(), TokenError::ExpiryRequired);
        assert_eq!(
            ok("t4", "x", Some(NOW)).unwrap_err(),
            TokenError::ExpiryPast
        );
        assert_eq!(
            ok("t4", "x", Some(NOW - 1)).unwrap_err(),
            TokenError::ExpiryPast
        );
        assert_eq!(
            ok("t4", "x", Some(NOW + 365 * DAY + 1)).unwrap_err(),
            TokenError::ExpiryTooFar
        );
        assert!(
            ok("t5", "x", Some(NOW + 365 * DAY)).is_ok(),
            "365 days exactly"
        );
        assert!(ok("t6", "x", Some(NOW + 1)).is_ok());
        // Every refusal changed nothing: only the four that were made exist (t2, t3, t5, t6).
        assert_eq!(s.list_tokens(&alice()).unwrap().len(), 4);
        assert_eq!(TokenError::ExpiryRequired.reason(), "expiry_required");
    });
}

#[test]
fn a_made_token_has_a_public_id_a_name_and_a_creation_time_and_the_listing_has_no_secret() {
    each(|s| {
        let rec = s
            .make(
                new_token("secret-one", "alice", "laptop", Some(NOW + 30 * DAY)),
                NOW,
            )
            .unwrap();
        assert_eq!(rec.id.len(), 16);
        assert!(
            rec.id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(
            (rec.name.as_str(), rec.created_at, rec.last_used_at),
            ("laptop", Some(NOW), None)
        );
        assert_eq!(
            s.find_token(&Sha256TokenHasher.hash("secret-one"))
                .unwrap()
                .unwrap(),
            rec
        );
        let listing = s.list_tokens(&alice()).unwrap();
        assert_eq!(listing.len(), 1);
        assert_eq!(
            (listing[0].id.as_str(), listing[0].name.as_str()),
            (rec.id.as_str(), "laptop")
        );
        // The listing carries no hash and no secret, and the id is neither.
        let shown = format!("{:?}", listing[0]);
        assert!(
            !shown.contains("secret-one")
                && !shown.contains(&Sha256TokenHasher.hash("secret-one").0),
            "{shown}"
        );
        assert_ne!(rec.id, Sha256TokenHasher.hash("secret-one").0);
        assert!(
            s.list_tokens(&UserId::new("bob")).unwrap().is_empty(),
            "another holder sees none"
        );
    });
}

#[test]
fn the_list_is_newest_first_and_never_more_than_200_and_the_201st_live_token_is_refused() {
    each(|s| {
        for i in 1..=205u64 {
            s.raw(TokenRecord {
                hash: Sha256TokenHasher.hash(&format!("old-{i}")),
                user: alice(),
                scope: scope(None),
                id: format!("{i:016x}"),
                name: format!("t{i}"),
                created_at: Some(i),
                last_used_at: None,
            });
        }
        let list = s.list_tokens(&alice()).unwrap();
        assert_eq!(list.len(), TOKENS_MAX);
        assert_eq!(
            (list[0].name.as_str(), list[199].name.as_str()),
            ("t205", "t6")
        );
        let times: Vec<_> = list
            .iter()
            .map(|t: &TokenListing| t.created_at.unwrap())
            .collect();
        assert!(times.windows(2).all(|w| w[0] > w[1]), "newest first");
        // 205 live tokens already: a new one is refused and nothing is made.
        assert_eq!(
            s.make(
                new_token("fresh", "alice", "one too many", Some(NOW + DAY)),
                NOW
            )
            .unwrap_err(),
            TokenError::TooMany
        );
        assert!(
            s.find_token(&Sha256TokenHasher.hash("fresh"))
                .unwrap()
                .is_none()
        );
        // Another account is not affected by alice's count.
        assert!(
            s.make(new_token("bobs", "bob", "mine", Some(NOW + DAY)), NOW)
                .is_ok()
        );
    });
}

#[test]
fn expired_tokens_do_not_count_against_the_bound() {
    each(|s| {
        for i in 1..=TOKENS_MAX as u64 {
            s.raw(TokenRecord {
                hash: Sha256TokenHasher.hash(&format!("e-{i}")),
                user: alice(),
                scope: scope(Some(NOW - 1)), // all expired
                id: format!("{i:016x}"),
                name: format!("e{i}"),
                created_at: Some(i),
                last_used_at: None,
            });
        }
        assert!(
            s.make(new_token("alive", "alice", "alive", Some(NOW + DAY)), NOW)
                .is_ok()
        );
    });
}

#[test]
fn a_token_is_revoked_by_its_id_and_its_holder_and_a_wrong_holder_changes_nothing() {
    each(|s| {
        let a = s
            .make(new_token("tok-a", "alice", "a", Some(NOW + DAY)), NOW)
            .unwrap();
        let b = s
            .make(new_token("tok-b", "bob", "b", Some(NOW + DAY)), NOW)
            .unwrap();
        // Bob presents alice's id: nothing happens, to either token.
        assert!(!s.revoke_token(&UserId::new("bob"), &a.id).unwrap());
        assert!(
            s.find_token(&Sha256TokenHasher.hash("tok-a"))
                .unwrap()
                .is_some()
        );
        assert_eq!(s.list_tokens(&alice()).unwrap().len(), 1);
        // An id nobody has, and an id of the other kind.
        assert!(!s.revoke_token(&alice(), "0000000000000000").unwrap());
        assert!(!s.revoke_token(&alice(), &b.id).unwrap());
        assert!(
            s.find_token(&Sha256TokenHasher.hash("tok-b"))
                .unwrap()
                .is_some()
        );
        // The holder revokes: the token is gone and refused.
        assert!(s.revoke_token(&alice(), &a.id).unwrap());
        assert!(
            s.find_token(&Sha256TokenHasher.hash("tok-a"))
                .unwrap()
                .is_none()
        );
        assert!(s.list_tokens(&alice()).unwrap().is_empty());
        assert!(
            !s.revoke_token(&alice(), &a.id).unwrap(),
            "twice is nothing"
        );
    });
}

/// A store that counts how the authenticator uses `touch_token`, over the real store.
struct Counting<S> {
    inner: S,
    touches: AtomicUsize,
    writes: AtomicUsize,
}
impl<S: Store> CredentialStore for Counting<S> {
    fn find_token(&self, h: &TokenHash) -> planeter_auth::identity::Result<Option<TokenRecord>> {
        self.inner.find_token(h)
    }
    fn list_tokens(&self, u: &UserId) -> planeter_auth::identity::Result<Vec<TokenListing>> {
        self.inner.list_tokens(u)
    }
    fn revoke_token(&self, u: &UserId, id: &str) -> planeter_auth::identity::Result<bool> {
        self.inner.revoke_token(u, id)
    }
    fn touch_token(&self, h: &TokenHash, now: u64) -> planeter_auth::identity::Result<bool> {
        self.touches.fetch_add(1, Ordering::SeqCst);
        let wrote = self.inner.touch_token(h, now)?;
        if wrote {
            self.writes.fetch_add(1, Ordering::SeqCst);
        }
        Ok(wrote)
    }
    fn find_ssh_key(
        &self,
        f: &str,
    ) -> planeter_auth::identity::Result<Option<planeter_auth::SshKeyRecord>> {
        self.inner.find_ssh_key(f)
    }
    fn find_deploy_key(
        &self,
        f: &str,
    ) -> planeter_auth::identity::Result<Option<planeter_auth::DeployKeyRecord>> {
        self.inner.find_deploy_key(f)
    }
}

static CLOCK: AtomicU64 = AtomicU64::new(NOW);
fn clock() -> u64 {
    CLOCK.load(Ordering::SeqCst)
}

struct StubHasher;
impl PasswordHasher for StubHasher {
    fn hash(&self, p: &str) -> PasswordHash {
        PasswordHash(p.to_owned())
    }
    fn verify(&self, p: &str, h: &PasswordHash) -> bool {
        p == h.0
    }
}

#[test]
fn last_used_is_written_at_most_once_an_hour_and_reading_is_not_a_write() {
    // One test owns the static clock: the in-memory and the SQLite store run one after the other.
    for sqlite in [false, true] {
        CLOCK.store(NOW, Ordering::SeqCst);
        let db = SqliteDb::open_in_memory().unwrap();
        let (store, probe): (Arc<dyn CredentialStore>, Option<SqliteDb>) = if sqlite {
            let inner = SqliteCredentialStore::new(db.clone()).unwrap();
            inner
                .make(new_token("tok", "alice", "t", Some(NOW + 300 * DAY)), NOW)
                .unwrap();
            (Arc::new(inner), Some(db))
        } else {
            let inner = InMemoryCredentialStore::new();
            inner
                .make(new_token("tok", "alice", "t", Some(NOW + 300 * DAY)), NOW)
                .unwrap();
            (Arc::new(inner), None)
        };
        let auth = Authenticator::new(
            Arc::new(InMemoryAccountStore::new()),
            store.clone(),
            Arc::new(StubHasher),
            Arc::new(Sha256TokenHasher),
        )
        .with_clock(clock);
        let last = || {
            store
                .find_token(&Sha256TokenHasher.hash("tok"))
                .unwrap()
                .unwrap()
                .last_used_at
        };
        let changes =
            |db: &Option<SqliteDb>| db.as_ref().map(|d| d.lock().unwrap().total_changes());

        // First use: written (the token was never used).
        assert_eq!(last(), None);
        assert!(auth.authenticate_token("tok").is_ok());
        assert_eq!(last(), Some(NOW));
        // 100 uses over the next 59 minutes: not one more write (the connection's own change counter does not move).
        let before = changes(&probe);
        for k in 1..=100u64 {
            CLOCK.store(NOW + k * 35, Ordering::SeqCst); // up to NOW + 3500 s
            assert!(auth.authenticate_token("tok").is_ok());
        }
        assert_eq!(last(), Some(NOW), "still the first time");
        assert_eq!(changes(&probe), before, "no row was written");
        // At the hour: written once; and once more only an hour after that.
        CLOCK.store(NOW + 3600, Ordering::SeqCst);
        assert!(auth.authenticate_token("tok").is_ok());
        assert_eq!(last(), Some(NOW + 3600));
        CLOCK.store(NOW + 3600 + 3599, Ordering::SeqCst);
        assert!(auth.authenticate_token("tok").is_ok());
        assert_eq!(last(), Some(NOW + 3600));
        CLOCK.store(NOW + 7200, Ordering::SeqCst);
        assert!(auth.authenticate_token("tok").is_ok());
        assert_eq!(last(), Some(NOW + 7200));
        // The store itself also compares first: a direct touch within the hour is not a write.
        let h = Sha256TokenHasher.hash("tok");
        let before = changes(&probe);
        assert!(!store.touch_token(&h, NOW + 7300).unwrap());
        assert_eq!(changes(&probe), before);
        assert!(
            !store
                .touch_token(&TokenHash("nobody".into()), NOW + 99_999)
                .unwrap(),
            "an unknown token is nothing"
        );
        // A refused token (expired) is not touched.
        CLOCK.store(NOW + 301 * DAY, Ordering::SeqCst);
        assert_eq!(
            auth.authenticate_token("tok"),
            Err(AuthError::BadCredentials)
        );
        assert_eq!(last(), Some(NOW + 7200));
    }
}

#[test]
fn the_authenticator_asks_the_store_to_touch_only_when_it_is_due() {
    let counting = Arc::new(Counting {
        inner: InMemoryCredentialStore::new(),
        touches: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
    });
    counting
        .inner
        .make(new_token("tok", "alice", "t", Some(NOW + 300 * DAY)), NOW)
        .unwrap();
    let auth = Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        counting.clone(),
        Arc::new(StubHasher),
        Arc::new(Sha256TokenHasher),
    )
    .with_clock(clock2);
    for k in 0..200u64 {
        CLOCK2.store(NOW + k * 10, Ordering::SeqCst); // 2,000 s
        assert!(auth.authenticate_token("tok").is_ok());
    }
    assert_eq!(
        counting.touches.load(Ordering::SeqCst),
        1,
        "200 uses in 33 minutes: one call"
    );
    assert_eq!(counting.writes.load(Ordering::SeqCst), 1);
    CLOCK2.store(NOW + 3600, Ordering::SeqCst);
    assert!(auth.authenticate_token("tok").is_ok());
    assert_eq!(counting.writes.load(Ordering::SeqCst), 2);
}

static CLOCK2: AtomicU64 = AtomicU64::new(NOW);
fn clock2() -> u64 {
    CLOCK2.load(Ordering::SeqCst)
}

#[test]
fn a_token_that_expired_is_refused_and_an_older_token_without_an_id_works_and_is_listed_with_the_placeholder()
 {
    each(|s| {
        // An older token, as 0.9.0 wrote it: stored with no id and no name (the writer gives it a placeholder).
        s.raw(TokenRecord {
            hash: Sha256TokenHasher.hash("tok-old"),
            user: alice(),
            scope: scope(None),
            id: String::new(),
            name: String::new(),
            created_at: None,
            last_used_at: None,
        });
        let list = s.list_tokens(&alice()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, LEGACY_TOKEN_NAME);
        assert_eq!(list[0].id.len(), 16);
        assert_eq!(list[0].created_at, None);
        // A token newer than it sorts before it.
        s.make(new_token("tok-new", "alice", "new", Some(NOW + DAY)), NOW)
            .unwrap();
        let names: Vec<_> = s
            .list_tokens(&alice())
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["new", LEGACY_TOKEN_NAME]);
    });
}
