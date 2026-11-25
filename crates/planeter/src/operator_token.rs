//! `planeter operator token …` (RFC 014 D-3): the operator makes, lists and revokes an account's access tokens. **This is the bootstrap**
//! of a deployment with no settings pages yet: `planeter-client` needs a token and nothing else makes one. They run **beside a running
//! server**, as the account commands do (no process lock; one `BEGIN IMMEDIATE` transaction each; the server reads tokens from the
//! database at every request, so what these do is seen on the very next one).
//!
//! **The token is shown once and exists nowhere else but as its hash.** It is minted here (`mint_token`) and only its hash goes to the
//! service (`AccountService::create_token_for`), which never sees the secret. By default it is printed on **one line of standard output
//! and nothing else goes to standard output** (so `TOKEN=$(planeter operator token create …)` works); everything the person should know
//! goes to standard error. With `--out FILE` it is written to a **new file** (created exclusively with mode 0600 from the moment it
//! exists, never overwritten, no symlink followed) and **not printed**.
//!
//! **Order and failure.** The file is written **first**, then the hash is stored; if storing fails the file is removed again, and if the
//! file cannot be written nothing is stored. A kill between the two leaves a file holding a token that was never stored (it opens
//! nothing). The other order (store, then write) would leave a stored token nobody has the secret of: that is the worse leftover.
//!
//! Reads only `PLANETER_DB` (and `LC_ALL`/`LC_MESSAGES`/`LANG`/`--lang`): never `PLANETER_REPOS_ROOT` (a repository named in `--repos` is
//! looked up in the database's repository table, as `account grant` does) and never a secret of the server.
//!
//! **Exit codes:** 0 done; 1 refused (nothing changed); 2 the command line; 5 local files or the database. 3 and 4 are not used.

use std::path::Path;
use std::sync::Arc;

use planeter_auth::{
    AccountError, AccountEventStore, AccountService, Argon2idHasher, CredentialStore, EventBy,
    Sha256TokenHasher, SqliteAccountEventStore, SqliteCredentialStore, TOKEN_NAME_MAX, TokenError,
    TokenHasher, TokenSpec, mint_token,
};
use planeter_core::{RepoScope, ScopeAccess};
use planeter_store::{RepoId, RepositoryStore, SqliteDb, SqliteRepositoryStore, UserId};
use planeter_web::catalog::{Key, T};

use crate::config;
use crate::operator::{Line, exit, fail};
use crate::operator_account::{backend, why};

/// How long a token lasts when `--expires-days` is not given.
const DEFAULT_DAYS: u64 = 90;
/// The most days a token may last.
const MAX_DAYS: u64 = 365;
/// The most repositories a token may be limited to.
const REPOS_MAX: usize = 20;

#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Create {
        handle: String,
        name: String,
        access: ScopeAccess,
        days: u64,
        repos: Vec<(String, String)>,
        out: Option<String>,
    },
    List {
        handle: String,
    },
    Revoke {
        handle: String,
        id: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Usage,
    RepoForm,
    NameMissing,
    NameLong(usize),
    NameUnsafe,
    ScopeMissing,
    ScopeUnknown,
    Days,
    ReposTooMany,
    ReposEmpty,
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

fn a_word(text: &str) -> Result<String, Refusal> {
    if text.is_empty() || text.contains('/') {
        Err(Refusal::Usage)
    } else {
        Ok(text.to_owned())
    }
}

/// The command line, or why not. **Pure**: no environment, no file.
fn parse(line: &Line) -> Result<Cmd, Refusal> {
    if line.malformed {
        return Err(Refusal::Usage);
    }
    let words: Vec<&str> = line.words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["token", "create", handle] => {
            only(line, &["name", "scope", "expires-days", "repos", "out"])?;
            let name = opt(line, "name").unwrap_or_default();
            if name.trim().is_empty() {
                return Err(Refusal::NameMissing);
            }
            let n = name.chars().count();
            if n > TOKEN_NAME_MAX {
                return Err(Refusal::NameLong(n));
            }
            if !planeter_auth::text::is_display_safe(name) {
                return Err(Refusal::NameUnsafe);
            }
            let access = match opt(line, "scope") {
                None => return Err(Refusal::ScopeMissing),
                Some("read-only") => ScopeAccess::ReadOnly,
                Some("write") => ScopeAccess::Write,
                Some("full") => ScopeAccess::Full,
                Some(_) => return Err(Refusal::ScopeUnknown),
            };
            let days = match opt(line, "expires-days") {
                None => DEFAULT_DAYS,
                Some(d) => d
                    .parse::<u64>()
                    .ok()
                    .filter(|d| (1..=MAX_DAYS).contains(d))
                    .ok_or(Refusal::Days)?,
            };
            let mut repos: Vec<(String, String)> = Vec::new();
            if let Some(list) = opt(line, "repos") {
                for item in list.split(',') {
                    let (o, r) = match item.split_once('/') {
                        Some((o, r)) if !o.is_empty() && !r.is_empty() && !r.contains('/') => {
                            (o.to_owned(), r.to_owned())
                        }
                        _ if item.is_empty() => return Err(Refusal::ReposEmpty),
                        _ => return Err(Refusal::RepoForm),
                    };
                    if !repos.contains(&(o.clone(), r.clone())) {
                        repos.push((o, r));
                    }
                }
                if repos.len() > REPOS_MAX {
                    return Err(Refusal::ReposTooMany);
                }
            }
            let out = match opt(line, "out") {
                Some("") => return Err(Refusal::Usage),
                other => other.map(str::to_owned),
            };
            Ok(Cmd::Create {
                handle: a_word(handle)?,
                name: name.to_owned(),
                access,
                days,
                repos,
                out,
            })
        }
        ["token", "list", handle] => {
            only(line, &[])?;
            Ok(Cmd::List {
                handle: a_word(handle)?,
            })
        }
        ["token", "revoke", handle, id] => {
            only(line, &[])?;
            Ok(Cmd::Revoke {
                handle: a_word(handle)?,
                id: (*id).to_owned(),
            })
        }
        _ => Err(Refusal::Usage),
    }
}

