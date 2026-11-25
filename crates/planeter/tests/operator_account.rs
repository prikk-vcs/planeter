//! RFC 014 D-3: **`planeter operator account …`**, the built binary, **beside a running real server** (real prikk and the real sandbox where
//! a repository is read). What a command does is seen on the very next request; a refusal changes nothing (every table dumped before and
//! after); a password is never an argument, an environment variable, an output or a stored thing but an Argon2id hash; the `stty` guard
//! turns the echo back on on every way out. Tests that need prikk, bubblewrap, prlimit or `script` self-skip, printing why.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use planeter_auth::{
    Account, AccountStore, Authenticator, InMemoryCredentialStore, Sha256TokenHasher,
    SqliteAccountStore, SqliteCredentialStore, TokenHasher, TokenRecord, VerifiedIdentity,
};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, UserId,
    Visibility,
};

const BIN: &str = env!("CARGO_BIN_EXE_planeter");
/// A secret of the server that no operator command may read, and a password that must be found nowhere.
const CANARY: &str = "canary-oidc-secret-7c1e9a40";
const PASSWORD: &str = "zebra-quartz-plinth-9137-marmot";

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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("opacct-{name}"));
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

    fn token(&self, secret: &str, user: &str) {
        SqliteCredentialStore::new(self.db())
            .unwrap()
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&Sha256TokenHasher, secret),
                user: UserId::new(user),
                scope: Scope {
                    access: ScopeAccess::Full,
                    repos: RepoScope::All,
                    expires_at: None,
                },
                id: String::new(),
                name: String::new(),
                created_at: None,
                last_used_at: None,
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

    fn read_as(&self, token: &str) -> u16 {
        self.http(
            "GET",
            "/api/v1/repos/alice/forge/history",
            &[("Authorization", format!("Bearer {token}"))],
            "",
        )
        .0
    }

    /// Sign in with a password: `(status, the session cookie if one was set)`.
    fn login(&self, user: &str, password: &str) -> (u16, Option<String>) {
        let csrf = "fixed-csrf-token-for-the-tests";
        let (status, head, _) = self.http(
            "POST",
            "/login",
            &[
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
                ("Cookie", format!("planeter_csrf={csrf}")),
            ],
            &format!("csrf={csrf}&username={user}&password={password}"),
        );
        let cookie = head
            .lines()
            .find(|l| {
                l.to_lowercase()
                    .starts_with("set-cookie: planeter_session=")
            })
            .and_then(|l| {
                l.split_once('=')
                    .map(|(_, v)| v.split(';').next().unwrap_or("").to_owned())
            });
        (status, cookie)
    }

    fn signed_in(&self, session: &str) -> bool {
        let (_, _, body) = self.http(
            "GET",
            "/",
            &[("Cookie", format!("planeter_session={session}"))],
            "",
        );
        body.contains("action=\"/logout\"")
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

// ------------------------------------------------------------------------------------------------ T4: beside a running server

#[test]
fn every_command_works_beside_a_running_server_and_is_seen_on_the_very_next_request() {
    need_server!();
    let w = world("beside");
    let server = start(&w);

    // (a) `create`, then the person signs in at once (a password; and, for a link, the authenticator as the OIDC tests have it).
    let o = w.op(
        &[
            "account",
            "create",
            "alice",
            "--name",
            "Alice A",
            "--password-fd",
            "3",
        ],
        Some(PASSWORD),
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("The account was created by this server's operator."),
        "{}",
        out(&o)
    );
    let (status, session_alice) = server.login("alice", PASSWORD);
    assert_eq!(status, 303, "signs in at once");
    let session_alice = session_alice.unwrap();
    assert!(server.signed_in(&session_alice));
    // alice's private repository is recorded now, with the server running (it reads the database at every request).
    w.repository("alice", "forge", "r1");
    assert_eq!(server.login("alice", "not the password").0, 401);
    assert_eq!(
        code(&w.op(
            &[
                "account",
                "create",
                "bob",
                "--name",
                "Bob B",
                "--password-fd",
                "3"
            ],
            Some(PASSWORD)
        )),
        0
    );
    w.token("tok-bob", "bob"); // a token row written by another process while the server runs: seen at the next request

    // (d) `grant` lets the person act at once; `revoke` stops them.
    assert_eq!(
        server.read_as("tok-bob"),
        404,
        "bob has no role on alice's private repository"
    );
    let o = w.op(
        &["account", "grant", "alice/forge", "bob", "--role", "read"],
        None,
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains(
            "Bob B (bob) was given read on alice/forge: look at the repository and clone it"
        ),
        "{}",
        out(&o)
    );
    assert_eq!(server.read_as("tok-bob"), 200, "at once");
    let o = w.op(&["account", "revoke", "alice/forge", "bob"], None);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(server.read_as("tok-bob"), 404, "revoked: at once");
    // a role again, for (b)
    assert_eq!(
        code(&w.op(
            &["account", "grant", "alice/forge", "bob", "--role", "write"],
            None
        )),
        0
    );
    assert_eq!(server.read_as("tok-bob"), 200);

    // (b) `disable` ends a live session and refuses a live token on the very next request; `enable` restores the token (not the session).
    let (_, bob_session) = server.login("bob", PASSWORD);
    let bob_session = bob_session.unwrap();
    assert!(server.signed_in(&bob_session));
    let o = w.op(
        &["account", "disable", "bob", "--reason", "left the project"],
        None,
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("The account was disabled by this server's operator: left the project")
    );
    assert!(
        !server.signed_in(&bob_session),
        "the session ended on the very next request"
    );
    assert_eq!(
        server.read_as("tok-bob"),
        404,
        "the token is refused on the very next request"
    );
    assert_eq!(
        server.login("bob", PASSWORD).0,
        401,
        "a password sign-in answers as for a wrong password"
    );
    let o = w.op(
        &["account", "grant", "alice/forge", "bob", "--role", "admin"],
        None,
    );
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("bob is disabled"), "{}", err(&o));
    assert_eq!(code(&w.op(&["account", "enable", "bob"], None)), 0);
    assert_eq!(server.read_as("tok-bob"), 200, "the token works again");
    assert!(
        !server.signed_in(&bob_session),
        "an ended session does not return"
    );

    // (c) `set-password` ends the sessions; the old password stops and the new one works.
    let new_password = "a-completely-different-password-4411";
    let o = w.op(
        &["account", "set-password", "alice", "--password-fd", "3"],
        Some(new_password),
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("It was signed out everywhere; its tokens are not touched."),
        "{}",
        out(&o)
    );
    assert!(!server.signed_in(&session_alice), "the live session ended");
    assert_eq!(
        server.login("alice", PASSWORD).0,
        401,
        "the old password stopped"
    );
    assert_eq!(
        server.login("alice", new_password).0,
        303,
        "the new one works"
    );

    // (f) `key add` still refuses beside a running server (exit 3), as before: a well-formed key, so that only the running server stops it.
    let public_key = "ab".repeat(32);
    let key_id = planeter_prikk::keys::key_id_of(&public_key).unwrap();
    let o = w.op(
        &[
            "key",
            "add",
            "alice/forge",
            "--key-id",
            &key_id,
            "--public-key",
            &public_key,
            "--reason",
            "x",
        ],
        None,
    );
    assert_eq!(code(&o), 3, "{}", err(&o));
    assert!(err(&o).contains("The server is running."), "{}", err(&o));
    // ... and the account commands never took the process lock: the lock file is the server's own (its pid), and they all ran.
    let lock = std::fs::read_to_string(w.dir.join("state.db.lock")).unwrap();
    assert!(lock.trim().parse::<u32>().is_ok(), "{lock}");
    let log = server.stop();
    // The server's log names no account, no token, no password.
    for secret in [PASSWORD, new_password, "tok-bob", CANARY] {
        assert!(!log.contains(secret), "the server's log carries a secret");
    }
}

