//! What a prikk call may print before the server stops reading and kills it (RFC 011 follow-on, IS-55).
//!
//! RFC 011 bounds a call's memory (`prlimit --data`), its time (the deadline) and its lifetime (it dies with the server). This is
//! the fourth bound: **what the server collects**. The memory ceiling protects the host from the child, not from the server's own
//! buffer, and a hostile repository or archive can make an honest verb print as much as the repository or archive holds (a million
//! one-line branches make `archive import --format json` print a report as large as the archive).
//!
//! # The table
//!
//! One table, one place, with its provenance (like `cost_model` and `CostRule`): [`for_args`] says what a call gets from its
//! arguments. Each limit is **at least twice the largest legitimate output measured** for the verb (the measurements are in the
//! part's review request, `driver-output-cap-logs/t1/`, prikk 0.54.1), and a verb whose output is bounded by construction gets
//! a tight one. Re-measure at each prikk re-baseline, as the cost tables are.
//!
//! * a cap is **never** "unlimited": [`crate::sandbox::OutputLimit`] has no such value;
//! * a call that goes over is killed and fails as [`crate::PrikkError::OutputTooLarge`]: the output is dropped (a truncated report is
//!   never parsed as a whole one), the call is counted by verb ([`output_too_large_total`]) and one line is logged;
//! * a caller that knows better asks for another limit by passing different arguments (`log --limit N` raises the cap with `N`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::sandbox::{Exceeded, OutputLimit};

const MIB: u64 = 1 << 20;

/// stderr: prikk's messages are a line or a few (a refusal with its reason, a damage report's first lines). 1 MiB is far above
/// anything measured (the largest stderr in the measurements is 171 bytes) and far below anything that matters to the server.
pub const STDERR: u64 = MIB;

/// What a call gets when its verb is not in [`TABLE`] (a verb added later without a measurement): a generous general limit, still
/// finite. The test of the table fails for a verb the drivers use that falls through to it.
pub const GENERAL: OutputLimit = OutputLimit::new(16 * MIB, STDERR);

/// How a verb's stdout limit is derived.
#[derive(Clone, Copy, Debug)]
pub enum Stdout {
    /// One number.
    Fixed(u64),
    /// `log`: the output grows with the number of blocks asked for (`--limit N`; prikk's own default is 20): `floor + N × per_block`,
    /// never above `ceiling`.
    PerLimit {
        floor: u64,
        per_block: u64,
        ceiling: u64,
    },
    /// `cat`: the bytes the caller asked for with `--max-bytes N` (prikk refuses a file larger than that, exit 1, nothing on stdout),
    /// plus `slack`; with no `--max-bytes`, prikk's own default bound (25 MiB, measured) twice over; with `--format json` (the
    /// metadata, not the bytes) `metadata`.
    Cat {
        slack: u64,
        metadata: u64,
        default_twice: u64,
    },
}

/// One row of [`TABLE`]: a class of verbs, its limit, and where the number comes from.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub verbs: &'static [&'static str],
    pub stdout: Stdout,
    /// The largest legitimate output, in bytes: measured, or extrapolated linearly from the measured per-item size to the shape named in
    /// `measured`. The limit is at least twice this (a test holds the table to it; for `log` the number is the default call's, and the
    /// 100,000-block call is tested on its own; for `cat` it is prikk's default bound).
    pub largest: u64,
    /// The largest legitimate output measured (prikk 0.54.1; `driver-output-cap-logs/t1/` of the part's review request).
    pub measured: &'static str,
    /// The limit's reasoning: the margin over `measured`, and what an attacker reaches.
    pub why: &'static str,
}

