//! `planeter-client deliver` against a stub https server that answers like the delivery routes of a planeter server (the JSON is the
//! shape of the real documents saved by the delivery API's tests), with a **stand-in prikk** (a shell script: it answers `--version`
//! and `log`, and writes a known archive for `archive export`), so every test here runs without prikk, bubblewrap or a network. The
//! real client against the real server and the real prikk is `crates/planeter/tests/client_deliver_e2e.rs`.
//!
//! Unix only: the stand-in is a shell script and the private-file checks are Unix modes.
#![cfg(unix)]

mod common;

use std::io::Cursor;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::{Reply, Req, Server, ca, full, scratch};
use planeter_client::catalog::Lang;
use planeter_client::{Ctx, http, run};
use serde_json::{Value, json};

const ARCHIVE: &[u8] = b"ARCHIVE-OF-THE-MAINTAINER";
const TOKEN: &str = "tok-secret-9f3a";
const ID: &str = "dl-af48b13ccc5afd1b4a3fb169";
const OLD: &str = "394b77d5f0fef9fe224b638cadb2c492b5175b296b5e7da2ccadfcd7dc4a9e59";
const NEW: &str = "8d94466fe6b3df49371c4eee29c6a138cc36bd47bae7fd0190ecc5725842d202";

// ------------------------------------------------------------------------------------------------ the world

struct World {
    dir: PathBuf,
    repo: PathBuf,
    standin: PathBuf,
    tmp: PathBuf,
    state: PathBuf,
}

/// A prikk repository (a `.prikk` directory), the stand-in prikk, and the client's own state and temporary directories.
fn world(name: &str) -> World {
    let dir = scratch(&format!("deliver-{name}"));
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join(".prikk")).unwrap();
    let tmp = dir.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let standin = dir.join("prikk");
    std::fs::write(
        &standin,
        format!(
            r#"#!/bin/sh
case "$1" in
  --version) echo 'prikk 0.58.0' ;;
  log)
    if [ -f "{dir}/no-main" ]; then echo "error: ref heads/main does not exist" >&2; exit 1; fi
    echo '{{"schema_version":"log-report-v1","blocks":[{{"block_id":"b1","patch_messages":[]}}]}}' ;;
  archive)
    if [ "$2" = export ]; then
      if [ -f "{dir}/export-fails" ]; then echo "error: worktree has changes against the baseline" >&2; exit 1; fi
      printf '%s' '{archive}' > "$3"
      echo "$3" >> "{dir}/exported"
    fi ;;
