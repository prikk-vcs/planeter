use super::*;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("planeter-plock-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_lock_is_taken_and_removed_on_drop() {
    let d = dir("basic");
    let p = d.join("planeter.db.lock");
    let lock = ProcessLock::acquire(&p).unwrap();
    assert_eq!(
        fs::read_to_string(&p).unwrap().trim(),
        std::process::id().to_string()
    );
    drop(lock);
    assert!(!p.exists());
    // And can be taken again.
    drop(ProcessLock::acquire(&p).unwrap());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_second_holder_is_refused_with_the_pid_and_what_to_do() {
    let d = dir("second");
    let p = d.join("x.lock");
    // Process 1 always exists on Linux; the "other server" is pid 1 here.
    fs::write(&p, "1\n").unwrap();
    let e = ProcessLock::acquire_as(&p, 4_000_000).unwrap_err();
    assert!(
        matches!(e, ProcessLockError::AlreadyRunning { pid: 1, .. }),
        "{e:?}"
    );
    let m = e.to_string();
    assert!(
        m.contains("process 1") && m.contains("only one server") && m.contains("x.lock"),
        "{m}"
    );
    // The refused starter leaves the other's file alone.
    assert_eq!(fs::read_to_string(&p).unwrap().trim(), "1");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_stale_file_from_a_dead_process_is_replaced() {
    let d = dir("stale");
    let p = d.join("x.lock");
    fs::write(&p, "4194999\n").unwrap(); // beyond pid_max on any default Linux
    let lock = ProcessLock::acquire_as(&p, 4_000_001).unwrap();
    assert_eq!(fs::read_to_string(&p).unwrap().trim(), "4000001");
    drop(lock);
    assert!(!p.exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_file_that_names_no_process_is_not_guessed_at() {
    let d = dir("unclear");
    let p = d.join("x.lock");
    fs::write(&p, "not a pid").unwrap();
    let e = ProcessLock::acquire(&p).unwrap_err();
    assert!(matches!(e, ProcessLockError::Unclear { .. }), "{e:?}");
    assert!(e.to_string().contains("remove the file"));
    assert!(p.exists(), "an unclear file is left in place");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn dropping_never_removes_another_processs_lock() {
    let d = dir("foreign");
    let p = d.join("x.lock");
    let lock = ProcessLock::acquire_as(&p, 4_000_002).unwrap();
    fs::write(&p, "1\n").unwrap(); // someone replaced it
    drop(lock);
    assert_eq!(fs::read_to_string(&p).unwrap().trim(), "1");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn an_unwritable_place_is_a_clear_error() {
    let e = ProcessLock::acquire("/nonexistent-dir-for-planeter/x.lock").unwrap_err();
    assert!(matches!(e, ProcessLockError::Io { .. }));
    assert!(e.to_string().contains("cannot take the lock file"));
}
