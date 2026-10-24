//! The version pin at the 0.54 floor (RFC 005 re-baseline, T1): a prikk below 0.54.0 is refused with a sentence that says
//! what was found and what is required; 0.54.0 and 0.54.1 are accepted. The binary is a stand-in that answers `--version`.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use planeter_prikk::cli::{CliPrikkRepo, MIN_PRIKK_VERSION};
use planeter_prikk::{PrikkError, Sandbox};

fn stand_in(name: &str, version: &str) -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("floor-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    let bin = dir.join("prikk");
    std::fs::write(&bin, format!("#!/bin/sh\necho 'prikk {version}'\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..200 {
        match std::process::Command::new(&bin).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    (dir.join("repo"), bin)
}

fn checked(name: &str, version: &str) -> Result<CliPrikkRepo, PrikkError> {
    let (repo, bin) = stand_in(name, version);
    CliPrikkRepo::with_binary(repo, bin)
        .with_sandbox(Sandbox::Unconfined)
        .checked()
}

#[test]
fn the_floor_is_0_54_0() {
    assert_eq!(MIN_PRIKK_VERSION, (0, 54, 0));
}

#[test]
fn prikk_0_53_0_and_0_52_0_are_refused_with_a_sentence_that_says_what_was_found_and_what_is_required()
 {
    for found in ["0.53.0", "0.52.0", "0.46.0"] {
        let err = checked(&format!("old-{found}"), found).unwrap_err();
        assert!(
            matches!(err, PrikkError::UnsupportedVersion { .. }),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!("unsupported prikk version {found}; planeter requires >= 0.54.0")
        );
    }
}

#[test]
fn prikk_0_54_0_and_0_54_1_and_later_are_accepted() {
    for ok in ["0.54.0", "0.54.1", "0.55.0", "1.0.0"] {
        checked(&format!("ok-{ok}"), ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
}
