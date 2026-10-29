//! The prikk runner against the real prikk, on every system the client is built on (it runs where prikk is on `PATH`; the CI job installs
//! the floor on Linux, macOS and Windows): the version is read and compared with the floor, a repository is made, a failing prikk call is a
//! plain sentence naming the step with prikk's own text kept as the detail. The commands themselves need prikk's key directory (its place
//! differs by system), so their tests are the Unix ones in `flows.rs`.

use planeter_client::catalog::Lang;
use planeter_client::exit::PRIKK;
use planeter_client::prikk::Prikk;

fn dir(name: &str) -> std::path::PathBuf {
    let d =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("client-runner-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn have_prikk() -> bool {
    std::process::Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn the_installed_prikk_is_found_and_is_at_least_the_floor() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let d = dir("check");
    Prikk::new(&d, Lang::En, None)
        .check()
        .expect("the floor or newer");
}

#[test]
fn a_repository_is_made_and_recognised_and_a_failing_call_is_one_plain_sentence() {
    if !have_prikk() {
        eprintln!("skipping: needs prikk on PATH");
        return;
    }
    let d = dir("init");
    let p = Prikk::new(&d, Lang::En, None);
    assert!(!Prikk::is_repository(&d));
    p.run("make the repository", &["init", "."])
        .expect("init works");
    assert!(Prikk::is_repository(&d));
    let failure = p
        .run(
            "read a branch nobody made",
            &["log", "--format", "json", "--ref", "heads/nobody"],
        )
        .unwrap_err();
    assert_eq!(failure.code, PRIKK);
    assert!(
        failure
            .message
            .starts_with("Your prikk could not do “read a branch nobody made”:"),
        "{}",
        failure.message
    );
    assert!(
        failure.detail.is_some(),
        "prikk's own text is kept for --verbose"
    );
}

#[test]
fn a_missing_prikk_is_a_sentence_with_the_floor_in_it() {
    let d = dir("missing");
    let p = Prikk::new(&d, Lang::En, Some("definitely-not-a-prikk-program".into()));
    let failure = p.check().unwrap_err();
    assert_eq!(failure.code, PRIKK);
    assert!(
        failure.message.contains("prikk was not found") && failure.message.contains("0.54.0"),
        "{}",
        failure.message
    );
}
