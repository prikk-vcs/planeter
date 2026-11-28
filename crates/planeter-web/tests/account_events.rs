//! RFC 014 D-7: the log of an account in the reader's language. **One function** (`render_event`) turns a stored event into a sentence, from
//! its typed form; the operator's commands and the account's page use it, so they cannot differ. Every kind in English and in Japanese
//! (a table), the plural boundaries, a hostile name, an unparsable detail shown as nothing, and **no stored detail holds a word of the
//! catalogs** (the generated parts are data: codes, counts, a host, a date).

use std::collections::BTreeSet;
use std::sync::Arc;

use planeter_auth::password::check_new_password;
use planeter_auth::{
    AccountEvent, AccountEventKind as K, AccountEventStore, AccountService, Argon2idHasher,
    EventBy, EventDetail, RepoRole, Sha256TokenHasher, SqliteAccountEventStore, SqliteAccountStore,
    SqliteCredentialStore, TokenHasher, TokenSpec,
};
use planeter_core::ScopeAccess;
use planeter_store::SqliteDb;
use planeter_web::account_events::render_event;
use planeter_web::catalog::{Lang, T, shipped};

const AT: u64 = 1_795_564_800;

fn t(tag: &str) -> T {
    T::new(Lang::from_tag(tag).unwrap())
}

fn ev(kind: K, by: EventBy, d: &EventDetail) -> AccountEvent {
    AccountEvent::typed("alice", kind, by, d, AT)
}

fn token(name: &str, access: ScopeAccess, repositories: u32) -> EventDetail {
    EventDetail::TokenCreated {
        name: name.into(),
        access,
        repositories,
        until: "2027-02-23".into(),
    }
}

const OP_EN: &str = "this server's operator";
const OP_JA: &str = "このサーバーを運用している人";