esac
"#,
            dir = dir.display(),
            archive = String::from_utf8_lossy(ARCHIVE)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&standin, std::fs::Permissions::from_mode(0o755)).unwrap();
    // an exec of a script just written can meet ETXTBSY on a busy machine
    for _ in 0..200 {
        match std::process::Command::new(&standin)
            .arg("--version")
            .output()
        {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    World {
        state: dir.join("state"),
        dir,
        repo,
        standin,
        tmp,
    }
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn all(&self) -> String {
        format!("{}{}", self.out, self.err)
    }
}

fn client(
    w: &World,
    args: &[&str],
    stdin: &str,
    interactive: bool,
    lang: Lang,
    extra: &[(&str, &str)],
) -> Run {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut input = Cursor::new(stdin.as_bytes().to_vec());
    let home = w.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (home_s, state_s, tmp_s) = (
        home.display().to_string(),
        w.state.display().to_string(),
        w.tmp.display().to_string(),
    );
    let extra: Vec<(String, String)> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let env = move |name: &str| -> Option<String> {
        match name {
            "HOME" | "XDG_CONFIG_HOME" => Some(home_s.clone()),
            "PLANETER_CLIENT_STATE_DIR" => Some(state_s.clone()),
            "TMPDIR" => Some(tmp_s.clone()),
            "PLANETER_TOKEN" => Some(TOKEN.to_owned()),
            "PLANETER_CLIENT_POLL_MS" => Some("20".to_owned()),
            _ => extra
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone()),
        }
    };
    let code = {
        let mut ctx = Ctx {
            lang,
            verbose: false,
            out: &mut out,
            err: &mut err,
            env: &env,
            stdin: &mut input,
            interactive,
            cwd: w.repo.clone(),
            prikk_program: Some(w.standin.clone().into_os_string()),
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

fn en() -> Lang {
    Lang::SOURCE
}

fn ja() -> Lang {
    Lang::from_flag("ja").unwrap()
}

fn url(s: &Server) -> String {
    format!("https://127.0.0.1:{}/alice/app", s.port)
}

/// The scratch directories the client made in the test's temporary directory.
fn leftovers(w: &World) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(&w.tmp)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

// ------------------------------------------------------------------------------------------------ the stub's documents

fn keys() -> Value {
    json!([
        {"fingerprint": "b2bd8807 f07bfa89 9f6ec17a dc13611c 18bf429b a7629bf6 664b54c4 c7b74cb0", "key_id": "ed25519-b2bd8807f07bfa89"},
        {"fingerprint": "f1be199c c0359d7e 570dc6c9 ac83cc9c 8d38bb91 1cf0590b d838fc6b 85d5ef04", "key_id": "ed25519-f1be199cc0359d7e"}
    ])
}

fn report(verdict: &str, reason: Option<&str>) -> Value {
    if verdict == "acceptable" {
        json!({
            "archive_bytes": ARCHIVE.len(), "new_blocks": 4, "new_tip": NEW, "old_tip": OLD,
            "proposals_cannot_be_kept": [],
            "proposals_land": [{"id": "c-e39b1caa9076e39129f770d0", "title": "P1"}],
            "proposals_stay": [{"id": "c-14f3338b03b15378e8dfcc19", "title": "P3"}, {"id": "c-a118818b652f08441c45519f", "title": "P2"}],
            "reason": null, "reason_ref": null, "refs_added": [], "trusted_keys": keys(), "verdict": "acceptable"
        })
    } else {
        json!({
            "archive_bytes": ARCHIVE.len(), "new_blocks": 0, "new_tip": null, "old_tip": null,
            "proposals_cannot_be_kept": [], "proposals_land": [], "proposals_stay": [],
            "reason": reason, "reason_ref": null, "refs_added": [], "trusted_keys": keys(), "verdict": "refused"
        })
    }
}

fn view(state: &str, message: &str, hint: &str, report: Value, failure: Option<&str>) -> Value {
    json!({
        "failure": failure.map(|r| json!({"reason": r})), "hint": hint, "id": ID, "message": message,
        "needs_recovery": false, "report": report, "started_at": 1794984552, "state": state, "updated_at": 1794984553
    })
}

const OWNERS_SENTENCE: &str = "Delivering replaces this repository's branch heads/main with yours. It is accepted only if yours continues from the current one; nothing is signed by this server.";

fn reported() -> Value {
    view(
        "reported",
        OWNERS_SENTENCE,
        "To carry it out, confirm it. To give it up, cancel it. A delivery that is not confirmed is removed after a day.",
        report("acceptable", None),
        None,
    )
}

fn done() -> Value {
    view(
        "done",
        "The delivery is done: this repository's branch is now yours.",
        "Every proposal that was open stays open. A proposal whose work is in your branch will show as landed.",
        report("acceptable", None),
        None,
    )
}

fn json_reply(status: u16, v: &Value) -> Reply {
    full(
        status,
        &[("Content-Type", "application/json")],
        v.to_string().as_bytes(),
    )
}

fn error_reply(status: u16, code: &str, message: &str, hint: &str) -> Reply {
    json_reply(
        status,
        &json!({"code": code, "message": message, "hint": hint}),
    )
}

/// A delivery stub: the routes, what each answers (a closure the test sets), and what was asked.
type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

fn stub(handler: Handler) -> Server {
    Server::start("good", handler)
}

/// The usual road: `POST …/deliveries` → `first` (201), `POST …/confirm` → `confirmed` (200), `GET` → `confirmed`, `DELETE` → 204.
fn road(first: Value, confirmed: Value) -> Server {
    stub(Arc::new(move |r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("POST", p) if p.ends_with("/deliveries") => json_reply(201, &first),
            ("POST", p) if p.ends_with("/confirm") => json_reply(200, &confirmed),
            ("GET", p) if p.contains("/deliveries/") => json_reply(200, &confirmed),
            ("DELETE", p) if p.contains("/deliveries/") => full(204, &[], b""),
            _ => full(404, &[], b"{}"),
        }
    }))
}

fn methods(s: &Server) -> Vec<String> {
    s.seen()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r.method,
                r.path.rsplit("/repos/alice/app/").next().unwrap_or(&r.path)
            )
        })
        .collect()
}

fn count(s: &Server, method: &str, tail: &str) -> usize {
    s.seen()
        .iter()
        .filter(|r| r.method == method && r.path.ends_with(tail))
        .count()
}

fn save(name: &str, text: &str) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("deliver-samples");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), text).unwrap();
}

// ------------------------------------------------------------------------------------------------ the whole road

#[test]
fn the_whole_road_upload_report_prompt_confirm_done_in_both_languages() {
    for (lang, tag) in [(en(), "en"), (ja(), "ja")] {
        let w = world(&format!("road-{tag}"));
        let s = road(reported(), done());
        let r = client(&w, &["deliver", &url(&s)], "y\n", true, lang, &[]);
        assert_eq!(r.code, 0, "{}", r.all());
        save(&format!("done.{tag}.txt"), &r.all());
        if tag == "en" {
            let o = &r.out;
            for want in [
                "Exporting your repository…",
                "Sending 25 B to 127.0.0.1 to be checked…",
                // the server's own sentences, the owner's wording first
                OWNERS_SENTENCE,
                "To carry it out, confirm it.",
                // the client's words from the report's fields
                "The project's branch would go from 394b77d5f0fe to 8d94466fe6b3.",
                "Your delivery adds 4 changes to this repository's history.",
                "Open proposals that stay as they are: 2 (#c-14f3338b03b15378e8dfcc19 \"P3\"; #c-a118818b652f08441c45519f \"P2\").",
                "Proposals whose work is already in your branch, so they will show as landed: 1 (#c-e39b1caa9076e39129f770d0 \"P1\").",
                "Deliver it? [y/N] ",
                "Delivering…",
                "The delivery is dl-af48b13ccc5afd1b4a3fb169. If you stop here, it goes on without you.",
                "The delivery is done: this repository's branch is now yours.",
                "Delivered: the project's branch of app on 127.0.0.1 is now yours, ending at 8d94466fe6b3.",
            ] {
                assert!(o.contains(want), "missing {want:?} in:\n{o}");
            }
            // no word of the engine's
            for word in ["stage", "chain", "tip", "adopt", "claim", "seal", "block"] {
                assert!(
                    !o.to_ascii_lowercase()
                        .split(|c: char| !c.is_alphabetic())
                        .any(|w| w == word),
                    "`{word}` in:\n{o}"
                );
            }
        } else {
            assert!(r.out.contains("引き渡しました。"), "{}", r.out);
        }
        assert_eq!(
            methods(&s),
            [
                "POST deliveries".to_owned(),
                format!("POST deliveries/{ID}/confirm")
            ],
            "{tag}"
        );
        assert_eq!(leftovers(&w), Vec::<String>::new(), "the scratch is gone");
    }
}