#[test]
fn an_identity_linked_by_the_command_signs_in_through_the_authenticator() {
    // The real server cannot reach a provider on this host (the egress guard refuses loopback), so the sign-in half is the authenticator
    // as the OIDC tests have it: the link the command wrote is what `authenticate_oidc` finds; and a command for another issuer is refused.
    let w = world("link");
    let issuer = "https://idp.example";
    assert_eq!(
        code(&w.op(&["account", "create", "alice", "--name", "Alice"], None)),
        0
    );
    // No provider set up: refused.
    let o = w.op(
        &[
            "account",
            "link-oidc",
            "alice",
            "--issuer",
            issuer,
            "--subject",
            "SECRET-SUBJECT-42",
        ],
        None,
    );
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("No sign-in provider is set up on this server"),
        "{}",
        err(&o)
    );
    // Another issuer than the configured one: refused.
    let o = w.op_env(
        &[
            "account",
            "link-oidc",
            "alice",
            "--issuer",
            "https://evil.example",
            "--subject",
            "s",
        ],
        None,
        &[("PLANETER_OIDC_ISSUER", issuer)],
    );
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("is not the sign-in provider this server is set up for"),
        "{}",
        err(&o)
    );
    // The configured one: linked.
    let o = w.op_env(
        &[
            "account",
            "link-oidc",
            "alice",
            "--issuer",
            issuer,
            "--subject",
            "SECRET-SUBJECT-42",
        ],
        None,
        &[("PLANETER_OIDC_ISSUER", issuer)],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains(
            "An identity at idp.example was linked to the account by this server's operator."
        ),
        "{}",
        out(&o)
    );
    assert!(
        !out(&o).contains("SECRET-SUBJECT") && !err(&o).contains("SECRET-SUBJECT"),
        "the subject is never shown"
    );
    let auth = Authenticator::new(
        std::sync::Arc::new(SqliteAccountStore::new(w.db()).unwrap()),
        std::sync::Arc::new(InMemoryCredentialStore::new()),
        std::sync::Arc::new(planeter_auth::Argon2idHasher),
        std::sync::Arc::new(Sha256TokenHasher),
    );
    let id = VerifiedIdentity {
        issuer: issuer.into(),
        subject: "SECRET-SUBJECT-42".into(),
        email: None,
    };
    assert!(
        auth.authenticate_oidc(&id).is_ok(),
        "the linked identity signs alice in"
    );
    // Another account cannot take it; the same account again changes nothing; unlinking works and says so.
    assert_eq!(
        code(&w.op(&["account", "create", "bob", "--name", "Bob"], None)),
        0
    );
    let env = [("PLANETER_OIDC_ISSUER", issuer)];
    let before = w.dump();
    let o = w.op_env(
        &[
            "account",
            "link-oidc",
            "bob",
            "--issuer",
            issuer,
            "--subject",
            "SECRET-SUBJECT-42",
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("already linked to another account"),
        "{}",
        err(&o)
    );
    let o = w.op_env(
        &[
            "account",
            "link-oidc",
            "alice",
            "--issuer",
            issuer,
            "--subject",
            "SECRET-SUBJECT-42",
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 1);
    assert_eq!(w.dump(), before, "both refusals changed nothing");
    let o = w.op_env(
        &[
            "account",
            "unlink-oidc",
            "alice",
            "--issuer",
            issuer,
            "--subject",
            "SECRET-SUBJECT-42",
        ],
        None,
        &env,
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(auth.authenticate_oidc(&id).is_err(), "no longer");
    // The subject is in no event and no output: only the link table ever held it.
    let events: String = w
        .dump()
        .into_iter()
        .filter(|l| l.starts_with("account_events:"))
        .collect();
    assert!(!events.contains("SECRET-SUBJECT"), "{events}");
}

// ------------------------------------------------------------------------------------------------ T5: refusals change nothing

fn same_in_both_languages(w: &World, args: &[&str], expect_code: i32, en: &str, ja: &str) {
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
    // The same refusal in Japanese, by `--lang` and by `LC_ALL`.
    let mut with_lang = args.to_vec();
    with_lang.extend(["--lang", "ja"]);
    let o = w.op(&with_lang, None);
    assert_eq!(code(&o), expect_code, "{with_lang:?}");
    assert!(err(&o).contains(ja), "{with_lang:?}: {}", err(&o));
    let o = w.op_env(args, None, &[("LC_ALL", "ja_JP.UTF-8")]);
    assert_eq!(code(&o), expect_code, "{args:?} in Japanese by LC_ALL");
    assert!(err(&o).contains(ja), "{args:?}: {}", err(&o));
    assert_eq!(w.dump(), before);
}

#[test]
fn every_refusal_changes_nothing_says_why_and_speaks_both_languages() {
    let w = world("refusals");
    assert_eq!(
        code(&w.op(&["account", "create", "alice", "--name", "Alice"], None)),
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
    // A repository row (the refusals need no prikk: they stop before anything is opened).
    let dir = w.dir.join("root/rr/r1");
    std::fs::create_dir_all(&dir).unwrap();
    SqliteRepositoryStore::new(w.db())
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "forge".into(),
            visibility: Visibility::Private,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: dir,
        })
        .unwrap();
    let long_name = "n".repeat(101);
    let long_reason = "r".repeat(201);
    // (arguments, exit code, an English phrase, a Japanese phrase)
    #[allow(clippy::type_complexity)]
    let cases: Vec<(Vec<&str>, i32, &str, &str)> = vec![
        (
            vec!["account"],
            2,
            "This command line is not understood",
            "このコマンドラインは解釈できません",
        ),
        (
            vec!["account", "frobnicate", "x"],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec!["account", "create", "Bad", "--name", "n"],
            2,
            "has capitals",
            "大文字が含まれています",
        ),
        (
            vec!["account", "create", "api", "--name", "n"],
            2,
            "a word this server uses",
            "このサーバーが自分のページに使っている言葉",
        ),
        (
            vec!["account", "create", "-x", "--name", "n"],
            2,
            "is not allowed",
            "は使えません",
        ),
        (
            vec!["account", "create", "ok", "--name", ""],
            2,
            "A display name is required",
            "表示名が必要です",
        ),
        (
            vec!["account", "create", "ok"],
            2,
            "A display name is required",
            "表示名が必要です",
        ),
        (
            vec!["account", "create", "ok", "--name", &long_name],
            2,
            "The display name is 101 characters long",
            "表示名は 101 文字あり",
        ),
        (
            vec!["account", "create", "ok", "--name", "bad\nname"],
            2,
            "cannot be shown",
            "表示できない文字",
        ),
        (
            vec!["account", "create", "alice", "--name", "Again"],
            1,
            "already in use",
            "すでに使われています",
        ),
        (
            vec![
                "account",
                "create",
                "ok",
                "--name",
                "n",
                "--password-prompt",
                "--password-fd",
                "3",
            ],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec![
                "account",
                "create",
                "ok",
                "--name",
                "n",
                "--password-fd",
                "2",
            ],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec![
                "account",
                "create",
                "ok",
                "--name",
                "n",
                "--password-prompt",
            ],
            2,
            "Standard input is not a terminal",
            "標準入力が端末ではない",
        ),
        (
            vec!["account", "set-password", "ghost", "--password-fd", "3"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["account", "disable", "alice"],
            2,
            "A reason is required",
            "理由が必要です",
        ),
        (
            vec!["account", "disable", "alice", "--reason", &long_reason],
            2,
            "The reason is 201 characters long",
            "理由が 201 文字あり",
        ),
        (
            vec!["account", "disable", "ghost", "--reason", "x"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["account", "disable", "carol", "--reason", "again"],
            1,
            "carol is already disabled",
            "carol はすでに無効になっています",
        ),
        (
            vec!["account", "enable", "alice"],
            1,
            "alice is not disabled",
            "alice は無効になっていません",
        ),
        (
            vec!["account", "enable", "ghost"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec![
                "account",
                "link-oidc",
                "alice",
                "--issuer",
                "https://idp.example",
                "--subject",
                "s",
            ],
            1,
            "No sign-in provider is set up",
            "ログインのプロバイダーが設定されていません",
        ),
        (
            vec![
                "account",
                "link-oidc",
                "alice",
                "--issuer",
                "https://idp.example",
            ],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec!["account", "grant", "alice/forge", "ghost", "--role", "read"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["account", "grant", "alice/forge", "alice", "--role", "root"],
            2,
            "The role must be read, write, maintain or admin",
            "権限は read、write、maintain、admin",
        ),
        (
            vec!["account", "grant", "alice/forge", "alice"],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec!["account", "grant", "alice", "alice", "--role", "read"],
            2,
            "OWNER/NAME",
            "OWNER/NAME",
        ),
        (
            vec![
                "account",
                "grant",
                "alice/nothing",
                "alice",
                "--role",
                "read",
            ],
            1,
            "There is no repository alice/nothing",
            "リポジトリ alice/nothing はありません",
        ),
        (
            vec!["account", "grant", "alice/forge", "carol", "--role", "read"],
            1,
            "carol is disabled, so it cannot be given a role",
            "carol は無効になっているので",
        ),
        (
            vec!["account", "revoke", "alice/forge", "alice"],
            1,
            "has no role of its own on alice/forge",
            "自分の権限を持っていない",
        ),
        (
            vec!["account", "revoke", "alice/nothing", "alice"],
            1,
            "There is no repository alice/nothing",
            "リポジトリ alice/nothing はありません",
        ),
        (
            vec!["account", "show", "ghost"],
            1,
            "There is no account ghost",
            "アカウント ghost はありません",
        ),
        (
            vec!["account", "list", "extra"],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
        (
            vec!["account", "list", "--check=yes"],
            2,
            "This command line is not understood",
            "解釈できません",
        ),
    ];
    for (args, expect, en, ja) in cases {
        same_in_both_languages(&w, &args, expect, en, ja);
    }
}

#[test]
fn a_missing_database_is_exit_5_and_is_not_made() {
    let w = world("nodb");
    std::fs::remove_file(w.dir.join("state.db")).unwrap();
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(w.dir.join(format!("state.db{ext}")));
    }
    let o = w.op(&["account", "list"], None);
    assert_eq!(code(&o), 5, "{}", err(&o));
    assert!(err(&o).contains("There is no database at"), "{}", err(&o));
    assert!(
        !w.dir.join("state.db").exists(),
        "the command never makes the database"
    );
}

// ------------------------------------------------------------------------------------------------ the results, the help, the reading

#[test]
fn the_help_describes_every_command_the_roles_and_the_exit_codes_and_key_add_is_as_it_was() {
    let w = world("help");
    for (lang, must) in [
        (
            "en",
            vec![
                "planeter operator account create HANDLE",
                "read: look at the repository and clone it; write: also propose work; maintain: also ask for a delivery and review proposals; admin: also manage who has access",
                "If your terminal stops echoing, run stty sane.",
                "These commands do not need the server stopped, so 3 is not used",
                "4 is not used either",
            ],
        ),
        (
            "ja",
            vec![
                "planeter operator account create HANDLE",
                "権限: read:",
                "stty sane",
                "3 は使いません",
                "4 も使いません",
            ],
        ),
    ] {
        let o = w.op(&["account", "--help", "--lang", lang], None);
        assert_eq!(code(&o), 0);
        for m in must {
            assert!(out(&o).contains(m), "{lang}: {m}\n{}", out(&o));
        }
    }
    // `planeter operator --help`: the text of `key add` as it was, and one line that names the account commands.
    let o = w.op(&["--help"], None);
    let text = out(&o);
    assert!(text.contains("usage: planeter operator key add OWNER/NAME --key-id ID --public-key HEX --reason TEXT [--lang en|ja]"));
    assert!(
        text.contains("key add        make one more public key a maintainer key of one repository")
    );
    assert!(text.contains("account …") && text.contains("planeter operator account --help"));
    assert!(text.contains("exit codes: 0 done; 1 refused (the repository or the key); 2 the command line; 3 the server is running"));
}

#[test]
fn the_happy_paths_print_the_sentence_the_holder_will_read_in_both_languages() {
    let w = world("results");
    // English.
    let o = w.op(
        &[
            "account",
            "create",
            "alice",
            "--name",
            "Alice A",
            "--password-fd",
            "3",
        ],
        Some(PASSWORD),
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(
        out(&o),
        "The account was created by this server's operator.\nA password was set by this server's operator.\nalice (Alice A) has a password.\n"
    );
    w.repository_row("alice", "forge", "r1");
    let o = w.op(&["account", "create", "bob", "--name", "Bob B"], None);
    assert_eq!(
        out(&o),
        "The account was created by this server's operator.\nbob (Bob B) has no password: it can sign in only through a linked identity.\n"
    );
    // Japanese.
    let o = w.op(
        &[
            "account",
            "create",
            "carol",
            "--name",
            "カロル",
            "--lang",
            "ja",
        ],
        None,
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).starts_with("アカウントは、このサーバーを運用している人が作成しました。\n"),
        "{}",
        out(&o)
    );
    // Roles and what they mean.
    for (role, means) in [
        ("read", "look at the repository and clone it"),
        ("write", "also propose work"),
        ("maintain", "also ask for a delivery and review proposals"),
        ("admin", "also manage who has access"),
    ] {
        let o = w.op(
            &["account", "grant", "alice/forge", "bob", "--role", role],
            None,
        );
        assert_eq!(code(&o), 0, "{}", err(&o));
        assert!(
            out(&o).contains(&format!(
                "The role \"{role}\" on alice/forge was given by this server's operator."
            )),
            "{}",
            out(&o)
        );
        assert!(
            out(&o).contains(&format!(
                "Bob B (bob) was given {role} on alice/forge: {means}"
            )),
            "{}",
            out(&o)
        );
    }
    // The same role again is a refusal (nothing changes); a revoke of an admin leaves an administrator (alice owns the repository).
    let before = w.dump();
    assert_eq!(
        code(&w.op(
            &["account", "grant", "alice/forge", "bob", "--role", "admin"],
            None
        )),
        1
    );
    assert_eq!(w.dump(), before);
    let o = w.op(&["account", "revoke", "alice/forge", "bob"], None);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains(
            "The role \"admin\" on alice/forge was taken away by this server's operator."
        )
    );
    assert!(
        !out(&o).contains("no administrator"),
        "alice, the owner, remains"
    );
    // list, show, in both languages; a Japanese `show`.
    let o = w.op(&["account", "list"], None);
    assert!(out(&o).starts_with("HANDLE | NAME | STATE | PASSWORD | IDENTITY | CREATED\nalice | Alice A | active | yes | no | "), "{}", out(&o));
    assert!(
        out(&o).contains("\nbob | Bob B | active | no | no | "),
        "{}",
        out(&o)
    );
    let o = w.op(&["account", "show", "bob", "--lang", "ja"], None);
    assert!(
        out(&o).starts_with("ユーザー名: bob\n表示名: Bob B\n状態: 有効\n"),
        "{}",
        out(&o)
    );
    assert!(out(&o).contains("直近の出来事:"), "{}", out(&o));
    let o = w.op(&["account", "show", "bob"], None);
    assert!(out(&o).contains("Last events:\n  "), "{}", out(&o));
    assert!(
        out(&o).contains(
            "The role \"admin\" on alice/forge was taken away by this server's operator."
        ),
        "{}",
        out(&o)
    );
    // list --check with nothing to report exits 0 and says so.
    let o = w.op(&["account", "list", "--check"], None);
    assert_eq!(
        (code(&o), out(&o).as_str()),
        (0, "No handles differ only by case.\n")
    );
}

#[test]
fn disable_names_the_repositories_it_leaves_without_an_administrator_bounded_and_then_proceeds() {
    let w = world("sole");
    assert_eq!(
        code(&w.op(&["account", "create", "bob", "--name", "Bob"], None)),
        0
    );
    // 25 organization repositories of which bob is the only administrator.
    for i in 0..25 {
        let path = w.dir.join(format!("root/rr/o{i:02}"));
        std::fs::create_dir_all(&path).unwrap();
        w.db()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO repositories (repo_id, owner_kind, owner, name, visibility, created_at, path) VALUES (?1, 'org', 'acme', ?2, 'public', 0, ?3)",
                (format!("o{i:02}"), format!("r{i:02}"), path.to_string_lossy().into_owned()),
            )
            .unwrap();
        assert_eq!(
            code(&w.op(
                &[
                    "account",
                    "grant",
                    &format!("acme/r{i:02}"),
                    "bob",
                    "--role",
                    "admin"
                ],
                None
            )),
            0
        );
    }
    let o = w.op(&["account", "disable", "bob", "--reason", "leaving"], None);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let first = out(&o).lines().next().unwrap().to_owned();
    assert!(
        first.starts_with(
            "These repositories will have no administrator until you grant one: acme/r00, acme/r01,"
        ),
        "{first}"
    );
    assert!(first.ends_with("and 5 more"), "{first}");
    assert_eq!(first.matches("acme/r").count(), 20, "at most 20 names");
    assert!(out(&o).contains("The account was disabled by this server's operator: leaving"));
    // Revoking the last administrator is refused without the flag, allowed with it, and says what it leaves.
    assert_eq!(code(&w.op(&["account", "enable", "bob"], None)), 0);
    let before = w.dump();
    let o = w.op(&["account", "revoke", "acme/r00", "bob"], None);
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("That would leave acme/r00 with no administrator"),
        "{}",
        err(&o)
    );
    assert_eq!(w.dump(), before);
    let o = w.op(
        &["account", "revoke", "acme/r00", "bob", "--allow-no-admin"],
        None,
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("acme/r00 is without an administrator until you grant one")
            || out(&o).contains("no administrator")
            || out(&o).contains("だれも管理できません")
            || out(&o).contains("until you grant one"),
        "{}",
        out(&o)
    );
}

#[test]
fn list_is_bounded_and_list_check_reports_a_case_collision_and_every_command_still_works() {
    let w = world("collision");
    let accounts = SqliteAccountStore::new(w.db()).unwrap();
    for h in ["Alice", "alice", "ALICE", "bob"] {
        accounts.upsert(Account::local(h, h, None)).unwrap();
    }
    let o = w.op(&["account", "list", "--check"], None);
    assert_eq!(code(&o), 0, "it is a report: {}", err(&o));
    assert!(
        out(&o)
            .contains("These handles differ only by case. They keep working and were not renamed:"),
        "{}",
        out(&o)
    );
    assert!(
        out(&o).contains("alice: ALICE, Alice, alice"),
        "{}",
        out(&o)
    );
    assert!(!out(&o).contains("bob"));
    let o = w.op(&["account", "list", "--check", "--lang", "ja"], None);
    assert!(
        out(&o).contains("大文字と小文字だけが違います"),
        "{}",
        out(&o)
    );
    // Every other command works on those accounts.
    assert_eq!(code(&w.op(&["account", "show", "Alice"], None)), 0);
    assert_eq!(
        code(&w.op(&["account", "disable", "ALICE", "--reason", "dup"], None)),
        0
    );
    assert_eq!(code(&w.op(&["account", "enable", "ALICE"], None)), 0);
    assert_eq!(
        code(&w.op(
            &["account", "set-password", "alice", "--password-fd", "3"],
            Some(PASSWORD)
        )),
        0
    );
    // A new handle that collides with one of them is refused.
    assert_eq!(
        code(&w.op(&["account", "create", "bob", "--name", "x"], None)),
        1
    );
    // 505 accounts: `list` prints 500 and says how many more.
    {
        let db = w.db();
        let conn = db.lock().unwrap();
        for i in 0..505 {
            conn.execute(
                "INSERT INTO accounts (user, display_name, handle_key) VALUES (?1, 'x', ?1)",
                (format!("u{i:04}"),),
            )
            .unwrap();
        }
    }
    let o = w.op(&["account", "list"], None);
    assert_eq!(
        out(&o).lines().count(),
        1 + 500 + 1,
        "the header, 500 lines and the 'more' line"
    );
    assert!(
        out(&o).trim_end().ends_with("and 9 more of 509"),
        "{}",
        out(&o).lines().last().unwrap_or("")
    );
}

#[test]
fn every_command_works_on_a_database_as_0_9_0_left_it() {
    let w = world("old");
    // The auth tables exactly as 0.9.0 created them (no new column, no events table), with an account, a token and a session in them.
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
             INSERT INTO sessions VALUES ('s1', 'alice', 1, 4000000000);",
        )
        .unwrap();
    }
    for args in [
        vec!["account", "list"],
        vec!["account", "show", "alice"],
        vec!["account", "create", "bob", "--name", "Bob"],
        vec!["account", "disable", "alice", "--reason", "test"],
        vec!["account", "enable", "alice"],
        vec!["account", "list", "--check"],
    ] {
        let o = w.op(&args, None);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
    }
    // The old token and the old session are what they were (the session ended by the disable, as asked).
    let db = w.db();
    let c = db.lock().unwrap();
    let tokens: i64 = c.query_row("SELECT COUNT(*) FROM tokens WHERE hash = 'h1' AND id IS NOT NULL AND name = '(created before 0.10.0)'", [], |r| r.get(0)).unwrap();
    assert_eq!(
        tokens, 1,
        "the old token is kept, with its new id and the placeholder name"
    );
}

