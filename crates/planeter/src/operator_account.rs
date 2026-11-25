//! `planeter operator account …` (RFC 014 D-3): the account half of the operator's commands. Each is **one `AccountService` call** (one
//! `BEGIN IMMEDIATE` transaction with its event inside it) plus the reads it needs, and **they run beside a running server**: an account
//! is a row of the database, not a repository, so the server's process lock is **not** taken. The server reads accounts, sessions and
//! tokens from the database at every request, so what these commands do is seen on the very next one.
//!
//! A refusal changes nothing. A database that stays busy past the 5-second timeout is said so (exit 5) and nothing is changed. Each
//! success prints **the sentence the account's holder will read in their own log** (it is rendered from the event that was just written),
//! and one line of what it means.
//!
//! Read only what it needs: `PLANETER_DB`, `PLANETER_OIDC_ISSUER` (to check a link names the configured provider) and `PATH` (for `stty`);
//! **no other variable**, and `PLANETER_OIDC_CLIENT_SECRET` never. **No password is ever an argument, an environment variable or a file
//! name** (`password_input.rs`).
//!
//! **Exit codes:** 0 done; 1 refused (nothing changed); 2 the command line (or no terminal for a password); 5 local files or the database.
//! 3 and 4 are not used: nothing needs the server stopped, and one transaction is never half done.

use std::path::Path;
use std::sync::Arc;

use planeter_auth::password::{PasswordError, check_new_password};
use planeter_auth::{
    AccountError, AccountEvent, AccountEventKind, AccountEventStore, AccountService,
    Argon2idHasher, DisplayNameError, EventBy, RepoRole, SqliteAccountEventStore,
    check_display_name,
};
use planeter_core::creation::{HandleError, check_new_handle};
use planeter_store::SqliteDb;
use planeter_web::catalog::{Key, T};

use crate::config;
use crate::operator::{Line, exit, fail};
use crate::password_input::{self, InputError, Source};

/// The longest reason, in characters: the same as `key add`'s.
const REASON_MAX: usize = 200;
/// The most lines `list` prints before it says how many more there are.
const LIST_MAX: usize = 500;
/// The most repositories `disable` names before "and N more".
const SOLE_ADMIN_MAX: usize = 20;
/// The most roles and events `show` prints.
const SHOW_MAX: usize = 50;

/// How a password is to be had, for `create` and `set-password`.
#[derive(Debug, PartialEq, Eq)]
enum Pw {
    None,
    Source(Source),
}

#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Create {
        handle: String,
        name: String,
        pw: Pw,
    },
    SetPassword {
        handle: String,
        source: Source,
    },
    Disable {
        handle: String,
        reason: String,
    },
    Enable {
        handle: String,
    },
    LinkOidc {
        handle: String,
        issuer: String,
        subject: String,
    },
    UnlinkOidc {
        handle: String,
        issuer: String,
        subject: String,
    },
    Grant {
        owner: String,
        name: String,
        handle: String,
        role: RepoRole,
    },
    Revoke {
        owner: String,
        name: String,
        handle: String,
        allow_no_admin: bool,
    },
    List {
        check: bool,
    },
    Show {
        handle: String,
    },
}

/// Why the command line is refused (before anything is read or opened).
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Usage,
    RepoForm,
    Handle(String, HandleError),
    NameMissing,
    NameLong(usize),
    NameUnsafe,
    ReasonMissing,
    ReasonLong(usize),
    ReasonControl,
    Role,
}

fn opt<'a>(line: &'a Line, name: &str) -> Option<&'a str> {
    line.options
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn only(line: &Line, allowed: &[&str]) -> Result<(), Refusal> {
    if line
        .options
        .iter()
        .any(|(n, _)| !allowed.contains(&n.as_str()))
    {
        Err(Refusal::Usage)
    } else {
        Ok(())
    }
}

fn password_fd(line: &Line) -> Result<Option<Source>, Refusal> {
    match opt(line, "password-fd") {
        None => Ok(None),
        Some(n) => n
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 2)
            .map(|n| Some(Source::Fd(n)))
            .ok_or(Refusal::Usage),
    }
}

fn repo_form(text: &str) -> Result<(String, String), Refusal> {
    match text.split_once('/') {
        Some((o, n)) if !o.is_empty() && !n.is_empty() && !n.contains('/') => {
            Ok((o.to_owned(), n.to_owned()))
        }
        _ => Err(Refusal::RepoForm),
    }
}