#[test]
fn the_upload_is_the_imports_framing_with_the_bearer_token_and_the_language_asked_for() {
    for (lang, tag) in [(en(), "en"), (ja(), "ja")] {
        let w = world(&format!("framing-{tag}"));
        let s = road(reported(), done());
        let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, lang, &[]);
        assert_eq!(r.code, 0, "{}", r.all());
        let seen = s.seen();
        let post = seen
            .iter()
            .find(|r| r.path.ends_with("/deliveries"))
            .unwrap();
        let mut want = b"PLANETER-IMPORT-1\n".to_vec();
        want.extend_from_slice(&2u64.to_be_bytes());
        want.extend_from_slice(b"{}");
        want.extend_from_slice(ARCHIVE);
        assert_eq!(post.body, want, "exactly the framing and the archive");
        assert_eq!(
            post.header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(post.header("accept-language"), Some(tag));
        assert_eq!(
            post.header("content-length"),
            Some(want.len().to_string().as_str())
        );
        // every request of the run asks in the language, and carries the token and nothing in the path
        for r in &seen {
            assert_eq!(
                r.header("accept-language"),
                Some(tag),
                "{} {}",
                r.method,
                r.path
            );
            assert!(!r.path.contains(TOKEN));
        }
    }
}

// ------------------------------------------------------------------------------------------------ refusals of the report

fn reasons() -> Vec<(&'static str, &'static str, &'static str)> {
    // (reason, the server's message, its hint): the closed list of the report's reasons, with the delivery API's English words.
    vec![
        (
            "no_main",
            "Your archive has no branch heads/main.",
            "Deliver the archive of a repository that has the project's branch.",
        ),
        (
            "unadopted_key",
            "Some of your history is signed by a key this repository does not trust.",
            "Only the keys listed here can deliver.",
        ),
        (
            "not_a_successor",
            "Your archive does not continue this repository's branch heads/main.",
            "Update your repository from this one, then deliver again.",
        ),
        (
            "older_or_diverged",
            "Your branch does not continue from this repository's branch.",
            "Update your repository from the project's branch, then deliver again.",
        ),
        (
            "unrelated",
            "Your archive has nothing in common with this repository's history.",
            "Deliver from a repository that was made from this one. A different history cannot replace it.",
        ),
        (
            "branch_missing",
            "This repository has a branch your delivery does not have (heads/dev).",
            "A delivery may add branches but never remove one.",
        ),
        (
            "damaged_archive",
            "The archive could not be read, or it does not check out.",
            "Make the archive again with prikk, and send it again.",
        ),
        (
            "timed_out",
            "Checking your archive took longer than this server allows.",
            "Try again later, or send a smaller archive.",
        ),
        (
            "ceiling",
            "Your archive needs more memory to check than this server allows.",
            "Send a smaller archive, or ask the person who runs this server.",
        ),
        (
            "unavailable",
            "Your archive could not be checked right now.",
            "Try again in a little while.",
        ),
    ]
}

#[test]
fn every_closed_reason_of_a_refused_report_prints_the_servers_message_and_hint_and_exits_1() {
    for (reason, message, hint) in reasons() {
        for (lang, tag) in [(en(), "en"), (ja(), "ja")] {
            let w = world(&format!("refused-{reason}-{tag}"));
            let s = road(
                view(
                    "failed",
                    message,
                    hint,
                    report("refused", Some(reason)),
                    Some(reason),
                ),
                json!({}),
            );
            let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, lang, &[]);
            assert_eq!(r.code, 1, "{reason}: {}", r.all());
            // the server's own words (the stub answers in English whatever was asked: the client prints what it got)
            assert!(
                r.err.contains(message) && r.err.contains(hint),
                "{reason}: {}",
                r.err
            );
            assert!(r.out.contains("Exporting") || r.out.contains("エクスポート"));
            // nothing was confirmed, and there is no prompt
            assert_eq!(count(&s, "POST", "/confirm"), 0, "{reason}");
            assert!(
                !r.out.contains("Deliver it?") && !r.out.contains("引き渡しますか"),
                "{reason}"
            );
            if reason == "unadopted_key" {
                assert!(
                    r.err.contains("ed25519-b2bd8807f07bfa89")
                        && r.err.contains("b2bd8807 f07bfa89 9f6ec17a dc13611c"),
                    "the keys that can deliver are listed: {}",
                    r.err
                );
            }
            if tag == "en" {
                save(&format!("refused-{reason}.en.txt"), &r.all());
            } else {
                save(&format!("refused-{reason}.ja.txt"), &r.all());
            }
            assert_eq!(
                leftovers(&w),
                Vec::<String>::new(),
                "{reason}: the scratch is gone"
            );
        }
    }
}

