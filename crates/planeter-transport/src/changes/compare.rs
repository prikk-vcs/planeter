//! Comparing two trees (RFC 005 part 6): for every path in either, `added`, `deleted`, `modified` or `type_changed`; unchanged paths
//! are dropped. A file moved to a new name is a deletion plus an addition: no rename is guessed.

use std::collections::BTreeSet;
use std::time::Instant;

use planeter_auth::sha256_hex;

use super::diff::{self, Op, diff_lines, split_lines};
use super::display::{display_line, display_line_bytes, display_path};
use super::model::{
    DiffHunk, DiffLine, DiffOp, FileDiff, FileEntry, Kind, NotShown, Status, Totals, Truncated,
};
use super::tree::{Content, MAX_ENTRIES, Node, TEXT_MAX, Tree};

#[derive(Debug, PartialEq, Eq)]
pub enum CompareError {
    /// More than [`MAX_ENTRIES`] changed files.
    TooMany,
    /// The deadline passed.
    TimedOut,
}

/// What to compute.
pub enum Want<'a> {
    /// Every changed file and its diff (the diffs go to the sink).
    All,
    /// Only the file with this id.
    One(&'a str),
}

/// The file id of a path: the SHA-256 (hex) of its exact bytes.
pub fn file_id(path: &[u8]) -> String {
    sha256_hex(path)
}

/// Compare `before` with `after`. Each changed file's [`FileDiff`] is handed to `sink`; the entries come back in display-path order.
pub fn compare(
    before: &Tree,
    after: &Tree,
    want: &Want<'_>,
    deadline: Instant,
    sink: &mut dyn FnMut(FileDiff),
) -> Result<(Vec<FileEntry>, Totals), CompareError> {
    let paths: BTreeSet<&Vec<u8>> = before.nodes.keys().chain(after.nodes.keys()).collect();
    let mut entries: Vec<FileEntry> = Vec::new();
    for path in paths {
        if Instant::now() >= deadline {
            return Err(CompareError::TimedOut);
        }
        let id = file_id(path);
        if let Want::One(wanted) = want
            && *wanted != id
        {
            continue;
        }
        let Some(diff) = classify(before, after, path, &id) else {
            continue;
        };
        if entries.len() >= MAX_ENTRIES {
            return Err(CompareError::TooMany);
        }
        entries.push(diff.entry.clone());
        sink(diff);
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.file_id.cmp(&b.file_id)));
    let totals = Totals {
        files_changed: entries.len() as u64,
        lines_added: entries
            .iter()
            .filter(|e| e.shown)
            .filter_map(|e| e.lines_added)
            .sum(),
        lines_removed: entries
            .iter()
            .filter(|e| e.shown)
            .filter_map(|e| e.lines_removed)
            .sum(),
        unshown: entries.iter().filter(|e| !e.shown).count() as u64,
    };
    Ok((entries, totals))
}

fn node_kind(n: &Node) -> Kind {
    match n {
        Node::File { .. } => Kind::Text,
        Node::Symlink { .. } => Kind::Symlink,
        Node::EmptyDir | Node::TooDeep | Node::Special => Kind::Directory,
    }
}

fn size_of(n: &Option<&Node>) -> Option<u64> {
    match n {
        Some(Node::File { size }) => Some(*size),
        _ => None,
    }
}

fn base_entry(
    path: &[u8],
    id: &str,
    status: Status,
    kind: Kind,
    b: Option<&Node>,
    a: Option<&Node>,
) -> FileEntry {
    let (shown_path, escaped) = display_path(path);
    FileEntry {
        file_id: id.to_owned(),
        path: shown_path,
        path_escaped: escaped,
        status,
        kind,
        size_before: size_of(&b),
        size_after: size_of(&a),
        lines_added: None,
        lines_removed: None,
        shown: false,
        not_shown_because: None,
    }
}

