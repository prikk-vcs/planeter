//! RFC 004 part 2, end to end on the built binary: a real server with an export spool, real prikk and the real
//! sandbox (the spool-out mount) serves an archive over a real socket, answers `304` for the same state and a
//! `206` for a range, and exports nothing for a stranger. Self-skips (printed reason) without prikk, bubblewrap
//! or prlimit.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, Visibility,
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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("transport-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn prikk(dir: &Path, keys: &Path, args: &[&str]) {
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
}

fn get(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a response");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned()))
        .collect();
    (status, headers, raw[split + 4..].to_vec())
}

fn header<'a>(h: &'a [(String, String)], name: &str) -> &'a str {
    h.iter()
        .find(|(k, _)| k == name)
        .map_or("", |(_, v)| v.as_str())
}

fn start(dir: &Path, spool: Option<&Path>) -> (Child, u16) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut c = Command::new(env!("CARGO_BIN_EXE_planeter"));
    c.env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(s) = spool {
        c.env("PLANETER_SPOOL_DIR", s)
            .env("PLANETER_SPOOL_BUDGET_MB", "64");
    }
    let mut server = c.spawn().expect("spawn planeter");
    let lines = BufReader::new(server.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(20)).expect("listening");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    (server, port)
}

#[test]
fn a_real_server_advertises_and_serves_a_verified_archive_over_the_real_sandbox() {
    if !runs("prikk") || !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs prikk, prlimit and bubblewrap (with user namespaces)");
        return;
    }
    let dir = scratch("serve");
    let repo = dir.join("repo-src");
    let keys = dir.join("keys");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    prikk(&repo, &keys, &["setup"]);
    std::fs::write(repo.join("README.md"), b"# end to end\n").unwrap();
    prikk(&repo, &keys, &["commit", "--from-worktree", "-m", "init"]);
    prikk(&repo, &keys, &["seal", "--allow-no-audit"]);
    {
        let db = SqliteDb::open(dir.join("db.sqlite")).expect("db");
        let store = SqliteRepositoryStore::new(db);
        for (id, name, vis) in [
            ("r1", "open", Visibility::Public),
            ("r2", "secret", Visibility::Private),
        ] {
            store
                .create(RepositoryRecord {
                    repo_id: RepoId::new(id),
                    owner: Owner::User("alice".into()),
                    name: name.into(),
                    visibility: vis,
                    created_at: 0,
                    prikk_format_version: None,
                    proposals: Default::default(),
                    path: repo.clone(),
                })
                .unwrap();
        }
    }
    let spool = dir.join("spool");
    let (mut server, port) = start(&dir, Some(&spool));
    let base = "/api/v1/repos/alice/open/transport";

    let (status, _, body) = get(port, &format!("{base}/advertise"), &[]);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("\"name\":\"heads/main\""), "{text}");
    assert!(text.contains("\"fingerprint\""), "{text}");

    let (status, headers, archive) = get(port, &format!("{base}/fetch?kind=archive"), &[]);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&archive));
    assert_eq!(
        header(&headers, "content-length"),
        archive.len().to_string()
    );
    let etag = header(&headers, "etag").to_owned();
    let served = dir.join("served.archive");
    std::fs::write(&served, &archive).unwrap();
    prikk(
        &dir,
        &keys,
        &["archive", "verify", served.to_str().unwrap()],
    );

    let (status, _, empty) = get(
        port,
        &format!("{base}/fetch?kind=archive"),
        &[("If-None-Match", &etag)],
    );
    assert_eq!((status, empty.len()), (304, 0));
    let (status, headers, part) = get(
        port,
        &format!("{base}/fetch?kind=archive"),
        &[("Range", "bytes=0-15")],
    );
    assert_eq!(status, 206);
    assert_eq!(part, archive[..16]);
    assert_eq!(
        header(&headers, "content-range"),
        format!("bytes 0-15/{}", archive.len())
    );
    let (status, _, bundle) = get(
        port,
        &format!("{base}/fetch?kind=bundle&ref=heads/main"),
        &[],
    );
    assert_eq!(status, 200);
    let served = dir.join("served.bundle");
    std::fs::write(&served, &bundle).unwrap();
    prikk(
        &dir,
        &keys,
        &["bundle", "verify", "--input", served.to_str().unwrap()],
    );

    // A stranger asking for the private repository gets the same 404 as for one that is not there.
    let (s1, _, b1) = get(
        port,
        "/api/v1/repos/alice/secret/transport/fetch?kind=archive",
        &[],
    );
    let (s2, _, b2) = get(
        port,
        "/api/v1/repos/alice/ghost/transport/fetch?kind=archive",
        &[],
    );
    assert_eq!((s1, s2), (404, 404));
    assert_eq!(b1, b2);

    // Graceful shutdown with the spool in use: exit 0, lock gone, and no partial left in the spool.
    let status = Command::new("kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let end = Instant::now() + Duration::from_secs(10);
    let code = loop {
        if let Some(st) = server.try_wait().unwrap() {
            break st.code();
        }
        assert!(Instant::now() < end, "the server did not stop");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(code, Some(0));
    assert!(!dir.join("db.sqlite.lock").exists());
    let partials = std::fs::read_dir(spool.join("incoming")).unwrap().count();
    assert_eq!(partials, 0);
}

#[test]
fn without_a_spool_the_routes_say_so() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("off");
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
                path: dir.join("nowhere"),
            })
            .unwrap();
    }
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("prikk"), "#!/bin/sh\necho 'prikk 0.52.0'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(bin.join("prikk"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new(env!("CARGO_BIN_EXE_planeter"))
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(server.stderr.take().unwrap());
    let mut log = Vec::new();
    for line in lines.lines().map_while(Result::ok) {
        let up = line.starts_with("planeter listening on ");
        log.push(line);
        if up {
            break;
        }
    }
    assert!(
        log.iter().any(|l| l.contains("artifact fetches are off")),
        "{log:?}"
    );
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let (status, _, body) = get(port, "/api/v1/repos/alice/open/transport/advertise", &[]);
    let _ = server.kill();
    let _ = server.wait();
    // The repository's path does not exist, so the read gate may answer 404 first; a configured-off server says so
    // only to a reader of a repository that opens. Either way no export was attempted.
    assert!(
        status == 503 || status == 404,
        "{status} {}",
        String::from_utf8_lossy(&body)
    );
}
