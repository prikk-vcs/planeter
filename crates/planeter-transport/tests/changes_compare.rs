//! The tree walk and the comparison (RFC 005 part 6) over trees built by hand in a temporary directory: every hostile shape of the
//! handoff, each with the property that matters (never followed, never opened, bounded, escaped).

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use planeter_transport::changes::compare::{CompareError, Want, compare, file_id};
use planeter_transport::changes::model::{
    DiffOp, FileDiff, FileEntry, Kind, NotShown, Status, Totals,
};
use planeter_transport::changes::tree::{MAX_DEPTH, MAX_ENTRIES, Node, TEXT_MAX, Tree, WalkError};

fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("changes-compare-{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn put(root: &Path, rel: &str, body: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

struct Out {
    entries: Vec<FileEntry>,
    totals: Totals,
    diffs: Vec<FileDiff>,
}

fn run(before: &Path, after: &Path) -> Out {
    let (b, a) = (Tree::walk(before).unwrap(), Tree::walk(after).unwrap());
    let mut diffs = Vec::new();
    let (entries, totals) = compare(
        &b,
        &a,
        &Want::All,
        Instant::now() + Duration::from_secs(30),
        &mut |d| diffs.push(d),
    )
    .unwrap();
    Out {
        entries,
        totals,
        diffs,
    }
}

fn entry<'a>(o: &'a Out, path: &str) -> &'a FileEntry {
    o.entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| {
            panic!(
                "no entry {path}: {:?}",
                o.entries.iter().map(|e| &e.path).collect::<Vec<_>>()
            )
        })
}

fn diff_of<'a>(o: &'a Out, path: &str) -> &'a FileDiff {
    o.diffs.iter().find(|d| d.entry.path == path).unwrap()
}

#[test]
fn an_edit_a_delete_an_add_and_an_unchanged_file_and_a_rename_as_delete_plus_add() {
    let s = scratch("basic");
    let (b, a) = (s.join("b"), s.join("a"));
    put(&b, "keep.txt", b"same\n");
    put(&b, "edit.txt", b"one\ntwo\nthree\n");
    put(&b, "gone.txt", b"bye\n");
    put(&b, "old-name.txt", b"moved content\n");
    put(&a, "keep.txt", b"same\n");
    put(&a, "edit.txt", b"one\n2\nthree\n");
    put(&a, "new.txt", b"hello\nworld\n");
    put(&a, "new-name.txt", b"moved content\n");
    let o = run(&b, &a);
    let by: Vec<(&str, Status)> = o
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.status))
        .collect();
    assert_eq!(
        by,
        vec![
            ("edit.txt", Status::Modified),
            ("gone.txt", Status::Deleted),
            ("new-name.txt", Status::Added),
            ("new.txt", Status::Added),
            ("old-name.txt", Status::Deleted),
        ],
        "ordered by display path; keep.txt is dropped; a rename is a delete plus an add"
    );
    let e = entry(&o, "edit.txt");
    assert_eq!(
        (e.lines_added, e.lines_removed, e.shown, e.kind),
        (Some(1), Some(1), true, Kind::Text)
    );
    assert_eq!(e.file_id, file_id(b"edit.txt"));
    assert_eq!(
        o.totals,
        Totals {
            files_changed: 5,
            lines_added: 4,
            lines_removed: 3,
            unshown: 0
        }
    );
    let add = diff_of(&o, "new.txt");
    assert_eq!(
        add.hunks[0].lines.iter().map(|l| l.op).collect::<Vec<_>>(),
        vec![DiffOp::Add, DiffOp::Add]
    );
}

