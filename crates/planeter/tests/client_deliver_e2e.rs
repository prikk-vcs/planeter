//! RFC 008 handoff 3, end to end: **`planeter-client deliver`, the real client binary, against the real `planeter` server** (its real
//! sandbox) behind a TLS terminator that speaks the test certificate, with the real prikk 0.58.0. The scenario is the measurement's: three
//! open proposals on the forge (P1 lands, P2 stays, P3 lands in part), a maintainer whose own repository continues the project's
//! branch, and the archives of the measurement as repositories (an older one, a divergent one, one signed by a key the forge does not
//! trust).
//!
//! * a **dry run** prints the report the API gives (checked against the API's own document for the same archive) and cancels;
//! * the delivery with `--yes` is carried out: the forge's `heads/main` is the maintainer's tip, **a fresh clone has it**, a proposal
//!   made on top with the real client (`propose`) is accepted on the delivered base, and the three proposals' pages answer;
//! * the same repository again is "Nothing to deliver" (exit 0);
//! * an **older** repository (a rollback), a **divergent** one and one **signed by an unadopted key** are refused in words (exit 1) and the
//!   forge's `heads/main` and its `.prikk` tree do not change;
//! * a `write` token gets the 403 sentence, a token for another repository the 404 sentence, a stranger the 403 sentence;
//! * every transcript is saved (both languages) under `target/tmp/client-deliver-e2e/transcripts/`.
//!
//! The test skips, printing why, without prikk, prlimit, bubblewrap or the built client and terminator.

#[path = "../../planeter-transport/tests/support/archives.rs"]
mod archives;
#[path = "../../planeter-transport/tests/support/delivery_world.rs"]
mod world;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use planeter_auth::{Sha256TokenHasher, SqliteCredentialStore, TokenHasher, TokenRecord};
use planeter_core::{RepoScope, Scope, ScopeAccess};
use planeter_store::{
    Owner, RepoId, RepositoryRecord, RepositoryStore, SqliteDb, SqliteRepositoryStore, UserId,
    Visibility,
};
use serde_json::Value;
use world::{Scenario, fixtures};

const MAGIC: &[u8] = b"PLANETER-IMPORT-1\n";

fn frame(archive: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&2u64.to_be_bytes());
    out.extend_from_slice(b"{}");
    out.extend_from_slice(archive);
    out
}

// ------------------------------------------------------------------------------------------------ the world on disk

struct Template {
    dir: PathBuf,
    sc: Scenario,
}

static TEMPLATE: OnceLock<Template> = OnceLock::new();

fn template() -> &'static Template {
    TEMPLATE.get_or_init(|| {
        let fx = fixtures();
        let dir = world::scratch("client-template");
        world::create_live(&dir, &fx.h0, &[&fx.m1, &fx.m2]);
        let sc = {
            let s = world::open(&dir, None);
            world::scenario(&s, fx)
        };
        Template { dir, sc }
    })
}

struct Instance {
    dir: PathBuf,
    sc: Scenario,
    live: PathBuf,
}

