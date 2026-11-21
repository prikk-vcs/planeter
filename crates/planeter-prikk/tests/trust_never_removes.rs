//! **RFC 013 D-1: planeter never removes a trusted key.** `prikk trust maintainer remove` means "this key's signatures never count": removing a
//! key that signed anything makes `verify` fail for everything it sealed (ledger PK-90, measured on prikk 0.58.0; PK-91). A repository on
//! which it was run cannot be repaired from planeter, and a page that said "retired" would claim something no component enforces. So no code of
//! planeter may spawn it, and the driver — the one door to prikk — has no method that could.
//!
//! Two tests hold it:
//!
//! * (a) a **source scan** over the non-test source of every crate (and the tools): no code names `trust … remove` — as the words, as a string
//!   literal `"remove"` near a `"trust"` or `"maintainer"` one, or as a method of the driver whose name has both words;
//! * (b) the **driver's trust calls**, made for real against a stand-in `prikk` that records what it is asked: every call the driver can build
//!   for the `trust` verb is `maintainer add`, `maintainer list` or `maintainer check`, and the set of the driver's `trust` methods is exactly
//!   those three (a fourth, a `remove`, is a new test and a decision, not a quiet addition).
//!
//! The mutation that shows them: add `fn trust_maintainer_remove` to the driver (trait and implementation) → both fail; take it out → both pass.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use planeter_prikk::cli::CliPrikkRepo;
use planeter_prikk::{PrikkRepo, Sandbox};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.rs` file under a `src` directory of a crate or a tool: the non-test source (the `tests/` directories are not scanned: they may
/// *say* the words, to hold them).
fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for group in ["crates", "tools"] {
        let Ok(rd) = std::fs::read_dir(workspace().join(group)) else {
            continue;
        };
        for c in rd.flatten() {
            walk(&c.path().join("src"), &mut out);
        }
    }
    assert!(out.len() > 50, "the scan found only {} files", out.len());
    out
}

/// The code of a line: what is before a `//` comment, and nothing at all for a doc or comment line.
fn code(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with("//") {
        return "";
    }
    line.split("//").next().unwrap_or("")
}

/// The findings of the scan over one file's text. Pure, so that it can be shown to fail.
fn findings(path: &Path, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(code).collect();
    let mut found = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let lower = l.to_lowercase();
        let named = lower.contains("trust maintainer remove")
            || lower.contains("trust_maintainer_remove")
            || lower.contains("maintainer_remove")
            || (lower.contains("fn ") && lower.contains("trust") && lower.contains("remove"));
        // the word `remove` as an argument, with `trust` or `maintainer` among the neighbouring lines (an argv built over several lines)
        let near = l.contains("\"remove\"") && {
            let from = i.saturating_sub(4);
            let to = (i + 4).min(lines.len() - 1);
            lines[from..=to]
                .iter()
                .any(|n| n.contains("\"trust\"") || n.contains("\"maintainer\""))
        };
        if named || near {
            found.push(format!("{}:{}: {}", path.display(), i + 1, l.trim()));
        }
    }
    found
}

/// (a) No code of planeter can spawn `prikk trust maintainer remove` (RFC 013 D-1, ledger PK-90).
#[test]
fn no_source_of_the_workspace_can_spawn_trust_maintainer_remove() {
    let mut found = Vec::new();
    for f in source_files() {
        found.extend(findings(&f, &std::fs::read_to_string(&f).unwrap()));
    }
    assert!(
        found.is_empty(),
        "RFC 013 D-1: planeter never removes a trusted key (prikk's `remove` makes `verify` fail for everything the key sealed, PK-90):\n{found:#?}"
    );
}

