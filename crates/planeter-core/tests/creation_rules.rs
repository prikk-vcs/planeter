//! RFC 004 part 7: the pure rules of creating a repository — names, who may create where, quotas, and what the creator
//! must type before a maintainer's key is trusted.

use planeter_core::creation::{
    AdoptionError, AdoptionRequest, Carried, NameError, NewRepo, RequestError, TargetRefusal,
    check_owner_handle, check_repo_name, creation_target, fits_quota, group_fingerprint,
    normalize_fingerprint, quota_for, resolve_adoption,
};
use planeter_core::{Principal, RepoScope, Scope, ScopeAccess};
use planeter_store::{
    InMemoryMembershipStore, OrgId, OrgRole, Owner, ProposalPolicy, UserId, Visibility,
};

fn none(_: &str) -> bool {
    false
}

fn reserved(word: &str) -> bool {
    ["api", "new", "login", "static"].contains(&word)
}

#[test]
fn good_names_pass() {
    for name in [
        "app",
        "my-app",
        "my_app",
        "a",
        "app.v2",
        "0day",
        "a1.b2-c3_d4",
        &"a".repeat(64),
    ] {
        assert_eq!(check_repo_name(name, &none), Ok(()), "{name}");
    }
}

#[test]
fn hostile_and_sloppy_names_are_refused_with_a_reason() {
    let cases: Vec<(String, NameError)> = vec![
        ("".into(), NameError::Empty),
        ("a".repeat(65), NameError::TooLong),
        ("App".into(), NameError::Uppercase),
        ("aPP".into(), NameError::Uppercase),
        ("../x".into(), NameError::BadCharacter),
        ("a/b".into(), NameError::BadCharacter),
        ("/etc".into(), NameError::BadCharacter),
        ("a\\b".into(), NameError::BadCharacter),
        ("a\0b".into(), NameError::BadCharacter),
        ("a b".into(), NameError::BadCharacter),
        ("a\nb".into(), NameError::BadCharacter),
        ("naïve".into(), NameError::BadCharacter),
        ("日本".into(), NameError::BadCharacter),
        ("a%2fb".into(), NameError::BadCharacter),
        ("a?b".into(), NameError::BadCharacter),
        (".hidden".into(), NameError::BadStart),
        ("-x".into(), NameError::BadStart),
        ("_x".into(), NameError::BadStart),
        (".".into(), NameError::BadStart),
        ("..".into(), NameError::BadStart),
        ("a..b".into(), NameError::Dotted),
        ("a.".into(), NameError::Dotted),
        ("app.git".into(), NameError::Dotted),
    ];
    for (name, want) in cases {
        assert_eq!(check_repo_name(&name, &none), Err(want), "{name:?}");
    }
}

#[test]
fn a_case_collision_cannot_be_made_because_uppercase_is_refused() {
    // `App` and `app` are two names on a case-sensitive disk and one on a case-insensitive one; only one can exist.
    assert_eq!(check_repo_name("app", &none), Ok(()));
    assert_eq!(check_repo_name("App", &none), Err(NameError::Uppercase));
    assert_eq!(check_repo_name("APP", &none), Err(NameError::Uppercase));
}

#[test]
fn reserved_words_are_refused_for_names_and_handles() {
    for w in ["api", "new", "login", "static"] {
        assert_eq!(
            check_repo_name(w, &reserved),
            Err(NameError::Reserved),
            "{w}"
        );
    }
    assert_eq!(
        check_owner_handle("API", &reserved),
        Err(NameError::Reserved),
        "a handle is compared in lowercase"
    );
    assert_eq!(
        check_repo_name("apis", &reserved),
        Ok(()),
        "a reserved word is the whole name, not a prefix"
    );
}

#[test]
fn owner_handles_are_checked_for_syntax_only() {
    for ok in [
        "alice",
        "Alice",
        "a.b-c_d",
        "alice@example.org",
        "a+b",
        "x1",
    ] {
        assert_eq!(check_owner_handle(ok, &none), Ok(()), "{ok}");
    }
    for (bad, want) in [
        ("", NameError::Empty),
        ("..", NameError::BadStart),
        ("../x", NameError::BadCharacter),
        ("/abs", NameError::BadCharacter),
        ("a\0", NameError::BadCharacter),
        ("a/b", NameError::BadCharacter),
        ("a b", NameError::BadCharacter),
        ("a..b", NameError::Dotted),
        ("-a", NameError::BadStart),
    ] {
        assert_eq!(check_owner_handle(bad, &none), Err(want), "{bad:?}");
    }
    assert_eq!(
        check_owner_handle(&"a".repeat(129), &none),
        Err(NameError::TooLong)
    );
}

