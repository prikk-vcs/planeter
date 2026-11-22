//! `planeter operator` — the commands for **the person who runs this server** (RFC 013 D-2). The first is `key add`: with the server
//! **stopped**, add one maintainer key to one repository.
//!
//! The whole of it is one sequence under the server's own **process lock** (so it cannot run beside a live server, and a server cannot
//! start meanwhile), and nothing else: it adds and never removes a key (D-1, ledger PK-90: removing a key that signed anything makes
//! `verify` fail), it touches no history, no proposal, no delivery and no token, and it opens no door into a running server.
//!
//! Read only what it needs: `PLANETER_DB`, `PLANETER_REPOS_ROOT`, and the settings of prikk's own binary (`PATH`); **every other variable is
//! ignored**, and in particular `PLANETER_OIDC_CLIENT_SECRET` is never read. Messages are the server's own catalog (`operator.*`), in the
//! language `--lang` names, else the one `LC_ALL`/`LC_MESSAGES`/`LANG` names, else English; the result goes to stdout, everything else to
//! stderr.
//!
//! **Exit codes:** 0 done (or an interrupted act completed); 1 refused (the repository or the key); 2 the command line; 3 the server is
//! running, or its lock cannot be taken; 4 prikk or the record failed after a change; 5 local files or the database.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use planeter_core::{HostingService, LeaseClass, ProcessLock, ProcessLockError};
use planeter_prikk::PrikkRepo;
use planeter_store::{
    ChangeStore, DeliveryStore, KeyEvent, KeyEventStore, KeySource, Owner, RepositoryStore,
    RevisionStore, SqliteChangeStore, SqliteDb, SqliteDeliveryStore, SqliteKeyEventStore,
    SqliteLeaseRecords, SqliteRepositoryStore, SqliteRevisions,
};
use planeter_transport::{KeySourceView, detail_for_record, repo_keys};
use planeter_web::catalog::{Key, Lang, T};

use crate::config;

/// The longest reason, in characters (the table keeps no more; here a longer one is **refused**, never cut).
const REASON_MAX: usize = 200;

/// The exit codes.
mod exit {
    pub const DONE: i32 = 0;
    pub const REFUSED: i32 = 1;
    pub const COMMAND_LINE: i32 = 2;
    pub const LOCKED: i32 = 3;
    pub const AFTER_CHANGE: i32 = 4;
    pub const LOCAL: i32 = 5;
}

/// A named stop of the sequence, for the kill tests (a **debug build** reads `PLANETER_FAILPOINT=<point>=<file>`: it writes the file and stops
/// there for good, so that a test can SIGKILL the process exactly there). The release build has no such code.
#[cfg(debug_assertions)]
fn at(point: &str) {
    if let Some(hook) = crate::failpoint::from_env() {
        hook(point);
    }
}

#[cfg(not(debug_assertions))]
fn at(_point: &str) {}

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Help,
    KeyAdd(KeyAdd),
}

#[derive(Debug, PartialEq, Eq)]
struct KeyAdd {
    owner: String,
    name: String,
    key_id: String,
    public_key: String,
    reason: String,
}

/// Why the command line is refused, as a catalog key and its values.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Usage,
    RepoForm,
    ReasonMissing,
    ReasonLong(usize),
    ReasonControl,
    KeyShape,
    KeyIdShape,
    KeyIdMismatch { given: String, derived: String },
}

/// Everything `--lang` and the word list of one command line.
struct Line {
    lang: Option<String>,
    words: Vec<String>,
    options: Vec<(String, String)>,
    help: bool,
    malformed: bool,
}