fn token_help(t: &T) -> String {
    format!(
        "{}\n\n{}\n\n{}\n",
        t.get(Key::OperatorTokenHelpUsage),
        t.get(Key::OperatorTokenHelpOutput),
        t.get(Key::OperatorAccountHelpExitCodes)
    )
}

/// Run `planeter operator token …`; the exit code.
pub(crate) fn run(t: &T, line: &Line) -> i32 {
    if line.help {
        print!("{}", token_help(t));
        return exit::DONE;
    }
    match parse(line) {
        Ok(cmd) => execute(t, cmd),
        Err(refusal) => {
            let msg = match &refusal {
                Refusal::Usage => t.msg(Key::OperatorTokenErrorUsage),
                Refusal::RepoForm => t.msg(Key::OperatorErrorRepoForm),
                Refusal::NameMissing => t
                    .msg(Key::OperatorTokenErrorNameMissing)
                    .with("max", TOKEN_NAME_MAX),
                Refusal::NameLong(n) => t
                    .msg(Key::OperatorTokenErrorNameLong)
                    .with("length", n)
                    .with("max", TOKEN_NAME_MAX),
                Refusal::NameUnsafe => t.msg(Key::OperatorTokenErrorNameUnsafe),
                Refusal::ScopeMissing => t.msg(Key::OperatorTokenErrorScopeMissing),
                Refusal::ScopeUnknown => t.msg(Key::OperatorTokenErrorScopeUnknown),
                Refusal::Days => t.msg(Key::OperatorTokenErrorDays).with("max", MAX_DAYS),
                Refusal::ReposTooMany => t
                    .msg(Key::OperatorTokenErrorReposTooMany)
                    .with("max", REPOS_MAX),
                Refusal::ReposEmpty => t.msg(Key::OperatorTokenErrorReposEmpty),
            };
            eprintln!("planeter operator: {}", msg.text());
            if refusal == Refusal::Usage {
                eprint!("{}", token_help(t));
            }
            exit::COMMAND_LINE
        }
    }
}

struct Ctx<'a> {
    t: &'a T,
    service: AccountService,
    creds: SqliteCredentialStore,
    repos: SqliteRepositoryStore,
    events: SqliteAccountEventStore,
}

fn open(t: &T) -> Result<Ctx<'_>, i32> {
    let paths = config::paths_from_env();
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
    Ok(Ctx {
        t,
        service,
        creds: SqliteCredentialStore::new(db.clone()).map_err(|e| backend(t, &e.to_string()))?,
        repos: SqliteRepositoryStore::new(db.clone()),
        events: SqliteAccountEventStore::new(db).map_err(|e| backend(t, &e.to_string()))?,
    })
}