fn a_handle_word(text: &str) -> Result<String, Refusal> {
    // Existing accounts may have handles the rule for new ones would refuse: only the shape of one word is checked here.
    if text.is_empty() || text.contains('/') {
        Err(Refusal::Usage)
    } else {
        Ok(text.to_owned())
    }
}

/// The command line, or why not. **Pure**: no environment, no file, no terminal.
fn parse(line: &Line) -> Result<Cmd, Refusal> {
    if line.malformed {
        return Err(Refusal::Usage);
    }
    let words: Vec<&str> = line.words.iter().map(String::as_str).collect();
    let get = |n: &str| opt(line, n).map(str::to_owned);
    match words.as_slice() {
        ["account", "create", handle] => {
            only(line, &["name", "password-prompt", "password-fd"])?;
            check_new_handle(handle, &planeter_web::creation::is_reserved)
                .map_err(|e| Refusal::Handle((*handle).to_owned(), e))?;
            let name = get("name").unwrap_or_default();
            if name.trim().is_empty() {
                return Err(Refusal::NameMissing);
            }
            match check_display_name(&name) {
                Ok(_) => {}
                Err(DisplayNameError::Empty) => return Err(Refusal::NameMissing),
                Err(DisplayNameError::TooLong) => {
                    return Err(Refusal::NameLong(name.trim().chars().count()));
                }
                Err(DisplayNameError::Unsafe) => return Err(Refusal::NameUnsafe),
            }
            let prompt = opt(line, "password-prompt").is_some();
            let fd = password_fd(line)?;
            let pw = match (prompt, fd) {
                (false, None) => Pw::None,
                (true, None) => Pw::Source(Source::Terminal),
                (false, Some(s)) => Pw::Source(s),
                (true, Some(_)) => return Err(Refusal::Usage),
            };
            Ok(Cmd::Create {
                handle: (*handle).to_owned(),
                name,
                pw,
            })
        }
        ["account", "set-password", handle] => {
            only(line, &["password-fd"])?;
            Ok(Cmd::SetPassword {
                handle: a_handle_word(handle)?,
                source: password_fd(line)?.unwrap_or(Source::Terminal),
            })
        }
        ["account", "disable", handle] => {
            only(line, &["reason"])?;
            let reason = get("reason").unwrap_or_default();
            if reason.trim().is_empty() {
                return Err(Refusal::ReasonMissing);
            }
            if reason.chars().any(char::is_control) {
                return Err(Refusal::ReasonControl);
            }
            let n = reason.chars().count();
            if n > REASON_MAX {
                return Err(Refusal::ReasonLong(n));
            }
            Ok(Cmd::Disable {
                handle: a_handle_word(handle)?,
                reason,
            })
        }
        ["account", "enable", handle] => {
            only(line, &[])?;
            Ok(Cmd::Enable {
                handle: a_handle_word(handle)?,
            })
        }
        ["account", verb @ ("link-oidc" | "unlink-oidc"), handle] => {
            only(line, &["issuer", "subject"])?;
            let (Some(issuer), Some(subject)) = (get("issuer"), get("subject")) else {
                return Err(Refusal::Usage);
            };
            if issuer.is_empty() || subject.is_empty() {
                return Err(Refusal::Usage);
            }
            let handle = a_handle_word(handle)?;
            Ok(if *verb == "link-oidc" {
                Cmd::LinkOidc {
                    handle,
                    issuer,
                    subject,
                }
            } else {
                Cmd::UnlinkOidc {
                    handle,
                    issuer,
                    subject,
                }
            })
        }
        ["account", "grant", repo, handle] => {
            only(line, &["role"])?;
            let (owner, name) = repo_form(repo)?;
            let role = get("role").ok_or(Refusal::Usage)?;
            let role = RepoRole::parse(&role).ok_or(Refusal::Role)?;
            Ok(Cmd::Grant {
                owner,
                name,
                handle: a_handle_word(handle)?,
                role,
            })
        }
        ["account", "revoke", repo, handle] => {
            only(line, &["allow-no-admin"])?;
            let (owner, name) = repo_form(repo)?;
            Ok(Cmd::Revoke {
                owner,
                name,
                handle: a_handle_word(handle)?,
                allow_no_admin: opt(line, "allow-no-admin").is_some(),
            })
        }
        ["account", "list"] => {
            only(line, &["check"])?;
            Ok(Cmd::List {
                check: opt(line, "check").is_some(),
            })
        }
        ["account", "show", handle] => {
            only(line, &[])?;
            Ok(Cmd::Show {
                handle: a_handle_word(handle)?,
            })
        }
        _ => Err(Refusal::Usage),
    }
}