// ------------------------------------------------------------------------------------------------ T5: held lock and concurrency

#[test]
fn a_held_write_lock_gives_the_busy_sentence_exit_5_and_nothing_changes() {
    let w = world("busy");
    let held = w.db();
    held.lock()
        .unwrap()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    let before = w.dump_without_locking_writers();
    let started = Instant::now();
    let o = w.op(&["account", "create", "alice", "--name", "Alice"], None);
    let took = started.elapsed();
    assert_eq!(code(&o), 5, "{}", err(&o));
    assert!(
        err(&o).contains("The database stayed busy for more than 5 seconds"),
        "{}",
        err(&o)
    );
    assert!(out(&o).is_empty());
    assert!(
        took >= Duration::from_secs(4) && took < Duration::from_secs(60),
        "{took:?}"
    );
    let o = w.op(
        &[
            "account", "create", "alice", "--name", "Alice", "--lang", "ja",
        ],
        None,
    );
    assert_eq!(code(&o), 5);
    assert!(
        err(&o).contains("データベースが 5 秒を超えて使用中でした"),
        "{}",
        err(&o)
    );
    held.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(w.dump(), before, "nothing was changed");
    // After the lock is let go the same command works.
    assert_eq!(
        code(&w.op(&["account", "create", "alice", "--name", "Alice"], None)),
        0
    );
}

