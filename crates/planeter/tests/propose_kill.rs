//! RFC 004 part 3b, T6: the real server, real prikk and the real sandbox; the server is **SIGKILLed in the middle of a
//! large accept** (a 1,000-patch artifact, as in the measurements). Afterwards:
//!
//! * the store has an **open intent** with the first attempt's list, and no revision;
//! * the next start flags the repository `needs-recovery` (the killed server could not), from the durable lease record;
//! * reads still work; a new proposal is refused with `needs_recovery`; `verify` and `doctor` are clean (U-13);
//! * after the recovery hook (unlock the stale prikk locks, clear the flag; part 4 replaces it), re-sending **the same
//!   artifact** resumes the intent, completes, and records **exactly one** revision, derived from the first attempt's
//!   list (all 1,000 patches), though by then a prefix of them was already pending.
//!
//! Self-skips (printing why) without prikk, prlimit or bubblewrap. Building the 1,000 commits takes about a minute.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    ChangeStore, IntentState, Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteChangeStore,
    SqliteDb, SqliteRepositoryStore, UserId, Visibility,
};

const PATCHES: u32 = 1000;

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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("propose-kill-{name}"));
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

fn pending_count(forge: &Path, keys: &Path) -> usize {
    let out = prikk(forge, keys, &["sync", "pending"]);
    out.lines()
        .filter(|l| {
            let t = l.trim();
            t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .count()
}

struct Server {
    child: Child,
    port: u16,
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
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn planeter");
    let lines = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut said = false;
        for line in lines.lines().map_while(Result::ok) {
            if !said && line.starts_with("planeter listening on ") {
                said = true;
                let _ = tx.send(());
            }
            // The rest of the log is read (so the server never blocks on a full pipe) and discarded.
        }
    });
    rx.recv_timeout(Duration::from_secs(30)).expect("listening");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    Server { child, port }
}

/// One request/response on a fresh connection.
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

/// The text after `"key":` (whitespace skipped), for the flat JSON of the replies and of prikk's reports.
fn after_key<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    Some(text[at..].trim_start().strip_prefix(':')?.trim_start())
}