#[test]
fn every_kind_is_rendered_in_english_and_in_japanese_from_its_typed_form() {
    let op = EventBy::Operator;
    let rows: Vec<(AccountEvent, String, String)> = vec![
        (
            ev(K::Created, op, &EventDetail::None),
            format!("The account was created by {OP_EN}."),
            format!("アカウントは、{OP_JA}が作成しました。"),
        ),
        (
            ev(K::PasswordSet, op, &EventDetail::SignInsEnded(0)),
            format!("A password was set by {OP_EN}."),
            format!("パスワードを、{OP_JA}が設定しました。"),
        ),
        (
            ev(K::PasswordSet, op, &EventDetail::SignInsEnded(2)),
            format!("A password was set by {OP_EN}. 2 sign-ins ended."),
            format!(
                "パスワードを、{OP_JA}が設定しました。ログイン中の状態を 2 件、終わらせました。"
            ),
        ),
        (
            ev(
                K::PasswordChanged,
                EventBy::SelfService,
                &EventDetail::SignInsEnded(1),
            ),
            "The password was changed by you. 1 sign-in ended.".to_owned(),
            "パスワードを、あなたが変更しました。ログイン中の状態を 1 件、終わらせました。"
                .to_owned(),
        ),
        (
            ev(
                K::IdentityLinked,
                EventBy::Provider,
                &EventDetail::Host("idp.example:8443".into()),
            ),
            "An identity at idp.example:8443 was linked to the account by the sign-in provider."
                .to_owned(),
            "idp.example:8443 の ID を、ログインのプロバイダーがアカウントに連携しました。"
                .to_owned(),
        ),
        (
            ev(
                K::IdentityUnlinked,
                op,
                &EventDetail::Host("idp.example".into()),
            ),
            format!("An identity at idp.example was unlinked from the account by {OP_EN}."),
            format!("idp.example の ID とアカウントの連携を、{OP_JA}が解除しました。"),
        ),
        (
            ev(
                K::TokenCreated,
                op,
                &token("my laptop", ScopeAccess::ReadOnly, 0),
            ),
            format!(
                "Access token \"my laptop\" (read-only, all repositories the account can reach, until 2027-02-23) was created by {OP_EN}."
            ),
            format!(
                "トークン「my laptop」（読み取り専用、アカウントがアクセスできるすべてのリポジトリ、2027-02-23 まで）を、{OP_JA}が作成しました。"
            ),
        ),
        (
            ev(K::TokenCreated, op, &token("ci", ScopeAccess::Write, 1)),
            format!(
                "Access token \"ci\" (write, 1 repository, until 2027-02-23) was created by {OP_EN}."
            ),
            format!(
                "トークン「ci」（書き込み、1 件のリポジトリ、2027-02-23 まで）を、{OP_JA}が作成しました。"
            ),
        ),
        (
            ev(K::TokenCreated, op, &token("ops", ScopeAccess::Full, 2)),
            format!(
                "Access token \"ops\" (full, 2 repositories, until 2027-02-23) was created by {OP_EN}."
            ),
            format!(
                "トークン「ops」（完全なアクセス、2 件のリポジトリ、2027-02-23 まで）を、{OP_JA}が作成しました。"
            ),
        ),
        (
            ev(
                K::TokenRevoked,
                EventBy::SelfService,
                &EventDetail::TokenRevoked {
                    name: "my laptop".into(),
                },
            ),
            "Access token \"my laptop\" was revoked by you.".to_owned(),
            "トークン「my laptop」を、あなたが取り消しました。".to_owned(),
        ),
        (
            ev(K::SessionEnded, op, &EventDetail::SignInsEnded(1)),
            format!("1 sign-in was ended by {OP_EN}."),
            format!("ログイン中の状態 1 件を、{OP_JA}が終わらせました。"),
        ),
        (
            ev(K::SessionEnded, op, &EventDetail::SignInsEnded(3)),
            format!("3 sign-ins were ended by {OP_EN}."),
            format!("ログイン中の状態 3 件を、{OP_JA}が終わらせました。"),
        ),
        (
            ev(
                K::RoleGranted,
                op,
                &EventDetail::Role {
                    repository: "alice/forge".into(),
                    role: RepoRole::Admin,
                },
            ),
            format!("The role \"admin\" on alice/forge was given by {OP_EN}."),
            format!("alice/forge での権限「リポジトリの管理者」を、{OP_JA}が与えました。"),
        ),
        (
            ev(
                K::RoleRevoked,
                op,
                &EventDetail::Role {
                    repository: "acme/tool".into(),
                    role: RepoRole::Read,
                },
            ),
            format!("The role \"read\" on acme/tool was taken away by {OP_EN}."),
            format!("acme/tool での権限「読み取り」を、{OP_JA}が取り消しました。"),
        ),
        (
            ev(
                K::RoleGranted,
                op,
                &EventDetail::Role {
                    repository: "acme/tool".into(),
                    role: RepoRole::Maintain,
                },
            ),
            format!("The role \"maintain\" on acme/tool was given by {OP_EN}."),
            format!("acme/tool での権限「メンテナー」を、{OP_JA}が与えました。"),
        ),
        (
            ev(
                K::Disabled,
                op,
                &EventDetail::Reason("left the project".into()),
            ),
            format!("The account was disabled by {OP_EN}: left the project"),
            format!("アカウントを、{OP_JA}が無効にしました: left the project"),
        ),
        (
            ev(K::Enabled, op, &EventDetail::None),
            format!("The account was enabled by {OP_EN}."),
            format!("アカウントを、{OP_JA}が有効にしました。"),
        ),
        (
            ev(K::SignedInNewMethod, EventBy::Provider, &EventDetail::None),
            "The account signed in by a method it had not used before.".to_owned(),
            "アカウントが、これまで使っていなかった方法でログインしました。".to_owned(),
        ),
    ];
    for (e, en, ja) in &rows {
        assert_eq!(&render_event(&t("en"), e), en, "{:?}", e.kind);
        assert_eq!(&render_event(&t("ja"), e), ja, "{:?}", e.kind);
    }
    // Every kind of the closed set is in the table.
    for k in K::ALL {
        assert!(rows.iter().any(|(e, _, _)| e.kind == k), "{k:?} has no row");
    }
}

