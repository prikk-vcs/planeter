//! RFC 014 D-4: the password rule, its Unicode normalization, the hashes made before the rule existed, and the rehash after a sign-in
//! with older Argon2id parameters.

use std::sync::Arc;

use planeter_auth::password::{
    PASSWORD_MAX, PASSWORD_MIN, PasswordError, check_new_password, common_list_len, nfkc,
    normalize_presented,
};
use planeter_auth::{
    Account, AccountStore, Argon2idHasher, AuthError, Authenticator, InMemoryAccountStore,
    InMemoryCredentialStore, PasswordHash, PasswordHasher, Sha256TokenHasher, SqliteAccountStore,
};
use planeter_core::Principal;
use planeter_store::{SqliteDb, UserId};

/// Made by the code **before** this rule existed (`Argon2idHasher.hash("correct horse battery staple")`, 2026-11-25, parameters
/// m = 19456, t = 2, p = 1): it must verify unchanged.
const OLD_HASH_ASCII: &str = "$argon2id$v=19$m=19456,t=2,p=1$i6Uv6eljQnib9VURzmGI6g$/gfXs+BBDU/a/tzdzZ24Sr+GeIdqNqi23jER9S8U0tU";
/// Made by the same code from the composed Japanese text `パスワードは十分に長いです`.
const OLD_HASH_JAPANESE: &str = "$argon2id$v=19$m=19456,t=2,p=1$bwfoGMR+zZDaIMK6/rf6Gg$YO5cmH4IS+K+lHzlK2YeUbs3junr5KHJ0J8MkKJWJ7g";

fn ok(pw: &str) -> Result<(), PasswordError> {
    check_new_password(pw, "alice").map(|_| ())
}

#[test]
fn the_length_rule_counts_scalar_values_after_normalization() {
    let of = |n: usize| "x1".repeat(n).chars().take(n).collect::<String>();
    assert_eq!(PASSWORD_MIN, 12);
    assert_eq!(PASSWORD_MAX, 128);
    assert_eq!(ok(&of(11)), Err(PasswordError::TooShort));
    assert_eq!(ok(&of(12)), Ok(()));
    assert_eq!(ok(&of(128)), Ok(()));
    assert_eq!(ok(&of(129)), Err(PasswordError::TooLong));
    assert_eq!(ok(""), Err(PasswordError::TooShort));
    // An emoji is one scalar value: eleven letters and one emoji are twelve characters (and are four bytes more than twelve letters).
    let emoji = format!("{}\u{1F600}", of(11));
    assert_eq!(emoji.chars().count(), 12);
    assert!(emoji.len() > 12);
    assert_eq!(ok(&emoji), Ok(()));
    // A family emoji is seven scalar values (four people and three joiners): it counts as seven, and that is said, not hidden.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert_eq!(family.chars().count(), 7);
    assert_eq!(ok(&format!("{}{}", of(5), family)), Ok(()));
    assert_eq!(
        ok(&format!("{}{}", of(4), family)),
        Err(PasswordError::TooShort)
    );
    // Counted AFTER normalization: twelve fullwidth letters are twelve; a composed `ｶﾞ` (halfwidth katakana + voiced mark, two
    // scalars) becomes the one scalar `ガ`.
    assert_eq!(nfkc("\u{ff76}\u{ff9e}"), "\u{30ac}");
    let halfwidth = "\u{ff76}\u{ff9e}".repeat(6); // twelve scalars before, six after
    assert_eq!(halfwidth.chars().count(), 12);
    assert_eq!(ok(&halfwidth), Err(PasswordError::TooShort));
}

#[test]
fn a_huge_input_is_refused_before_it_is_normalized() {
    let huge = "a".repeat(1_000_000);
    assert_eq!(ok(&huge), Err(PasswordError::TooLong));
    assert!(normalize_presented(&huge).is_none());
    assert!(normalize_presented(&"a".repeat(4096)).is_some());
    assert!(normalize_presented(&"a".repeat(4097)).is_none());
}

#[test]
fn composed_and_decomposed_forms_are_one_password() {
    // が as U+304C, and as U+304B U+3099.
    let composed = "\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}\u{304c}";
    let decomposed = composed.replace('\u{304c}', "\u{304b}\u{3099}");
    assert_ne!(composed, decomposed);
    assert_eq!(composed.chars().count(), 12);
    assert_eq!(decomposed.chars().count(), 24);
    let a = check_new_password(composed, "alice").unwrap();
    let b = check_new_password(&decomposed, "alice").unwrap();
    assert_eq!(a, b, "the same password after normalization");
    assert_eq!(
        b.as_str().chars().count(),
        12,
        "counted after normalization"
    );

    // Through the authenticator, with real Argon2id: set one way, sign in the other way, both ways.
    let hasher = Arc::new(Argon2idHasher);
    let accounts = Arc::new(InMemoryAccountStore::new());
    for (user, set_with) in [("composed", composed), ("decomposed", decomposed.as_str())] {
        let pw = check_new_password(set_with, user).unwrap();
        accounts
            .upsert(Account::local(user, user, Some(hasher.hash(pw.as_str()))))
            .unwrap();
    }
    let auth = Authenticator::new(
        accounts,
        Arc::new(InMemoryCredentialStore::new()),
        hasher,
        Arc::new(Sha256TokenHasher),
    );
    for user in ["composed", "decomposed"] {
        for typed in [composed, decomposed.as_str()] {
            assert_eq!(
                auth.authenticate_password(user, typed),
                Ok(Principal::User(UserId::new(user))),
                "{user} typed as {} scalars",
                typed.chars().count()
            );
        }
        assert_eq!(
            auth.authenticate_password(user, "\u{304c}"),
            Err(AuthError::BadCredentials)
        );
    }
}

