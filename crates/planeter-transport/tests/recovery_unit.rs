//! RFC 004 part 4: the pure parts of recovery — whose doing a flag is, and whether a lock is the repository's own.

use planeter_transport::{Cause, classify_cause, inside_repository};

#[test]
fn a_flag_is_ours_only_when_it_says_one_of_our_causes() {
    for c in [
        "accept_deadline",
        "accept_ceiling",
        "accept_failed",
        "accept_unreadable",
        "accept_not_recorded",
    ] {
        assert_eq!(
            classify_cause(&format!("a write did not finish ({c})")).code(),
            c
        );
    }
    assert_eq!(
        classify_cause("the server stopped during a write (sync accept, 1 open intent(s))"),
        Cause::Ours("server_stopped")
    );
    for other in [
        "",
        "x",
        "set by hand",
        "a write did not finish (accept_something_new)",
        "a write did not finish (accept_failed",
        "a write did not finish ()",
        "A write did not finish (accept_failed)",
        "a write did not finish (accept_failed) and more",
    ] {
        assert_eq!(classify_cause(other), Cause::Other, "{other:?}");
    }
}

#[test]
fn a_lock_is_the_repositorys_only_when_it_resolves_to_a_lock_file_inside_its_dot_prikk() {
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("recovery-unit-locks");
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    std::fs::create_dir_all(repo.join(".prikk/active/default")).unwrap();
    std::fs::create_dir_all(base.join("elsewhere")).unwrap();
    let dir = repo.to_str().unwrap();
    let inside = repo.join(".prikk/active/default/active.lock");
    std::fs::write(&inside, "x").unwrap();
    assert!(inside_repository(dir, inside.to_str().unwrap()));
    // not a lock file
    let data = repo.join(".prikk/active/default/queue.wal");
    std::fs::write(&data, "x").unwrap();
    assert!(!inside_repository(dir, data.to_str().unwrap()));
    // outside the repository
    let outside = base.join("elsewhere/a.lock");
    std::fs::write(&outside, "x").unwrap();
    assert!(!inside_repository(dir, outside.to_str().unwrap()));
    // a link inside that points outside
    let link = repo.join(".prikk/active/default/link.lock");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(!inside_repository(dir, link.to_str().unwrap()));
    // `..` that climbs out
    let climb = format!("{dir}/.prikk/../../elsewhere/a.lock");
    assert!(!inside_repository(dir, &climb));
    // `..` that stays in is still inside
    let stay = format!("{dir}/.prikk/active/../active/default/active.lock");
    assert!(inside_repository(dir, &stay));
    // missing file, missing repository
    assert!(!inside_repository(
        dir,
        repo.join(".prikk/nope.lock").to_str().unwrap()
    ));
    assert!(!inside_repository(
        base.join("nowhere").to_str().unwrap(),
        inside.to_str().unwrap()
    ));
    // the repository reached through a link to it
    let alias = base.join("alias");
    std::os::unix::fs::symlink(&repo, &alias).unwrap();
    assert!(inside_repository(
        alias.to_str().unwrap(),
        inside.to_str().unwrap()
    ));
    assert!(inside_repository(
        dir,
        alias
            .join(".prikk/active/default/active.lock")
            .to_str()
            .unwrap()
    ));
}