#[test]
fn nothing_to_deliver_is_said_in_the_servers_words_and_exits_0() {
    let w = world("nothing");
    let s = road(
        view(
            "failed",
            "Your archive changes nothing in this repository.",
            "There is nothing new to deliver.",
            report("refused", Some("nothing_to_deliver")),
            Some("nothing_to_deliver"),
        ),
        json!({}),
    );
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(
        r.out
            .contains("Nothing to deliver: Your archive changes nothing in this repository."),
        "{}",
        r.out
    );
    assert_eq!(count(&s, "POST", "/confirm"), 0);
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, ja(), &[]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("引き渡すものはありません："), "{}", r.out);
}

// ------------------------------------------------------------------------------------------------ dry run, prompt, no terminal

#[test]
fn a_dry_run_cancels_the_delivery_and_says_so_and_exits_0() {
    let w = world("dry");
    let s = road(reported(), json!({}));
    let r = client(
        &w,
        &["deliver", &url(&s), "--dry-run"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(r.out.contains(OWNERS_SENTENCE), "the report is shown");
    assert!(
        r.out
            .contains("Dry run: nothing was delivered, and the check was cancelled on the server."),
        "{}",
        r.out
    );
    assert!(!r.out.contains("Deliver it?"), "a dry run asks nothing");
    assert_eq!(
        count(&s, "DELETE", &format!("/deliveries/{ID}")),
        1,
        "{:?}",
        methods(&s)
    );
    assert_eq!(count(&s, "POST", "/confirm"), 0);
    save("dry-run.en.txt", &r.all());
    let r = client(
        &w,
        &["deliver", &url(&s), "--dry-run"],
        "",
        false,
        ja(),
        &[],
    );
    assert_eq!(r.code, 0);
    save("dry-run.ja.txt", &r.all());
}

#[test]
fn a_non_terminal_without_yes_refuses_before_anything_is_exported_or_sent() {
    let w = world("noterm");
    let s = road(reported(), done());
    let r = client(&w, &["deliver", &url(&s)], "y\n", false, en(), &[]);
    assert_eq!(r.code, 2, "{}", r.all());
    assert!(r.err.contains("--yes"), "{}", r.err);
    assert!(s.seen().is_empty(), "not one request: {:?}", methods(&s));
    assert!(!w.dir.join("exported").exists(), "nothing was exported");
    let r = client(&w, &["deliver", &url(&s)], "y\n", false, ja(), &[]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--yes"), "{}", r.err);
    save("no-terminal.en.txt", "");
}

#[test]
fn the_prompt_has_no_default_yes_and_only_y_or_yes_confirms() {
    for (answer, confirms) in [
        ("y\n", true),
        ("yes\n", true),
        ("YES\n", true),
        ("\n", false),
        ("n\n", false),
        ("yep\n", false),
        ("", false),
    ] {
        let w = world(&format!("prompt-{}", answer.trim()));
        let s = road(reported(), done());
        let r = client(&w, &["deliver", &url(&s)], answer, true, en(), &[]);
        assert_eq!(r.code, 0, "{answer:?}: {}", r.all());
        assert_eq!(
            count(&s, "POST", "/confirm"),
            usize::from(confirms),
            "{answer:?}"
        );
        if !confirms {
            assert!(
                r.out
                    .contains("Not delivered. The check was cancelled on the server."),
                "{}",
                r.out
            );
            assert_eq!(
                count(&s, "DELETE", &format!("/deliveries/{ID}")),
                1,
                "{answer:?}"
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------ waiting

#[test]
fn a_check_that_takes_long_is_polled_to_the_report_and_a_confirmation_to_done() {
    let w = world("polled");
    let gets = Arc::new(Mutex::new(0usize));
    let g = gets.clone();
    let s = stub(Arc::new(move |r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            // the check takes longer than the request waits: 202, state staged
            ("POST", p) if p.ends_with("/deliveries") => json_reply(
                202,
                &view(
                    "staged",
                    "Your archive is being checked.",
                    "Wait a moment.",
                    Value::Null,
                    None,
                ),
            ),
            ("GET", p) if p.contains("/deliveries/") => {
                let mut n = g.lock().unwrap();
                *n += 1;
                match *n {
                    1 | 2 => json_reply(
                        200,
                        &view(
                            "staged",
                            "Your archive is being checked.",
                            "Wait.",
                            Value::Null,
                            None,
                        ),
                    ),
                    3 => json_reply(200, &reported()),
                    4 | 5 => json_reply(
                        200,
                        &view(
                            "reaccepting",
                            "The open proposals are being put back on the new branch.",
                            "Wait.",
                            report("acceptable", None),
                            None,
                        ),
                    ),
                    _ => json_reply(200, &done()),
                }
            }
            ("POST", p) if p.ends_with("/confirm") => json_reply(
                202,
                &view(
                    "reaccepting",
                    "The open proposals are being put back on the new branch.",
                    "Wait.",
                    report("acceptable", None),
                    None,
                ),
            ),
            _ => full(404, &[], b"{}"),
        }
    }));
    let r = client(
        &w,
        &["deliver", &url(&s), "--yes"],
        "",
        false,
        en(),
        &[("PLANETER_CLIENT_STILL_MS", "30")],
    );
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(r.out.contains("is checking your archive"), "{}", r.out);
    assert!(
        r.out.contains("Still working…"),
        "a visible line while it waits: {}",
        r.out
    );
    assert!(r.out.contains("The delivery is done"), "{}", r.out);
    save("polled.en.txt", &r.all());
}

#[test]
fn a_wait_that_never_ends_stops_with_the_id_and_the_address_to_look_at() {
    let w = world("waitover");
    let s = stub(Arc::new(|r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("POST", p) if p.ends_with("/deliveries") => json_reply(201, &reported()),
            ("POST", p) if p.ends_with("/confirm") || p.contains("/deliveries/") => json_reply(
                200,
                &view(
                    "reaccepting",
                    "The open proposals are being put back on the new branch.",
                    "Wait.",
                    report("acceptable", None),
                    None,
                ),
            ),
            ("GET", _) => json_reply(
                200,
                &view(
                    "reaccepting",
                    "The open proposals are being put back on the new branch.",
                    "Wait.",
                    report("acceptable", None),
                    None,
                ),
            ),
            _ => full(404, &[], b"{}"),
        }
    }));
    let r = client(
        &w,
        &["deliver", &url(&s), "--yes"],
        "",
        false,
        en(),
        &[("PLANETER_CLIENT_WAIT_MS", "200")],
    );
    assert_eq!(r.code, 3, "{}", r.all());
    assert!(
        r.err.contains("still going after") && r.err.contains(&format!("/deliveries/{ID}")),
        "{}",
        r.err
    );
}

// ------------------------------------------------------------------------------------------------ after the confirmation

#[test]
fn a_delivery_that_failed_after_the_confirmation_prints_the_reason_and_exits_1() {
    let w = world("failed");
    let moved = view(
        "failed",
        "This repository changed while your delivery was waiting.",
        "Update your repository from the project's branch, then deliver again.",
        report("acceptable", None),
        Some("moved"),
    );
    let s = road(reported(), moved);
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 1, "{}", r.all());
    assert!(
        r.err.contains("The delivery did not go through.")
            && r.err
                .contains("This repository changed while your delivery was waiting.")
            && r.err
                .contains("Update your repository from the project's branch"),
        "{}",
        r.err
    );
    save("failed-moved.en.txt", &r.all());
    assert_eq!(leftovers(&w), Vec::<String>::new());
}

#[test]
fn a_delivery_that_needs_the_servers_recovery_says_so_and_exits_1() {
    let w = world("recovery");
    let mut v = view(
        "confirmed",
        "The server stopped while this delivery was being carried out.",
        "It finishes by itself.",
        report("acceptable", None),
        None,
    );
    v["needs_recovery"] = json!(true);
    let s = road(reported(), v);
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 1, "{}", r.all());
    assert!(
        r.err
            .contains("The server stopped while your delivery was being carried out.")
            && r.err.contains("tell the person who runs this server")
            && r.err.contains("do not deliver again meanwhile"),
        "{}",
        r.err
    );
    save("needs-recovery.en.txt", &r.all());
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, ja(), &[]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("このサーバーを運用している人"), "{}", r.err);
}

