//! `unlock`: the driver clears a lock with `--force` and never with `--yes`. prikk 0.57.0 lets `--yes` clear a lock whose holder is
//! `unknown` (what a process in another PID namespace is, which is what the sandbox produces); **0.58.0 does not** (measured in the 0.58
//! re-baseline, A6: a sandboxed accept killed with SIGKILL leaves `liveness: unknown`; `unlock --lock <path> --yes` exits 1 and leaves the
//! lock, `--force` clears it; a holder that "does not appear to be running" is still cleared by `--yes` on both). planeter passes only `--force`, on every version, so that a lock is cleared on its own evidence
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
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'prikk 0.58.0'; exit 0; fi\necho \"$*\" >> {}\nexit 0\n",
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

/// The scan, as a function of one file's path and text, so that it can be shown to fail: the lines of `text` that carry a `"--yes"` or a `"-y"`
/// (outside a `//` comment), **but** the one legitimate line: `planeter-client deliver --yes`, the client's own command-line option ("deliver
/// without asking"), parsed in the client's `cli.rs` as exactly `"--yes" => out.yes = true,`. It is **an exact-line match in one file**: a second
/// `"--yes"` in that file (an `unlock --yes`, say) and the same line in any other file are findings. The deliver module's own scan
/// (`planeter-client/tests/deliver.rs`: `the_deliver_module_never_calls_…_yes_on_prikk`) forbids the literal in the command's source.
fn yes_findings(path: &Path, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let clients_own_option = path.ends_with("planeter-client/src/cli.rs")
            && code.trim() == "\"--yes\" => out.yes = true,";
        if !clients_own_option && (code.contains("\"--yes\"") || code.contains("\"-y\"")) {
            found.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
        }
    }
    found
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
                found.extend(yes_findings(&p, &std::fs::read_to_string(&p).unwrap()));
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

/// The exemption is an exact-line match in one file: **the test that fails if a second `"--yes"` appears** (RFC 008 handoff 4, T0.3).
#[test]
fn the_exemption_for_the_clients_own_yes_option_is_one_exact_line_in_one_file() {
    let client_cli = Path::new("crates/planeter-client/src/cli.rs");
    let the_line = "                \"--yes\" => out.yes = true,\n";
    // the real line alone: no finding
    assert!(yes_findings(client_cli, the_line).is_empty());
    // a second `"--yes"` in the same file — as an `unlock` argument, as another match arm, or in a list — is a finding
    for planted in [
        "    prikk.run(\"unlock\", &[\"unlock\", \"--lock\", p, \"--yes\"]);\n",
        "                \"--yes\" => out.assume = true,\n",
        "    let args = [\"--yes\", \"-y\"];\n",
    ] {
        let text = format!("{the_line}{planted}");
        let found = yes_findings(client_cli, &text);
        assert_eq!(found.len(), 1, "{planted:?}: {found:?}");
        assert!(found[0].contains(":2:"), "{found:?}");
    }
    // the same line in any other file is a finding
    assert_eq!(
        yes_findings(Path::new("crates/planeter-prikk/src/cli.rs"), the_line).len(),
        1
    );
    assert_eq!(
        yes_findings(
            Path::new("crates/planeter-client/src/commands/deliver.rs"),
            the_line
        )
        .len(),
        1
    );
    // a comment is not code
    assert!(yes_findings(client_cli, "    // the option \"--yes\" is the client's\n").is_empty());
}