/// The first string value of `"key": "…"` in `text`.
fn json_str(text: &str, key: &str) -> Option<String> {
    let rest = after_key(text, key)?.strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

/// The first number value of `"key": N` in `text`.
fn json_num(text: &str, key: &str) -> Option<u64> {
    let digits: String = after_key(text, key)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn frame(meta: &str, artifact: &[u8]) -> Vec<u8> {
    let mut out = b"PLANETER-PROPOSE-1\n".to_vec();
    out.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(artifact);
    out
}

#[test]
fn a_server_killed_in_the_middle_of_a_1000_patch_accept_leaves_a_state_the_next_start_reads_and_a_resend_records_exactly_one_revision()
 {
    if !runs("prikk") || !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs prikk, prlimit and bubblewrap (with user namespaces)");
        return;
    }
    let dir = scratch("sigkill");
    let (client, client_keys) = (dir.join("client"), dir.join("client-keys"));
    let (forge, forge_keys) = (dir.join("forge"), dir.join("forge-keys"));
    for d in [&client, &client_keys, &forge, &forge_keys] {
        std::fs::create_dir_all(d).unwrap();
    }
    // The keyed client, and a keyless forge that adopted its key by an archive import.
    prikk(&client, &client_keys, &["setup"]);
    std::fs::write(client.join("README.md"), b"# kill\n").unwrap();
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

    // 1,000 patches in the client, and the artifact that closes the gap to the forge.
    let built = Instant::now();
    // 1,000 queued patches sealed into one block (a block holds many patches; sealing each alone would take minutes).
    for i in 0..PATCHES {
        std::fs::write(
            client.join(format!("f{i}.txt")),
            format!("patch {i}\n").repeat(10),
        )
        .unwrap();
        prikk(
            &client,
            &client_keys,
            &["commit", "--from-worktree", "-m", &format!("p{i}")],
        );
    }
    prikk(&client, &client_keys, &["seal", "--allow-no-audit"]);
    let have = dir.join("forge.have");
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
    let art_path = dir.join("big.art");
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
    eprintln!(
        "built {PATCHES} patches and a {} byte artifact in {:?}",
        art.len(),
        built.elapsed()
    );

    // The server's database: the repository, and a token for its owner.
    {
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
                hash: TokenHasher::hash(&Sha256TokenHasher, "tok-alice"),
                user: UserId::new("alice"),
                scope: Scope {
                    access: ScopeAccess::Write,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }

    // First start: send the artifact on a thread, and kill the server once a prefix of the patches is pending.
    let mut server = start(&dir);
    let port = server.port;
    let body = frame(
        r#"{"title":"a thousand patches","client":"propose-kill/1"}"#,
        &art,
    );
    let sender = std::thread::spawn(move || {
        http(
            port,
            "POST",
            "/api/v1/repos/alice/forge/transport/propose",
            &[
                ("Authorization", "Bearer tok-alice".to_owned()),
                ("Content-Type", "application/octet-stream".to_owned()),
            ],
            &body,
        )
    });
    let watch = Instant::now();
    let prefix = loop {
        let n = pending_count(&forge, &forge_keys);
        if n >= 100 {
            break n;
        }
        assert!(
            watch.elapsed() < Duration::from_secs(180),
            "the accept never got going (pending {n})"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    server.child.kill().unwrap(); // SIGKILL
    let _ = server.child.wait();
    let _ = sender.join();
    let at_kill = pending_count(&forge, &forge_keys);
    eprintln!("killed with {prefix}..{at_kill} of {PATCHES} patches pending");
    assert!(
        at_kill < PATCHES as usize,
        "the accept had already finished ({at_kill} pending): the machine is too fast for the kill window"
    );

    // What the killed process left: an open intent with the first attempt's (empty) list, no revision, no change.
    let repo = RepoId::new("r1");
    {
        let db = SqliteDb::open(dir.join("db.sqlite")).unwrap();
        let changes = SqliteChangeStore::new(db).unwrap();
        let open = changes.open_intents(&repo).unwrap();
        assert_eq!(open.len(), 1, "one open intent");
        assert!(
            open[0].pending_before.is_empty(),
            "the first attempt's list"
        );
        assert_eq!(open[0].state, IntentState::Open);
        assert!(
            changes.list_changes(&repo, None).unwrap().is_empty(),
            "a killed accept records nothing"
        );
        assert!(
            !changes.repo_flags(&repo).unwrap().needs_recovery,
            "the killed process could not set the flag"
        );
    }

    // Second start: the flag is set from the durable lease record (reconcile at start).
    let mut server = start(&dir);
    {
        let db = SqliteDb::open(dir.join("db.sqlite")).unwrap();
        let changes = SqliteChangeStore::new(db).unwrap();
        let f = changes.repo_flags(&repo).unwrap();
        assert!(f.needs_recovery, "the next start flags the repository");
        assert!(
            f.reason
                .unwrap_or_default()
                .contains("stopped during a write")
        );
    }
    let port = server.port;
    // Reads still work; a new proposal is refused with the named reason.
    let (status, _) = http(
        port,
        "GET",
        "/api/v1/repos/alice/forge/transport/advertise",
        &[],
        b"",
    );
    assert_eq!(status, 200);
    let (status, refused) = http(
        port,
        "POST",
        "/api/v1/repos/alice/forge/transport/propose",
        &[("Authorization", "Bearer tok-alice".to_owned())],
        &frame(r#"{"title":"x"}"#, &art),
    );
    assert_eq!(status, 503);
    assert!(String::from_utf8_lossy(&refused).contains("needs_recovery"));
    // verify and doctor are clean after the kill (U-13).
    prikk(&forge, &forge_keys, &["verify"]);
    prikk(&forge, &forge_keys, &["doctor"]);

    // The recovery hook (part 4 replaces it): unlock the stale prikk locks, stop the server, clear the flag and the
    // stale lease record, start again.
    let _ = server.child.kill();
    let _ = server.child.wait();
    let listing = prikk(&forge, &forge_keys, &["unlock"]);
    let locks: Vec<String> = listing
        .split_whitespace()
        .filter(|t| t.ends_with(".lock"))
        .map(str::to_owned)
        .collect();
    assert!(
        !locks.is_empty(),
        "the killed accept left its locks:\n{listing}"
    );
    for l in &locks {
        prikk(&forge, &forge_keys, &["unlock", "--lock", l, "--force"]);
    }
    {
        let db = SqliteDb::open(dir.join("db.sqlite")).unwrap();
        db.lock()
            .unwrap()
            .execute("DELETE FROM lease_holders", [])
            .unwrap();
        SqliteChangeStore::new(db)
            .unwrap()
            .clear_needs_recovery(&repo)
            .unwrap();
    }
    let mut server = start(&dir);
    let started = Instant::now();
    let (status, reply) = http(
        server.port,
        "POST",
        "/api/v1/repos/alice/forge/transport/propose",
        &[
            ("Authorization", "Bearer tok-alice".to_owned()),
            ("Content-Type", "application/octet-stream".to_owned()),
        ],
        &frame(
            r#"{"title":"a thousand patches","client":"propose-kill/1"}"#,
            &art,
        ),
    );
    let reply_text = String::from_utf8_lossy(&reply).into_owned();
    assert_eq!(status, 201, "{reply_text}");
    eprintln!("the resumed accept took {:?}", started.elapsed());
    assert_eq!(
        json_num(&reply_text, "added_count"),
        Some(u64::from(PATCHES)),
        "derived from the first attempt's list: all {PATCHES} patches, though a prefix was pending when the retry began"
    );
    assert_eq!(json_num(&reply_text, "seq"), Some(1));
    assert_eq!(pending_count(&forge, &forge_keys), PATCHES as usize);

    // Exactly one revision, the intent complete after two attempts, nothing open.
    let change_id = json_str(&reply_text, "change_id").expect("change_id");
    let db = SqliteDb::open(dir.join("db.sqlite")).unwrap();
    let changes = SqliteChangeStore::new(db).unwrap();
    assert_eq!(
        changes.revisions_of(&change_id).unwrap().len(),
        1,
        "exactly one revision"
    );
    assert_eq!(changes.list_changes(&repo, None).unwrap().len(), 1);
    assert!(changes.open_intents(&repo).unwrap().is_empty());
    let stored = changes.revisions_of(&change_id).unwrap();
    assert_eq!(stored[0].added_patch_ids.len(), PATCHES as usize);
    // The size of the intent at this scale (its pending_before is empty here; the 20,000-id bound is measured in the
    // store's tests).
    let _ = server.child.kill();
    let _ = server.child.wait();
}
