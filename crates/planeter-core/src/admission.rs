//! The read-path cost model and the repository size estimate (RFC 011 D-1, D-3).
//!
//! A read is admitted by **estimated memory**, not just by process count (investigation I1/C2: the gate
//! bounded the number of prikk processes while 8 concurrent `verify` calls on a 1 GiB repository used
//! 16 GB). The estimate has two inputs: the repository's **store size S** — a fresh walk of its `.prikk`
//! directory ([`store_size`], 50–65 µs, no cache) — and the verb, through the one table below.
//!
//! # The table is a per-release measurement, not a constant
//!
//! [`ReadVerb::rule`] is matched **exhaustively** (adding a read verb without a cost does not compile),
//! and every rule carries the prikk version it was measured on ([`MEASURED_ON`]). A test fails when that
//! differs from [`planeter_prikk::cli::MIN_PRIKK_VERSION`], so the table is re-measured at every floor
//! change (`docs/UPSTREAM.md`, re-baseline procedure). prikk's own multiples are undocumented and depend on
//! history since the last snapshot (ledger PK-53).
//!
//! # Worst-case multiples (investigation I1; they never under-predict)
//!
//! Peak memory follows S plus the largest single object, which no `stat` reveals, so the multiples are the
//! worst case — a repository dominated by one large file: `tree`/`cat` 2.1 × S, `verify` 4.1 × S. For
//! many-small-file repositories this over-reserves up to 2.1× (accepted: capacity, never safety). `log`,
//! `branch` and `tag` are flat (≈ 9.5 MB). `show` is *not*: it follows the patch's content (up to ≈ 3.1 × S for
//! text), so it carries the worst-case multiple 3.6 × S (chosen so that its ceiling clears the measured RLIMIT_DATA need). The patch-plan `file` read is the tree
//! pre-check plus ≈ 3.1 × the file, and the file is bounded by [`CONTENT_MAX_BYTES`].
//!
//! Every cost has a floor of [`COST_FLOOR`]: prikk's own working set, and the reason a ceiling of
//! `cost × 1.5` cannot be smaller than a process needs to start.

use std::path::Path;

/// One mebibyte.
pub const MIB: u64 = 1 << 20;

/// The smallest cost any read is admitted with (and so the smallest ceiling is `COST_FLOOR × 1.5`).
pub const COST_FLOOR: u64 = 16 * MIB;

/// The most a read may cost to be in the **light class** (RFC 011 R-1): reads at the floor (`log`, refs, a
/// `show` of a small patch, a tiny repository's `tree`) draw from the light pool of the budget, everything
/// above from the heavy pool, so a cheap read never queues behind a large one.
pub const LIGHT_CLASS_MAX_COST: u64 = 2 * COST_FLOOR;

/// The ceiling is this many tenths of the cost (15 = 1.5 ×; was 1.25 ×). The ceiling is an address-space
/// limit (`RLIMIT_DATA`), and `verify` of a single-large-file repository needs 0.98 of a 1.25 × ceiling
/// (T5 of the first build); it reserves no budget, so a wider margin costs enforcement tightness only.
pub const CEILING_TENTHS: u64 = 15;

/// The floor for `show`: a 10,000-file patch of small files already needs ≈ 18.8 MB (prikk 0.52.0), above
/// [`COST_FLOOR`], and a store walk cannot see the size of a patch.
pub const SHOW_FLOOR: u64 = 32 * MIB;

/// The most content one response will carry: **one constant for both** the raw route and the API's
/// `file` view (RFC 011 D-3). A larger file is answered `too_large`. The server's own buffer, and prikk's
/// (≈ 3.1 × the file for `file`), are thereby bounded.
pub const CONTENT_MAX_BYTES: u64 = 4 * MIB;

/// The prikk version the cost table was measured on (`MIN_PRIKK_VERSION` when the floor was last raised).
pub const MEASURED_ON: (u64, u64, u64) = (0, 52, 0);

/// The most entries a store walk will visit before giving up (a sane store has about forty files).
pub const MAX_STORE_ENTRIES: usize = 100_000;

/// A read the service performs, for costing. Matched exhaustively wherever a cost is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadVerb {
    /// `log` (history).
    Log,
    /// `show` (a change).
    Show,
    /// `branch` + `tag`.
    Refs,
    /// `tree`.
    Tree,
    /// `cat` — the raw bytes and the metadata read that precedes them.
    Cat,
    /// The API's `file` view: a `tree` pre-check for the file's size, then `checkout --patch-plan`.
    FileContent,
    /// `verify`.
    Verify,
}

/// How a verb's cost is derived: `max(floor, (S × store_tenths + CONTENT_MAX_BYTES × content_tenths) / 10)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostRule {
    /// The prikk version this entry was measured on.
    pub measured_on: (u64, u64, u64),
    /// The multiple of the store size, in tenths (21 = 2.1 ×).
    pub store_tenths: u64,
    /// The multiple of [`CONTENT_MAX_BYTES`], in tenths (31 = 3.1 ×): the bounded content a read holds.
    pub content_tenths: u64,
    /// The smallest cost.
    pub floor: u64,
}