#[test]
fn a_request_is_validated_field_by_field_and_proposals_defaults_to_members() {
    let ok = NewRepo::parse("alice", "app", "private", None, &none).unwrap();
    assert_eq!(ok.visibility, Visibility::Private);
    assert_eq!(ok.proposals, ProposalPolicy::Members);
    let open = NewRepo::parse("alice", "app", "public", Some("any-authenticated"), &none).unwrap();
    assert_eq!(open.proposals, ProposalPolicy::AnyAuthenticated);
    assert_eq!(
        NewRepo::parse("alice", "app", "secret", None, &none),
        Err(RequestError::Visibility)
    );
    assert_eq!(
        NewRepo::parse("alice", "app", "public", Some("anyone"), &none),
        Err(RequestError::Proposals),
        "a typo in a setting is refused, never defaulted"
    );
    assert_eq!(
        NewRepo::parse("../a", "app", "public", None, &none),
        Err(RequestError::Owner(NameError::BadCharacter))
    );
    assert_eq!(
        NewRepo::parse("alice", "App", "public", None, &none),
        Err(RequestError::Name(NameError::Uppercase))
    );
}

// ---------------------------------------------------------------------------------------------------- who may create where

fn membership() -> InMemoryMembershipStore {
    let m = InMemoryMembershipStore::new();
    m.set_org_role(UserId::new("alice"), OrgId::new("acme"), OrgRole::Member)
        .unwrap();
    m.set_org_role(UserId::new("boss"), OrgId::new("acme"), OrgRole::Admin)
        .unwrap();
    m
}

fn user(id: &str) -> Principal {
    Principal::User(UserId::new(id))
}

#[test]
fn a_user_creates_in_their_own_namespace() {
    assert_eq!(
        creation_target(&user("alice"), "alice", &membership(), 0),
        Ok(Owner::User("alice".into()))
    );
}

#[test]
fn an_organization_member_and_admin_may_create_in_the_organization() {
    let m = membership();
    assert_eq!(
        creation_target(&user("alice"), "acme", &m, 0),
        Ok(Owner::Org("acme".into()))
    );
    assert_eq!(
        creation_target(&user("boss"), "acme", &m, 0),
        Ok(Owner::Org("acme".into()))
    );
}

#[test]
fn a_stranger_a_missing_organization_and_another_users_namespace_are_one_refusal() {
    let m = membership();
    // Not a member of an organization that exists; an organization that does not exist; another person's namespace.
    let a = creation_target(&user("mallory"), "acme", &m, 0);
    let b = creation_target(&user("mallory"), "nonesuch", &m, 0);
    let c = creation_target(&user("mallory"), "alice", &m, 0);
    assert_eq!(a, Err(TargetRefusal::Refused));
    assert_eq!(a, b, "no oracle for which organizations exist");
    assert_eq!(a, c);
}

#[test]
fn anonymous_is_not_signed_in_and_a_ci_job_is_refused() {
    let m = membership();
    assert_eq!(
        creation_target(&Principal::Anonymous, "alice", &m, 0),
        Err(TargetRefusal::NotSignedIn)
    );
    let ci = Principal::CiJob {
        scope: Scope {
            access: ScopeAccess::Full,
            repos: RepoScope::All,
            expires_at: None,
        },
    };
    assert_eq!(
        creation_target(&ci, "alice", &m, 0),
        Err(TargetRefusal::Refused)
    );
}

#[test]
fn a_credentials_own_ceiling_applies_in_both_namespaces() {
    let m = membership();
    let token = |access, expires_at| Principal::MachineAsUser {
        user: UserId::new("alice"),
        scope: Scope {
            access,
            repos: RepoScope::All,
            expires_at,
        },
    };
    assert_eq!(
        creation_target(&token(ScopeAccess::Write, None), "alice", &m, 0),
        Ok(Owner::User("alice".into()))
    );
    assert_eq!(
        creation_target(&token(ScopeAccess::Write, None), "acme", &m, 0),
        Ok(Owner::Org("acme".into()))
    );
    for handle in ["alice", "acme"] {
        assert_eq!(
            creation_target(&token(ScopeAccess::ReadOnly, None), handle, &m, 0),
            Err(TargetRefusal::Refused),
            "a read-only token creates nothing ({handle})"
        );
        assert_eq!(
            creation_target(&token(ScopeAccess::Full, Some(10)), handle, &m, 10),
            Err(TargetRefusal::Refused),
            "an expired token creates nothing ({handle})"
        );
    }
}