impl World {
    fn dump_without_locking_writers(&self) -> Vec<String> {
        // Reads are not blocked by a writer in WAL mode.
        self.dump()
    }
}

#[test]
fn many_commands_at_once_beside_a_server_neither_corrupt_nor_deadlock() {
    need_server!();
    let w = world("concurrent");
    w.repository("alice", "forge", "r1");
    let server = start(&w);
    let started = Instant::now();
    let children: Vec<Child> = (0..8)
        .map(|i| {
            let handle = format!("user{i}");
            w.spawn_op(
                &["account", "create", &handle, "--name", &format!("User {i}")],
                &[],
            )
        })
        .collect();
    // While they run, the server keeps answering.
    for _ in 0..5 {
        assert_eq!(server.http("GET", "/", &[], "").0, 200);
    }
    let results: Vec<Output> = children
        .into_iter()
        .map(|c| c.wait_with_output().unwrap())
        .collect();
    for o in &results {
        assert_eq!(code(o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    }
    assert!(started.elapsed() < Duration::from_secs(60));
    let o = w.op(&["account", "list"], None);
    assert_eq!(out(&o).lines().count(), 1 + 8);
    // Each wrote exactly two events (created, and none other: no password).
    let events: i64 = w
        .db()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM account_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 8);
    let _ = server.stop();
}

// ------------------------------------------------------------------------------------------------ the canaries

#[test]
fn a_password_is_nowhere_but_as_an_argon2id_hash_and_the_one_secret_of_the_server_is_never_read() {
    let w = world("canary");
    // While the command waits for its password, look at what a process list and the environment show.
    let mut child = w.spawn_op(
        &[
            "account",
            "create",
            "alice",
            "--name",
            "Alice",
            "--password-fd",
            "3",
        ],
        &[],
    );
    let pid = child.id();
    std::thread::sleep(Duration::from_millis(300));
    if let (Ok(cmdline), Ok(environ)) = (
        std::fs::read(format!("/proc/{pid}/cmdline")),
        std::fs::read(format!("/proc/{pid}/environ")),
    ) {
        let (cmdline, environ) = (
            String::from_utf8_lossy(&cmdline).into_owned(),
            String::from_utf8_lossy(&environ).into_owned(),
        );
        assert!(
            !cmdline.contains(PASSWORD) && !environ.contains(PASSWORD),
            "the password is not in `ps` or the environment"
        );
        assert!(cmdline.contains("--password-fd"), "{cmdline}");
    }
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PASSWORD}\n").as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(code(&o), 0, "{}", err(&o));
    // A password the rule refuses: the error names the rule and never the password.
    let bad = "zq9!x";
    let o = w.op(
        &["account", "set-password", "alice", "--password-fd", "3"],
        Some(bad),
    );
    assert_eq!(code(&o), 1);
    let all = format!("{}{}", out(&o), err(&o));
    assert!(all.contains("too short"), "{all}");
    assert!(
        !all.contains(bad),
        "the error names the rule, not the password: {all}"
    );
    let o = w.op(
        &["account", "set-password", "alice", "--password-fd", "3"],
        Some("alice"),
    );
    assert_eq!(code(&o), 1);
    // Everything every command printed, every event, the whole database, with the canaries.
    let dump = w.dump().join("\n");
    assert!(
        !dump.contains(PASSWORD),
        "the password is stored only as a hash"
    );
    assert!(dump.contains("$argon2id$"));
    assert!(!dump.contains(CANARY));
    for args in [
        vec!["account", "list"],
        vec!["account", "show", "alice"],
        vec!["account", "list", "--check"],
    ] {
        let o = w.op(&args, None);
        let all = format!("{}{}", out(&o), err(&o));
        assert!(
            !all.contains(PASSWORD) && !all.contains(CANARY) && !all.contains("argon2"),
            "{args:?}: {all}"
        );
    }
}