fn account_help(t: &T) -> String {
    format!(
        "{}\n\n{}\n\n{}\n\n{}\n",
        t.get(Key::OperatorAccountHelpUsage),
        t.get(Key::OperatorAccountHelpRoles),
        t.get(Key::OperatorAccountHelpPassword),
        t.get(Key::OperatorAccountHelpExitCodes)
    )
}

/// Run `planeter operator account …`; the exit code.
pub(crate) fn run(t: &T, line: &Line) -> i32 {
    if line.help {
        print!("{}", account_help(t));
        return exit::DONE;
    }
    match parse(line) {
        Ok(cmd) => execute(t, cmd),
        Err(refusal) => {
            let msg = match &refusal {
                Refusal::Usage => t.msg(Key::OperatorAccountErrorUsage),
                Refusal::RepoForm => t.msg(Key::OperatorErrorRepoForm),
                Refusal::Handle(h, e) => match e {
                    HandleError::Uppercase => t
                        .msg(Key::OperatorAccountErrorHandleUpper)
                        .with("handle", h),
                    HandleError::Reserved => t
                        .msg(Key::OperatorAccountErrorHandleReserved)
                        .with("handle", h),
                    _ => t
                        .msg(Key::OperatorAccountErrorHandleShape)
                        .with("handle", h),
                },
                Refusal::NameMissing => t
                    .msg(Key::OperatorAccountErrorNameMissing)
                    .with("max", planeter_auth::DISPLAY_NAME_MAX),
                Refusal::NameLong(n) => t
                    .msg(Key::OperatorAccountErrorNameLong)
                    .with("length", n)
                    .with("max", planeter_auth::DISPLAY_NAME_MAX),
                Refusal::NameUnsafe => t.msg(Key::OperatorAccountErrorNameUnsafe),
                Refusal::ReasonMissing => t
                    .msg(Key::OperatorAccountErrorReasonMissing)
                    .with("max", REASON_MAX),
                Refusal::ReasonLong(n) => t
                    .msg(Key::OperatorErrorReasonLong)
                    .with("length", n)
                    .with("max", REASON_MAX),
                Refusal::ReasonControl => t.msg(Key::OperatorErrorReasonControl),
                Refusal::Role => t.msg(Key::OperatorAccountErrorRole),
            };
            eprintln!("planeter operator: {}", msg.text());
            if refusal == Refusal::Usage {
                eprint!("{}", account_help(t));
            }
            exit::COMMAND_LINE
        }
    }
}

/// What the commands share once the database is open.
struct Ctx<'a> {
    t: &'a T,
    service: AccountService,
    events: SqliteAccountEventStore,
}

/// A backend failure, said: a database that stayed busy is its own sentence.
pub(crate) fn backend(t: &T, why: &str) -> i32 {
    let lower = why.to_lowercase();
    if lower.contains("database is locked") || lower.contains("busy") {
        fail(
            t.msg(Key::OperatorAccountErrorBusy).with("seconds", 5),
            exit::LOCAL,
        )
    } else {
        fail(
            t.msg(Key::OperatorErrorDatabase).with("why", why),
            exit::LOCAL,
        )
    }
}

/// The words of a failed account write: the database's own message for a backend failure (a busy database is known by it).
pub(crate) fn why(e: &AccountError) -> String {
    match e {
        AccountError::Backend(s) => s.clone(),
        other => other.to_string(),
    }
}

fn open(t: &T) -> Result<Ctx<'_>, i32> {
    let paths = config::paths_from_env();
    // The database must be there: it is never made here (the server makes it).
    if !Path::new(&paths.db_path).is_file() {
        return Err(fail(
            t.msg(Key::OperatorErrorNoDatabase)
                .with("path", &paths.db_path),
            exit::LOCAL,
        ));
    }
    let db = SqliteDb::open(&paths.db_path).map_err(|e| backend(t, &e.to_string()))?;
    let service = AccountService::new(
        db.clone(),
        Arc::new(Argon2idHasher),
        Arc::new(planeter_web::creation::is_reserved),
    )
    .map_err(|e| backend(t, &why(&e)))?;
    let events = SqliteAccountEventStore::new(db).map_err(|e| backend(t, &e.to_string()))?;
    Ok(Ctx { t, service, events })
}