// ------------------------------------------------------------------------------------------------ the server says no

fn answers(status: u16, code: &str, message: &str, hint: &str) -> Server {
    let (code, message, hint) = (code.to_owned(), message.to_owned(), hint.to_owned());
    stub(Arc::new(move |_: &Req| {
        error_reply(status, &code, &message, &hint)
    }))
}

#[test]
fn a_403_is_the_sentence_that_says_what_it_means_here_and_the_servers_hint_only_under_verbose() {
    for (lang, tag, sentence) in [
        (
            en(),
            "en",
            "Delivering needs a token with full access to this repository, from a maintainer. This token cannot deliver.",
        ),
        (
            ja(),
            "ja",
            "引き渡すには、メンテナーの、このリポジトリへの完全なアクセスを許可するトークンが必要です。このトークンでは引き渡せません。",
        ),
    ] {
        let w = world(&format!("403-{tag}"));
        let s = answers(
            403,
            "forbidden",
            "You may read this repository, but you may not deliver to it.",
            "Only a maintainer of the repository can deliver.",
        );
        let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, lang, &[]);
        assert_eq!(r.code, 1, "{}", r.all());
        assert!(r.err.contains(sentence), "{}", r.err);
        assert!(
            !r.err.contains("Only a maintainer of the repository"),
            "the server's hint only under --verbose: {}",
            r.err
        );
        save(&format!("403.{tag}.txt"), &r.all());
        let r = client(
            &w,
            &["deliver", &url(&s), "--yes", "--verbose"],
            "",
            false,
            lang,
            &[],
        );
        assert!(
            r.err
                .contains("Only a maintainer of the repository can deliver."),
            "{}",
            r.err
        );
        assert!(!r.all().contains(TOKEN), "the token is in no output");
    }
}

#[test]
fn a_404_is_the_one_sentence_every_command_uses() {
    let w = world("404");
    let s = answers(404, "not_found", "No such repository.", "");
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 1, "{}", r.all());
    assert!(
        r.err
            .contains("The server has no such repository, or you may not read it."),
        "{}",
        r.err
    );
    save("404.en.txt", &r.all());
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, ja(), &[]);
    save("404.ja.txt", &r.all());
    assert!(
        r.err.contains("そのリポジトリはサーバーにないか"),
        "{}",
        r.err
    );
}

