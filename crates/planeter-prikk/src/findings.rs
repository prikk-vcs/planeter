//! What the forge learned about an archive before anyone trusted it (RFC 004 part 7): the maintainer keys it carries, where each
//! of its refs would land, how many objects it holds, and whether the repository it makes would verify.
//!
//! The record is written when an archive is **staged** and read when the creator confirms. It lives in this crate because every
//! fact in it is something prikk said; it is stored as JSON text by the stage store, which treats it as opaque.

use serde::{Deserialize, Serialize};

/// The most refs a record lists (a hostile archive may carry a great many; the count is kept in full).
pub const REFS_LISTED_MAX: usize = 200;
/// The most maintainer keys an archive may carry for a stage to be kept.
pub const KEYS_MAX: usize = 64;

/// One carried maintainer key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundKey {
    /// prikk's short id.
    pub key_id: String,
    /// The public key, 64 lowercase hexadecimal characters: the **fingerprint** the creator is asked to type.
    pub public_key: String,
}

/// Where one ref of the archive lands **if every carried key is adopted** (the best case).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundRef {
    pub ref_name: String,
    /// It would be a published branch.
    pub publishable: bool,
    /// The name it would land under.
    pub landed_as: String,
    /// prikk's reason when it would not be published (its words, kept as they came).
    pub reason: Option<String>,
}

/// Everything the stage kept about an archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveFindings {
    pub keys: Vec<FoundKey>,
    pub refs: Vec<FoundRef>,
    /// How many refs the archive carries (more than `refs` lists when it carries a great many).
    pub refs_total: u64,
    /// Objects by kind (`patch`, `blob`, `block`, …), as the dry-run import wrote them.
    pub objects: std::collections::BTreeMap<String, u64>,
    pub object_total: u64,
    /// The repository that the dry run made — every carried key adopted — passed `verify`.
    pub verifies_with_all_keys: bool,
    /// The ids of the conditions that failed when it did not (prikk's ids, e.g. `publication-trust`).
    pub verify_failures: Vec<String>,
    /// prikk's own note about the import (kept as it came, may be empty).
    pub note: Option<String>,
}

impl ArchiveFindings {
    /// The JSON text the stage store keeps.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Read it back; `None` for anything that is not this record.
    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}
