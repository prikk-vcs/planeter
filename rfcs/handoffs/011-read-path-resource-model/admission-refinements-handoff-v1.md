# RFC 011 — Read-path resource model — Implementation Handoff (v1): refinements after the first admission build

| | |
|---|---|
| Document | Implements the four decisions the architect took on review of `7475a0d` (private review `admission-and-size-bounds-review-v1`): a reserved slice of the budget for light reads, a ceiling margin of 1.5, the raw route keeping its own 25 MiB cap, and per-route hints. A small follow-up of [`admission-and-size-bounds-handoff-v1.md`](admission-and-size-bounds-handoff-v1.md). Issue IS-36. |
| Status | **Done, reviewed Approved (2026-10-19; commit `f967fae`).** IS-36 closed. |
| Basis | RFC 011 "Refinements after the first implementation" (added with this handoff) and D-1…D-4; the dev team's review request for `7475a0d` (measurements for decisions 1 and 2, the starvation test); `crates/planeter-web/src/{gate,api,errors,ui}.rs`, `crates/planeter-core/src/{admission,read}.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — A reserved slice for light reads.** Split the byte budget into two pools: a **light pool** of
  `max(budget / 8, 4 × the floor)` and a **heavy pool** of the rest. A read whose cost is the **floor class**
  (the verbs/costs at the 16 MiB floor — `log`, refs and the like; define the class by cost ≤ 2 × the floor, in
  one named constant) draws only from the light pool; every other read only from the heavy pool. Each pool is
  its own FIFO. A cost above its **own pool** is `too_large` at once (so the heavy pool's size is the effective
  whole-budget bound for heavy reads: say so in RELEASING). Process slots stay shared and unchanged. Counters
  report both pools (in-flight and high-water per pool). The start-up line states the two sizes.
  **Acceptance:** repeat the starvation scenarios of your review request (the three timed rows and the
  n = 200 race): a `log` never waits behind a waiting `verify` and is never refused `busy` while the light pool has
  room; show the numbers before and after.
- **T2 — The ceiling margin.** `ceiling = min(cost × 1.5, budget)` (was 1.25). Update the constant, the
  `the_ceiling_is_…` tests and the address-space test (`the_ceiling_clears_the_smallest_rlimit_data…`), and
  report the new tightest ratio (expect ≈ 0.82 at `verify` of a single-large-file repository). Do not change
  `4.1`, `2.1`, `3.1` or `3.6`.
- **T3 — The raw bound is unchanged for raw.** Restore a separate `RAW_MAX_BYTES` (25 MiB) for the API raw
  handler and wherever the UI's raw/download path used it before 0.3.0; `CONTENT_MAX_BYTES` (4 MiB) stays for
  `file` and for what the UI file page and the README preview used (1 MiB for the preview, as before). Raw still
  decides from the metadata read and keeps `cat --max-bytes` as a backstop. Check by diff against tag `0.3.0`
  that **no released limit changed** other than `file`'s new bound, and say so in the report. Correct the
  CHANGELOG entry and RELEASING (the raw route is not reduced).
- **T4 — Hints that are true per route.** `too_large` keeps its code, status and message. The **hint** depends
  on the cause: for `file` and for a cost above the budget, today's hint ("Download it from the content origin, or
  ask the operator to raise the read memory budget."); for **raw** above its cap: "This server does not serve
  files that large; ask the operator." Implement as the smallest change that keeps one error code (for example a
  second hint constant chosen at the call site); update OpenAPI's description of the code and the drift/one-sentence
  tests; add a test per cause.
- **T5 — Tests** under `tests/`; each change covered; the mutation evidence for T1 (a read from the wrong pool)
  and T3 (raw reduced again); which tests self-skip.

## Non-change scope

No cost-table change; no new variable (the slice is derived, not configurable); no rate limits or fairness
between tenants; no `unsafe`; no new dependency; no version or release change.

## Required documentation updates

RELEASING (the two pools, what "the heavy pool" means for sizing: `budget × 7/8 ≥ 4.1 × the largest repository you
`verify``), the start-up line, CHANGELOG `[Unreleased]` (correct the raw statement; add the margin and the
reserved slice in operator terms), STATUS IS-36 closed on review. The architect updates RFC 011 and the ledger.

## Acceptance criteria

T1's before/after starvation numbers; T2's tightest ratio; T3's diff against `0.3.0`; T4's per-cause tests; every
gate green including both supply-chain gates (fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No configurable slice; no borrowing between pools in this version (report if you think it is needed); no
change to a released limit; no weakening of a calibration test.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the before/after starvation table; the new
tightest ceiling ratio; the `0.3.0` limits diff; test names and self-skips; deviations and open questions.
