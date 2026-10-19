//! RFC 011 D-1 / D-2 / D-3: the cost table, the ceiling, and the store-size estimate.
//!
//! Pure tests plus a few that build a sealed repository with real prikk (self-skip with a printed reason
//! when `prikk` is not on `PATH`). Calibration rows are the investigation's measurements (prikk 0.52.0).

use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_core::admission::{
    CEILING_TENTHS, CONTENT_MAX_BYTES, COST_FLOOR, LIGHT_CLASS_MAX_COST, MAX_STORE_ENTRIES,
    MEASURED_ON, MIB, ReadVerb, SHOW_FLOOR, StoreSizeError, ceiling_for, store_size,
    store_size_limited,
};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("admission-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------- the table

#[test]
fn every_verb_has_a_cost_rule_measured_on_the_current_prikk_floor() {
    // Re-measure the table at each floor change (docs/UPSTREAM.md): this fails until it is.
    for v in ReadVerb::ALL {
        assert_eq!(
            v.rule().measured_on,
            planeter_prikk::cli::MIN_PRIKK_VERSION,
            "{v:?} was measured on {:?}, the floor is {:?}",
            v.rule().measured_on,
            planeter_prikk::cli::MIN_PRIKK_VERSION
        );
    }
    assert_eq!(MEASURED_ON, planeter_prikk::cli::MIN_PRIKK_VERSION);
}

#[test]
fn the_verb_list_is_complete_and_exhaustively_matched() {
    // No wildcard: adding a `ReadVerb` makes this fail to compile until it is placed (and so costed).
    fn slot(v: ReadVerb) -> usize {
        match v {
            ReadVerb::Log => 0,
            ReadVerb::Show => 1,
            ReadVerb::Refs => 2,
            ReadVerb::Tree => 3,
            ReadVerb::Cat => 4,
            ReadVerb::FileContent => 5,
            ReadVerb::Verify => 6,
        }
    }
    for (i, v) in ReadVerb::ALL.iter().enumerate() {
        assert_eq!(slot(*v), i, "ReadVerb::ALL is out of step with the enum");
    }
}

#[test]
fn the_multiples_are_the_worst_case_ones_with_a_floor() {
    let s = 1000 * MIB; // 1000 MiB store
    assert_eq!(ReadVerb::Tree.cost(s), 2100 * MIB);
    assert_eq!(ReadVerb::Cat.cost(s), 2100 * MIB);
    assert_eq!(ReadVerb::Verify.cost(s), 4100 * MIB);
    // `file` = the tree pre-check plus 3.1 x the bounded content.
    assert_eq!(
        ReadVerb::FileContent.cost(s),
        2100 * MIB + CONTENT_MAX_BYTES * 31 / 10
    );
    for v in [ReadVerb::Log, ReadVerb::Refs] {
        assert_eq!(v.cost(s), COST_FLOOR, "{v:?} is flat");
    }
    // `show` follows the patch's content (up to ≈ 3.1 × S for text), so it carries a store multiple and a floor.
    assert_eq!(ReadVerb::Show.cost(s), 3600 * MIB);
    assert_eq!(ReadVerb::Show.cost(0), SHOW_FLOOR);
    // Everything has the floor, even against an empty store.
    for v in ReadVerb::ALL {
        assert!(v.cost(0) >= COST_FLOOR, "{v:?}");
    }
    // No overflow at the extreme.
    assert_eq!(ReadVerb::Verify.cost(u64::MAX), u64::MAX);
}

#[test]
fn the_table_never_under_predicts_a_measured_case() {
    // (fixture, verb, store bytes by the walk, measured peak bytes) — investigation I1 (29 tree/cat/verify/file
    // cases, `file` bounded to its <= 1 MiB form), plus the flat verbs and the patch-plan `file` read.
    let cases: &[(&str, ReadVerb, u64, u64)] = &[
        ("m1", ReadVerb::Show, 1013680, 8962048),
        ("m16", ReadVerb::Show, 15831709, 8966144),
        ("m64", ReadVerb::Show, 66201411, 8753152),
        ("m200", ReadVerb::Show, 208915749, 8982528),
        ("m1000", ReadVerb::Show, 1048411603, 8990720),
        ("hist1000", ReadVerb::Show, 2405152, 8957952),
        ("files10k", ReadVerb::Show, 7389309, 17858560),
        ("txt64", ReadVerb::Show, 67241474, 209870848),
        ("txt200", ReadVerb::Show, 209940730, 642416640),
        ("bin64one", ReadVerb::Show, 67112573, 138719232),
        ("txt64many", ReadVerb::Show, 67154383, 209047552),
        ("bin64one", ReadVerb::Tree, 67112573, 139780096),
        ("bin64one", ReadVerb::Verify, 67112573, 274006016),
        ("files10k", ReadVerb::Tree, 7389309, 16334848),
        ("files10k", ReadVerb::Verify, 7389309, 24944640),
        ("hist1000", ReadVerb::Tree, 2405152, 9805824),
        ("hist1000", ReadVerb::Verify, 2405152, 10997760),
        ("m1", ReadVerb::Cat, 1013680, 9801728),
        ("m1", ReadVerb::Tree, 1013680, 9822208),
        ("m1", ReadVerb::Verify, 1013680, 9809920),
        ("m1000", ReadVerb::Cat, 1048411603, 1055420416),
        ("m1000", ReadVerb::Tree, 1048411603, 1054437376),
        ("m1000", ReadVerb::Verify, 1048411603, 2102853632),
        ("m16", ReadVerb::Cat, 15831709, 23343104),
        ("m16", ReadVerb::Tree, 15831709, 22994944),
        ("m16", ReadVerb::Verify, 15831709, 38678528),
        ("m200", ReadVerb::Cat, 208915749, 216260608),
        ("m200", ReadVerb::Tree, 208915749, 216084480),
        ("m200", ReadVerb::Verify, 208915749, 425172992),
        ("m64", ReadVerb::Cat, 66201411, 73662464),
        ("m64", ReadVerb::Tree, 66201411, 73269248),
        ("m64", ReadVerb::Verify, 66201411, 139849728),
        ("txt200", ReadVerb::Tree, 209940730, 425033728),
        ("txt200", ReadVerb::Verify, 209940730, 845590528),
        ("txt64", ReadVerb::Tree, 67241474, 139759616),
        ("txt64", ReadVerb::Verify, 67241474, 274796544),
        ("txt64many", ReadVerb::Tree, 67154383, 73568256),
        ("txt64many", ReadVerb::Verify, 67154383, 141950976),
        ("m1", ReadVerb::FileContent, 1013680, 8986624),
        ("m16", ReadVerb::FileContent, 15831709, 23068672),
        ("m64", ReadVerb::FileContent, 66201411, 73342976),
        ("m200", ReadVerb::FileContent, 208915749, 216084480),
        ("m1000", ReadVerb::FileContent, 1048411603, 1054076928),
        ("hist1000", ReadVerb::FileContent, 2405152, 8957952),
        ("files10k", ReadVerb::FileContent, 7389309, 16056320),
        ("m1", ReadVerb::Log, 1013680, 9818112),
        ("m1", ReadVerb::Refs, 1013680, 8966144),
        ("m64", ReadVerb::Log, 66201411, 9793536),
        ("m64", ReadVerb::Refs, 66201411, 8966144),
        ("m200", ReadVerb::Log, 208915749, 9793536),
        ("m200", ReadVerb::Refs, 208915749, 8966144),
        ("m1000", ReadVerb::Log, 1048411603, 9826304),
        ("m1000", ReadVerb::Refs, 1048411603, 8966144),
        ("hist1000", ReadVerb::Log, 2405152, 9834496),
        ("hist1000", ReadVerb::Refs, 2405152, 8966144),
        ("files10k", ReadVerb::Log, 7389309, 10194944),
        ("files10k", ReadVerb::Refs, 7389309, 8966144),
    ];
    let mut worst = (0.0_f64, "");
    let mut over = Vec::new();
    for (fx, verb, store, peak) in cases {
        let cost = verb.cost(*store);
        assert!(
            cost >= *peak,
            "{fx} {verb:?}: cost {cost} < measured peak {peak} (the table under-predicts)"
        );
        let ratio = cost as f64 / *peak as f64;
        over.push(ratio);
        if ratio > worst.0 {
            worst = (ratio, fx);
        }
        // And the ceiling (1.5 x the cost) leaves the call headroom over its own measured peak.
        assert!(ceiling_for(cost, None) >= *peak);
    }
    over.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "calibration: {} cases, over-reservation min {:.2} median {:.2} max {:.2} ({})",
        over.len(),
        over[0],
        over[over.len() / 2],
        over[over.len() - 1],
        worst.1
    );
}

