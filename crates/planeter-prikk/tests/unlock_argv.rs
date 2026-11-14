//! `unlock`: the driver clears a lock with `--force` and never with `--yes`. prikk 0.57.0 still lets `--yes` clear a lock whose holder is
//! `unknown` (what a process in another PID namespace is, which is what the sandbox produces); the fix is in 0.58.0, where `--yes` is
//! dropped for `--force`. planeter passes only `--force`, on every version, so that a lock is cleared on its own evidence
//! (the witnesses of the recovery procedure) and not on prikk's liveness line. The argv is recorded by a stand-in prikk.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use planeter_prikk::cli::CliPrikkRepo;
use planeter_prikk::{PrikkRepo, Sandbox};

fn stand_in(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("unlock-argv-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    let log = dir.join("argv.log");
    let bin = dir.join("prikk");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'prikk 0.57.0'; exit 0; fi\necho \"$*\" >> {}\nexit 0\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..200 {
        match std::process::Command::new(&bin).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    (dir.join("repo"), bin, log)
}

#[test]
fn the_argv_the_driver_builds_for_unlock_has_force_and_never_yes() {
    let (repo, bin, log) = stand_in("force");
    let driver = CliPrikkRepo::with_binary(repo, bin).with_sandbox(Sandbox::Unconfined);
    driver
        .unlock_force("/some/repo/.prikk/active/default/active.lock")
        .unwrap();
    let argv = std::fs::read_to_string(&log).unwrap();
    let calls: Vec<&str> = argv.lines().filter(|l| l.starts_with("unlock")).collect();
    assert_eq!(calls.len(), 1, "{argv}");
    let words: Vec<&str> = calls[0].split_whitespace().collect();
    assert!(words.contains(&"--force"), "{calls:?}");
    assert!(!words.contains(&"--yes"), "{calls:?}");
    assert!(!words.contains(&"-y"), "{calls:?}");
    assert_eq!(
        words,
        [
            "unlock",
            "--lock",
            "/some/repo/.prikk/active/default/active.lock",
            "--force"
        ]
    );
}

/// Nothing in the driver's or the server's source builds an `unlock` argv with `--yes`: the one place that clears a lock is `cli.rs`.
#[test]
fn no_source_file_of_the_workspace_passes_yes_to_unlock() {
    fn walk(dir: &Path, found: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, found);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                for (i, line) in text.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("\"--yes\"") || code.contains("\"-y\"") {
                        found.push(format!("{}:{}: {}", p.display(), i + 1, line.trim()));
                    }
                }
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    for c in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = c.path().join("src");
        if src.is_dir() {
            walk(&src, &mut found);
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}
