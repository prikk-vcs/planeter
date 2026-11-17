//! A bounded line diff (Myers, O(ND)) with a hard cap on the edit distance explored. **No dependency, no clock**: the same input gives
//! the same output on every machine, which is what lets a result be cached for ever under a content-addressed key.
//!
//! When the two sides differ by more than [`MAX_EDIT_DISTANCE`] lines (after the common beginning and end are set aside), the diff is the
//! **whole-file replacement** (every old line removed, every new line added) and says `approximate`. The output is split into hunks with
//! [`CONTEXT`] lines of context, and at most [`MAX_DIFF_LINES`] lines of hunks are returned per file; the totals count every line.

/// Lines of context around a change.
pub const CONTEXT: usize = 3;
/// At most this many diff lines (context included) are returned per file.
pub const MAX_DIFF_LINES: usize = 5_000;
/// The most edits (lines removed plus lines added, after trimming the common beginning and end) explored before the replacement is
/// returned instead. The work is at most `(old + new) × MAX_EDIT_DISTANCE` steps and the memory about `MAX_EDIT_DISTANCE²` small integers.
pub const MAX_EDIT_DISTANCE: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub op: Op,
    pub text: String,
    /// Characters of the line left out of `text` (0: none).
    pub cut_chars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<Line>,
}

/// The result of comparing two lists of lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDiff {
    pub hunks: Vec<Hunk>,
    /// Every line added / removed (counted over the whole diff, not only the lines returned).
    pub lines_added: u64,
    pub lines_removed: u64,
    /// How many diff lines were left out because of [`MAX_DIFF_LINES`].
    pub lines_not_shown: u64,
    /// The edit distance cap was exceeded and the diff is the whole-file replacement.
    pub approximate: bool,
}

/// Split text into lines on `\n`. A final line without a terminator counts as a line; a terminator at the very end does not start another.
pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// An edit script as `(kind, old index, new index)` for each line of the result, in order: `kind` 0 = both (context), 1 = removed, 2 = added.
type Script = Vec<(u8, usize, usize)>;

/// Compare two lists of lines. `display` turns a line into the text that is returned (escaping, cutting).
pub fn diff_lines(old: &[&str], new: &[&str], display: &dyn Fn(&str) -> (String, u64)) -> LineDiff {
    let (script, approximate) = edit_script(old, new);
    build(old, new, &script, approximate, display)
}

fn edit_script(old: &[&str], new: &[&str]) -> (Script, bool) {
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let (a, b) = (
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
    );
    let mut script: Script = (0..prefix).map(|i| (0, i, i)).collect();
    let (middle, approximate) = match myers(a, b) {
        Some(m) => (m, false),
        None => {
            // the whole-file replacement: every old line removed (new position 0), then every new line added (old position after them)
            let mut all: Script = (0..a.len()).map(|i| (1, i, 0)).collect();
            all.extend((0..b.len()).map(|j| (2, a.len(), j)));
            (all, true)
        }
    };
    script.extend(
        middle
            .into_iter()
            .map(|(k, i, j)| (k, prefix + i, prefix + j)),
    );
    for s in 0..suffix {
        script.push((0, old.len() - suffix + s, new.len() - suffix + s));
    }
    (script, approximate)
}

/// Myers' greedy forward algorithm, the trace kept for the walk back. `None` when the distance exceeds [`MAX_EDIT_DISTANCE`].
/// The result is `(kind, old index, new index)`: for a removal the new index is the position of the next new line, for an addition the old
/// index is the position of the next old line.
fn myers(a: &[&str], b: &[&str]) -> Option<Script> {
    let (n, m) = (a.len() as i64, b.len() as i64);
    if n == 0 || m == 0 {
        // nothing in common is possible: all removed or all added, exactly (this is not an approximation)
        let mut all: Script = (0..a.len()).map(|i| (1, i, 0)).collect();
        all.extend((0..b.len()).map(|j| (2, 0, j)));
        return Some(all);
    }
    let max = (n + m).min(MAX_EDIT_DISTANCE as i64);
    let offset = max + 1;
    let mut v = vec![0i64; (2 * max + 3) as usize];
    let at = |k: i64| (k + offset) as usize;
    // the snapshot of v at the start of round d, for k in -(d+1)..=(d+1)
    let mut trace: Vec<Vec<i64>> = Vec::new();
    let mut last = None;
    'rounds: for d in 0..=max {
        trace.push(v[at(-(d + 1))..=at(d + 1)].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)]
            } else {
                v[at(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                last = Some(d);
                break 'rounds;
            }
            k += 2;
        }
    }
    let d_final = last?;
    let mut out: Script = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..=d_final).rev() {
        let snap = &trace[d as usize];
        // snap[0] is k = -(d+1)
        let get = |k: i64| snap[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && get(k - 1) < get(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = get(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            out.push((0, x as usize, y as usize));
        }
        if d > 0 {
            if x == prev_x {
                // a step down: b[prev_y] was added
                out.push((2, x as usize, prev_y as usize));
            } else {
                // a step right: a[prev_x] was removed
                out.push((1, prev_x as usize, y as usize));
            }
        }
        x = prev_x;
        y = prev_y;
    }
    out.reverse();
    Some(out)
}

fn build(
    old: &[&str],
    new: &[&str],
    script: &Script,
    approximate: bool,
    display: &dyn Fn(&str) -> (String, u64),
) -> LineDiff {
    let lines_added = script.iter().filter(|s| s.0 == 2).count() as u64;
    let lines_removed = script.iter().filter(|s| s.0 == 1).count() as u64;
    // positions (in the script) of the changes, grouped with CONTEXT lines around them
    let changed: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, s)| s.0 != 0)
        .map(|(i, _)| i)
        .collect();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut budget = MAX_DIFF_LINES;
    let mut not_shown: u64 = 0;
    let mut g = 0;
    while g < changed.len() {
        let mut end = g;
        while end + 1 < changed.len() && changed[end + 1] - changed[end] <= 2 * CONTEXT + 1 {
            end += 1;
        }
        let from = changed[g].saturating_sub(CONTEXT);
        let to = (changed[end] + CONTEXT).min(script.len() - 1);
        let slice = &script[from..=to];
        let mut lines = Vec::with_capacity(slice.len());
        let first = slice[0];
        for &(k, i, j) in slice {
            if budget == 0 {
                not_shown += 1;
                continue;
            }
            budget -= 1;
            let (op, text) = match k {
                0 => (Op::Context, old[i]),
                1 => (Op::Remove, old[i]),
                _ => (Op::Add, new[j]),
            };
            let (text, cut_chars) = display(text);
            lines.push(Line {
                op,
                text,
                cut_chars,
            });
        }
        if !lines.is_empty() {
            // the counts and starts are those of the lines returned (a hunk cut by the cap is shorter)
            let old_lines = lines.iter().filter(|l| l.op != Op::Add).count();
            let new_lines = lines.iter().filter(|l| l.op != Op::Remove).count();
            // in a unified diff the start of a side with no lines is the line before the hunk
            let old_start = if old_lines == 0 { first.1 } else { first.1 + 1 };
            let new_start = if new_lines == 0 { first.2 } else { first.2 + 1 };
            hunks.push(Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines,
            });
        }
        g = end + 1;
    }
    LineDiff {
        hunks,
        lines_added,
        lines_removed,
        lines_not_shown: not_shown,
        approximate,
    }
}
