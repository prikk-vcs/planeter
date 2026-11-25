//! RFC 014 D-11, D-13 on **the built server**: `PLANETER_OIDC_ALLOW_PRIVATE_ISSUER` and `PLANETER_SESSION_HOURS` are judged at start (a wrong value
//! refuses the start with a sentence, in English and Japanese), the start-up lines say what is on, the lifetime reaches the cookie of a real
//! sign-in, and the allowance changes whether the server tries a private provider at all (a listener counts the connections). Tests that need
//! bubblewrap or prlimit self-skip, printing why.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use planeter_auth::{SqliteAccountStore, SqliteCredentialStore};
use planeter_store::SqliteDb;

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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("provsettings-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("root")).unwrap();
    // The database exists as the server makes it (the commands never make it).
    let db = SqliteDb::open(dir.join("state.db")).unwrap();
    drop(SqliteAccountStore::new(db.clone()).unwrap());
    drop(SqliteCredentialStore::new(db).unwrap());
    World { dir }
}

impl World {
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
    start_with(w, &[])
}

fn start_with(w: &World, extra: &[(&str, &str)]) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log = w.dir.join("server.log");
    let mut command = Command::new(BIN);
    command
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
        .stderr(Stdio::piped());
    for (k, v) in extra {
        command.env(k, v);
    }
    let mut child = command.spawn().expect("spawn planeter");
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

// ------------------------------------------------------------------------------------------------ the tests

