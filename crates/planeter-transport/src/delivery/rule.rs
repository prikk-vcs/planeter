//! The written rule of a delivery (RFC 008 D-3) as **one pure function**: [`decide`] takes what the live repository and the stage
//! look like (typed facts, read by the driver's readers) and answers [`Acceptable`] or one closed [`Refusal`]. No file, no
//! process, no clock: the table of the measurement (`r12.json`, both forge states) is its test, as data.
//!
//! The order is the RFC's; the first failure is the answer.
//!
//! 0. the archive was refused by prikk (`damaged_archive`);
//! 1. nothing may have landed as *received* (a ref signed by a key the repository does not trust: `unadopted_key`), and
//!    `heads/main` must exist as a local branch of the stage (`no_main`);
//! 2. every local ref's tip, and every block, is sealed by an adopted key, and `verify` passes with no publication-trust issue
//!    (`unadopted_key`; a `verify` that fails for another reason is `damaged_archive`);
//! 3. for every published ref of the live repository that the stage also has, the live tip is a block of the staged chain of the
//!    same ref: else `unrelated` (the two histories share no block), `older_or_diverged` (they share one: `heads/main`), or
//!    `not_a_successor` (any other ref);
//! 4. no live ref is missing in the stage (`branch_missing`); a ref the live repository lacks may be added;
//! 5. the staged `heads/main` differs from the live one (`nothing_to_deliver`).
//!
//! Where two reasons both hold for the same archive the more specific one wins (an unadopted key's block makes `no_main` too:
//! the answer is `unadopted_key`).

use std::collections::BTreeSet;

use crate::proposals::FOLLOWED_REF;

/// One published ref as the readers show it: the chain of blocks `log` lists, **tip first** (so `blocks[0]` is the tip).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefFacts {
    pub name: String,
    /// The block ids of the whole chain, tip first. Never a prefix: the reader counts and asks again until it holds all of it.
    pub blocks: Vec<String>,
}

impl RefFacts {
    pub fn tip(&self) -> Option<&str> {
        self.blocks.first().map(String::as_str)
    }
}

/// What the live repository is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveFacts {
    /// Every published (local, not closed) ref.
    pub refs: Vec<RefFacts>,
}

/// What the stage is after `archive import --adopt`, `branch --all`, `verify` and `log`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StagedFacts {
    /// prikk refused the archive (`archive verify` or `archive import` failed, or a report could not be bounded).
    pub damaged: bool,
    /// The refs that landed as local branches, each with its whole chain.
    pub local: Vec<RefFacts>,
    /// The refs that landed as *received* (not adopted).
    pub received: Vec<String>,
    /// `verify`'s verdict, and the ids of the conditions that failed.
    pub verify_ok: bool,
    pub verify_failed: Vec<String>,
    /// Publication-trust findings `verify` printed.
    pub publication_trust_issues: usize,
    /// For each local ref, the keys that sealed its tip.
    pub tip_sealers: Vec<(String, Vec<String>)>,
    /// Every block `verify` lists and the keys that sealed it.
    pub block_sealers: Vec<(String, Vec<String>)>,
    /// The patch ids of the blocks of the followed ref's chain (for the report's landing, not for the rule).
    pub followed_patch_ids: BTreeSet<String>,
    /// The key ids of the live repository's trust list: the adopted keys.
    pub adopted: Vec<String>,
}

/// The closed list of refusals the rule gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    NoMain,
    UnadoptedKey,
    NotASuccessor,
    OlderOrDiverged,
    Unrelated,
    BranchMissing,
    NothingToDeliver,
    DamagedArchive,
}

impl Refusal {
    /// The stable word (the report's `reason`, the counters' key, the API's code).
    pub fn code(self) -> &'static str {
        match self {
            Refusal::NoMain => "no_main",
            Refusal::UnadoptedKey => "unadopted_key",
            Refusal::NotASuccessor => "not_a_successor",
            Refusal::OlderOrDiverged => "older_or_diverged",
            Refusal::Unrelated => "unrelated",
            Refusal::BranchMissing => "branch_missing",
            Refusal::NothingToDeliver => "nothing_to_deliver",
            Refusal::DamagedArchive => "damaged_archive",
        }
    }

    pub fn parse(code: &str) -> Option<Refusal> {
        Refusal::ALL.into_iter().find(|r| r.code() == code)
    }

    pub const ALL: [Refusal; 8] = [
        Refusal::NoMain,
        Refusal::UnadoptedKey,
        Refusal::NotASuccessor,
        Refusal::OlderOrDiverged,
        Refusal::Unrelated,
        Refusal::BranchMissing,
        Refusal::NothingToDeliver,
        Refusal::DamagedArchive,
    ];
}

