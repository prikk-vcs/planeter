//! Walking a tree that prikk wrote and that this process does not trust (RFC 005 part 6, "Walking the trees").
//!
//! * **Nothing is followed.** The type of an entry is the directory entry's own (`lstat`); a link is read (its target is bytes) and never
//!   opened; only an entry that is a regular file is ever opened, and re-checked as one just before.
//! * **Bounds.** Depth at most [`MAX_DEPTH`]; at most [`MAX_ENTRIES`] leaves per tree; a file is read only up to [`TEXT_MAX`] bytes (larger
//!   files are compared by a streamed hash and sniffed on their first [`SNIFF_BYTES`] bytes).
//! * A pipe, a device, a socket, a directory that cannot be listed and anything below [`MAX_DEPTH`] are leaves that are **never opened**.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use planeter_auth::Sha256Stream;

/// A side of a text diff is read up to this many bytes; a larger file is `too_large`.
pub const TEXT_MAX: u64 = 256 * 1024;
/// The deepest directory that is listed.
pub const MAX_DEPTH: usize = 64;
/// The most leaves a tree may have (the extract bound `DEFAULT_MAX_FILES`).
pub const MAX_ENTRIES: usize = planeter_prikk::extract::DEFAULT_MAX_FILES as usize;
/// A file over [`TEXT_MAX`] is looked at on its first bytes only, to tell text from binary.
pub const SNIFF_BYTES: usize = 8 * 1024;

/// A leaf of a tree. A directory that holds anything is not a leaf (its contents are).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    File {
        size: u64,
    },
    /// A symbolic link and its target, as bytes. Never followed.
    Symlink {
        target: Vec<u8>,
    },
    /// A directory with nothing in it.
    EmptyDir,
    /// A pipe, a device, a socket, or a directory that could not be listed: never opened.
    Special,
    /// A directory below [`MAX_DEPTH`]: not listed.
    TooDeep,
}

#[derive(Debug)]
pub enum WalkError {
    /// More than [`MAX_ENTRIES`] leaves.
    TooMany,
    Io(String),
}

/// The leaves of a tree by path bytes (relative to the root, `/`-separated).
#[derive(Debug, Clone, Default)]
pub struct Tree {
    root: PathBuf,
    pub nodes: BTreeMap<Vec<u8>, Node>,
}

impl Tree {
    /// The empty tree (a proposal that starts a history has no earlier file).
    pub fn empty() -> Tree {
        Tree::default()
    }

    /// Walk the tree under `root`.
    pub fn walk(root: &Path) -> Result<Tree, WalkError> {
        let meta =
            fs::symlink_metadata(root).map_err(|e| WalkError::Io(format!("the tree: {e}")))?;
        if !meta.is_dir() {
            return Err(WalkError::Io("the tree is not a directory".to_owned()));
        }
        let mut nodes = BTreeMap::new();
        let mut stack: Vec<(PathBuf, Vec<u8>, usize)> = vec![(root.to_path_buf(), Vec::new(), 0)];
        while let Some((dir, rel, depth)) = stack.pop() {
            let listing = match fs::read_dir(&dir) {
                Ok(l) => l,
                Err(_) => {
                    if !rel.is_empty() {
                        nodes.insert(rel, Node::Special);
                    }
                    continue;
                }
            };
            let mut any = false;
            for entry in listing {
                let Ok(entry) = entry else {
                    // an entry that cannot be read is still an entry
                    any = true;
                    continue;
                };
                any = true;
                let name = entry.file_name();
                let mut child = rel.clone();
                if !child.is_empty() {
                    child.push(b'/');
                }
                child.extend_from_slice(name.as_bytes());
                let Ok(ft) = entry.file_type() else {
                    nodes.insert(child, Node::Special);
                    continue;
                };
                if ft.is_symlink() {
                    let node = match fs::read_link(entry.path()) {
                        Ok(t) => Node::Symlink {
                            target: t.into_os_string().into_vec(),
                        },
                        Err(_) => Node::Special,
                    };
                    nodes.insert(child, node);
                } else if ft.is_file() {
                    let size = entry.metadata().map_or(0, |m| m.len());
                    nodes.insert(child, Node::File { size });
                } else if ft.is_dir() {
                    if depth + 1 > MAX_DEPTH {
                        nodes.insert(child, Node::TooDeep);
                    } else {
                        stack.push((entry.path(), child, depth + 1));
                    }
                } else {
                    nodes.insert(child, Node::Special);
                }
                if nodes.len() > MAX_ENTRIES {
                    return Err(WalkError::TooMany);
                }
            }
            if !any && !rel.is_empty() {
                nodes.insert(rel, Node::EmptyDir);
            }
        }
        Ok(Tree {
            root: root.to_path_buf(),
            nodes,
        })
    }