/// Split the arguments into words, `--option value` pairs (also `--option=value`) and `--help`. An option without a value, a non-UTF-8
/// argument or an option twice is a malformed command line.
fn split(args: &[OsString]) -> Line {
    let mut line = Line {
        lang: None,
        words: Vec::new(),
        options: Vec::new(),
        help: false,
        malformed: false,
    };
    let mut i = 0;
    while i < args.len() {
        let Some(a) = args[i].to_str() else {
            line.malformed = true;
            i += 1;
            continue;
        };
        if a == "--help" || a == "-h" {
            line.help = true;
        } else if let Some(opt) = a.strip_prefix("--") {
            let (name, value) = match opt.split_once('=') {
                Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
                None => (opt.to_owned(), None),
            };
            let value = match value {
                Some(v) => Some(v),
                None => {
                    i += 1;
                    args.get(i).and_then(|v| v.to_str()).map(str::to_owned)
                }
            };
            match value {
                Some(v) => {
                    if name == "lang" {
                        if line.lang.is_some() {
                            line.malformed = true;
                        }
                        line.lang = Some(v);
                    } else if line.options.iter().any(|(n, _)| *n == name) {
                        line.malformed = true;
                    } else {
                        line.options.push((name, v));
                    }
                }
                None => line.malformed = true,
            }
        } else {
            line.words.push(a.to_owned());
        }
        i += 1;
    }
    line
}

/// The command line, or why not. **Pure**: no environment, no file.
fn parse(line: &Line) -> Result<Parsed, Refusal> {
    if line.help {
        return Ok(Parsed::Help);
    }
    if line.malformed || line.words.len() != 3 || line.words[0] != "key" || line.words[1] != "add" {
        return Err(Refusal::Usage);
    }
    for (name, _) in &line.options {
        if !matches!(name.as_str(), "key-id" | "public-key" | "reason") {
            return Err(Refusal::Usage);
        }
    }
    let get = |n: &str| {
        line.options
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.clone())
    };
    let repo = &line.words[2];
    let (owner, name) = match repo.split_once('/') {
        Some((o, n)) if !o.is_empty() && !n.is_empty() && !n.contains('/') => (o, n),
        _ => return Err(Refusal::RepoForm),
    };
    let (Some(key_id), Some(public_key)) = (get("key-id"), get("public-key")) else {
        return Err(Refusal::Usage);
    };
    let reason = get("reason").unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(Refusal::ReasonMissing);
    }
    if reason.chars().any(char::is_control) {
        return Err(Refusal::ReasonControl);
    }
    let length = reason.chars().count();
    if length > REASON_MAX {
        return Err(Refusal::ReasonLong(length));
    }
    if !planeter_prikk::keys::is_public_key(&public_key) {
        return Err(Refusal::KeyShape);
    }
    let shape = key_id.strip_prefix("ed25519-").is_some_and(|r| {
        r.len() == 16 && r.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    });
    if !shape {
        return Err(Refusal::KeyIdShape);
    }
    let derived = planeter_prikk::keys::key_id_of(&public_key).unwrap_or_default();
    if derived != key_id {
        return Err(Refusal::KeyIdMismatch {
            given: key_id,
            derived,
        });
    }
    Ok(Parsed::KeyAdd(KeyAdd {
        owner: owner.to_owned(),
        name: name.to_owned(),
        key_id,
        public_key,
        reason,
    }))
}

fn language(line: &Line) -> Option<Lang> {
    match &line.lang {
        Some(tag) => Lang::from_tag(tag),
        None => Some(Lang::from_environment(|n| std::env::var(n).ok())),
    }
}

fn help(t: &T) -> String {
    format!(
        "{}\n\n{}\n\n{}\n",
        t.get(Key::OperatorHelpPurpose),
        t.get(Key::OperatorHelpUsage),
        t.get(Key::OperatorHelpExitCodes)
    )
}

