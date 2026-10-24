//! RFC 004 parts 3b and 4: the real server, real prikk and the real sandbox; the server is **SIGKILLed in the middle of
//! an accept**. Afterwards, for each kill:
//!
//! * the store has an **open intent** with the first attempt's list, and no revision;
//! * the next start flags the repository `needs-recovery` (the killed server could not), from the durable lease record;
//!   with automatic recovery off, reads still work, a new proposal is refused with `needs_recovery`, `verify` and
//!   `doctor` are clean (U-13), and the administrator's door shows a plan that may run unattended;
//! * with automatic recovery on, **the next start clears the flag by itself** (the stale locks cleared, `verify` passed,
//!   the dead lease record removed) and writes a recovery record that shows the steps;
//! * re-sending **the same artifact** resumes the intent, completes, and records **exactly one** revision, derived from
//!   the first attempt's list, though by then a prefix of the patches was already pending.
//!
//! Two tests: one over a 1,000-patch artifact killed once (as part 3b), one over a 300-patch artifact killed at roughly
//! 15, 40, 70 and 90 % of the accept (found by watching `sync pending` grow). Self-skip (printing why) without prikk,
//! prlimit or bubblewrap.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    ChangeStore, IntentState, Owner, RecoveryStore, RepoId, RepositoryRecord, RepositoryStore,
    SqliteChangeStore, SqliteDb, SqliteRecoveries, SqliteRepositoryStore, UserId, Visibility,
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

fn start(dir: &Path, automatic_recovery: bool) -> Server {
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
        .env(
            "PLANETER_AUTOMATIC_RECOVERY",
            if automatic_recovery { "on" } else { "off" },
        )
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

// ------------------------------------------------------------------------------------------------ the fixture

/// A keyed client with `patches` queued patches sealed into one block, a keyless forge that adopted its key (kept
/// pristine), and the artifact that closes the gap. Each kill gets a copy of the forge and a database of its own.
struct Fixture {
    dir: PathBuf,
    forge_pristine: PathBuf,
    forge_keys: PathBuf,
    art: Vec<u8>,
    patches: u32,
}

fn build_fixture(name: &str, patches: u32) -> Fixture {
    let dir = scratch(name);
    let (client, client_keys) = (dir.join("client"), dir.join("client-keys"));
    let (forge, forge_keys) = (dir.join("forge-pristine"), dir.join("forge-keys"));
    for d in [&client, &client_keys, &forge, &forge_keys] {
        std::fs::create_dir_all(d).unwrap();
    }
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
    let built = Instant::now();
    // Queued patches are sealed in batches of 1,000 (a queue holds at most that many).
    for i in 0..patches {
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
        if i % 1000 == 999 {
            prikk(&client, &client_keys, &["seal", "--allow-no-audit"]);
        }
    }
    if !patches.is_multiple_of(1000) {
        prikk(&client, &client_keys, &["seal", "--allow-no-audit"]);
    }
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
        "built {patches} patches and a {} byte artifact in {:?}",
        art.len(),
        built.elapsed()
    );
    Fixture {
        dir,
        forge_pristine: forge,
        forge_keys,
        art,
        patches,
    }
}

/// A server directory: a copy of the pristine forge, a database with the repository and the owner's token.
struct Instance {
    dir: PathBuf,
    forge: PathBuf,
}

fn instance(fx: &Fixture, tag: &str) -> Instance {
    let dir = fx.dir.join(format!("inst-{tag}"));
    std::fs::create_dir_all(&dir).unwrap();
    let forge = dir.join("forge");
    let st = Command::new("cp")
        .arg("-a")
        .arg(&fx.forge_pristine)
        .arg(&forge)
        .status()
        .unwrap();
    assert!(st.success());
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
            // `Full`: the token of an administrator (it may use the recovery door).
            scope: Scope {
                access: ScopeAccess::Full,
                repos: RepoScope::All,
                expires_at: None,
            },
        })
        .unwrap();
    Instance { dir, forge }
}

const PROPOSE: &str = "/api/v1/repos/alice/forge/transport/propose";
const RECOVERY: &str = "/api/v1/repos/alice/forge/recovery";

fn bearer() -> Vec<(&'static str, String)> {
    vec![("Authorization", "Bearer tok-alice".to_owned())]
}

fn propose_headers() -> Vec<(&'static str, String)> {
    let mut h = bearer();
    h.push(("Content-Type", "application/octet-stream".to_owned()));
    h
}

fn change_store(inst: &Instance) -> SqliteChangeStore {
    SqliteChangeStore::new(SqliteDb::open(inst.dir.join("db.sqlite")).unwrap()).unwrap()
}

