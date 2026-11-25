//! RFC 014 token commands, the service half: a token made for an account in one transaction with its event, the rules, the refusals that
//! change nothing, a failure that leaves no token and no event, and a revoke by id and holder. The secret never reaches this layer.

use std::sync::Arc;

use planeter_auth::{
    AccountError, AccountEventKind, AccountEventStore, AccountService, Argon2idHasher,
    Authenticator, CredentialStore, EventBy, Sha256TokenHasher, SqliteAccountEventStore,
    SqliteAccountStore, SqliteCredentialStore, TOKENS_MAX, TokenError, TokenHasher, TokenSpec,
    mint_token,
};
use planeter_core::{RepoScope, ScopeAccess};
use planeter_store::{SqliteDb, UserId};

const NOW: u64 = 1_795_564_800; // 2026-11-25
const DAY: u64 = 86_400;

struct World {
    db: SqliteDb,
    service: AccountService,
    creds: SqliteCredentialStore,
    events: SqliteAccountEventStore,
}

fn world() -> World {
    let db = SqliteDb::open_in_memory().unwrap();
    let service = AccountService::new(
        db.clone(),
        Arc::new(Argon2idHasher),
        Arc::new(|w: &str| w == "api"),
    )
    .unwrap();
    let _ = SqliteAccountStore::new(db.clone()).unwrap();
    for (h, n) in [("alice", "Alice A"), ("bob", "Bob B")] {
        service
            .create_account(h, n, None, EventBy::Operator, NOW)
            .unwrap();
    }
    World {
        creds: SqliteCredentialStore::new(db.clone()).unwrap(),
        events: SqliteAccountEventStore::new(db.clone()).unwrap(),
        db,
        service,
    }
}

fn repo(w: &World, id: &str, owner: &str, name: &str) {
    w.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, path) VALUES (?1, 'user', ?2, ?3, 'private', 0, '/p')",
            (id, owner, name),
        )
        .unwrap();
}

fn dump(db: &SqliteDb) -> Vec<String> {
    let conn = db.lock().unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    names
        .into_iter()
        .map(|t| {
            let cols: Vec<String> = conn
                .prepare(&format!(
                    "SELECT name FROM pragma_table_info('{t}') ORDER BY cid"
                ))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let select = cols
                .iter()
                .map(|c| format!("quote(\"{c}\")"))
                .collect::<Vec<_>>()
                .join(" || '|' || ");
            let rows: Vec<String> = conn
                .prepare(&format!("SELECT {select} FROM {t} ORDER BY rowid"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            format!("{t}: {}", rows.join(";;"))
        })
        .collect()
}

fn spec<'a>(
    handle: &'a str,
    secret: &str,
    name: &'a str,
    access: ScopeAccess,
    repos: &'a [(String, String)],
    days: u64,
) -> TokenSpec<'a> {
    TokenSpec {
        handle,
        hash: Sha256TokenHasher.hash(secret),
        name,
        access,
        repos,
        expires_at: NOW + days * DAY,
    }
}

fn auth(w: &World) -> Authenticator {
    Authenticator::new(
        Arc::new(SqliteAccountStore::new(w.db.clone()).unwrap()),
        Arc::new(w.creds.clone()),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    )
    .with_clock(|| NOW + 10)
}

