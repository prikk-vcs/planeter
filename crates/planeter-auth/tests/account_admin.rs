//! RFC 014 operator handoff: the service's role grants, the last-administrator rule, the sole-administrator listing and the reads the
//! `account list` and `account show` commands print (never a hash, a token or a subject).

use std::sync::Arc;

use planeter_auth::{
    AccountError, AccountEventKind, AccountEventStore, AccountService, Argon2idHasher, EventBy,
    RepoRole, SqliteAccountEventStore, SqliteAccountStore,
};
use planeter_store::SqliteDb;

const NOW: u64 = 7_000;

struct World {
    db: SqliteDb,
    service: AccountService,
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
    for (h, n) in [("alice", "Alice A"), ("bob", "Bob B"), ("carol", "Carol C")] {
        service
            .create_account(h, n, None, EventBy::Operator, NOW)
            .unwrap();
    }
    let events = SqliteAccountEventStore::new(db.clone()).unwrap();
    World {
        db,
        service,
        events,
    }
}

fn repo(w: &World, id: &str, kind: &str, owner: &str, name: &str) {
    w.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, path) VALUES (?1, ?2, ?3, ?4, 'public', 0, '/p')",
            rusqlite::params![id, kind, owner, name],
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
            format!("{t}: {}", rows.join(";;"))
        })
        .collect()
}