fn flagged(inst: &Instance) -> bool {
    change_store(inst)
        .repo_flags(&RepoId::new("r1"))
        .unwrap()
        .needs_recovery
}

/// What one kill showed.
struct Report {
    pending_at_kill: usize,
    recovery_steps: Vec<String>,
    resumed: Duration,
}

/// Send the fixture's artifact to a server, SIGKILL the server once `kill_at` patches are pending, and walk the whole
/// recovery: restart with automatic recovery off (the state a person would find), then on (the server recovers by
/// itself), then send the same artifact again.
fn kill_and_recover(fx: &Fixture, tag: &str, kill_at: usize) -> Report {
    let inst = instance(fx, tag);
    let repo = RepoId::new("r1");
    let n = fx.patches as usize;

    // First start: send the artifact on a thread, kill the server once a prefix is pending.
    let mut server = start(&inst.dir, true);
    let port = server.port;
    let body = frame(r#"{"title":"killed","client":"propose-kill/1"}"#, &fx.art);
    let sender = std::thread::spawn(move || http(port, "POST", PROPOSE, &propose_headers(), &body));
    let watch = Instant::now();
    loop {
        let pending = pending_count(&inst.forge, &fx.forge_keys);
        if pending >= kill_at {
            break;
        }
        assert!(
            watch.elapsed() < Duration::from_secs(300),
            "the accept never got going (pending {pending})"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    server.child.kill().unwrap(); // SIGKILL
    let _ = server.child.wait();
    let _ = sender.join();
    let pending_at_kill = pending_count(&inst.forge, &fx.forge_keys);
    assert!(
        pending_at_kill < n,
        "the accept had already finished ({pending_at_kill} pending): the kill window was missed"
    );
    eprintln!("[{tag}] killed with {pending_at_kill} of {n} patches pending");

    // What the killed process left: an open intent with the first attempt's (empty) list, no revision, no flag.
    {
        let changes = change_store(&inst);
        let open = changes.open_intents(&repo).unwrap();
        assert_eq!(open.len(), 1, "[{tag}] one open intent");
        assert!(
            open[0].pending_before.is_empty(),
            "[{tag}] the first attempt's list"
        );
        assert_eq!(open[0].state, IntentState::Open);
        assert!(
            changes.list_changes(&repo, None).unwrap().is_empty(),
            "[{tag}] a killed accept records nothing"
        );
        assert!(
            !changes.repo_flags(&repo).unwrap().needs_recovery,
            "[{tag}] the killed process could not set the flag"
        );
    }

    // Second start, automatic recovery OFF: the flag is set from the durable lease record; reads work; a new proposal
    // is refused; the administrator's door shows a plan that may run unattended; verify and doctor are clean.
    let mut server = start(&inst.dir, false);
    {
        let f = change_store(&inst).repo_flags(&repo).unwrap();
        assert!(
            f.needs_recovery,
            "[{tag}] the next start flags the repository"
        );
        assert!(
            f.reason
                .unwrap_or_default()
                .contains("stopped during a write")
        );
    }
    let port = server.port;
    let (status, _) = http(
        port,
        "GET",
        "/api/v1/repos/alice/forge/transport/advertise",
        &[],
        b"",
    );
    assert_eq!(status, 200, "[{tag}] reads work");
    let (status, refused) = http(
        port,
        "POST",
        PROPOSE,
        &bearer(),
        &frame(r#"{"title":"x"}"#, &fx.art),
    );
    assert_eq!(status, 503);
    assert!(
        String::from_utf8_lossy(&refused).contains("needs_recovery"),
        "[{tag}]"
    );
    let (status, plan) = http(port, "GET", RECOVERY, &bearer(), b"");
    let plan = String::from_utf8_lossy(&plan).into_owned();
    assert_eq!(status, 200, "{plan}");
    assert!(
        plan.contains("\"may_run_unattended\":true"),
        "[{tag}] {plan}"
    );
    assert!(
        plan.contains("\"cause\":{\"code\":\"server_stopped\""),
        "[{tag}] {plan}"
    );
    prikk(&inst.forge, &fx.forge_keys, &["verify"]);
    prikk(&inst.forge, &fx.forge_keys, &["doctor"]);
    assert!(
        prikk(&inst.forge, &fx.forge_keys, &["unlock"]).contains(".lock"),
        "[{tag}] the killed accept left its locks"
    );
    let _ = server.child.kill();
    let _ = server.child.wait();

    // Third start, automatic recovery ON: the server clears the flag by itself.
    let mut server = start(&inst.dir, true);
    let end = Instant::now() + Duration::from_secs(120);
    while flagged(&inst) {
        assert!(
            Instant::now() < end,
            "[{tag}] automatic recovery did not clear the flag"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // The flag is cleared by the run's last step; its record is closed an instant later.
    let recoveries =
        SqliteRecoveries::new(SqliteDb::open(inst.dir.join("db.sqlite")).unwrap()).unwrap();
    let closed = Instant::now() + Duration::from_secs(20);
    let runs = loop {
        let runs = recoveries.recent(&repo, 5).unwrap();
        if runs.first().is_some_and(|r| r.outcome.is_some()) || Instant::now() > closed {
            break runs;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(runs.len(), 1, "[{tag}] one recovery record");
    assert_eq!(
        (runs[0].actor.as_str(), runs[0].outcome.as_deref()),
        ("automatic", Some("recovered"))
    );
    let recovery_steps: Vec<String> = runs[0].steps.iter().map(|s| s.name.clone()).collect();
    // An accept leaves `active.lock` at once and `objects.lock` after about a second and a half: one lock or two.
    let forced = runs[0]
        .steps
        .iter()
        .filter(|s| s.name == "unlock-force")
        .count();
    assert!(
        (1..=2).contains(&forced),
        "[{tag}] {forced} locks cleared: {recovery_steps:?}"
    );
    assert!(
        runs[0]
            .steps
            .iter()
            .all(|s| s.outcome.as_deref() == Some("ok")),
        "[{tag}] {recovery_steps:?}"
    );
    assert!(
        prikk(&inst.forge, &fx.forge_keys, &["unlock"]).contains("no locks currently held"),
        "[{tag}]"
    );
    prikk(&inst.forge, &fx.forge_keys, &["verify"]);
    prikk(&inst.forge, &fx.forge_keys, &["doctor"]);
    assert!(
        change_store(&inst).open_intents(&repo).unwrap().len() == 1,
        "[{tag}] recovery never abandons an intent"
    );

    // The same artifact again: it resumes the intent and records exactly one revision, from the first attempt's list.
    let started = Instant::now();
    let (status, reply) = http(
        server.port,
        "POST",
        PROPOSE,
        &propose_headers(),
        &frame(r#"{"title":"killed","client":"propose-kill/1"}"#, &fx.art),
    );
    let resumed = started.elapsed();
    let reply = String::from_utf8_lossy(&reply).into_owned();
    assert_eq!(status, 201, "[{tag}] {reply}");
    assert_eq!(
        json_num(&reply, "added_count"),
        Some(u64::from(fx.patches)),
        "[{tag}] derived from the first attempt's list: all {n} patches, though {pending_at_kill} were pending when the retry began"
    );
    let change_id = json_str(&reply, "proposal_id").expect("proposal_id");
    let changes = change_store(&inst);
    assert_eq!(
        changes.revisions_of(&change_id).unwrap().len(),
        1,
        "[{tag}] exactly one revision"
    );
    assert_eq!(changes.list_changes(&repo, None).unwrap().len(), 1);
    assert!(changes.open_intents(&repo).unwrap().is_empty());
    assert_eq!(pending_count(&inst.forge, &fx.forge_keys), n);
    let _ = server.child.kill();
    let _ = server.child.wait();
    Report {
        pending_at_kill,
        recovery_steps,
        resumed,
    }
}

fn needs_tools() -> bool {
    if !runs("prikk") || !runs("prlimit") || !bwrap_works() {
        eprintln!("skipping: needs prikk, prlimit and bubblewrap (with user namespaces)");
        return true;
    }
    false
}

#[test]
fn a_server_killed_in_the_middle_of_a_1000_patch_accept_recovers_by_itself_and_a_resend_records_exactly_one_revision()
 {
    if needs_tools() {
        return;
    }
    let fx = build_fixture("sigkill-1000", 1000);
    let r = kill_and_recover(&fx, "1000", 100);
    eprintln!(
        "1000 patches: killed at {} pending; recovery steps {:?}; the resumed accept took {:?}",
        r.pending_at_kill, r.recovery_steps, r.resumed
    );
}

#[test]
fn recovery_converges_wherever_a_300_patch_accept_is_killed() {
    if needs_tools() {
        return;
    }
    let fx = build_fixture("sigkill-300", 300);
    let n = fx.patches as usize;
    for pct in [15usize, 40, 70, 90] {
        let r = kill_and_recover(&fx, &format!("{pct}pct"), n * pct / 100);
        eprintln!(
            "{pct}% point: killed at {} of {n} pending; recovery steps {:?}; the resumed accept took {:?}",
            r.pending_at_kill, r.recovery_steps, r.resumed
        );
    }
}