#[test]
fn a_side_over_256_kib_a_binary_file_and_a_non_utf8_file_have_a_reason_instead_of_a_diff() {
    let s = scratch("reasons");
    let (b, a) = (s.join("b"), s.join("a"));
    let big_old = "x line of text\n".repeat(20_000); // ~300 KiB
    let big_new = big_old.replacen("x line", "y line", 1);
    assert!(big_old.len() as u64 > TEXT_MAX);
    put(&b, "big.txt", big_old.as_bytes());
    put(&a, "big.txt", big_new.as_bytes());
    put(&b, "bin.dat", b"\x00\x01\x02 binary");
    put(&a, "bin.dat", b"\x00\x01\x03 binary");
    // Shift_JIS "テスト"
    put(&b, "sjis.txt", b"\x83e\x83X\x83g\n");
    put(&a, "sjis.txt", b"\x83e\x83X\x83g\n\x83e\n");
    // a one-line 5 MB text file, unchanged vs changed by one byte
    let line = "z".repeat(5 * 1024 * 1024);
    put(&b, "oneline.txt", line.as_bytes());
    put(&a, "oneline.txt", line.replacen('z', "y", 1).as_bytes());
    put(&b, "same-big.txt", line.as_bytes());
    put(&a, "same-big.txt", line.as_bytes());
    let o = run(&b, &a);
    for (p, why, kind) in [
        ("big.txt", NotShown::TooLarge, Kind::Text),
        ("bin.dat", NotShown::Binary, Kind::Binary),
        ("sjis.txt", NotShown::NotUtf8, Kind::Binary),
        ("oneline.txt", NotShown::TooLarge, Kind::Text),
    ] {
        let e = entry(&o, p);
        assert!(!e.shown, "{p}");
        assert_eq!(e.not_shown_because, Some(why), "{p}");
        assert_eq!(e.kind, kind, "{p}");
        assert_eq!(e.status, Status::Modified);
        assert!(diff_of(&o, p).hunks.is_empty());
    }
    assert!(
        o.entries.iter().all(|e| e.path != "same-big.txt"),
        "a big file with the same bytes is unchanged (streamed hash)"
    );
    assert_eq!(o.totals.unshown, 4);
    assert_eq!(entry(&o, "big.txt").size_before, Some(big_old.len() as u64));
}

#[test]
fn hostile_names_are_escaped_and_the_url_id_is_the_hash_of_the_exact_bytes() {
    let s = scratch("names");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&a).unwrap();
    let names: Vec<Vec<u8>> = vec![
        b"line\nbreak.txt".to_vec(),
        "fdp.\u{202E}txt".as_bytes().to_vec(),
        b"bad-\xFF-byte.txt".to_vec(),
        b"<script>alert(1)<!--.txt".to_vec(),
        "zero\u{200B}width.txt".as_bytes().to_vec(),
    ];
    for n in &names {
        let p = a.join(std::ffi::OsStr::from_bytes(n));
        fs::write(p, b"content\n").unwrap();
    }
    let o = run(&b, &a);
    assert_eq!(o.entries.len(), names.len());
    let by_id = |n: &[u8]| o.entries.iter().find(|e| e.file_id == file_id(n)).unwrap();
    assert_eq!(by_id(&names[0]).path, "line\\u{A}break.txt");
    assert_eq!(by_id(&names[1]).path, "fdp.\\u{202E}txt");
    assert_eq!(by_id(&names[2]).path, "bad-\\xFF-byte.txt");
    assert_eq!(by_id(&names[3]).path, "<script>alert(1)<!--.txt");
    assert!(
        !by_id(&names[3]).path_escaped,
        "html is the page's to escape, not a disguise"
    );
    assert_eq!(by_id(&names[4]).path, "zero\\u{200B}width.txt");
    for e in &o.entries {
        assert!(
            !e.path
                .chars()
                .any(|c| c == '\n' || c == '\u{202E}' || c == '\u{200B}'),
            "{:?}",
            e.path
        );
    }
    assert!(by_id(&names[0]).path_escaped && by_id(&names[2]).path_escaped);
}

#[test]
fn a_symlink_shows_its_target_as_text_and_is_never_followed() {
    let s = scratch("symlink");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&a).unwrap();
    symlink("/etc/passwd", a.join("passwd-link")).unwrap();
    let o = run(&b, &a);
    let e = entry(&o, "passwd-link");
    assert_eq!(
        (e.kind, e.status, e.shown),
        (Kind::Symlink, Status::Added, true)
    );
    assert_eq!(e.size_after, None, "the target's size is not read");
    let d = diff_of(&o, "passwd-link");
    assert_eq!(d.hunks[0].lines.len(), 1);
    assert_eq!(d.hunks[0].lines[0].text, "/etc/passwd");
    assert!(
        !d.hunks[0].lines[0].text.contains("root:"),
        "the content of /etc/passwd is not read"
    );
}