#[test]
fn a_grant_sets_exactly_the_role_writes_the_event_and_does_nothing_twice() {
    let w = world();
    repo(&w, "r1", "user", "alice", "app");
    let g = w
        .service
        .grant_role(
            "bob",
            "alice",
            "app",
            RepoRole::Write,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    assert_eq!((g.display_name.as_str(), g.previous), ("Bob B", None));
    let g = w
        .service
        .grant_role(
            "bob",
            "alice",
            "app",
            RepoRole::Admin,
            EventBy::Operator,
            NOW + 1,
        )
        .unwrap();
    assert_eq!(g.previous, Some(RepoRole::Write));
    // Exactly the role asked: a lower one lowers.
    let g = w
        .service
        .grant_role(
            "bob",
            "alice",
            "app",
            RepoRole::Read,
            EventBy::Operator,
            NOW + 2,
        )
        .unwrap();
    assert_eq!(g.previous, Some(RepoRole::Admin));
    let before = dump(&w.db);
    let g = w
        .service
        .grant_role(
            "bob",
            "alice",
            "app",
            RepoRole::Read,
            EventBy::Operator,
            NOW + 3,
        )
        .unwrap();
    assert_eq!(g.previous, Some(RepoRole::Read), "already so");
    assert_eq!(dump(&w.db), before, "nothing changed and no event");
    let ev = w.events.all("bob").unwrap();
    let granted: Vec<_> = ev
        .iter()
        .filter(|e| e.kind == AccountEventKind::RoleGranted)
        .map(|e| e.detail.as_str())
        .collect();
    assert_eq!(
        granted,
        ["alice/app write", "alice/app admin", "alice/app read"]
    );
    assert!(
        ev.iter()
            .all(|e| e.kind == AccountEventKind::Created || e.by == EventBy::Operator)
    );
}

#[test]
fn a_grant_that_is_refused_changes_nothing() {
    let w = world();
    repo(&w, "r1", "user", "alice", "app");
    w.service
        .disable("carol", "left", EventBy::Operator, NOW)
        .unwrap();
    let before = dump(&w.db);
    for (handle, owner, name, want) in [
        ("bob", "alice", "ghost", AccountError::NoSuchRepository),
        ("bob", "nobody", "app", AccountError::NoSuchRepository),
        ("ghost", "alice", "app", AccountError::NoSuchAccount),
        ("carol", "alice", "app", AccountError::AccountDisabled),
    ] {
        assert_eq!(
            w.service
                .grant_role(handle, owner, name, RepoRole::Write, EventBy::Operator, NOW)
                .err(),
            Some(want),
            "{handle} {owner}/{name}"
        );
        assert_eq!(dump(&w.db), before);
    }
}

#[test]
fn the_last_administrator_cannot_be_revoked_without_saying_so() {
    let w = world();
    // An organization's repository: bob is its only administrator (a grant of his own); carol is an ordinary member.
    repo(&w, "r1", "org", "acme", "tool");
    w.service
        .grant_role(
            "bob",
            "acme",
            "tool",
            RepoRole::Admin,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    w.service
        .grant_role(
            "carol",
            "acme",
            "tool",
            RepoRole::Write,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    let before = dump(&w.db);
    assert_eq!(
        w.service
            .revoke_role("bob", "acme", "tool", false, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::LastAdministrator)
    );
    assert_eq!(dump(&w.db), before, "refused: nothing changed");
    // A non-administrator role is revoked freely, and one that is not held is `NoGrant`.
    assert_eq!(
        w.service
            .revoke_role("carol", "acme", "tool", false, EventBy::Operator, NOW)
            .unwrap()
            .was,
        RepoRole::Write
    );
    assert_eq!(
        w.service
            .revoke_role("carol", "acme", "tool", false, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::NoGrant)
    );
    // Another administrator by any source lets him go: an organization administrator.
    w.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO org_roles (user, org, role) VALUES ('alice', 'acme', 'admin')",
            [],
        )
        .unwrap();
    let r = w
        .service
        .revoke_role("bob", "acme", "tool", false, EventBy::Operator, NOW + 5)
        .unwrap();
    assert_eq!((r.display_name.as_str(), r.was), ("Bob B", RepoRole::Admin));
    let last = w.events.recent("bob", 1).unwrap().remove(0);
    assert_eq!(
        (last.kind, last.detail.as_str()),
        (AccountEventKind::RoleRevoked, "acme/tool admin")
    );
    // With `allow_no_admin` the last one goes.
    w.db.lock()
        .unwrap()
        .execute("DELETE FROM org_roles", [])
        .unwrap();
    w.service
        .grant_role(
            "bob",
            "acme",
            "tool",
            RepoRole::Admin,
            EventBy::Operator,
            NOW + 6,
        )
        .unwrap();
    assert!(
        w.service
            .revoke_role("bob", "acme", "tool", true, EventBy::Operator, NOW + 7)
            .is_ok()
    );
}

#[test]
fn a_disabled_administrator_does_not_count_and_an_owner_always_does() {
    let w = world();
    repo(&w, "r1", "org", "acme", "tool");
    w.service
        .grant_role(
            "bob",
            "acme",
            "tool",
            RepoRole::Admin,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    w.service
        .grant_role(
            "carol",
            "acme",
            "tool",
            RepoRole::Admin,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    w.service
        .disable("carol", "away", EventBy::Operator, NOW)
        .unwrap();
    // carol is disabled: bob is the only one who can act.
    assert_eq!(
        w.service
            .revoke_role("bob", "acme", "tool", false, EventBy::Operator, NOW)
            .err(),
        Some(AccountError::LastAdministrator)
    );
    // A user's own repository: its owner is an administrator by ownership, so an admin grant of his own can go.
    repo(&w, "r2", "user", "alice", "mine");
    w.service
        .grant_role(
            "alice",
            "alice",
            "mine",
            RepoRole::Admin,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    assert!(
        w.service
            .revoke_role("alice", "alice", "mine", false, EventBy::Operator, NOW)
            .is_ok()
    );
}

#[test]
fn the_sole_administrator_listing_names_the_repositories_bounded() {
    let w = world();
    for i in 0..25 {
        repo(&w, &format!("o{i:02}"), "org", "acme", &format!("r{i:02}"));
        w.service
            .grant_role(
                "bob",
                "acme",
                &format!("r{i:02}"),
                RepoRole::Admin,
                EventBy::Operator,
                NOW,
            )
            .unwrap();
    }
    repo(&w, "shared", "org", "acme", "shared");
    w.service
        .grant_role(
            "bob",
            "acme",
            "shared",
            RepoRole::Admin,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    w.service
        .grant_role(
            "carol",
            "acme",
            "shared",
            RepoRole::Admin,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    repo(&w, "p1", "user", "bob", "own");
    let s = w.service.sole_administrator_of("bob", 20).unwrap();
    assert_eq!(
        s.total, 26,
        "25 organization repositories and bob's own; `shared` has two administrators"
    );
    assert_eq!(s.repos.len(), 20);
    assert_eq!(s.repos[0], "acme/r00");
    assert!(!s.repos.iter().any(|r| r == "acme/shared"));
    let c = w.service.sole_administrator_of("carol", 20).unwrap();
    assert_eq!(
        (c.total, c.repos.len()),
        (0, 0),
        "carol shares `shared` with bob"
    );
    assert_eq!(
        w.service.sole_administrator_of("ghost", 20).unwrap().total,
        0
    );
    // Once carol is disabled, bob is the only administrator of `shared` too.
    w.service
        .disable("carol", "away", EventBy::Operator, NOW)
        .unwrap();
    assert_eq!(
        w.service.sole_administrator_of("bob", 100).unwrap().total,
        27
    );
}

#[test]
fn list_show_and_the_collisions_are_bounded_sorted_and_carry_no_secret() {
    let w = world();
    w.db.lock()
        .unwrap()
        .execute_batch(
            "INSERT INTO accounts (user, display_name, handle_key, handle_collides, password_hash) VALUES ('Zed', 'Zed 1', 'zed', 1, '$argon2id$secret'), ('zed', 'Zed 2', 'zed', 1, NULL);",
        )
        .unwrap();
    w.service
        .link_oidc(
            "alice",
            "https://idp.example",
            "SECRET-SUBJECT",
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    repo(&w, "r1", "user", "alice", "app");
    repo(&w, "r2", "org", "acme", "tool");
    w.service
        .grant_role(
            "alice",
            "acme",
            "tool",
            RepoRole::Write,
            EventBy::Operator,
            NOW,
        )
        .unwrap();
    let list = w.service.list_accounts(3).unwrap();
    assert_eq!(list.total, 5);
    assert_eq!(
        list.rows
            .iter()
            .map(|r| r.handle.as_str())
            .collect::<Vec<_>>(),
        ["Zed", "alice", "bob"],
        "sorted by handle (bytewise), bounded"
    );
    assert_eq!(
        w.service.collisions().unwrap(),
        [("zed".to_owned(), vec!["Zed".to_owned(), "zed".to_owned()])]
    );
    let d = w.service.show_account("alice", 10).unwrap().unwrap();
    assert_eq!(
        (d.row.identities, d.row.has_password, d.row.disabled),
        (1, false, false)
    );
    assert_eq!(
        d.roles,
        [
            ("acme/tool".to_owned(), "write".to_owned()),
            ("alice/app".to_owned(), "owner".to_owned())
        ]
    );
    assert_eq!(d.roles_total, 2);
    assert_eq!(
        w.service
            .show_account("alice", 1)
            .unwrap()
            .unwrap()
            .roles
            .len(),
        1
    );
    assert!(w.service.show_account("ghost", 10).unwrap().is_none());
    let shown = format!("{d:?}{list:?}");
    assert!(
        !shown.contains("SECRET") && !shown.contains("argon2"),
        "{shown}"
    );
}