#[test]
fn the_ceiling_is_one_and_a_half_times_the_cost_and_never_above_the_budget() {
    assert_eq!(ceiling_for(400 * MIB, None), 600 * MIB);
    assert_eq!(ceiling_for(400 * MIB, Some(4096 * MIB)), 600 * MIB);
    // Near the budget: capped at it, but never below the cost (which the budget check guarantees fits).
    assert_eq!(ceiling_for(400 * MIB, Some(450 * MIB)), 450 * MIB);
    assert_eq!(ceiling_for(400 * MIB, Some(400 * MIB)), 400 * MIB);
    // The floor read: 16 MiB costs 24 MiB of ceiling.
    assert_eq!(ceiling_for(16 * MIB, None), 24 * MIB);
    assert_eq!(CEILING_TENTHS, 15);
    // The light class is the floor class: twice the floor, no more.
    assert_eq!(LIGHT_CLASS_MAX_COST, 2 * COST_FLOOR);
}

// ---------------------------------------------------------------- the estimate

/// An independent sum of regular files' apparent sizes (symlinks neither followed nor counted).
fn true_size(dir: &Path) -> u64 {
    let mut total = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let md = std::fs::symlink_metadata(e.path()).unwrap();
        if md.is_dir() {
            total += true_size(&e.path());
        } else if md.is_file() {
            total += md.len();
        }
    }
    total
}