#[test]
fn a_link_that_changes_target_a_file_that_becomes_a_link_and_a_link_to_a_directory() {
    let s = scratch("linkchange");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&a).unwrap();
    symlink("old-target", b.join("l")).unwrap();
    symlink("new-target\u{202E}", a.join("l")).unwrap();
    put(&b, "f", b"was a file\n");
    symlink("/etc/shadow", a.join("f")).unwrap();
    fs::create_dir_all(a.join("realdir")).unwrap();
    put(&a, "realdir/inner.txt", b"x\n");
    symlink("realdir", a.join("dirlink")).unwrap();
    let o = run(&b, &a);
    let l = entry(&o, "l");
    assert_eq!(
        (l.status, l.kind, l.shown),
        (Status::Modified, Kind::Symlink, true)
    );
    let lines: Vec<(&DiffOp, &str)> = diff_of(&o, "l").hunks[0]
        .lines
        .iter()
        .map(|x| (&x.op, x.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        vec![
            (&DiffOp::Remove, "old-target"),
            (&DiffOp::Add, "new-target\\u{202E}")
        ]
    );
    let f = entry(&o, "f");
    assert_eq!(
        (f.status, f.kind, f.not_shown_because),
        (Status::TypeChanged, Kind::Symlink, Some(NotShown::Symlink))
    );
    let d = entry(&o, "dirlink");
    assert_eq!(
        (d.status, d.kind),
        (Status::Added, Kind::Symlink),
        "a link to a directory is a link, not the directory"
    );
    assert!(o.entries.iter().any(|e| e.path == "realdir/inner.txt"));
    assert!(
        !o.entries.iter().any(|e| e.path.starts_with("dirlink/")),
        "the link is not entered"
    );
}

#[test]
fn a_fifo_is_never_opened_and_the_call_returns() {
    let s = scratch("fifo");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&a).unwrap();
    let st = Command::new("mkfifo").arg(a.join("pipe")).status();
    if !st.map(|s| s.success()).unwrap_or(false) {
        eprintln!("SKIPPED: mkfifo is not available");
        return;
    }
    // and the same name as a regular file before: the fifo must not be opened to compare it
    put(&b, "pipe", b"was a file\n");
    let t = Instant::now();
    let o = run(&b, &a);
    assert!(t.elapsed() < Duration::from_secs(5), "the call returned");
    let e = entry(&o, "pipe");
    assert_eq!(
        (e.status, e.not_shown_because, e.shown),
        (Status::TypeChanged, Some(NotShown::Unreadable), false)
    );
    // a fifo on one side only
    let s2 = scratch("fifo2");
    let (b2, a2) = (s2.join("b"), s2.join("a"));
    fs::create_dir_all(&b2).unwrap();
    fs::create_dir_all(&a2).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(a2.join("p2"))
            .status()
            .unwrap()
            .success()
    );
    let o2 = run(&b2, &a2);
    let e2 = entry(&o2, "p2");
    assert_eq!(
        (e2.status, e2.not_shown_because),
        (Status::Added, Some(NotShown::Unreadable))
    );
}

#[test]
fn symlink_loops_are_listed_as_links_and_the_walk_ends() {
    let s = scratch("loop");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(a.join("d")).unwrap();
    symlink("b", a.join("a-link")).unwrap();
    symlink("a-link", a.join("b")).unwrap();
    symlink("..", a.join("d/up")).unwrap();
    symlink(".", a.join("self")).unwrap();
    let t = Instant::now();
    let o = run(&b, &a);
    assert!(t.elapsed() < Duration::from_secs(5));
    for p in ["a-link", "b", "d/up", "self"] {
        assert_eq!(entry(&o, p).kind, Kind::Symlink, "{p}");
    }
    assert_eq!(o.entries.len(), 4);
}