/// The table. Each limit is **at least twice the largest legitimate output measured** for the verb ("legitimate" = the shapes the
/// handoff names, up to 20,000 refs / blocks / pending patches, 10,000 files, a file at the 4 MiB content bound); a verb whose
/// output is bounded by construction gets a tight one. Re-measure at each prikk re-baseline, as the cost tables are.
pub const TABLE: &[Rule] = &[
    Rule {
        verbs: &[
            "--version",
            "init",
            "key status",
            "trust maintainer",
            "config",
            "bundle",
            "archive export",
            "sync summary",
            "sync have",
            "sync build",
            "sync seal",
            "unlock",
            "status",
            "extract",
        ],
        stdout: Stdout::Fixed(MIB),
        largest: 6923,
        measured: "at most 3 KiB: --version 13 B, init 41 B, key status 911 B, trust list of 64 keys 6.9 KB, config list 48 B, bundle verify 840 B, \
                   archive export 476 B, sync summary 90 B, sync have 107 B (63 B on 0.55.0; the have-list itself is a file written by --output, not stdout: the \
                   server bounds it by `HAVE_MAX_BYTES`, 32 bytes a patch measured), sync build 319 B, unlock 24 B, status 604 B, extract --format json \
                   421 B (the same for 10,000 files, plan-only or not)",
        why: "bounded by construction: one report of a fixed shape (a key list is bounded by the 64 keys of the creation form); 1 MiB is > 100 \
              times the largest measured",
    },
    Rule {
        verbs: &["branch", "tag"],
        stdout: Stdout::Fixed(16 * MIB),
        largest: 3020000,
        measured: "branch --all 151 B per branch (2,000: 303,008 B; 20,000 extrapolates to 3.0 MB), tag 119 B per tag (500: 59,554 B)",
        why: "5 x the 20,000-ref shape; a 100 MiB archive (about 800 B a ref) can carry about 130,000 refs, i.e. 20 MB: beyond the limit, and \
              the listing is then unavailable, not partial",
    },
    Rule {
        verbs: &["worktree-status"],
        stdout: Stdout::Fixed(32 * MIB),
        largest: 14700000,
        measured: "147 B per changed file (2,000 modified: 294,402 B; 100,000 files extrapolate to 14.7 MB)",
        why: "2.2 x a worktree of 100,000 changed files",
    },
    Rule {
        verbs: &["tree"],
        stdout: Stdout::Fixed(32 * MIB),
        largest: 9700000,
        measured: "97 B per file (1,000: 92,083 B; 10,000: 970,280 B; text form 340,210 B for 10,000)",
        why: "3.4 x a tree of 100,000 files (9.7 MB); a tree larger than the store-entry bound (100,000) is refused before the listing",
    },
    Rule {
        verbs: &["show"],
        stdout: Stdout::Fixed(16 * MIB),
        largest: 324510,
        measured: "a block of 1,000 patches: 324,510 B (325 B per patch)",
        why: "25 x the measured block: a block of 25,000 patches would still be read",
    },
    Rule {
        verbs: &["checkout"],
        stdout: Stdout::Fixed(64 * MIB),
        largest: 25166201,
        measured: "`--patch-plan --content-path` of a file at the 4 MiB content bound: text 4,290,006 B; the worst case, 4 MiB of control bytes \
                   (every byte a 6-byte \\u00XX escape), 25,166,201 B",
        why: "2.6 x the worst case at CONTENT_MAX_BYTES (the read path checks the size before the call)",
    },
    Rule {
        verbs: &["cat"],
        stdout: Stdout::Cat {
            slack: MIB,
            metadata: MIB,
            default_twice: 50 * MIB,
        },
        largest: 26214400,
        measured: "raw content is the file itself: exactly --max-bytes N at most (25 MiB file, N = 26,214,400: 26,214,400 B; N = 1,000: refused, \
                   exit 1, 0 B); with no --max-bytes prikk's own bound is 26,214,400 B; --format json metadata 246-329 B",
        why: "N plus 1 MiB (prikk refuses a larger file by itself); 2 x prikk's default bound when no N is given. Bounded by what the caller \
              asked for; the file route asks for at most 25 MiB (RAW_MAX_BYTES), the content view for at most 4 MiB",
    },
    Rule {
        verbs: &["log"],
        stdout: Stdout::PerLimit {
            floor: 8 * MIB,
            per_block: 1536,
            ceiling: 192 * MIB,
        },
        largest: 2300000,
        measured: "635 B per one-patch block as JSON (200 blocks, --limit 100000: 127,137 B; text 90,270 B); a block of 1,000 patches 114,563 B; \
                   prikk's default is the last 20 blocks (20 blocks of 1,000 patches: about 2.3 MB). A 100,000-block log extrapolates to 63.5 MB",
        why: "floor 8 MiB (3.6 x 20 blocks of 1,000 patches); 1,536 B per block asked for (2.4 x measured) so that `--limit 100000` (the \
              observation's LOG_BLOCK_LIMIT) is admitted at 162 MB, 2.5 x its 63.5 MB; ceiling 192 MiB",
    },
    Rule {
        verbs: &["verify"],
        stdout: Stdout::Fixed(32 * MIB),
        largest: 11500000,
        measured: "healthy 1,179 B; damaged 1,325-1,966 B (two shapes); 2,000 unreached objects 231,292 B (115 B each; text form 432,768 B)",
        why: "2.2 x 100,000 unreached objects (11.5 MB), more than a 100 MiB archive's refs can leave unreached",
    },
    Rule {
        verbs: &["doctor"],
        stdout: Stdout::Fixed(32 * MIB),
        largest: 9500000,
        measured: "healthy 2,943-2,959 B; damaged 6,846-7,690 B; 2,000 unreached objects 949,322 B (475 B each: 20,000 extrapolate to 9.5 MB)",
        why: "3.4 x the 20,000-unreached shape",
    },
    Rule {
        verbs: &["archive verify"],
        stdout: Stdout::Fixed(16 * MIB),
        largest: 6800000,
        measured: "716-719 B for 1 to 2,000 branches (it does not list refs); damaged: 48,853 B JSON / 52,529 B text for a 742,560 B archive with \
                   736 damaged entries (66 B each); more damage fails earlier with 104 B on stderr and nothing on stdout",
        why: "a 100 MiB archive damaged once every 1,009 bytes is about 6.8 MB of entries, so 2.4 x that",
    },
    Rule {
        verbs: &["archive import"],
        stdout: Stdout::Fixed(64 * MIB),
        largest: 22000000,
        measured: "171 B per branch with --adopt (1: 689 B; 100: 17,107 B; 2,000: 338,308 B; 20,000 extrapolates to 3.4 MB)",
        why: "19 x the 20,000-branch shape and more than a 100 MiB archive (the edge bound; about 130,000 refs, 22 MB) can print, so a \
              creation never meets the limit on an archive the edge accepted",
    },
    Rule {
        verbs: &["sync pending"],
        stdout: Stdout::Fixed(16 * MIB),
        largest: 1340000,
        measured: "67 B per pending patch (none: 40 B; 1,000: 67,043 B; 20,000 extrapolates to 1.3 MB)",
        why: "12 x the 20,000-pending shape",
    },
    Rule {
        verbs: &["sync accept"],
        stdout: Stdout::Fixed(16 * MIB),
        largest: 10000,
        measured: "107 B per claim (an artifact of 1,000 blocks: 107,259 B; 20,000 claims extrapolate to 2.1 MB); an artifact of 1,000 patches \
                   in one block: 369 B (the report lists claims, not patches); the claims go to --claims-out",
        why: "7.5 x the 20,000-claim shape; claims are bounded by the artifact, which the edge bounds",
    },
];

