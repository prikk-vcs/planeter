//! RFC 014 D-7: **an event's detail is data, never prose.** Every kind has one typed form with a constructor and a parser; what is stored
//! is codes, counts, a host, a date and the two texts a person typed; a detail that does not parse is `None` (never the raw text); the
//! longest legal event fits the column's cut.

use planeter_auth::{
    AccountEvent, AccountEventKind, AccountEventStore, EventBy, EventDetail,
    InMemoryAccountEventStore, RepoRole, SqliteAccountEventStore,
};
use planeter_core::ScopeAccess;
use planeter_store::SqliteDb;

const AT: u64 = 1_795_564_800;

fn typed(kind: AccountEventKind, d: &EventDetail) -> AccountEvent {
    AccountEvent::typed("alice", kind, EventBy::Operator, d, AT)
}

/// Every kind with an example of its typed form: the table of this module's documentation, as data.
fn table() -> Vec<(AccountEventKind, EventDetail, &'static str)> {
    use AccountEventKind as K;
    vec![
        (K::Created, EventDetail::None, ""),
        (K::Enabled, EventDetail::None, ""),
        (K::SignedInNewMethod, EventDetail::None, ""),
        (K::PasswordSet, EventDetail::SignInsEnded(0), "0"),
        (K::PasswordChanged, EventDetail::SignInsEnded(2), "2"),
        (K::SessionEnded, EventDetail::SignInsEnded(1), "1"),
        (
            K::IdentityLinked,
            EventDetail::Host("idp.example:8443".into()),
            "idp.example:8443",
        ),
        (
            K::IdentityUnlinked,
            EventDetail::Host("idp.example".into()),
            "idp.example",
        ),
        (
            K::Disabled,
            EventDetail::Reason("left the project".into()),
            "left the project",
        ),
        (
            K::TokenCreated,
            EventDetail::TokenCreated {
                name: "my laptop".into(),
                access: ScopeAccess::ReadOnly,
                repositories: 0,
                until: "2027-02-23".into(),
            },
            "my laptop\tread-only\t0\t2027-02-23",
        ),
        (
            K::TokenCreated,
            EventDetail::TokenCreated {
                name: "ci".into(),
                access: ScopeAccess::Write,
                repositories: 2,
                until: "2026-12-25".into(),
            },
            "ci\twrite\t2\t2026-12-25",
        ),
        (
            K::TokenCreated,
            EventDetail::TokenCreated {
                name: "ops".into(),
                access: ScopeAccess::Full,
                repositories: 1,
                until: "2026-12-01".into(),
            },
            "ops\tfull\t1\t2026-12-01",
        ),
        (
            K::TokenRevoked,
            EventDetail::TokenRevoked {
                name: "my laptop".into(),
            },
            "my laptop",
        ),
        (
            K::RoleGranted,
            EventDetail::Role {
                repository: "alice/forge".into(),
                role: RepoRole::Admin,
            },
            "alice/forge\tadmin",
        ),
        (
            K::RoleRevoked,
            EventDetail::Role {
                repository: "acme/tool".into(),
                role: RepoRole::Read,
            },
            "acme/tool\tread",
        ),
    ]
}

#[test]
fn every_kind_has_one_stored_form_and_parses_back_to_the_data_it_was_made_from() {
    for (kind, data, stored) in table() {
        let e = typed(kind, &data);
        assert_eq!(e.detail, stored, "{kind:?}");
        assert_eq!(e.detail_data(), Some(data.clone()), "{kind:?}");
        // Through both stores: the stored text is the same, the tabs survive, the typed form is read back.
        let mem = InMemoryAccountEventStore::new();
        mem.record(&e).unwrap();
        assert_eq!(
            mem.all("alice").unwrap()[0].detail_data(),
            Some(data.clone())
        );
        let sql = SqliteAccountEventStore::new(SqliteDb::open_in_memory().unwrap()).unwrap();
        sql.record(&e).unwrap();
        let back = sql.all("alice").unwrap().remove(0);
        assert_eq!(back.detail, stored, "{kind:?}");
        assert_eq!(back.detail_data(), Some(data), "{kind:?}");
    }
    // Every kind of the closed set is in the table.
    for k in AccountEventKind::ALL {
        assert!(
            table().iter().any(|(t, _, _)| *t == k),
            "{k:?} has no row in the table"
        );
    }
}

