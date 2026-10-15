//! T3 (prikk 0.52.0 re-baseline, PK-31/PK-40): the incoming-bound seam on the sandbox profile. Pure
//! argument-vector tests — no prikk or bubblewrap is run, so these never self-skip.

use std::ffi::OsStr;
use std::path::Path;

use planeter_prikk::Sandbox;
use planeter_prikk::sandbox::{BwrapConfig, IncomingBounds};

fn args_of(sandbox: &Sandbox) -> Vec<String> {
    sandbox
        .command(OsStr::new("prikk"), Path::new("/repo"), &["status"])
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// Every `--setenv NAME VALUE` triple, in order.
fn setenvs(args: &[String]) -> Vec<(String, String)> {
    args.windows(3)
        .filter(|w| w[0] == "--setenv")
        .map(|w| (w[1].clone(), w[2].clone()))
        .collect()
}

#[test]
fn default_profile_passes_only_path() {
    let args = args_of(&Sandbox::default());
    assert_eq!(setenvs(&args), [("PATH".to_owned(), "/usr/bin".to_owned())]);
}

#[test]
fn each_bound_is_emitted_only_when_set() {
    let cfg = BwrapConfig {
        bounds: IncomingBounds {
            exchange_max_bytes: Some(1_048_576),
            bundle_max_objects: Some(500),
            ..IncomingBounds::default()
        },
        ..BwrapConfig::default()
    };
    let args = args_of(&Sandbox::Bwrap(cfg));
    assert_eq!(
        setenvs(&args),
        [
            ("PATH".to_owned(), "/usr/bin".to_owned()),
            ("PRIKK_EXCHANGE_MAX_BYTES".to_owned(), "1048576".to_owned()),
            ("PRIKK_BUNDLE_MAX_OBJECTS".to_owned(), "500".to_owned()),
        ]
    );
    for absent in ["PRIKK_EXCHANGE_MAX_OBJECTS", "PRIKK_BUNDLE_MAX_BYTES"] {
        assert!(
            !args.iter().any(|a| a == absent),
            "{absent} must not appear"
        );
    }
}

#[test]
fn all_four_bounds_use_prikks_documented_names() {
    let cfg = BwrapConfig {
        bounds: IncomingBounds {
            exchange_max_bytes: Some(1),
            exchange_max_objects: Some(2),
            bundle_max_bytes: Some(3),
            bundle_max_objects: Some(4),
        },
        ..BwrapConfig::default()
    };
    let got = setenvs(&args_of(&Sandbox::Bwrap(cfg)));
    let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "PATH",
            "PRIKK_EXCHANGE_MAX_BYTES",
            "PRIKK_EXCHANGE_MAX_OBJECTS",
            "PRIKK_BUNDLE_MAX_BYTES",
            "PRIKK_BUNDLE_MAX_OBJECTS",
        ]
    );
}

#[test]
fn bounds_precede_the_separator_and_the_command_is_unchanged() {
    let cfg = BwrapConfig {
        bounds: IncomingBounds {
            exchange_max_bytes: Some(7),
            ..IncomingBounds::default()
        },
        ..BwrapConfig::default()
    };
    let args = args_of(&Sandbox::Bwrap(cfg));
    let sep = args.iter().position(|a| a == "--").expect("-- separator");
    let bound = args
        .iter()
        .position(|a| a == "PRIKK_EXCHANGE_MAX_BYTES")
        .expect("bound present");
    assert!(bound < sep, "the bound must be set before prikk is exec'd");
    assert_eq!(args.last().map(String::as_str), Some("status"));
    assert!(
        args.contains(&"--clearenv".to_owned()),
        "env is still cleared"
    );
}