impl ReadVerb {
    /// Every verb (kept in step with the enum by `tests/admission.rs`).
    pub const ALL: [ReadVerb; 7] = [
        ReadVerb::Log,
        ReadVerb::Show,
        ReadVerb::Refs,
        ReadVerb::Tree,
        ReadVerb::Cat,
        ReadVerb::FileContent,
        ReadVerb::Verify,
    ];

    /// The cost rule: **the one table**. No wildcard arm — a new verb must be given a cost here.
    pub const fn rule(self) -> CostRule {
        let (store_tenths, content_tenths, floor) = match self {
            // Flat: ≈ 9.5 MB at any repository size.
            ReadVerb::Log | ReadVerb::Refs => (0, 0, COST_FLOOR),
            // `show` follows the *patch's content*, which no store walk can see: flat (≈ 9 MB) for a patch of
            // small binary files, ≈ 18.8 MB for a 10,000-file patch, ≈ 2.1 × S for one large binary file and
            // ≈ 3.1 × S for text (`txt200`: 642 MB; measured 2026-10-18). Its *address-space* need, which is what the
            // ceiling (RLIMIT_DATA) counts, is ≈ 4.06 × S, so the multiple is 3.6: ceiling 4.5 × S clears it with ~10%.
            ReadVerb::Show => (36, 0, SHOW_FLOOR),
            ReadVerb::Tree | ReadVerb::Cat => (21, 0, COST_FLOOR),
            ReadVerb::Verify => (41, 0, COST_FLOOR),
            // The tree pre-check (2.1 × S) and the patch-plan read, whose content is ≤ CONTENT_MAX_BYTES.
            ReadVerb::FileContent => (21, 31, COST_FLOOR),
        };
        CostRule {
            measured_on: MEASURED_ON,
            store_tenths,
            content_tenths,
            floor,
        }
    }

    /// The estimated peak memory, in bytes, of this read against a store of `store_bytes`.
    pub fn cost(self, store_bytes: u64) -> u64 {
        let r = self.rule();
        let tenths = u128::from(store_bytes) * u128::from(r.store_tenths)
            + u128::from(CONTENT_MAX_BYTES) * u128::from(r.content_tenths);
        u64::try_from(tenths / 10).unwrap_or(u64::MAX).max(r.floor)
    }

    /// A short name for log lines.
    pub const fn name(self) -> &'static str {
        match self {
            ReadVerb::Log => "log",
            ReadVerb::Show => "show",
            ReadVerb::Refs => "refs",
            ReadVerb::Tree => "tree",
            ReadVerb::Cat => "cat",
            ReadVerb::FileContent => "file",
            ReadVerb::Verify => "verify",
        }
    }
}

/// The memory ceiling for one call: `cost × 1.5` ([`CEILING_TENTHS`]), never above `budget` (when there is
/// one) — and never below the cost itself, which the budget check already guarantees fits.
pub fn ceiling_for(cost: u64, budget: Option<u64>) -> u64 {
    let ceiling =
        u64::try_from(u128::from(cost) * u128::from(CEILING_TENTHS) / 10).unwrap_or(u64::MAX);
    match budget {
        Some(b) => ceiling.min(b).max(cost.min(b)),
        None => ceiling,
    }
}

/// Why the store could not be sized.
#[derive(Debug)]
pub enum StoreSizeError {
    /// A directory or file could not be read (including a missing `.prikk`).
    Io(std::io::Error),
    /// More than [`MAX_STORE_ENTRIES`] entries: not a store this server will walk.
    TooManyEntries,
}

impl std::fmt::Display for StoreSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreSizeError::Io(e) => write!(f, "the repository store could not be measured: {e}"),
            StoreSizeError::TooManyEntries => {
                write!(
                    f,
                    "the repository store has more than {MAX_STORE_ENTRIES} entries"
                )
            }
        }
    }
}

impl std::error::Error for StoreSizeError {}

/// The repository's store size **S**: the apparent bytes of every regular file under
/// `<repo_root>/.prikk`, found by a fresh walk. `lstat` only — **symlinks are never followed** and add
/// nothing — and no cache (a cache saves ≤ 65 µs and adds a staleness window: investigation I1). Bounded
/// by [`MAX_STORE_ENTRIES`]; any failure is an error, never a guess.
pub fn store_size(repo_root: &Path) -> Result<u64, StoreSizeError> {
    store_size_limited(repo_root, MAX_STORE_ENTRIES)
}

/// [`store_size`] with an explicit entry bound (so the bound itself can be tested cheaply).
pub fn store_size_limited(repo_root: &Path, max_entries: usize) -> Result<u64, StoreSizeError> {
    let mut total: u64 = 0;
    let mut seen: usize = 0;
    let mut stack = vec![repo_root.join(".prikk")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(StoreSizeError::Io)? {
            let entry = entry.map_err(StoreSizeError::Io)?;
            seen += 1;
            if seen > max_entries {
                return Err(StoreSizeError::TooManyEntries);
            }
            let md = std::fs::symlink_metadata(entry.path()).map_err(StoreSizeError::Io)?;
            let kind = md.file_type();
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                total = total.saturating_add(md.len());
            }
        }
    }
    Ok(total)
}