fn by_word(t: &T, by: EventBy) -> String {
    t.get(match by {
        EventBy::SelfService => Key::AccountBySelf,
        EventBy::Operator => Key::AccountByOperator,
        EventBy::Provider => Key::AccountByProvider,
    })
}

/// The sentence of one event, **the one the account's holder reads in their own log** (RFC 014 D-7), in the language of `t`.
pub(crate) fn event_sentence(t: &T, e: &AccountEvent) -> String {
    let key = match e.kind {
        AccountEventKind::Created => Key::AccountEventCreated,
        AccountEventKind::PasswordSet => Key::AccountEventPasswordSet,
        AccountEventKind::PasswordChanged => Key::AccountEventPasswordChanged,
        AccountEventKind::IdentityLinked => Key::AccountEventIdentityLinked,
        AccountEventKind::IdentityUnlinked => Key::AccountEventIdentityUnlinked,
        AccountEventKind::TokenCreated => Key::AccountEventTokenCreated,
        AccountEventKind::TokenRevoked => Key::AccountEventTokenRevoked,
        AccountEventKind::SessionEnded => Key::AccountEventSessionEnded,
        AccountEventKind::RoleGranted => Key::AccountEventRoleGranted,
        AccountEventKind::RoleRevoked => Key::AccountEventRoleRevoked,
        AccountEventKind::Disabled => Key::AccountEventDisabled,
        AccountEventKind::Enabled => Key::AccountEventEnabled,
        AccountEventKind::SignedInNewMethod => Key::AccountEventSignedInNewMethod,
    };
    t.msg(key)
        .with("by", by_word(t, e.by))
        .with("detail", &e.detail)
        .text()
}

impl Ctx<'_> {
    /// Print the sentence of the newest event of `handle`: the act that has just been written.
    fn say_latest(&self, handle: &str) {
        if let Ok(mut recent) = self.events.recent(handle, 1)
            && let Some(e) = recent.pop()
        {
            println!("{}", event_sentence(self.t, &e));
        }
    }

    /// Print the sentences of the newest `n` events of `handle`, oldest first.
    fn say_recent(&self, handle: &str, n: usize) {
        if let Ok(mut recent) = self.events.recent(handle, n) {
            recent.reverse();
            for e in recent {
                println!("{}", event_sentence(self.t, &e));
            }
        }
    }

    /// A link names the sign-in provider this server is set up for (`PLANETER_OIDC_ISSUER`, read for this and nothing else).
    fn check_issuer(&self, issuer: &str) -> Result<(), i32> {
        let configured = std::env::var("PLANETER_OIDC_ISSUER").unwrap_or_default();
        if configured.trim().is_empty() {
            return Err(fail(
                self.t.msg(Key::OperatorAccountErrorNoProvider),
                exit::REFUSED,
            ));
        }
        if configured.trim_end_matches('/') != issuer.trim_end_matches('/') {
            return Err(fail(
                self.t
                    .msg(Key::OperatorAccountErrorOtherIssuer)
                    .with("issuer", issuer),
                exit::REFUSED,
            ));
        }
        Ok(())
    }

    fn no_account(&self, handle: &str) -> i32 {
        fail(
            self.t
                .msg(Key::OperatorAccountErrorNoAccount)
                .with("handle", handle),
            exit::REFUSED,
        )
    }

    /// An error of the service as a sentence and a code.
    fn refuse(&self, e: &AccountError, handle: &str, repo: &str) -> i32 {
        let t = self.t;
        match e {
            AccountError::Handle(h) => {
                let msg = match h {
                    HandleError::Uppercase => t.msg(Key::OperatorAccountErrorHandleUpper),
                    HandleError::Reserved => t.msg(Key::OperatorAccountErrorHandleReserved),
                    _ => t.msg(Key::OperatorAccountErrorHandleShape),
                };
                fail(msg.with("handle", handle), exit::COMMAND_LINE)
            }
            AccountError::HandleTaken => fail(
                t.msg(Key::OperatorAccountErrorHandleTaken)
                    .with("handle", handle),
                exit::REFUSED,
            ),
            AccountError::DisplayName(DisplayNameError::Unsafe) => fail(
                t.msg(Key::OperatorAccountErrorNameUnsafe),
                exit::COMMAND_LINE,
            ),
            AccountError::DisplayName(_) => fail(
                t.msg(Key::OperatorAccountErrorNameMissing)
                    .with("max", planeter_auth::DISPLAY_NAME_MAX),
                exit::COMMAND_LINE,
            ),
            AccountError::NoSuchAccount => self.no_account(handle),
            AccountError::IdentityTaken => {
                fail(t.msg(Key::OperatorAccountErrorIdentityTaken), exit::REFUSED)
            }
            AccountError::NotLinked => fail(
                t.msg(Key::OperatorAccountErrorNotLinked)
                    .with("handle", handle),
                exit::REFUSED,
            ),
            AccountError::NoSuchRepository => fail(
                t.msg(Key::OperatorErrorUnknownRepo).with("repo", repo),
                exit::REFUSED,
            ),
            AccountError::AccountDisabled => fail(
                t.msg(Key::OperatorAccountErrorAccountDisabled)
                    .with("handle", handle),
                exit::REFUSED,
            ),
            AccountError::LastAdministrator => fail(
                t.msg(Key::OperatorAccountErrorLastAdmin).with("repo", repo),
                exit::REFUSED,
            ),
            AccountError::NoGrant => fail(
                t.msg(Key::OperatorAccountErrorNoGrant)
                    .with("handle", handle)
                    .with("repo", repo),
                exit::REFUSED,
            ),
            AccountError::Backend(why) => backend(t, why),
            // The token commands' errors (`operator_token.rs`): the account commands never make them.
            other @ (AccountError::Token(_)
            | AccountError::NoSuchToken
            | AccountError::RepositoryUnknown(_)) => backend(t, &other.to_string()),
        }
    }

    /// Whether the account exists, and whether it is disabled; `Err` is the exit code of a failure already said.
    fn lookup(&self, handle: &str) -> Result<Option<planeter_auth::AccountDetail>, i32> {
        self.service
            .show_account(handle, 0)
            .map_err(|e| self.refuse(&e, handle, ""))
    }
}