#[test]
fn a_fullwidth_latin_password_is_its_ascii_form_because_that_is_what_nfkc_does() {
    // NFKC folds compatibility forms: fullwidth `ｃｏｒｒｅｃｔ` is `correct`. The same folding happens when the password is set and
    // when it is typed, so nobody is locked out by it; it also means that two passwords that differ only by such forms are one password.
    let fullwidth = "\u{ff43}\u{ff4f}\u{ff52}\u{ff52}\u{ff45}\u{ff43}\u{ff54}-horse-1";
    let ascii = "correct-horse-1";
    assert_eq!(nfkc(fullwidth), ascii);
    assert_eq!(
        check_new_password(fullwidth, "alice").unwrap(),
        check_new_password(ascii, "alice").unwrap()
    );
    // And for ASCII, normalization changes nothing at all.
    for s in [
        "correct horse battery staple",
        "Tr0ub4dor&3 plus more",
        "~!@#$%^&*()_+{}|:<>?",
    ] {
        assert_eq!(nfkc(s), s);
    }
}

#[test]
fn a_password_equal_to_the_handle_is_refused_in_any_case_and_any_width() {
    let handle = "alice-the-maintainer";
    for pw in [
        "alice-the-maintainer",
        "ALICE-THE-MAINTAINER",
        "Alice-The-Maintainer",
        "\u{ff41}lice-the-maintainer", // a fullwidth `ａ`
    ] {
        assert_eq!(
            check_new_password(pw, handle).err(),
            Some(PasswordError::IsHandle),
            "{pw:?}"
        );
    }
    // Containing the handle is not equal to it.
    assert!(check_new_password("alice-the-maintainer-2", handle).is_ok());
}

#[test]
fn a_common_password_is_refused_in_any_case_and_after_normalization() {
    assert!(common_list_len() >= 1000);
    let list = include_str!("../data/common-passwords.txt");
    let entry = list
        .lines()
        .find(|l| !l.starts_with('#') && !l.is_empty())
        .unwrap();
    assert!(entry.chars().count() >= PASSWORD_MIN, "{entry}");
    for variant in [entry.to_owned(), entry.to_uppercase(), {
        let mut c = entry.chars();
        c.next().unwrap().to_uppercase().collect::<String>() + c.as_str()
    }] {
        assert_eq!(
            check_new_password(&variant, "alice").err(),
            Some(PasswordError::TooCommon),
            "{variant:?}"
        );
    }
    // A fullwidth spelling of a common ASCII password is the same after NFKC.
    let wide: String = entry
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() {
                char::from_u32(c as u32 + 0xfee0).unwrap()
            } else {
                c
            }
        })
        .collect();
    assert_eq!(
        check_new_password(&wide, "alice").err(),
        Some(PasswordError::TooCommon)
    );
    // Not a common password: accepted.
    assert!(check_new_password(&format!("{entry}-and-more-words"), "alice").is_ok());
}

#[test]
fn no_error_and_no_debug_text_contains_the_password() {
    let secret = "zebra-quartz-plinth-9137"; // long enough and not common
    let n = check_new_password(secret, "alice").unwrap();
    assert!(!format!("{n:?}").contains("zebra"));
    // Every refusal, for a password that carries a recognisable word.
    let cases = [
        ("zebra-9", "alice"),
        (&"zebra".repeat(40)[..], "alice"),
        ("zebra-quartz-plinth", "zebra-quartz-plinth"),
    ];
    for (pw, handle) in cases {
        if let Err(e) = check_new_password(pw, handle) {
            for text in [format!("{e}"), format!("{e:?}"), e.reason().to_owned()] {
                assert!(!text.contains("zebra"), "{text}");
            }
        } else {
            panic!("{pw:?} should have been refused");
        }
    }
    let common = include_str!("../data/common-passwords.txt")
        .lines()
        .find(|l| !l.starts_with('#') && !l.is_empty())
        .unwrap()
        .to_owned();
    let e = check_new_password(&common, "alice").unwrap_err();
    for text in [format!("{e}"), format!("{e:?}"), e.reason().to_owned()] {
        assert!(!text.contains(&common), "{text}");
    }
    // The four words are stable and distinct.
    let words = [
        PasswordError::TooShort,
        PasswordError::TooLong,
        PasswordError::IsHandle,
        PasswordError::TooCommon,
    ]
    .map(PasswordError::reason);
    assert_eq!(words, ["too_short", "too_long", "is_handle", "too_common"]);
}