#[test]
fn the_plural_boundaries_for_repositories_and_sign_ins_in_both_languages() {
    let repos = |n: u32| {
        let e = ev(
            K::TokenCreated,
            EventBy::SelfService,
            &token("t", ScopeAccess::Write, n),
        );
        (render_event(&t("en"), &e), render_event(&t("ja"), &e))
    };
    let (en0, ja0) = repos(0);
    let (en1, ja1) = repos(1);
    let (en2, ja2) = repos(2);
    assert!(
        en0.contains("all repositories the account can reach")
            && ja0.contains("アカウントがアクセスできるすべてのリポジトリ")
    );
    assert!(en1.contains("(write, 1 repository, until") && ja1.contains("、1 件のリポジトリ、"));
    assert!(en2.contains("(write, 2 repositories, until") && ja2.contains("、2 件のリポジトリ、"));
    assert!(!en1.contains("1 repositories"));
    let ended = |n: u32| {
        let e = ev(
            K::PasswordChanged,
            EventBy::SelfService,
            &EventDetail::SignInsEnded(n),
        );
        (render_event(&t("en"), &e), render_event(&t("ja"), &e))
    };
    let (en0, ja0) = ended(0);
    let (en1, ja1) = ended(1);
    let (en2, ja2) = ended(2);
    assert_eq!(en0, "The password was changed by you.");
    assert_eq!(ja0, "パスワードを、あなたが変更しました。");
    assert!(
        en1.ends_with("1 sign-in ended.")
            && ja1.ends_with("ログイン中の状態を 1 件、終わらせました。")
    );
    assert!(
        en2.ends_with("2 sign-ins ended.")
            && ja2.ends_with("ログイン中の状態を 2 件、終わらせました。")
    );
}

#[test]
fn a_hostile_name_is_visible_and_cannot_break_the_sentence() {
    // The service refuses such a name; a row written some other way must still be shown safely: escapes visible, one line.
    let hostile = "x\u{202E}y\"); DROP\nTABLE\t{by}";
    let e = ev(
        K::TokenCreated,
        EventBy::Operator,
        &token(hostile, ScopeAccess::Full, 0),
    );
    for tag in ["en", "ja"] {
        let s = render_event(&t(tag), &e);
        assert!(
            s.contains("\\u{202E}") && s.contains("\\u{A}") && s.contains("\\u{9}"),
            "{s}"
        );
        assert!(
            !s.contains('\u{202E}') && !s.contains('\n') && !s.contains('\t'),
            "{s:?}"
        );
        assert!(
            s.contains("{by}"),
            "a name that looks like a placeholder is shown as it is: {s}"
        );
    }
    let r = ev(
        K::Disabled,
        EventBy::Operator,
        &EventDetail::Reason("because\u{202E}\nwhy".into()),
    );
    let s = render_event(&t("en"), &r);
    assert!(!s.contains('\u{202E}') && !s.contains('\n'), "{s:?}");
}

#[test]
fn an_unparsable_detail_is_shown_as_nothing_never_as_raw_text_and_breaks_nothing() {
    let junk = [
        "2 session(s) ended",
        "laptop; write, 1 repositories; until 2026-12-01",
        "alice/forge admin",
        "a\tb\tc\td\te",
        "\t",
        "",
        "ZZRAW",
    ];
    for k in K::ALL {
        for text in junk {
            let e = AccountEvent {
                account: "alice".into(),
                kind: k,
                by: EventBy::Operator,
                detail: text.to_owned(),
                at: AT,
            };
            for tag in ["en", "ja"] {
                let s = render_event(&t(tag), &e);
                assert!(!s.is_empty(), "{k:?} {text:?}");
                if e.detail_data().is_none() {
                    // Nothing of the unparsable text appears.
                    assert!(
                        !s.contains("ZZRAW")
                            && !s.contains("session(s)")
                            && !s.contains("laptop")
                            && !s.contains("repositories;"),
                        "{k:?} {text:?}: {s}"
                    );
                }
            }
        }
    }
    // The plain sentence of a kind whose detail is lost still says who did what.
    let lost = AccountEvent {
        account: "alice".into(),
        kind: K::TokenCreated,
        by: EventBy::Operator,
        detail: "garbage".into(),
        at: AT,
    };
    assert_eq!(
        render_event(&t("en"), &lost),
        format!("An access token was created by {OP_EN}.")
    );
    assert_eq!(
        render_event(&t("ja"), &lost),
        format!("トークンを、{OP_JA}が作成しました。")
    );
}

// ------------------------------------------------------------------------------------------------ no stored word

/// The English words of the catalog (ASCII letter runs of three or more letters, lower case, placeholders left out).
fn catalog_words() -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    for lang in shipped() {
        for e in lang.entries {
            for text in [e.one(), e.other()] {
                let mut inside = false;
                let mut run = String::new();
                for c in text.chars().chain(std::iter::once(' ')) {
                    if c == '{' {
                        inside = true;
                    }
                    if c == '}' {
                        inside = false;
                        run.clear();
                        continue;
                    }
                    if !inside && c.is_ascii_alphabetic() {
                        run.push(c.to_ascii_lowercase());
                    } else {
                        if run.len() >= 3 {
                            words.insert(run.clone());
                        }
                        run.clear();
                    }
                }
            }
        }
    }
    words
}

