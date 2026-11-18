//! RFC 008 handoff 1, T2: the written rule of a delivery as one pure function. The table of the measurement (`r12.json`: seventeen
//! archives against two states of the forge) is the test, as data; block ids are small names, a chain is written tip first.

use planeter_transport::delivery::rule::{
    Acceptable, LiveFacts, RefFacts, Refusal, StagedFacts, decide,
};

const M1: &str = "ed25519-m1";
const M2: &str = "ed25519-m2";
const X: &str = "ed25519-x";

fn chain(name: &str, tip_first: &[&str]) -> RefFacts {
    RefFacts {
        name: name.to_owned(),
        blocks: tip_first.iter().map(|s| (*s).to_owned()).collect(),
    }
}

/// `b1..bn` as a chain, tip first, with `prefix` naming the blocks.
fn blocks(prefix: &str, from: usize, to: usize) -> Vec<String> {
    (from..=to).rev().map(|i| format!("{prefix}{i}")).collect()
}

fn on_top(base: &[String], prefix: &str, from: usize, to: usize) -> Vec<String> {
    let mut v = blocks(prefix, from, to);
    v.extend(base.iter().cloned());
    v
}

fn refs(name: &str, b: &[String]) -> RefFacts {
    RefFacts {
        name: name.to_owned(),
        blocks: b.to_vec(),
    }
}

/// A clean stage: every ref's tip and block sealed by the given keys, verify ok.
fn clean(local: Vec<RefFacts>, sealer: &str) -> StagedFacts {
    let tip_sealers = local
        .iter()
        .map(|r| (r.name.clone(), vec![sealer.to_owned()]))
        .collect();
    let block_sealers = local
        .iter()
        .flat_map(|r| {
            r.blocks
                .iter()
                .map(|b| (b.clone(), vec![sealer.to_owned()]))
        })
        .collect();
    StagedFacts {
        damaged: false,
        local,
        received: Vec::new(),
        verify_ok: true,
        verify_failed: Vec::new(),
        publication_trust_issues: 0,
        tip_sealers,
        block_sealers,
        followed_patch_ids: Default::default(),
        adopted: vec![M1.to_owned(), M2.to_owned()],
    }
}

/// The history of the measurement: H0 (5 blocks, h1..h5); D1 = H0 + d6..d10; D2 = D1 + d11.
fn h0() -> Vec<String> {
    blocks("h", 1, 5)
}
fn d1() -> Vec<String> {
    on_top(&h0(), "d", 6, 10)
}

fn live(name_chains: &[(&str, Vec<String>)]) -> LiveFacts {
    LiveFacts {
        refs: name_chains.iter().map(|(n, c)| refs(n, c)).collect(),
    }
}