#[test]
fn a_hundred_level_directory_is_cut_at_the_depth_cap_and_says_so() {
    let s = scratch("deep");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    let mut deep = a.clone();
    for i in 0..100 {
        deep = deep.join(format!("d{i}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("bottom.txt"), "x\n").unwrap();
    let tree = Tree::walk(&a).unwrap();
    assert!(tree.nodes.values().any(|n| *n == Node::TooDeep));
    assert!(
        tree.nodes
            .keys()
            .all(|k| k.iter().filter(|&&c| c == b'/').count() <= MAX_DEPTH),
        "no path below the cap"
    );
    let o = run(&b, &a);
    assert_eq!(o.entries.len(), 1);
    let e = &o.entries[0];
    assert_eq!(
        (e.kind, e.not_shown_because, e.status),
        (Kind::Directory, Some(NotShown::Unreadable), Status::Added)
    );
}

#[test]
fn an_empty_directory_that_appears_or_disappears_is_listed_and_a_directory_with_files_is_not() {
    let s = scratch("emptydir");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(b.join("gone-empty")).unwrap();
    fs::create_dir_all(a.join("new-empty")).unwrap();
    put(&a, "dir-with-file/f.txt", b"x\n");
    let o = run(&b, &a);
    let paths: Vec<&str> = o.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["dir-with-file/f.txt", "gone-empty", "new-empty"]
    );
    assert_eq!(entry(&o, "new-empty").kind, Kind::Directory);
    assert_eq!(entry(&o, "gone-empty").status, Status::Deleted);
}

#[test]
fn more_than_ten_thousand_files_in_a_tree_is_too_many_and_so_are_more_than_ten_thousand_changes() {
    let s = scratch("many");
    let big = s.join("big");
    fs::create_dir_all(&big).unwrap();
    for i in 0..(MAX_ENTRIES + 1) {
        fs::write(big.join(format!("f{i}")), "").unwrap();
    }
    assert!(matches!(Tree::walk(&big), Err(WalkError::TooMany)));
    // exactly the bound is fine for the walk
    fs::remove_file(big.join("f0")).unwrap();
    assert_eq!(Tree::walk(&big).unwrap().nodes.len(), MAX_ENTRIES);
    // the comparison: 10,000 added files are fine, one more is TooMany
    let empty = s.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let (b, a) = (Tree::walk(&empty).unwrap(), Tree::walk(&big).unwrap());
    let ok = compare(
        &b,
        &a,
        &Want::All,
        Instant::now() + Duration::from_secs(60),
        &mut |_| {},
    );
    assert_eq!(ok.unwrap().1.files_changed, MAX_ENTRIES as u64);
    fs::write(big.join("one-more"), "").unwrap();
    let a2 = Tree::walk(&big);
    assert!(matches!(a2, Err(WalkError::TooMany)));
}

#[test]
fn a_file_of_six_thousand_changed_lines_is_cut_at_the_cap_and_the_rest_is_counted() {
    let s = scratch("cap");
    let (b, a) = (s.join("b"), s.join("a"));
    put(&b, "f.txt", b"");
    let body: String = (0..6000).map(|i| format!("l{i}\n")).collect();
    put(&a, "f.txt", body.as_bytes());
    let o = run(&b, &a);
    let d = diff_of(&o, "f.txt");
    assert_eq!(d.entry.lines_added, Some(6000));
    assert_eq!(d.hunks.iter().map(|h| h.lines.len()).sum::<usize>(), 5000);
    assert_eq!(d.truncated.as_ref().unwrap().lines_not_shown, 1000);
    assert_eq!(o.totals.lines_added, 6000);
}

#[test]
fn crlf_files_show_their_carriage_returns_and_a_crlf_to_lf_change_is_every_line() {
    let s = scratch("crlf");
    let (b, a) = (s.join("b"), s.join("a"));
    put(&b, "f.txt", b"one\r\ntwo\r\nthree\r\n");
    put(&a, "f.txt", b"one\ntwo\nthree\n");
    let o = run(&b, &a);
    let d = diff_of(&o, "f.txt");
    assert_eq!(
        (d.entry.lines_added, d.entry.lines_removed),
        (Some(3), Some(3))
    );
    let removed: Vec<&str> = d.hunks[0]
        .lines
        .iter()
        .filter(|l| l.op == DiffOp::Remove)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(removed, vec!["one\u{240D}", "two\u{240D}", "three\u{240D}"]);
}

#[test]
fn text_with_bidi_controls_and_escape_sequences_is_escaped_in_every_diff_line() {
    let s = scratch("textescape");
    let (b, a) = (s.join("b"), s.join("a"));
    put(&b, "f.txt", b"safe\n");
    put(
        &a,
        "f.txt",
        "evil \u{202E}reversed\n\x1b[31mred\n".as_bytes(),
    );
    let o = run(&b, &a);
    for l in diff_of(&o, "f.txt").hunks.iter().flat_map(|h| &h.lines) {
        assert!(
            !l.text.contains('\u{202E}') && !l.text.contains('\u{1b}'),
            "{:?}",
            l.text
        );
    }
}

#[test]
fn want_one_computes_only_that_file_and_a_passed_deadline_is_timed_out() {
    let s = scratch("one");
    let (b, a) = (s.join("b"), s.join("a"));
    put(&b, "x.txt", b"1\n");
    put(&a, "x.txt", b"2\n");
    put(&a, "y.txt", b"y\n");
    let (bt, at) = (Tree::walk(&b).unwrap(), Tree::walk(&a).unwrap());
    let id = file_id(b"y.txt");
    let mut got = Vec::new();
    let (entries, _) = compare(
        &bt,
        &at,
        &Want::One(&id),
        Instant::now() + Duration::from_secs(5),
        &mut |d| got.push(d),
    )
    .unwrap();
    assert_eq!((entries.len(), got.len()), (1, 1));
    assert_eq!(entries[0].path, "y.txt");
    let late = compare(
        &bt,
        &at,
        &Want::All,
        Instant::now() - Duration::from_secs(1),
        &mut |_| {},
    );
    assert_eq!(late.unwrap_err(), CompareError::TimedOut);
}

#[test]
fn the_empty_tree_is_a_valid_before_side() {
    let s = scratch("emptybefore");
    let a = s.join("a");
    put(&a, "README", b"hello\n");
    let at = Tree::walk(&a).unwrap();
    let (entries, totals) = compare(
        &Tree::empty(),
        &at,
        &Want::All,
        Instant::now() + Duration::from_secs(5),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!((entries.len(), totals.lines_added), (1, 1));
    assert_eq!(entries[0].status, Status::Added);
}

#[test]
fn two_trees_that_are_each_within_the_bound_can_still_change_more_files_than_the_bound() {
    let s = scratch("manychanges");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&a).unwrap();
    for i in 0..MAX_ENTRIES {
        fs::write(b.join(format!("gone{i}")), "").unwrap();
        fs::write(a.join(format!("new{i}")), "").unwrap();
    }
    let (bt, at) = (Tree::walk(&b).unwrap(), Tree::walk(&a).unwrap());
    assert_eq!((bt.nodes.len(), at.nodes.len()), (MAX_ENTRIES, MAX_ENTRIES));
    let r = compare(
        &bt,
        &at,
        &Want::All,
        Instant::now() + Duration::from_secs(120),
        &mut |_| {},
    );
    assert_eq!(
        r.unwrap_err(),
        CompareError::TooMany,
        "20,000 changed files: past the bound"
    );
}

#[test]
fn a_long_line_is_its_head_with_cut_chars_and_a_line_that_looks_like_the_old_marker_is_just_text() {
    let s = scratch("cut-line");
    let (b, a) = (s.join("b"), s.join("a"));
    put(&b, "f.txt", b"keep\n");
    let long = "z".repeat(2500);
    let looks = "see \u{2026}(500 more characters)";
    put(
        &a,
        "f.txt",
        format!("keep\n{long}\n{looks}\nand an ordinary line\n").as_bytes(),
    );
    let o = run(&b, &a);
    let d = diff_of(&o, "f.txt");
    let lines: Vec<_> = d.hunks.iter().flat_map(|h| h.lines.iter()).collect();
    let cut = lines.iter().find(|l| l.text.starts_with("zzz")).unwrap();
    assert_eq!(cut.text, "z".repeat(2000), "the head, and nothing else");
    assert_eq!(cut.cut_chars, 500);
    let same = lines.iter().find(|l| l.text == looks).unwrap();
    assert_eq!(
        same.cut_chars, 0,
        "text that ends like the old marker is itself"
    );
    // the wire form: cut_chars only where there is one
    let json = serde_json::to_string(d).unwrap();
    assert_eq!(json.matches("cut_chars").count(), 1, "{json}");
    assert!(
        !json.contains(&format!("{}\u{2026}", "z".repeat(2000))),
        "no marker is written after the head"
    );
    assert!(!json.contains("path_cut_bytes"), "omitted when 0");
}

#[test]
fn a_name_over_the_limit_is_its_head_with_path_cut_bytes_and_the_id_is_of_the_whole_name() {
    let s = scratch("cut-name");
    let (b, a) = (s.join("b"), s.join("a"));
    fs::create_dir_all(&b).unwrap();
    // six directories of 200 bytes each: a path of more than 1,200 bytes, each part within the file system's limit
    let dir = "d".repeat(200);
    let rel = format!("{dir}/{dir}/{dir}/{dir}/{dir}/{dir}/leaf.txt");
    put(&a, &rel, b"x\n");
    let o = run(&b, &a);
    assert_eq!(o.entries.len(), 1);
    let e = &o.entries[0];
    assert_eq!(e.path.len(), 1024, "the head of the name");
    assert!(!e.path.contains("more bytes"));
    assert_eq!(e.path_cut_bytes as usize, rel.len() - 1024);
    assert!(e.path_escaped);
    assert_eq!(
        e.file_id,
        file_id(rel.as_bytes()),
        "the id is the hash of the whole name"
    );
    let json = serde_json::to_string(e).unwrap();
    assert!(json.contains("\"path_cut_bytes\""), "{json}");
}