/// The password a person types or a script gives, checked by the rule of D-4. `Err` is the exit code of a failure already said.
fn new_password(
    t: &T,
    source: Source,
    handle: &str,
) -> Result<planeter_auth::password::NormalizedPassword, i32> {
    let text = match password_input::read_new(
        source,
        (
            &format!("{} ", t.get(Key::OperatorAccountPromptPassword)),
            &format!("{} ", t.get(Key::OperatorAccountPromptAgain)),
        ),
    ) {
        Ok(p) => p,
        Err(InputError::NoTerminal) => {
            return Err(fail(
                t.msg(Key::OperatorAccountErrorPasswordNoTerminal),
                exit::COMMAND_LINE,
            ));
        }
        Err(InputError::Echo(why)) => {
            return Err(fail(
                t.msg(Key::OperatorAccountErrorPasswordEcho)
                    .with("why", why),
                exit::LOCAL,
            ));
        }
        Err(InputError::Fd { fd, why }) => {
            return Err(fail(
                t.msg(Key::OperatorAccountErrorPasswordFd)
                    .with("fd", fd)
                    .with("why", why),
                exit::COMMAND_LINE,
            ));
        }
        Err(InputError::Differ) => {
            return Err(fail(
                t.msg(Key::OperatorAccountErrorPasswordDiffer),
                exit::REFUSED,
            ));
        }
    };
    check_new_password(&text, handle).map_err(|e| {
        // The error names the rule, never the password.
        let msg = match e {
            PasswordError::TooShort => t
                .msg(Key::OperatorAccountErrorPasswordShort)
                .with("min", planeter_auth::password::PASSWORD_MIN),
            PasswordError::TooLong => t
                .msg(Key::OperatorAccountErrorPasswordLong)
                .with("max", planeter_auth::password::PASSWORD_MAX),
            PasswordError::IsHandle => t.msg(Key::OperatorAccountErrorPasswordHandle),
            PasswordError::TooCommon => t.msg(Key::OperatorAccountErrorPasswordCommon),
        };
        fail(msg, exit::REFUSED)
    })
}