/// The row of a verb, if it has one.
pub fn rule_of(verb: &str) -> Option<&'static Rule> {
    TABLE.iter().find(|r| r.verbs.contains(&verb))
}

/// Whether a verb has a measured row.
pub fn is_in_table(verb: &str) -> bool {
    rule_of(verb).is_some()
}

/// The value after a flag in the arguments (`--limit 100000`).
fn flag_value<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| *a == flag)
        .and_then(|i| args.get(i + 1))
        .copied()
}

/// The verb of a call, for the table and the counter: the first word, or the first two for the verbs that have sub-verbs
/// (`trust maintainer add` is `trust maintainer`).
pub fn verb_of(args: &[&str]) -> String {
    let first = args.first().copied().unwrap_or("");
    match first {
        "archive" | "bundle" | "sync" | "key" | "config" => match args.get(1) {
            Some(second) if !second.starts_with('-') && first != "bundle" && first != "config" => {
                format!("{first} {second}")
            }
            _ => first.to_owned(),
        },
        "trust" => match (args.get(1), args.get(2)) {
            (Some(&"maintainer"), _) => "trust maintainer".to_owned(),
            _ => "trust".to_owned(),
        },
        _ => first.to_owned(),
    }
}

/// The limit of a call, from its arguments. A verb that is not in [`TABLE`] gets [`GENERAL`].
pub fn for_args(args: &[&str]) -> OutputLimit {
    let verb = verb_of(args);
    let stdout = match rule_of(&verb).map(|r| r.stdout) {
        None => GENERAL.stdout,
        Some(Stdout::Fixed(n)) => n,
        Some(Stdout::PerLimit {
            floor,
            per_block,
            ceiling,
        }) => {
            let n = flag_value(args, "--limit")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            floor
                .saturating_add(n.saturating_mul(per_block))
                .min(ceiling)
        }
        Some(Stdout::Cat {
            slack,
            metadata,
            default_twice,
        }) => {
            if args.contains(&"--format") {
                metadata
            } else {
                match flag_value(args, "--max-bytes").and_then(|v| v.parse::<u64>().ok()) {
                    Some(n) => n.saturating_add(slack),
                    None => default_twice,
                }
            }
        }
    };
    OutputLimit::new(stdout, STDERR)
}

// ------------------------------------------------------------------------------------------------ the counter

static OVER: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// Count one call that went over its limit and log one line (the verb and the limit; never a path, a name or a byte of output).
pub(crate) fn record(args: &[&str], over: &Exceeded) -> String {
    let verb = verb_of(args);
    *OVER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(verb.clone())
        .or_insert(0) += 1;
    eprintln!(
        "planeter: a prikk call printed more than it may: verb={verb} stream={} limit={} bytes",
        over.stream, over.limit
    );
    verb
}

/// `prikk_output_too_large_total` by verb: the calls killed since start for printing more than their limit.
pub fn output_too_large_total() -> BTreeMap<String, u64> {
    OVER.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}