/// The scan itself: it finds what it is meant to find, and nothing in an ordinary line.
#[test]
fn the_scan_finds_a_remove_however_it_is_written() {
    let p = Path::new("x.rs");
    for bad in [
        "    self.run(&[\"trust\", \"maintainer\", \"remove\", \"--key-id\", key_id])",
        "    fn trust_maintainer_remove(&self, key_id: &str) -> Result<()>;",
        "    let verb = \"trust maintainer remove\";",
        "    fn remove_trusted_key(&self) {} fn trust_remove(&self) {}",
    ] {
        assert_eq!(findings(p, bad).len(), 1, "{bad}");
    }
    // an argv over several lines
    let split = "let args = [\n    \"trust\",\n    \"maintainer\",\n    \"remove\",\n];";
    assert_eq!(findings(p, split).len(), 1);
    // ordinary text is not a finding: a comment that says why, `remove` far from `trust`, `remove_dir_all`
    for fine in [
        "    // planeter never calls `trust maintainer remove` (RFC 013 D-1)",
        "    std::fs::remove_dir_all(&dir).unwrap();",
        "    let verb = \"remove\"; // a file",
        "    fn trust_list(&self) -> Result<model::TrustList>;",
    ] {
        assert!(findings(p, fine).is_empty(), "{fine}");
    }
}

/// The names of the driver's methods about `trust`, as written in the trait and in its implementation.
fn driver_trust_methods() -> Vec<String> {
    let mut names = Vec::new();
    for f in ["src/repo.rs", "src/cli.rs"] {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(f)).unwrap();
        for l in text.lines().map(code) {
            if let Some(rest) = l.trim_start().strip_prefix("fn ")
                && rest.starts_with("trust")
            {
                names.push(rest.split(['(', '<']).next().unwrap().to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// A stand-in `prikk` that records its argv (and answers the three trust reports well enough for the driver).
fn recording_standin(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("trust-never-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    let log = dir.join("argv.log");
    let bin = dir.join("prikk");
    std::fs::write(
        &bin,
        format!(
            r#"#!/bin/sh
if [ "$1" = --version ]; then echo 'prikk 0.58.0'; exit 0; fi
echo "$*" >> {log}
case "$1 $2 $3" in
  "trust maintainer list") echo '{{"schema_version":"trust-list-v1","keys":[]}}' ;;
  "trust maintainer check") echo '{{"schema_version":"trust-check-v1","key_id":"k","trusted":false,"public_key":null}}'; exit 1 ;;
  *) ;;
esac
exit 0
"#,
            log = log.display()
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

/// (b) Every call the driver can build for the `trust` verb is `maintainer add`, `maintainer list` or `maintainer check`, and the driver's
/// `trust` methods are exactly those three.
#[test]
fn the_drivers_trust_calls_are_add_list_and_check_and_nothing_else() {
    assert_eq!(
        driver_trust_methods(),
        ["trust_check", "trust_list", "trust_maintainer_add"],
        "RFC 013 D-1: the driver has no method that could remove a trusted key; a new `trust` method is a decision, and a new test"
    );
    let (repo, bin, log) = recording_standin("argv");
    let driver = CliPrikkRepo::with_binary(repo, bin).with_sandbox(Sandbox::Unconfined);
    driver.trust_list().unwrap();
    driver.trust_check("ed25519-0123456789abcdef").unwrap();
    driver
        .trust_maintainer_add("ed25519-0123456789abcdef", &"ab".repeat(32))
        .ok();
    let calls: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("trust"))
        .map(str::to_owned)
        .collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    for c in &calls {
        let w: Vec<&str> = c.split_whitespace().collect();
        assert_eq!(w[0], "trust", "{c}");
        assert_eq!(w[1], "maintainer", "{c}");
        assert!(matches!(w[2], "add" | "list" | "check"), "{c}");
        assert!(!w.contains(&"remove"), "{c}");
    }
    assert!(calls.iter().any(|c| c.starts_with("trust maintainer list")));
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("trust maintainer check"))
    );
    assert!(calls.iter().any(|c| c.starts_with("trust maintainer add")));
}
