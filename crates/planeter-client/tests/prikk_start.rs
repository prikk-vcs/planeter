//! A prikk that is there but cannot be started is not "not found" (part 11, T1): each way of not starting has its own sentence, and
//! only a program that is not there says "not found". Unix: the stand-ins are modes and scripts. One test function, so the scripts
//! written here are never started while another thread of this process forks (a script open for writing in a forked child cannot be
//! executed: `Text file busy`).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use planeter_client::catalog::Lang;
use planeter_client::prikk::Prikk;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("prikk-start-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file(dir: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    p
}

fn check(lang: Lang, program: &Path, work: &Path) -> planeter_client::exit::Failure {
    Prikk::new(work, lang, Some(program.into()))
        .check()
        .expect_err("this stand-in is not a working prikk")
}

#[test]
fn each_way_a_prikk_cannot_be_started_has_its_own_sentence_and_only_a_missing_one_says_not_found() {
    let d = dir("ways");
    let not_found = "prikk was not found";

    // Not there at all.
    let gone = check(Lang::En, &d.join("no-such-prikk"), &d);
    assert!(gone.message.contains(not_found), "{}", gone.message);
    assert!(gone.message.contains("0.54.0"), "{}", gone.message);

    // There, but not executable.
    let plain = file(&d, "plain", "#!/bin/sh\necho 'prikk 0.55.0'\n", 0o644);
    let denied = check(Lang::En, &plain, &d);
    assert!(
        denied
            .message
            .starts_with("prikk was found but could not be started: Permission denied."),
        "{}",
        denied.message
    );
    assert!(
        denied
            .message
            .ends_with("Check that it runs by itself: prikk --version."),
        "{}",
        denied.message
    );
    assert!(!denied.message.contains(not_found));
    assert!(
        denied
            .detail
            .as_deref()
            .is_some_and(|x| x.contains("os error 13")),
        "the OS error is kept: {:?}",
        denied.detail
    );

    // A directory where the program should be.
    let a_dir = d.join("a-directory");
    std::fs::create_dir_all(&a_dir).unwrap();
    let dirsay = check(Lang::En, &a_dir, &d);
    assert!(
        dirsay
            .message
            .starts_with("prikk was found but could not be started:"),
        "{}",
        dirsay.message
    );
    assert!(!dirsay.message.contains(not_found));

    // A script whose interpreter is gone: the system says "no such file", about the interpreter.
    let broken = file(&d, "broken", "#!/nonexistent/interpreter\n", 0o755);
    let broke = check(Lang::En, &broken, &d);
    assert!(
        broke
            .message
            .starts_with("prikk was found but could not be started:"),
        "{}",
        broke.message
    );
    assert!(!broke.message.contains(not_found), "{}", broke.message);
    assert!(
        broke
            .detail
            .as_deref()
            .is_some_and(|x| x.contains("os error 2")),
        "{:?}",
        broke.detail
    );

    // It starts and does not say a version: what it said is shown.
    let odd = file(&d, "odd", "#!/bin/sh\necho 'hello there'\n", 0o755);
    let odd = check(Lang::En, &odd, &d);
    assert!(
        odd.message
            .starts_with("prikk did not say which version it is (it said: “hello there”)."),
        "{}",
        odd.message
    );
    assert!(!odd.message.contains(not_found));
    // ... or nothing at all, and fails.
    let mute = file(&d, "mute", "#!/bin/sh\nexit 3\n", 0o755);
    let mute = check(Lang::En, &mute, &d);
    assert!(
        mute.message.contains("it said: “exit status: 3”") && !mute.message.contains(not_found),
        "{}",
        mute.message
    );

    // `run` (the commands) tells the same truth as `check`.
    let ran = Prikk::new(&d, Lang::En, Some(plain.clone().into()))
        .run("make the repository", &["init", "."])
        .expect_err("not executable");
    assert!(
        ran.message
            .starts_with("prikk was found but could not be started: Permission denied."),
        "{}",
        ran.message
    );
    let ran_missing = Prikk::new(&d, Lang::En, Some(d.join("no-such-prikk").into()))
        .run("make the repository", &["init", "."])
        .expect_err("missing");
    assert!(
        ran_missing.message.contains(not_found),
        "{}",
        ran_missing.message
    );

    // Japanese says the same three things, and the OS reason is kept in the detail only as the system words it.
    let ja = check(Lang::Ja, &plain, &d);
    assert!(
        ja.message
            .contains("見つかりましたが、起動できませんでした"),
        "{}",
        ja.message
    );
    assert!(ja.message.contains("prikk --version"), "{}", ja.message);
    let ja_gone = check(Lang::Ja, &d.join("no-such-prikk"), &d);
    assert!(
        ja_gone.message.contains("prikk が見つかりません"),
        "{}",
        ja_gone.message
    );
    let ja_odd = check(Lang::Ja, &d.join("odd"), &d);
    assert!(
        ja_odd.message.contains("バージョンを答えませんでした"),
        "{}",
        ja_odd.message
    );
}
