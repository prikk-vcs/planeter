//! The commands against a stub https server that answers like a planeter server (the advertisement, the have-list, the archive and
//! the propose route are built from a real repository made with real prikk; the propose answer is whatever the test chooses), and
//! the real prikk. Skipped, printing why, without prikk. Unix only: the stub server's repositories and prikk's key directories are
//! made with the shell-free `Command`, but the permissions the token test needs are Unix's.
#![cfg(unix)]

mod common;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use common::{Reply, Req, Server, ca, full, scratch};
use planeter_client::catalog::Lang;
use planeter_client::{Ctx, http, run};

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

fn have_prikk() -> bool {
    Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A maintainer's repository, and the keyless "server" repository that adopted its key.
struct World {
    dir: PathBuf,
    maintainer: PathBuf,
    keys: PathBuf,
    forge: PathBuf,
    forge_keys: PathBuf,
    key: String,
    key_id: String,
}

fn field(report: &str, name: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(report).unwrap();
    v["maintainer_keys"][0][name].as_str().unwrap().to_owned()
}

fn world(name: &str) -> World {
    let dir = scratch(name);
    let (maintainer, keys, forge, forge_keys) = (
        dir.join("maintainer"),
        dir.join("maintainer-keys"),
        dir.join("forge"),
        dir.join("forge-keys"),
    );
    for d in [&maintainer, &keys, &forge, &forge_keys] {
        std::fs::create_dir_all(d).unwrap();
    }
    prikk(&maintainer, &keys, &["setup"]);
    std::fs::write(maintainer.join("README.md"), b"# stub\n").unwrap();
    prikk(
        &maintainer,
        &keys,
        &["commit", "--from-worktree", "-m", "init"],
    );
    prikk(&maintainer, &keys, &["seal", "--allow-no-audit"]);
    let archive = dir.join("m.archive");
    prikk(
        &maintainer,
        &keys,
        &["archive", "export", archive.to_str().unwrap()],
    );
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
    let (key, key_id) = (field(&report, "public_key"), field(&report, "key_id"));
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
    World {
        dir,
        maintainer,
        keys,
        forge,
        forge_keys,
        key,
        key_id,
    }
}

impl World {
    fn commit(&self, n: usize, tag: &str) {
        for i in 0..n {
            std::fs::write(
                self.maintainer.join(format!("{tag}-{i}.txt")),
                format!("{tag} {i}\n").repeat(5),
            )
            .unwrap();
            prikk(
                &self.maintainer,
                &self.keys,
                &["commit", "--from-worktree", "-m", &format!("{tag} {i}")],
            );
            prikk(&self.maintainer, &self.keys, &["seal", "--allow-no-audit"]);
        }
    }
    fn tip(&self, repo: &Path, keys: &Path) -> String {
        let v: serde_json::Value = serde_json::from_str(&prikk(
            repo,
            keys,
            &["log", "--format", "json", "--limit", "1"],
        ))
        .unwrap();
        v["blocks"][0]["block_id"].as_str().unwrap().to_owned()
    }
    fn export(&self, repo: &Path, keys: &Path, name: &str) -> Vec<u8> {
        let p = self.dir.join(name);
        let _ = std::fs::remove_file(&p);
        prikk(repo, keys, &["archive", "export", p.to_str().unwrap()]);
        std::fs::read(&p).unwrap()
    }
    fn have(&self) -> Vec<u8> {
        let p = self.dir.join("forge.have");
        let _ = std::fs::remove_file(&p);
        prikk(
            &self.forge,
            &self.forge_keys,
            &[
                "sync",
                "have",
                "heads/main",
                "--output",
                p.to_str().unwrap(),
            ],
        );
        std::fs::read(&p).unwrap()
    }
}

/// What the stub says on the propose route, and what it was sent.
type Propose = Arc<Mutex<Box<dyn FnMut(&Req) -> Reply + Send>>>;

fn stub(
    w: &World,
    archive: Arc<Mutex<Vec<u8>>>,
    etag: Arc<Mutex<String>>,
    tip: String,
    propose: Propose,
) -> Server {
    let have = w.have();
    Server::start(
        "good",
        Arc::new(move |r: &Req| {
            let path = r.path.split('?').next().unwrap_or("");
            match path {
                p if p.ends_with("/transport/advertise") => {
                    let et = etag.lock().unwrap().clone();
                    let adv = serde_json::json!({
                        "fingerprint": "00112233445566778899aabbccddeeff",
                        "refs": [{
                            "name": "heads/main", "state": "published", "kind": "branch", "closed": false,
                            "tip_block": tip, "ref_state_id": "rs-1",
                            "fetch": [
                                {"kind": "bundle", "ref": "heads/main", "size": null, "etag": "\"b\""},
                                {"kind": "have", "ref": "heads/main", "size": have.len(), "etag": "\"h\""}
                            ]
                        }],
                        "archive": {"kind": "archive", "ref": null, "size": null, "etag": et},
                    });
                    full(
                        200,
                        &[("Content-Type", "application/json")],
                        adv.to_string().as_bytes(),
                    )
                }
                p if p.ends_with("/transport/have") => full(200, &[("ETag", "\"h\"")], &have),
                p if p.ends_with("/transport/fetch") => {
                    let et = etag.lock().unwrap().clone();
                    if r.header("if-none-match") == Some(et.as_str()) {
                        full(304, &[("ETag", &et)], b"")
                    } else {
                        full(200, &[("ETag", &et)], &archive.lock().unwrap())
                    }
                }
                p if p.ends_with("/transport/propose") => (propose.lock().unwrap())(r),
                _ => full(404, &[], b"{}"),
            }
        }),
    )
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn client(
    args: &[&str],
    cwd: &Path,
    home: &Path,
    state: &Path,
    stdin: &str,
    interactive: bool,
    extra: &[(&str, &str)],
) -> Run {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut input = Cursor::new(stdin.as_bytes().to_vec());
    let home_s = home.display().to_string();
    let state_s = state.display().to_string();
    let extra: Vec<(String, String)> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let env = move |name: &str| -> Option<String> {
        match name {
            "HOME" | "XDG_CONFIG_HOME" => Some(home_s.clone()),
            "PLANETER_CLIENT_STATE_DIR" => Some(state_s.clone()),
            _ => extra
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone()),
        }
    };
    // The real prikk reads its keys from HOME: the child inherits this process's environment, so give it the right one.
    // (`Command` in the client does not clear the environment; the HOME of the test process is set per call below.)
    let code = {
        let mut ctx = Ctx {
            lang: Lang::En,
            verbose: false,
            out: &mut out,
            err: &mut err,
            env: &env,
            stdin: &mut input,
            interactive,
            cwd: cwd.to_path_buf(),
            prikk_program: Some(home_wrapper(home)),
            timeouts: http::Timeouts::default(),
        };
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        all.push("--cacert".into());
        all.push(ca().display().to_string());
        run(&all, &mut ctx)
    };
    Run {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

/// A one-line shell wrapper that runs the real prikk with HOME set to `home` (prikk finds the person's keys there).
fn home_wrapper(home: &Path) -> std::ffi::OsString {
    let dir = home.join("..").join("wrappers");
    std::fs::create_dir_all(&dir).unwrap();
    let name = format!("prikk-as-{}", home.file_name().unwrap().to_string_lossy());
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nHOME='{h}' XDG_CONFIG_HOME='{h}' exec prikk \"$@\"\n",
            h = home.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.into_os_string()
}

fn setup_propose(
    name: &str,
    reply: Box<dyn FnMut(&Req) -> Reply + Send>,
) -> (World, Server, Propose) {
    let w = world(name);
    let archive = Arc::new(Mutex::new(w.export(
        &w.forge,
        &w.forge_keys,
        "forge.archive",
    )));
    let tip = w.tip(&w.forge, &w.forge_keys);
    let propose: Propose = Arc::new(Mutex::new(reply));
    let server = stub(
        &w,
        archive,
        Arc::new(Mutex::new("\"a1\"".into())),
        tip,
        propose.clone(),
    );
    (w, server, propose)
}

fn url(s: &Server) -> String {
    format!("https://127.0.0.1:{}/alice/app", s.port)
}

fn token_file(dir: &Path) -> PathBuf {
    let p = dir.join("token");
    std::fs::write(&p, "tok-secret\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    p
}

#[test]
fn a_key_typed_whole_is_trusted_a_wrong_one_is_told_so_an_empty_line_skips_and_there_is_no_default_yes()
 {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let w = world("typed");
    let archive = Arc::new(Mutex::new(w.export(&w.maintainer, &w.keys, "all.archive")));
    let server = stub(
        &w,
        archive,
        Arc::new(Mutex::new("\"a1\"".into())),
        w.tip(&w.maintainer, &w.keys),
        Arc::new(Mutex::new(Box::new(|_: &Req| full(500, &[], b"{}")))),
    );
    let (home, state) = (w.dir.join("reader"), w.dir.join("state"));
    std::fs::create_dir_all(&home).unwrap();
    let grouped: String = w
        .key
        .as_bytes()
        .chunks(8)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    // A wrong key first, then the right one in groups: trusted.
    let dest = w.dir.join("clone-typed");
    let r = client(
        &["clone", &url(&server), dest.to_str().unwrap()],
        &w.dir,
        &home,
        &state,
        &format!("nonsense\n{grouped}\n"),
        true,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out.contains(&grouped),
        "the whole key is shown in groups: {}",
        r.out
    );
    assert!(
        r.out.contains("That is not key 1") && r.out.contains("Key 1 is trusted"),
        "{}",
        r.out
    );
    assert!(prikk(&dest, &home, &["trust", "maintainer", "list"]).contains(&w.key_id));
    // An empty line skips the key: nothing is trusted, the clone still lands (as received copies).
    let skipped = w.dir.join("clone-skipped");
    let r = client(
        &["clone", &url(&server), skipped.to_str().unwrap()],
        &w.dir,
        &home,
        &state,
        "\n",
        true,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out.contains("Key 1 is skipped") && r.out.contains("0 of 1 keys are trusted"),
        "{}",
        r.out
    );
    assert!(!prikk(&skipped, &home, &["trust", "maintainer", "list"]).contains(&w.key_id));
    // Two wrong answers skip the key too; an end of input is an empty answer, never "yes".
    let twice = w.dir.join("clone-twice");
    let r = client(
        &["clone", &url(&server), twice.to_str().unwrap()],
        &w.dir,
        &home,
        &state,
        "x\ny\n",
        true,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("0 of 1 keys are trusted"), "{}", r.out);
    // A key that is a prefix of the real one, or the short id, is not the key.
    let prefix = w.dir.join("clone-prefix");
    let r = client(
        &["clone", &url(&server), prefix.to_str().unwrap()],
        &w.dir,
        &home,
        &state,
        &format!("{}\n{}\n", &w.key[..32], w.key_id),
        true,
        &[],
    );
    assert!(r.out.contains("0 of 1 keys are trusted"), "{}", r.out);
}

#[test]
fn fetch_lands_a_newer_state_as_received_copies_and_refuses_a_history_that_does_not_continue() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let w = world("fetch");
    let archive = Arc::new(Mutex::new(w.export(&w.maintainer, &w.keys, "v1.archive")));
    let etag = Arc::new(Mutex::new("\"a1\"".to_owned()));
    let tip = Arc::new(Mutex::new(w.tip(&w.maintainer, &w.keys)));
    let server = {
        // The advertisement follows the shared `tip`.
        let (a2, e2, t2) = (archive.clone(), etag.clone(), tip.clone());
        let have = w.have();
        Server::start(
            "good",
            Arc::new(move |r: &Req| {
                let path = r.path.split('?').next().unwrap_or("");
                if path.ends_with("/transport/advertise") {
                    let adv = serde_json::json!({
                        "fingerprint": "ff", "refs": [{"name": "heads/main", "state": "published", "kind": "branch", "closed": false,
                            "tip_block": t2.lock().unwrap().clone(), "ref_state_id": "rs", "fetch": [{"kind": "have", "ref": "heads/main", "size": have.len(), "etag": "\"h\""}]}],
                        "archive": {"kind": "archive", "ref": null, "size": null, "etag": e2.lock().unwrap().clone()}
                    });
                    full(200, &[], adv.to_string().as_bytes())
                } else if path.ends_with("/transport/fetch") {
                    let et = e2.lock().unwrap().clone();
                    if r.header("if-none-match") == Some(et.as_str()) {
                        full(304, &[("ETag", &et)], b"")
                    } else {
                        full(200, &[("ETag", &et)], &a2.lock().unwrap())
                    }
                } else {
                    full(404, &[], b"{}")
                }
            }),
        )
    };
    let (home, state) = (w.dir.join("reader"), w.dir.join("state"));
    std::fs::create_dir_all(&home).unwrap();
    let dest = w.dir.join("clone");
    let r = client(
        &[
            "clone",
            &url(&server),
            dest.to_str().unwrap(),
            "--trust-key",
            &w.key,
        ],
        &w.dir,
        &home,
        &state,
        "",
        false,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    // Nothing changed: a 304.
    let r = client(&["fetch"], &dest, &home, &state, "", false, &[]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("Nothing changed on the server"), "{}", r.out);
    // The server moves on (the maintainer's history grows): a new state, landed as received copies; the person's branches stay.
    w.commit(2, "next");
    *archive.lock().unwrap() = w.export(&w.maintainer, &w.keys, "v2.archive");
    *etag.lock().unwrap() = "\"a2\"".into();
    *tip.lock().unwrap() = w.tip(&w.maintainer, &w.keys);
    let before = prikk(&dest, &home, &["log", "--format", "json", "--limit", "1"]);
    let r = client(&["fetch"], &dest, &home, &state, "", false, &[]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out.contains("Fetched the server's state")
            && r.out
                .contains("Next: review what came, then merge it yourself"),
        "{}",
        r.out
    );
    assert_eq!(
        prikk(&dest, &home, &["log", "--format", "json", "--limit", "1"]),
        before,
        "the person's own branch did not move"
    );
    // A server that now shows a history which does not continue from the one it showed: two histories under one name.
    let other = world("fetch-other");
    other.commit(1, "elsewhere");
    *archive.lock().unwrap() = other.export(&other.maintainer, &other.keys, "other.archive");
    *etag.lock().unwrap() = "\"a3\"".into();
    *tip.lock().unwrap() = other.tip(&other.maintainer, &other.keys);
    let remotes_before = prikk(
        &dest,
        &home,
        &["branch", "list", "--all", "--format", "json"],
    );
    let r = client(&["fetch"], &dest, &home, &state, "", false, &[]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert!(
        r.err
            .contains("does not continue from the one it showed you before")
            && r.err.contains("nothing was landed"),
        "{}",
        r.err
    );
    assert_eq!(
        prikk(
            &dest,
            &home,
            &["branch", "list", "--all", "--format", "json"]
        ),
        remotes_before,
        "nothing of a split view reached the repository"
    );
}

fn json_error(
    status: u16,
    code: &str,
    message: &str,
    hint: &str,
    extra: serde_json::Value,
) -> Reply {
    let mut body = serde_json::json!({"code": code, "message": message, "hint": hint});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        body[&k] = v;
    }
    full(
        status,
        &[("Content-Type", "application/json")],
        body.to_string().as_bytes(),
    )
}

#[test]
fn every_refusal_of_the_propose_route_is_one_reason_and_one_remedy_and_the_servers_words_only_under_verbose()
 {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let replies: Vec<(&str, u16, &str, serde_json::Value, &[&str])> = vec![
        (
            "incomplete",
            503,
            "incomplete",
            serde_json::json!({"intent": "i-1", "cause": "accept_failed"}),
            &["did not finish taking your work in", "--retry"],
        ),
        (
            "quota",
            422,
            "quota_exceeded",
            serde_json::json!({"quota": "open_proposals", "limit": 5, "used": 5, "unit": "count"}),
            &[
                "reached a limit",
                "open_proposals",
                "close finished proposals",
            ],
        ),
        (
            "recovery",
            503,
            "needs_recovery",
            serde_json::json!({}),
            &["repairing this repository"],
        ),
        (
            "declared",
            422,
            "declaration_misses_added",
            serde_json::json!({}),
            &["does not match what the artifact holds"],
        ),
        (
            "base",
            422,
            "base_unknown",
            serde_json::json!({}),
            &["does not know the point your work was built on"],
        ),
        (
            "big",
            413,
            "payload_too_large",
            serde_json::json!({}),
            &["will not take this much at once"],
        ),
        (
            "memory",
            422,
            "too_large",
            serde_json::json!({}),
            &["cannot take work of this size right now"],
        ),
        (
            "closed",
            422,
            "proposal_not_open",
            serde_json::json!({}),
            &["no longer open"],
        ),
        (
            "rate",
            429,
            "rate_limited",
            serde_json::json!({}),
            &["asked too often", "Wait 17 seconds"],
        ),
        (
            "notoken",
            401,
            "unauthorized",
            serde_json::json!({}),
            &["did not accept the token"],
        ),
        (
            "forbidden",
            403,
            "forbidden",
            serde_json::json!({}),
            &["You may not do this on this repository"],
        ),
        (
            "missing",
            404,
            "not_found",
            serde_json::json!({}),
            &["no such repository, or you may not read it"],
        ),
    ];
    for (name, status, code, extra, want) in replies {
        let body = json_error(
            status,
            code,
            "SERVER-WORDS-MESSAGE.",
            "SERVER-WORDS-HINT.",
            extra,
        );
        let Reply::Full {
            status: st,
            headers,
            body,
        } = body
        else {
            unreachable!()
        };
        let mut headers = headers;
        if status == 429 {
            headers.push(("Retry-After".into(), "17".into()));
        }
        let (w, server, _) = setup_propose(
            &format!("refusal-{name}"),
            Box::new(move |_| Reply::Full {
                status: st,
                headers: headers.clone(),
                body: body.clone(),
            }),
        );
        w.commit(1, "work");
        let (state, tok) = (w.dir.join("state"), token_file(&w.dir));
        let r = client(
            &[
                "propose",
                &url(&server),
                "--title",
                "T",
                "--token-file",
                tok.to_str().unwrap(),
            ],
            &w.maintainer,
            &w.keys,
            &state,
            "",
            false,
            &[],
        );
        assert_eq!(r.code, 1, "{name}: {}{}", r.out, r.err);
        for phrase in want {
            assert!(
                r.err.contains(phrase),
                "{name}: {phrase:?} not in {}",
                r.err
            );
        }
        assert!(
            !r.err.contains("SERVER-WORDS"),
            "{name}: the server's own text is only for --verbose: {}",
            r.err
        );
        assert!(!r.err.contains("tok-secret") && !r.out.contains("tok-secret"));
        let v = client(
            &[
                "propose",
                &url(&server),
                "--title",
                "T",
                "--token-file",
                tok.to_str().unwrap(),
                "--verbose",
            ],
            &w.maintainer,
            &w.keys,
            &state,
            "",
            false,
            &[],
        );
        if !["rate", "notoken", "forbidden", "missing"].contains(&name) || name == "rate" {
            assert!(
                v.err.contains("(detail)") || name == "incomplete",
                "{name}: {}",
                v.err
            );
        }
    }
}

#[test]
fn an_unfinished_send_is_only_repeated_when_asked_and_a_success_is_said_in_plain_words() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let calls = Arc::new(Mutex::new(0u32));
    let c2 = calls.clone();
    let (w, server, _) = setup_propose(
        "retry",
        Box::new(move |_| {
            let mut n = c2.lock().unwrap();
            *n += 1;
            if *n == 1 {
                json_error(
                    503,
                    "incomplete",
                    "The server did not finish.",
                    "Send it again.",
                    serde_json::json!({"intent": "i-1"}),
                )
            } else {
                full(
                    201,
                    &[("Content-Type", "application/json")],
                    br#"{"proposal_id":"c-42","revision":{"id":"r-1","seq":1},"created_proposal":true,"already_recorded":false,"added_count":1,"message":"held."}"#,
                )
            }
        }),
    );
    w.commit(1, "work");
    let (state, tok) = (w.dir.join("state"), token_file(&w.dir));
    let args = |extra: &[&'static str]| -> Vec<String> {
        let mut a = vec![
            "propose".to_owned(),
            url(&server),
            "--title".into(),
            "T".into(),
            "--token-file".into(),
            tok.display().to_string(),
        ];
        a.extend(extra.iter().map(|s| (*s).to_owned()));
        a
    };
    let go = |extra: &[&'static str]| {
        let a = args(extra);
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        client(&a, &w.maintainer, &w.keys, &state, "", false, &[])
    };
    let first = go(&[]);
    assert_eq!(first.code, 1, "{}{}", first.out, first.err);
    assert_eq!(*calls.lock().unwrap(), 1);
    // The same work again, without --retry: refused here, nothing is sent.
    let second = go(&[]);
    assert_eq!(second.code, 1, "{}{}", second.out, second.err);
    assert!(second.err.contains("--retry"), "{}", second.err);
    assert_eq!(*calls.lock().unwrap(), 1, "no silent re-send");
    // With --retry it is sent once more, and the success is plain: the next step is named, nothing says sealed or merged.
    let third = go(&["--retry"]);
    assert_eq!(third.code, 0, "{}{}", third.out, third.err);
    assert_eq!(*calls.lock().unwrap(), 2);
    assert!(third.out.contains("Proposed as proposal #c-42 (revision 1). Your work is held for review on this server; no branch has changed. Reviewers can see it at https://127.0.0.1:"), "{}", third.out);
    assert!(
        third.out.contains("/alice/app/proposals/c-42"),
        "{}",
        third.out
    );
    // The record is cleared by the success: the same work is simply sent again next time (the server decides it is a repeat).
    let fourth = go(&[]);
    assert_eq!(fourth.code, 0, "{}{}", fourth.out, fourth.err);
    assert_eq!(*calls.lock().unwrap(), 3);
}

#[test]
fn a_redirect_from_the_propose_route_is_reported_and_not_followed_and_the_sent_body_is_exactly_the_framing()
 {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let seen: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let (w, server, _) = setup_propose(
        "redirect",
        Box::new(move |r| {
            s2.lock().unwrap().push(r.clone());
            full(
                307,
                &[("Location", "https://elsewhere.example/propose")],
                b"",
            )
        }),
    );
    w.commit(2, "work");
    let (state, tok) = (w.dir.join("state"), token_file(&w.dir));
    let r = client(
        &[
            "propose",
            &url(&server),
            "--title",
            "T",
            "--description-file",
            w.maintainer.join("work-0.txt").to_str().unwrap(),
            "--token-file",
            tok.to_str().unwrap(),
        ],
        &w.maintainer,
        &w.keys,
        &state,
        "",
        false,
        &[],
    );
    assert_eq!(r.code, 3, "{}{}", r.out, r.err);
    assert!(r.err.contains("never follows"), "{}", r.err);
    let sent = seen.lock().unwrap().clone();
    assert_eq!(
        sent.len(),
        1,
        "one request, no second one to the redirect target"
    );
    let body = &sent[0].body;
    assert!(body.starts_with(b"PLANETER-PROPOSE-1\n"));
    let m = u64::from_be_bytes(body[19..27].try_into().unwrap()) as usize;
    let meta: serde_json::Value = serde_json::from_slice(&body[27..27 + m]).unwrap();
    assert_eq!(meta["title"], "T");
    assert_eq!(meta["target_ref"], "heads/main");
    assert_eq!(meta["declared_patches"].as_array().unwrap().len(), 2);
    assert_eq!(
        meta["base"].as_str().unwrap(),
        w.tip(&w.forge, &w.forge_keys)
    );
    assert!(meta["description"].as_str().unwrap().starts_with("work 0"));
    assert!(
        meta["client"]
            .as_str()
            .unwrap()
            .starts_with("planeter-client/")
    );
    assert!(meta.get("proposal").is_none());
    assert_eq!(
        sent[0]
            .header("content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        body.len()
    );
    assert_eq!(sent[0].header("authorization"), Some("Bearer tok-secret"));
}

#[test]
fn a_token_file_other_users_can_read_is_not_used_and_without_a_token_a_send_is_not_attempted() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let (w, server, _) = setup_propose("tokenfile", Box::new(|_| full(500, &[], b"{}")));
    w.commit(1, "work");
    let state = w.dir.join("state");
    let open = w.dir.join("open-token");
    std::fs::write(&open, "tok-secret\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
    let r = client(
        &[
            "propose",
            &url(&server),
            "--title",
            "T",
            "--token-file",
            open.to_str().unwrap(),
        ],
        &w.maintainer,
        &w.keys,
        &state,
        "",
        false,
        &[],
    );
    assert_eq!(r.code, 5, "{}{}", r.out, r.err);
    assert!(
        r.err.contains("can be read by other users") && r.err.contains("chmod 600"),
        "{}",
        r.err
    );
    assert!(!r.err.contains("tok-secret"));
    let r = client(
        &["propose", &url(&server), "--title", "T"],
        &w.maintainer,
        &w.keys,
        &state,
        "",
        false,
        &[],
    );
    assert_eq!(r.code, 2, "{}{}", r.out, r.err);
    assert!(r.err.contains("Proposing needs a token"), "{}", r.err);
    // The token from the environment is used.
    let r = client(
        &["propose", &url(&server), "--title", "T"],
        &w.maintainer,
        &w.keys,
        &state,
        "",
        false,
        &[("PLANETER_TOKEN", "tok-secret")],
    );
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    // A dry run needs no token and sends nothing.
    let r = client(
        &["propose", &url(&server), "--dry-run"],
        &w.maintainer,
        &w.keys,
        &state,
        "",
        false,
        &[],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("Dry run: nothing was sent"), "{}", r.out);
}