#[test]
fn the_estimate_equals_the_true_size_of_a_nested_store() {
    let root = scratch("nested");
    let store = root.join(".prikk");
    std::fs::create_dir_all(store.join("containers/blob")).unwrap();
    std::fs::create_dir_all(store.join("refs/tmp")).unwrap();
    std::fs::write(store.join("FORMAT"), b"7\n").unwrap();
    std::fs::write(store.join("containers/blob/a.container"), vec![1u8; 70_001]).unwrap();
    std::fs::write(store.join("containers/index.container"), vec![2u8; 123]).unwrap();
    std::fs::write(store.join("refs/tmp/x"), b"").unwrap();
    // Content outside `.prikk` is not the store.
    std::fs::write(root.join("worktree-file"), vec![3u8; 999_999]).unwrap();
    assert_eq!(store_size(&root).unwrap(), 2 + 70_001 + 123);
    assert_eq!(store_size(&root).unwrap(), true_size(&store));
}

#[test]
fn symlinks_are_not_followed_and_add_nothing() {
    let root = scratch("symlinks");
    let store = root.join(".prikk");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("real"), vec![0u8; 1000]).unwrap();
    // A link to a large file outside the store, and a link back to the store (a loop if followed).
    let outside = scratch("symlinks-outside");
    std::fs::write(outside.join("huge"), vec![0u8; 5_000_000]).unwrap();
    std::os::unix::fs::symlink(outside.join("huge"), store.join("to-huge")).unwrap();
    std::os::unix::fs::symlink(&outside, store.join("to-dir")).unwrap();
    std::os::unix::fs::symlink(&store, store.join("loop")).unwrap();
    assert_eq!(store_size(&root).unwrap(), 1000);
}

#[test]
fn a_missing_store_or_an_unreadable_one_is_an_error_never_a_guess() {
    let root = scratch("missing");
    assert!(matches!(store_size(&root), Err(StoreSizeError::Io(_))));
}