/// What an acceptable delivery would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acceptable {
    /// The live `heads/main` tip (`None` for a repository that has none yet).
    pub old_tip: Option<String>,
    pub new_tip: String,
    /// The staged chain's blocks after the live tip (all of them when the live repository has no `heads/main`).
    pub new_blocks: u64,
    /// The refs the live repository does not have and the delivery adds, sorted.
    pub refs_added: Vec<String>,
}

fn find<'a>(refs: &'a [RefFacts], name: &str) -> Option<&'a RefFacts> {
    refs.iter().find(|r| r.name == name)
}

/// The rule. Pure.
pub fn decide(live: &LiveFacts, staged: &StagedFacts) -> Result<Acceptable, Refusal> {
    // 0. prikk's own refusal.
    if staged.damaged {
        return Err(Refusal::DamagedArchive);
    }
    // 1. nothing received; heads/main is a local branch.
    if !staged.received.is_empty() {
        return Err(Refusal::UnadoptedKey);
    }
    let Some(main) = find(&staged.local, FOLLOWED_REF) else {
        return Err(Refusal::NoMain);
    };
    // 2. every tip and every block sealed by an adopted key; verify passes without a publication-trust issue.
    let adopted: BTreeSet<&str> = staged.adopted.iter().map(String::as_str).collect();
    let unadopted = |keys: &Vec<String>| keys.iter().any(|k| !adopted.contains(k.as_str()));
    if staged.tip_sealers.iter().any(|(_, keys)| unadopted(keys))
        || staged.block_sealers.iter().any(|(_, keys)| unadopted(keys))
        || staged.publication_trust_issues > 0
        || staged
            .verify_failed
            .iter()
            .any(|c| c == "publication-trust")
    {
        return Err(Refusal::UnadoptedKey);
    }
    if !staged.verify_ok {
        return Err(Refusal::DamagedArchive);
    }
    // 3. every live ref the stage has: the live tip is a block of the staged chain of the same ref.
    for l in &live.refs {
        let Some(s) = find(&staged.local, &l.name) else {
            continue;
        };
        let Some(tip) = l.tip() else { continue };
        if s.blocks.iter().any(|b| b == tip) {
            continue;
        }
        if l.name != FOLLOWED_REF {
            return Err(Refusal::NotASuccessor);
        }
        let staged_ids: BTreeSet<&str> = s.blocks.iter().map(String::as_str).collect();
        return Err(
            if l.blocks.iter().any(|b| staged_ids.contains(b.as_str())) {
                Refusal::OlderOrDiverged
            } else {
                Refusal::Unrelated
            },
        );
    }
    // 4. no live ref missing in the stage.
    if live
        .refs
        .iter()
        .any(|l| find(&staged.local, &l.name).is_none())
    {
        return Err(Refusal::BranchMissing);
    }
    // 5. the delivered heads/main differs from the live one.
    let new_tip = main.tip().ok_or(Refusal::NoMain)?.to_owned();
    let old = find(&live.refs, FOLLOWED_REF).and_then(RefFacts::tip);
    if old == Some(new_tip.as_str()) {
        return Err(Refusal::NothingToDeliver);
    }
    let new_blocks = match old {
        Some(o) => main.blocks.iter().position(|b| b == o).unwrap_or(0) as u64,
        None => main.blocks.len() as u64,
    };
    let mut refs_added: Vec<String> = staged
        .local
        .iter()
        .filter(|s| find(&live.refs, &s.name).is_none())
        .map(|s| s.name.clone())
        .collect();
    refs_added.sort();
    Ok(Acceptable {
        old_tip: old.map(str::to_owned),
        new_tip,
        new_blocks,
        refs_added,
    })
}
