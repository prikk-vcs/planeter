# RFC 005 — Change review — Implementation Handoff (v2, part 5a): the landing rule corrected, and what the part 5b/5 reviews found

| | |
|---|---|
| Document | Part 5's open question 1 exposed an **error in the architect's handoff**: it required a proposal's patches to be *absent from `sync pending` and present in a block*. On a keyless forge that can never be true for history that was **received** (prikk 0.54.1: a received block does not clear `sync pending`), and received refs are last-writer-wins and unverified, so they are not evidence that something landed. This handoff corrects the rule (D-40) and the assurance precedence, shows received sightings honestly, and fixes four small things from the part 5 and 5b reviews. |
| Status | **Done, reviewed Approved (2026-10-27; commits `26240cc`…`ccb1335`).** IS-53 closed. |
| Basis | `proposals-read-side-review-request.md` §11 (question 1, the transcript `observation/received-block-does-not-clear-pending.txt`); [`../../accepted/005-change-review.md`](../../accepted/005-change-review.md) D-6 (amended 2026-10-27 by the architect) and D-10; [`../../accepted/008-seal-one-click-merge.md`](../../accepted/008-seal-one-click-merge.md) and the D-8 text in RFC 004 (until the owner decides D-8, nothing here may promise that a proposal lands); ledger PK-67 (received refs), PK-71; `crates/planeter-transport/src/proposals.rs` (`sealed`, `refresh`), `crates/planeter-core/src/change.rs` (`patch_assurance`), `crates/planeter-transport/src/propose.rs` (`patches_in_logs`), `crates/planeter-transport/src/recovery.rs` (`evaluate`), `crates/planeter/src/main.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — The landing rule (D-40)

**A proposal is `landed` when every patch id of its current patch set is present in a block that `log` lists for a *published* ref of the repository (`heads/*` as `branch --all` reports them as branches, not as received refs). `sync pending` is not consulted for landing.** Reasons (put the short form in the module doc and `docs/proposals.md`): a patch can be at once in the queue and in an imported published block; a received ref is replaceable and unverified, so it cannot make a state final (a landed proposal never changes again); on this forge a published ref advances only when a key-holding action puts history there, which D-8 will define — until then a proposal usually stays open and the page says so truthfully.
(a) `refresh`: every **open** proposal is a candidate (no `observe_landing`-over-pending pre-filter); the published refs' logs are read **once per state fingerprint** (cached in the observation, as `sealed` is now); a proposal lands only if its whole set is in that cached set; keep the 100,000-block guard (a log that fills it lands nothing) and the compare-and-set. Keep the cost test: ten open proposals, many requests, one read of the published logs per state; no `log` when no proposal is open.
(b) `sealed()` reads **published refs only**. Add `received_sightings`: for each open proposal, the received refs (name only) whose logs contain **any** of its patches, found from the received refs' logs of the same observation (read only when an open proposal exists; same 100,000-block guard; a failure means "no sightings", never an error). Do not use sightings for any state.
(c) `patch_assurance` precedence: **a patch in a block of a published ref is `Sealed` even if `sync pending` still lists it**; `listed_pending` without such a block is `ForgeApprovedUnsealed`; neither is `None`. (This reverses the part 3a decision "pending wins", which assumed one store never had a patch both queued and sealed; imports break that assumption.) Update `docs/proposals.md`, `docs/changes`-era comments, the model tests (the 8-combination test of 3a changes by design; say so) and `patches_in_logs` in `propose.rs`: it keeps serving the declaration's *held* check over **all** refs, but the **assurance** of the propose response uses published refs only.
(d) The pages and API: the detail shows, under Details only, "Also found in history received here (not verified here): `<ref>`" for each sighting, with the plain explanation that a received history can be replaced; the JSON has `received_sightings: [{ref}]` (OpenAPI, drift guard). The default view is unchanged. The *landed* sentence stays "Landed: a maintainer has signed this work into the project's history." only for a proposal that landed by the new rule; the glossary row for *landed* drops "and received here".
(e) Tests: the stand-in test `a_patch_in_a_block_of_a_received_ref_counts…` is **replaced** by one that shows a patch in a received ref's block does **not** land the proposal, yields a sighting, and leaves the patch `forge-approved-unsealed`; a published-ref test (stand-in) lands it with pending still listing the patch and shows the patch `Sealed`; the real-prikk test stays (forge-side `sync seal` with the client's key makes a published block — say in a comment that this models "a key-holding action published it"), plus a **real-prikk** test of the sighting: `archive import` (no `--adopt`) of the maintainer's sealed history into the forge → proposal open, sighting present, `sync pending` still lists the patch (this is the transcript of the question, now pinned). Mutation: the published-only filter removed is caught; the pending check re-added is caught.

## T2 — Closed reason in the default view

A closed proposal shows its reason **beside the state** in the default view, one line ("Closed: *reason*"), escaped, truncated for display at 200 characters with the full text under Details (this overrides deviation 1 of the part 5 review request). Tests and both-language catalog texts as before; the checker must stay at zero violations.

## T3 — Doctor's "unreached object" warnings must not keep recovery from running

(Part 5b §3.) prikk 0.54 reports unreached objects as a **warning** (`PRIKK-DOCTOR-UNREACHED-OBJECT`, recommendation "the bytes stay; no command removes them yet") and `verify` still exits 0. In `evaluate`, a `doctor` report whose **only** non-info problems are warnings of exactly this code, with no named repair, is **not** a blocker: it is recorded in the plan as a note (`notes`, shown in the administrator's `GET …/recovery`, with the count) and in the recovery record's `doctor` step text. Any other code, any error, or this code beside another problem keeps `doctor_problems` as today. The test that pinned today's behaviour (`doctors_unreached_object_warnings_keep_recovery_from_running_today`) is changed on purpose into the new rule, plus: an unknown warning code still blocks; the same code beside a repair-naming warning stays an administrator's decision; an unattended run on a store with only these warnings recovers. Catalog text for the note (English and Japanese draft row in the next addendum). `docs/propose.md` runbook: one paragraph.

## T4 — A prikk below the floor is refused at start-up

(Part 5b deviation 1.) `main.rs` probes `prikk --version` at start (with `prlimit`), refuses to start below `MIN_PRIKK_VERSION` with the pin's sentence and exit 1, and the misleading comment is corrected. A test with a stand-in `prikk` answering 0.53.0 (binary-level, like the start-up line tests). A missing `prikk` is reported as today.

## T5 — Smaller items

(a) `docs/propose.md` "Limits worth knowing": the one honest sentence that **a proposal does not move a branch and, until the maintainer's history is published here, will usually stay open** (it is true under every D-8 answer). (b) The retention 410 keeps "kept until … and have since been removed" (confirmed). (c) The environment variables `PLANETER_RETENTION_SWEEP_MINUTES` and `PLANETER_PROPOSAL_GRACE_DAYS` keep refusing 0 (confirmed: a sweep that cannot be switched off protects the disk). (d) The service-level compare-and-set test is **not** wanted (the store-level test suffices).

## Not here (decided, held)

The wording of the owner-approved badge **"Approved, awaiting signature"** and of `explain.queued` ("This change was approved on this server…") overclaims for a proposal (nobody has approved it) and uses *change* for a patch; the architect asks the owner first and issues the rewording with its Japanese rows as its own small handoff. Do not touch either text now.

## Non-change scope

No comments or decisions (part 6); no file-level view; no change to the propose route, quotas or recovery beyond T3; no new dependency; no `unsafe`; no version or release change.

## Acceptance criteria

T1's tests including the three negative ones (received block, pending-only, merely absent from pending); the real-prikk sighting test; T2–T4 tests; the pages still pass the WCAG checker and the plain-words tests in both languages; the IS-14 table, OpenAPI drift guard, ENF-2 and every earlier check pass; every gate green **except** the two supply-chain gates while `advisory-crypto-bigint-tokio-handoff-v1.md` is open (report their exits as they are, on a fresh database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No landing from a received ref; no landing from absence in `sync pending`; no state derived from a sighting; no assurance from `queued`; no hard-coded user-visible text; no touching the two owner-approved texts.

## Required evidence and review-request format

Commits (hash → what, by task); the replaced and new tests with their transcripts; the HTML of a proposal with a sighting (Details) and of a closed one (default view), both languages, with the checker's result; the cost test showing one read of the published logs per state; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