#[test]
fn the_walk_is_bounded_by_the_entry_count() {
    let root = scratch("bounded");
    let store = root.join(".prikk");
    std::fs::create_dir_all(&store).unwrap();
    for i in 0..50 {
        std::fs::write(store.join(format!("f{i}")), b"x").unwrap();
    }
    assert_eq!(store_size_limited(&root, 50).unwrap(), 50);
    assert!(matches!(
        store_size_limited(&root, 49),
        Err(StoreSizeError::TooManyEntries)
    ));
    assert_eq!(MAX_STORE_ENTRIES, 100_000);
}

// ---------------------------------------------------------------- the estimate on a real repository

fn have_prikk() -> bool {
    Command::new("prikk")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn the_estimate_equals_the_true_size_of_a_real_sealed_repository() {
    if !have_prikk() {
        eprintln!("skipping: prikk is not on PATH");
        return;
    }
    let base = scratch("real");
    let repo = base.join("repo");
    let keys = base.join("keys");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&keys).unwrap();
    let prikk = |args: &[&str]| {
        let ok = Command::new("prikk")
            .args(args)
            .current_dir(&repo)
            .env("HOME", &keys)
            .env("XDG_CONFIG_HOME", &keys)
            .status()
            .unwrap();
        assert!(ok.success(), "prikk {args:?}");
    };
    prikk(&["setup"]);
    std::fs::write(repo.join("a.bin"), vec![7u8; 300_000]).unwrap();
    std::fs::write(repo.join("README.md"), b"hello\n").unwrap();
    prikk(&["commit", "--from-worktree", "-m", "init"]);
    prikk(&["seal", "--allow-no-audit"]);
    let got = store_size(&repo).unwrap();
    assert_eq!(got, true_size(&repo.join(".prikk")));
    assert!(got > 300_000, "{got}");
}