// ------------------------------------------------------------------------------------------------ the `stty` guard

/// A fake `stty` that records its arguments, one call a line; it fails on `-echo` when the file `fail` exists.
fn fake_stty(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.join("stty.log");
    let script = bin.join("stty");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nif [ \"$1\" = \"-echo\" ] && [ -e '{}' ]; then exit 1; fi\nexit 0\n",
            log.display(),
            dir.join("fail").display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

fn script_works() -> bool {
    Command::new("script")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// How long a test waits for a prompt before it fails with what it did receive.
const PROMPT_GUARD: Duration = Duration::from_secs(30);

/// Run `planeter operator account …` on a pseudo-terminal (util-linux `script`), typing `typed` lines; the PATH starts with `bin`.
///
/// **Each line is typed only after its prompt has appeared** in the terminal's transcript (the first after `Password:`, the second after
/// `Password again:`), as a person types after seeing the prompt. The command turns the echo off **before** it prints the first prompt,
/// so "prompt seen" means "echo already off": the order is prompt printed → echo off → input sent. A fixed pause instead raced with the
/// command's start under the load of a whole-workspace run (the typed password was echoed once). If the command ends before a prompt
/// (echo that cannot be turned off asks for nothing), nothing more is typed. If a prompt does not appear within [`PROMPT_GUARD`] the
/// test fails with the transcript it received.
fn on_a_terminal(w: &World, args: &[&str], typed: &[&str], bin: Option<&Path>) -> (i32, String) {
    let transcript = w.dir.join("typescript");
    let _ = std::fs::remove_file(&transcript);
    let mut path = std::env::var("PATH").unwrap_or_default();
    if let Some(b) = bin {
        path = format!("{}:{path}", b.display());
    }
    let inner = format!("{BIN} operator {}", args.join(" "));
    let mut child = Command::new("script")
        .args(["-q", "-e", "-f", "-c", &inner])
        .arg(&transcript)
        .env_clear()
        .env("PATH", path)
        .env("PLANETER_DB", w.dir.join("state.db"))
        .env("HOME", w.dir.join("home"))
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("run script");
    let mut stdin = child.stdin.take().unwrap();
    let seen = || std::fs::read_to_string(&transcript).unwrap_or_default();
    for (i, line) in typed.iter().enumerate() {
        // The (i + 1)-th prompt: "Password:" then "Password again:" (both contain "Password"; the second also "again").
        let prompt = if i == 0 {
            "Password:"
        } else {
            "Password again:"
        };
        let deadline = Instant::now() + PROMPT_GUARD;
        let mut ended = false;
        loop {
            if seen().matches(prompt).count() >= 1 {
                break;
            }
            if child.try_wait().unwrap().is_some() {
                ended = true;
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the prompt {prompt:?} did not appear within {PROMPT_GUARD:?}; the terminal showed: {:?}",
                seen()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        if ended {
            break;
        }
        let _ = stdin.write_all(format!("{line}\n").as_bytes());
        // The next prompt is waited for in the transcript; nothing is assumed about how fast the command reads.
    }
    // Let the command finish reading before its input closes: it ends by itself once it has what it asked for.
    let end = Instant::now() + PROMPT_GUARD;
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if Instant::now() > end {
            drop(stdin);
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    (status.code().unwrap_or(-1), seen())
}

#[test]
fn the_echo_is_turned_off_and_back_on_in_that_order_on_every_way_out() {
    if !script_works() {
        eprintln!("skipping: needs script (util-linux)");
        return;
    }
    let w = world("stty");
    let (bin, log) = fake_stty(&w.dir);
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();
    let reset = || {
        let _ = std::fs::remove_file(&log);
    };

    // 1. success: two equal good entries.
    let (c, _) = on_a_terminal(
        &w,
        &[
            "account",
            "create",
            "alice",
            "--name",
            "A",
            "--password-prompt",
        ],
        &[PASSWORD, PASSWORD],
        Some(&bin),
    );
    assert_eq!(c, 0);
    assert_eq!(calls(), "-echo\necho\n", "on success: off, then on");
    // 2. a refused password (too short): the same order.
    reset();
    let (c, _) = on_a_terminal(
        &w,
        &["account", "set-password", "alice"],
        &["short", "short"],
        Some(&bin),
    );
    assert_eq!(c, 1);
    assert_eq!(
        calls(),
        "-echo\necho\n",
        "on a refused password: off, then on"
    );
    // 3. the second entry does not match: the same order, exit 1, nothing changed.
    reset();
    let before = w.dump();
    let (c, _) = on_a_terminal(
        &w,
        &["account", "set-password", "alice"],
        &[PASSWORD, "something else entirely"],
        Some(&bin),
    );
    assert_eq!(c, 1);
    assert_eq!(
        calls(),
        "-echo\necho\n",
        "when the entries differ: off, then on"
    );
    assert_eq!(w.dump(), before);
    // 4. echo cannot be turned off: no password is asked for, nothing is turned back on that was not turned off, exit 5.
    reset();
    std::fs::write(w.dir.join("fail"), "").unwrap();
    let (c, _) = on_a_terminal(
        &w,
        &["account", "set-password", "alice"],
        &[PASSWORD, PASSWORD],
        Some(&bin),
    );
    assert_eq!(c, 5);
    assert_eq!(
        calls(),
        "-echo\n",
        "echo off failed: asked nothing, no `echo`"
    );
    assert_eq!(w.dump(), before);
}

#[test]
fn with_the_real_stty_what_is_typed_is_not_shown() {
    if !script_works() || Command::new("stty").arg("--version").output().is_err() {
        eprintln!("skipping: needs script and stty");
        return;
    }
    let w = world("realstty");
    let (c, shown) = on_a_terminal(
        &w,
        &[
            "account",
            "create",
            "alice",
            "--name",
            "A",
            "--password-prompt",
        ],
        &[PASSWORD, PASSWORD],
        None,
    );
    assert_eq!(c, 0, "{shown}");
    assert!(
        shown.contains("Password:") && shown.contains("Password again:"),
        "the prompts are shown: {shown}"
    );
    assert!(
        !shown.contains(PASSWORD),
        "the typed password is not echoed: {shown}"
    );
    assert!(!shown.contains("zebra-quartz"), "{shown}");
}

#[test]
fn without_a_terminal_and_without_a_descriptor_a_password_is_refused_not_read_from_a_pipe() {
    let w = world("noterm");
    let o = w.op(
        &[
            "account",
            "create",
            "alice",
            "--name",
            "A",
            "--password-prompt",
        ],
        Some(PASSWORD),
    );
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("Standard input is not a terminal"),
        "{}",
        err(&o)
    );
    let o = w.op(&["account", "set-password", "alice"], Some(PASSWORD));
    assert_eq!(code(&o), 1, "set-password: the account must exist first");
    assert_eq!(
        code(&w.op(&["account", "create", "alice", "--name", "A"], None)),
        0
    );
    let o = w.op(&["account", "set-password", "alice"], Some(PASSWORD));
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("Standard input is not a terminal"),
        "{}",
        err(&o)
    );
    // A descriptor that is not open: said, nothing changed.
    let before = w.dump();
    let o = w.op(
        &["account", "set-password", "alice", "--password-fd", "9"],
        None,
    );
    assert_eq!(code(&o), 2, "{}", err(&o));
    assert!(
        err(&o).contains("The file descriptor 9 cannot be read"),
        "{}",
        err(&o)
    );
    assert_eq!(w.dump(), before);
}

// ------------------------------------------------------------------------------------------------ the sentence is the holder's sentence

#[test]
fn the_sentence_a_command_prints_is_render_event_of_the_event_it_just_wrote_in_both_languages() {
    use planeter_auth::{AccountEventStore, SqliteAccountEventStore};
    use planeter_web::account_events::render_event;
    use planeter_web::catalog::{Lang, T};
    let w = world("sentence");
    w.repository_row("alice", "forge", "r1");
    let t_en = T::new(Lang::from_tag("en").unwrap());
    let t_ja = T::new(Lang::from_tag("ja").unwrap());
    let newest = |handle: &str| {
        SqliteAccountEventStore::new(w.db())
            .unwrap()
            .recent(handle, 1)
            .unwrap()
            .remove(0)
    };
    for (tag, t) in [("en", &t_en), ("ja", &t_ja)] {
        let handle = format!("p{tag}");
        let lang = ["--lang", tag];
        let run = |args: &[&str], pw: Option<&str>| {
            let mut a: Vec<&str> = args.to_vec();
            a.extend(lang);
            let o = w.op(&a, pw);
            assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
            out(&o)
        };
        // Each act in turn: the printed lines contain the rendering of the newest event, character for character.
        for (args, pw) in [
            (
                vec![
                    "account",
                    "create",
                    handle.as_str(),
                    "--name",
                    "P P",
                    "--password-fd",
                    "3",
                ],
                Some(PASSWORD),
            ),
            (
                vec![
                    "account",
                    "set-password",
                    handle.as_str(),
                    "--password-fd",
                    "3",
                ],
                Some(PASSWORD),
            ),
            (
                vec![
                    "account",
                    "grant",
                    "alice/forge",
                    handle.as_str(),
                    "--role",
                    "maintain",
                ],
                None,
            ),
            (
                vec!["account", "revoke", "alice/forge", handle.as_str()],
                None,
            ),
            (
                vec!["account", "disable", handle.as_str(), "--reason", "away"],
                None,
            ),
            (vec!["account", "enable", handle.as_str()], None),
        ] {
            let printed = run(&args, pw);
            let event = newest(&handle);
            let sentence = render_event(t, &event);
            assert!(
                printed.lines().any(|l| l == sentence),
                "{args:?}: {sentence:?} not among\n{printed}"
            );
        }
    }
}
