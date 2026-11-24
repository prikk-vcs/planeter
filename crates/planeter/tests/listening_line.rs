//! IS-70: **"planeter listening on …" is printed after the socket is bound, and only then.**
//!
//! (a) A server whose address is already taken never says it is listening: it says it cannot, exits 1, and removes its lock.
//! (b) A connection made **at once** (no retry, no sleep) at the moment the line is read is accepted, every time, against fresh servers.
//!
//! Self-skips (printed reason) where `prlimit` cannot run (the server's start-up checks need it).

#[allow(dead_code)]
#[path = "common/exec_wait.rs"]
mod exec_wait;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// How many fresh servers (b) starts. A start is about a fifth of a second on an idle host, so the default of 20 costs a few seconds;
/// `PLANETER_LISTENING_LINE_SERVERS=200` (or more) is the long form for a mutation check or a loaded host.
const FRESH_SERVERS_DEFAULT: usize = 20;

fn fresh_servers() -> usize {
    std::env::var("PLANETER_LISTENING_LINE_SERVERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FRESH_SERVERS_DEFAULT)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("listening-line-{name}"));
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

/// A stand-in `prikk` that answers the version pin (the server refuses to start without one).
fn stand_in_bin(dir: &Path) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("prikk");
    std::fs::write(&script, "#!/bin/sh\necho 'prikk 0.58.0'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    exec_wait::wait_until_runs(&script);
    bin_dir
}

fn command(dir: &Path, bin_dir: &Path, port: u16) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_planeter"));
    c.env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    c
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_server_whose_address_is_taken_never_says_it_is_listening_and_leaves_no_lock() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("taken");
    let bin_dir = stand_in_bin(&dir);
    // The test holds the port (bound and listening) for the whole run.
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();

    let out = command(&dir, &bin_dir, port)
        .output()
        .expect("run planeter");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    eprintln!("{stderr}");

    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        !stderr.contains("listening"),
        "a server that could not bind must not say it is listening: {stderr}"
    );
    assert!(
        stderr.contains(&format!("planeter: cannot listen on 127.0.0.1:{port}: ")),
        "{stderr}"
    );
    assert!(
        !stderr.contains("server error"),
        "the failure is reported once, as what it is: {stderr}"
    );
    assert!(
        !dir.join("db.sqlite.lock").exists(),
        "a start that could not listen must not leave its lock file"
    );
    drop(holder);
}

fn start_and_connect_at_once(dir: &Path, bin_dir: &Path) -> Result<Child, String> {
    let port = free_port();
    let mut server = command(dir, bin_dir, port).spawn().expect("spawn planeter");
    let stderr = server.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let mut said = Vec::new();
    let mut listening = false;
    for line in lines.by_ref().map_while(Result::ok) {
        let up = line.starts_with("planeter listening on ");
        said.push(line);
        if up {
            listening = true;
            break;
        }
    }
    // Keep draining: closing the read end would hand the running server a broken pipe on its next log line.
    std::thread::spawn(move || for _ in lines {});
    if !listening {
        let _ = server.kill();
        let _ = server.wait();
        return Err(format!("the server did not say it is listening: {said:?}"));
    }
    // The moment the line was read: one connect, no retry, no sleep.
    match TcpStream::connect(("127.0.0.1", port)) {
        Ok(_) => Ok(server),
        Err(e) => {
            let _ = server.kill();
            let _ = server.wait();
            Err(format!(
                "connect at once after the line was refused: {e}; it said {said:?}"
            ))
        }
    }
}

#[test]
fn a_connection_made_the_moment_the_line_is_read_is_accepted_every_time() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("at-once");
    let bin_dir = stand_in_bin(&dir);
    let total = fresh_servers();
    let mut refused = Vec::new();
    for n in 1..=total {
        match start_and_connect_at_once(&dir, &bin_dir) {
            Ok(mut server) => {
                let _ = server.kill();
                let _ = server.wait();
            }
            Err(why) => refused.push(format!("server {n}: {why}")),
        }
    }
    assert!(
        refused.is_empty(),
        "{} of {total} fresh servers refused a connection made at once:\n{}",
        refused.len(),
        refused.join("\n")
    );
}
