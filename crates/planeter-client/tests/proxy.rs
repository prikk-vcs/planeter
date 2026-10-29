//! `HTTPS_PROXY` and `NO_PROXY` are honoured (T1 (d)). The program is run as a child process with the variables set (a test
//! cannot set its own environment without `unsafe`); a stand-in prikk answers `--version`, so the command reaches the network.
#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Arc, Mutex};

use common::{Req, Server, ca, full, scratch};

/// A "proxy" that records the first line of each connection and refuses it (502).
fn proxy() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut first = String::new();
            let _ = BufReader::new(&conn).read_line(&mut first);
            log.lock().unwrap().push(first.trim().to_owned());
            let _ = conn.write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (port, seen)
}

fn stand_in_prikk(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("prikk");
    std::fs::write(&p, "#!/bin/sh\necho 'prikk 0.55.0'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn clone(dir: &std::path::Path, server_port: u16, env: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_planeter-client"));
    cmd.args([
        "clone",
        &format!("https://127.0.0.1:{server_port}/a/b"),
        dir.join("dest").to_str().unwrap(),
        "--no-adopt",
        "--cacert",
        ca().to_str().unwrap(),
    ])
    .env("PLANETER_CLIENT_PRIKK", stand_in_prikk(dir))
    .env("PLANETER_CLIENT_STATE_DIR", dir.join("state"))
    .env_remove("HTTPS_PROXY")
    .env_remove("https_proxy")
    .env_remove("NO_PROXY")
    .env_remove("no_proxy")
    .env_remove("ALL_PROXY")
    .env_remove("all_proxy")
    .current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn the_proxy_variable_is_honoured_and_no_proxy_bypasses_it() {
    let dir = scratch("proxy");
    let server = Server::start(
        "good",
        Arc::new(|_: &Req| {
            full(
                200,
                &[("Content-Type", "application/json")],
                br#"{"nothing":"useful"}"#,
            )
        }),
    );
    let (proxy_port, seen) = proxy();
    let via = format!("http://127.0.0.1:{proxy_port}");
    // With the proxy named, the program asks the proxy to open the tunnel (CONNECT host:port) and never reaches the server itself.
    let (code, text) = clone(&dir, server.port, &[("HTTPS_PROXY", &via)]);
    assert_ne!(code, 0, "{text}");
    let lines = seen.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|l| l == &format!("CONNECT 127.0.0.1:{} HTTP/1.1", server.port)),
        "{lines:?}"
    );
    assert!(
        server.seen().is_empty(),
        "the server was reached around the proxy"
    );
    // With NO_PROXY naming the host, the proxy is not used and the server is reached directly (its answer is not an advertisement:
    // the program says so, which proves the request got there).
    let before = seen.lock().unwrap().len();
    let (code, text) = clone(
        &dir,
        server.port,
        &[("HTTPS_PROXY", &via), ("NO_PROXY", "127.0.0.1")],
    );
    assert_eq!(code, 3, "{text}");
    assert!(text.contains("could not be understood"), "{text}");
    assert_eq!(
        seen.lock().unwrap().len(),
        before,
        "the proxy saw a connection it should not have"
    );
    assert!(!server.seen().is_empty());
}
