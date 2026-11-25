//! RFC 014 T6: the account writes (create, set a password, link and unlink an identity, disable and enable): the rules, one transaction
//! each, the event in the same transaction, a refusal that changes nothing, a failure that leaves no event, and no secret anywhere.

use std::sync::Arc;

use planeter_auth::password::check_new_password;
use planeter_auth::{
    AccountError, AccountEventKind, AccountEventStore, AccountService, AccountStore,
    Argon2idHasher, Authenticator, DisplayNameError, EventBy, InMemoryCredentialStore,
    SessionStore, Sha256TokenHasher, SqliteAccountEventStore, SqliteAccountStore,
    SqliteCredentialStore, SqliteSessionStore, check_display_name,
};
use planeter_core::creation::HandleError;
use planeter_store::{SqliteDb, UserId};

const NOW: u64 = 5_000;
const SECRET_PASSWORD: &str = "canary-password-zebra-quartz-plinth";

struct World {
    db: SqliteDb,
    service: AccountService,
    accounts: SqliteAccountStore,
    events: SqliteAccountEventStore,
    sessions: SqliteSessionStore,
}

fn reserved(w: &str) -> bool {
    ["api", "new", "login", "static", "admin"].contains(&w)
}

fn world() -> World {
    let db = SqliteDb::open_in_memory().unwrap();
    let service =
        AccountService::new(db.clone(), Arc::new(Argon2idHasher), Arc::new(reserved)).unwrap();
    World {
        accounts: SqliteAccountStore::new(db.clone()).unwrap(),
        events: SqliteAccountEventStore::new(db.clone()).unwrap(),
        sessions: SqliteSessionStore::new(db.clone()).unwrap(),
        service,
        db,
    }
}

fn pw(text: &str, handle: &str) -> planeter_auth::password::NormalizedPassword {
    check_new_password(text, handle).unwrap()
}