/// One row of the table: the staged facts, and what each forge state answers.
struct Case {
    label: &'static str,
    staged: StagedFacts,
    s0: Result<(u64, Vec<&'static str>), Refusal>,
    s1: Result<(u64, Vec<&'static str>), Refusal>,
}

fn ok(new_blocks: u64) -> Result<(u64, Vec<&'static str>), Refusal> {
    Ok((new_blocks, Vec::new()))
}

fn received_x() -> StagedFacts {
    // `x` and `mix`: the stage landed the ref as received (the key is not adopted), verify says publication-trust, no local main.
    StagedFacts {
        damaged: false,
        local: Vec::new(),
        received: vec!["remotes/heads/main".to_owned()],
        verify_ok: false,
        verify_failed: vec!["publication-trust".to_owned()],
        publication_trust_issues: 4,
        tip_sealers: Vec::new(),
        block_sealers: vec![("a9f63231".to_owned(), vec![X.to_owned()])],
        followed_patch_ids: Default::default(),
        adopted: vec![M1.to_owned(), M2.to_owned()],
    }
}

fn damaged() -> StagedFacts {
    StagedFacts {
        damaged: true,
        ..StagedFacts::default()
    }
}

fn table() -> Vec<Case> {
    let d2 = on_top(&d1(), "d", 11, 11);
    let m2 = on_top(&d1(), "m", 11, 12);
    let div = on_top(&h0(), "v", 6, 8);
    let m2div = on_top(&h0(), "w", 6, 8);
    let xcarry = on_top(&d1(), "c", 11, 11);
    let unrelated = blocks("u", 1, 5);
    let mut m2_staged = clean(vec![refs("heads/main", &m2)], M1);
    // the tip of m2 is sealed by M2, an adopted key
    m2_staged.tip_sealers = vec![("heads/main".to_owned(), vec![M2.to_owned()])];
    let mut m2div_staged = clean(vec![refs("heads/main", &m2div)], M2);
    m2div_staged.tip_sealers = vec![("heads/main".to_owned(), vec![M2.to_owned()])];
    let mut dev = clean(
        vec![
            refs("heads/main", &d1()),
            refs("heads/dev", &on_top(&d1(), "e", 11, 11)),
        ],
        M1,
    );
    dev.adopted = vec![M1.to_owned(), M2.to_owned()];
    vec![
        Case {
            label: "D1 (10 blocks, M1)",
            staged: clean(vec![refs("heads/main", &d1())], M1),
            s0: ok(5),
            s1: Err(Refusal::NothingToDeliver),
        },
        Case {
            label: "D2 (D1 + 1)",
            staged: clean(vec![refs("heads/main", &d2)], M1),
            s0: ok(6),
            s1: ok(1),
        },
        Case {
            label: "H0 (older than D1)",
            staged: clean(vec![refs("heads/main", &h0())], M1),
            s0: Err(Refusal::NothingToDeliver),
            s1: Err(Refusal::OlderOrDiverged),
        },
        Case {
            label: "div (a rewrite from H0)",
            staged: clean(vec![refs("heads/main", &div)], M1),
            s0: ok(3),
            s1: Err(Refusal::OlderOrDiverged),
        },
        Case {
            label: "m2 (D1 + 2 blocks by M2)",
            staged: m2_staged,
            s0: ok(7),
            s1: ok(2),
        },
        Case {
            label: "m2div (3 blocks by M2 from H0)",
            staged: m2div_staged,
            s0: ok(3),
            s1: Err(Refusal::OlderOrDiverged),
        },
        Case {
            label: "x (D1 + 2 blocks by unadopted X)",
            staged: received_x(),
            s0: Err(Refusal::UnadoptedKey),
            s1: Err(Refusal::UnadoptedKey),
        },
        Case {
            label: "mix (X block in the middle, M2 tip)",
            staged: received_x(),
            s0: Err(Refusal::UnadoptedKey),
            s1: Err(Refusal::UnadoptedKey),
        },
        Case {
            label: "xcarry (carries X's key; every block by M1)",
            staged: clean(vec![refs("heads/main", &xcarry)], M1),
            s0: ok(6),
            s1: ok(1),
        },
        Case {
            label: "unrelated (M1, no common block)",
            staged: clean(vec![refs("heads/main", &unrelated)], M1),
            s0: Err(Refusal::Unrelated),
            s1: Err(Refusal::Unrelated),
        },
        Case {
            label: "dev (D1 + heads/dev, M1)",
            staged: dev,
            // the blocks of main the forge lacks (5) and the one block of the branch
            s0: Ok((6, vec!["heads/dev"])),
            // the same heads/main tip as the forge's, and a branch to add: acceptable (the delivery changes a ref)
            s1: Ok((1, vec!["heads/dev"])),
        },
        Case {
            label: "truncated D1",
            staged: damaged(),
            s0: Err(Refusal::DamagedArchive),
            s1: Err(Refusal::DamagedArchive),
        },
        Case {
            label: "D1, byte flipped (five places)",
            staged: damaged(),
            s0: Err(Refusal::DamagedArchive),
            s1: Err(Refusal::DamagedArchive),
        },
    ]
}

fn forge(state: &str) -> LiveFacts {
    match state {
        "S0" => live(&[("heads/main", h0())]),
        _ => live(&[("heads/main", d1())]),
    }
}

fn shape(r: Result<Acceptable, Refusal>) -> Result<(u64, Vec<String>), Refusal> {
    r.map(|a| (a.new_blocks, a.refs_added))
}

fn expect(r: &Result<(u64, Vec<&'static str>), Refusal>) -> Result<(u64, Vec<String>), Refusal> {
    r.clone()
        .map(|(n, v)| (n, v.into_iter().map(str::to_owned).collect()))
}

#[test]
fn the_whole_measured_table_in_both_forge_states() {
    for case in table() {
        for (state, want) in [("S0", &case.s0), ("S1", &case.s1)] {
            let got = shape(decide(&forge(state), &case.staged));
            assert_eq!(got, expect(want), "{} against {state}", case.label);
        }
    }
}

#[test]
fn an_accepted_delivery_names_both_tips_and_the_blocks_between_them() {
    let a = decide(&forge("S0"), &clean(vec![refs("heads/main", &d1())], M1)).unwrap();
    assert_eq!(a.old_tip.as_deref(), Some("h5"));
    assert_eq!(a.new_tip, "d10");
    assert_eq!(a.new_blocks, 5);
    assert!(a.refs_added.is_empty());
}

#[test]
fn an_extra_branch_is_accepted_and_listed_and_a_deleted_branch_is_refused() {
    // the live repository has heads/main and heads/dev
    let live = live(&[
        ("heads/main", h0()),
        ("heads/dev", on_top(&h0(), "e", 6, 6)),
    ]);
    // a delivery that has them both, main moved: accepted
    let both = clean(
        vec![
            refs("heads/main", &on_top(&h0(), "d", 6, 9)),
            refs("heads/dev", &on_top(&h0(), "e", 6, 7)),
        ],
        M1,
    );
    assert!(decide(&live, &both).is_ok());
    // without heads/dev: refused, whatever else is true
    let deleted = clean(vec![refs("heads/main", &on_top(&h0(), "d", 6, 9))], M1);
    assert_eq!(decide(&live, &deleted), Err(Refusal::BranchMissing));
    // heads/dev rewritten (its live tip is not in the staged chain): not a successor
    let rewritten = clean(
        vec![
            refs("heads/main", &on_top(&h0(), "d", 6, 9)),
            refs("heads/dev", &on_top(&h0(), "z", 6, 7)),
        ],
        M1,
    );
    assert_eq!(decide(&live, &rewritten), Err(Refusal::NotASuccessor));
    // a delivery that adds a branch the live repository lacks
    let add = clean(
        vec![
            refs("heads/main", &on_top(&h0(), "d", 6, 9)),
            refs("heads/new", &on_top(&h0(), "n", 6, 6)),
        ],
        M1,
    );
    let live_main = LiveFacts {
        refs: vec![refs("heads/main", &h0())],
    };
    assert_eq!(
        decide(&live_main, &add).unwrap().refs_added,
        vec!["heads/new".to_owned()]
    );
}

#[test]
fn the_same_tip_is_nothing_to_deliver_and_a_missing_main_is_no_main() {
    let same = clean(vec![refs("heads/main", &h0())], M1);
    assert_eq!(decide(&forge("S0"), &same), Err(Refusal::NothingToDeliver));
    let only_dev = clean(vec![refs("heads/dev", &h0())], M1);
    assert_eq!(decide(&forge("S0"), &only_dev), Err(Refusal::NoMain));
    assert_eq!(
        decide(&forge("S0"), &StagedFacts::default()),
        Err(Refusal::NoMain)
    );
}

#[test]
fn a_repository_without_a_main_takes_any_acceptable_history() {
    let a = decide(
        &LiveFacts::default(),
        &clean(vec![refs("heads/main", &h0())], M1),
    )
    .unwrap();
    assert_eq!((a.old_tip, a.new_blocks), (None, 5));
}

#[test]
fn an_unadopted_key_is_refused_wherever_it_shows() {
    let base = || clean(vec![refs("heads/main", &d1())], M1);
    // a block sealed by an unadopted key under an adopted tip (`mix`)
    let mut mix = base();
    mix.block_sealers.push(("d3".into(), vec![X.into()]));
    assert_eq!(decide(&forge("S0"), &mix), Err(Refusal::UnadoptedKey));
    // the tip
    let mut tip = base();
    tip.tip_sealers = vec![("heads/main".into(), vec![X.into()])];
    assert_eq!(decide(&forge("S0"), &tip), Err(Refusal::UnadoptedKey));
    // a publication-trust finding of verify, nothing else
    let mut pt = base();
    pt.publication_trust_issues = 1;
    assert_eq!(decide(&forge("S0"), &pt), Err(Refusal::UnadoptedKey));
    let mut failed = base();
    failed.verify_ok = false;
    failed.verify_failed = vec!["publication-trust".into()];
    assert_eq!(decide(&forge("S0"), &failed), Err(Refusal::UnadoptedKey));
    // a ref that landed as received
    let mut recv = base();
    recv.received = vec!["remotes/heads/dev".into()];
    assert_eq!(decide(&forge("S0"), &recv), Err(Refusal::UnadoptedKey));
    // a verify that fails for another reason is the archive's
    let mut other = base();
    other.verify_ok = false;
    other.verify_failed = vec!["structure".into()];
    assert_eq!(decide(&forge("S0"), &other), Err(Refusal::DamagedArchive));
}

#[test]
fn a_six_thousand_block_history_is_a_successor_not_a_false_refusal() {
    // the live tip is the 1st block of a 6,000-block staged chain: a reader that stopped at a fixed 5,000 would not see it
    let long = blocks("l", 1, 6000);
    let live = LiveFacts {
        refs: vec![refs("heads/main", &[long[5999].clone()])],
    };
    let a = decide(&live, &clean(vec![refs("heads/main", &long)], M1)).unwrap();
    assert_eq!(a.new_blocks, 5999);
    // and 6,000 live blocks under a 6,001-block staged chain
    let longer = on_top(&long, "m", 1, 1);
    let live = LiveFacts {
        refs: vec![refs("heads/main", &long)],
    };
    assert_eq!(
        decide(&live, &clean(vec![refs("heads/main", &longer)], M1))
            .unwrap()
            .new_blocks,
        1
    );
}

#[test]
fn the_reasons_are_a_closed_list_of_words() {
    let words: Vec<&str> = Refusal::ALL.iter().map(|r| r.code()).collect();
    assert_eq!(
        words,
        [
            "no_main",
            "unadopted_key",
            "not_a_successor",
            "older_or_diverged",
            "unrelated",
            "branch_missing",
            "nothing_to_deliver",
            "damaged_archive"
        ]
    );
    for r in Refusal::ALL {
        assert_eq!(Refusal::parse(r.code()), Some(r));
    }
    assert_eq!(Refusal::parse("other"), None);
    let _ = chain("x", &["a"]);
}

/// ENF-2: **the server signs nothing.** The engine's code never calls anything that would: no `seal`, no `merge`, no `branch close`,
/// no `--allow-non-fast-forward`, no `--yes` to `unlock`, no `unsafe`. A source test, as the other guards of this kind are.
#[test]
fn the_engine_never_signs_closes_merges_or_forces() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/delivery");
    let forbidden = [
        "allow-non-fast-forward",
        "sync_seal",
        "\"seal\"",
        "\"merge\"",
        "\"close\"",
        "branch_close",
        "--yes",
        "unsafe",
        "Command::new",
    ];
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for word in forbidden {
                assert!(
                    !line.contains(word),
                    "{}:{}: `{word}` in the engine: {line}",
                    entry.path().display(),
                    n + 1
                );
            }
        }
        checked += 1;
    }
    assert!(checked >= 2);
}

