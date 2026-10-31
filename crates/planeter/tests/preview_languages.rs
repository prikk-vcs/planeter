//! RFC 012 D-3f: `PLANETER_PREVIEW_LANGUAGES` — a draft language is offered only when the operator asks for it (none exists at
//! present: Japanese was signed off). Empty is silent; a language that is already offered is accepted and logged as not needed;
//! a value that names no draft language is refused with the names that exist.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("preview-languages-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn runs(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A stand-in `prikk` that answers the version pin (the server refuses to start without one).
fn stand_in_bin(dir: &Path) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("prikk");
    std::fs::write(&script, "#!/bin/sh\necho 'prikk 0.54.1'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin_dir
}

fn command(dir: &Path, bin_dir: &Path) -> Command {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut c = Command::new(env!("CARGO_BIN_EXE_planeter"));
    c.env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("PLANETER_ADDR", format!("127.0.0.1:{port}"))
        .env("PLANETER_REPOS_ROOT", dir.join("repos"))
        .env("PLANETER_DB", dir.join("db.sqlite"))
        .env("PLANETER_CONTENT_ORIGIN", "https://raw.invalid")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    c
}

/// Start the server, collect its start-up log up to "listening", and stop it.
fn start_log(dir: &Path, bin_dir: &Path, preview: Option<&str>) -> String {
    let mut c = command(dir, bin_dir);
    if let Some(v) = preview {
        c.env("PLANETER_PREVIEW_LANGUAGES", v);
    }
    let mut server = c.spawn().expect("spawn planeter");
    let lines = BufReader::new(server.stderr.take().unwrap());
    let mut seen = Vec::new();
    for line in lines.lines().map_while(Result::ok) {
        let up = line.starts_with("planeter listening on ");
        seen.push(line);
        if up {
            break;
        }
    }
    let _ = server.kill();
    let _ = server.wait();
    seen.join("\n")
}

#[test]
fn the_preview_setting_is_silent_when_empty_accepts_a_shipped_language_and_refuses_a_typo() {
    if !runs("prlimit") {
        eprintln!("skipping: needs prlimit (util-linux)");
        return;
    }
    let dir = scratch("all");
    let bin_dir = stand_in_bin(&dir);

    let off = start_log(&dir, &bin_dir, None);
    assert!(off.contains("planeter listening on"), "{off}");
    assert!(
        !off.contains("PREVIEW languages"),
        "off must be silent: {off}"
    );
    let empty = start_log(&dir, &bin_dir, Some(""));
    assert!(!empty.contains("PREVIEW languages"), "{empty}");

    // Japanese is signed off: naming it is accepted, says so once, and is not a "preview".
    let on = start_log(&dir, &bin_dir, Some("ja"));
    assert!(on.contains("planeter listening on"), "{on}");
    assert!(
        on.contains("ja already offered (signed off); the setting is not needed"),
        "{on}"
    );
    assert!(!on.contains("PREVIEW languages"), "{on}");
    assert!(!on.contains("not signed off"), "{on}");

    let out = command(&dir, &bin_dir)
        .env("PLANETER_PREVIEW_LANGUAGES", "ja,jp")
        .output()
        .expect("run planeter");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("PLANETER_PREVIEW_LANGUAGES"), "{stderr}");
    assert!(
        stderr.contains("\"jp\"") && stderr.contains("none at present"),
        "{stderr}"
    );
    assert!(
        !dir.join("db.sqlite.lock").exists(),
        "a refused start leaves no lock"
    );
}