fn execute(t: &T, cmd: Cmd) -> i32 {
    let ctx = match open(t) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let now = config::unix_now();
    match cmd {
        Cmd::Create { handle, name, pw } => {
            let password = match pw {
                Pw::None => None,
                Pw::Source(source) => match new_password(t, source, &handle) {
                    Ok(p) => Some(p),
                    Err(code) => return code,
                },
            };
            if let Err(e) = ctx.service.create_account(
                &handle,
                &name,
                password.as_ref(),
                EventBy::Operator,
                now,
            ) {
                return ctx.refuse(&e, &handle, "");
            }
            // What the holder will read: the account was created, and (with a password) that a password was set.
            ctx.say_recent(&handle, if password.is_some() { 2 } else { 1 });
            let shown = name.trim();
            let line = if password.is_some() {
                t.msg(Key::OperatorAccountDoneHasPassword)
            } else {
                t.msg(Key::OperatorAccountDoneNoPassword)
            };
            println!(
                "{}",
                line.with("handle", &handle).with("name", shown).text()
            );
            exit::DONE
        }
        Cmd::SetPassword { handle, source } => {
            match ctx.lookup(&handle) {
                Err(code) => return code,
                Ok(None) => return ctx.no_account(&handle),
                Ok(Some(_)) => {}
            }
            let password = match new_password(t, source, &handle) {
                Ok(p) => p,
                Err(code) => return code,
            };
            if let Err(e) = ctx
                .service
                .set_password(&handle, &password, EventBy::Operator, now)
            {
                return ctx.refuse(&e, &handle, "");
            }
            ctx.say_latest(&handle);
            println!("{}", t.get(Key::OperatorAccountDoneSessionsEnded));
            exit::DONE
        }
        Cmd::Disable { handle, reason } => {
            let detail = match ctx.lookup(&handle) {
                Err(code) => return code,
                Ok(None) => return ctx.no_account(&handle),
                Ok(Some(d)) => d,
            };
            if detail.row.disabled {
                return fail(
                    t.msg(Key::OperatorAccountErrorAlreadyDisabled)
                        .with("handle", &handle),
                    exit::REFUSED,
                );
            }
            // Before it acts: the repositories this leaves without an administrator (a bounded list).
            match ctx.service.sole_administrator_of(&handle, SOLE_ADMIN_MAX) {
                Ok(s) if s.total > 0 => {
                    let repos = s.repos.join(", ");
                    let msg = if s.total > s.repos.len() {
                        t.msg(Key::OperatorAccountWarnSoleAdminMore)
                            .with("repos", repos)
                            .with("more", s.total - s.repos.len())
                    } else {
                        t.msg(Key::OperatorAccountWarnSoleAdmin)
                            .with("repos", repos)
                    };
                    println!("{}", msg.text());
                }
                Ok(_) => {}
                Err(e) => return ctx.refuse(&e, &handle, ""),
            }
            match ctx
                .service
                .disable(&handle, &reason, EventBy::Operator, now)
            {
                Ok(true) => {}
                Ok(false) => {
                    return fail(
                        t.msg(Key::OperatorAccountErrorAlreadyDisabled)
                            .with("handle", &handle),
                        exit::REFUSED,
                    );
                }
                Err(e) => return ctx.refuse(&e, &handle, ""),
            }
            ctx.say_latest(&handle);
            println!("{}", t.get(Key::OperatorAccountDoneDisabled));
            exit::DONE
        }
        Cmd::Enable { handle } => {
            let detail = match ctx.lookup(&handle) {
                Err(code) => return code,
                Ok(None) => return ctx.no_account(&handle),
                Ok(Some(d)) => d,
            };
            if !detail.row.disabled {
                return fail(
                    t.msg(Key::OperatorAccountErrorNotDisabled)
                        .with("handle", &handle),
                    exit::REFUSED,
                );
            }
            match ctx.service.enable(&handle, EventBy::Operator, now) {
                Ok(true) => {}
                Ok(false) => {
                    return fail(
                        t.msg(Key::OperatorAccountErrorNotDisabled)
                            .with("handle", &handle),
                        exit::REFUSED,
                    );
                }
                Err(e) => return ctx.refuse(&e, &handle, ""),
            }
            ctx.say_latest(&handle);
            println!("{}", t.get(Key::OperatorAccountDoneEnabled));
            exit::DONE
        }
        Cmd::LinkOidc {
            handle,
            issuer,
            subject,
        } => {
            if let Err(code) = ctx.check_issuer(&issuer) {
                return code;
            }
            match ctx
                .service
                .link_oidc(&handle, &issuer, &subject, EventBy::Operator, now)
            {
                Ok(true) => {
                    ctx.say_latest(&handle);
                    println!(
                        "{}",
                        t.msg(Key::OperatorAccountDoneLinked)
                            .with("handle", &handle)
                            .text()
                    );
                    exit::DONE
                }
                Ok(false) => fail(
                    t.msg(Key::OperatorAccountErrorAlreadyLinked)
                        .with("handle", &handle),
                    exit::REFUSED,
                ),
                Err(e) => ctx.refuse(&e, &handle, ""),
            }
        }
        Cmd::UnlinkOidc {
            handle,
            issuer,
            subject,
        } => {
            if let Err(code) = ctx.check_issuer(&issuer) {
                return code;
            }
            match ctx
                .service
                .unlink_oidc(&handle, &issuer, &subject, EventBy::Operator, now)
            {
                Ok(()) => {
                    ctx.say_latest(&handle);
                    println!(
                        "{}",
                        t.msg(Key::OperatorAccountDoneUnlinked)
                            .with("handle", &handle)
                            .text()
                    );
                    exit::DONE
                }
                Err(e) => ctx.refuse(&e, &handle, ""),
            }
        }
        Cmd::Grant {
            owner,
            name,
            handle,
            role,
        } => {
            let repo = format!("{owner}/{name}");
            match ctx
                .service
                .grant_role(&handle, &owner, &name, role, EventBy::Operator, now)
            {
                Ok(g) if g.previous == Some(role) => fail(
                    t.msg(Key::OperatorAccountErrorAlreadyRole)
                        .with("handle", &handle)
                        .with("role", role.as_str())
                        .with("repo", &repo),
                    exit::REFUSED,
                ),
                Ok(g) => {
                    ctx.say_latest(&handle);
                    let means = t.get(match role {
                        RepoRole::Read => Key::OperatorAccountRoleRead,
                        RepoRole::Write => Key::OperatorAccountRoleWrite,
                        RepoRole::Maintain => Key::OperatorAccountRoleMaintain,
                        RepoRole::Admin => Key::OperatorAccountRoleAdmin,
                    });
                    println!(
                        "{}",
                        t.msg(Key::OperatorAccountDoneGranted)
                            .with("name", g.display_name)
                            .with("handle", &handle)
                            .with("role", role.as_str())
                            .with("repo", &repo)
                            .with("means", means)
                            .text()
                    );
                    exit::DONE
                }
                Err(e) => ctx.refuse(&e, &handle, &repo),
            }
        }
        Cmd::Revoke {
            owner,
            name,
            handle,
            allow_no_admin,
        } => {
            let repo = format!("{owner}/{name}");
            match ctx.service.revoke_role(
                &handle,
                &owner,
                &name,
                allow_no_admin,
                EventBy::Operator,
                now,
            ) {
                Ok(r) => {
                    ctx.say_latest(&handle);
                    println!(
                        "{}",
                        t.msg(Key::OperatorAccountDoneRevoked)
                            .with("name", r.display_name)
                            .with("handle", &handle)
                            .with("role", r.was.as_str())
                            .with("repo", &repo)
                            .text()
                    );
                    if r.was == RepoRole::Admin
                        && matches!(
                            ctx.service.repo_has_administrator(&owner, &name),
                            Ok(Some(false))
                        )
                    {
                        println!(
                            "{}",
                            t.msg(Key::OperatorAccountDoneNoAdmin)
                                .with("repo", &repo)
                                .text()
                        );
                    }
                    exit::DONE
                }
                Err(e) => ctx.refuse(&e, &handle, &repo),
            }
        }
        Cmd::List { check } => list(&ctx, check),
        Cmd::Show { handle } => show(&ctx, &handle),
    }
}

