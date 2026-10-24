//! RFC 011 D-5 (IS-37), end to end: **a killed server leaves none of its children running.**
//!
//! The built `planeter` binary serves a public repository whose "prikk" is a stand-in that answers
//! `--version` and then hangs (so a read is in flight inside the production `bwrap` vector). The server is
//! `SIGKILL`ed with the call in flight, and within a bounded settle time no `bwrap`/prikk process of that
//! call may remain. Before RFC 011 the `timeout` → `bwrap` tree outlived the server until the call ended
//! (up to the 120 s bound).
//!
//! Self-skips (printed reason) where bubblewrap or `prlimit` cannot run.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, Visibility,
};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("server-kill-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

/// Processes of the call against `root`: the sandbox's command line names it, or it runs in it.
fn procs_of(root: &Path) -> Vec<u32> {
    let root_s = root.to_string_lossy().into_owned();
    let me = std::process::id();
    let mut found = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == me {
            continue;
        }
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        if !matches!(
            comm.trim(),
            "bwrap" | "sleep" | "prlimit" | "timeout" | "prikk" | "sh"
        ) {
            continue;
        }
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let cmd = String::from_utf8_lossy(&cmd).replace('\0', " ");
        let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap_or_default();
        if cmd.contains(&root_s) || cwd == root {
            found.push(pid);
        }
    }
    found
}

fn reap(root: &Path) {
    for pid in procs_of(root) {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

fn wait_for<F: FnMut() -> bool>(limit: Duration, mut ok: F) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    ok()
}

/// Start the server on a port reserved just before (the server announces its configured address, not the
/// bound one), return it and the port. Another process can take a freed port in the gap, so a start that
/// does not end up accepting connections is retried with a new port.
fn start_server(dir: &Path, bin_dir: &Path) -> (Child, u16) {
    for attempt in 1..=5 {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut server = Command::new(env!("CARGO_BIN_EXE_planeter"))
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
            .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
            .env("PLANETER_REPOS_ROOT", dir.join("repos"))
            .env("PLANETER_DB", dir.join("db.sqlite"))
            .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn planeter");
        let stderr = server.stderr.take().unwrap();
        let mut lines = BufReader::new(stderr).lines();
        let mut listening = false;
        for line in lines.by_ref().map_while(Result::ok) {
            if line.starts_with("planeter listening on ") {
                listening = true;
                break;
            }
        }
        // Keep draining: closing the read end would hand the running server a broken pipe on its next log line.
        std::thread::spawn(move || for _ in lines {});
        if listening && TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (server, port);
        }
        eprintln!("server start attempt {attempt} did not accept on port {port}; retrying");
        let _ = server.kill();
        let _ = server.wait();
    }
    panic!("the server never accepted connections");
}

#[test]
fn a_sigkilled_server_leaves_no_bwrap_or_prikk_process_of_an_in_flight_call() {
    if !runs("bwrap") || !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs bubblewrap (with user namespaces) and prlimit");
        return;
    }
    let dir = scratch("kill");
    let repo_dir = dir.join("repo");
    let bin_dir = dir.join("bin");
    // The stand-in repository needs a store directory: admission measures `.prikk` before any prikk call.
    std::fs::create_dir_all(repo_dir.join(".prikk")).unwrap();
    std::fs::write(repo_dir.join(".prikk/FORMAT"), b"7\n").unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();

    // A stand-in prikk: answers the version pin, then hangs for any other verb (the in-flight read).
    let script = bin_dir.join("prikk");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$1\" in --version) echo 'prikk 0.54.1';; *) sleep 60;; esac\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // A public repository record pointing at the (empty) directory the sandbox will bind.
    {
        let db = SqliteDb::open(dir.join("db.sqlite")).expect("db");
        SqliteRepositoryStore::new(db)
            .create(RepositoryRecord {
                repo_id: RepoId::new("r1"),
                owner: Owner::User("alice".into()),
                name: "open".into(),
                visibility: Visibility::Public,
                created_at: 0,
                prikk_format_version: None,
                proposals: Default::default(),
                path: repo_dir.clone(),
            })
            .expect("seed record");
    }

    let (mut server, port) = start_server(&dir, &bin_dir);
    // An anonymous read of the public repository: `log`, which the stand-in never answers.
    let request = std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = s.write_all(
            b"GET /api/v1/repos/alice/open/history HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
    });

    let in_flight = wait_for(Duration::from_secs(15), || !procs_of(&repo_dir).is_empty());
    if !in_flight {
        let _ = server.kill();
        let _ = server.wait();
        reap(&repo_dir);
        panic!("the read never reached the sandbox: no bwrap/sleep process for {repo_dir:?}");
    }
    let before = procs_of(&repo_dir).len();

    server.kill().expect("SIGKILL the server");
    let _ = server.wait();

    // The whole tree goes within a few seconds (it takes milliseconds); the old shape left it running.
    let gone = wait_for(Duration::from_secs(5), || procs_of(&repo_dir).is_empty());
    let left = procs_of(&repo_dir);
    reap(&repo_dir);
    drop(request); // the client thread ends when its socket closes
    assert!(
        gone,
        "{} process(es) of the call survived the server's death ({before} before the kill): {left:?}",
        left.len()
    );
}