#[test]
fn the_ceiling_clears_the_smallest_rlimit_data_a_legitimate_call_needs() {
    // The ceiling is an RLIMIT_DATA, which counts address space reserved (allocator growth included), not
    // only what is resident — so it is checked against the *smallest working limit* found by bisection
    // (T5, prikk 0.52.0: (fixture, verb, store bytes by the walk, smallest working limit in bytes)).
    let needs: &[(&str, ReadVerb, u64, u64)] = &[
        ("m1", ReadVerb::Log, 1013680, 2134016),
        ("m1", ReadVerb::Show, 1013680, 2174976),
        ("m1", ReadVerb::Refs, 1013680, 2134016),
        ("m1", ReadVerb::Tree, 1013680, 2134016),
        ("m1", ReadVerb::Cat, 1013680, 3055616),
        ("m1", ReadVerb::FileContent, 1013680, 2134016),
        ("m1", ReadVerb::Verify, 1013680, 5009408),
        ("m16", ReadVerb::Log, 15831709, 2134016),
        ("m16", ReadVerb::Show, 15831709, 2502301),
        ("m16", ReadVerb::Refs, 15831709, 2134016),
        ("m16", ReadVerb::Tree, 15831709, 18128216),
        ("m16", ReadVerb::Cat, 15831709, 18128216),
        ("m16", ReadVerb::FileContent, 15831709, 18201981),
        ("m16", ReadVerb::Verify, 15831709, 34052919),
        ("m64", ReadVerb::Log, 66201411, 2134016),
        ("m64", ReadVerb::Show, 66201411, 2674904),
        ("m64", ReadVerb::Refs, 66201411, 2134016),
        ("m64", ReadVerb::Tree, 66201411, 68489626),
        ("m64", ReadVerb::Cat, 66201411, 68489626),
        ("m64", ReadVerb::FileContent, 66201411, 68534925),
        ("m64", ReadVerb::Verify, 66201411, 135127201),
        ("m200", ReadVerb::Log, 208915749, 2134016),
        ("m200", ReadVerb::Show, 208915749, 3929229),
        ("m200", ReadVerb::Refs, 208915749, 2134016),
        ("m200", ReadVerb::Tree, 208915749, 211230181),
        ("m200", ReadVerb::Cat, 208915749, 211230181),
        ("m200", ReadVerb::FileContent, 208915749, 211958258),
        ("m1000", ReadVerb::Log, 1048411603, 2134016),
        ("m1000", ReadVerb::Show, 1048411603, 11307611),
        ("m1000", ReadVerb::Refs, 1048411603, 2134016),
        ("hist1000", ReadVerb::Log, 2405152, 2134016),
        ("hist1000", ReadVerb::Show, 2405152, 2174976),
        ("hist1000", ReadVerb::Refs, 2405152, 2134016),
        ("hist1000", ReadVerb::Tree, 2405152, 2134016),
        ("hist1000", ReadVerb::Cat, 2405152, 2134016),
        ("hist1000", ReadVerb::FileContent, 2405152, 2137131),
        ("hist1000", ReadVerb::Verify, 2405152, 5894144),
        ("files10k", ReadVerb::Log, 7389309, 6336512),
        ("files10k", ReadVerb::Show, 7389309, 16961536),
        ("files10k", ReadVerb::Refs, 7389309, 4567040),
        ("files10k", ReadVerb::Tree, 7389309, 13856768),
        ("files10k", ReadVerb::Cat, 7389309, 13856768),
        ("files10k", ReadVerb::FileContent, 7389309, 13893025),
        ("files10k", ReadVerb::Verify, 7389309, 21800279),
        ("txt64", ReadVerb::Log, 67241474, 2134016),
        ("txt64", ReadVerb::Show, 67241474, 272655062),
        ("txt64", ReadVerb::Refs, 67241474, 2134016),
        ("txt64", ReadVerb::Tree, 67241474, 134949826),
        ("txt64", ReadVerb::Cat, 67241474, 134949826),
        ("txt64", ReadVerb::FileContent, 67241474, 135041053),
        ("txt64", ReadVerb::Verify, 67241474, 336584847),
        ("txt200", ReadVerb::Log, 209940730, 2134016),
        ("txt200", ReadVerb::Refs, 209940730, 2134016),
        ("bin64one", ReadVerb::Log, 67112573, 2134016),
        ("bin64one", ReadVerb::Show, 67112573, 134479046),
        ("bin64one", ReadVerb::Refs, 67112573, 2134016),
        ("bin64one", ReadVerb::Tree, 67112573, 134692087),
        ("bin64one", ReadVerb::Cat, 67112573, 134692087),
        ("bin64one", ReadVerb::FileContent, 67112573, 134805123),
        ("bin64one", ReadVerb::Verify, 67112573, 335939714),
        ("txt64many", ReadVerb::Log, 67154383, 2134016),
        ("txt64many", ReadVerb::Show, 67154383, 203725206),
        ("txt64many", ReadVerb::Refs, 67154383, 2134016),
        ("txt64many", ReadVerb::Tree, 67154383, 68776620),
        ("txt64many", ReadVerb::Cat, 67154383, 68776620),
        ("txt64many", ReadVerb::FileContent, 67154383, 68675372),
        ("txt64many", ReadVerb::Verify, 67154383, 137054085),
    ];
    let default_cap = 8192 * MIB;
    let mut tightest = (0.0_f64, "", ReadVerb::Log);
    for (fx, verb, store, need) in needs {
        let ceiling = ceiling_for(verb.cost(*store), Some(default_cap));
        assert!(
            ceiling >= *need,
            "{fx} {verb:?}: ceiling {ceiling} < the {need} bytes of address space the call needs"
        );
        let ratio = *need as f64 / ceiling as f64;
        if ratio > tightest.0 {
            tightest = (ratio, fx, *verb);
        }
    }
    eprintln!(
        "tightest: need/ceiling = {:.2} ({} {:?})",
        tightest.0, tightest.1, tightest.2
    );
    // With the 1.5 margin the tightest call (verify of a single large file) uses 0.82 of its ceiling; it
    // was 0.98 at 1.25. A margin that shrinks back would pass the `>=` above long before it worked.
    assert!(
        tightest.0 < 0.86,
        "the margin is thin again: {:.2}",
        tightest.0
    );
}
