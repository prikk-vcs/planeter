# RFC 011 — Read-path resource model — Investigation Handoff (v1: U1–U3)

| | |
|---|---|
| Document | An **investigation** handoff under the accepted RFC 011 (workflow Phase 1–2; RFC 010 D-8). It settles the three unknowns RFC 011 names before the implementation handoff is written. It measures and builds small proofs of concept in scratch directories; it implements nothing in the repository. Issues IS-36, IS-37. |
| Status | **Assigned** (2026-10-18), to start **after** the ammonia lock-update handoff (IS-38) is returned, since that is on the release path. |
| Basis | [`../../accepted/011-read-path-resource-model.md`](../../accepted/011-read-path-resource-model.md) (D-1, D-2, D-5; unknowns U1–U4); the private measurement report `read-process-model-measurement-review-request.md` and its logs and harness (reuse them); `crates/planeter-prikk/src/sandbox.rs` (`#![forbid(unsafe_code)]` in the crate); ledger PK-51. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |
| Scope | Scratch only. **Non-change scope:** nothing in the repository except the review request; no product code, no manifest change, no commit to `main`. |

## Rows

| ID | Question | What to do and record |
|---|---|---|
| I1 (U1) | **What is a cheap, trustworthy estimate of a repository's size for admission?** | Compare at least: (a) a walk of the `.prikk` store summing file sizes; (b) the same, cached with a time-to-live (try 5, 60, 600 s) and invalidated by the repository's last-change marker if prikk offers one; (c) any size prikk already reports (a verb's output or a file in the store). On the fixtures of the measurement work (1/16/64/200/1000 MiB, 1,000 blocks, 10,000 files): the **cost of each** (ms, n ≥ 200, cold and warm), its **error against the true size**, and whether the **multipliers of PK-51 predict the measured peak memory** within 25% when fed by it. State what a repository that grows during a call does to the estimate. |
| I2 (U2) | **How is a per-call ceiling applied without `unsafe` in `planeter-prikk` and without a hidden prerequisite?** | Evaluate: (a) the `prlimit` wrapper named in `BwrapConfig.timeout`-style composition; (b) another way to set `RLIMIT_AS`/`RLIMIT_DATA` for the child using only safe std/tokio APIs or a crate already in the tree (state the crate and whether `unsafe` lives in it); (c) anything `bwrap` itself offers. For each: does it reach `prikk` (read `/proc/<pid>/limits`), is `prlimit` present in the **container image** and on the supported hosts (check the image's base and the Dockerfile), what the failure looks like when it is absent, and the added latency (n ≥ 200). Recommend one **as a measurement-backed finding, not a design**. |
| I3 (U3) | **How is a child's lifetime tied to the server?** | Reproduce IS-37 (kill -9 the server during a call; the `timeout`/`bwrap`/`prikk` tree survives). Then try, in a scratch harness that uses the production argument vector via the `planeter-prikk` crate by path: (a) the parent-death signal set by the spawner — **test the thread caveat**: spawn from a tokio blocking-pool thread, let that thread exit while the process lives, and show whether healthy children are killed early; (b) a dedicated long-lived spawner thread; (c) a process group or session that a supervisor kills; (d) a wall-time bound implemented inside planeter in place of `timeout`. For each: does a server kill remove the whole tree, within how many seconds; does a **normal** long call still complete; does the wall-time and 5 s kill-after still hold; whether it needs `unsafe` (and where it would live). |

## Ground rules

Same as the earlier measurement handoffs: a new empty directory per fixture; scripts apart from data;
distributions not single runs (n ≥ 200 for latency); host description; negative results are results; stop and
hand back if a measurement contradicts a ledger row marked CONFIRMED; no absolute `cd`.

## Required evidence and review-request format

Per row: commands or harness source (kept in the scratch directory, named in the report), host, the numbers,
and a short "what this says" separating **measured** from **inferred**. End with a table "candidate →
works / fails / needs `unsafe` / needs a new prerequisite" for I2 and I3, and the recommended candidate for
each **with the evidence that supports it**. Do not recommend a design beyond that: the architect writes the
implementation handoff from your findings.
