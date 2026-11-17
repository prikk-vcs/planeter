//! The bounded line diff (RFC 005 part 6): correctness against a reconstruction oracle, the hunks' shape, the caps, and the two worst cases.

use planeter_transport::changes::diff::{
    CONTEXT, Hunk, LineDiff, MAX_DIFF_LINES, MAX_EDIT_DISTANCE, Op, diff_lines, split_lines,
};
use std::time::Instant;

fn d(old: &str, new: &str) -> LineDiff {
    diff_lines(&split_lines(old), &split_lines(new), &|s| (s.to_owned(), 0))
}

/// Rebuild both sides from a diff and the unchanged parts: every hunk's context + removed lines must be the old lines it covers and
/// context + added lines the new ones, at the positions it claims.
fn check_against_oracle(old: &str, new: &str, diff: &LineDiff) {
    let (o, n) = (split_lines(old), split_lines(new));
    let (mut removed, mut added) = (0u64, 0u64);
    for h in &diff.hunks {
        let mut oi = if h.old_lines == 0 {
            h.old_start
        } else {
            h.old_start - 1
        };
        let mut ni = if h.new_lines == 0 {
            h.new_start
        } else {
            h.new_start - 1
        };
        let (mut oc, mut nc) = (0, 0);
        for l in &h.lines {
            match l.op {
                Op::Context => {
                    assert_eq!(l.text, o[oi], "context vs old at {oi}");
                    assert_eq!(l.text, n[ni], "context vs new at {ni}");
                    oi += 1;
                    ni += 1;
                    oc += 1;
                    nc += 1;
                }
                Op::Remove => {
                    assert_eq!(l.text, o[oi], "removed vs old at {oi}");
                    oi += 1;
                    oc += 1;
                    removed += 1;
                }
                Op::Add => {
                    assert_eq!(l.text, n[ni], "added vs new at {ni}");
                    ni += 1;
                    nc += 1;
                    added += 1;
                }
            }
        }
        assert_eq!((oc, nc), (h.old_lines, h.new_lines), "hunk counts");
    }
    if diff.lines_not_shown == 0 {
        assert_eq!((removed, added), (diff.lines_removed, diff.lines_added));
    }
    // hunks are ordered and do not overlap
    for w in diff.hunks.windows(2) {
        assert!(
            w[0].old_start + w[0].old_lines <= w[1].old_start.max(1),
            "overlap {w:?}"
        );
    }
}

#[test]
fn identical_files_have_no_hunks_and_no_changes() {
    let x = d("a\nb\nc\n", "a\nb\nc\n");
    assert!(x.hunks.is_empty());
    assert_eq!(
        (x.lines_added, x.lines_removed, x.approximate),
        (0, 0, false)
    );
}

#[test]
fn a_single_edit_is_a_remove_and_an_add_with_three_lines_of_context() {
    let old: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    let new = old.replace("line 10\n", "line ten\n");
    let x = d(&old, &new);
    assert_eq!(x.hunks.len(), 1);
    let h: &Hunk = &x.hunks[0];
    assert_eq!(
        (h.old_start, h.old_lines, h.new_start, h.new_lines),
        (7, 7, 7, 7)
    );
    assert_eq!((x.lines_removed, x.lines_added), (1, 1));
    assert_eq!(
        h.lines.iter().filter(|l| l.op == Op::Context).count(),
        2 * CONTEXT
    );
    check_against_oracle(&old, &new, &x);
}

#[test]
fn two_distant_edits_are_two_hunks_and_two_near_edits_are_one() {
    let old: String = (1..=40).map(|i| format!("l{i}\n")).collect();
    let far = old
        .replace("l5\n", "five\n")
        .replace("l35\n", "thirty-five\n");
    let x = d(&old, &far);
    assert_eq!(x.hunks.len(), 2);
    check_against_oracle(&old, &far, &x);
    let near = old.replace("l5\n", "five\n").replace("l9\n", "nine\n");
    let y = d(&old, &near);
    assert_eq!(y.hunks.len(), 1, "seven lines apart: the contexts touch");
    check_against_oracle(&old, &near, &y);
}

#[test]
fn an_empty_side_is_all_added_or_all_removed() {
    let x = d("", "a\nb\n");
    assert_eq!((x.lines_added, x.lines_removed), (2, 0));
    assert_eq!(
        (
            x.hunks[0].old_start,
            x.hunks[0].old_lines,
            x.hunks[0].new_start,
            x.hunks[0].new_lines
        ),
        (0, 0, 1, 2)
    );
    let y = d("a\nb\n", "");
    assert_eq!((y.lines_added, y.lines_removed), (0, 2));
    assert_eq!(
        (
            y.hunks[0].old_start,
            y.hunks[0].old_lines,
            y.hunks[0].new_start,
            y.hunks[0].new_lines
        ),
        (1, 2, 0, 0)
    );
    assert!(d("", "").hunks.is_empty());
}

#[test]
fn a_missing_final_newline_is_a_line_like_any_other() {
    let y = d("a\nb", "a\nb\nc");
    assert_eq!((y.lines_added, y.lines_removed), (1, 0));
}