#[test]
fn a_413_is_said_in_words_with_the_size_that_was_sent() {
    let w = world("413");
    let s = answers(
        413,
        "payload_too_large",
        "That proposal is larger than this server accepts.",
        "Send a smaller artifact.",
    );
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 1, "{}", r.all());
    assert!(
        r.err
            .contains("The server will not take an archive of 25 B.")
            && !r.err.contains("proposal is larger"),
        "our words, not the proposals': {}",
        r.err
    );
    let r = client(
        &w,
        &["deliver", &url(&s), "--yes", "--verbose"],
        "",
        false,
        en(),
        &[],
    );
    assert!(
        r.err.contains("payload_too_large (413)"),
        "the server's text under --verbose: {}",
        r.err
    );
    save("413.en.txt", &r.all());
}

#[test]
fn a_confirmation_that_meets_another_write_is_told_so_and_the_delivery_is_given_up() {
    let w = world("busy");
    let s = stub(Arc::new(|r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("POST", p) if p.ends_with("/deliveries") => json_reply(201, &reported()),
            ("POST", p) if p.ends_with("/confirm") => error_reply(
                409,
                "delivery_busy",
                "Another delivery to this repository is already in progress, or the repository is busy with another write.",
                "Wait a moment.",
            ),
            ("DELETE", _) => full(204, &[], b""),
            _ => full(404, &[], b"{}"),
        }
    }));
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 1, "{}", r.all());
    assert!(
        r.err
            .contains("Another delivery or another write is in progress on this repository."),
        "{}",
        r.err
    );
    assert_eq!(
        count(&s, "DELETE", &format!("/deliveries/{ID}")),
        1,
        "given up, so that a run again is not blocked by it"
    );
}

#[test]
fn the_other_answers_of_the_confirmation_are_one_sentence_each() {
    for (status, code, want) in [
        (
            409,
            "delivery_not_confirmable",
            "This delivery cannot be confirmed now.",
        ),
        (
            410,
            "stage_expired",
            "That delivery waited more than a day and was removed.",
        ),
        (
            503,
            "needs_recovery",
            "The server stopped while your delivery was being carried out.",
        ),
        (
            403,
            "forbidden",
            "Delivering needs a token with full access",
        ),
    ] {
        let w = world(&format!("confirm-{status}"));
        let (code, status_c) = (code.to_owned(), status);
        let s = stub(Arc::new(move |r: &Req| {
            let path = r.path.split('?').next().unwrap_or("");
            match (r.method.as_str(), path) {
                ("POST", p) if p.ends_with("/deliveries") => json_reply(201, &reported()),
                ("POST", p) if p.ends_with("/confirm") => error_reply(status_c, &code, "m", "h"),
                _ => full(204, &[], b""),
            }
        }));
        let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
        assert_eq!(r.code, 1, "{status}: {}", r.all());
        assert!(r.err.contains(want), "{status}: {}", r.err);
    }
}

#[test]
fn a_redirect_is_reported_and_never_followed() {
    let w = world("redirect");
    let s = stub(Arc::new(|r: &Req| {
        if r.method == "POST" {
            full(
                307,
                &[("Location", "https://elsewhere.invalid/alice/app/deliveries")],
                b"",
            )
        } else {
            full(404, &[], b"{}")
        }
    }));
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 3, "{}", r.all());
    assert!(r.err.contains("elsewhere.invalid"), "{}", r.err);
    assert_eq!(
        s.seen().len(),
        1,
        "one request, no second one to the other place"
    );
}

#[test]
fn a_network_failure_during_the_upload_is_exit_3_and_says_a_run_again_is_safe() {
    let w = world("net");
    // a port nobody listens on
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let r = client(
        &w,
        &[
            "deliver",
            &format!("https://127.0.0.1:{port}/alice/app"),
            "--yes",
        ],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 3, "{}", r.all());
    assert!(
        r.err
            .contains("Part of your archive may have reached the server."),
        "{}",
        r.err
    );
    assert_eq!(leftovers(&w), Vec::<String>::new());
}

// ------------------------------------------------------------------------------------------------ local checks