fn instance(name: &str) -> Instance {
    let t = template();
    let dir = world::scratch(&format!("client-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    world::copy_dir(&t.dir, &dir);
    let live = world::repo_path(&dir);
    let db = SqliteDb::open(dir.join("state.db")).unwrap();
    SqliteRepositoryStore::new(db.clone())
        .create(RepositoryRecord {
            repo_id: RepoId::new(world::REPO),
            owner: Owner::User("alice".into()),
            name: "forge".into(),
            visibility: Visibility::Public,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: live.clone(),
        })
        .unwrap();
    let creds = SqliteCredentialStore::new(db).unwrap();
    for (tok, user, access, repos) in [
        ("tok-alice", "alice", ScopeAccess::Full, RepoScope::All),
        ("tok-alice-w", "alice", ScopeAccess::Write, RepoScope::All),
        ("tok-eve", "eve", ScopeAccess::Full, RepoScope::All),
        (
            "tok-alice-elsewhere",
            "alice",
            ScopeAccess::Full,
            RepoScope::Only(vec![RepoId::new("some-other-repository")]),
        ),
    ] {
        creds
            .add_token(TokenRecord {
                id: String::new(),
                name: String::new(),
                created_at: None,
                last_used_at: None,
                hash: TokenHasher::hash(&Sha256TokenHasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access,
                    repos,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    let sc = Scenario {
        p1: t.sc.p1.clone(),
        p2: t.sc.p2.clone(),
        p3: t.sc.p3.clone(),
        stays: t.sc.stays.clone(),
        p2_patches: t.sc.p2_patches.clone(),
        d: dir.join("work/delivery.archive"),
        d2: dir.join("work/delivery2.archive"),
        d_tip: t.sc.d_tip.clone(),
        work: dir.join("work"),
    };
    Instance { dir, sc, live }
}

// ------------------------------------------------------------------------------------------------ the processes

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

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
    rx.recv_timeout(Duration::from_secs(60)).expect("ready");
    child
}

struct Stack {
    server: Child,
    terminator: Child,
    port: u16,
    tls_port: u16,
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = self.terminator.kill();
        let _ = self.terminator.wait();
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

fn start(dir: &Path, terminator_exe: &Path, tls: &Path) -> Stack {
    let port = free_port();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_planeter"));
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("root"))
        .env("PLANETER_DB", dir.join("state.db"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .env("PLANETER_ARTIFACT_DIR", dir.join("artifacts"))
        .env("PLANETER_SPOOL_DIR", dir.join("spool"))
        .env("PLANETER_SPOOL_BUDGET_MB", "512")
        .env("PLANETER_AUTOMATIC_RECOVERY", "on")
        .current_dir(dir);
    let server = spawn_ready(cmd, "planeter listening on ");
    let tls_port = free_port();
    let mut term = Command::new(terminator_exe);
    term.args([
        tls_port.to_string(),
        port.to_string(),
        tls.join("good.pem").display().to_string(),
        tls.join("good.key").display().to_string(),
    ]);
    let terminator = spawn_ready(term, "listening");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && TcpStream::connect(("127.0.0.1", tls_port)).is_err() {
        std::thread::sleep(Duration::from_millis(20));
    }
    Stack {
        server,
        terminator,
        port,
        tls_port,
    }
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
}

/// One request/response on a fresh connection to the server's own port.
fn http(port: u16, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(300))).ok();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nAccept-Language: en\r\n"
    );
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
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
    Reply {
        status,
        body: raw.get(split + 4..).unwrap_or(&[]).to_vec(),
    }
}

fn prikk(dir: &Path, keys: &Path, args: &[&str]) -> String {
    archives::run(dir, keys, args)
}

fn tip_of(dir: &Path, keys: &Path, reference: &str) -> String {
    let out = prikk(
        dir,
        keys,
        &[
            "log", "--ref", reference, "--format", "json", "--limit", "1",
        ],
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    v["blocks"][0]["block_id"].as_str().unwrap().to_owned()
}

/// A digest of every file under `.prikk` but the caches (what prikk derives and rewrites whenever something reads).
fn tree_hash(dir: &Path) -> String {
    let listing: String = world::tree_files(dir)
        .into_iter()
        .filter(|(name, _)| !name.starts_with(".prikk/cache/"))
        .map(|(name, digest)| format!("{name} {digest}\n"))
        .collect();
    planeter_auth::sha256_hex(listing.as_bytes())
}

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Out {
    fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

// ------------------------------------------------------------------------------------------------ the test

#[test]
fn a_maintainer_delivers_with_the_real_client_and_everything_after_it_works() {
    if !world::prikk_ok() || !world::sandbox_works() {
        eprintln!("skipping: needs prikk, bubblewrap and prlimit");
        return;
    }
    let server_exe = PathBuf::from(env!("CARGO_BIN_EXE_planeter"));
    let target = server_exe.parent().unwrap();
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let client_exe = target.join(format!("planeter-client{exe}"));
    let terminator_exe = target.join("examples").join(format!("tls_terminator{exe}"));
    if !client_exe.exists() || !terminator_exe.exists() {
        eprintln!(
            "skipping: needs the built planeter-client and tls_terminator (cargo test --workspace builds them)"
        );
        return;
    }
    let tls =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../planeter-client/tests/fixtures/tls");
    let inst = instance("deliver");
    let fx = fixtures();
    let transcripts = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("client-deliver-e2e")
        .join("transcripts");
    let _ = std::fs::remove_dir_all(&transcripts);
    std::fs::create_dir_all(&transcripts).unwrap();
    let forge_keys = inst.dir.join("forge-keys");

    // the maintainer's own repositories: the one that continues the project's branch (the scenario's `cc2`), and, as repositories, the
    // measurement's older, divergent and unadopted-key archives (copies: the client only reads them, but a copy keeps the fixtures whole)
    let cc2 = archives::Maintainer {
        work: inst.dir.join("work/cc2"),
        keys: inst.dir.join("work/cc2-keys"),
    };
    let copy_of = |name: &str, work: &Path, keys: &Path| -> (PathBuf, PathBuf) {
        let (w, k) = (inst.dir.join(name), inst.dir.join(format!("{name}-keys")));
        world::copy_dir(work, &w);
        world::copy_dir(keys, &k);
        (w, k)
    };
    let (older, older_keys) = copy_of("older", &fx.m1_at_h0, &fx.m1_at_h0_keys);
    let (div, div_keys) = copy_of("div", &fx.dir.join("div"), &fx.dir.join("div-keys"));
    let (dev, dev_keys) = copy_of("dev", &fx.dir.join("dev"), &fx.dir.join("dev-keys"));
    let (unrelated, unrelated_keys) =
        copy_of("unrelated", &fx.dir.join("u"), &fx.dir.join("u-keys"));
    let (unadopted, unadopted_keys) =
        copy_of("unadopted", &fx.dir.join("xb"), &fx.dir.join("xb-keys"));

    let stack = start(&inst.dir, &terminator_exe, &tls);
    let url = format!("https://127.0.0.1:{}/alice/forge", stack.tls_port);
    let api = "/api/v1/repos/alice/forge";
    let state = inst.dir.join("client-state");
    let tmp = inst.dir.join("client-tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let log = std::cell::RefCell::new(String::new());
    let run = |label: &str,
               args: &[&str],
               cwd: &Path,
               home: &Path,
               token: &str,
               lang: Option<&str>|
     -> Out {
        let mut cmd = Command::new(&client_exe);
        cmd.args(args)
            .arg("--cacert")
            .arg(tls.join("ca.pem"))
            .current_dir(cwd)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("PLANETER_TOKEN", token)
            .env("PLANETER_CLIENT_STATE_DIR", &state)
            .env("TMPDIR", &tmp)
            .env_remove("LANG")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .stdin(Stdio::null());
        if let Some(l) = lang {
            cmd.args(["--lang", l]);
        }
        let out = cmd.output().unwrap();
        let o = Out {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        };
        let text = format!(
            "$ planeter-client {} [{}]\n[exit {}]\n{}{}",
            args.join(" "),
            lang.unwrap_or("en"),
            o.code.map_or("?".to_owned(), |c| c.to_string()),
            o.stdout,
            o.stderr
        );
        std::fs::write(transcripts.join(format!("{label}.txt")), &text).unwrap();
        log.borrow_mut().push_str(&text);
        o
    };
    let forge_tip = || tip_of(&inst.live, &forge_keys, "heads/main");
    let old_tip = forge_tip();
    let before_hash = tree_hash(&inst.live);

    // ---- a dry run: the report of the API for the same archive, then a cancel
    let archive = &inst.sc.d2;
    let api_view = {
        let r = http(
            stack.port,
            "POST",
            &format!("{api}/deliveries"),
            Some("tok-alice"),
            &frame(&std::fs::read(archive).unwrap()),
        );
        assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        let id = v["id"].as_str().unwrap().to_owned();
        let d = http(
            stack.port,
            "DELETE",
            &format!("{api}/deliveries/{id}"),
            Some("tok-alice"),
            &[],
        );
        assert_eq!(d.status, 204);
        v
    };
    let new_tip = api_view["report"]["new_tip"].as_str().unwrap().to_owned();
    let r = run(
        "dry-run.en",
        &["deliver", &url, "--dry-run"],
        &cc2.work,
        &cc2.keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    // what the client prints is what the API's document says
    let rep = &api_view["report"];
    assert!(
        r.stdout.contains(api_view["message"].as_str().unwrap()),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains("Delivering replaces this repository's branch heads/main with yours."),
        "the owner's sentence: {}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains(&format!("from {} to {}", &old_tip[..12], &new_tip[..12])),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains(&format!("adds {} changes", rep["new_blocks"])),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains(&format!("(#{} \"P1\")", inst.sc.p1)),
        "P1 lands: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains(&format!("#{} \"P2\"", inst.sc.p2))
            && r.stdout.contains(&format!("#{} \"P3\"", inst.sc.p3)),
        "P2 and P3 stay: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains("Open proposals that stay as they are: 2"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("Dry run: nothing was delivered"),
        "{}",
        r.stdout
    );
    assert_eq!(forge_tip(), old_tip, "a dry run moves nothing");
    assert_eq!(tree_hash(&inst.live), before_hash);
    assert_eq!(
        std::fs::read_dir(&tmp).unwrap().count(),
        0,
        "the client's scratch is gone"
    );
    let r = run(
        "dry-run.ja",
        &["deliver", &url, "--dry-run"],
        &cc2.work,
        &cc2.keys,
        "tok-alice",
        Some("ja"),
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("試行のみです。"), "{}", r.stdout);

    // ---- a delivery that adds a branch (the measurement's `dev`): the report says which branch it adds
    let r = run(
        "dry-run-adds-a-branch.en",
        &["deliver", &url, "--dry-run"],
        &dev,
        &dev_keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("Branches it adds: heads/dev."),
        "{}",
        r.stdout
    );
    let r = run(
        "dry-run-adds-a-branch.ja",
        &["deliver", &url, "--dry-run"],
        &dev,
        &dev_keys,
        "tok-alice",
        Some("ja"),
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("加わるブランチ：heads/dev。"),
        "{}",
        r.stdout
    );
    assert_eq!(forge_tip(), old_tip);

    // ---- a repository with nothing in common with the forge's history
    let r = run(
        "refused-unrelated.en",
        &["deliver", &url, "--yes"],
        &unrelated,
        &unrelated_keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr
            .contains("Your archive has nothing in common with this repository's history."),
        "{}",
        r.stderr
    );
    let r = run(
        "refused-unrelated.ja",
        &["deliver", &url, "--yes"],
        &unrelated,
        &unrelated_keys,
        "tok-alice",
        Some("ja"),
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr
            .contains("アーカイブの履歴には、このリポジトリの履歴と共通するものがありません。"),
        "{}",
        r.stderr
    );
    assert_eq!(forge_tip(), old_tip);

    // ---- the refusals, while the forge is still at its first state: an unadopted key; the others after the delivery
    let r = run(
        "refused-unadopted-key.en",
        &["deliver", &url, "--yes"],
        &unadopted,
        &unadopted_keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr
            .contains("Some of your history is signed by a key this repository does not trust."),
        "{}",
        r.stderr
    );
    assert!(
        r.stderr
            .contains("Only the keys this repository trusts can deliver to it:")
            && r.stderr.contains(&fx.m1.key_id),
        "the keys that can deliver: {}",
        r.stderr
    );
    let r = run(
        "refused-unadopted-key.ja",
        &["deliver", &url, "--yes"],
        &unadopted,
        &unadopted_keys,
        "tok-alice",
        Some("ja"),
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr.contains("引き渡しは受け付けられませんでした。"),
        "{}",
        r.stderr
    );
    assert_eq!(forge_tip(), old_tip);

    // ---- the tokens
    let r = run(
        "403-write-token.en",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice-w",
        None,
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(r.stderr.contains("Delivering needs a token with full access to this repository, from a maintainer. This token cannot deliver."), "{}", r.stderr);
    let r = run(
        "403-write-token.ja",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice-w",
        Some("ja"),
    );
    assert!(r.stderr.contains("引き渡すには、メンテナーの、このリポジトリへの完全なアクセスを許可するトークンが必要です。"), "{}", r.stderr);
    let r = run(
        "403-stranger.en",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-eve",
        None,
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr
            .contains("Delivering needs a token with full access"),
        "a stranger on a public repository: {}",
        r.stderr
    );
    let r = run(
        "404-other-repository.en",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice-elsewhere",
        None,
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert!(
        r.stderr
            .contains("The server has no such repository, or you may not read it."),
        "{}",
        r.stderr
    );
    let r = run(
        "404-other-repository.ja",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice-elsewhere",
        Some("ja"),
    );
    assert!(
        r.stderr.contains("そのリポジトリはサーバーにないか"),
        "{}",
        r.stderr
    );
    assert_eq!(forge_tip(), old_tip);

    // ---- the delivery
    let r = run(
        "done.en",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout
            .contains("The delivery is done: this repository's branch is now yours."),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout
            .contains("Delivered: the project's branch of forge on 127.0.0.1 is now yours"),
        "{}",
        r.stdout
    );
    let delivered = tip_of(&cc2.work, &cc2.keys, "heads/main");
    assert_eq!(delivered, new_tip);
    assert_eq!(
        forge_tip(),
        delivered,
        "the forge's branch is the maintainer's"
    );
    {
        use planeter_prikk::PrikkRepo;
        let driver = planeter_prikk::CliPrikkRepo::new(&inst.live);
        assert!(driver.verify().unwrap().verdict.ok, "verify");
        assert_eq!(driver.doctor_report().unwrap().errors, 0, "doctor");
        assert!(driver.unlock_list().unwrap().is_empty(), "no stale lock");
    }
    assert_eq!(
        std::fs::read_dir(&tmp).unwrap().count(),
        0,
        "the client's scratch is gone"
    );
    for p in [&inst.sc.p1, &inst.sc.p2, &inst.sc.p3] {
        let page = http(
            stack.port,
            "GET",
            &format!("/alice/forge/proposals/{p}"),
            None,
            &[],
        );
        assert_eq!(page.status, 200, "{p}");
    }

    // ---- a fresh clone has the delivered branch and its files
    let reader = inst.dir.join("reader-keys");
    std::fs::create_dir_all(&reader).unwrap();
    let fresh = inst.dir.join("fresh-clone");
    let r = run(
        "clone-after.en",
        &[
            "clone",
            &url,
            fresh.to_str().unwrap(),
            "--trust-key",
            &fx.m1.public_key,
        ],
        &inst.dir,
        &reader,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert_eq!(
        tip_of(&fresh, &reader, "heads/main"),
        delivered,
        "heads/main of a fresh clone is the delivered tip"
    );
    let listing = |dir: &Path, keys: &Path, reference: &str| -> String {
        prikk(dir, keys, &["tree", "--format", "json", "--ref", reference])
    };
    let theirs: Value = serde_json::from_str(&listing(&fresh, &reader, "heads/main")).unwrap();
    let mine: Value = serde_json::from_str(&listing(&cc2.work, &cc2.keys, "heads/main")).unwrap();
    assert_eq!(
        theirs["entries"], mine["entries"],
        "the fresh clone's files are the maintainer's"
    );

    // ---- a contributor proposes on top, with the real client: accepted on the delivered base
    let pc = world::clone_of(&cc2, &inst.dir, "on-top");
    world::step(&pc, "on-top");
    let r = run(
        "propose-on-top.en",
        &["propose", &url, "--title", "on top of the delivery"],
        &pc.work,
        &pc.keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(r.stdout.contains("Proposed as proposal #"), "{}", r.stdout);
    let list = http(
        stack.port,
        "GET",
        &format!("{api}/proposals"),
        Some("tok-alice"),
        &[],
    );
    assert!(
        String::from_utf8_lossy(&list.body).contains("on top of the delivery"),
        "{}",
        String::from_utf8_lossy(&list.body)
    );

    // ---- the same repository again: nothing to deliver, exit 0
    let again_tip = forge_tip();
    let r = run(
        "nothing-to-deliver.en",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout
            .contains("Nothing to deliver: Your archive changes nothing in this repository."),
        "{}",
        r.stdout
    );
    let r = run(
        "nothing-to-deliver.ja",
        &["deliver", &url, "--yes"],
        &cc2.work,
        &cc2.keys,
        "tok-alice",
        Some("ja"),
    );
    assert_eq!(r.code, Some(0), "{}", r.all());
    assert!(
        r.stdout.contains("引き渡すものはありません："),
        "{}",
        r.stdout
    );
    assert_eq!(forge_tip(), again_tip);

    // ---- an older repository (a rollback) and a divergent one: refused in words; the forge does not change
    let hash_now = tree_hash(&inst.live);
    for (label, repo, keys, sentence) in [
        (
            "refused-older",
            &older,
            &older_keys,
            "Your branch does not continue from this repository's branch.",
        ),
        (
            "refused-divergent",
            &div,
            &div_keys,
            "Your branch does not continue from this repository's branch.",
        ),
    ] {
        let r = run(
            &format!("{label}.en"),
            &["deliver", &url, "--yes"],
            repo,
            keys,
            "tok-alice",
            None,
        );
        assert_eq!(r.code, Some(1), "{label}: {}", r.all());
        assert!(r.stderr.contains(sentence), "{label}: {}", r.stderr);
        assert!(
            r.stderr
                .contains("Update your repository from the project's branch"),
            "{label}: what to do: {}",
            r.stderr
        );
        let r = run(
            &format!("{label}.ja"),
            &["deliver", &url, "--yes"],
            repo,
            keys,
            "tok-alice",
            Some("ja"),
        );
        assert_eq!(r.code, Some(1), "{label}: {}", r.all());
        assert!(
            r.stderr.contains("引き渡しは受け付けられませんでした。"),
            "{label}: {}",
            r.stderr
        );
        assert_eq!(
            forge_tip(),
            again_tip,
            "{label}: the forge's branch did not change"
        );
        assert_eq!(
            tree_hash(&inst.live),
            hash_now,
            "{label}: nor did its .prikk tree"
        );
    }
    // the unadopted key again, now that the forge has moved
    let r = run(
        "refused-unadopted-key-after.en",
        &["deliver", &url, "--yes"],
        &unadopted,
        &unadopted_keys,
        "tok-alice",
        None,
    );
    assert_eq!(r.code, Some(1), "{}", r.all());
    assert_eq!(tree_hash(&inst.live), hash_now);

    // ---- the token is in no transcript
    let all = log.borrow();
    for token in ["tok-alice", "tok-alice-w", "tok-eve", "tok-alice-elsewhere"] {
        assert!(!all.contains(token), "the token {token} is in a transcript");
    }
    drop(all);
    drop(stack);
}