#[test]
fn randomized_pairs_rebuild_exactly() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..300 {
        let words = 3 + (next() % 6) as usize;
        let mk = |n: &mut dyn FnMut() -> u64| -> String {
            let len = (n() % 60) as usize;
            (0..len)
                .map(|_| format!("w{}\n", n() % words as u64))
                .collect()
        };
        let old = mk(&mut next);
        let new = mk(&mut next);
        let x = d(&old, &new);
        check_against_oracle(&old, &new, &x);
        // the counts are the true edit script's: removed - added == old len - new len
        assert_eq!(
            x.lines_removed as i64 - x.lines_added as i64,
            split_lines(&old).len() as i64 - split_lines(&new).len() as i64,
            "round {round}"
        );
    }
}

#[test]
fn the_script_is_a_shortest_one_on_a_known_case() {
    // ABCABBA -> CBABAC has an edit distance of 5 (the paper's example)
    let (o, n) = ("A\nB\nC\nA\nB\nB\nA\n", "C\nB\nA\nB\nA\nC\n");
    let x = d(o, n);
    assert_eq!(x.lines_added + x.lines_removed, 5);
    assert!(!x.approximate);
    check_against_oracle(o, n, &x);
}

#[test]
fn more_than_the_cap_of_changed_lines_is_the_whole_file_replacement_and_says_so() {
    let old: String = (0..3000).map(|i| format!("old {i}\n")).collect();
    let new: String = (0..3000).map(|i| format!("new {i}\n")).collect();
    let x = d(&old, &new);
    assert!(x.approximate);
    assert_eq!((x.lines_removed, x.lines_added), (3000, 3000));
    assert!(old.lines().count() * 2 > MAX_EDIT_DISTANCE);
    // shown up to the cap of returned lines
    let shown: usize = x.hunks.iter().map(|h| h.lines.len()).sum();
    assert_eq!(shown, MAX_DIFF_LINES);
    assert_eq!(x.lines_not_shown, 6000 - MAX_DIFF_LINES as u64);
    check_against_oracle(&old, &new, &x);
}

#[test]
fn the_returned_lines_are_capped_and_the_rest_is_counted() {
    let new: String = (0..7000).map(|i| format!("n{i}\n")).collect();
    let x = d("", &new);
    assert!(
        !x.approximate,
        "an addition only is distance 7000 but trivial: it must not need the cap"
    );
    let shown: usize = x.hunks.iter().map(|h| h.lines.len()).sum();
    assert_eq!(shown, MAX_DIFF_LINES);
    assert_eq!(x.lines_not_shown, 2000);
    assert_eq!(x.lines_added, 7000);
}

#[test]
fn worst_case_8000_lines_with_no_common_line_and_every_other_line_changed_finish_well_inside_the_bound()
 {
    // two 256 KiB-class files of 8,000 lines
    let a: String = (0..8000)
        .map(|i| format!("{:<31}\n", format!("old line {i:05} first file")))
        .collect();
    let b: String = (0..8000)
        .map(|i| format!("{:<31}\n", format!("new line {i:05} other file")))
        .collect();
    assert!(
        a.len() <= 256 * 1024 && b.len() <= 256 * 1024,
        "{}",
        a.len()
    );
    let t = Instant::now();
    let x = d(&a, &b);
    let t1 = t.elapsed();
    assert!(x.approximate);
    assert_eq!((x.lines_removed, x.lines_added), (8000, 8000));
    let c: String = (0..8000)
        .map(|i| {
            if i % 2 == 0 {
                format!("{:<31}\n", format!("same line {i:05} both"))
            } else {
                format!("{:<31}\n", format!("left only {i:05}"))
            }
        })
        .collect();
    let e: String = (0..8000)
        .map(|i| {
            if i % 2 == 0 {
                format!("{:<31}\n", format!("same line {i:05} both"))
            } else {
                format!("{:<31}\n", format!("right only {i:05}"))
            }
        })
        .collect();
    let t = Instant::now();
    let y = d(&c, &e);
    let t2 = t.elapsed();
    assert!(
        y.approximate,
        "4000 + 4000 edits are over the cap of {MAX_EDIT_DISTANCE}"
    );
    assert_eq!(
        (y.lines_removed, y.lines_added),
        (7999, 7999),
        "the first line is common and set aside"
    );
    eprintln!("worst case timings: no common line {t1:?}, every other line changed {t2:?}");
    assert!(t1.as_secs() < 5 && t2.as_secs() < 5, "{t1:?} {t2:?}");
}

#[test]
fn a_file_whose_changes_are_just_under_the_cap_is_still_diffed_exactly() {
    let old: String = (0..4000).map(|i| format!("keep {i}\n")).collect();
    // 900 lines changed in the middle: 900 removed + 900 added = 1800 < 2000
    let mut lines: Vec<String> = (0..4000).map(|i| format!("keep {i}\n")).collect();
    for l in lines.iter_mut().skip(1000).take(900) {
        *l = format!("changed {l}");
    }
    let new: String = lines.concat();
    let x = d(&old, &new);
    assert!(!x.approximate);
    assert_eq!((x.lines_removed, x.lines_added), (900, 900));
    check_against_oracle(&old, &new, &x);
}
