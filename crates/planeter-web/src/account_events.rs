//! The log of an account in the reader's language (RFC 014 D-7): **one function**, [`render_event`], turns a stored event into its sentence.
//! The operator's commands print it as "the sentence the holder will read", and the account's own page (step 2) shows it, so the two
//! cannot differ.
//!
//! An event stores **data, never prose** (see `planeter_auth::events`): codes, counts, a host, a date, and the two texts a person typed. The
//! words, the plural forms and the order of a sentence are the catalog's: nothing here is English or Japanese. A detail that does not parse
//! (an older or damaged row) is **shown as nothing**: the sentence of the kind without that part, never the raw text.

use planeter_auth::{AccountEvent, AccountEventKind, EventBy, EventDetail, RepoRole};
use planeter_core::ScopeAccess;

use crate::catalog::{Key, T};

/// Who did it, in words.
pub fn by_word(t: &T, by: EventBy) -> String {
    t.get(match by {
        EventBy::SelfService => Key::AccountBySelf,
        EventBy::Operator => Key::AccountByOperator,
        EventBy::Provider => Key::AccountByProvider,
    })
}

fn role_word(t: &T, role: RepoRole) -> String {
    t.get(match role {
        RepoRole::Read => Key::AccountEventRoleRead,
        RepoRole::Write => Key::AccountEventRoleWrite,
        RepoRole::Maintain => Key::AccountEventRoleMaintain,
        RepoRole::Admin => Key::AccountEventRoleAdmin,
    })
}

/// A token's scope in the words the token commands use.
fn scope_word(t: &T, access: ScopeAccess) -> String {
    t.get(match access {
        ScopeAccess::ReadOnly => Key::OperatorTokenListReadOnly,
        ScopeAccess::Write => Key::OperatorTokenListWrite,
        ScopeAccess::Full => Key::OperatorTokenListFull,
    })
}

/// The sentence of one event, in the language of `t`, from its typed form.
pub fn render_event(t: &T, e: &AccountEvent) -> String {
    let by = by_word(t, e.by);
    let detail = e.detail_data();
    let plain = |key: Key| t.msg(key).with("by", &by).text();
    match (e.kind, detail) {
        (AccountEventKind::Created, _) => plain(Key::AccountEventCreated),
        (AccountEventKind::Enabled, _) => plain(Key::AccountEventEnabled),
        (AccountEventKind::SignedInNewMethod, _) => plain(Key::AccountEventSignedInNewMethod),
        (kind @ (AccountEventKind::PasswordSet | AccountEventKind::PasswordChanged), detail) => {
            let first = plain(if kind == AccountEventKind::PasswordSet {
                Key::AccountEventPasswordSet
            } else {
                Key::AccountEventPasswordChanged
            });
            match detail {
                Some(EventDetail::SignInsEnded(n)) if n > 0 => t
                    .msg(Key::AccountEventThen)
                    .with("first", first)
                    .with(
                        "second",
                        t.msg(Key::AccountEventSignInsEnded).n(u64::from(n)).text(),
                    )
                    .text(),
                _ => first,
            }
        }
        (AccountEventKind::SessionEnded, detail) => {
            let n = match detail {
                Some(EventDetail::SignInsEnded(n)) => u64::from(n),
                _ => 1,
            };
            t.msg(Key::AccountEventSessionEnded)
                .n(n)
                .with("by", &by)
                .text()
        }
        (AccountEventKind::IdentityLinked, Some(EventDetail::Host(host))) => t
            .msg(Key::AccountEventIdentityLinked)
            .with("host", host)
            .with("by", &by)
            .text(),
        (AccountEventKind::IdentityLinked, _) => plain(Key::AccountEventPlainIdentityLinked),
        (AccountEventKind::IdentityUnlinked, Some(EventDetail::Host(host))) => t
            .msg(Key::AccountEventIdentityUnlinked)
            .with("host", host)
            .with("by", &by)
            .text(),
        (AccountEventKind::IdentityUnlinked, _) => plain(Key::AccountEventPlainIdentityUnlinked),
        (
            AccountEventKind::TokenCreated,
            Some(EventDetail::TokenCreated {
                name,
                access,
                repositories,
                until,
            }),
        ) => {
            let reach = if repositories == 0 {
                t.get(Key::AccountEventTokenRepositoriesAll)
            } else {
                t.msg(Key::AccountEventTokenRepositories)
                    .n(u64::from(repositories))
                    .text()
            };
            t.msg(Key::AccountEventTokenCreated)
                .with("name", name)
                .with("scope", scope_word(t, access))
                .with("repositories", reach)
                // The stored date is `YYYY-MM-DD`, the project's own date shape (`timefmt::date`): shown as it is.
                .with("date", until)
                .with("by", &by)
                .text()
        }
        (AccountEventKind::TokenCreated, _) => plain(Key::AccountEventPlainTokenCreated),
        (AccountEventKind::TokenRevoked, Some(EventDetail::TokenRevoked { name })) => t
            .msg(Key::AccountEventTokenRevoked)
            .with("name", name)
            .with("by", &by)
            .text(),
        (AccountEventKind::TokenRevoked, _) => plain(Key::AccountEventPlainTokenRevoked),
        (AccountEventKind::RoleGranted, Some(EventDetail::Role { repository, role })) => t
            .msg(Key::AccountEventRoleGranted)
            .with("repository", repository)
            .with("role", role_word(t, role))
            .with("by", &by)
            .text(),
        (AccountEventKind::RoleGranted, _) => plain(Key::AccountEventPlainRoleGranted),
        (AccountEventKind::RoleRevoked, Some(EventDetail::Role { repository, role })) => t
            .msg(Key::AccountEventRoleRevoked)
            .with("repository", repository)
            .with("role", role_word(t, role))
            .with("by", &by)
            .text(),
        (AccountEventKind::RoleRevoked, _) => plain(Key::AccountEventPlainRoleRevoked),
        (AccountEventKind::Disabled, Some(EventDetail::Reason(reason))) if !reason.is_empty() => t
            .msg(Key::AccountEventDisabled)
            .with("reason", reason)
            .with("by", &by)
            .text(),
        (AccountEventKind::Disabled, _) => plain(Key::AccountEventPlainDisabled),
    }
}
