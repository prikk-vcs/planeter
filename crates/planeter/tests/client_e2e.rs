//! RFC 004 part 9, end to end: the real `planeter` server (its real sandbox), behind a TLS terminator that speaks the test
//! certificate, driven by the real `planeter-client` binary and the real prikk. Clone (a wrong key refused; the right one
//! typed on the command line; `--no-adopt`; no terminal and no key), fetch (`304`, then a new state), propose (the result
//! sentence, then the same work again as already recorded), and the token in no output at all.
//!
//! The client binary and the terminator (an example of the `planeter-client` package) are siblings of the server binary in the
//! target directory: `cargo test --workspace` builds them. The test skips, printing why, if they are missing or if prikk,
//! prlimit or bubblewrap (user namespaces) are not usable.

use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    ChangeStore, Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteChangeStore, SqliteDb,
    SqliteRepositoryStore, UserId, Visibility,
};

fn runs(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn bwrap_works() -> bool {
    Command::new("bwrap")
        .args([
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/lib",
            "/lib64",
            "--symlink",
            "usr/bin",
            "/bin",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--unshare-all",
            "/usr/bin/true",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("client-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn prikk(dir: &Path, keys: &Path, args: &[&str]) -> String {
    let out = Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Spawn a child and wait for a line starting with `ready` on its stderr.
fn spawn_ready(mut cmd: Command, ready: &str) -> Child {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let lines = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let ready = ready.to_owned();
    std::thread::spawn(move || {
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            if !said && line.starts_with(&ready) {
                said = true;
                let _ = tx.send(());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(30)).expect("ready");
    child
}

struct Stack {
    server: Child,
    terminator: Child,
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = self.terminator.kill();
        let _ = self.terminator.wait();
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[test]
fn the_client_clones_fetches_and_proposes_against_a_real_server_behind_tls() {
    if !runs("prikk") || !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs prikk, prlimit and bubblewrap (with user namespaces)");
        return;
    }
    let server_exe = PathBuf::from(env!("CARGO_BIN_EXE_planeter"));
    let target = server_exe.parent().unwrap();
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let client_exe = target.join(format!("planeter-client{exe}"));
    let terminator_exe = target.join("examples").join(format!("tls_terminator{exe}"));
    if !client_exe.exists() || !terminator_exe.exists() {
        eprintln!(
            "skipping: needs {} and {} (cargo test --workspace builds them)",
            client_exe.display(),
            terminator_exe.display()
        );
        return;
    }
    let tls =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../planeter-client/tests/fixtures/tls");
    let dir = scratch("flow");

    // The maintainer's own repository (keyed), and the keyless server-side repository that adopted the maintainer's key.
    let client_repo = dir.join("maintainer");
    let client_keys = dir.join("maintainer-keys");
    std::fs::create_dir_all(&client_repo).unwrap();
    std::fs::create_dir_all(&client_keys).unwrap();
    prikk(&client_repo, &client_keys, &["setup"]);
    std::fs::write(client_repo.join("README.md"), b"# end to end\n").unwrap();
    prikk(
        &client_repo,
        &client_keys,
        &["commit", "--from-worktree", "-m", "init"],
    );
    prikk(&client_repo, &client_keys, &["seal", "--allow-no-audit"]);
    let archive = dir.join("maintainer.archive");
    prikk(
        &client_repo,
        &client_keys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    let forge = dir.join("forge");
    let forge_keys = dir.join("forge-keys");
    std::fs::create_dir_all(&forge).unwrap();
    std::fs::create_dir_all(&forge_keys).unwrap();
    prikk(&forge, &forge_keys, &["init", "."]);
    let report = prikk(
        &forge,
        &forge_keys,
        &[
            "archive",
            "verify",
            archive.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    // The report is JSON; the planeter crate has no JSON parser of its own, and the two fields are plain strings.
    let field = |name: &str| -> String {
        let at = report.find(&format!("\"{name}\"")).expect(name);
        let rest = &report[at + name.len() + 2..];
        let start = rest.find('"').unwrap() + 1;
        rest[start..start + rest[start..].find('"').unwrap()].to_owned()
    };
    let (key, key_id) = (field("public_key"), field("key_id"));
    prikk(
        &forge,
        &forge_keys,
        &[
            "trust",
            "maintainer",
            "add",
            "--key-id",
            &key_id,
            "--public-key",
            &key,
        ],
    );
    prikk(
        &forge,
        &forge_keys,
        &["archive", "import", archive.to_str().unwrap(), "--adopt"],
    );

    // The server's database: the repository and alice's token.
    let db_path = dir.join("db.sqlite");
    {
        let db = SqliteDb::open(&db_path).unwrap();
        SqliteRepositoryStore::new(db.clone())
            .create(RepositoryRecord {
                repo_id: RepoId::new("r1"),
                owner: Owner::User("alice".into()),
                name: "app".into(),
                visibility: Visibility::Public,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path: forge.clone(),
            })
            .unwrap();
        SqliteCredentialStore::new(db)
            .unwrap()
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&Sha256TokenHasher, "tok-alice"),
                user: UserId::new("alice"),
                scope: Scope {
                    access: ScopeAccess::Full,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let port = free_port();
    let mut cmd = Command::new(&server_exe);
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", &db_path)
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .env("PLANETER_ARTIFACT_DIR", dir.join("artifacts"))
        .env("PLANETER_SPOOL_DIR", dir.join("spool"))
        .env("PLANETER_SPOOL_BUDGET_MB", "64")
        .current_dir(&dir);
    let server = spawn_ready(cmd, "planeter listening on ");
    let tls_port = free_port();
    let mut term = Command::new(&terminator_exe);
    term.args([
        tls_port.to_string(),
        port.to_string(),
        tls.join("good.pem").display().to_string(),
        tls.join("good.key").display().to_string(),
    ]);
    let terminator = spawn_ready(term, "listening");
    let _stack = Stack { server, terminator };
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", tls_port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let url = format!("https://127.0.0.1:{tls_port}/alice/app");
    let state = dir.join("state");
    let all_output = std::cell::RefCell::new(String::new());
    let run = |args: &[&str], cwd: &Path, home: &Path| -> Out {
        let out = Command::new(&client_exe)
            .args(args)
            .arg("--cacert")
            .arg(tls.join("ca.pem"))
            .current_dir(cwd)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("PLANETER_TOKEN", "tok-alice")
            .env("PLANETER_CLIENT_STATE_DIR", &state)
            .env_remove("LANG")
            .env_remove("LC_ALL")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let o = Out {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        };
        all_output
            .borrow_mut()
            .push_str(&format!("{args:?}\n{}\n{}\n", o.stdout, o.stderr));
        o
    };
    let reader_home = dir.join("reader-keys");
    std::fs::create_dir_all(&reader_home).unwrap();

    // ---- clone: no terminal and no key; a key the repository does not carry; then the right key.
    let target_dir = dir.join("clone-a");
    let r = run(
        &["clone", &url, target_dir.to_str().unwrap()],
        &dir,
        &reader_home,
    );
    assert_eq!(r.code, Some(2), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("no terminal") || r.stderr.contains("There is no terminal"),
        "{}",
        r.stderr
    );
    assert!(!target_dir.exists(), "nothing was cloned");
    let wrong = "0".repeat(64);
    let r = run(
        &[
            "clone",
            &url,
            target_dir.to_str().unwrap(),
            "--trust-key",
            &wrong,
        ],
        &dir,
        &reader_home,
    );
    assert_eq!(r.code, Some(1), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("not one this repository carries"),
        "{}",
        r.stderr
    );
    assert!(!target_dir.exists(), "a wrong key clones nothing");
    let r = run(
        &[
            "clone",
            &url,
            target_dir.to_str().unwrap(),
            "--trust-key",
            &key,
        ],
        &dir,
        &reader_home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("Cloned app into") && r.stdout.contains("1 of 1 keys are trusted"),
        "{}",
        r.stdout
    );
    let trusted = prikk(&target_dir, &reader_home, &["trust", "maintainer", "list"]);
    assert!(trusted.contains(&key_id), "{trusted}");
    prikk(&target_dir, &reader_home, &["verify"]);

    // ---- clone --no-adopt: everything is a received copy and nothing is trusted.
    let plain_dir = dir.join("clone-b");
    let r = run(
        &["clone", &url, plain_dir.to_str().unwrap(), "--no-adopt"],
        &dir,
        &reader_home,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("Nothing is verified or trusted"),
        "{}",
        r.stdout
    );
    let trusted = prikk(&plain_dir, &reader_home, &["trust", "maintainer", "list"]);
    assert!(!trusted.contains(&key_id), "{trusted}");

    // ---- fetch: nothing changed (304), then a new state after a proposal.
    let r = run(&["fetch"], &target_dir, &reader_home);
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("Nothing changed on the server"),
        "{}",
        r.stdout
    );

    // ---- propose: the maintainer commits two patches and proposes them.
    for i in 0..2 {
        std::fs::write(
            client_repo.join(format!("file-{i}.txt")),
            format!("content {i}\n").repeat(10),
        )
        .unwrap();
        prikk(
            &client_repo,
            &client_keys,
            &["commit", "--from-worktree", "-m", &format!("change {i}")],
        );
        prikk(&client_repo, &client_keys, &["seal", "--allow-no-audit"]);
    }
    let r = run(
        &["propose", &url, "--title", "Two files", "--dry-run"],
        &client_repo,
        &client_keys,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("Dry run: nothing was sent") && r.stdout.contains("2 patches"),
        "{}",
        r.stdout
    );
    let changes = || SqliteChangeStore::new(SqliteDb::open(&db_path).unwrap()).unwrap();
    assert!(
        changes()
            .list_changes(&RepoId::new("r1"), None)
            .unwrap()
            .is_empty(),
        "a dry run sends nothing"
    );
    let r = run(
        &["propose", &url, "--title", "Two files"],
        &client_repo,
        &client_keys,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("Proposed as proposal #")
            && r.stdout
                .contains("held for review on this server; no branch has changed"),
        "{}",
        r.stdout
    );
    assert_eq!(
        changes()
            .list_changes(&RepoId::new("r1"), None)
            .unwrap()
            .len(),
        1
    );
    // The same work again: already recorded, nothing new.
    let r = run(
        &["propose", &url, "--title", "Two files"],
        &client_repo,
        &client_keys,
    );
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("already recorded"), "{}", r.stdout);
    assert_eq!(
        changes()
            .list_changes(&RepoId::new("r1"), None)
            .unwrap()
            .len(),
        1
    );

    // ---- fetch again: the server's state moved with the accept, so the new state comes as received copies.
    let r = run(&["fetch"], &target_dir, &reader_home);
    assert_eq!(r.code, Some(0), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("Fetched the server's state"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("Next: review what came"), "{}", r.stdout);
    let again = run(&["fetch"], &target_dir, &reader_home);
    assert!(
        again.stdout.contains("Nothing changed on the server"),
        "{}",
        again.stdout
    );

    // ---- the token appears in no output at all (stdout and stderr of every run).
    assert!(
        !all_output.borrow().contains("tok-alice"),
        "the token was printed"
    );
}
