//! The layering gate (RFC 001 `LAY-1`): dependencies point **downward only**, so no lower/core crate
//! may declare a dependency on an upper crate. Enforced here by reading each crate's `Cargo.toml`
//! (std-only, no external dependency). A violation fails naming the edge — the discipline prikk settled
//! in its own RFC 149.

use std::fs;
use std::path::{Path, PathBuf};

/// Lower + core crates: none of these may depend on an upper crate.
const LOWER_CORE: &[&str] = &[
    "planeter-prikk",
    "planeter-store",
    "planeter-core",
    "planeter-auth",
];

/// Upper (surface) crates: allowed to depend downward, never depended on by lower/core.
const UPPER: &[&str] = &[
    "planeter-web",
    "planeter-transport",
    "planeter-ci",
    "planeter-registry",
];

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/tools/boundary-check
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the manifest dir")
        .to_path_buf()
}

/// Does `manifest` declare a dependency on crate `dep`? A dependency line is `dep = ...`,
/// `dep.workspace = true`, or `dep = { ... }`; the trailing check avoids matching a longer name.
fn declares_dep_on(manifest: &str, dep: &str) -> bool {
    manifest.lines().any(|line| {
        let t = line.trim_start();
        t.starts_with(dep) && matches!(t[dep.len()..].chars().next(), Some(' ' | '=' | '.'))
    })
}

#[test]
fn lower_and_core_do_not_depend_on_upper() {
    let root = workspace_root();
    for name in LOWER_CORE {
        let path = root.join("crates").join(name).join("Cargo.toml");
        let manifest =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for upper in UPPER {
            assert!(
                !declares_dep_on(&manifest, upper),
                "LAYER violation (RFC 001 LAY-1): lower/core crate `{name}` declares a dependency on \
                 upper crate `{upper}`"
            );
        }
    }
}

#[test]
fn all_layer_crates_exist() {
    let root = workspace_root();
    for name in LOWER_CORE.iter().chain(UPPER.iter()) {
        let path = root.join("crates").join(name).join("Cargo.toml");
        assert!(
            path.exists(),
            "layer crate `{name}` missing at {}",
            path.display()
        );
    }
}

#[test]
fn gate_catches_a_planted_upward_edge() {
    // RFC 001 T1 acceptance: the gate must fail a planted edge.
    let planted = "[dependencies]\nplaneter-web = { path = \"../planeter-web\" }\n";
    assert!(declares_dep_on(planted, "planeter-web"));

    // ...and must not false-positive on a legitimate downward edge or a longer name.
    let clean = "[dependencies]\nplaneter-store = { path = \"../planeter-store\" }\n";
    assert!(!declares_dep_on(clean, "planeter-web"));
    let longer = "planeter-webhook = \"1\"\n";
    assert!(!declares_dep_on(longer, "planeter-web"));
}