fn yes_no(t: &T, v: bool) -> String {
    t.get(if v {
        Key::OperatorAccountListYes
    } else {
        Key::OperatorAccountListNo
    })
}

fn state_word(t: &T, disabled: bool) -> String {
    t.get(if disabled {
        Key::OperatorAccountListDisabled
    } else {
        Key::OperatorAccountListActive
    })
}

fn date_or_dash(at: Option<u64>) -> String {
    at.map_or_else(|| "-".to_owned(), planeter_web::timefmt::date)
}

/// `account list` and `account list --check`: read only; **no hash and no secret is ever printed**; at most [`LIST_MAX`] lines.
fn list(ctx: &Ctx<'_>, check: bool) -> i32 {
    let t = ctx.t;
    if check {
        let groups = match ctx.service.collisions() {
            Ok(g) => g,
            Err(e) => return ctx.refuse(&e, "", ""),
        };
        if groups.is_empty() {
            println!("{}", t.get(Key::OperatorAccountCheckNone));
            return exit::DONE;
        }
        println!("{}", t.get(Key::OperatorAccountCheckExplain));
        let total = groups.len();
        for (key, handles) in groups.iter().take(LIST_MAX) {
            println!(
                "{}",
                t.msg(Key::OperatorAccountCheckGroup)
                    .with("key", key)
                    .with("handles", handles.join(", "))
                    .text()
            );
        }
        if total > LIST_MAX {
            println!(
                "{}",
                t.msg(Key::OperatorAccountCheckMore)
                    .with("more", total - LIST_MAX)
                    .with("total", total)
                    .text()
            );
        }
        // It is a report: it exits 0 whether or not there is anything in it.
        return exit::DONE;
    }
    let list = match ctx.service.list_accounts(LIST_MAX) {
        Ok(l) => l,
        Err(e) => return ctx.refuse(&e, "", ""),
    };
    if list.total == 0 {
        println!("{}", t.get(Key::OperatorAccountListNone));
        return exit::DONE;
    }
    println!("{}", t.get(Key::OperatorAccountListHeader));
    for r in &list.rows {
        println!(
            "{} | {} | {} | {} | {} | {}",
            r.handle,
            r.display_name,
            state_word(t, r.disabled),
            yes_no(t, r.has_password),
            yes_no(t, r.identities > 0),
            date_or_dash(r.created_at)
        );
    }
    if list.total > list.rows.len() {
        println!(
            "{}",
            t.msg(Key::OperatorAccountListMore)
                .with("more", list.total - list.rows.len())
                .with("total", list.total)
                .text()
        );
    }
    exit::DONE
}

