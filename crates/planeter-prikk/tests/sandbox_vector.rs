//! RFC 004 D-6: the confinement vector of every existing call, pinned byte for byte. Written and passing
//! **before** the optional mounts existed (commit "Pin the sandbox vector…"); it must keep passing with none
//! configured.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use planeter_prikk::Sandbox;
use planeter_prikk::sandbox::{BwrapConfig, IncomingBounds};

fn vector(c: &Command) -> Vec<String> {
    c.get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// RFC 004 D-6: the confinement vector of every existing call, **byte for byte** (this test was written and
/// passed before the optional mounts existed; it must keep passing without them configured).
#[test]
fn the_default_vector_is_pinned_byte_for_byte() {
    let c = Sandbox::default().command(
        OsStr::new("/opt/prikk"),
        Path::new("/repo"),
        &["status", "--json"],
    );
    assert_eq!(c.get_program(), OsStr::new("bwrap"));
    let golden = [
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/lib",
        "/lib64",
        "--symlink",
        "usr/bin",
        "/bin",
        "--symlink",
        "usr/bin",
        "/sbin",
        "--ro-bind",
        "/opt/prikk",
        "/opt/prikk",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--bind",
        "/repo",
        "/repo",
        "--chdir",
        "/repo",
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--",
        "/opt/prikk",
        "status",
        "--json",
    ];
    assert_eq!(vector(&c), golden);
    // With the incoming bounds set (an existing option) the environment pairs follow, as before.
    let cfg = BwrapConfig {
        bounds: IncomingBounds {
            exchange_max_bytes: Some(10),
            bundle_max_objects: Some(7),
            ..IncomingBounds::default()
        },
        ..BwrapConfig::default()
    };
    let c = Sandbox::Bwrap(cfg).command(OsStr::new("/opt/prikk"), Path::new("/repo"), &["log"]);
    let v = vector(&c);
    let at = v
        .iter()
        .position(|a| a == "PRIKK_EXCHANGE_MAX_BYTES")
        .unwrap();
    assert_eq!(
        &v[at - 1..],
        [
            "--setenv",
            "PRIKK_EXCHANGE_MAX_BYTES",
            "10",
            "--setenv",
            "PRIKK_BUNDLE_MAX_OBJECTS",
            "7",
            "--",
            "/opt/prikk",
            "log"
        ]
    );
}
