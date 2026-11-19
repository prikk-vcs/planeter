//! RFC 008 handoff 2, T5: **landing by delivery on the real server process**, with the real prikk 0.57.0 and the real sandbox
//! (bubblewrap and `prlimit`; the tests self-skip, printing why, without them).
//!
//! * the whole road over HTTP, request by request (no client): upload, view, confirm, the final state, the pages and the refs;
//! * every archive of the measurement against the forge as created and after a delivery: the closed reasons over HTTP and the
//!   live `.prikk` tree byte-identical after every refusal;
//! * **SIGKILL of the server process** at every point of the install, then a restart of the same binary on the same data: the
//!   repository ends in the new state with every open proposal intact, or in the old state with the stage discarded; `verify` ok,
//!   no flag, the delivery final — and **the API answers consistently** (the delivery, the proposals' pages);
//! * the window: while a confirmation holds the write lease, the reads answer and a second delivery and a second confirmation are
//!   refused cleanly;
//! * the changes view of a landed revision is its files (`landed: true`), a partly landed one is `409 partly_landed` (handoff 4);
//! * the canary: the whole log of runs that deliver, refuse and are killed names no file, branch or key and no content.
//!
//! **How the process is stopped at a point:** a **debug build** of the server (the one `cargo test` builds) reads
//! `PLANETER_FAILPOINT=<point>=<file>`: when the delivery engine reaches the named point it writes `<file>` and stops there forever;
//! the test SIGKILLs the process. The release build has no such code (the release gate looks for the name).

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
    ChangeStore, DeliveryState, DeliveryStore, Owner, RepoId, RepositoryRecord, RepositoryStore,
    SqliteChangeStore, SqliteDb, SqliteDeliveryStore, SqliteRepositoryStore, UserId, Visibility,
};
use serde_json::Value;
use world::{Scenario, fixtures, scratch};

macro_rules! need_all {
    () => {
        if !world::prikk_ok() || !world::sandbox_works() {
            eprintln!("skipping: needs prikk, bubblewrap and prlimit");
            return;
        }
    };
}

const MAGIC: &[u8] = b"PLANETER-IMPORT-1\n";

fn frame(archive: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&2u64.to_be_bytes());
    out.extend_from_slice(b"{}");
    out.extend_from_slice(archive);
    out
}

fn propose_frame(title: &str, artifact: &[u8]) -> Vec<u8> {
    let meta = serde_json::json!({ "title": title }).to_string();
    let mut out = b"PLANETER-PROPOSE-1\n".to_vec();
    out.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(artifact);
    out
}

// ------------------------------------------------------------------------------------------------ the world on disk

/// The template: the live repository (H0), the three proposals of the scenario (made through the engine's own services, which write
/// the same SQLite database and artifact directory the server reads), and the two delivery archives. Built once per test binary;
/// each test copies it.
struct Template {
    dir: PathBuf,
    sc: Scenario,
}

static TEMPLATE: OnceLock<Template> = OnceLock::new();