/// Run `planeter operator …`; the exit code.
pub fn run(args: Vec<OsString>) -> i32 {
    let line = split(&args);
    // A language named with `--lang` that is not offered is a malformed command line, said in English (the language it asked for does not exist).
    let Some(lang) = language(&line) else {
        let t = T::new(planeter_web::catalog::EN);
        eprintln!("planeter operator: {}", t.get(Key::OperatorErrorUsage));
        eprint!("{}", help(&t));
        return exit::COMMAND_LINE;
    };
    let t = T::new(lang);
    match parse(&line) {
        Ok(Parsed::Help) => {
            print!("{}", help(&t));
            exit::DONE
        }
        Ok(Parsed::KeyAdd(cmd)) => key_add(&t, &cmd),
        Err(refusal) => {
            let (key, code) = match &refusal {
                Refusal::Usage => (t.msg(Key::OperatorErrorUsage), exit::COMMAND_LINE),
                Refusal::RepoForm => (t.msg(Key::OperatorErrorRepoForm), exit::COMMAND_LINE),
                Refusal::ReasonMissing => (
                    t.msg(Key::OperatorErrorReasonMissing)
                        .with("max", REASON_MAX),
                    exit::COMMAND_LINE,
                ),
                Refusal::ReasonLong(n) => (
                    t.msg(Key::OperatorErrorReasonLong)
                        .with("length", n)
                        .with("max", REASON_MAX),
                    exit::COMMAND_LINE,
                ),
                Refusal::ReasonControl => {
                    (t.msg(Key::OperatorErrorReasonControl), exit::COMMAND_LINE)
                }
                Refusal::KeyShape => (t.msg(Key::OperatorErrorKeyShape), exit::COMMAND_LINE),
                Refusal::KeyIdShape => (t.msg(Key::OperatorErrorKeyIdShape), exit::COMMAND_LINE),
                Refusal::KeyIdMismatch { given, derived } => (
                    t.msg(Key::OperatorErrorKeyIdMismatch)
                        .with("given", given)
                        .with("derived", derived),
                    exit::REFUSED,
                ),
            };
            eprintln!("planeter operator: {}", key.text());
            if refusal == Refusal::Usage {
                eprint!("{}", help(&t));
            }
            code
        }
    }
}

/// Say a message on stderr and return the code.
fn fail(msg: planeter_web::catalog::Msg, code: i32) -> i32 {
    eprintln!("planeter operator: {}", msg.text());
    code
}