    /// The file system path of a leaf.
    fn path_of(&self, rel: &[u8]) -> PathBuf {
        self.root.join(OsStr::from_bytes(rel))
    }

    /// Open a leaf that is a regular file: re-checked with `lstat` first, never followed.
    fn open(&self, rel: &[u8]) -> Option<fs::File> {
        let p = self.path_of(rel);
        let meta = fs::symlink_metadata(&p).ok()?;
        if !meta.file_type().is_file() {
            return None;
        }
        fs::File::open(&p).ok()
    }

    /// Classify and, for text, read a regular file.
    pub fn load(&self, rel: &[u8], size: u64) -> Content {
        let Some(mut f) = self.open(rel) else {
            return Content::Unreadable;
        };
        if size > TEXT_MAX {
            let mut head = vec![0u8; SNIFF_BYTES];
            let n = match read_up_to(&mut f, &mut head) {
                Ok(n) => n,
                Err(_) => return Content::Unreadable,
            };
            head.truncate(n);
            return if looks_like_text(&head) {
                Content::TooLarge
            } else {
                Content::Binary
            };
        }
        let mut bytes = Vec::with_capacity(size as usize);
        if (&mut f).take(TEXT_MAX + 1).read_to_end(&mut bytes).is_err() {
            return Content::Unreadable;
        }
        if bytes.len() as u64 > TEXT_MAX {
            // grew while we looked: not what lstat said
            return Content::TooLarge;
        }
        if bytes.contains(&0) {
            return Content::Binary;
        }
        match String::from_utf8(bytes) {
            Ok(s) => Content::Text(s),
            Err(_) => Content::NotUtf8,
        }
    }

    /// A regular file's bytes, if it is one and not over [`TEXT_MAX`].
    pub fn load_bytes(&self, rel: &[u8], size: u64) -> Option<Vec<u8>> {
        if size > TEXT_MAX {
            return None;
        }
        let mut f = self.open(rel)?;
        let mut bytes = Vec::with_capacity(size as usize);
        (&mut f).take(TEXT_MAX + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= TEXT_MAX).then_some(bytes)
    }

    /// SHA-256 of a regular file's bytes, streamed (the file is not held).
    pub fn digest(&self, rel: &[u8]) -> Option<String> {
        let mut f = self.open(rel)?;
        let mut h = Sha256Stream::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Some(h.finish_hex())
    }
}

/// What a regular file holds, as far as the diff cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// A NUL byte.
    Binary,
    /// No NUL byte, not UTF-8.
    NotUtf8,
    /// Text by its first bytes, but over [`TEXT_MAX`].
    TooLarge,
    Unreadable,
}

fn read_up_to(f: &mut fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        let n = f.read(&mut buf[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    Ok(got)
}

/// No NUL, and valid UTF-8 except possibly a character cut at the end of the sample.
fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    let mut chunks = head.utf8_chunks().peekable();
    while let Some(c) = chunks.next() {
        if !c.invalid().is_empty() && (chunks.peek().is_some() || c.invalid().len() > 3) {
            return false;
        }
    }
    true
}