/// Every table's rows as text, in a fixed order: what "changes nothing" compares.
fn dump(db: &SqliteDb) -> Vec<String> {
    let conn = db.lock().unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut out = Vec::new();
    for t in names {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {t} ORDER BY rowid"))
            .unwrap();
        let n = stmt.column_count();
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                (0..n)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(|v| v.join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        out.push(format!("{t}: {}", rows.join(" ;; ")));
    }
    out
}

fn kinds(w: &World, handle: &str) -> Vec<AccountEventKind> {
    w.events
        .all(handle)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

fn signs_in(w: &World, handle: &str, password: &str) -> bool {
    Authenticator::new(
        Arc::new(w.accounts.clone()),
        Arc::new(InMemoryCredentialStore::new()),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    )
    .authenticate_password(handle, password)
    .is_ok()
}

#[test]
fn create_makes_the_account_its_password_and_two_events_in_one_step() {
    let w = world();
    w.service
        .create_account(
            "alice",
            "Alice Smith",
            Some(&pw(SECRET_PASSWORD, "alice")),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    let a = w.accounts.get(&UserId::new("alice")).unwrap().unwrap();
    assert_eq!(
        (a.display_name.as_str(), a.created_at, a.password_changed_at),
        ("Alice Smith", Some(NOW), Some(NOW))
    );
    assert!(a.disabled.is_none() && a.password.is_some());
    assert!(signs_in(&w, "alice", SECRET_PASSWORD));
    assert_eq!(
        kinds(&w, "alice"),
        [AccountEventKind::Created, AccountEventKind::PasswordSet]
    );
    let e = w.events.all("alice").unwrap();
    assert!(e.iter().all(|e| e.by == EventBy::Operator && e.at == NOW));
    // No password: the account exists and cannot sign in with one; one event.
    w.service
        .create_account("bob", "Bob", None, EventBy::Operator, NOW)
        .unwrap();
    let b = w.accounts.get(&UserId::new("bob")).unwrap().unwrap();
    assert!(b.password.is_none() && b.password_changed_at.is_none());
    assert_eq!(kinds(&w, "bob"), [AccountEventKind::Created]);
    assert!(!signs_in(&w, "bob", SECRET_PASSWORD));
}

#[test]
fn every_refusal_changes_nothing_not_a_row_not_an_event() {
    let w = world();
    w.service
        .create_account("alice", "Alice", None, EventBy::Operator, NOW)
        .unwrap();
    // An organization, and the owner of a repository, that are not accounts.
    {
        let conn = w.db.lock().unwrap();
        conn.execute(
            "INSERT INTO org_roles (user, org, role) VALUES ('alice', 'AcmeCorp', 'admin')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO teams (team, org) VALUES ('t1', 'Platform')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, path) VALUES ('r1', 'user', 'Squatter', 'x', 'public', 0, '/p')",
            [],
        )
        .unwrap();
    }
    let pw12 = pw(SECRET_PASSWORD, "x");
    let before = dump(&w.db);
    let long_name = "n".repeat(101);
    let long_handle = "a".repeat(40);
    let cases: Vec<(&str, &str, AccountError)> = vec![
        ("", "Name", AccountError::Handle(HandleError::Empty)),
        (
            "Alice2",
            "Name",
            AccountError::Handle(HandleError::Uppercase),
        ),
        (
            "al ice",
            "Name",
            AccountError::Handle(HandleError::BadCharacter),
        ),
        ("-x", "Name", AccountError::Handle(HandleError::BadStart)),
        ("x.", "Name", AccountError::Handle(HandleError::Dotted)),
        ("api", "Name", AccountError::Handle(HandleError::Reserved)),
        (
            long_handle.as_str(),
            "Name",
            AccountError::Handle(HandleError::TooLong),
        ),
        // Taken: by an account (the same, and one that differs only by case: a handle that is not valid is refused on its own rule first,
        // so the case-insensitive collision is shown with the valid lowercase spelling of an existing mixed-case account below).
        ("alice", "Name", AccountError::HandleTaken),
        ("acmecorp", "Name", AccountError::HandleTaken),
        ("platform", "Name", AccountError::HandleTaken),
        ("squatter", "Name", AccountError::HandleTaken),
        (
            "fresh",
            "",
            AccountError::DisplayName(DisplayNameError::Empty),
        ),
        (
            "fresh",
            long_name.as_str(),
            AccountError::DisplayName(DisplayNameError::TooLong),
        ),
        (
            "fresh",
            "bad\u{202E}name",
            AccountError::DisplayName(DisplayNameError::Unsafe),
        ),
        (
            "fresh",
            "two\nlines",
            AccountError::DisplayName(DisplayNameError::Unsafe),
        ),
    ];
    for (handle, name, want) in cases {
        let got = w
            .service
            .create_account(handle, name, Some(&pw12), EventBy::Operator, NOW);
        assert_eq!(got, Err(want), "{handle:?} / {name:?}");
        assert_eq!(dump(&w.db), before, "{handle:?}: the database is as it was");
    }
    assert!(
        check_display_name("  Alice  ").unwrap() == "Alice",
        "a name is stored trimmed"
    );
}

#[test]
fn a_handle_that_differs_only_by_case_from_an_existing_mixed_case_account_is_taken() {
    let w = world();
    // An account made before the rule (mixed case), through the legacy writer.
    w.accounts
        .upsert(planeter_auth::Account::local("Carol", "Carol", None))
        .unwrap();
    assert_eq!(
        w.service
            .create_account("carol", "Another", None, EventBy::Operator, NOW),
        Err(AccountError::HandleTaken)
    );
    assert!(w.accounts.get(&UserId::new("carol")).unwrap().is_none());
}

#[test]
fn a_write_that_fails_half_way_leaves_no_row_and_no_event() {
    let w = world();
    // The `password_set` event cannot be written: the account row and the `created` event written before it are rolled back with it.
    w.db.lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_password_event BEFORE INSERT ON account_events WHEN NEW.kind = 'password_set' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    let before = dump(&w.db);
    let got = w.service.create_account(
        "alice",
        "Alice",
        Some(&pw(SECRET_PASSWORD, "alice")),
        EventBy::Operator,
        NOW,
    );
    assert!(matches!(got, Err(AccountError::Backend(_))), "{got:?}");
    assert_eq!(dump(&w.db), before, "no account, no `created` event");
    assert!(w.accounts.get(&UserId::new("alice")).unwrap().is_none());
    // The same for a password change: the old password still works, the sessions are still there, there is no event.
    w.db.lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_password_event;")
        .unwrap();
    w.service
        .create_account(
            "alice",
            "Alice",
            Some(&pw(SECRET_PASSWORD, "alice")),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    let session = w.sessions.create(UserId::new("alice"), 10, 99_999).unwrap();
    w.db.lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_password_event BEFORE INSERT ON account_events WHEN NEW.kind = 'password_set' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    let before = dump(&w.db);
    let got = w.service.set_password(
        "alice",
        &pw("another-long-password-1", "alice"),
        EventBy::Operator,
        NOW + 1,
    );
    assert!(matches!(got, Err(AccountError::Backend(_))));
    assert_eq!(dump(&w.db), before);
    assert!(
        signs_in(&w, "alice", SECRET_PASSWORD),
        "the old password still works"
    );
    assert!(
        w.sessions.get(&session.id, 20).unwrap().is_some(),
        "and the session was not ended"
    );
}

#[test]
fn set_password_ends_that_accounts_sessions_and_writes_the_event_and_the_time() {
    let w = world();
    w.service
        .create_account(
            "alice",
            "Alice",
            Some(&pw(SECRET_PASSWORD, "alice")),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    w.service
        .create_account("bob", "Bob", None, EventBy::Operator, NOW)
        .unwrap();
    let a1 = w.sessions.create(UserId::new("alice"), 10, 99_999).unwrap();
    let a2 = w.sessions.create(UserId::new("alice"), 11, 99_999).unwrap();
    let b1 = w.sessions.create(UserId::new("bob"), 12, 99_999).unwrap();
    w.service
        .set_password(
            "alice",
            &pw("a-brand-new-password-9", "alice"),
            EventBy::Operator,
            NOW + 10,
        )
        .unwrap();
    assert!(
        w.sessions.get(&a1.id, 20).unwrap().is_none()
            && w.sessions.get(&a2.id, 20).unwrap().is_none()
    );
    assert!(
        w.sessions.get(&b1.id, 20).unwrap().is_some(),
        "bob's session stays"
    );
    assert!(!signs_in(&w, "alice", SECRET_PASSWORD));
    assert!(signs_in(&w, "alice", "a-brand-new-password-9"));
    let a = w.accounts.get(&UserId::new("alice")).unwrap().unwrap();
    assert_eq!(a.password_changed_at, Some(NOW + 10));
    let last = w.events.recent("alice", 1).unwrap().remove(0);
    assert_eq!(
        (last.kind, last.by, last.detail.as_str()),
        (
            AccountEventKind::PasswordSet,
            EventBy::Operator,
            "2 session(s) ended"
        )
    );
    // Set by the holder: the other word.
    w.service
        .set_password(
            "alice",
            &pw("yet-another-password-77", "alice"),
            EventBy::SelfService,
            NOW + 20,
        )
        .unwrap();
    assert_eq!(
        w.events.recent("alice", 1).unwrap()[0].kind,
        AccountEventKind::PasswordChanged
    );
    // An account that does not exist: refused, nothing written.
    let before = dump(&w.db);
    assert_eq!(
        w.service.set_password(
            "ghost",
            &pw(SECRET_PASSWORD, "ghost"),
            EventBy::Operator,
            NOW
        ),
        Err(AccountError::NoSuchAccount)
    );
    assert_eq!(dump(&w.db), before);
}

#[test]
fn link_and_unlink_write_events_that_name_the_host_and_never_the_subject() {
    let w = world();
    for (h, n) in [("alice", "Alice"), ("bob", "Bob")] {
        w.service
            .create_account(h, n, None, EventBy::Operator, NOW)
            .unwrap();
    }
    let issuer = "https://user:pass@idp.example:8443/realms/main?x=1#f";
    let subject = "SECRET-SUBJECT-123";
    assert!(
        w.service
            .link_oidc("alice", issuer, subject, EventBy::Operator, NOW)
            .unwrap()
    );
    assert_eq!(
        w.accounts.lookup_oidc(issuer, subject).unwrap(),
        Some(UserId::new("alice"))
    );
    let e = w.events.recent("alice", 1).unwrap().remove(0);
    assert_eq!(
        (e.kind, e.detail.as_str()),
        (AccountEventKind::IdentityLinked, "idp.example:8443")
    );
    // Again: nothing changes, no second event. By another account: refused, nothing changes.
    let before = dump(&w.db);
    assert!(
        !w.service
            .link_oidc("alice", issuer, subject, EventBy::Operator, NOW)
            .unwrap()
    );
    assert_eq!(
        w.service
            .link_oidc("bob", issuer, subject, EventBy::Operator, NOW),
        Err(AccountError::IdentityTaken)
    );
    assert_eq!(
        w.service
            .link_oidc("ghost", issuer, "other", EventBy::Operator, NOW),
        Err(AccountError::NoSuchAccount)
    );
    assert_eq!(dump(&w.db), before);
    // Unlink: only from the account it is linked to.
    assert_eq!(
        w.service
            .unlink_oidc("bob", issuer, subject, EventBy::Operator, NOW),
        Err(AccountError::NotLinked)
    );
    assert_eq!(dump(&w.db), before);
    w.service
        .unlink_oidc("alice", issuer, subject, EventBy::Operator, NOW + 1)
        .unwrap();
    assert_eq!(w.accounts.lookup_oidc(issuer, subject).unwrap(), None);
    assert_eq!(
        kinds(&w, "alice"),
        [
            AccountEventKind::Created,
            AccountEventKind::IdentityLinked,
            AccountEventKind::IdentityUnlinked
        ]
    );
    // The subject is nowhere in the events (nor the userinfo of the issuer URL, nor its path).
    for e in w.events.all("alice").unwrap() {
        assert!(
            !e.detail.contains("SECRET")
                && !e.detail.contains("pass")
                && !e.detail.contains("realms"),
            "{e:?}"
        );
    }
}

#[test]
fn disable_and_enable_write_their_events_end_sessions_and_do_nothing_twice() {
    let w = world();
    w.service
        .create_account("alice", "Alice", None, EventBy::Operator, NOW)
        .unwrap();
    let s = w.sessions.create(UserId::new("alice"), 10, 99_999).unwrap();
    assert!(
        w.service
            .disable(
                "alice",
                "left the project\u{202E}",
                EventBy::Operator,
                NOW + 1
            )
            .unwrap()
    );
    assert!(w.sessions.get(&s.id, 20).unwrap().is_none());
    let e = w.events.recent("alice", 1).unwrap().remove(0);
    assert_eq!(
        (e.kind, e.detail.as_str()),
        (AccountEventKind::Disabled, "left the project\\u{202E}")
    );
    let before = dump(&w.db);
    assert!(
        !w.service
            .disable("alice", "again", EventBy::Operator, NOW + 2)
            .unwrap(),
        "already disabled"
    );
    assert_eq!(dump(&w.db), before);
    assert!(
        w.service
            .enable("alice", EventBy::Operator, NOW + 3)
            .unwrap()
    );
    assert!(
        !w.service
            .enable("alice", EventBy::Operator, NOW + 4)
            .unwrap()
    );
    assert_eq!(
        kinds(&w, "alice"),
        [
            AccountEventKind::Created,
            AccountEventKind::Disabled,
            AccountEventKind::Enabled
        ]
    );
    assert_eq!(
        w.service.disable("ghost", "x", EventBy::Operator, NOW),
        Err(AccountError::NoSuchAccount)
    );
}

#[test]
fn no_secret_is_anywhere_in_the_database_but_as_an_argon2id_hash() {
    let w = world();
    w.service
        .create_account(
            "alice",
            "Alice",
            Some(&pw(SECRET_PASSWORD, "alice")),
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    w.service
        .set_password(
            "alice",
            &pw("second-canary-password-xylophone", "alice"),
            EventBy::SelfService,
            NOW + 1,
        )
        .unwrap();
    w.service
        .link_oidc(
            "alice",
            "https://idp.example",
            "canary-subject-marmot",
            EventBy::Provider,
            NOW + 2,
        )
        .unwrap();
    let _ = SqliteCredentialStore::new(w.db.clone()).unwrap();
    let everything = dump(&w.db).join("\n");
    for canary in [SECRET_PASSWORD, "second-canary-password-xylophone"] {
        assert!(
            !everything.contains(canary),
            "the password is stored only as a hash"
        );
    }
    assert!(everything.contains("$argon2id$"));
    // The subject lives in the link table (it must, to find the account) and in no event.
    let events_only: String = w
        .events
        .all("alice")
        .unwrap()
        .iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(!events_only.contains("canary-subject-marmot"));
    assert!(!events_only.contains("argon2"));
}