impl Ctx<'_> {
    fn no_account(&self, handle: &str) -> i32 {
        fail(
            self.t
                .msg(Key::OperatorAccountErrorNoAccount)
                .with("handle", handle),
            exit::REFUSED,
        )
    }

    /// An error of the service as a sentence and a code.
    fn refuse(&self, e: &AccountError, handle: &str, id: &str) -> i32 {
        let t = self.t;
        match e {
            AccountError::NoSuchAccount => self.no_account(handle),
            AccountError::AccountDisabled => fail(
                t.msg(Key::OperatorTokenErrorAccountDisabled)
                    .with("handle", handle),
                exit::REFUSED,
            ),
            AccountError::NoSuchToken => fail(
                t.msg(Key::OperatorTokenErrorNoToken)
                    .with("handle", handle)
                    .with("id", id),
                exit::REFUSED,
            ),
            AccountError::RepositoryUnknown(repo) => fail(
                t.msg(Key::OperatorErrorUnknownRepo).with("repo", repo),
                exit::REFUSED,
            ),
            AccountError::Token(TokenError::TooMany) => fail(
                t.msg(Key::OperatorTokenErrorTooManyTokens)
                    .with("handle", handle)
                    .with("max", planeter_auth::TOKENS_MAX),
                exit::REFUSED,
            ),
            AccountError::Token(TokenError::NameEmpty) => fail(
                t.msg(Key::OperatorTokenErrorNameMissing)
                    .with("max", TOKEN_NAME_MAX),
                exit::COMMAND_LINE,
            ),
            AccountError::Token(TokenError::NameTooLong) => fail(
                t.msg(Key::OperatorTokenErrorNameLong)
                    .with("length", TOKEN_NAME_MAX + 1)
                    .with("max", TOKEN_NAME_MAX),
                exit::COMMAND_LINE,
            ),
            AccountError::Token(TokenError::NameUnsafe) => {
                fail(t.msg(Key::OperatorTokenErrorNameUnsafe), exit::COMMAND_LINE)
            }
            AccountError::Token(_) => fail(
                t.msg(Key::OperatorTokenErrorDays).with("max", MAX_DAYS),
                exit::COMMAND_LINE,
            ),
            AccountError::Backend(w) => backend(t, w),
            other => backend(t, &other.to_string()),
        }
    }
}

/// Create `file` for the token: **exclusively** (an existing file, and a symbolic link whether or not it points anywhere, is refused:
/// `O_EXCL` does not follow one), mode 0600 **from the moment it exists** (the mode is given to the call that creates it, not set
/// afterwards), in a directory that exists. The token and one newline are written and the file is flushed to disk.
#[cfg(unix)]
fn write_token_file(file: &str, secret: &str) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(file)?;
    let written = f
        .write_all(format!("{secret}\n").as_bytes())
        .and_then(|()| f.sync_all());
    if written.is_err() {
        drop(f);
        let _ = std::fs::remove_file(file);
    }
    written
}

#[cfg(not(unix))]
fn write_token_file(_file: &str, _secret: &str) -> Result<(), std::io::Error> {
    Err(std::io::Error::other(
        "a token file with a private mode is only made on a system with permission bits",
    ))
}

fn scope_sentence(t: &T, access: ScopeAccess) -> String {
    t.get(match access {
        ScopeAccess::ReadOnly => Key::OperatorTokenScopeReadOnly,
        ScopeAccess::Write => Key::OperatorTokenScopeWrite,
        ScopeAccess::Full => Key::OperatorTokenScopeFull,
    })
}