#[test]
fn no_stored_detail_holds_a_word_of_the_catalogs_except_the_role_and_scope_codes() {
    let db = SqliteDb::open_in_memory().unwrap();
    let service = AccountService::new(
        db.clone(),
        Arc::new(Argon2idHasher),
        Arc::new(|w: &str| w == "api"),
    )
    .unwrap();
    let _ = SqliteAccountStore::new(db.clone()).unwrap();
    let _ = SqliteCredentialStore::new(db.clone()).unwrap();
    let events = SqliteAccountEventStore::new(db.clone()).unwrap();
    // Names that are not words of any catalog: what a person typed is theirs, and the canary tells it from what the system wrote.
    let (handle, owner, repo, issuer, reason, token_name) = (
        "zqalice",
        "zqowner",
        "zqrepo",
        "https://qzv9.zkx:8443",
        "xqzwreason",
        "xqzwtoken",
    );
    db.lock()
        .unwrap()
        .execute(
            "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, path) VALUES ('r1', 'user', ?1, ?2, 'private', 0, '/p')",
            (owner, repo),
        )
        .unwrap();
    service
        .create_account(handle, "Zq Alice", None, EventBy::Operator, AT)
        .unwrap();
    let pw = check_new_password("zebra-quartz-plinth-9137-marmot", handle).unwrap();
    for by in [EventBy::Operator, EventBy::SelfService] {
        service.set_password(handle, &pw, by, AT).unwrap();
    }
    service
        .link_oidc(handle, issuer, "subject-1", EventBy::Operator, AT)
        .unwrap();
    service
        .unlink_oidc(handle, issuer, "subject-1", EventBy::Operator, AT)
        .unwrap();
    service
        .grant_role(
            handle,
            owner,
            repo,
            RepoRole::Maintain,
            EventBy::Operator,
            AT,
        )
        .unwrap();
    service
        .revoke_role(handle, owner, repo, true, EventBy::Operator, AT)
        .unwrap();
    let repos = [(owner.to_owned(), repo.to_owned())];
    for (access, limited) in [
        (ScopeAccess::ReadOnly, &[][..]),
        (ScopeAccess::Write, &repos[..]),
        (ScopeAccess::Full, &[][..]),
    ] {
        let spec = TokenSpec {
            handle,
            hash: Sha256TokenHasher.hash(&format!("secret-{access:?}")),
            name: token_name,
            access,
            repos: limited,
            expires_at: AT + 30 * 86_400,
        };
        let made = service
            .create_token_for(spec, EventBy::Operator, AT)
            .unwrap();
        service
            .revoke_token_of(handle, &made.id, EventBy::Operator, AT)
            .unwrap();
    }
    service
        .disable(handle, reason, EventBy::Operator, AT)
        .unwrap();
    service.enable(handle, EventBy::Operator, AT).unwrap();
    let all = events.all(handle).unwrap();
    let written: BTreeSet<&str> = all.iter().map(|e| e.kind.as_str()).collect();
    for k in K::ALL {
        // `session_ended` and `signed_in_new_method` are not written by the service yet (step 2); the rest all are.
        if !matches!(k, K::SessionEnded | K::SignedInNewMethod) {
            assert!(written.contains(k.as_str()), "{k:?} was not exercised");
        }
    }
    let words = catalog_words();
    let allowed: BTreeSet<&str> = ["read", "write", "maintain", "admin", "full", "only"]
        .into_iter()
        .collect();
    let canaries = [handle, owner, repo, reason, token_name, "qzv", "zkx"];
    for e in &all {
        assert!(
            e.detail_data().is_some(),
            "{:?} {:?} does not parse",
            e.kind,
            e.detail
        );
        for field in e.detail.split('\t') {
            for run in field
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|r| r.len() >= 3)
            {
                let run = run.to_ascii_lowercase();
                if allowed.contains(run.as_str())
                    || canaries.iter().any(|c| c.contains(run.as_str()))
                {
                    continue;
                }
                assert!(
                    !words.contains(&run),
                    "{:?} stores the word {run:?} in {:?}: a detail is data, not prose",
                    e.kind,
                    e.detail
                );
            }
        }
    }
    // The canary is not a word of the catalogs (so the test could tell), and the roles and scopes it allows are the codes.
    for c in canaries {
        assert!(!words.contains(c), "{c} is a catalog word");
    }
}
