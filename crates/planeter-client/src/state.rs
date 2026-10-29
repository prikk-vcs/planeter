//! What the helper remembers about a server between runs (RFC 004 D-8, pin freshness): per server and repository, the validator
//! of the last archive and the tip of each branch it was shown. It lives in the user's state directory, never in the repository or
//! the worktree. The record is only ever compared with what a server says now; a server that shows a branch at a state that does
//! not continue from the recorded one is serving a split view.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::{Key, Lang, t};
use crate::exit::{Failure, LOCAL};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefRecord {
    pub tip_block: Option<String>,
    pub ref_state_id: Option<String>,
    /// The tip's `update_seq` (RFC 004 D-8) when the server said one; only ever raised.
    #[serde(default)]
    pub update_seq: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The server and repository this record is about (`https://host/owner/name`); checked on load.
    pub key: String,
    /// The `ETag` of the last archive landed.
    pub etag: Option<String>,
    /// The highest `update_seq` seen on any branch, when the server says one.
    pub update_seq: Option<u64>,
    pub refs: BTreeMap<String, RefRecord>,
    /// The key of the work whose send ended `incomplete` (the server did not finish): repeated only with `--retry`.
    #[serde(default)]
    pub unfinished: Option<String>,
}

/// The user's state directory for this program: `PLANETER_CLIENT_STATE_DIR`, else the platform's usual place.
pub fn state_dir(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(d) = env("PLANETER_CLIENT_STATE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        if let Some(d) = env("LOCALAPPDATA").filter(|d| !d.is_empty()) {
            return PathBuf::from(d).join("planeter-client");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(h) = env("HOME").filter(|d| !d.is_empty()) {
            return PathBuf::from(h).join("Library/Application Support/planeter-client");
        }
    }
    if let Some(d) = env("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(d).join("planeter-client");
    }
    if let Some(h) = env("HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(h).join(".local/state/planeter-client");
    }
    PathBuf::from(".planeter-client-state")
}

fn file_for(dir: &Path, key: &str) -> PathBuf {
    let clean: String = key
        .trim_start_matches("https://")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{clean}.json"))
}

/// The record for `key`, or an empty one if there is none yet.
pub fn load(dir: &Path, key: &str, lang: Lang) -> Result<Record, Failure> {
    let path = file_for(dir, key);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let rec: Record = serde_json::from_slice(&bytes).map_err(|e| {
                Failure::new(
                    LOCAL,
                    t(lang, Key::LocalState).with("path", path.display()).text(),
                )
                .with_detail(e.to_string())
            })?;
            if rec.key != key {
                return Ok(Record {
                    key: key.to_owned(),
                    ..Record::default()
                });
            }
            Ok(rec)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Record {
            key: key.to_owned(),
            ..Record::default()
        }),
        Err(e) => Err(Failure::new(
            LOCAL,
            t(lang, Key::LocalState).with("path", path.display()).text(),
        )
        .with_detail(e.to_string())),
    }
}

/// Write the record (to a temporary file, then renamed).
pub fn save(dir: &Path, rec: &Record, lang: Lang) -> Result<(), Failure> {
    let path = file_for(dir, &rec.key);
    let fail = |e: std::io::Error| {
        Failure::new(
            LOCAL,
            t(lang, Key::LocalState).with("path", path.display()).text(),
        )
        .with_detail(e.to_string())
    };
    std::fs::create_dir_all(dir).map_err(fail)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(rec).unwrap_or_default()).map_err(fail)?;
    std::fs::rename(&tmp, &path).map_err(fail)
}

/// Remember what the advertisement showed: the tip, the RefState id and the sequence of each published branch. The sequence is only ever
/// raised (a lower one was refused before the download), and `update_seq` of the record is the highest of them.
pub fn record_refs(rec: &mut Record, advert: &crate::commands::Advert) {
    let before = std::mem::take(&mut rec.refs);
    for r in advert.refs.iter().filter(|r| r.published) {
        let kept = before.get(&r.name).and_then(|b| b.update_seq);
        let seq = match (kept, r.update_seq) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => b.or(a),
        };
        rec.refs.insert(
            r.name.clone(),
            RefRecord {
                tip_block: r.tip_block.clone(),
                ref_state_id: r.ref_state_id.clone(),
                update_seq: seq,
            },
        );
    }
    rec.update_seq = rec
        .refs
        .values()
        .filter_map(|r| r.update_seq)
        .max()
        .max(rec.update_seq);
}

/// Where each clone made by this program came from, by the clone's absolute path: `fetch` needs no address because of it.
pub fn remember_origin(dir: &Path, clone: &Path, url: &str, lang: Lang) -> Result<(), Failure> {
    let path = dir.join("clones.json");
    let fail = |e: std::io::Error| {
        Failure::new(
            LOCAL,
            t(lang, Key::LocalState).with("path", path.display()).text(),
        )
        .with_detail(e.to_string())
    };
    let mut map: BTreeMap<String, String> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    map.insert(canonical(clone), url.to_owned());
    std::fs::create_dir_all(dir).map_err(fail)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&map).unwrap_or_default()).map_err(fail)?;
    std::fs::rename(&tmp, &path).map_err(fail)
}

/// The address a clone came from, if this program made it.
pub fn origin_of(dir: &Path, clone: &Path) -> Option<String> {
    let bytes = std::fs::read(dir.join("clones.json")).ok()?;
    let map: BTreeMap<String, String> = serde_json::from_slice(&bytes).ok()?;
    map.get(&canonical(clone)).cloned()
}

fn canonical(p: &Path) -> String {
    std::fs::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .display()
        .to_string()
}