fn execute(t: &T, cmd: Cmd) -> i32 {
    let ctx = match open(t) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let now = config::unix_now();
    match cmd {
        Cmd::Create {
            handle,
            name,
            access,
            days,
            repos,
            out,
        } => {
            // The account first: nothing is written for a person who cannot be given a token.
            match ctx.service.show_account(&handle, 0) {
                Err(e) => return ctx.refuse(&e, &handle, ""),
                Ok(None) => return ctx.no_account(&handle),
                Ok(Some(d)) if d.row.disabled => {
                    return ctx.refuse(&AccountError::AccountDisabled, &handle, "");
                }
                Ok(Some(_)) => {}
            }
            let secret = mint_token();
            let hash = TokenHasher::hash(&Sha256TokenHasher, &secret);
            let expires_at = now + days * 86_400;
            // The file first (an existing file, a missing directory: nothing is stored).
            if let Some(file) = &out
                && let Err(e) = write_token_file(file, &secret)
            {
                let msg = if e.kind() == std::io::ErrorKind::AlreadyExists {
                    t.msg(Key::OperatorTokenErrorOutExists).with("file", file)
                } else {
                    t.msg(Key::OperatorTokenErrorOutFailed)
                        .with("file", file)
                        .with("why", e.kind())
                };
                return fail(msg, exit::REFUSED);
            }
            let spec = TokenSpec {
                handle: &handle,
                hash,
                name: &name,
                access,
                repos: &repos,
                expires_at,
            };
            let made = match ctx.service.create_token_for(spec, EventBy::Operator, now) {
                Ok(m) => m,
                Err(e) => {
                    // Storing failed: the file that was written for it goes again.
                    if let Some(file) = &out {
                        let _ = std::fs::remove_file(file);
                    }
                    return ctx.refuse(&e, &handle, "");
                }
            };
            // What the person needs to know goes to standard error; standard output is the token alone (or, with `--out`, the path).
            if let Ok(mut recent) = ctx.events.recent(&handle, 1)
                && let Some(e) = recent.pop()
            {
                eprintln!("{}", crate::operator_account::event_sentence(t, &e));
            }
            eprintln!(
                "{}",
                t.msg(Key::OperatorTokenDoneCreated)
                    .with("handle", &handle)
                    .with("name", &name)
                    .with("id", &made.id)
                    .text()
            );
            eprintln!("{}", scope_sentence(t, access));
            if made.repos.is_empty() {
                eprintln!("{}", t.get(Key::OperatorTokenReposAll));
            } else {
                eprintln!(
                    "{}",
                    t.msg(Key::OperatorTokenReposOnly)
                        .with("repos", made.repos.join(", "))
                        .text()
                );
            }
            eprintln!(
                "{}",
                t.msg(Key::OperatorTokenDoneExpires)
                    .with("date", planeter_web::timefmt::date(expires_at))
                    .text()
            );
            match &out {
                Some(file) => {
                    println!(
                        "{}",
                        t.msg(Key::OperatorTokenDoneOut).with("file", file).text()
                    );
                }
                None => {
                    eprintln!("{}", t.get(Key::OperatorTokenDoneOnce));
                    println!("{secret}");
                }
            }
            exit::DONE
        }
        Cmd::List { handle } => {
            match ctx.service.show_account(&handle, 0) {
                Err(e) => return ctx.refuse(&e, &handle, ""),
                Ok(None) => return ctx.no_account(&handle),
                Ok(Some(_)) => {}
            }
            let tokens = match ctx.creds.list_tokens(&UserId::new(&handle)) {
                Ok(l) => l,
                Err(e) => return backend(t, &e.to_string()),
            };
            if tokens.is_empty() {
                println!(
                    "{}",
                    t.msg(Key::OperatorTokenListNone)
                        .with("handle", &handle)
                        .text()
                );
                return exit::DONE;
            }
            println!("{}", t.get(Key::OperatorTokenListHeader));
            for tok in &tokens {
                let scope = t.get(match tok.scope.access {
                    ScopeAccess::ReadOnly => Key::OperatorTokenListReadOnly,
                    ScopeAccess::Write => Key::OperatorTokenListWrite,
                    ScopeAccess::Full => Key::OperatorTokenListFull,
                });
                let repos = match &tok.scope.repos {
                    RepoScope::All => t.get(Key::OperatorTokenListAll),
                    RepoScope::Only(ids) => ids
                        .iter()
                        .map(|id| repo_name(&ctx.repos, id))
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                let date = |at: Option<u64>| {
                    at.map_or_else(|| "-".to_owned(), planeter_web::timefmt::date)
                };
                println!(
                    "{} | {} | {} | {} | {} | {} | {}",
                    tok.id,
                    tok.name,
                    scope,
                    repos,
                    date(tok.created_at),
                    tok.last_used_at.map_or_else(
                        || t.get(Key::OperatorTokenListNever),
                        planeter_web::timefmt::date
                    ),
                    date(tok.scope.expires_at)
                );
            }
            exit::DONE
        }
        Cmd::Revoke { handle, id } => {
            match ctx
                .service
                .revoke_token_of(&handle, &id, EventBy::Operator, now)
            {
                Ok(name) => {
                    if let Ok(mut recent) = ctx.events.recent(&handle, 1)
                        && let Some(e) = recent.pop()
                    {
                        println!("{}", crate::operator_account::event_sentence(t, &e));
                    }
                    println!(
                        "{}",
                        t.msg(Key::OperatorTokenDoneRevoked)
                            .with("id", &id)
                            .with("name", name)
                            .with("handle", &handle)
                            .text()
                    );
                    exit::DONE
                }
                Err(e) => ctx.refuse(&e, &handle, &id),
            }
        }
    }
}

/// `owner/name` of a repository id, or the id if the repository is gone.
fn repo_name(repos: &SqliteRepositoryStore, id: &RepoId) -> String {
    match repos.get(id) {
        Ok(Some(r)) => {
            let owner = match &r.owner {
                planeter_store::Owner::User(h) | planeter_store::Owner::Org(h) => h.clone(),
            };
            format!("{owner}/{}", r.name)
        }
        _ => id.as_str().to_owned(),
    }
}
