# RFC 010 / IS-72 — a test that reads an order off a clock: `a_confirmation_that_takes_longer_than_the_wait_answers_202_and_is_polled_to_done` — Implementation Handoff (v1)

| | |
|---|---|
| Document | Found by the dev team in the gate-cost measurement (`gate-cost-measurement-review-request.md`, "A finding in the logs"): under an outside load of 67, `deliveries_http.rs:1067` saw the state `confirmed` where it expected `reaccepting`; it passed in the six other runs. It is a test of the IS-69 kind: the order of two states is read from a clock (the test polls and expects to catch a transient state), so a slow host makes the transient state pass before the poll. A release run is exactly where this must not fail. Not touched by the dev team (non-change scope); assigned now (SCHEDULE D-140). |
| Status | Assigned 2026-11-26. Small; before `docs/accounts.md`. |
| Basis | `crates/planeter-web/tests/deliveries_http.rs` around line 1067; `wall-clock-tests-handoff-v1` and its review (the principle: assert the order, hold the state with a latch the test releases, not a sleep); `crates/planeter/src/failpoint.rs` and the engine's failpoints (debug builds only) if they give a way to hold the delivery in `reaccepting`; `docs/propose.md`/`docs/deliveries.md` for what the states promise. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Make the test hold the state.** The property it holds is: *a confirmation that takes longer than the wait answers 202 and is then polled to `done`* — the **intermediate state is observable while the work is held**, and the end state is reached when it is released. Rewrite it so the delivery is **held in `reaccepting` by a latch the test controls** (a stand-in prikk that blocks until the test lets it go, or an existing failpoint if it is available in the debug test build; say which and why), asserts `reaccepting` **while held** and `done` after release, with a generous guard (60 s) that fails with what was last seen. **Do not weaken the assertion** and do not remove the wait-longer-than-the-wait behaviour (the 202 at the edge) from what it checks.
2. **Look for the same shape.** Search `deliveries_http.rs`, `delivery_real_server.rs`, `delivery_real.rs`, `delivery_kill.rs` and the proposal and creation tests for polling loops that expect to **catch a transient state** (`reaccepting`, `staged`, `checking`, `queued`, …) without holding it; list each hit with a decision (hold it with a latch / it is already held / not an order assertion). Fix those that read an order off a clock the same way; report the rest.
3. **Evidence under contention, not a count:** run the changed tests **with the confined load** (`tools/repeat-under-load.sh --confine 4 --workers 16`, 20 runs: a small, targeted use as Amendment 1 principle 3 says) before and after: the old test is expected to fail at least once under it, the new one 0 of 20; if the old one does not fail, say so (do not claim it was caught). A mutation (the work released too early) fails the new test.

## Non-change scope
No product code, no change to a state's meaning, no new dependency, no change to the other tests' assertions beyond item 2's list. No tag, release or upload.

## Acceptance criteria
The test holds `reaccepting` with a latch and asserts it; every hit of item 2 decided; the evidence of item 3; gates green (the review tier of the new gates script if it exists by then, else the usual set); CI green on all eight jobs.

## Required evidence
Commits (hash → what); the old and new test; the item 2 table; the confined runs' results before and after; the mutation; the gates; deviations and open questions.