fn authenticator_with(hash: &str, store: Arc<dyn AccountStore>) -> Authenticator {
    store
        .upsert(Account::local(
            "alice",
            "Alice",
            Some(PasswordHash(hash.to_owned())),
        ))
        .unwrap();
    Authenticator::new(
        store,
        Arc::new(InMemoryCredentialStore::new()),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    )
}

#[test]
fn a_hash_made_before_the_rule_existed_still_verifies() {
    let auth = authenticator_with(OLD_HASH_ASCII, Arc::new(InMemoryAccountStore::new()));
    assert_eq!(
        auth.authenticate_password("alice", "correct horse battery staple"),
        Ok(Principal::User(UserId::new("alice")))
    );
    assert_eq!(
        auth.authenticate_password("alice", "correct horse battery stapl"),
        Err(AuthError::BadCredentials)
    );
    // The hash of composed Japanese text, made by the old code from the raw text, verifies when the same text is typed composed (what
    // a keyboard that sends composed text gives) **and** when it is typed decomposed (which the old code would have refused).
    let auth = authenticator_with(OLD_HASH_JAPANESE, Arc::new(InMemoryAccountStore::new()));
    let composed = "パスワードは十分に長いです";
    let decomposed = "\u{30cf}\u{309a}\u{30b9}\u{30ef}\u{30fc}\u{30c8}\u{3099}\u{306f}十分に長い\u{3066}\u{3099}\u{3059}";
    assert_ne!(composed, decomposed);
    assert_eq!(nfkc(decomposed), composed);
    assert!(auth.authenticate_password("alice", composed).is_ok());
    assert!(auth.authenticate_password("alice", decomposed).is_ok());
}

fn params_of(hash: &str) -> (u32, u32, u32) {
    let parsed = argon2::password_hash::PasswordHash::new(hash).unwrap();
    let p = argon2::Params::try_from(&parsed).unwrap();
    (p.m_cost(), p.t_cost(), p.p_cost())
}

#[test]
fn a_hash_with_older_parameters_is_replaced_after_a_correct_sign_in_and_only_then() {
    use argon2::PasswordHasher as _;
    // The parameters now: m = 19456 KiB, t = 2, p = 1 (Argon2id, version 0x13).
    assert_eq!(params_of(OLD_HASH_ASCII), (19456, 2, 1));
    assert!(!Argon2idHasher.needs_rehash(&PasswordHash(OLD_HASH_ASCII.to_owned())));
    // A deliberately weaker stored hash of the same password: m = 8 KiB, t = 1, p = 1.
    let weak = argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(8, 1, 1, None).unwrap(),
    )
    .hash_password(
        b"correct horse battery staple",
        &argon2::password_hash::SaltString::generate(&mut argon2::password_hash::rand_core::OsRng),
    )
    .unwrap()
    .to_string();
    assert_eq!(params_of(&weak), (8, 1, 1));
    assert!(Argon2idHasher.needs_rehash(&PasswordHash(weak.clone())));

    let db = SqliteDb::open_in_memory().unwrap();
    let store = Arc::new(SqliteAccountStore::new(db).unwrap());
    let auth = authenticator_with(&weak, store.clone());
    let stored = |s: &SqliteAccountStore| {
        s.get(&UserId::new("alice"))
            .unwrap()
            .unwrap()
            .password
            .unwrap()
            .0
    };

    // A wrong password does not replace it.
    assert!(
        auth.authenticate_password("alice", "not the password")
            .is_err()
    );
    assert_eq!(stored(&store), weak);
    // The right password does, once, with today's parameters, and the new hash verifies.
    assert!(
        auth.authenticate_password("alice", "correct horse battery staple")
            .is_ok()
    );
    let fresh = stored(&store);
    assert_ne!(fresh, weak);
    assert_eq!(params_of(&fresh), (19456, 2, 1));
    assert!(Argon2idHasher.verify("correct horse battery staple", &PasswordHash(fresh.clone())));
    // A second sign-in finds nothing to upgrade and leaves the hash alone.
    assert!(
        auth.authenticate_password("alice", "correct horse battery staple")
            .is_ok()
    );
    assert_eq!(stored(&store), fresh);
    // A hash that is not Argon2 at all is "not outdated": it cannot be verified, so it is not replaced by guessing.
    assert!(!Argon2idHasher.needs_rehash(&PasswordHash("not a hash".into())));
}

#[test]
fn a_presented_password_too_long_to_have_been_set_is_a_wrong_password() {
    let auth = authenticator_with(OLD_HASH_ASCII, Arc::new(InMemoryAccountStore::new()));
    let huge = "correct horse battery staple".repeat(1000);
    assert_eq!(
        auth.authenticate_password("alice", &huge),
        Err(AuthError::BadCredentials)
    );
}