#[test]
fn quotas_are_per_owner_kind_and_count_up_to_the_limit() {
    assert_eq!(quota_for(&Owner::User("a".into()), 50, 500), 50);
    assert_eq!(quota_for(&Owner::Org("a".into()), 50, 500), 500);
    assert!(fits_quota(49, 50));
    assert!(!fits_quota(50, 50));
    assert!(!fits_quota(0, 0));
}

// ---------------------------------------------------------------------------------------------------- fingerprints

const KEY: &str = "388c758c92be70eebd0c5bcd8faed2fcd8c7dfe360018e7c4b88de34446f44e1";
const OTHER: &str = "0264de0ad99c151736a9d25ec4d5747256ba6f74709d945610309cbe6c8d9f1d";

#[test]
fn a_fingerprint_is_the_whole_key_ignoring_case_spaces_and_colons() {
    assert_eq!(normalize_fingerprint(KEY).as_deref(), Some(KEY));
    assert_eq!(
        normalize_fingerprint(&KEY.to_uppercase()).as_deref(),
        Some(KEY)
    );
    assert_eq!(
        normalize_fingerprint(&group_fingerprint(KEY)).as_deref(),
        Some(KEY)
    );
    assert_eq!(
        normalize_fingerprint(&format!("  {}\n", group_fingerprint(KEY))).as_deref(),
        Some(KEY)
    );
    let colons: String = KEY
        .as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(":");
    assert_eq!(normalize_fingerprint(&colons).as_deref(), Some(KEY));
}

#[test]
fn a_prefix_a_key_id_or_anything_else_is_not_a_fingerprint() {
    assert_eq!(normalize_fingerprint(&KEY[..63]), None, "one short");
    assert_eq!(
        normalize_fingerprint(&KEY[..16]),
        None,
        "the short id's hex"
    );
    assert_eq!(normalize_fingerprint("ed25519-388c758c92be70ee"), None);
    assert_eq!(normalize_fingerprint(&format!("{KEY}0")), None, "one long");
    assert_eq!(normalize_fingerprint(&format!("{KEY}{KEY}")), None);
    assert_eq!(normalize_fingerprint(""), None);
    assert_eq!(normalize_fingerprint(&KEY.replace('3', "g")), None);
    assert_eq!(normalize_fingerprint(&format!("{KEY} ; rm")), None);
}

#[test]
fn grouping_is_eight_by_eight() {
    assert_eq!(
        group_fingerprint(KEY),
        "388c758c 92be70ee bd0c5bcd 8faed2fc d8c7dfe3 60018e7c 4b88de34 446f44e1"
    );
    assert_eq!(group_fingerprint("short"), "short");
}

fn carried() -> Vec<Carried> {
    vec![
        Carried {
            key_id: "ed25519-388c758c92be70ee".into(),
            fingerprint: KEY.into(),
        },
        Carried {
            key_id: "ed25519-0264de0ad99c1517".into(),
            fingerprint: OTHER.into(),
        },
    ]
}

fn adopt(pairs: &[(&str, &str)], leave: &[&str], no_adopt: bool) -> AdoptionRequest {
    AdoptionRequest {
        adopt: pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect(),
        leave_out: leave.iter().map(|s| (*s).to_owned()).collect(),
        no_adopt,
    }
}

#[test]
fn every_key_typed_correctly_is_adopted() {
    let r = resolve_adoption(
        &carried(),
        &adopt(
            &[
                ("ed25519-388c758c92be70ee", &group_fingerprint(KEY)),
                ("ed25519-0264de0ad99c1517", &OTHER.to_uppercase()),
            ],
            &[],
            false,
        ),
    )
    .unwrap();
    assert_eq!(r.adopted.len(), 2);
    assert!(r.left_out.is_empty());
}