#[test]
fn a_token_is_made_with_its_event_in_one_step_and_works_as_its_scope_says() {
    let w = world();
    repo(&w, "r1", "alice", "forge");
    repo(&w, "r2", "alice", "other");
    let secret = mint_token();
    let only = [("alice".to_owned(), "forge".to_owned())];
    let made = w
        .service
        .create_token_for(
            spec("alice", &secret, "laptop", ScopeAccess::Write, &only, 90),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    assert_eq!(made.id.len(), 16);
    assert_eq!(
        (made.display_name.as_str(), made.repos.as_slice()),
        ("Alice A", ["alice/forge".to_owned()].as_slice())
    );
    // It authenticates, with the scope the operator asked for.
    match auth(&w).authenticate_token(&secret).unwrap() {
        planeter_core::Principal::MachineAsUser { user, scope } => {
            assert_eq!(user, UserId::new("alice"));
            assert_eq!(scope.access, ScopeAccess::Write);
            assert_eq!(
                scope.repos,
                RepoScope::Only(vec![planeter_store::RepoId::new("r1")])
            );
            assert_eq!(scope.expires_at, Some(NOW + 90 * DAY));
        }
        other => panic!("{other:?}"),
    }
    // The event: name, scope, repositories (a count), expiry date; and nothing of the token.
    let e = w.events.recent("alice", 1).unwrap().remove(0);
    assert_eq!(
        (e.kind, e.by),
        (AccountEventKind::TokenCreated, EventBy::Operator)
    );
    assert_eq!(e.detail, "laptop\twrite\t1\t2027-02-23");
    for forbidden in [
        secret.as_str(),
        &Sha256TokenHasher.hash(&secret).0,
        &made.id,
    ] {
        assert!(!e.detail.contains(forbidden), "{}", e.detail);
    }
    // And the listing shows it, newest first, with the id and no secret.
    let list = w.creds.list_tokens(&UserId::new("alice")).unwrap();
    assert_eq!(
        (
            list[0].id.as_str(),
            list[0].name.as_str(),
            list[0].created_at
        ),
        (made.id.as_str(), "laptop", Some(NOW))
    );
    assert!(!format!("{list:?}").contains(&secret));
}

#[test]
fn every_refusal_changes_nothing() {
    let w = world();
    repo(&w, "r1", "alice", "forge");
    w.service
        .disable("bob", "away", EventBy::Operator, NOW)
        .unwrap();
    let before = dump(&w.db);
    let none: [(String, String); 0] = [];
    let long_name = "n".repeat(61);
    let ghost = [("alice".to_owned(), "ghost".to_owned())];
    let cases: Vec<(TokenSpec<'_>, AccountError)> = vec![
        (
            spec("nobody", "s1", "n", ScopeAccess::Full, &none, 90),
            AccountError::NoSuchAccount,
        ),
        (
            spec("bob", "s2", "n", ScopeAccess::Full, &none, 90),
            AccountError::AccountDisabled,
        ),
        (
            spec("alice", "s3", "", ScopeAccess::Full, &none, 90),
            AccountError::Token(TokenError::NameEmpty),
        ),
        (
            spec("alice", "s4", &long_name, ScopeAccess::Full, &none, 90),
            AccountError::Token(TokenError::NameTooLong),
        ),
        (
            spec("alice", "s5", "bad\nname", ScopeAccess::Full, &none, 90),
            AccountError::Token(TokenError::NameUnsafe),
        ),
        (
            spec("alice", "s6", "n", ScopeAccess::Full, &none, 366),
            AccountError::Token(TokenError::ExpiryTooFar),
        ),
        (
            spec("alice", "s7", "n", ScopeAccess::Full, &ghost, 90),
            AccountError::RepositoryUnknown("alice/ghost".to_owned()),
        ),
    ];
    for (s, want) in cases {
        let handle = s.handle.to_owned();
        assert_eq!(
            w.service.create_token_for(s, EventBy::Operator, NOW).err(),
            Some(want),
            "{handle}"
        );
        assert_eq!(dump(&w.db), before, "{handle}: nothing changed");
    }
    // An expiry that is not in the future.
    let past = TokenSpec {
        expires_at: NOW,
        ..spec("alice", "s8", "n", ScopeAccess::Full, &none, 1)
    };
    assert_eq!(
        w.service
            .create_token_for(past, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::Token(TokenError::ExpiryPast))
    );
    assert_eq!(dump(&w.db), before);
}

#[test]
fn the_201st_unexpired_token_is_refused_and_an_expired_one_does_not_count() {
    let w = world();
    let none: [(String, String); 0] = [];
    for i in 0..TOKENS_MAX {
        w.service
            .create_token_for(
                spec(
                    "alice",
                    &format!("s{i}"),
                    &format!("t{i}"),
                    ScopeAccess::ReadOnly,
                    &none,
                    30,
                ),
                EventBy::Operator,
                NOW,
            )
            .unwrap();
    }
    let before = dump(&w.db);
    assert_eq!(
        w.service
            .create_token_for(
                spec("alice", "one-more", "x", ScopeAccess::ReadOnly, &none, 30),
                EventBy::Operator,
                NOW
            )
            .err(),
        Some(AccountError::Token(TokenError::TooMany))
    );
    assert_eq!(dump(&w.db), before);
    // Thirty-one days later the first 200 have expired: a new one is made.
    let later = TokenSpec {
        expires_at: NOW + 61 * DAY,
        ..spec("alice", "later", "x", ScopeAccess::ReadOnly, &none, 30)
    };
    assert!(
        w.service
            .create_token_for(later, EventBy::Operator, NOW + 31 * DAY)
            .is_ok()
    );
}

#[test]
fn a_failure_after_the_token_row_leaves_no_token_and_no_event() {
    let w = world();
    let none: [(String, String); 0] = [];
    w.db.lock()
        .unwrap()
        .execute_batch("CREATE TRIGGER refuse_token_event BEFORE INSERT ON account_events WHEN NEW.kind = 'token_created' BEGIN SELECT RAISE(ABORT, 'refused'); END;")
        .unwrap();
    let before = dump(&w.db);
    let got = w.service.create_token_for(
        spec("alice", "boom", "n", ScopeAccess::Write, &none, 30),
        EventBy::Operator,
        NOW,
    );
    assert!(matches!(got, Err(AccountError::Backend(_))), "{got:?}");
    assert_eq!(
        dump(&w.db),
        before,
        "the token row was rolled back with the event"
    );
    assert!(
        w.creds
            .find_token(&Sha256TokenHasher.hash("boom"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_token_is_revoked_by_id_and_holder_with_an_event_that_names_the_token_only() {
    let w = world();
    let none: [(String, String); 0] = [];
    let a = w
        .service
        .create_token_for(
            spec(
                "alice",
                "tok-a",
                "alice's laptop",
                ScopeAccess::Write,
                &none,
                30,
            ),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    let b = w
        .service
        .create_token_for(
            spec("bob", "tok-b", "bob's", ScopeAccess::Write, &none, 30),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    let before = dump(&w.db);
    // Bob names alice's id: no such token, nothing written; and an id nobody has; and an account that does not exist.
    assert_eq!(
        w.service
            .revoke_token_of("bob", &a.id, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::NoSuchToken)
    );
    assert_eq!(
        w.service
            .revoke_token_of("alice", &b.id, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::NoSuchToken)
    );
    assert_eq!(
        w.service
            .revoke_token_of("alice", "0000000000000000", EventBy::Operator, NOW)
            .err(),
        Some(AccountError::NoSuchToken)
    );
    assert_eq!(
        w.service
            .revoke_token_of("ghost", &a.id, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::NoSuchAccount)
    );
    assert_eq!(dump(&w.db), before);
    assert!(auth(&w).authenticate_token("tok-a").is_ok());
    // The holder revokes: the next use is refused; the event says the name and no secret.
    assert_eq!(
        w.service
            .revoke_token_of("alice", &a.id, EventBy::Operator, NOW + 5)
            .unwrap(),
        "alice's laptop"
    );
    assert!(auth(&w).authenticate_token("tok-a").is_err());
    assert!(auth(&w).authenticate_token("tok-b").is_ok());
    let e = w.events.recent("alice", 1).unwrap().remove(0);
    assert_eq!(
        (e.kind, e.detail.as_str()),
        (AccountEventKind::TokenRevoked, "alice's laptop")
    );
    assert!(!e.detail.contains("tok-a") && !e.detail.contains(&a.id));
    // Twice is nothing.
    assert_eq!(
        w.service
            .revoke_token_of("alice", &a.id, EventBy::Operator, NOW + 6)
            .err(),
        Some(AccountError::NoSuchToken)
    );
}