fn unshown(mut e: FileEntry, why: NotShown) -> FileDiff {
    e.shown = false;
    e.not_shown_because = Some(why);
    FileDiff {
        entry: e,
        hunks: Vec::new(),
        truncated: None,
        approximate: false,
    }
}

/// One side of a path, read as far as a diff needs.
enum Side {
    Absent,
    Text(String),
    /// A symbolic link's target bytes.
    Link(Vec<u8>),
    /// Something with a reason instead of text.
    Other(Kind, NotShown),
}

fn read_side(tree: &Tree, path: &[u8], node: Option<&Node>) -> Side {
    match node {
        None => Side::Absent,
        Some(Node::File { size }) => match tree.load(path, *size) {
            Content::Text(s) => Side::Text(s),
            Content::Binary => Side::Other(Kind::Binary, NotShown::Binary),
            Content::NotUtf8 => Side::Other(Kind::Binary, NotShown::NotUtf8),
            Content::TooLarge => Side::Other(Kind::Text, NotShown::TooLarge),
            Content::Unreadable => Side::Other(Kind::Binary, NotShown::Unreadable),
        },
        Some(Node::Symlink { target }) => Side::Link(target.clone()),
        Some(Node::EmptyDir) => Side::Other(Kind::Directory, NotShown::Directory),
        Some(Node::TooDeep) => Side::Other(Kind::Directory, NotShown::Unreadable),
        Some(Node::Special) => Side::Other(Kind::Binary, NotShown::Unreadable),
    }
}

fn classify(before: &Tree, after: &Tree, path: &[u8], id: &str) -> Option<FileDiff> {
    let (b, a) = (before.nodes.get(path), after.nodes.get(path));
    match (b, a) {
        (None, None) => None,
        (Some(bn), Some(an)) => classify_both(before, after, path, id, bn, an),
        (None, Some(an)) => Some(one_sided(after, path, id, an, Status::Added)),
        (Some(bn), None) => Some(one_sided(before, path, id, bn, Status::Deleted)),
    }
}

fn one_sided(tree: &Tree, path: &[u8], id: &str, node: &Node, status: Status) -> FileDiff {
    let added = status == Status::Added;
    let (bn, an) = if added {
        (None, Some(node))
    } else {
        (Some(node), None)
    };
    let side = read_side(tree, path, Some(node));
    let mut e = base_entry(path, id, status, node_kind(node), bn, an);
    match side {
        Side::Text(text) => {
            let lines = split_lines(&text);
            let d = if added {
                diff_lines(&[], &lines, &display_line)
            } else {
                diff_lines(&lines, &[], &display_line)
            };
            finish_text(e, d)
        }
        Side::Link(target) => {
            e.kind = Kind::Symlink;
            let line = display_line_bytes(&target);
            e.shown = true;
            e.lines_added = None;
            let (op, old_n, new_n) = if added {
                (DiffOp::Add, 0, 1)
            } else {
                (DiffOp::Remove, 1, 0)
            };
            FileDiff {
                entry: e,
                hunks: vec![DiffHunk {
                    old_start: u64::from(!added),
                    old_lines: old_n,
                    new_start: u64::from(added),
                    new_lines: new_n,
                    lines: vec![DiffLine { op, text: line }],
                }],
                truncated: None,
                approximate: false,
            }
        }
        Side::Other(kind, why) => {
            e.kind = kind;
            unshown(e, why)
        }
        Side::Absent => unshown(e, NotShown::Unreadable),
    }
}

