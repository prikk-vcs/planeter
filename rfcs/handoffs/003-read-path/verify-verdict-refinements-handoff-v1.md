# RFC 003 follow-on — Verification verdict refinements — Implementation Handoff (v1)

| | |
|---|---|
| Document | A small follow-on of [`verify-failure-honesty-handoff-v1.md`](verify-failure-honesty-handoff-v1.md) (done, approved 2026-10-22), taking the architect's decisions on the dev team's two questions in its review request. Issue **IS-43**. |
| Status | **Assigned** (2026-10-22), after the advisory handoff (IS-42) which clears the red CI. |
| Basis | The dev team's review request for IS-39 (decision 1 and its second question; `real-corruptions.txt`, `other-real-corruptions.txt`, `ceiling-sweep.txt`); `crates/planeter-prikk/src/cli.rs` (`verify`, `is_complete_verdict`, `is_resource_failure`); RFC 011 D-2; ledger PK-50, PK-51. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Decisions already taken (for context)

1. The verdict is read **from the report on stdout, not from stderr text** (stderr is identical for a ceiling hit and
   for genuine damage). Accepted as built.
2. A failed stage that says the process ran out of memory is a ceiling hit, not a verdict. Accepted, **with the
   tightening in T1**: a stage message can embed data the repository owner controls (a ref or path name), so a
   bare substring match could be steered.
3. A genuine failure whose later stages are `not_evaluated` because an earlier stage failed with a finding is a
   real verdict shape (ref pointer index damage: 3 failed + 2 blocked). It should be read as a verdict **if the
   report itself says so structurally** (T2).

## Change scope

- **T1 — Tighten the resource-failure match.** `is_resource_failure` must match only the shape measured in the
  ceiling sweep: a `Failed` stage whose message **begins with** `i/o error:` (prikk's error class prefix) and then
  names the out-of-memory condition. Measure on prikk 0.52.0 and report the exact messages seen for (a) a ceiling
  hit on the stage that fails (`ceiling-sweep.txt`), (b) the allocator-abort route (exit 134, unchanged), and
  (c) genuine damage whose message *contains* the phrase because a name does (plant a ref or file named
  `out of memory` in a corrupted fixture and show the result is a verdict, not a ceiling hit). Add that case as a
  test. If a real ceiling hit produces a message that does not start with `i/o error:`, stop and report.
- **T2 — Blocked stages, only if the report says so.** Inspect the `verify-report-v1` JSON of the ref-pointer-index
  corruption and of the blocked stages: does a `not_evaluated` stage carry a machine-readable reason or the id of
  the stage that blocked it (not text parsing)? **If yes:** a `not_evaluated` stage whose reported blocker is a
  stage of the same report that is `failed` with a non-resource message is part of the verdict; every other
  `not_evaluated`/`halted` stage still makes the report partial. Add real-prikk tests for the ref-pointer-index
  case (verdict `unverified`, with and without a ceiling) and stand-in tests for: blocker is itself `not_evaluated`;
  blocker missing from the report; blocker failed with a resource message; unknown blocker id — all partial.
  **If no** such structure exists in 0.52.0's report, make no change, record the report shape in your review
  request, and the architect will ask prikk (the dependency ledger row is mine).
- **T3 — Keep the honesty invariant.** The property-style tests of the original handoff stay and must still pass
  unchanged; `PrikkVerified` still requires `ok` and every stage `evaluated`. Extend the exhaustive `VerifyView`
  test if the model gains a field.
- **T4 — Mutation evidence** for T1's prefix requirement (remove it → the planted-name test fails) and T2's blocker
  check (accept any `not_evaluated` → a stand-in test fails).

## Non-change scope

No wording change; no new error code; no other verb's handling of exit 1; no `unsafe`; no new dependency; no
version or release change.

## Required documentation updates

The `cli.rs` doc comments, CHANGELOG `[Unreleased]` only if an operator-visible fact changes (a damaged
ref-pointer-index repository shows `unverified` instead of unavailable, if T2 applies), STATUS IS-43 closed on
review. The architect updates the dependency ledger (the report shapes, and a re-baseline output: re-measure the
resource-failure message at each prikk re-baseline, as the cost table is re-measured).

## Acceptance criteria

T1's planted-name test passes and fails under its mutation; T2 either passes its tests and mutation or the review
request contains the measured report shape and states no change; the original IS-39 tests pass unchanged; every
gate green including both supply-chain gates (fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No stderr parsing; no text match on user-influenced content without the prefix and shape check; no verdict from a
partial report; no weakening of the RFC 011 ceiling tests.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code and the advisory-database HEAD; the measured
messages for T1(a–c); the report JSON excerpts for T2; test names and self-skips; mutation table; deviations and
open questions.