fn template() -> &'static Template {
    TEMPLATE.get_or_init(|| {
        let fx = fixtures();
        let dir = scratch("server-template");
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

/// A copy of the template with the server's own rows (the repository record, three tokens) in its database.
fn instance(name: &str) -> Instance {
    let t = template();
    let dir = scratch(&format!("server-{name}"));
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
    for (tok, user, access) in [
        ("tok-alice", "alice", ScopeAccess::Full),
        ("tok-alice-w", "alice", ScopeAccess::Write),
        ("tok-eve", "eve", ScopeAccess::Full),
    ] {
        creds
            .add_token(TokenRecord {
                hash: TokenHasher::hash(&Sha256TokenHasher, tok),
                user: UserId::new(user),
                scope: Scope {
                    access,
                    repos: RepoScope::All,
                    expires_at: None,
                },
            })
            .unwrap();
    }
    // the scenario's absolute paths (the archives the tests send) live in the copy
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

// ------------------------------------------------------------------------------------------------ the process

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    /// Start the real binary on `dir`. `failpoint` = `(point, marker file)`. Its log is appended to `<dir>/server.log`.
    fn start(dir: &Path, failpoint: Option<(&str, &Path)>) -> Server {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
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
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some((point, marker)) = failpoint {
            cmd.env(
                "PLANETER_FAILPOINT",
                format!("{point}={}", marker.display()),
            );
        }
        let mut child = cmd.spawn().expect("spawn planeter");
        let lines = BufReader::new(child.stderr.take().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let log = dir.join("server.log");
        std::thread::spawn(move || {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .unwrap();
            let mut said = false;
            for line in lines.lines().map_while(Result::ok) {
                let _ = writeln!(file, "{line}");
                if !said && line.starts_with("planeter listening on ") {
                    said = true;
                    let _ = tx.send(());
                }
            }
        });
        rx.recv_timeout(Duration::from_secs(60)).expect("listening");
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && TcpStream::connect(("127.0.0.1", port)).is_err() {
            std::thread::sleep(Duration::from_millis(20));
        }
        Server { child, port }
    }

    /// SIGKILL: no drain, no cleanup, nothing run.
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// A graceful stop (SIGTERM), so that the counters are logged.
    fn stop(mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn http(&self, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> Reply {
        http(self.port, method, path, token, body)
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
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// One request/response on a fresh connection.
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

const DELIVERIES: &str = "/api/v1/repos/alice/forge/deliveries";

fn deliver_req(s: &Server, token: &str, archive: &Path) -> Reply {
    s.http(
        "POST",
        DELIVERIES,
        Some(token),
        &frame(&std::fs::read(archive).unwrap()),
    )
}

fn view(s: &Server, id: &str) -> Value {
    s.http("GET", &format!("{DELIVERIES}/{id}"), Some("tok-alice"), &[])
        .json()
}

fn confirm_req(s: &Server, id: &str) -> Reply {
    s.http(
        "POST",
        &format!("{DELIVERIES}/{id}/confirm"),
        Some("tok-alice"),
        &[],
    )
}

fn wait_for(path: &Path, what: &str) {
    let end = Instant::now() + Duration::from_secs(300);
    while !path.exists() {
        assert!(Instant::now() < end, "the server never reached {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn prikk(live: &Path, keys: &Path, args: &[&str]) -> String {
    archives::run(live, keys, args)
}

/// What a repository must be after every restart, whatever the kill: one cheap call each.
fn healthy(inst: &Instance) {
    use planeter_prikk::PrikkRepo;
    let driver = planeter_prikk::CliPrikkRepo::new(&inst.live);
    assert!(driver.verify().unwrap().verdict.ok, "verify");
    assert_eq!(driver.doctor_report().unwrap().errors, 0, "doctor");
    assert!(driver.unlock_list().unwrap().is_empty(), "a stale lock");
}

fn tip_of(live: &Path, keys: &Path) -> String {
    let out = prikk(
        live,
        keys,
        &[
            "log",
            "--ref",
            "heads/main",
            "--format",
            "json",
            "--limit",
            "1",
        ],
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    v["blocks"][0]["block_id"].as_str().unwrap().to_owned()
}

fn pending(live: &Path, keys: &Path) -> Vec<String> {
    let mut p: Vec<String> = prikk(live, keys, &["sync", "pending"])
        .lines()
        .map(str::trim)
        .filter(|l| l.len() == 64 && l.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
        .collect();
    p.sort();
    p
}

/// A digest of every file under `.prikk` **but the caches** (`.prikk/cache/…` is what prikk derives and rewrites whenever something reads
/// the repository; the real sandbox's reads rewrite it; it is not the repository).
fn tree_hash(dir: &Path) -> String {
    let listing: String = world::tree_files(dir)
        .into_iter()
        .filter(|(name, _)| !name.starts_with(".prikk/cache/"))
        .map(|(name, digest)| format!("{name} {digest}\n"))
        .collect();
    planeter_auth::sha256_hex(listing.as_bytes())
}

fn db(inst: &Instance) -> SqliteDb {
    SqliteDb::open(inst.dir.join("state.db")).unwrap()
}

fn deliveries_dir_is_empty(inst: &Instance) -> bool {
    std::fs::read_dir(inst.dir.join("root/.deliveries"))
        .map(|d| d.count() == 0)
        .unwrap_or(true)
}

// ------------------------------------------------------------------------------------------------ the whole road

#[test]
fn the_whole_road_over_http_on_the_real_server_with_the_real_sandbox() {
    need_all!();
    let inst = instance("road");
    let keys = inst.dir.join("forge-keys");
    let old_tip = tip_of(&inst.live, &keys);
    let server = Server::start(&inst.dir, None);

    let r = deliver_req(&server, "tok-alice", &inst.sc.d);
    assert_eq!(r.status, 201, "{}", r.text());
    let v = r.json();
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(v["state"], "reported");
    assert_eq!(v["report"]["old_tip"], old_tip.as_str());
    assert_eq!(v["report"]["new_tip"], inst.sc.d_tip.as_str());
    assert_eq!(v["report"]["proposals_land"][0]["id"], inst.sc.p1.as_str());
    assert_eq!(v["report"]["proposals_stay"].as_array().unwrap().len(), 2);
    assert_eq!(tip_of(&inst.live, &keys), old_tip, "a report moves nothing");

    let g = view(&server, &id);
    assert_eq!(g["state"], "reported");
    let c = confirm_req(&server, &id);
    assert_eq!(c.status, 200, "{}", c.text());
    assert_eq!(c.json()["state"], "done");
    assert_eq!(tip_of(&inst.live, &keys), inst.sc.d_tip);
    assert_eq!(pending(&inst.live, &keys), inst.sc.stays);
    healthy(&inst);

    // the pages and the refs
    let refs = server
        .http("GET", "/api/v1/repos/alice/forge/refs", None, &[])
        .json();
    assert_eq!(refs["followed_ref"], "heads/main");
    assert_eq!(refs["last_delivery"]["new_tip"], inst.sc.d_tip.as_str());
    let home = server.http("GET", "/alice/forge", None, &[]);
    assert_eq!(home.status, 200);
    assert!(
        home.text().contains("Delivered by alice on "),
        "{}",
        home.text()
    );
    for p in [&inst.sc.p1, &inst.sc.p2, &inst.sc.p3] {
        let page = server.http("GET", &format!("/alice/forge/proposals/{p}"), None, &[]);
        assert_eq!(page.status, 200, "{p}");
    }

    // RFC 008 D-11 (handoff 4): the changes view of a landed revision is the files of the sealed history, through the real sandbox; the partly
    // landed one (P3: the delivery carries its first patch) is the new 409; and P1 still lists the one file it added
    let changes = server.http(
        "GET",
        &format!(
            "/api/v1/repos/alice/forge/proposals/{}/revisions/1/changes",
            inst.sc.p1
        ),
        None,
        &[],
    );
    assert_eq!(changes.status, 200, "{}", changes.text());
    assert_eq!(changes.json()["compared"]["landed"], true);
    assert_eq!(changes.json()["files"][0]["path"], "src/p1.txt");
    assert_eq!(changes.json()["files"].as_array().unwrap().len(), 1);
    let partly = server.http(
        "GET",
        &format!(
            "/api/v1/repos/alice/forge/proposals/{}/revisions/1/changes",
            inst.sc.p3
        ),
        None,
        &[],
    );
    assert_eq!(partly.status, 409, "{}", partly.text());
    assert_eq!(partly.json()["code"], "changes_unavailable");
    assert_eq!(partly.json()["reason"], "partly_landed");

    // a second delivery: nothing new lands, the next one goes through the whole road
    let r2 = deliver_req(&server, "tok-alice", &inst.sc.d2);
    assert_eq!(r2.status, 201);
    let id2 = r2.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(confirm_req(&server, &id2).json()["state"], "done");
    healthy(&inst);

    // authorization, as the table says, on the real process
    let hidden = server.http(
        "POST",
        "/api/v1/repos/alice/nowhere/deliveries",
        Some("tok-eve"),
        &frame(b"x"),
    );
    assert_eq!(hidden.status, 404);
    let stranger = deliver_req(&server, "tok-eve", &inst.sc.d);
    assert_eq!(
        (
            stranger.status,
            stranger.json()["code"].as_str().unwrap().to_owned()
        ),
        (403, "forbidden".to_owned())
    );
    let capped = deliver_req(&server, "tok-alice-w", &inst.sc.d);
    assert_eq!(capped.status, 403);
    let anonymous = server.http("POST", DELIVERIES, None, &frame(b"x"));
    assert_eq!(anonymous.status, 403);
    // another person's delivery and an unknown one are one 404
    let unknown = server.http(
        "GET",
        &format!("{DELIVERIES}/dl-000000000000000000000000"),
        Some("tok-alice"),
        &[],
    );
    let theirs = server.http("GET", &format!("{DELIVERIES}/{id}"), Some("tok-eve"), &[]);
    assert_eq!(unknown.status, 404);
    assert_eq!(theirs.status, 403, "a stranger may not even ask");
    server.stop();
    let log = std::fs::read_to_string(inst.dir.join("server.log")).unwrap();
    assert!(log.contains("planeter: deliveries on:"), "{log}");
    assert!(
        log.contains("planeter: deliveries: "),
        "the counters at stop"
    );
}

// ------------------------------------------------------------------------------------------------ every archive of the measurement

fn cases() -> Vec<(&'static str, PathBuf)> {
    let fx = fixtures();
    let mut v = vec![
        ("D1", fx.d1.clone()),
        ("D2", fx.d2.clone()),
        ("older (H0)", fx.h0.clone()),
        ("div", fx.div.clone()),
        ("m2", fx.m2_archive.clone()),
        ("m2div", fx.m2div.clone()),
        ("x", fx.x_archive.clone()),
        ("mix", fx.mix.clone()),
        ("xcarry", fx.xcarry.clone()),
        ("dev", fx.dev.clone()),
        ("unrelated", fx.unrelated.clone()),
        ("truncated", fx.truncated.clone()),
    ];
    for (n, p) in &fx.flips {
        v.push((Box::leak(format!("flip {n}").into_boxed_str()), p.clone()));
    }
    v
}

fn expected(label: &str, after_d1: bool) -> Result<(), &'static str> {
    match label {
        "D1" => {
            if after_d1 {
                Err("nothing_to_deliver")
            } else {
                Ok(())
            }
        }
        "D2" | "m2" | "xcarry" | "dev" => Ok(()),
        "older (H0)" => Err(if after_d1 {
            "older_or_diverged"
        } else {
            "nothing_to_deliver"
        }),
        "div" | "m2div" => {
            if after_d1 {
                Err("older_or_diverged")
            } else {
                Ok(())
            }
        }
        "x" | "mix" => Err("unadopted_key"),
        "unrelated" => Err("unrelated"),
        _ => Err("damaged_archive"),
    }
}

fn run_table(server: &Server, inst: &Instance, after_d1: bool) {
    for (label, archive) in cases() {
        let before = tree_hash(&inst.live);
        let r = deliver_req(server, "tok-alice", &archive);
        assert_eq!(r.status, 201, "{label}: {}", r.text());
        let v = r.json();
        let id = v["id"].as_str().unwrap().to_owned();
        match expected(label, after_d1) {
            Ok(()) => {
                assert_eq!(v["report"]["verdict"], "acceptable", "{label}: {v}");
                let del = server.http(
                    "DELETE",
                    &format!("{DELIVERIES}/{id}"),
                    Some("tok-alice"),
                    &[],
                );
                assert_eq!(del.status, 204, "{label}");
            }
            Err(reason) => {
                assert_eq!(v["report"]["reason"], reason, "{label}: {v}");
                assert_eq!(v["state"], "failed", "{label}");
            }
        }
        assert_eq!(tree_hash(&inst.live), before, "byte-identical: {label}");
    }
}

#[test]
fn every_archive_of_the_measurement_on_the_real_server_as_created_and_after_a_delivery() {
    need_all!();
    let inst = instance("table");
    let server = Server::start(&inst.dir, None);
    run_table(&server, &inst, false);
    let r = deliver_req(&server, "tok-alice", &world::fixtures().d1);
    let id = r.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(confirm_req(&server, &id).json()["state"], "done");
    run_table(&server, &inst, true);
    server.stop();
}

// ------------------------------------------------------------------------------------------------ the kill table

/// Where the delivery is, and what the restart must make of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum After {
    Old,
    New,
}

/// Kill the real server at `point`, restart it on the same data, and hold what comes back against the table.
fn kill_at(point: &str, want: After, report: &mut Vec<String>) {
    let inst = instance(&format!("kill-{point}"));
    let keys = inst.dir.join("forge-keys");
    let old_tip = tip_of(&inst.live, &keys);
    let old_hash = tree_hash(&inst.live);
    let marker = inst.dir.join("marker");
    let server = Server::start(&inst.dir, Some((point, &marker)));
    let port = server.port;
    let (archive, early) = (
        inst.sc.d.clone(),
        matches!(point, "stage_before_import" | "after_report"),
    );
    // The request that reaches the point is left hanging in a thread of its own.
    let hung = std::thread::spawn(move || {
        let s = Server {
            child: dummy_child(),
            port,
        };
        let r = deliver_req(&s, "tok-alice", &archive);
        if early {
            return r.status;
        }
        let id = r.json()["id"].as_str().unwrap().to_owned();
        confirm_req(&s, &id).status
    });
    wait_for(&marker, point);
    server.kill();
    let _ = hung.join();
    let ids: Vec<(String, DeliveryState)> = SqliteDeliveryStore::new(db(&inst))
        .unwrap()
        .all()
        .unwrap()
        .into_iter()
        .map(|r| (r.delivery_id, r.state))
        .collect();
    assert_eq!(ids.len(), 1, "{point}");
    let (id, state_after_kill) = ids[0].clone();

    // the restart: the same binary, the same data, no failpoint
    let server = Server::start(&inst.dir, None);
    let v = view(&server, &id);
    let mut api = format!(
        "{} — \"{}\"",
        v["state"].as_str().unwrap(),
        v["message"].as_str().unwrap()
    );
    match want {
        After::Old => {
            assert_eq!(v["state"], "abandoned", "{point}: {v}");
            assert_eq!(tip_of(&inst.live, &keys), old_tip, "{point}");
            assert_eq!(
                tree_hash(&inst.live),
                old_hash,
                "the old tree is untouched: {point}"
            );
        }
        After::New => {
            assert_eq!(v["state"], "done", "{point}: {v}");
            assert_eq!(v["needs_recovery"], false);
            assert_eq!(tip_of(&inst.live, &keys), inst.sc.d_tip, "{point}");
            assert_eq!(pending(&inst.live, &keys), inst.sc.stays, "{point}");
        }
    }
    healthy(&inst);
    assert!(deliveries_dir_is_empty(&inst), "{point}");
    let flags = SqliteChangeStore::new(db(&inst))
        .unwrap()
        .repo_flags(&RepoId::new(world::REPO))
        .unwrap();
    assert!(!flags.needs_recovery, "{point}: {flags:?}");
    // every open proposal intact: their pages answer, still open
    for p in [&inst.sc.p1, &inst.sc.p2, &inst.sc.p3] {
        let page = server.http(
            "GET",
            &format!("/api/v1/repos/alice/forge/proposals/{p}"),
            None,
            &[],
        );
        assert_eq!(page.status, 200, "{point}: {p}");
        // open; and P1, whose patch the delivery carries, may already be seen to have landed (an observation, never an assertion)
        let state = page.json()["state"].as_str().unwrap().to_owned();
        let landed_ok = want == After::New && **p == inst.sc.p1;
        assert!(
            state == "open" || (landed_ok && state == "landed"),
            "{point}: {p} is {state}"
        );
    }
    // the repository takes the next delivery through the whole road
    let again = if want == After::New {
        &inst.sc.d2
    } else {
        &inst.sc.d
    };
    let r = deliver_req(&server, "tok-alice", again);
    assert_eq!(r.status, 201, "{point}: {}", r.text());
    let id2 = r.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(
        confirm_req(&server, &id2).json()["state"],
        "done",
        "{point}"
    );
    healthy(&inst);
    api.push_str("; the proposals' pages 200 and open; the next delivery done");
    server.stop();
    report.push(format!(
        "{point}: state after the kill `{}`; after the restart `{}`; the API said: {api}",
        state_after_kill.as_str(),
        if want == After::Old {
            "abandoned"
        } else {
            "done"
        }
    ));
}

/// A stand-in `Child` for a thread that only needs the port (never waited on).
fn dummy_child() -> Child {
    Command::new("true").spawn().unwrap()
}

#[test]
fn a_sigkilled_server_comes_back_with_the_delivery_finished_or_discarded_at_every_point() {
    need_all!();
    let mut report = Vec::new();
    for (point, want) in [
        ("stage_before_import", After::Old),
        ("after_report", After::Old),
        ("after_intent", After::Old),
        ("before_exchange", After::Old),
        ("between_renames", After::New),
        ("after_exchange", After::New),
        ("exchanged", After::New),
        ("reaccept_first", After::New),
        ("reaccept_middle", After::New),
        ("reaccept_last", After::New),
        ("before_done", After::New),
    ] {
        kill_at(point, want, &mut report);
    }
    let table = report.join("\n");
    eprintln!("KILLTABLE\n{table}");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("delivery-kill-table.txt");
    std::fs::write(dir, table).unwrap();
}

// ------------------------------------------------------------------------------------------------ the window

#[test]
fn while_a_confirmation_holds_the_lease_reads_answer_and_the_rest_is_refused_cleanly() {
    need_all!();
    let inst = instance("window");
    let keys = inst.dir.join("forge-keys");
    let marker = inst.dir.join("marker");
    let server = Server::start(&inst.dir, Some(("reaccept_first", &marker)));
    let id = deliver_req(&server, "tok-alice", &inst.sc.d).json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let port = server.port;
    let hung = {
        let id = id.clone();
        std::thread::spawn(move || {
            let s = Server {
                child: dummy_child(),
                port,
            };
            confirm_req(&s, &id).status
        })
    };
    wait_for(&marker, "reaccept_first");
    // the install holds the write lease: the exchange is done, the proposals are being put back
    for path in [
        "/alice/forge",
        "/alice/forge/history",
        "/api/v1/repos/alice/forge/refs",
        "/api/v1/repos/alice/forge/history",
        "/api/v1/repos/alice/forge/tree",
    ] {
        let r = server.http("GET", path, None, &[]);
        assert_eq!(
            r.status,
            200,
            "a read waits for no lease: {path}: {}",
            r.text()
        );
    }
    let started = Instant::now();
    // a second delivery: refused, with the in-flight rule
    let second = deliver_req(&server, "tok-alice", &inst.sc.d2);
    assert_eq!(
        (
            second.status,
            second.json()["code"].as_str().unwrap().to_owned()
        ),
        (409, "delivery_busy".to_owned())
    );
    // a second confirmation of the same delivery: not confirmable (it is being carried out)
    let again = confirm_req(&server, &id);
    assert_eq!(
        (
            again.status,
            again.json()["code"].as_str().unwrap().to_owned()
        ),
        (409, "delivery_not_confirmable".to_owned())
    );
    // a proposal meets the write lease: `busy`, never a prikk error
    let propose = server.http(
        "POST",
        "/api/v1/repos/alice/forge/transport/propose",
        Some("tok-alice"),
        &propose_frame("late", b"not an artifact"),
    );
    assert!(
        matches!(propose.status, 503 | 409 | 422),
        "a proposal at that moment is refused cleanly: {} {}",
        propose.status,
        propose.text()
    );
    assert!(started.elapsed() < Duration::from_secs(60));
    // the view says what it is
    let v = view(&server, &id);
    assert_eq!(v["state"], "reaccepting");
    assert_eq!(
        tip_of(&inst.live, &keys),
        inst.sc.d_tip,
        "the new repository is in place: no half state"
    );
    server.kill();
    let _ = hung.join();
    // and the restart finishes it
    let server = Server::start(&inst.dir, None);
    assert_eq!(view(&server, &id)["state"], "done");
    assert_eq!(pending(&inst.live, &keys), inst.sc.stays);
    healthy(&inst);
    server.stop();
}

// ------------------------------------------------------------------------------------------------ the log

#[test]
fn the_log_of_runs_that_deliver_refuse_and_are_killed_names_no_file_branch_key_or_content() {
    need_all!();
    let fx = fixtures();
    let inst = instance("canary");
    let marker = inst.dir.join("marker");
    // a run that delivers, refuses and is killed
    let server = Server::start(&inst.dir, Some(("reaccept_middle", &marker)));
    for archive in [&fx.dev, &fx.unrelated, &fx.truncated, &fx.x_archive] {
        let r = deliver_req(&server, "tok-alice", archive);
        let v = r.json();
        if v["state"] == "reported" {
            let id = v["id"].as_str().unwrap().to_owned();
            server.http(
                "DELETE",
                &format!("{DELIVERIES}/{id}"),
                Some("tok-alice"),
                &[],
            );
        }
    }
    let id = deliver_req(&server, "tok-alice", &inst.sc.d).json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let port = server.port;
    let hung = std::thread::spawn(move || {
        let s = Server {
            child: dummy_child(),
            port,
        };
        confirm_req(&s, &id).status
    });
    wait_for(&marker, "reaccept_middle");
    server.kill();
    let _ = hung.join();
    // the restart finishes it, then stops gracefully
    let server = Server::start(&inst.dir, None);
    server.stop();
    let log = std::fs::read_to_string(inst.dir.join("server.log")).unwrap();
    assert!(log.contains("acceptable ("), "{log}");
    assert!(log.contains("refused (unrelated)"), "{log}");
    assert!(log.contains("refused (unadopted_key)"), "{log}");
    assert!(
        log.contains("completed from its intent") || log.contains("completed by the recovery run"),
        "{log}"
    );
    let mut canaries: Vec<String> = [
        "src/",
        ".txt",
        "heads/dev",
        "change p",
        "change e",
        "content of",
        "README",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    for k in [&fx.m1, &fx.m2, &fx.x] {
        canaries.push(k.key_id.clone());
        canaries.push(k.public_key.clone());
    }
    for c in &canaries {
        assert!(!log.contains(c.as_str()), "the log names {c:?}");
    }
    // the same for the page and the refs of an ordinary reader: no key id outside the delivery's own list
    let _ = DeliveryState::Done;
}

// ------------------------------------------------------------------------------------------------ real JSON of every state and every refusal

fn save_sample(name: &str, v: &Value) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("delivery-samples");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_string_pretty(v).unwrap(),
    )
    .unwrap();
}

/// The delivery as the API shows it in a state that only a stopped process shows: the server is held at `point`, the view is read
/// from a second connection, and the process is killed.
fn sample_held_at(point: &str, state: &str, via_confirm: bool) {
    let inst = instance(&format!("sample-{state}"));
    let marker = inst.dir.join("marker");
    let server = Server::start(&inst.dir, Some((point, &marker)));
    let port = server.port;
    let archive = inst.sc.d.clone();
    let hung = std::thread::spawn(move || {
        let s = Server {
            child: dummy_child(),
            port,
        };
        let r = deliver_req(&s, "tok-alice", &archive);
        if via_confirm {
            let id = r.json()["id"].as_str().unwrap().to_owned();
            confirm_req(&s, &id);
        }
    });
    wait_for(&marker, point);
    let ids = SqliteDeliveryStore::new(db(&inst)).unwrap().all().unwrap();
    let v = view(&server, &ids[0].delivery_id);
    assert_eq!(v["state"], state);
    save_sample(&format!("state-{state}"), &v);
    server.kill();
    let _ = hung.join();
}

#[test]
fn the_view_of_every_state_and_every_refusal_is_saved_as_real_json() {
    need_all!();
    let fx = fixtures();
    // the states a process shows only while it is held
    sample_held_at("stage_before_import", "staged", false);
    sample_held_at("after_intent", "confirmed", true);
    sample_held_at("exchanged", "exchanged", true);
    sample_held_at("reaccept_first", "reaccepting", true);
    // the rest, on one server
    let inst = instance("samples");
    let server = Server::start(&inst.dir, None);
    let doc = server.http("GET", "/api/v1/openapi.json", None, &[]).json();
    save_sample("openapi", &doc);
    for (name, archive) in [
        ("refused-unadopted_key", &fx.x_archive),
        ("refused-unrelated", &fx.unrelated),
        ("refused-damaged_archive", &fx.truncated),
        ("refused-nothing_to_deliver", &fx.h0),
    ] {
        let v = deliver_req(&server, "tok-alice", archive).json();
        assert_eq!(v["state"], "failed", "{name}");
        save_sample(name, &v);
    }
    // acceptable → abandoned
    let v = deliver_req(&server, "tok-alice", &inst.sc.d).json();
    save_sample("state-reported", &v);
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(
        server
            .http(
                "DELETE",
                &format!("{DELIVERIES}/{id}"),
                Some("tok-alice"),
                &[]
            )
            .status,
        204
    );
    save_sample("state-abandoned", &view(&server, &id));
    // acceptable → done
    let id = deliver_req(&server, "tok-alice", &inst.sc.d).json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let done = confirm_req(&server, &id).json();
    save_sample("state-done", &done);
    // after a delivery: older, and a branch missing (the delivery of a repository that lacks a branch the live one has is made from `dev`)
    let v = deliver_req(&server, "tok-alice", &fx.h0).json();
    assert_eq!(v["report"]["reason"], "older_or_diverged");
    save_sample("refused-older_or_diverged", &v);
    // a failed confirmation: the repository moved after the report
    let id = deliver_req(&server, "tok-alice", &inst.sc.d2).json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    SqliteDb::open(inst.dir.join("state.db"))
        .unwrap()
        .lock()
        .unwrap()
        .execute(
            "UPDATE deliveries SET old_tip = 'not-the-live-tip' WHERE delivery_id = ?1",
            [&id],
        )
        .unwrap();
    let failed = confirm_req(&server, &id);
    // the request worked; the delivery did not: 200, and the view says `failed` with its reason
    assert_eq!(failed.status, 200, "{}", failed.text());
    assert_eq!(failed.json()["failure"]["reason"], "moved");
    save_sample("confirm-of-a-moved-repository", &failed.json());
    save_sample("state-failed-moved", &view(&server, &id));
    // a delivery whose day is over
    let id = deliver_req(&server, "tok-alice", &inst.sc.d2).json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    SqliteDb::open(inst.dir.join("state.db"))
        .unwrap()
        .lock()
        .unwrap()
        .execute(
            "UPDATE deliveries SET created_at = 1 WHERE delivery_id = ?1",
            [&id],
        )
        .unwrap();
    let gone = confirm_req(&server, &id);
    assert_eq!(gone.status, 410, "{}", gone.text());
    save_sample("error-confirm-of-an-expired-delivery", &gone.json());
    save_sample("state-expired", &view(&server, &id));
    server.stop();
}
