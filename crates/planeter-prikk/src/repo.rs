//! The [`PrikkRepo`] trait — planeter's typed operation surface over one prikk repository
//! (RFC 001 D-2). Defined as a trait so the implementation is swappable (RFC 001 D-7 / OQ-7:
//! evaluate reusing stikk's `stikk-prikk`); the subprocess implementation lands in T3.
//!
//! Methods are synchronous (each drives the prikk CLI, a blocking subprocess); a concurrent caller
//! wraps them off the async runtime. The trait is object-safe. Read methods return the typed
//! [`crate::model`] structs; exchange methods (bundle/sync — RFC 004 transport) are file-oriented and
//! their stdout parsing is finalized against the binary in T3.

use std::path::Path;

use crate::error::Result;
use crate::model;

/// A handle to one hosted prikk repository (one `.prikk`), driven through the prikk CLI.
pub trait PrikkRepo {
    // -- version / pin (RFC 001 D-3) --

    /// The prikk binary's version string (e.g. `"0.43.0"`, parsed from `prikk --version`).
    fn version(&self) -> Result<String>;

    // -- reads (`--format json`; RFC 003 read path) --

    /// `log` — the sealed ref history. `ref_name`/`limit` are optional filters.
    fn log(&self, ref_name: Option<&str>, limit: Option<usize>) -> Result<model::LogReport>;

    /// `show` — a block/patch's operations and content.
    fn show(&self, target: &str) -> Result<model::ShowReport>;

    /// `verify` — repository verification status (read `verdict.ok`).
    fn verify(&self) -> Result<model::VerifyReport>;

    /// `branch` list. `all` includes closed branches.
    fn branches(&self, all: bool) -> Result<model::BranchList>;

    /// `tag` list.
    fn tags(&self) -> Result<model::TagList>;

    /// `status` — repo/queue status.
    fn status(&self) -> Result<model::StatusReport>;

    /// `worktree-status`.
    fn worktree_status(&self) -> Result<model::WorktreeStatusReport>;

    /// `checkout --patch-plan --content-path` — content of the named paths at `ref_name`.
    fn patch_plan_content(
        &self,
        ref_name: Option<&str>,
        paths: &[&str],
    ) -> Result<model::PatchPlanContent>;

    /// `tree` — the files present at a point (leaves only; planeter builds directories from segments).
    /// `point` is a ref or a bare block id (None = the current branch); `prefix` filters by whole path
    /// components (prikk 0.46.0).
    fn tree(&self, point: Option<&str>, prefix: Option<&str>) -> Result<model::TreeListing>;

    /// `cat --format json` — metadata (no bytes) for a file at a point: `kind`/`encoding`/`size`, so a
    /// caller can decide inline-render vs. download before fetching (prikk 0.46.0).
    fn path_content_meta(&self, point: Option<&str>, path: &str) -> Result<model::PathContentMeta>;

    /// `cat` — the raw reconstructed bytes of a file at a point (text **and** binary). `max_bytes`, if
    /// set, refuses (writing **nothing**) when the content exceeds it. **Note (PK-24):** this bounds the
    /// bytes returned, **not** prikk's memory — bound hostile input by what you accept, upstream of here.
    fn cat_bytes(&self, point: Option<&str>, path: &str, max_bytes: Option<u64>)
    -> Result<Vec<u8>>;

    /// `key status`.
    fn key_status(&self) -> Result<model::KeyStatus>;

    /// `trust maintainer list`.
    fn trust_list(&self) -> Result<model::TrustList>;

    /// `trust maintainer check --key-id`.
    fn trust_check(&self, key_id: &str) -> Result<model::TrustCheck>;

    // -- artifact exchange (files in / files out; RFC 004 transport) --
    //
    // These carry prikk's `bundle`/`sync` artifacts. planeter ferries the bytes; prikk produces and
    // consumes them (RFC 004 INT-2). The `--adopt`/repository-complete-artifact paths (prikk RFC 154/
    // 155) are added when that prikk capability ships (dependency-ledger PK-14/PK-15).

    /// `bundle export --ref <ref>` → write a bundle for one ref to `out`.
    fn bundle_export(&self, ref_name: &str, out: &Path) -> Result<()>;

    /// `bundle import <artifact>` → import a bundle (lands under `remotes/`).
    fn bundle_import(&self, artifact: &Path) -> Result<()>;

    /// `sync summary --output <out>` → this repo's `PSYNCSU1` summary.
    fn sync_summary(&self, out: &Path) -> Result<()>;

    /// `sync have <ref> --output <out>` → a `PSYNCHV1` have-list for one ref.
    fn sync_have(&self, ref_name: &str, out: &Path) -> Result<()>;

    /// `sync build <ref> --have <have> --output <out>` → a `PEXCH002` artifact closing the gap.
    fn sync_build(&self, ref_name: &str, have: &Path, out: &Path) -> Result<()>;

    /// `sync accept <artifact>` → ingest author-signed patches (keyless); returns accepted claim ids.
    fn sync_accept(&self, artifact: &Path) -> Result<Vec<String>>;

    /// `sync pending` → accepted-but-unsealed claim/patch ids.
    fn sync_pending(&self) -> Result<Vec<String>>;

    /// `sync seal <ref> --claim <id>` → seal an accepted claim (needs a maintainer key in this repo;
    /// a keyless forge cannot call this — RFC 001 §0a / dependency-ledger PK-3).
    fn sync_seal(&self, ref_name: &str, claim: &str) -> Result<()>;
}
