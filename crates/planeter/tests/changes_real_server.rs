//! The review view on the built binary (RFC 005 part 6): the real server, real prikk and the **real sandbox** (the extract's `out` mount) serve
//! the list and a diff of a revision, and the server's whole log of the run holds neither a file name nor any file content, on the success
//! path or on the refusal path. Self-skips (printing why) without prikk, bubblewrap or prlimit.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, UserId,
    Visibility,
};

const NAME_CANARY: &str = "CANARY-NAME-4e91";
const CONTENT_CANARY: &str = "CANARY-CONTENT-7f3a";

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

fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    s.write_all(req.as_bytes()).unwrap();
    let _ = s.write_all(body);
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(raw.len().saturating_sub(4));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, raw.get(split + 4..).unwrap_or(&[]).to_vec())
}

fn frame(meta: &str, artifact: &[u8]) -> Vec<u8> {
    let mut out = b"PLANETER-PROPOSE-1\n".to_vec();
    out.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(artifact);
    out
}

/// The first string value of `"key": "…"` in `text`.
fn json_str(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = text[at..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

fn bearer() -> Vec<(&'static str, String)> {
    vec![("Authorization", "Bearer tok-alice".to_owned())]
}

struct Server {
    child: Child,
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

fn start(dir: &Path) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_planeter"))
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .env("PLANETER_ARTIFACT_DIR", dir.join("artifacts"))
        .env("PLANETER_SPOOL_DIR", dir.join("spool"))
        .env("PLANETER_SPOOL_BUDGET_MB", "64")
        .env("PLANETER_AUTOMATIC_RECOVERY", "off")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let lines = BufReader::new(child.stderr.take().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    let sink = Arc::clone(&log);
    let reader = std::thread::spawn(move || {
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
            sink.lock().unwrap().push(line);
        }
    });
    rx.recv_timeout(Duration::from_secs(30)).expect("listening");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    Server {
        child,
        port,
        log,
        reader: Some(reader),
    }
}

impl Server {
    /// Stop it with SIGTERM (so it prints its last lines) and return its whole log.
    fn stop(mut self) -> Vec<String> {
        let pid = self.child.id().to_string();
        let _ = Command::new("kill").args(["-TERM", &pid]).status();
        let _ = self.child.wait();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        self.log.lock().unwrap().clone()
    }
}

#[test]
fn the_real_sandbox_serves_the_view_and_the_log_holds_no_file_name_and_no_content() {
    if !runs("prikk") || !runs("prlimit") || !runs("bwrap") || !bwrap_works() {
        eprintln!("SKIPPED: prikk, prlimit or a working bubblewrap is not available");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("changes-real-server");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (client, client_keys, forge, forge_keys) = (
        dir.join("client"),
        dir.join("client-keys"),
        dir.join("forge"),
        dir.join("forge-keys"),
    );
    for d in [&client, &client_keys, &forge, &forge_keys] {
        std::fs::create_dir_all(d).unwrap();
    }
    prikk(&client, &client_keys, &["setup"]);
    std::fs::write(client.join("README.md"), b"# kept\n").unwrap();
    prikk(
        &client,
        &client_keys,
        &["commit", "--from-worktree", "-m", "init"],
    );
    prikk(&client, &client_keys, &["seal", "--allow-no-audit"]);
    let archive = dir.join("client.archive");
    prikk(
        &client,
        &client_keys,
        &["archive", "export", archive.to_str().unwrap()],
    );
    prikk(&forge, &forge_keys, &["init", "."]);
    let verify = prikk(
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
    let keys_part = &verify[verify.find("\"maintainer_keys\"").expect("maintainer_keys")..];
    let key_id = json_str(keys_part, "key_id").expect("key_id");
    let public_key = json_str(keys_part, "public_key").expect("public_key");
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
            &public_key,
        ],
    );
    prikk(
        &forge,
        &forge_keys,
        &["archive", "import", archive.to_str().unwrap(), "--adopt"],
    );
    // the work: a file whose NAME and whose CONTENT are canaries
    std::fs::write(
        client.join(format!("{NAME_CANARY}.txt")),
        format!("{CONTENT_CANARY}\nsecond line\n"),
    )
    .unwrap();
    std::fs::write(
        client.join("README.md"),
        format!("# kept\n{CONTENT_CANARY} in a readme\n"),
    )
    .unwrap();
    prikk(
        &client,
        &client_keys,
        &["commit", "--from-worktree", "-m", "the work"],
    );
    prikk(&client, &client_keys, &["seal", "--allow-no-audit"]);
    let have = dir.join("forge.have");
    let art_path = dir.join("work.art");
    prikk(
        &forge,
        &forge_keys,
        &[
            "sync",
            "have",
            "heads/main",
            "--output",
            have.to_str().unwrap(),
        ],
    );
    prikk(
        &client,
        &client_keys,
        &[
            "sync",
            "build",
            "heads/main",
            "--have",
            have.to_str().unwrap(),
            "--output",
            art_path.to_str().unwrap(),
        ],
    );
    let art = std::fs::read(&art_path).unwrap();

    let db = SqliteDb::open(dir.join("db.sqlite")).unwrap();
    SqliteRepositoryStore::new(db.clone())
        .create(RepositoryRecord {
            repo_id: RepoId::new("r1"),
            owner: Owner::User("alice".into()),
            name: "forge".into(),
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
            id: String::new(),
            name: String::new(),
            created_at: None,
            last_used_at: None,
            hash: TokenHasher::hash(&Sha256TokenHasher, "tok-alice"),
            user: UserId::new("alice"),
            scope: Scope {
                access: ScopeAccess::Full,
                repos: RepoScope::All,
                expires_at: None,
            },
        })
        .unwrap();

    let server = start(&dir);
    let mut headers = bearer();
    headers.push(("Content-Type", "application/octet-stream".to_owned()));
    let (status, reply) = http(
        server.port,
        "POST",
        "/api/v1/repos/alice/forge/transport/propose",
        &headers,
        &frame(r#"{"title":"canaries","client":"real-server/1"}"#, &art),
    );
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&reply));
    let id = json_str(&String::from_utf8_lossy(&reply), "proposal_id").expect("proposal_id");

    // the view, under the real sandbox
    let base = format!("/api/v1/repos/alice/forge/proposals/{id}/revisions/1/changes");
    let (status, body) = http(server.port, "GET", &base, &bearer(), b"");
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let text = String::from_utf8_lossy(&body).into_owned();
    assert!(text.contains("\"files_changed\":2"), "{text}");
    // the entry of the canary file: its `file_id` is the key just before its `path`
    let at = text.find(NAME_CANARY).expect("the canary file is listed");
    let head = &text[..at];
    let id_at = head
        .rfind("\"file_id\":\"")
        .expect("a file id before the path")
        + "\"file_id\":\"".len();
    let file_id = &head[id_at..id_at + 64];
    let (status, body) = http(
        server.port,
        "GET",
        &format!("{base}/{file_id}"),
        &bearer(),
        b"",
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert!(
        String::from_utf8_lossy(&body).contains(CONTENT_CANARY),
        "the diff carries the content: that is its job"
    );
    // a refusal: an unknown file, and a revision that does not exist
    assert_eq!(
        http(
            server.port,
            "GET",
            &format!("{base}/{}", "0".repeat(64)),
            &bearer(),
            b""
        )
        .0,
        404
    );
    assert_eq!(
        http(
            server.port,
            "GET",
            &base.replace("revisions/1", "revisions/7"),
            &bearer(),
            b""
        )
        .0,
        404
    );
    // and the not-held path: the forge receives the client's sealed history as a received ref
    let archive2 = dir.join("work.archive");
    prikk(
        &client,
        &client_keys,
        &["archive", "export", archive2.to_str().unwrap()],
    );
    prikk(
        &forge,
        &forge_keys,
        &["archive", "import", archive2.to_str().unwrap()],
    );
    // (a cached answer is still served: it is a pure function of immutable inputs)
    assert_eq!(http(server.port, "GET", &base, &bearer(), b"").0, 200);

    let log = server.stop();
    assert!(!log.is_empty());
    let counters = log
        .iter()
        .find(|l| l.starts_with("planeter: the file-by-file view: "))
        .expect("the counters are printed at stop");
    assert!(
        counters.contains("ok=") && counters.contains("cache_hit="),
        "{counters}"
    );
    for line in &log {
        assert!(
            !line.contains(NAME_CANARY),
            "a file name is in the server log: {line}"
        );
        assert!(
            !line.contains(CONTENT_CANARY),
            "file content is in the server log: {line}"
        );
    }
    eprintln!(
        "the server's log of this run: {} lines, none with a file name or content",
        log.len()
    );
}