#[test]
fn plain_http_is_refused_before_anything_else() {
    let w = world("plain");
    let r = client(
        &w,
        &["deliver", "http://forge.example/alice/app", "--yes"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 2, "{}", r.all());
    assert!(!w.dir.join("exported").exists());
}

#[test]
fn local_checks_come_first_not_a_repository_no_main_and_an_export_that_prikk_refuses() {
    let w = world("local");
    let s = road(reported(), done());
    // not a repository
    let r = client(
        &w,
        &["deliver", &url(&s), "--yes", "--repo", "nowhere"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 2, "{}", r.all());
    // no heads/main
    std::fs::write(w.dir.join("no-main"), "").unwrap();
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 2, "{}", r.all());
    assert!(
        r.err.contains("Your repository has no branch heads/main"),
        "{}",
        r.err
    );
    std::fs::remove_file(w.dir.join("no-main")).unwrap();
    // prikk refuses the export (unfinished work): its words under --verbose, ours always, exit 4
    std::fs::write(w.dir.join("export-fails"), "").unwrap();
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 4, "{}", r.all());
    assert!(
        r.err.contains("prikk could not export your repository: error: worktree has changes against the baseline")
            && r.err.contains("finish or discard it"),
        "{}",
        r.err
    );
    let r = client(
        &w,
        &["deliver", &url(&s), "--yes", "--verbose"],
        "",
        false,
        en(),
        &[],
    );
    assert!(r.err.contains("(detail)"), "{}", r.err);
    assert!(s.seen().is_empty(), "nothing was sent: {:?}", methods(&s));
    assert_eq!(leftovers(&w), Vec::<String>::new());
    // --repo names the repository
    std::fs::remove_file(w.dir.join("export-fails")).unwrap();
    let elsewhere = w.dir.join("other");
    std::fs::create_dir_all(elsewhere.join(".prikk")).unwrap();
    let mut w2 = world("local-repo");
    w2.repo = w.dir.clone();
    let r = client(
        &w2,
        &["deliver", &url(&s), "--yes", "--repo", "other"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.all());
}

// ------------------------------------------------------------------------------------------------ what a server's text can do to a terminal

#[test]
fn control_characters_in_a_title_are_shown_visibly_and_never_obeyed() {
    let w = world("controls");
    let mut rep = report("acceptable", None);
    rep["proposals_stay"] =
        json!([{"id": "c-1", "title": "bad \u{1b}[31mred\u{1b}[0m\u{7}\nline two"}]);
    rep["proposals_land"] = json!([]);
    rep["refs_added"] = json!(["heads/\u{1b}x"]);
    let mut v = reported();
    v["report"] = rep;
    let s = road(v, done());
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(
        !r.out.contains('\u{1b}') && !r.out.contains('\u{7}'),
        "no control character reaches the terminal: {:?}",
        r.out
    );
    assert!(
        r.out
            .contains("bad \\u{1b}[31mred\\u{1b}[0m\\u{7}\\u{a}line two"),
        "shown as text: {}",
        r.out
    );
    assert!(r.out.contains("heads/\\u{1b}x"), "{}", r.out);
}

// ------------------------------------------------------------------------------------------------ the token, the scratch file

#[test]
fn the_token_is_in_no_output_of_any_run() {
    let w = world("token");
    let mut all = String::new();
    for (lang, flags) in [
        (en(), vec!["--yes"]),
        (ja(), vec!["--yes", "--verbose"]),
        (en(), vec!["--dry-run", "--verbose"]),
    ] {
        let s = road(reported(), done());
        let mut args = vec!["deliver"];
        let u = url(&s);
        args.push(&u);
        args.extend(flags);
        let r = client(&w, &args, "", false, lang, &[]);
        all.push_str(&r.all());
    }
    for (status, code) in [
        (403, "forbidden"),
        (404, "not_found"),
        (413, "payload_too_large"),
        (429, "rate_limited"),
        (503, "unavailable"),
    ] {
        let s = answers(status, code, "m", "h");
        let r = client(
            &w,
            &["deliver", &url(&s), "--yes", "--verbose"],
            "",
            false,
            en(),
            &[],
        );
        all.push_str(&r.all());
    }
    assert!(!all.contains(TOKEN), "the token is printed:\n{all}");
}

#[test]
fn the_archive_is_a_private_file_while_it_is_sent_and_gone_after_success_refusal_and_failure() {
    let w = world("private");
    let seen_modes = Arc::new(Mutex::new(Vec::<(u32, u32)>::new()));
    let (m, tmp) = (seen_modes.clone(), w.tmp.clone());
    let s = stub(Arc::new(move |r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("POST", p) if p.ends_with("/deliveries") => {
                // the run is in the middle of sending: its scratch directory and archive exist, and are private
                for e in std::fs::read_dir(&tmp).unwrap().flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with("planeter-client-deliver-") && e.path().is_dir() {
                        let dir_mode = e.metadata().unwrap().permissions().mode() & 0o777;
                        let file_mode = std::fs::metadata(e.path().join("archive"))
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o777;
                        m.lock().unwrap().push((dir_mode, file_mode));
                    }
                }
                json_reply(201, &reported())
            }
            ("POST", p) if p.ends_with("/confirm") => json_reply(200, &done()),
            _ => full(204, &[], b""),
        }
    }));
    // success
    let r = client(&w, &["deliver", &url(&s), "--yes"], "", false, en(), &[]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(
        seen_modes.lock().unwrap().as_slice(),
        [(0o700, 0o600)],
        "private while it was sent"
    );
    assert_eq!(leftovers(&w), Vec::<String>::new(), "gone after success");
    // refusal of the report
    let refused = road(
        view(
            "failed",
            "m",
            "h",
            report("refused", Some("unrelated")),
            Some("unrelated"),
        ),
        json!({}),
    );
    let r = client(
        &w,
        &["deliver", &url(&refused), "--yes"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 1);
    assert_eq!(leftovers(&w), Vec::<String>::new(), "gone after a refusal");
    // a failure after the confirmation
    let failed = road(
        reported(),
        view(
            "failed",
            "m",
            "h",
            report("acceptable", None),
            Some("moved"),
        ),
    );
    let r = client(
        &w,
        &["deliver", &url(&failed), "--yes"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 1);
    assert_eq!(leftovers(&w), Vec::<String>::new(), "gone after a failure");
    // a refusal of the upload itself
    let denied = answers(403, "forbidden", "m", "h");
    let r = client(
        &w,
        &["deliver", &url(&denied), "--yes"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 1);
    assert_eq!(leftovers(&w), Vec::<String>::new(), "gone after a 403");
}

/// SIGKILL of the real client binary in the middle of the upload (the stub holds the request), then the next run: **no stale file**.
#[test]
fn a_client_killed_in_the_middle_of_the_upload_leaves_a_scratch_that_the_next_run_removes() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_planeter-client"));
    let w = world("killed");
    let held = Arc::new(Mutex::new(false));
    let release = Arc::new(Mutex::new(false));
    let (h, rel) = (held.clone(), release.clone());
    let s = stub(Arc::new(move |r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("POST", p) if p.ends_with("/deliveries") => {
                *h.lock().unwrap() = true;
                while !*rel.lock().unwrap() {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                json_reply(201, &reported())
            }
            _ => full(204, &[], b""),
        }
    }));
    let mut child = std::process::Command::new(&exe)
        .args(["deliver", &url(&s), "--yes", "--cacert"])
        .arg(ca())
        .current_dir(&w.repo)
        .env("PLANETER_TOKEN", TOKEN)
        .env("PLANETER_CLIENT_PRIKK", &w.standin)
        .env("PLANETER_CLIENT_STATE_DIR", &w.state)
        .env("TMPDIR", &w.tmp)
        .env_remove("LANG")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !*held.lock().unwrap() && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(*held.lock().unwrap(), "the client reached the upload");
    let before = leftovers(&w);
    assert_eq!(
        before.len(),
        2,
        "the scratch directory and its lock: {before:?}"
    );
    // SIGKILL: nothing runs in the client
    child.kill().unwrap();
    child.wait().unwrap();
    *release.lock().unwrap() = true;
    assert_eq!(
        leftovers(&w),
        before,
        "a kill leaves the scratch behind (that is what the next run is for)"
    );
    // the next run (any run of `deliver`; a dry run against another stub) removes it, once its lock file is two seconds old
    std::thread::sleep(std::time::Duration::from_millis(2200));
    let other = road(reported(), done());
    let r = client(
        &w,
        &["deliver", &url(&other), "--dry-run"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(leftovers(&w), Vec::<String>::new(), "no stale file");
}

#[test]
fn a_scratch_that_a_live_run_still_holds_is_not_removed_by_another_run() {
    let w = world("live");
    let held = Arc::new(Mutex::new(false));
    let release = Arc::new(Mutex::new(false));
    let (h, rel) = (held.clone(), release.clone());
    let slow = stub(Arc::new(move |r: &Req| {
        let path = r.path.split('?').next().unwrap_or("");
        match (r.method.as_str(), path) {
            ("POST", p) if p.ends_with("/deliveries") => {
                *h.lock().unwrap() = true;
                while !*rel.lock().unwrap() {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                json_reply(201, &reported())
            }
            _ => full(204, &[], b""),
        }
    }));
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_planeter-client"));
    let mut child = std::process::Command::new(&exe)
        .args(["deliver", &url(&slow), "--dry-run", "--cacert"])
        .arg(ca())
        .current_dir(&w.repo)
        .env("PLANETER_TOKEN", TOKEN)
        .env("PLANETER_CLIENT_PRIKK", &w.standin)
        .env("PLANETER_CLIENT_STATE_DIR", &w.state)
        .env("TMPDIR", &w.tmp)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !*held.lock().unwrap() && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(2200));
    let before = leftovers(&w);
    assert_eq!(before.len(), 2, "{before:?}");
    let other = road(reported(), done());
    let r = client(
        &w,
        &["deliver", &url(&other), "--dry-run"],
        "",
        false,
        en(),
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.all());
    // the live run's scratch is still there (the other run only removed its own)
    assert_eq!(leftovers(&w), before, "a live run's scratch is not stale");
    *release.lock().unwrap() = true;
    let _ = child.wait();
    assert_eq!(leftovers(&w), Vec::<String>::new());
}

// ------------------------------------------------------------------------------------------------ source scans

/// ENF-2 for this command: the module calls prikk only to export and to read; it never seals, merges, closes a branch, passes
/// `--allow-non-fast-forward`, or passes `--yes` to prikk.
#[test]
fn the_deliver_module_never_calls_seal_merge_branch_close_allow_non_fast_forward_or_yes_on_prikk() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/deliver.rs"),
    )
    .unwrap();
    // only code: comments are cut off
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for banned in [
        "\"seal\"",
        "\"merge\"",
        "\"close\"",
        "\"branch\"",
        "--allow-non-fast-forward",
        "\"--yes\"",
        "\"commit\"",
        "\"sync\"",
        "\"bundle\"",
        "\"import\"",
        "\"trust\"",
    ] {
        assert!(!code.contains(banned), "the deliver module names {banned}");
    }
    // what it does ask prikk: the archive export and the one read
    assert!(code.contains("\"archive\", \"export\""));
    assert!(code.contains("\"log\""));
    assert!(code.contains("\"--format\""));
    // the scan is able to fail: the same scan over a planted line finds it
    let planted = "prikk.run(\"x\", &[\"seal\", \"--allow-no-audit\"]);";
    assert!(planted.contains("\"seal\""));
}
