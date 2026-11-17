//! The change set of one revision: what it does to the files, compared with the revision's own base (RFC 005 part 6).

use serde::{Deserialize, Serialize};

/// What happened to a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Added,
    Deleted,
    Modified,
    /// File ↔ symbolic link ↔ (empty) directory, or any other change of what the path is.
    TypeChanged,
}

/// What the path is (on the side that exists; the after side when both do).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Binary,
    Symlink,
    Directory,
}

/// Why no diff is returned for a file: a closed list, each with a sentence in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotShown {
    /// A side is over the size the diff reads (256 KiB).
    TooLarge,
    /// No NUL byte, but not valid UTF-8 (a Shift_JIS file, for one).
    NotUtf8,
    /// A NUL byte: not text.
    Binary,
    /// The change involves a symbolic link in a way a text diff cannot show (file ↔ link).
    Symlink,
    /// The change involves a directory in a way a text diff cannot show.
    Directory,
    /// Not a regular file (a pipe, a device, a socket), too deep to walk, or unreadable. Never opened.
    Unreadable,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// SHA-256 (hex) of the exact path bytes: this, never the path, goes into URLs.
    pub file_id: String,
    /// The display-safe path. A name over 1,024 bytes is its first 1,024 bytes (cut at a character boundary), with **no marker** in it.
    pub path: String,
    /// How many bytes of the name were left out of `path` (omitted when 0). The file id is still the hash of the whole name.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub path_cut_bytes: u64,
    /// The display differs from the bytes.
    pub path_escaped: bool,
    pub status: Status,
    pub kind: Kind,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
    /// Lines of a text diff (`None` when there is none).
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
    /// A diff is available for this file.
    pub shown: bool,
    pub not_shown_because: Option<NotShown>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffOp {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub op: DiffOp,
    /// A line over 2,000 characters is its first 2,000 characters, with **no marker** in it.
    pub text: String,
    /// How many characters of the line were left out of `text` (omitted when 0).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cut_chars: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: u64,
    pub old_lines: u64,
    pub new_start: u64,
    pub new_lines: u64,
    pub lines: Vec<DiffLine>,
}

/// How many diff lines were left out of a file's hunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Truncated {
    pub lines_not_shown: u64,
}

/// One file's diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub entry: FileEntry,
    pub hunks: Vec<DiffHunk>,
    pub truncated: Option<Truncated>,
    /// The edit-distance cap was exceeded: the diff is the whole-file replacement.
    pub approximate: bool,
}

/// What was compared with what, and how much to trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compared {
    /// The block whose files are the "before" tree; `None`: the proposal starts a history, so there is no earlier file.
    pub before_block: Option<String>,
    /// The foot claim's target block (the "after" tree is `extract --claim` of the foot claim).
    pub after_block: String,
    /// The revision's declared base is not the real parent; the real parent is what was compared.
    pub base_differs: bool,
    pub declared_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub files_changed: u64,
    /// Over the text files that are shown.
    pub lines_added: u64,
    pub lines_removed: u64,
    /// Files with a reason instead of a diff.
    pub unshown: u64,
}

/// The whole answer for a revision, files in display-path order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub compared: Compared,
    /// Always true: the files are rebuilt by prikk from the proposal's patches.
    pub rebuilt: bool,
    /// No checkpoint verified by the repository itself was in reach (copied from the extract report).
    pub provisional: bool,
    /// The weakest signature outcome over the chain, as prikk said it (`sound`, `unverifiable`, `invalid`); copied, not displayed.
    pub chain_signature: Option<String>,
    pub totals: Totals,
    pub files: Vec<FileEntry>,
}