fn classify_both(
    before: &Tree,
    after: &Tree,
    path: &[u8],
    id: &str,
    bn: &Node,
    an: &Node,
) -> Option<FileDiff> {
    match (bn, an) {
        (Node::File { size: sb }, Node::File { size: sa }) => {
            if sb == sa && same_content(before, after, path, *sa) {
                return None;
            }
            let (sbf, saf) = (
                read_side(before, path, Some(bn)),
                read_side(after, path, Some(an)),
            );
            let mut e = base_entry(path, id, Status::Modified, Kind::Text, Some(bn), Some(an));
            match (sbf, saf) {
                (Side::Text(old), Side::Text(new)) => {
                    let (o, n) = (split_lines(&old), split_lines(&new));
                    Some(finish_text(e, diff_lines(&o, &n, &display_line)))
                }
                (x, y) => {
                    let reason = [&x, &y]
                        .iter()
                        .filter_map(|s| match s {
                            Side::Other(_, why) => Some(*why),
                            _ => None,
                        })
                        .min_by_key(|w| match w {
                            NotShown::Unreadable => 0,
                            NotShown::Binary => 1,
                            NotShown::NotUtf8 => 2,
                            _ => 3,
                        })
                        .unwrap_or(NotShown::Unreadable);
                    e.kind = match (&y, &x) {
                        (Side::Other(k, _), _) | (_, Side::Other(k, _)) => *k,
                        _ => Kind::Text,
                    };
                    Some(unshown(e, reason))
                }
            }
        }
        (Node::Symlink { target: tb }, Node::Symlink { target: ta }) => {
            if tb == ta {
                return None;
            }
            let mut e = base_entry(
                path,
                id,
                Status::Modified,
                Kind::Symlink,
                Some(bn),
                Some(an),
            );
            e.shown = true;
            Some(FileDiff {
                entry: e,
                hunks: vec![DiffHunk {
                    old_start: 1,
                    old_lines: 1,
                    new_start: 1,
                    new_lines: 1,
                    lines: vec![
                        DiffLine {
                            op: DiffOp::Remove,
                            text: display_line_bytes(tb),
                        },
                        DiffLine {
                            op: DiffOp::Add,
                            text: display_line_bytes(ta),
                        },
                    ],
                }],
                truncated: None,
                approximate: false,
            })
        }
        (Node::EmptyDir, Node::EmptyDir)
        | (Node::Special, Node::Special)
        | (Node::TooDeep, Node::TooDeep) => None,
        _ => {
            // what the path is changed
            let e = base_entry(
                path,
                id,
                Status::TypeChanged,
                node_kind(an),
                Some(bn),
                Some(an),
            );
            let why = if matches!(bn, Node::Symlink { .. }) || matches!(an, Node::Symlink { .. }) {
                NotShown::Symlink
            } else if matches!(bn, Node::Special | Node::TooDeep)
                || matches!(an, Node::Special | Node::TooDeep)
            {
                NotShown::Unreadable
            } else {
                NotShown::Directory
            };
            Some(unshown(e, why))
        }
    }
}

fn same_content(before: &Tree, after: &Tree, path: &[u8], size: u64) -> bool {
    if size <= TEXT_MAX {
        match (before.load_bytes(path, size), after.load_bytes(path, size)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    } else {
        match (before.digest(path), after.digest(path)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }
}

fn finish_text(mut e: FileEntry, d: diff::LineDiff) -> FileDiff {
    e.kind = Kind::Text;
    e.shown = true;
    e.lines_added = Some(d.lines_added);
    e.lines_removed = Some(d.lines_removed);
    FileDiff {
        entry: e,
        hunks: d
            .hunks
            .into_iter()
            .map(|h| DiffHunk {
                old_start: h.old_start as u64,
                old_lines: h.old_lines as u64,
                new_start: h.new_start as u64,
                new_lines: h.new_lines as u64,
                lines: h
                    .lines
                    .into_iter()
                    .map(|l| DiffLine {
                        op: match l.op {
                            Op::Context => DiffOp::Context,
                            Op::Add => DiffOp::Add,
                            Op::Remove => DiffOp::Remove,
                        },
                        text: l.text,
                    })
                    .collect(),
            })
            .collect(),
        truncated: (d.lines_not_shown > 0).then_some(Truncated {
            lines_not_shown: d.lines_not_shown,
        }),
        approximate: d.approximate,
    }
}