/// Start the server with `extra` and **expect it to refuse**: its exit code and what it said.
fn refused(w: &World, extra: &[(&str, &str)]) -> (i32, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut command = Command::new(BIN);
    command
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
        .stderr(Stdio::piped());
    for (k, v) in extra {
        command.env(k, v);
    }
    let mut child = command.spawn().expect("spawn planeter");
    let end = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if Instant::now() > end {
            let _ = child.kill();
            panic!("the server neither refused nor stopped within 60 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut said = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut said)
        .unwrap();
    (status.code().unwrap_or(-1), said)
}

const PROVIDER: [(&str, &str); 3] = [
    ("PLANETER_OIDC_ISSUER", "https://idp.internal.example:8443"),
    ("PLANETER_OIDC_CLIENT_ID", "planeter"),
    (
        "PLANETER_OIDC_REDIRECT_URI",
        "https://forge.example/login/oidc/callback",
    ),
];

#[test]
fn a_wrong_value_refuses_the_start_with_a_sentence_in_both_languages() {
    let w = world("refusals");
    let ja = [("LC_ALL", "ja_JP.UTF-8")];
    #[allow(clippy::type_complexity)]
    let cases: Vec<(Vec<(&str, &str)>, &str, &str)> = vec![
        (
            vec![("PLANETER_SESSION_HOURS", "0")],
            "must be a whole number of hours from 1 to 720, and it is 0",
            "1 から 720 までの整数の時間にしてください。今の値は 0 です",
        ),
        (
            vec![("PLANETER_SESSION_HOURS", "721")],
            "and it is 721",
            "今の値は 721 です",
        ),
        (
            vec![("PLANETER_SESSION_HOURS", "a week")],
            "and it is a week",
            "今の値は a week です",
        ),
        (
            vec![("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "yes")],
            "must be on or off, and it is yes",
            "今の値は yes です",
        ),
        (
            vec![("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "on")],
            "needs a sign-in provider",
            "プロバイダーが必要です",
        ),
    ];
    for (env, en, jp) in cases {
        let (code, said) = refused(&w, &env);
        assert_eq!(code, 1, "{env:?}: {said}");
        assert!(said.contains(en), "{env:?}: {said}");
        assert!(!said.contains("listening"), "{said}");
        let mut with_ja: Vec<(&str, &str)> = env.clone();
        with_ja.extend(ja);
        let (code, said) = refused(&w, &with_ja);
        assert_eq!(code, 1);
        assert!(said.contains(jp), "{env:?}: {said}");
    }
    // Values that are fine do not refuse (the allowance off by name, and the edges of the range).
    for ok in [
        vec![("PLANETER_SESSION_HOURS", "1")],
        vec![("PLANETER_SESSION_HOURS", "720")],
        vec![("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "off")],
    ] {
        let s = start_with(&w, &ok);
        let _ = s.stop();
    }
}

#[test]
fn the_start_up_lines_say_what_is_on_and_the_default_is_seven_days() {
    let w = world("lines");
    let s = start(&w);
    let log = s.stop();
    assert!(log.contains("planeter: Sign-ins last 168 hours."), "{log}");
    assert!(
        !log.contains("private address"),
        "the allowance is off by default: {log}"
    );

    let mut env = PROVIDER.to_vec();
    env.extend([
        ("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "on"),
        ("PLANETER_SESSION_HOURS", "24"),
    ]);
    let s = start_with(&w, &env);
    let log = s.stop();
    assert!(log.contains("planeter: Sign-ins last 24 hours."), "{log}");
    assert!(
        log.contains("The sign-in provider's host idp.internal.example is allowed to be on a private address (PLANETER_OIDC_ALLOW_PRIVATE_ISSUER=on). Every other outbound address is checked as before."),
        "{log}"
    );
    // The line names a host and a number; nothing secret.
    assert!(!log.contains(CANARY));
    let mut ja_env = env.clone();
    ja_env.push(("LC_ALL", "ja_JP.UTF-8"));
    let log = start_with(&w, &ja_env).stop();
    assert!(
        log.contains("ログイン中の状態は 24 時間続きます。")
            && log.contains(
                "idp.internal.example は、プライベートアドレスにあることが許可されています"
            ),
        "{log}"
    );
}

#[test]
fn the_lifetime_reaches_the_cookie_of_a_real_sign_in() {
    let w = world("cookie");
    // An account with a password, made by the operator's own command (the password is read from a descriptor, as the commands do it).
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
        Some("zebra-quartz-plinth-9137-marmot"),
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    for (hours, max_age) in [
        (None, 168 * 3600),
        (Some("2"), 2 * 3600),
        (Some("720"), 720 * 3600),
    ] {
        let extra: Vec<(&str, &str)> = hours
            .map(|h| vec![("PLANETER_SESSION_HOURS", h)])
            .unwrap_or_default();
        let server = start_with(&w, &extra);
        let csrf = "csrf-token-of-this-test";
        let (status, head, _) = server.http(
            "POST",
            "/login",
            &[
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded".to_owned(),
                ),
                ("Cookie", format!("planeter_csrf={csrf}")),
            ],
            &format!("csrf={csrf}&username=alice&password=zebra-quartz-plinth-9137-marmot"),
        );
        let set_cookie = head
            .lines()
            .find(|l| {
                l.to_ascii_lowercase()
                    .starts_with("set-cookie: planeter_session=")
            })
            .unwrap_or_default()
            .to_owned();
        assert_eq!(status, 303, "{head}");
        assert!(
            set_cookie.contains(&format!("Max-Age={max_age}")),
            "{hours:?}: {set_cookie}"
        );
        let _ = server.stop();
    }
}

#[test]
fn the_allowance_decides_whether_the_server_tries_a_private_provider_at_all() {
    need_server!();
    let w = world("allowance");
    // A listener on loopback that only counts connections (the server's curl will find a TLS server that is not one, or a certificate it does
    // not trust: what matters is whether it *tried*).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let issuer = format!("https://127.0.0.1:{port}");
    let accepted = |l: &std::net::TcpListener| {
        let mut n = 0;
        let end = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < end {
            match l.accept() {
                Ok((s, _)) => {
                    n += 1;
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        n
    };
    let provider = [
        ("PLANETER_OIDC_ISSUER", issuer.as_str()),
        ("PLANETER_OIDC_CLIENT_ID", "planeter"),
        (
            "PLANETER_OIDC_REDIRECT_URI",
            "https://forge.example/login/oidc/callback",
        ),
    ];
    // Off (the default): the guard refuses the loopback address; nothing connects to the listener.
    let off = start_with(&w, &provider);
    let (status, _, body) = off.http("GET", "/login/oidc", &[], "");
    assert_ne!(status, 303, "no redirect to a provider that was refused");
    assert!(!body.is_empty());
    assert_eq!(
        accepted(&listener),
        0,
        "allowance off: the server did not connect to a private provider"
    );
    let _ = off.stop();
    // On, for exactly that host: the server tries (and then fails, because the listener is not a provider).
    let mut env = provider.to_vec();
    env.push(("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "on"));
    let on = start_with(&w, &env);
    let (status, _, _) = on.http("GET", "/login/oidc", &[], "");
    assert_ne!(status, 303);
    assert!(
        accepted(&listener) >= 1,
        "allowance on: the server connected to the provider's own (private) host"
    );
    let _ = on.stop();
}
