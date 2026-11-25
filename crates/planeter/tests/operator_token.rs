//! RFC 014 D-3: **`planeter operator token …`**, the built binary, **beside a running real server** (real prikk and the real sandbox where
//! a repository is read). What a command does is seen on the very next request; a refusal changes nothing (every table dumped before and
//! after); a password is never an argument, an environment variable, an output or a stored thing but an Argon2id hash; the `stty` guard
//! turns the echo back on on every way out. Tests that need prikk, bubblewrap, prlimit or `script` self-skip, printing why.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use planeter_auth::{
    Sha256TokenHasher, SqliteAccountStore, SqliteCredentialStore, TokenHasher, TokenRecord,
};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, UserId,
    Visibility,
};

const BIN: &str = env!("CARGO_BIN_EXE_planeter");
/// A secret of the server that no operator command may read, and a password that must be found nowhere.
const CANARY: &str = "canary-oidc-secret-7c1e9a40";

fn sandbox_works() -> bool {
    ["bwrap", "prlimit"].iter().all(|t| {
        Command::new(t)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

macro_rules! need_server {
    () => {
        if !archives::prikk_available() || !sandbox_works() {
            eprintln!("skipping: needs prikk, bubblewrap and prlimit");
            return;
        }
    };
}

// ------------------------------------------------------------------------------------------------ the world

struct World {
    dir: PathBuf,
}

fn world(name: &str) -> World {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("optoken-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("root")).unwrap();
    // The database exists as the server makes it (the commands never make it).
    let db = SqliteDb::open(dir.join("state.db")).unwrap();
    drop(SqliteAccountStore::new(db.clone()).unwrap());
    drop(SqliteCredentialStore::new(db).unwrap());
    World { dir }
}

impl World {
    fn db(&self) -> SqliteDb {
        SqliteDb::open(self.dir.join("state.db")).unwrap()
    }

    /// A private repository `owner/name` made by the real prikk, recorded the way the server records one.
    fn repository(&self, owner: &str, name: &str, id: &str) {
        let path = self.dir.join("root").join("rr").join(id);
        std::fs::create_dir_all(&path).unwrap();
        archives::run(&path, &self.dir.join("home"), &["init", "."]);
        self.repository_row(owner, name, id);
    }

    /// The record of a private repository and an (empty) directory, **without prikk**: what the role commands need (they read no history).
    fn repository_row(&self, owner: &str, name: &str, id: &str) {
        let path = self.dir.join("root").join("rr").join(id);
        std::fs::create_dir_all(&path).unwrap();
        SqliteRepositoryStore::new(self.db())
            .create(RepositoryRecord {
                repo_id: RepoId::new(id),
                owner: Owner::User(owner.into()),
                name: name.into(),
                visibility: Visibility::Private,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path,
            })
            .unwrap();
    }

    /// Every table, every row, as text (`quote()` of every column: text as it is, never as bytes): what "changes nothing" compares and
    /// what a canary is looked for in.
    fn dump(&self) -> Vec<String> {
        let db = self.db();
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
                let columns: Vec<String> = conn
                    .prepare(&format!(
                        "SELECT name FROM pragma_table_info('{t}') ORDER BY cid"
                    ))
                    .unwrap()
                    .query_map([], |r| r.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                let select = columns
                    .iter()
                    .map(|c| format!("quote(\"{c}\")"))
                    .collect::<Vec<_>>()
                    .join(" || '|' || ");
                let rows: Vec<String> = conn
                    .prepare(&format!("SELECT {select} FROM {t} ORDER BY rowid"))
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                format!("{t}: {}", rows.join(";;"))
            })
            .collect()
    }

    /// The command: only the settings it needs, and a canary in the one secret it must never read. `password` is given on file
    /// descriptor 3 (the shell redirects it from the standard input), never as an argument or in the environment.
    fn op(&self, args: &[&str], password: Option<&str>) -> Output {
        self.op_env(args, password, &[])
    }

    fn op_env(&self, args: &[&str], password: Option<&str>, env: &[(&str, &str)]) -> Output {
        let mut child = self.spawn_op(args, env);
        if let Some(p) = password {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(format!("{p}\n").as_bytes()).unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }

    fn spawn_op(&self, args: &[&str], env: &[(&str, &str)]) -> Child {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exec \"$0\" \"$@\" 3<&0", BIN, "operator"])
            .args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("PLANETER_DB", self.dir.join("state.db"))
            .env("PLANETER_OIDC_CLIENT_SECRET", CANARY)
            .env("HOME", self.dir.join("home"))
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.spawn().expect("run planeter operator")
    }
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

// ------------------------------------------------------------------------------------------------ the real server

struct Server {
    child: Child,
    port: u16,
    log: PathBuf,
}

fn start(w: &World) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log = w.dir.join("server.log");
    let mut child = Command::new(BIN)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", w.dir.join("root"))
        .env("PLANETER_DB", w.dir.join("state.db"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .env("PLANETER_SPOOL_DIR", w.dir.join("spool"))
        .env("PLANETER_ARTIFACT_DIR", w.dir.join("artifacts"))
        .env("HOME", w.dir.join("home"))
        .current_dir(&w.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let reader = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let path = log.clone();
    std::thread::spawn(move || {
        let mut file = std::fs::File::create(path).unwrap();
        let mut said = false;
        for line in reader.lines().map_while(Result::ok) {
            let _ = writeln!(file, "{line}");
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(60))
        .expect("the server never said it is listening");
    Server { child, port, log }
}

impl Server {
    /// `(status, headers, body)` of one request.
    fn http(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, String)],
        body: &str,
    ) -> (u16, String, String) {
        let end = Instant::now() + Duration::from_secs(10);
        let mut s = loop {
            match TcpStream::connect(("127.0.0.1", self.port)) {
                Ok(s) => break s,
                Err(_) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => panic!("the server does not accept connections: {e}"),
            }
        };
        s.set_read_timeout(Some(Duration::from_secs(120))).ok();
        let mut req =
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
        for (k, v) in headers {
            req += &format!("{k}: {v}\r\n");
        }
        req += &format!("Content-Length: {}\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let raw = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (status, head.to_owned(), body.to_owned())
    }

    fn stop(mut self) -> String {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

// ------------------------------------------------------------------------------------------------ helpers of this file

impl Server {
    fn status(&self, token: &str, method: &str, path: &str) -> u16 {
        self.http(
            method,
            path,
            &[("Authorization", format!("Bearer {token}"))],
            "",
        )
        .0
    }
}

const HISTORY: &str = "/api/v1/repos/alice/forge/history";
const OTHER_HISTORY: &str = "/api/v1/repos/alice/other/history";
const PROPOSE: &str = "/api/v1/repos/alice/forge/transport/propose";
const DELIVER: &str = "/api/v1/repos/alice/forge/deliveries";

/// The token a `token create` printed: exactly one line of standard output.
fn made(o: &Output) -> String {
    assert_eq!(code(o), 0, "{}", err(o));
    let stdout = out(o);
    assert_eq!(
        stdout.lines().count(),
        1,
        "standard output is one line: {stdout:?}"
    );
    assert!(stdout.ends_with('\n'));
    let token = stdout.trim_end().to_owned();
    assert!(token.starts_with("plt_") && token.len() > 40, "{token}");
    token
}

/// `create` with the usual arguments plus `extra`.
fn create(w: &World, handle: &str, name: &str, scope: &str, extra: &[&str]) -> Output {
    let mut args = vec!["token", "create", handle, "--name", name, "--scope", scope];
    args.extend(extra.iter());
    w.op(&args, None)
}

fn with_two_repositories(name: &str) -> World {
    let w = world(name);
    // The account first (a repository's owner handle is taken from then on), then the repositories.
    assert_eq!(
        code(&w.op(&["account", "create", "alice", "--name", "Alice A"], None)),
        0
    );
    // The tests that read a repository through the server have real ones (and skip without prikk); the others need only the records,
    // so they run where there is no prikk (the CI `build` job).
    for (name, id) in [("forge", "r1"), ("other", "r2")] {
        if archives::prikk_available() {
            w.repository("alice", name, id);
        } else {
            w.repository_row("alice", name, id);
        }
    }
    w
}

// ------------------------------------------------------------------------------------------------ T5 (a): exactly what the scope says

#[test]
fn a_token_works_at_once_and_is_limited_exactly_as_its_scope_and_repositories_say() {
    need_server!();
    let w = with_two_repositories("scopes");
    let server = start(&w);
    let read_only = made(&create(&w, "alice", "reader", "read-only", &[]));
    let write = made(&create(&w, "alice", "writer", "write", &[]));
    let full = made(&create(&w, "alice", "maintainer", "full", &[]));
    let limited = made(&create(
        &w,
        "alice",
        "forge only",
        "write",
        &["--repos", "alice/forge"],
    ));
    let nothing = "plt_not-a-token-at-all-0000000000000000000000";

    // At once: the very first request with each is authorized (an unknown token is a 404 on a private repository).
    assert_eq!(server.status(nothing, "GET", HISTORY), 404);
    for t in [&read_only, &write, &full, &limited] {
        assert_eq!(
            server.status(t, "GET", HISTORY),
            200,
            "reads alice's private repository"
        );
    }
    // A read-only token reads and cannot propose; a write token proposes (it gets past authorization: the empty body is refused after).
    assert_eq!(
        server.status(&read_only, "POST", PROPOSE),
        403,
        "read-only cannot propose"
    );
    for t in [&write, &full, &limited] {
        let s = server.status(t, "POST", PROPOSE);
        assert!(
            !matches!(s, 401 | 403 | 404),
            "a token that may propose is not refused for its scope: {s}"
        );
    }
    // A write token cannot deliver (403); the full token of the repository's maintainer can reach the delivery route's authorization.
    assert_eq!(server.status(&read_only, "POST", DELIVER), 403);
    assert_eq!(
        server.status(&write, "POST", DELIVER),
        403,
        "a write token is capped below merge-seal"
    );
    let s = server.status(&full, "POST", DELIVER);
    assert!(
        !matches!(s, 401 | 403 | 404),
        "a full token of a maintainer is authorized to deliver: {s}"
    );
    // A token with --repos reaches only those repositories: another repository answers as a hidden one does (404).
    assert_eq!(server.status(&limited, "GET", OTHER_HISTORY), 404);
    assert_eq!(
        server.status(&write, "GET", OTHER_HISTORY),
        200,
        "the same account's other token reaches it"
    );
    // A token never exceeds the person: bob has only `write` on the repository, so even a full token of his cannot deliver.
    assert_eq!(
        code(&w.op(&["account", "create", "bob", "--name", "Bob B"], None)),
        0
    );
    assert_eq!(
        code(&w.op(
            &["account", "grant", "alice/forge", "bob", "--role", "write"],
            None
        )),
        0
    );
    let bob_full = made(&create(&w, "bob", "bob full", "full", &[]));
    assert_eq!(server.status(&bob_full, "GET", HISTORY), 200);
    assert_eq!(
        server.status(&bob_full, "POST", DELIVER),
        403,
        "the token is the intersection with the person's role"
    );
    assert_eq!(
        server.status(&bob_full, "GET", OTHER_HISTORY),
        404,
        "bob has no role on alice's other repository"
    );
    let log = server.stop();
    for t in [&read_only, &write, &full, &limited, &bob_full] {
        assert!(
            !log.contains(t.as_str()),
            "the server's log carries a token"
        );
    }
}

// ------------------------------------------------------------------------------------------------ T5 (b): revoke, disable, expiry, last used

#[test]
fn revoke_and_disable_refuse_the_token_on_the_very_next_request_and_an_expired_one_is_listed_and_refused()
 {
    need_server!();
    let w = with_two_repositories("refuse");
    let server = start(&w);
    let a = made(&create(&w, "alice", "to revoke", "write", &[]));
    let b = made(&create(&w, "alice", "to disable", "write", &[]));
    let c = made(&create(
        &w,
        "alice",
        "to expire",
        "write",
        &["--expires-days", "30"],
    ));
    for t in [&a, &b, &c] {
        assert_eq!(server.status(t, "GET", HISTORY), 200);
    }
    // `token list`: ids, names, scope in words, never a hash or a secret; last used shows a date for the ones used.
    let o = w.op(&["token", "list", "alice"], None);
    let listing = out(&o);
    assert!(
        listing.starts_with("ID | NAME | SCOPE | REPOSITORIES | CREATED | LAST USED | EXPIRES\n"),
        "{listing}"
    );
    for name in ["to revoke", "to disable", "to expire"] {
        assert!(listing.contains(name), "{listing}");
    }
    for t in [&a, &b, &c] {
        assert!(!listing.contains(t.as_str()));
    }
    assert!(!listing.contains("never"), "all three were used: {listing}");
    // Reading twice within the hour writes once: the stored time does not move.
    let stamp = |w: &World| -> String {
        w.db()
            .lock()
            .unwrap()
            .query_row(
                "SELECT group_concat(last_used_at) FROM tokens ORDER BY name",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
    };
    let before = stamp(&w);
    for t in [&a, &b, &c] {
        assert_eq!(server.status(t, "GET", HISTORY), 200);
    }
    assert_eq!(
        stamp(&w),
        before,
        "a second use within the hour wrote nothing"
    );
    // Revoke: the next request is refused; the others still work.
    let id_a = listing
        .lines()
        .find(|l| l.contains("to revoke"))
        .and_then(|l| l.split(" | ").next())
        .unwrap()
        .to_owned();
    let o = w.op(&["token", "revoke", "alice", &id_a], None);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("Access token \"to revoke\" was revoked by this server's operator."),
        "{}",
        out(&o)
    );
    assert!(
        out(&o).contains(&format!(
            "Token {id_a} (to revoke) of alice was revoked. It is refused from the next request."
        )),
        "{}",
        out(&o)
    );
    assert_eq!(
        server.status(&a, "GET", HISTORY),
        404,
        "revoked: refused on the very next request"
    );
    assert_eq!(server.status(&b, "GET", HISTORY), 200);
    // Disable the account: every token it holds is refused on the very next request; enable: they work again.
    assert_eq!(
        code(&w.op(&["account", "disable", "alice", "--reason", "away"], None)),
        0
    );
    assert_eq!(server.status(&b, "GET", HISTORY), 404);
    assert_eq!(server.status(&c, "GET", HISTORY), 404);
    let o = create(&w, "alice", "late", "read-only", &[]);
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("alice is disabled, so it cannot be given a token"),
        "{}",
        err(&o)
    );
    assert_eq!(code(&w.op(&["account", "enable", "alice"], None)), 0);
    assert_eq!(server.status(&b, "GET", HISTORY), 200);
    // An expired token (the clock moved: its end set to 1970-01-01): refused, and listed with its date.
    w.db()
        .lock()
        .unwrap()
        .execute(
            "UPDATE tokens SET expires_at = 1 WHERE name = 'to expire'",
            [],
        )
        .unwrap();
    assert_eq!(
        server.status(&c, "GET", HISTORY),
        404,
        "an expired token is refused"
    );
    let listing = out(&w.op(&["token", "list", "alice"], None));
    assert!(
        listing
            .lines()
            .any(|l| l.contains("to expire") && l.ends_with("1970-01-01")),
        "{listing}"
    );
    let _ = server.stop();
}

// ------------------------------------------------------------------------------------------------ T5 (c), (d): --out and the one line

#[cfg(unix)]
#[test]
fn out_writes_a_private_new_file_and_prints_only_the_path_sentence() {
    use std::os::unix::fs::PermissionsExt;
    let w = with_two_repositories("out");
    let file = w.dir.join("alice.token");
    let o = create(
        &w,
        "alice",
        "file",
        "full",
        &["--out", &file.to_string_lossy()],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    // Standard output is the path sentence and nothing else; the token is not printed anywhere.
    assert_eq!(
        out(&o),
        format!(
            "The token was written to {} (mode 0600). Give that path to planeter-client --token-file.\n",
            file.display()
        )
    );
    let secret = std::fs::read_to_string(&file).unwrap();
    let secret = secret.trim_end();
    assert!(secret.starts_with("plt_"));
    assert!(!err(&o).contains(secret) && !out(&o).contains(secret));
    assert!(
        err(&o).contains(
            "This token can do everything the account can, including delivering a branch."
        ),
        "{}",
        err(&o)
    );
    // Mode 0600, and the file holds one line.
    let meta = std::fs::metadata(&file).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
    // The token from the file works as a bearer (the authenticator reads the database the command wrote).
    let auth = planeter_auth::Authenticator::new(
        Arc::new(SqliteAccountStore::new(w.db()).unwrap()),
        Arc::new(SqliteCredentialStore::new(w.db()).unwrap()),
        Arc::new(planeter_auth::Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    );
    assert!(auth.authenticate_token(secret).is_ok());
    // The secret exists nowhere else: not in the database (only its hash), not in an event.
    let dump = w.dump().join("\n");
    assert!(
        !dump.contains(secret),
        "the database holds the hash, not the token"
    );
    // An existing file is never overwritten (its content is the same afterwards); neither is a symbolic link followed.
    let before = w.dump();
    let o = create(
        &w,
        "alice",
        "again",
        "read-only",
        &["--out", &file.to_string_lossy()],
    );
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("already exists, and a token file is never overwritten"),
        "{}",
        err(&o)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap().trim_end(), secret);
    assert_eq!(w.dump(), before, "nothing was stored");
    let target = w.dir.join("victim");
    std::fs::write(&target, "do not touch\n").unwrap();
    let link = w.dir.join("link.token");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let o = create(
        &w,
        "alice",
        "link",
        "read-only",
        &["--out", &link.to_string_lossy()],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not touch\n");
    let dangling = w.dir.join("dangling.token");
    std::os::unix::fs::symlink(w.dir.join("nowhere"), &dangling).unwrap();
    let o = create(
        &w,
        "alice",
        "dangling",
        "read-only",
        &["--out", &dangling.to_string_lossy()],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert!(
        !w.dir.join("nowhere").exists(),
        "a dangling link is not followed to make its target"
    );
    assert_eq!(w.dump(), before);
    // A directory that does not exist: refused, nothing stored.
    let o = create(
        &w,
        "alice",
        "nodir",
        "read-only",
        &["--out", &w.dir.join("missing/dir/t").to_string_lossy()],
    );
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("The token could not be written to"),
        "{}",
        err(&o)
    );
    assert_eq!(w.dump(), before);
}

#[cfg(unix)]
#[test]
fn a_failure_after_the_file_is_written_leaves_no_file_and_no_token() {
    let w = with_two_repositories("outfail");
    // The store fails (the event cannot be written): the file that was written first is removed again, and no token row remains.
    w.db()
        .lock()
        .unwrap()
        .execute_batch("CREATE TRIGGER refuse_token_event BEFORE INSERT ON account_events WHEN NEW.kind = 'token_created' BEGIN SELECT RAISE(ABORT, 'refused'); END;")
        .unwrap();
    let before = w.dump();
    let file = w.dir.join("never.token");
    let o = create(
        &w,
        "alice",
        "x",
        "full",
        &["--out", &file.to_string_lossy()],
    );
    assert_ne!(code(&o), 0, "{}", out(&o));
    assert!(!file.exists(), "the file written for it was removed");
    assert_eq!(w.dump(), before, "no token row and no event");
    assert!(
        out(&o).is_empty() || !out(&o).contains("plt_"),
        "{}",
        out(&o)
    );
    // The same without --out: nothing is printed for a token that was not stored.
    let o = create(&w, "alice", "x", "full", &[]);
    assert_ne!(code(&o), 0);
    assert!(
        out(&o).is_empty(),
        "no token is printed when storing failed: {}",
        out(&o)
    );
    assert_eq!(w.dump(), before);
}

#[test]
fn standard_output_is_one_line_and_standard_error_says_what_it_is_in_plain_words() {
    let w = with_two_repositories("oneline");
    let o = create(
        &w,
        "alice",
        "laptop",
        "read-only",
        &[
            "--expires-days",
            "30",
            "--repos",
            "alice/forge,alice/other,alice/forge",
        ],
    );
    let token = made(&o);
    let e = err(&o);
    assert!(!e.contains(&token), "the token is not on standard error");
    for must in [
        "Access token \"laptop\" (read-only, 2 repositories, until ",
        "A token for alice (laptop) was created. Its id is ",
        "It is read-only: it can look at repositories and clone them, and nothing else.",
        "It reaches only these repositories: alice/forge, alice/other.",
        "It ends on ",
        "This is the only time the token is shown. Keep it somewhere private: it cannot be shown again.",
    ] {
        assert!(e.contains(must), "{must}\n{e}");
    }
    // Japanese: the same, and standard output is still the token alone.
    let o = create(&w, "alice", "ノート", "write", &["--lang", "ja"]);
    let token_ja = made(&o);
    assert!(
        err(&o).contains(
            "読むことと、作業を提案することができますが、ブランチを引き渡すことはできません。"
        ),
        "{}",
        err(&o)
    );
    assert!(!err(&o).contains(&token_ja));
    // The default lifetime is 90 days.
    let listing = out(&w.op(&["token", "list", "alice"], None));
    let line = listing.lines().find(|l| l.contains("ノート")).unwrap();
    assert!(
        line.contains("| write |") && line.contains("| all |"),
        "{line}"
    );
}

// ------------------------------------------------------------------------------------------------ T5 (e): refusals change nothing

fn refusal(w: &World, args: &[&str], expect_code: i32, en: &str, ja: &str) {
    let before = w.dump();
    let o = w.op(args, None);
    assert_eq!(code(&o), expect_code, "{args:?}: {}", err(&o));
    assert!(
        out(&o).is_empty(),
        "{args:?}: a refusal prints nothing on standard output: {}",
        out(&o)
    );
    assert!(err(&o).contains(en), "{args:?}: {}", err(&o));
    assert_eq!(w.dump(), before, "{args:?}: the database is as it was");
    let mut with_lang = args.to_vec();
    with_lang.extend(["--lang", "ja"]);
    let o = w.op(&with_lang, None);
    assert_eq!(code(&o), expect_code, "{with_lang:?}");
    assert!(err(&o).contains(ja), "{with_lang:?}: {}", err(&o));
    let o = w.op_env(args, None, &[("LC_ALL", "ja_JP.UTF-8")]);
    assert_eq!(code(&o), expect_code);
    assert!(err(&o).contains(ja), "{args:?}: {}", err(&o));
    assert_eq!(w.dump(), before);
}

#[test]
fn every_refusal_changes_nothing_says_why_and_speaks_both_languages() {
    let w = with_two_repositories("refusals");
    assert_eq!(
        code(&w.op(&["account", "create", "bob", "--name", "Bob"], None)),
        0
    );
    assert_eq!(
        code(&w.op(&["account", "create", "carol", "--name", "Carol"], None)),
        0
    );
    assert_eq!(
        code(&w.op(&["account", "disable", "carol", "--reason", "away"], None)),
        0
    );
    made(&create(&w, "bob", "bobs", "read-only", &[]));
    let bobs_id: String = w
        .db()
        .lock()
        .unwrap()
        .query_row("SELECT id FROM tokens WHERE user = 'bob'", [], |r| r.get(0))
        .unwrap();
    let name61 = "n".repeat(61);
    let many: String = (0..21)
        .map(|i| format!("alice/r{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let file = w.dir.join("exists.token");
    std::fs::write(&file, "x\n").unwrap();
    let file_s = file.to_string_lossy().into_owned();
    let missing_dir = w.dir.join("no/such/dir/t").to_string_lossy().into_owned();
    // 200 tokens that have not expired: the 201st is refused.
    {
        let creds = SqliteCredentialStore::new(w.db()).unwrap();
        for i in 0..planeter_auth::TOKENS_MAX {
            creds
                .add_token(TokenRecord {
                    hash: TokenHasher::hash(&Sha256TokenHasher, &format!("bulk-{i}")),
                    user: UserId::new("alice"),
                    scope: Scope {
                        access: ScopeAccess::ReadOnly,
                        repos: RepoScope::All,
                        expires_at: Some(4_000_000_000),
                    },
                    id: format!("b{i:015x}"),
                    name: format!("bulk {i}"),
                    created_at: Some(1),
                    last_used_at: None,
                })
                .unwrap();
        }
    }
    #[allow(clippy::type_complexity)]
    let cases: Vec<(Vec<&str>, i32, &str, &str)> = vec![
        (
            vec!["token"],
            2,
            "This command line is not understood",
            "このコマンドラインは解釈できません",
        ),
        (
            vec!["token", "create", "alice", "--scope", "full"],
            2,
            "A name is required",
            "名前が必要です",
        ),
        (
            vec!["token", "create", "alice", "--name", "", "--scope", "full"],
            2,
            "A name is required",
            "名前が必要です",
        ),
        (
            vec![
                "token", "create", "alice", "--name", &name61, "--scope", "full",
            ],
            2,
            "The name is 61 characters long",
            "名前は 61 文字あり",
        ),
        (
            vec![
                "token",
                "create",
                "alice",
                "--name",
                "bad\nname",
                "--scope",
                "full",
            ],
            2,
            "cannot be shown",
            "表示できない文字",
        ),
        (
            vec!["token", "create", "alice", "--name", "x"],
            2,
            "A scope is required",
            "スコープが必要です",
        ),
        (
            vec!["token", "create", "alice", "--name", "x", "--scope", "root"],
            2,
            "The scope must be read-only, write or full",
            "スコープは read-only、write、full",
        ),
        (
            vec![
                "token",
                "create",
                "alice",
                "--name",
                "x",
                "--scope",
                "full",
                "--expires-days",
                "0",
            ],
            2,
            "--expires-days must be a whole number of days from 1 to 365",
            "--expires-days は、1〜365",
        ),
        (
            vec![
                "token",
                "create",
                "alice",
                "--name",
                "x",
                "--scope",
                "full",
                "--expires-days",
                "366",
            ],
            2,
            "from 1 to 365",
            "1〜365",
        ),
        (
            vec![
                "token",
                "create",
                "alice",
                "--name",
                "x",
                "--scope",
                "full",
                "--expires-days",
                "soon",
            ],
            2,
            "from 1 to 365",
            "1〜365",
        ),
        (
            vec![
                "token",
                "create",
                "alice",
                "--name",
                "x",
                "--scope",
                "full",
                "--repos",
                "alice/ghost",
            ],
            1,
            "There is no repository alice/ghost",
            "リポジトリ alice/ghost はありません",
        ),
        (
            vec![
                "token", "create", "alice", "--name", "x", "--scope", "full", "--repos", &many,
            ],
            2,
            "at most 20 repositories",
            "20 個まで",
        ),
        (
            vec![
                "token", "create", "alice", "--name", "x", "--scope", "full", "--repos", "",
            ],
            2,
            "--repos needs one or more OWNER/NAME",
            "--repos には、OWNER/NAME を 1 つ以上",
        ),
        (
            vec![
                "token", "create", "alice", "--name", "x", "--scope", "full", "--repos", "alice",
            ],
            2,
            "OWNER/NAME",
            "OWNER/NAME",
        ),
        (
            vec!["token", "create", "ghost", "--name", "x", "--scope", "full"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["token", "create", "carol", "--name", "x", "--scope", "full"],
            1,
            "carol is disabled, so it cannot be given a token",
            "carol は無効になっているので",
        ),
        (
            vec!["token", "create", "alice", "--name", "x", "--scope", "full"],
            1,
            "already has 200 tokens that have not expired",
            "期限が切れていないトークンがすでに 200 個",
        ),
        (
            vec![
                "token", "create", "bob", "--name", "x", "--scope", "full", "--out", &file_s,
            ],
            1,
            "already exists, and a token file is never overwritten",
            "上書きしません",
        ),
        (
            vec![
                "token",
                "create",
                "bob",
                "--name",
                "x",
                "--scope",
                "full",
                "--out",
                &missing_dir,
            ],
            1,
            "The token could not be written to",
            "に書けませんでした",
        ),
        (
            vec![
                "token", "create", "bob", "--name", "x", "--scope", "full", "--out", "",
            ],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec!["token", "list", "ghost"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["token", "list"],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec!["token", "revoke", "alice", "0000000000000000"],
            1,
            "alice has no token 0000000000000000",
            "alice には、トークン 0000000000000000 はありません",
        ),
        (
            vec!["token", "revoke", "alice", &bobs_id],
            1,
            "alice has no token",
            "alice には、トークン",
        ),
        (
            vec!["token", "revoke", "ghost", "0000000000000000"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["token", "revoke", "alice"],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
    ];
    for (args, expect, en, ja) in cases {
        refusal(&w, &args, expect, en, ja);
    }
    // Nothing was left of the refused `--out` writes.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x\n");
    assert!(!w.dir.join("no").exists());
}

// ------------------------------------------------------------------------------------------------ T5 (f): beside a live server, a held lock, a 0.9.0 directory

#[test]
fn eight_creates_at_once_beside_a_server_and_a_held_lock_gives_the_busy_sentence() {
    need_server!();
    let w = with_two_repositories("concurrent");
    let server = start(&w);
    let children: Vec<Child> = (0..8)
        .map(|i| {
            w.spawn_op(
                &[
                    "token",
                    "create",
                    "alice",
                    "--name",
                    &format!("t{i}"),
                    "--scope",
                    "read-only",
                ],
                &[],
            )
        })
        .collect();
    for _ in 0..5 {
        assert_eq!(
            server.http("GET", "/", &[], "").0,
            200,
            "the server answers meanwhile"
        );
    }
    let tokens: Vec<String> = children
        .into_iter()
        .map(|c| {
            let o = c.wait_with_output().unwrap();
            made(&o)
        })
        .collect();
    assert_eq!(
        tokens
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        8,
        "eight different tokens"
    );
    for t in &tokens {
        assert_eq!(server.status(t, "GET", HISTORY), 200);
    }
    let events: i64 = w
        .db()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM account_events WHERE kind = 'token_created'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 8);
    let _ = server.stop();
    // A held write lock: the busy sentence, exit 5, nothing changed (and no token printed).
    let held = w.db();
    held.lock()
        .unwrap()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    let before = w.dump();
    let o = create(&w, "alice", "late", "read-only", &[]);
    assert_eq!(code(&o), 5, "{}", err(&o));
    assert!(
        err(&o).contains("The database stayed busy for more than 5 seconds"),
        "{}",
        err(&o)
    );
    assert!(out(&o).is_empty());
    held.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(w.dump(), before);
}

#[test]
fn the_commands_work_on_a_database_as_0_9_0_left_it_and_list_shows_the_old_tokens_with_the_placeholder_name()
 {
    let w = world("old");
    {
        let db = w.db();
        let c = db.lock().unwrap();
        c.execute_batch("DROP TABLE account_events; DROP TABLE accounts; DROP TABLE tokens; DROP TABLE sessions;").unwrap();
        c.execute_batch(
            "CREATE TABLE accounts (user TEXT PRIMARY KEY, display_name TEXT NOT NULL, email TEXT, password_hash TEXT);
             CREATE TABLE tokens (hash TEXT PRIMARY KEY, user TEXT NOT NULL, access TEXT NOT NULL, repos TEXT, expires_at INTEGER);
             CREATE TABLE sessions (id TEXT PRIMARY KEY, user TEXT NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
             INSERT INTO accounts VALUES ('alice', 'Alice (0.9.0)', NULL, NULL);
             INSERT INTO tokens VALUES ('h1', 'alice', 'full', NULL, NULL);
             INSERT INTO tokens VALUES ('h2', 'alice', 'read-only', NULL, 4000000000);",
        )
        .unwrap();
    }
    let o = w.op(&["token", "list", "alice"], None);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let listing = out(&o);
    assert_eq!(
        listing.matches("(created before 0.10.0)").count(),
        2,
        "{listing}"
    );
    assert!(
        listing.contains("| full | all | - | never | -")
            && listing.contains("| read-only | all | - | never | 2096-"),
        "{listing}"
    );
    let token = made(&create(&w, "alice", "new", "write", &[]));
    let o = w.op(&["token", "list", "alice"], None);
    assert!(
        out(&o).lines().nth(1).unwrap().contains("| new |"),
        "newest first: {}",
        out(&o)
    );
    let id: String = w
        .db()
        .lock()
        .unwrap()
        .query_row("SELECT id FROM tokens WHERE hash = 'h1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(code(&w.op(&["token", "revoke", "alice", &id], None)), 0);
    assert!(token.starts_with("plt_"));
}

// ------------------------------------------------------------------------------------------------ canaries

#[test]
fn the_token_is_in_no_event_no_log_no_error_and_no_database_row_but_as_its_hash() {
    let w = with_two_repositories("canary");
    let o = create(&w, "alice", "secret one", "full", &[]);
    let token = made(&o);
    let hash = TokenHasher::hash(&Sha256TokenHasher, &token).0;
    let dump = w.dump().join("\n");
    assert!(!dump.contains(&token), "the secret is not stored");
    assert!(dump.contains(&hash), "its hash is");
    let events: String = w
        .dump()
        .into_iter()
        .filter(|l| l.starts_with("account_events:"))
        .collect();
    assert!(
        !events.contains(&hash) && !events.contains(&token),
        "an event names the token's name, scope and expiry only"
    );
    // Every other command's output, and the failing ones', is clean of it.
    for args in [
        vec!["token", "list", "alice"],
        vec!["account", "show", "alice"],
        vec!["account", "list"],
        vec!["token", "revoke", "alice", "0000000000000000"],
    ] {
        let o = w.op(&args, None);
        let all = format!("{}{}", out(&o), err(&o));
        assert!(
            !all.contains(&token) && !all.contains(&hash) && !all.contains(CANARY),
            "{args:?}: {all}"
        );
    }
    // The command's own process list and environment never held it (it is minted inside, after they are read): look at one that waits.
    let mut child = w.spawn_op(
        &[
            "token",
            "create",
            "alice",
            "--name",
            "waits",
            "--scope",
            "read-only",
        ],
        &[],
    );
    let pid = child.id();
    if let (Ok(cmdline), Ok(environ)) = (
        std::fs::read(format!("/proc/{pid}/cmdline")),
        std::fs::read(format!("/proc/{pid}/environ")),
    ) {
        let both = format!(
            "{}{}",
            String::from_utf8_lossy(&cmdline),
            String::from_utf8_lossy(&environ)
        );
        assert!(
            !both.contains("plt_"),
            "no token in `ps` or the environment"
        );
        assert!(
            !both.contains(CANARY) || both.contains("PLANETER_OIDC_CLIENT_SECRET"),
            "only the variable the test set"
        );
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    assert_eq!(code(&o), 0);
}

// ------------------------------------------------------------------------------------------------ the sentence is the holder's sentence

#[test]
fn the_token_sentences_are_render_event_of_the_events_just_written_in_both_languages() {
    use planeter_auth::{AccountEventStore, SqliteAccountEventStore};
    use planeter_web::account_events::render_event;
    use planeter_web::catalog::{Lang, T};
    let w = with_two_repositories("holder-sentence");
    let newest = || {
        SqliteAccountEventStore::new(w.db())
            .unwrap()
            .recent("alice", 1)
            .unwrap()
            .remove(0)
    };
    for tag in ["en", "ja"] {
        let t = T::new(Lang::from_tag(tag).unwrap());
        let o = create(
            &w,
            "alice",
            &format!("n-{tag}"),
            "write",
            &["--repos", "alice/forge,alice/other", "--lang", tag],
        );
        made(&o);
        let sentence = render_event(&t, &newest());
        assert!(
            err(&o).lines().any(|l| l == sentence),
            "{sentence:?} not among\n{}",
            err(&o)
        );
        let id: String = w
            .db()
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM tokens WHERE name = ?1",
                [format!("n-{tag}")],
                |r| r.get(0),
            )
            .unwrap();
        let o = w.op(&["token", "revoke", "alice", &id, "--lang", tag], None);
        assert_eq!(code(&o), 0, "{}", err(&o));
        let sentence = render_event(&t, &newest());
        assert!(
            out(&o).lines().any(|l| l == sentence),
            "{sentence:?} not among\n{}",
            out(&o)
        );
    }
}