#[test]
fn a_wrong_fingerprint_is_refused_naming_the_key_and_never_the_expected_value() {
    let err = resolve_adoption(
        &carried(),
        &adopt(
            &[
                ("ed25519-388c758c92be70ee", KEY),
                ("ed25519-0264de0ad99c1517", KEY), // the other key's text typed for this one
            ],
            &[],
            false,
        ),
    )
    .unwrap_err();
    assert_eq!(
        err,
        AdoptionError::Mismatch {
            key_id: "ed25519-0264de0ad99c1517".into()
        }
    );
    let shown = format!("{err:?}");
    assert!(
        !shown.contains(OTHER),
        "the expected fingerprint is not in the error: {shown}"
    );
}

#[test]
fn a_malformed_a_short_and_an_empty_entry_are_refused_as_malformed() {
    for typed in ["", &KEY[..32], "not hex at all", "ed25519-388c758c92be70ee"] {
        let err = resolve_adoption(
            &carried(),
            &adopt(
                &[("ed25519-388c758c92be70ee", typed)],
                &["ed25519-0264de0ad99c1517"],
                false,
            ),
        )
        .unwrap_err();
        assert_eq!(
            err,
            AdoptionError::Malformed {
                key_id: "ed25519-388c758c92be70ee".into()
            },
            "{typed:?}"
        );
    }
}

#[test]
fn every_carried_key_must_be_adopted_or_explicitly_left_out() {
    let err = resolve_adoption(
        &carried(),
        &adopt(&[("ed25519-388c758c92be70ee", KEY)], &[], false),
    )
    .unwrap_err();
    assert_eq!(
        err,
        AdoptionError::Unaddressed {
            key_ids: vec!["ed25519-0264de0ad99c1517".into()]
        }
    );
    // Nothing at all is not a choice either.
    assert!(matches!(
        resolve_adoption(&carried(), &adopt(&[], &[], false)),
        Err(AdoptionError::Unaddressed { .. })
    ));
    // Leaving the second out explicitly makes it a choice.
    let r = resolve_adoption(
        &carried(),
        &adopt(
            &[("ed25519-388c758c92be70ee", KEY)],
            &["ed25519-0264de0ad99c1517"],
            false,
        ),
    )
    .unwrap();
    assert_eq!((r.adopted.len(), r.left_out.len()), (1, 1));
}

#[test]
fn no_adopt_leaves_every_key_out_and_cannot_be_mixed() {
    let r = resolve_adoption(&carried(), &adopt(&[], &[], true)).unwrap();
    assert!(r.adopted.is_empty());
    assert_eq!(r.left_out.len(), 2);
    assert_eq!(
        resolve_adoption(
            &carried(),
            &adopt(&[("ed25519-388c758c92be70ee", KEY)], &[], true)
        ),
        Err(AdoptionError::Conflicting)
    );
    assert_eq!(
        resolve_adoption(&carried(), &adopt(&[], &["ed25519-388c758c92be70ee"], true)),
        Err(AdoptionError::Conflicting)
    );
}

#[test]
fn an_unknown_or_repeated_key_is_refused() {
    assert_eq!(
        resolve_adoption(
            &carried(),
            &adopt(&[("ed25519-ffffffffffffffff", KEY)], &[], false)
        ),
        Err(AdoptionError::UnknownKey {
            key_id: "ed25519-ffffffffffffffff".into()
        })
    );
    assert_eq!(
        resolve_adoption(
            &carried(),
            &adopt(
                &[
                    ("ed25519-388c758c92be70ee", KEY),
                    ("ed25519-388c758c92be70ee", KEY)
                ],
                &["ed25519-0264de0ad99c1517"],
                false
            )
        ),
        Err(AdoptionError::Duplicate {
            key_id: "ed25519-388c758c92be70ee".into()
        })
    );
    assert_eq!(
        resolve_adoption(
            &carried(),
            &adopt(
                &[("ed25519-388c758c92be70ee", KEY)],
                &["ed25519-388c758c92be70ee", "ed25519-0264de0ad99c1517"],
                false
            )
        ),
        Err(AdoptionError::Duplicate {
            key_id: "ed25519-388c758c92be70ee".into()
        }),
        "adopted and left out at once"
    );
}

#[test]
fn an_archive_that_carries_no_key_has_nothing_to_adopt() {
    let r = resolve_adoption(&[], &adopt(&[], &[], false)).unwrap();
    assert!(r.adopted.is_empty() && r.left_out.is_empty());
    assert!(resolve_adoption(&[], &adopt(&[], &[], true)).is_ok());
}
