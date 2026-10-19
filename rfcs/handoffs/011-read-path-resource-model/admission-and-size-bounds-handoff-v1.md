# RFC 011 — Read-path resource model — Implementation Handoff (v1): admission, `too_large` and size bounds

| | |
|---|---|
| Document | Implements RFC 011 **D-1** (memory-weighted admission), **D-2** (supplying the ceiling to the boundary built in handoff 1), **D-3** (a size bound on `file`), **D-4** (the `too_large` error), **D-6** (counters). The second and last implementation handoff of RFC 011. Issue IS-36. |
| Status | **Done, reviewed Approved (2026-10-18, commit `7475a0d`), with refinements in `admission-refinements-handoff-v1.md` (done, approved 2026-10-19).** Targets the 0.4.0 line. |
| Basis | [`../../accepted/011-read-path-resource-model.md`](../../accepted/011-read-path-resource-model.md) (D-1…D-6 and the resolved unknowns); [`prikk-boundary-ceiling-and-lifetime-handoff-v1.md`](prikk-boundary-ceiling-and-lifetime-handoff-v1.md) (done: `CallLimits`, `prlimit`); the private investigation report `resource-model-investigation-review-request.md` (its I1 tables are your calibration data and fixtures); ledger PK-51 (as corrected), PK-53; `crates/planeter-web/src/{gate,api,errors,openapi,ui}.rs`, `crates/planeter-core/src/read.rs`, `crates/planeter/src/{main,args}.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The cost model, one table.** A `ReadVerb`-keyed table (an `enum` matched **exhaustively**, so adding
  a read verb without a cost does not compile). Cost in bytes from the repository's store size **S** (T2):
  `tree`/`cat`/`checkout`/file-metadata reads **2.1 × S**; `verify` **4.1 × S**; the patch-plan `file` read
  **3.1 × the file's size** plus the tree pre-check's cost (T6); `log`, `show`, refs and the other small verbs a
  **fixed floor of 16 MiB** (`log` measured ≈ 9.5 MB at any size). Each entry carries the prikk version it was
  measured on (`0.52.0`); a test fails when that differs from `MIN_PRIKK_VERSION`, so the table is
  re-measured at each floor change. Add the re-measurement as an output of the re-baseline procedure in
  `docs/UPSTREAM.md` (RFC 010 D-1). Calibrate against the investigation's 29 cases: the model must **never
  under-predict** any of them; report the over-reservation ratios.
- **T2 — The size estimate.** A fresh walk of `<repository>/.prikk` at every admission: `lstat` each file, sum
  apparent bytes, do not follow symlinks, **no cache** (investigation I1: 50–65 µs; a cache saves ≤ 65 µs and
  adds a staleness window). Bounded (stop and treat as unavailable past a sane entry count, say 100,000). A walk
  failure is `unavailable` with one log line, never a guess.
- **T3 — The memory-weighted gate.** Extend the gate (`BlockingGate` and the prikk gate built from it) with a
  **byte budget** kept as a `tokio` semaphore in MiB units, beside the existing process count. A read acquires
  its byte permits and a process slot within **one** bounded wait (the existing
  `PLANETER_PRIKK_QUEUE_WAIT_SECS`); both are released when the job ends (moved into the blocking job, as today,
  so a cancelled request cannot leak them). A cost **above the whole budget** is refused at once with `too_large`
  (never queued); a wait that ends is `busy` + `Retry-After`. **Risk to test:** a FIFO semaphore can park cheap
  reads behind one large waiter. Add a test that shows what happens to a `log` behind a `verify` that is waiting
  for budget; if cheap reads starve beyond the bounded wait, **stop and report** with the measurement and a
  proposal (do not invent a scheduling policy). The auth gate is untouched.
- **T4 — Budget configuration.** `PLANETER_PRIKK_MEMORY_BUDGET_MB`: a positive integer, validated like the
  sibling variables. **Default** (when unset): **one quarter of the memory the process may use** — the cgroup
  limit (`memory.max` of the process's cgroup, v2; `max` means none) if set, otherwise `MemTotal` — **floored at
  256 MiB and capped at 8 GiB**. An **explicit value is never capped**. The start-up log line states the budget,
  its source (`default`, `default, capped at 8 GiB`, `PLANETER_PRIKK_MEMORY_BUDGET_MB`) and the variable to
  change it. Add the variable to `args::USAGE` (the usage/`main.rs` test already guards both directions) and to
  RELEASING's sizing notes with the rule "budget ≥ 4.1 × the largest repository you expect to `verify`".
- **T5 — Supply the ceiling.** `read_blocking` passes `CallLimits { data_ceiling: Some(cost × 1.25) }`, never
  above the budget (handoff 1 added the mechanism). **Verify empirically** that a legitimate call never trips
  its own ceiling on the investigation's fixtures for every verb in the table (RLIMIT_DATA counts private
  writable mappings, not the binary's file-backed pages): report the highest ratio of measured peak to ceiling.
  If it trips, stop and report.
- **T6 — Bound the content `file` returns (D-3).** One named constant (`CONTENT_MAX_BYTES`, today's 4 MiB) used
  by **both** raw and `file`. `file` first learns the entry's size from the tree listing (the entry carries
  `size`) and refuses a larger file with `too_large` **before** calling `patch_plan_content`; the tree
  pre-check is part of the file route's admission cost. If you find a cheaper or more faithful way to bound it
  (state the measurement), report it instead of implementing silently. Raw keeps `cat --max-bytes`; its
  over-cap outcome changes from `unavailable` to `too_large`.
- **T7 — `too_large` (D-4).** New `ReadError::TooLarge` in core and `ErrorCode::TooLarge` in the web crate:
  status **422**, code `too_large`, message "This is too large to serve here.", hint "Download it from the
  content origin, or ask the operator to raise the read memory budget." Added to the OpenAPI document (every
  operation that can return it) and to the drift guard (the code table and the "one plain sentence per code"
  test). The UI maps `TooLarge` to a plain page using the same message (a minimal mapping, not a wording
  project: that belongs to the web-UI RFC, IS-30).
- **T8 — Two pre-existing mappings (handoff 1's deviation 4).** `raw_file`'s metadata read must not turn a
  timeout (status 124) into `NotFound`: timeouts and ceiling hits are `Unavailable`; only a real "no such path"
  is `NotFound`. A test for each.
- **T9 — Counters (D-6).** The gate counts admitted, waited, refused-busy, refused-too-large, in-flight bytes
  and the high-water mark (atomics, read by a test and later by metrics). A refusal logs one line with the
  verb, the cost and the budget (no path, no repository name, no tool output).

## Non-change scope

No rate limits, quotas or per-tenant fairness (C-9a, IS-25); no metrics endpoint; no change to the auth gate;
no `unsafe`; no new dependency (if reading the cgroup limit seems to need one, stop and report; plain file
reads of `/sys/fs/cgroup/memory.max` and `/proc/meminfo` should suffice); no version or release change.

## Required documentation updates

README and RELEASING (the variable, the default rule, `too_large`, sizing), `args::USAGE`, OpenAPI,
CHANGELOG `[Unreleased]` (**a new variable; a new error code; `file` and raw refuse files above 4 MiB with 422;
reads are admitted by estimated memory and may be answered `busy` or `too_large`; the over-cap raw outcome
changed**), `docs/UPSTREAM.md` (re-measure the cost table at each re-baseline), STATUS IS-36 closed on review.
The architect updates the threat model T-9 line; flag anything that should change in it.

## Acceptance criteria

The cost table is exhaustive and matches the floor version; the estimator equals the true size on the
fixtures; a request above the whole budget is `too_large` immediately; a second request waits for budget and is
served when the first finishes; the budget is released on completion, timeout, cancellation and ceiling hit; an
under-sized budget makes the C2 worst case (8 × `verify` of a 1 GiB repository) hold the descendants' memory at
or below the budget (repeat the measurement, report the numbers); the ceiling never trips on a legitimate call
(T5); `file` and raw refuse above the constant with `too_large`; a timeout and a ceiling hit never become
`NotFound`; the OpenAPI drift guard covers `too_large`; every gate green including both supply-chain gates (fresh
advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No cache on the estimator; no best-guess cost when the walk fails; no capping of an explicit budget; no
queueing of a request that can never fit; no inline tests; no changing the cost table to make a test pass
(change it only from a measurement); no `unsafe`.

## Known risks

Over-reservation up to 2.1× for many-small-file repositories (accepted: capacity, not safety); the FIFO
parking of cheap reads (T3); a budget below a deployment's largest repository makes every `verify` of it
`too_large` (the message and RELEASING say which variable to raise); the cost table goes stale with prikk
(T1's test and the re-baseline output).

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the calibration table (29 cases: measured peak,
cost, ratio); the C2 worst-case repeat; the T5 ratio; the T3 starvation measurement; test names and which
self-skipped; one captured `too_large` response (API) and one UI rendering; deviations and open questions.