#[test]
fn no_stored_form_holds_a_word_but_the_role_and_scope_codes_and_what_a_person_typed() {
    // The generated parts are digits, a date, a host and the codes; a sentence would have spaces and words of its own.
    for (kind, data, stored) in table() {
        let generated: Vec<&str> = match &data {
            EventDetail::SignInsEnded(_) => vec![stored],
            EventDetail::TokenCreated { .. } => stored.split('\t').skip(1).collect(),
            EventDetail::Role { .. } => stored.split('\t').skip(1).collect(),
            _ => vec![],
        };
        for part in generated {
            assert!(!part.contains(' '), "{kind:?}: {part:?} looks like prose");
        }
    }
    let old = [
        "2 session(s) ended",
        "laptop; write, 1 repositories; until 2026-12-01",
        "alice/forge admin",
    ];
    // The foundation's old prose is not any kind's typed form: it parses as nothing.
    for k in AccountEventKind::ALL {
        for text in old {
            let e = AccountEvent {
                account: "alice".into(),
                kind: k,
                by: EventBy::Operator,
                detail: text.to_owned(),
                at: AT,
            };
            let parsed = e.detail_data();
            // `Disabled` takes any one-line text as its reason: it is text a person typed, not a generated sentence.
            if k == AccountEventKind::Disabled
                || matches!(
                    k,
                    AccountEventKind::IdentityLinked
                        | AccountEventKind::IdentityUnlinked
                        | AccountEventKind::TokenRevoked
                )
            {
                continue;
            }
            assert_eq!(parsed, None, "{k:?}: {text:?} must not parse");
        }
    }
}

#[test]
fn a_detail_that_does_not_parse_is_none_never_the_raw_text() {
    use AccountEventKind as K;
    let bad: &[(K, &str)] = &[
        (K::Created, "x"),
        (K::PasswordSet, ""),
        (K::PasswordSet, "two"),
        (K::PasswordChanged, "-1"),
        (K::SessionEnded, "99999999999"),
        (K::IdentityLinked, ""),
        (K::IdentityLinked, "a\tb"),
        (K::Disabled, "a\tb"),
        (K::TokenCreated, ""),
        (K::TokenCreated, "n\tadmin\t1\t2027-01-01"),
        (K::TokenCreated, "n\tfull\tmany\t2027-01-01"),
        (K::TokenCreated, "n\tfull\t1\t27-01-01"),
        (K::TokenCreated, "n\tfull\t1\t2027-01-01\textra"),
        (K::TokenCreated, "\tfull\t1\t2027-01-01"),
        (K::TokenRevoked, ""),
        (K::TokenRevoked, "a\tb"),
        (K::RoleGranted, "alice/forge"),
        (K::RoleGranted, "alice/forge\towner"),
        (K::RoleRevoked, "noslash\tread"),
        (K::RoleRevoked, "alice/forge\tread\textra"),
    ];
    for (k, text) in bad {
        let e = AccountEvent {
            account: "alice".into(),
            kind: *k,
            by: EventBy::Operator,
            detail: (*text).to_owned(),
            at: AT,
        };
        assert_eq!(e.detail_data(), None, "{k:?} {text:?}");
    }
}

#[test]
fn the_longest_legal_event_fits_the_cut_and_still_parses() {
    let name = "n".repeat(60);
    let token = EventDetail::TokenCreated {
        name,
        access: ScopeAccess::ReadOnly,
        repositories: 20,
        until: "2027-12-31".into(),
    };
    let role = EventDetail::Role {
        repository: format!("{}/{}", "o".repeat(39), "r".repeat(64)),
        role: RepoRole::Maintain,
    };
    let host = EventDetail::Host("h".repeat(100));
    for (kind, d) in [
        (AccountEventKind::TokenCreated, token),
        (AccountEventKind::RoleGranted, role),
        (AccountEventKind::IdentityLinked, host),
    ] {
        let e = typed(kind, &d);
        let n = e.detail.chars().count();
        eprintln!("{kind:?}: {n} characters");
        assert!(n < planeter_auth::EVENT_DETAIL_MAX_CHARS, "{kind:?}: {n}");
        assert_eq!(e.detail_data(), Some(d));
    }
}

#[test]
fn what_a_person_typed_is_shown_as_typed_and_a_disguise_never_breaks_the_fields() {
    // A reason is kept as typed (made visible when it is a disguise); a tab in it cannot become a field separator.
    let e = typed(
        AccountEventKind::Disabled,
        &EventDetail::Reason("a\tb\u{202E}c".into()),
    );
    assert_eq!(e.detail, "a\\u{9}b\\u{202E}c");
    assert_eq!(
        e.detail_data(),
        Some(EventDetail::Reason("a\\u{9}b\\u{202E}c".into()))
    );
    // A token name with a tab (a hand-built row, not one the service accepts) cannot add a field.
    let t = typed(
        AccountEventKind::TokenCreated,
        &EventDetail::TokenCreated {
            name: "x\ty".into(),
            access: ScopeAccess::Full,
            repositories: 0,
            until: "2027-01-01".into(),
        },
    );
    assert_eq!(t.detail.split('\t').count(), 4);
    assert!(matches!(
        t.detail_data(),
        Some(EventDetail::TokenCreated { .. })
    ));
    // `AccountEvent::new` (for typed-in text) escapes a tab too.
    let n = AccountEvent::new(
        "alice",
        AccountEventKind::Disabled,
        EventBy::Operator,
        "p\tq",
        AT,
    );
    assert_eq!(n.detail, "p\\u{9}q");
}