#[test]
fn a_delivery_is_nothing_to_deliver_only_when_no_ref_changes_and_none_is_added() {
    let live = live(&[
        ("heads/main", d1()),
        ("heads/dev", on_top(&d1(), "e", 11, 11)),
    ]);
    // nothing moved, nothing added
    let same = clean(
        vec![
            refs("heads/main", &d1()),
            refs("heads/dev", &on_top(&d1(), "e", 11, 11)),
        ],
        M1,
    );
    assert_eq!(decide(&live, &same), Err(Refusal::NothingToDeliver));
    // only a branch moved: acceptable, main's tip is the old one, one new block
    let dev_moved = clean(
        vec![
            refs("heads/main", &d1()),
            refs("heads/dev", &on_top(&d1(), "e", 11, 12)),
        ],
        M1,
    );
    let a = decide(&live, &dev_moved).unwrap();
    assert_eq!(
        (a.old_tip.as_deref(), a.new_tip.as_str()),
        (Some("d10"), "d10")
    );
    assert_eq!(a.new_blocks, 1);
    assert!(a.refs_added.is_empty());
    // only a branch added
    let live_main = LiveFacts {
        refs: vec![refs("heads/main", &d1())],
    };
    let added = clean(
        vec![
            refs("heads/main", &d1()),
            refs("heads/dev", &on_top(&d1(), "e", 11, 11)),
        ],
        M1,
    );
    let a = decide(&live_main, &added).unwrap();
    assert_eq!(a.refs_added, vec!["heads/dev".to_owned()]);
    assert_eq!(a.new_blocks, 1);
    // the blocks both branches share are counted once
    let two = clean(
        vec![
            refs("heads/main", &on_top(&d1(), "d", 11, 11)),
            refs("heads/dev", &on_top(&d1(), "d", 11, 11)),
        ],
        M1,
    );
    assert_eq!(decide(&live_main, &two).unwrap().new_blocks, 1);
}