/// `account show HANDLE`: one account, its roles (at most [`SHOW_MAX`]) and its last [`SHOW_MAX`] events.
fn show(ctx: &Ctx<'_>, handle: &str) -> i32 {
    let t = ctx.t;
    let d = match ctx.service.show_account(handle, SHOW_MAX) {
        Ok(Some(d)) => d,
        Ok(None) => return ctx.no_account(handle),
        Err(e) => return ctx.refuse(&e, handle, ""),
    };
    let line = |label: Key, value: String| println!("{}: {}", t.get(label), value);
    line(Key::OperatorAccountShowHandle, d.row.handle.clone());
    line(Key::OperatorAccountShowName, d.row.display_name.clone());
    let mut state = state_word(t, d.row.disabled);
    if let Some((at, reason)) = &d.disabled {
        state = format!(
            "{state} ({})",
            t.msg(Key::OperatorAccountShowDisabled)
                .with("date", planeter_web::timefmt::date(*at))
                .with("reason", reason)
                .text()
        );
    }
    line(Key::OperatorAccountShowState, state);
    line(
        Key::OperatorAccountShowCreated,
        date_or_dash(d.row.created_at),
    );
    let password = if d.row.has_password {
        match d.password_changed_at {
            Some(at) => t
                .msg(Key::OperatorAccountShowPasswordChanged)
                .with("date", planeter_web::timefmt::date(at))
                .text(),
            None => yes_no(t, true),
        }
    } else {
        yes_no(t, false)
    };
    line(Key::OperatorAccountShowPassword, password);
    line(
        Key::OperatorAccountShowIdentities,
        d.row.identities.to_string(),
    );
    if d.roles.is_empty() {
        line(
            Key::OperatorAccountShowRoles,
            t.get(Key::OperatorAccountShowRolesNone),
        );
    } else {
        println!("{}:", t.get(Key::OperatorAccountShowRoles));
        for (repo, role) in &d.roles {
            let word = if role == "owner" {
                t.get(Key::OperatorAccountShowOwner)
            } else {
                role.clone()
            };
            println!("  {repo}  {word}");
        }
        if d.roles_total > d.roles.len() {
            println!(
                "  {}",
                t.msg(Key::OperatorAccountShowRolesMore)
                    .with("more", d.roles_total - d.roles.len())
                    .with("total", d.roles_total)
                    .text()
            );
        }
    }
    match ctx.events.recent(handle, SHOW_MAX) {
        Ok(events) if !events.is_empty() => {
            println!("{}:", t.get(Key::OperatorAccountShowEvents));
            for e in events {
                println!(
                    "  {}",
                    t.msg(Key::OperatorAccountShowEventLine)
                        .with("date", planeter_web::timefmt::date(e.at))
                        .with("sentence", event_sentence(t, &e))
                        .text()
                );
            }
        }
        Ok(_) => line(
            Key::OperatorAccountShowEvents,
            t.get(Key::OperatorAccountShowEventsNone),
        ),
        Err(e) => return backend(t, &e.to_string()),
    }
    exit::DONE
}