/// The sequence of RFC 013 D-2 (steps 2–11; step 1 is [`parse`]). Each refusal changes nothing.
fn key_add(t: &T, cmd: &KeyAdd) -> i32 {
    let repo_text = format!("{}/{}", cmd.owner, cmd.name);
    let paths = config::paths_from_env();

    // 2. The process lock: a server holds it for its whole life. A database that is not there is not made here.
    if !Path::new(&paths.db_path).is_file() {
        return fail(
            t.msg(Key::OperatorErrorNoDatabase)
                .with("path", &paths.db_path),
            exit::LOCAL,
        );
    }
    let lock = match ProcessLock::acquire(paths.lock_file()) {
        Ok(l) => l,
        Err(ProcessLockError::AlreadyRunning { .. }) => {
            return fail(t.msg(Key::OperatorErrorRunning), exit::LOCKED);
        }
        Err(other) => {
            return fail(
                t.msg(Key::OperatorErrorLock).with("why", other),
                exit::LOCKED,
            );
        }
    };

    // 3. The database; the repository.
    let db = match SqliteDb::open(&paths.db_path) {
        Ok(db) => db,
        Err(e) => {
            return fail(
                t.msg(Key::OperatorErrorDatabase).with("why", e),
                exit::LOCAL,
            );
        }
    };
    let local = |e: &dyn std::fmt::Display| {
        fail(t.msg(Key::OperatorErrorFiles).with("why", e), exit::LOCAL)
    };
    let repos: Arc<dyn RepositoryStore> = Arc::new(SqliteRepositoryStore::new(db.clone()));
    let id = [
        Owner::User(cmd.owner.clone()),
        Owner::Org(cmd.owner.clone()),
    ]
    .iter()
    .find_map(|o| repos.resolve(o, &cmd.name).ok().flatten());
    let Some(record) = id.and_then(|id| repos.get(&id).ok().flatten()) else {
        return fail(
            t.msg(Key::OperatorErrorUnknownRepo)
                .with("repo", &repo_text),
            exit::REFUSED,
        );
    };
    let repo = record.repo_id.clone();

    // 4. Refuse a repository that has something unfinished: a recovery flag, an open intent, a delivery in flight, a lease record that a dead
    //    process left. The server's own start puts those right; this command does not build on them.
    let changes = match SqliteChangeStore::new(db.clone()) {
        Ok(c) => c,
        Err(e) => return local(&e),
    };
    let deliveries = match SqliteDeliveryStore::new(db.clone()) {
        Ok(d) => d,
        Err(e) => return local(&e),
    };
    let leases = match SqliteLeaseRecords::new(db.clone()) {
        Ok(r) => config::lease_manager(r),
        Err(e) => return local(&e),
    };
    let unfinished = changes
        .repo_flags(&repo)
        .map(|f| f.needs_recovery)
        .unwrap_or(true)
        || changes
            .open_intents(&repo)
            .map(|v| !v.is_empty())
            .unwrap_or(true)
        || deliveries
            .in_flight(&repo)
            .map(|d| d.is_some())
            .unwrap_or(true)
        || leases
            .orphans()
            .map(|o| o.iter().any(|h| h.repo_id == repo))
            .unwrap_or(true);
    if unfinished {
        return fail(t.msg(Key::OperatorErrorNeedsRecovery), exit::REFUSED);
    }

    // 5. prikk's list is the truth. A key already trusted: refused, unless its record is missing (an act interrupted after the add): then the
    //    record is written now.
    let hosting = HostingService::new(paths.repos_root_path(), repos.clone());
    let ceiling = verify_ceiling(&record.path);
    let driver = match hosting
        .open_record_with(&record, planeter_prikk::CallLimits::data_ceiling(ceiling))
    {
        Ok(d) => d,
        Err(e) => {
            return fail(t.msg(Key::OperatorErrorPrikk).with("why", e), exit::LOCAL);
        }
    };
    let events = match SqliteKeyEventStore::new(db.clone()) {
        Ok(e) => e,
        Err(e) => return local(&e),
    };
    let keys = match repo_keys(&driver, &events, &repo) {
        Ok(k) => k,
        Err(e) => {
            return fail(t.msg(Key::OperatorErrorPrikk).with("why", e), exit::LOCAL);
        }
    };
    let already = keys.iter().find(|k| k.public_key == cmd.public_key);
    // An interrupted act of this command always leaves at least one row (step 7b writes the older keys' records before the add), so only
    // a repository that already has a key record can have one. In a repository with none, every trusted key is "not recorded"; a key
    // trusted since creation is no act of this command, and the page must not say it was added by the operator (RFC 013 D-4). An error
    // reading the table counts as "no record": the key is refused as already trusted, and nothing is claimed.
    let has_records = events.has_any(&repo).unwrap_or(false);
    let completing = match already {
        Some(k) if k.source == KeySourceView::NotRecorded && has_records => true,
        Some(_) => {
            return fail(t.msg(Key::OperatorErrorAlreadyTrusted), exit::REFUSED);
        }
        None => false,
    };

    // 6. The bound (RFC 013 D-6).
    if !completing && planeter_prikk::keys::room_for(keys.len(), 1).is_err() {
        return fail(
            t.msg(Key::OperatorErrorBound)
                .with("max", planeter_prikk::keys::KEYS_MAX),
            exit::REFUSED,
        );
    }

    // 7. The repository's W lease: its durable holder record is the intent. A kill leaves it; the next server's start finds it and recovers.
    let lease = match leases.acquire(
        &repo,
        LeaseClass::Write,
        "operator key add",
        Duration::from_secs(5),
    ) {
        Ok(l) => l,
        Err(e) => return local(&e),
    };
    // 7b. A repository with no record at all (it has not been through the server's start-up pass yet, or it was made with no key) is given the
    //     record of the keys it trusts **now**, before the add, as that pass would (source `creation`, `backfilled`; in 0.8.0 the creation
    //     form was the only way a key got there). Otherwise the one row this command writes would make the repository look recorded, the
    //     pass would never fill the older keys in, and a kill between the add and the record would let the pass label the new key "creation".
    if !events.has_any(&repo).unwrap_or(true) {
        let now = config::unix_now();
        let existing: Vec<KeyEvent> = keys
            .iter()
            .filter(|k| k.public_key != cmd.public_key)
            .map(|k| KeyEvent {
                repo_id: repo.clone(),
                key_id: k.key_id.clone(),
                public_key: k.public_key.clone(),
                source: KeySource::Creation,
                detail: String::new(),
                delivery_id: None,
                recorded_at: now,
                backfilled: true,
            })
            .collect();
        if let Err(e) = events.record_all_if_none(&repo, &existing) {
            return local(&e);
        }
    }
    at("operator_after_lease");

    // 8. `prikk trust maintainer add`, then `verify`. After the add nothing can undo it (D-1): a repository that no longer verifies is flagged.
    if !completing {
        if let Err(e) = driver.trust_maintainer_add(&cmd.key_id, &cmd.public_key) {
            return fail(
                t.msg(Key::OperatorErrorAdd).with("why", e),
                exit::AFTER_CHANGE,
            );
        }
        at("operator_after_add");
    }
    let verified = driver
        .verify()
        .map(|v| v.verdict.ok && v.publication_trust_issues.is_empty())
        .unwrap_or(false);
    if !verified {
        let _ = changes.set_needs_recovery(
            &repo,
            "an operator's key add did not verify",
            config::unix_now(),
        );
        return fail(t.msg(Key::OperatorErrorVerify), exit::AFTER_CHANGE);
    }
    at("operator_after_verify");

    // 9. The revision counter, as every other write does.
    let Ok(revisions) = SqliteRevisions::new(db.clone()) else {
        return fail(t.msg(Key::OperatorErrorRecord), exit::AFTER_CHANGE);
    };
    if revisions.bump(&repo).is_err() {
        return fail(t.msg(Key::OperatorErrorRecord), exit::AFTER_CHANGE);
    }
    at("operator_after_bump");

    // 10. The record: how this key came (source `operator`, the reason through the display function).
    let now = config::unix_now();
    let recorded = events.record(&KeyEvent {
        repo_id: repo.clone(),
        key_id: cmd.key_id.clone(),
        public_key: cmd.public_key.clone(),
        source: KeySource::Operator,
        detail: detail_for_record(&cmd.reason),
        delivery_id: None,
        recorded_at: now,
        backfilled: false,
    });
    if recorded.is_err() {
        return fail(t.msg(Key::OperatorErrorRecord), exit::AFTER_CHANGE);
    }
    at("operator_after_event");

    // 11. Release the lease and the lock; the result, in the words the page will show.
    drop(lease);
    drop(lock);
    let fingerprint = planeter_core::creation::group_fingerprint(&cmd.public_key);
    let sentence = if completing {
        t.msg(Key::OperatorDoneRecorded)
    } else {
        t.msg(Key::OperatorDone)
    }
    .with("fingerprint", fingerprint)
    .with("repo", &repo_text)
    .with("date", planeter_web::timefmt::date(now))
    .with("reason", detail_for_record(&cmd.reason));
    println!("{}", sentence.text());
    println!("{}", t.get(Key::OperatorStart));
    exit::DONE
}

/// The memory ceiling of the calls of this command: the cost of a `verify` of the repository's store, at least the light class.
fn verify_ceiling(root: &Path) -> u64 {
    let size = planeter_core::admission::store_size(root).unwrap_or(0);
    planeter_core::admission::ceiling_for(
        planeter_core::admission::ReadVerb::Verify
            .cost(size)
            .max(planeter_core::admission::COST_FLOOR),
        None,
    )
}
