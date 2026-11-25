//! RFC 014 T3: the schema, idempotent, nothing existing touched. A database as 0.9.0 left it (its exact tables, filled) is opened by this
//! code: every account gets a case-insensitive key, every token a public id and a placeholder name, the events table appears, a second
//! start changes nothing, and two accounts whose handles differ only by case do not stop the start and are not renamed.

#![allow(clippy::type_complexity)]
use std::path::{Path, PathBuf};
use std::sync::Arc;

use planeter_auth::{
    AccountEvent, AccountEventKind, AccountEventStore, AccountStore, Argon2idHasher, Authenticator,
    CredentialStore, EventBy, LEGACY_TOKEN_NAME, Sha256TokenHasher, SqliteAccountEventStore,
    SqliteAccountStore, SqliteCredentialStore, SqliteSessionStore, TokenHasher,
};
use planeter_store::{SqliteDb, UserId};
use rusqlite::Connection;

/// The auth tables exactly as 0.9.0 created them (no new column, no events table).
const SCHEMA_0_9_0: &str = "
CREATE TABLE accounts (user TEXT PRIMARY KEY, display_name TEXT NOT NULL, email TEXT, password_hash TEXT);
CREATE TABLE tokens (hash TEXT PRIMARY KEY, user TEXT NOT NULL, access TEXT NOT NULL, repos TEXT, expires_at INTEGER);
CREATE TABLE ssh_keys (fingerprint TEXT PRIMARY KEY, user TEXT NOT NULL, access TEXT NOT NULL, repos TEXT, expires_at INTEGER);
CREATE TABLE oidc_links (issuer TEXT NOT NULL, subject TEXT NOT NULL, user TEXT NOT NULL, PRIMARY KEY (issuer, subject));
CREATE TABLE sessions (id TEXT PRIMARY KEY, user TEXT NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
CREATE TABLE deploy_keys (fingerprint TEXT PRIMARY KEY, registrant TEXT NOT NULL, repo_id TEXT NOT NULL, write INTEGER NOT NULL);
";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("schema-migration-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.sqlite")
}

fn token_hash(secret: &str) -> String {
    Sha256TokenHasher.hash(secret).0
}

/// A 0.9.0 database with the given handles, three tokens, a session, an OIDC link and an SSH key.
fn database_of_0_9_0(path: &Path, handles: &[&str]) {
    let c = Connection::open(path).unwrap();
    c.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
    c.execute_batch(SCHEMA_0_9_0).unwrap();
    let hasher = Argon2idHasher;
    for h in handles {
        c.execute(
            "INSERT INTO accounts (user, display_name, email, password_hash) VALUES (?1, ?2, NULL, ?3)",
            rusqlite::params![h, format!("{h} name"), planeter_auth::PasswordHasher::hash(&hasher, "correct horse battery staple").0],
        )
        .unwrap();
    }
    for (secret, user, access, repos, exp) in [
        ("tok-one", "alice", "full", None, None::<i64>),
        (
            "tok-two",
            "alice",
            "read-only",
            Some("r1\nr2"),
            Some(4_000_000_000),
        ),
        ("tok-three", "bob", "write", None, Some(1)),
    ] {
        c.execute(
            "INSERT INTO tokens (hash, user, access, repos, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![token_hash(secret), user, access, repos, exp],
        )
        .unwrap();
    }
    c.execute(
        "INSERT INTO sessions VALUES ('sess-1', 'alice', 10, 4000000000)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO oidc_links VALUES ('https://idp.example', 'sub-1', 'alice')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO ssh_keys VALUES ('SHA256:x', 'alice', 'full', NULL, NULL)",
        [],
    )
    .unwrap();
}

