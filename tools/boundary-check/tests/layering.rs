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

/// The client helper (RFC 004 part 9, D-50) is a program a person runs on their own machine; the server must not gain its HTTP
/// and TLS stack, and the helper must not gain the server's crates.
const CLIENT_ONLY_DEPS: &[&str] = &["ureq", "rustls", "webpki-roots", "planeter-client"];

#[test]
fn the_servers_crates_do_not_depend_on_the_clients_stack() {
    let root = workspace_root();
    for name in LOWER_CORE.iter().chain(UPPER.iter()).chain(&["planeter"]) {
        let path = root.join("crates").join(name).join("Cargo.toml");
        let manifest =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for dep in CLIENT_ONLY_DEPS {
            assert!(
                !declares_dep_on(&manifest, dep),
                "the server crate `{name}` declares a dependency on `{dep}`, the client helper's stack (D-50)"
            );
        }
    }
}

#[test]
fn the_client_depends_on_no_server_crate() {
    let path = workspace_root().join("crates/planeter-client/Cargo.toml");
    let manifest =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    for dep in LOWER_CORE.iter().chain(UPPER.iter()).chain(&["planeter"]) {
        assert!(
            !declares_dep_on(&manifest, dep),
            "the client helper declares a dependency on the server crate `{dep}`"
        );
    }
}

#[test]
fn the_client_boundary_check_finds_a_planted_edge() {
    let planted = "[dependencies]\nureq = \"3\"\n";
    assert!(CLIENT_ONLY_DEPS.iter().any(|d| declares_dep_on(planted, d)));
    let clean = "[dependencies]\nserde = \"1\"\n";
    assert!(!CLIENT_ONLY_DEPS.iter().any(|d| declares_dep_on(clean, d)));
}

/// The language registry (handoff `language-registry-handoff-v1` T1) is shared by the server and the command-line program, so it may
/// depend on neither: no `planeter-*` crate and no external crate at all. The server crate that renders messages (`planeter-web`) and
/// the client may depend on it; nothing else needs to.
#[test]
fn the_shared_language_crate_depends_on_nothing() {
    let path = workspace_root().join("crates/planeter-i18n/Cargo.toml");
    let manifest =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let deps: Vec<&str> = {
        let mut on = false;
        let mut out = Vec::new();
        for line in manifest.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                on = t == "[dependencies]"
                    || t == "[dev-dependencies]"
                    || t == "[build-dependencies]";
                continue;
            }
            if on && !t.is_empty() && !t.starts_with('#') {
                out.push(t);
            }
        }
        out
    };
    assert!(
        deps.is_empty(),
        "planeter-i18n declares dependencies: {deps:?}"
    );
}

#[test]
fn only_the_server_that_renders_messages_the_client_and_the_tools_depend_on_the_language_crate() {
    let root = workspace_root();
    let mut dependents = Vec::new();
    for dir in ["crates", "tools"] {
        for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if let Ok(text) = fs::read_to_string(&manifest)
                && declares_dep_on(&text, "planeter-i18n")
            {
                dependents.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    dependents.sort();
    assert_eq!(
        dependents,
        ["i18n-sheet", "planeter-client", "planeter-web"],
        "{dependents:?}"
    );
    // The lower and core crates never depend on it (they say nothing a person reads).
    for name in LOWER_CORE {
        let text = fs::read_to_string(root.join("crates").join(name).join("Cargo.toml")).unwrap();
        assert!(!declares_dep_on(&text, "planeter-i18n"), "{name}");
    }
}

#[test]
fn the_language_crate_check_finds_a_planted_dependency() {
    let planted = "[package]\nname = \"planeter-i18n\"\n[dependencies]\nserde = \"1\"\n";
    assert!(declares_dep_on(
        "[dependencies]\nplaneter-i18n = { path = \"../x\" }\n",
        "planeter-i18n"
    ));
    assert!(!declares_dep_on(planted, "planeter-i18n"));
}
