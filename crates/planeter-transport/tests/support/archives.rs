//! Real maintainers' archives for the tests of repository creation (RFC 004 part 7): a keyed prikk builds the history and
//! `archive export` writes the archive, exactly as a maintainer would. Included by path from the web and binary tests too.
//! Nothing here is part of the product.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Whether a prikk is on `PATH` (the tests that need one say why they skip).
pub fn prikk_available() -> bool {
    Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// One carried key as the archive reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub key_id: String,
    pub public_key: String,
}

/// A maintainer: a working repository and a key directory of their own.
pub struct Maintainer {
    pub work: PathBuf,
    pub keys: PathBuf,
}

pub fn run(dir: &Path, keys: &Path, args: &[&str]) -> String {
    let out = Command::new("prikk")
        .args(args)
        .current_dir(dir)
        .env("HOME", keys)
        .env("XDG_CONFIG_HOME", keys)
        .output()
        .expect("run prikk");
    assert!(
        out.status.success(),
        "prikk {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

impl Maintainer {
    /// A new maintainer under `base/<name>` with one sealed commit on `heads/main`.
    pub fn new(base: &Path, name: &str) -> Maintainer {
        let work = base.join(name);
        let keys = base.join(format!("{name}-keys"));
        for d in [&work, &keys] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).unwrap();
        }
        run(&work, &keys, &["setup", "."]);
        let m = Maintainer { work, keys };
        m.write("README.md", "# a project\n");
        m.commit("initial");
        m.seal(None);
        m
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.work.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    pub fn commit(&self, message: &str) {
        run(
            &self.work,
            &self.keys,
            &["commit", "--from-worktree", "-m", message],
        );
    }

    pub fn seal(&self, reference: Option<&str>) {
        match reference {
            None => run(&self.work, &self.keys, &["seal", "--allow-no-audit"]),
            Some(r) => run(
                &self.work,
                &self.keys,
                &["seal", "--allow-no-audit", "--ref", r],
            ),
        };
    }

    /// `n` more sealed commits, one file each.
    pub fn history(&self, n: usize) {
        for i in 0..n {
            self.write(&format!("src/f{i}.txt"), &format!("file {i}\n"));
            self.commit(&format!("add f{i}"));
        }
        self.seal(None);
    }

    pub fn archive(&self, out: &Path) -> PathBuf {
        let _ = std::fs::remove_file(out);
        run(
            &self.work,
            &self.keys,
            &["archive", "export", &out.to_string_lossy()],
        );
        out.to_path_buf()
    }

    /// The keys an archive carries, as `archive verify` reports them.
    pub fn carried(&self, archive: &Path) -> Vec<KeyInfo> {
        let json = run(
            &self.work,
            &self.keys,
            &[
                "archive",
                "verify",
                &archive.to_string_lossy(),
                "--format",
                "json",
            ],
        );
        // The two fields are found in the text; the tests' archives carry a handful of keys.
        let mut out = Vec::new();
        let mut rest = json.as_str();
        while let Some(i) = rest.find("\"key_id\"") {
            rest = &rest[i + 8..];
            let key_id = quoted(rest);
            let j = rest.find("\"public_key\"").unwrap();
            rest = &rest[j + 12..];
            out.push(KeyInfo {
                key_id,
                public_key: quoted(rest),
            });
        }
        out
    }
}

/// The first string literal in `text` after a colon.
fn quoted(text: &str) -> String {
    let start = text.find('"').unwrap() + 1;
    let end = start + text[start..].find('"').unwrap();
    text[start..end].to_owned()
}

/// A plain archive with one maintainer key, `patches` extra commits, and (when `blob_kib` is set) one file of that size.
pub fn simple_archive(
    base: &Path,
    name: &str,
    patches: usize,
    blob_kib: usize,
) -> (PathBuf, Vec<KeyInfo>) {
    let m = Maintainer::new(base, name);
    if patches > 0 {
        m.history(patches);
    }
    if blob_kib > 0 {
        m.write("big.bin", &"x".repeat(blob_kib * 1024));
        m.commit("a large file");
        m.seal(None);
    }
    let out = base.join(format!("{name}.archive"));
    m.archive(&out);
    let keys = m.carried(&out);
    (out, keys)
}

/// An archive that carries **two** maintainer keys and two published refs: `heads/main` signed by A and `heads/dev` signed by B.
pub fn two_key_archive(base: &Path, name: &str) -> (PathBuf, Vec<KeyInfo>) {
    let a = Maintainer::new(base, &format!("{name}-a"));
    let b = Maintainer::new(base, &format!("{name}-b"));
    run(
        &b.work,
        &b.keys,
        &["branch", "create", "heads/dev", "--from", "heads/main"],
    );
    run(&b.work, &b.keys, &["branch", "switch", "heads/dev"]);
    b.write("b.txt", "from B\n");
    b.commit("work of B");
    b.seal(Some("heads/dev"));
    let from_b = base.join(format!("{name}-b.archive"));
    b.archive(&from_b);
    // A adopts B's key and takes B's history in, so A's archive carries both keys and both refs.
    for k in b.carried(&from_b) {
        run(
            &a.work,
            &a.keys,
            &[
                "trust",
                "maintainer",
                "add",
                "--key-id",
                &k.key_id,
                "--public-key",
                &k.public_key,
            ],
        );
    }
    run(
        &a.work,
        &a.keys,
        &["archive", "import", &from_b.to_string_lossy(), "--adopt"],
    );
    let out = base.join(format!("{name}.archive"));
    a.archive(&out);
    let keys = a.carried(&out);
    (out, keys)
}

/// An archive whose repository **cannot verify even with every carried key adopted**: it carries a ref signed by a key it does not
/// list (A took B's history in without adopting B's key).
pub fn unverifiable_archive(base: &Path, name: &str) -> (PathBuf, Vec<KeyInfo>) {
    let a = Maintainer::new(base, &format!("{name}-a"));
    let b = Maintainer::new(base, &format!("{name}-b"));
    let from_b = base.join(format!("{name}-b.archive"));
    b.archive(&from_b);
    run(
        &a.work,
        &a.keys,
        &["archive", "import", &from_b.to_string_lossy()],
    );
    let out = base.join(format!("{name}.archive"));
    a.archive(&out);
    let keys = a.carried(&out);
    (out, keys)
}

/// Flip one byte in the middle of a copy of `archive`.
pub fn tampered(archive: &Path, out: &Path) -> PathBuf {
    let mut bytes = std::fs::read(archive).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 1;
    std::fs::write(out, bytes).unwrap();
    out.to_path_buf()
}