fn dump(path: &Path, table: &str, order: &str) -> Vec<Vec<String>> {
    let c = Connection::open(path).unwrap();
    let mut stmt = c
        .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
        .unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |r| {
        (0..n)
            .map(|i| {
                r.get::<_, rusqlite::types::Value>(i)
                    .map(|v| format!("{v:?}"))
            })
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn tables(path: &Path) -> Vec<String> {
    let c = Connection::open(path).unwrap();
    let mut stmt = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn open_all(path: &Path) -> (SqliteAccountStore, SqliteCredentialStore) {
    let db = SqliteDb::open(path).unwrap();
    (
        SqliteAccountStore::new(db.clone()).unwrap(),
        SqliteCredentialStore::new(db).unwrap(),
    )
}

#[test]
fn a_0_9_0_database_is_migrated_once_and_nothing_existing_changes() {
    let path = scratch("upgrade");
    database_of_0_9_0(&path, &["alice", "bob", "carol"]);
    assert!(!tables(&path).contains(&"account_events".to_owned()));
    let before_accounts = dump(&path, "accounts", "user");
    let before_tokens = dump(&path, "tokens", "hash");
    let before_sessions = dump(&path, "sessions", "id");
    let before_links = dump(&path, "oidc_links", "subject");

    let (accounts, creds) = open_all(&path);
    let report = accounts.migration();
    assert_eq!(report.handles_keyed, 3);
    assert_eq!(report.tokens_identified, 3);
    assert_eq!(report.colliding_accounts, 0);
    assert!(tables(&path).contains(&"account_events".to_owned()));

    // Accounts: the original four columns are exactly as they were; each has its case-insensitive key; the state columns are empty.
    let c = Connection::open(&path).unwrap();
    let rows: Vec<(String, String, String, Option<i64>, Option<i64>, Option<String>, Option<i64>, i64)> = c
        .prepare("SELECT user, display_name, handle_key, created_at, disabled_at, disabled_reason, password_changed_at, handle_collides FROM accounts ORDER BY user")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 3);
    for (user, name, key, created, disabled, reason, changed, collides) in &rows {
        assert_eq!(name, &format!("{user} name"));
        assert_eq!(key, user);
        assert!(created.is_none() && disabled.is_none() && reason.is_none() && changed.is_none());
        assert_eq!(*collides, 0);
    }
    let after_accounts_old_columns: Vec<Vec<String>> = {
        let mut stmt = c
            .prepare("SELECT user, display_name, email, password_hash FROM accounts ORDER BY user")
            .unwrap();
        stmt.query_map([], |r| {
            (0..4)
                .map(|i| {
                    r.get::<_, rusqlite::types::Value>(i)
                        .map(|v| format!("{v:?}"))
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    assert_eq!(
        after_accounts_old_columns,
        before_accounts
            .iter()
            .map(|r| r[..4].to_vec())
            .collect::<Vec<_>>(),
        "the four columns an account had are byte for byte what they were"
    );

    // Tokens: each has a unique 16-character lowercase hex id and the placeholder name; hash, user, access, repos and expiry are untouched.
    let token_rows: Vec<(String, String, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>)> = c
        .prepare("SELECT hash, user, access, id, name, expires_at, created_at, last_used_at FROM tokens ORDER BY hash")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut ids = std::collections::BTreeSet::new();
    for (_, _, _, id, name, _, created, used) in &token_rows {
        assert_eq!(id.len(), 16, "{id}");
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{id}"
        );
        assert!(ids.insert(id.clone()), "ids are unique");
        assert_eq!(name.as_deref(), Some(LEGACY_TOKEN_NAME));
        assert!(created.is_none() && used.is_none());
    }
    let after_tokens_old: Vec<Vec<String>> = {
        let mut stmt = c
            .prepare("SELECT hash, user, access, repos, expires_at FROM tokens ORDER BY hash")
            .unwrap();
        stmt.query_map([], |r| {
            (0..5)
                .map(|i| {
                    r.get::<_, rusqlite::types::Value>(i)
                        .map(|v| format!("{v:?}"))
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    assert_eq!(
        after_tokens_old,
        before_tokens
            .iter()
            .map(|r| r[..5].to_vec())
            .collect::<Vec<_>>()
    );
    // Sessions and links are byte for byte what they were.
    assert_eq!(dump(&path, "sessions", "id"), before_sessions);
    assert_eq!(dump(&path, "oidc_links", "subject"), before_links);

    // Existing tokens still authenticate (the live one) and an expired one is still refused; existing passwords still verify.
    let auth = Authenticator::new(
        Arc::new(accounts.clone()),
        Arc::new(creds.clone()),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    );
    assert!(auth.authenticate_token("tok-one").is_ok());
    assert!(auth.authenticate_token("tok-two").is_ok());
    assert!(
        auth.authenticate_token("tok-three").is_err(),
        "expired at 1"
    );
    assert!(
        auth.authenticate_password("alice", "correct horse battery staple")
            .is_ok()
    );
    assert!(
        creds
            .find_token(&Sha256TokenHasher.hash("tok-one"))
            .unwrap()
            .is_some()
    );

    // A second start does nothing: no work counted, every table identical.
    let all = [
        "accounts",
        "tokens",
        "sessions",
        "oidc_links",
        "ssh_keys",
        "account_events",
    ];
    let snapshot = |p: &Path| all.map(|t| dump(p, t, "rowid"));
    let first = snapshot(&path);
    let (accounts2, _) = open_all(&path);
    let r2 = accounts2.migration();
    assert_eq!(
        (
            r2.handles_keyed,
            r2.tokens_identified,
            r2.colliding_accounts
        ),
        (0, 0, 0)
    );
    assert_eq!(snapshot(&path), first, "a second start changes nothing");
    let _ = SqliteSessionStore::new(SqliteDb::open(&path).unwrap()).unwrap();
    assert_eq!(
        snapshot(&path),
        first,
        "and neither does the session store's"
    );
}

#[test]
fn two_accounts_that_differ_only_by_case_do_not_stop_the_start_and_are_not_renamed() {
    let path = scratch("collision");
    database_of_0_9_0(&path, &["Alice", "alice", "ALICE", "bob"]);
    let (accounts, _) = open_all(&path);
    let report = accounts.migration();
    assert_eq!(
        report.colliding_accounts, 3,
        "the three that share a key, and only those"
    );
    let c = Connection::open(&path).unwrap();
    let handles: Vec<String> = c
        .prepare("SELECT user FROM accounts ORDER BY user")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        handles,
        ["ALICE", "Alice", "alice", "bob"],
        "nobody was renamed"
    );
    // The index exists and leaves the colliding accounts out: its SQL says so.
    let sql: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'accounts_handle_key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        sql.contains("UNIQUE") && sql.contains("WHERE handle_collides = 0"),
        "{sql}"
    );
    // ... and it holds the rule for everybody else: a second non-colliding row with the same key is refused by the database.
    let dup = c.execute(
        "INSERT INTO accounts (user, display_name, handle_key, handle_collides) VALUES ('BOB', 'x', 'bob', 0)",
        [],
    );
    assert!(
        dup.is_err(),
        "the unique index holds for accounts that do not collide"
    );
    // Everyone still signs in, including each of the three.
    let auth = Authenticator::new(
        Arc::new(accounts.clone()),
        Arc::new(SqliteCredentialStore::new(SqliteDb::open(&path).unwrap()).unwrap()),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    );
    for h in ["Alice", "alice", "ALICE", "bob"] {
        assert!(
            auth.authenticate_password(h, "correct horse battery staple")
                .is_ok(),
            "{h}"
        );
    }
    // A second start finds the same collision, changes nothing, and still does not fail.
    let before = dump(&path, "accounts", "user");
    let (again, _) = open_all(&path);
    assert_eq!(again.migration().colliding_accounts, 3);
    assert_eq!(dump(&path, "accounts", "user"), before);
    // The legacy write path still works for a colliding handle (nothing here refuses an existing account's update).
    accounts
        .upsert(planeter_auth::Account::local(
            "Alice",
            "Alice renamed",
            None,
        ))
        .unwrap();
    assert_eq!(
        accounts
            .get(&UserId::new("Alice"))
            .unwrap()
            .unwrap()
            .display_name,
        "Alice renamed"
    );
}

#[test]
fn the_events_table_takes_only_its_closed_set_and_keeps_text_display_safe() {
    let db = SqliteDb::open_in_memory().unwrap();
    let store = SqliteAccountEventStore::new(db.clone()).unwrap();
    // The closed set of kinds and of actors: the database refuses any other word.
    {
        let conn = db.lock().unwrap();
        assert!(conn.execute("INSERT INTO account_events (account, kind, \"by\", detail, at) VALUES ('a', 'deleted', 'self', '', 1)", []).is_err());
        assert!(conn.execute("INSERT INTO account_events (account, kind, \"by\", detail, at) VALUES ('a', 'created', 'admin', '', 1)", []).is_err());
    }
    for (i, kind) in AccountEventKind::ALL.into_iter().enumerate() {
        assert_eq!(AccountEventKind::parse(kind.as_str()), Some(kind));
        store
            .record(&AccountEvent::new(
                "alice",
                kind,
                EventBy::Operator,
                "x",
                i as u64,
            ))
            .unwrap();
    }
    assert_eq!(AccountEventKind::ALL.len(), 13);
    assert_eq!(store.all("alice").unwrap().len(), 13);
    // A disguise character is shown as an escape; an overlong text is cut, never refused.
    let long = "あ".repeat(500);
    store
        .record(&AccountEvent::new(
            "bob",
            AccountEventKind::Disabled,
            EventBy::Operator,
            &long,
            1,
        ))
        .unwrap();
    store
        .record(&AccountEvent::new(
            "bob",
            AccountEventKind::Disabled,
            EventBy::Operator,
            "x\u{202E}y\nz",
            2,
        ))
        .unwrap();
    // A caller that builds the struct by hand cannot write a disguise or a long text either.
    store
        .record(&AccountEvent {
            account: "bob".into(),
            kind: AccountEventKind::Enabled,
            by: EventBy::Provider,
            detail: format!("raw\u{202E}{}", "b".repeat(500)),
            at: 3,
        })
        .unwrap();
    let all = store.all("bob").unwrap();
    assert_eq!(all[0].detail.chars().count(), 200);
    assert_eq!(all[1].detail, "x\\u{202E}y\\u{A}z");
    assert!(all[2].detail.chars().count() <= 200 && all[2].detail.starts_with("raw\\u{202E}"));
    for e in &all {
        assert!(!e.detail.contains('\u{202E}') && !e.detail.contains('\n'));
    }
}

#[test]
fn recent_is_newest_first_and_never_more_than_200() {
    let store = SqliteAccountEventStore::new(SqliteDb::open_in_memory().unwrap()).unwrap();
    for i in 0..250u64 {
        store
            .record(&AccountEvent::new(
                "a",
                AccountEventKind::SessionEnded,
                EventBy::SelfService,
                &format!("n{i}"),
                i,
            ))
            .unwrap();
    }
    store
        .record(&AccountEvent::new(
            "other",
            AccountEventKind::Created,
            EventBy::Operator,
            "",
            1,
        ))
        .unwrap();
    let five = store.recent("a", 5).unwrap();
    assert_eq!(
        five.iter().map(|e| e.detail.as_str()).collect::<Vec<_>>(),
        ["n249", "n248", "n247", "n246", "n245"]
    );
    assert_eq!(store.recent("a", 10_000).unwrap().len(), 200, "capped");
    assert_eq!(store.recent("a", 0).unwrap().len(), 0);
    assert_eq!(store.all("a").unwrap().len(), 250);
    assert_eq!(store.all("other").unwrap().len(), 1);
    assert_eq!(store.recent("nobody", 10).unwrap().len(), 0);
}

#[test]
fn no_code_deletes_or_rewrites_an_event() {
    // The store has no delete and no update, and nothing in the crate's source says DELETE or UPDATE of the table.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut seen = 0;
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let l = line.to_ascii_lowercase();
                if l.trim_start().starts_with("//") {
                    continue;
                }
                assert!(
                    !(l.contains("delete from account_events")
                        || l.contains("update account_events")
                        || l.contains("drop table account_events")),
                    "{}:{}: {line}",
                    path.display(),
                    n + 1
                );
            }
            seen += 1;
        }
    }
    assert!(seen >= 8, "the scan read the crate's sources");
}
