# RFC 005 / RFC 004 — Implementation Handoff (v2): the 0.55 re-baseline — two new refusal ids, the `sync accept` number prikk asked for, and what to re-validate

| | |
|---|---|
| Document | prikk 0.55.0 is out (letter of 2026-10-29, ledger PK-73). For planeter it changes little by design, and asks one thing of us: the `sync accept` number on 0.55.0. This handoff **re-baselines** (suite and re-validation on 0.55.0, the floor stays 0.54.0), teaches the `extract` driver the two ids 0.55.0 adds, and **re-measures the accept** (faster writes may change the per-object cost that RFC 004 D-5.4's object bound and deadline are set from). No UI, no new feature. |
| Status | **Done, reviewed Approved (2026-10-29; commits `8e60b97`, `bf24297`).** (Originally: Assigned 2026-10-29. **Precondition: the owner installs prikk 0.55.0 on this machine** (`prikk --version` must say 0.55.0; the architect will say so in the handoff message). Do it **after** the IS-55 delivery (`driver-output-cap-handoff-v1.md`) so the two do not share a tree. |
| Basis | Ledger PK-73 and PK-70/PK-71; prikk's letter `.git-exclude/upstream/prikk/receive/2026-10-29-0-55-0-is-out-and-our-answers-to-your-two-letters.md`; [`prikk-0-54-rebaseline-and-extract-driver-handoff-v2.md`](prikk-0-54-rebaseline-and-extract-driver-handoff-v2.md) (the driver, the 13 ids, `ClaimParentNotSealedOrDamaged`); RFC 004 D-5.3/D-5.4/R-3 (as amended), `cost_model` in `propose.rs`, `CostRule` in `admission.rs`, `MEASURED_ON`; ledger PK-68 (one flush per object), PK-69. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — Re-baseline

On prikk 0.55.0: run the whole suite (with and without prikk on `PATH`) before changing anything and report any drift test by test (expected: none). The floor **stays 0.54.0** (nothing in 0.55.0 is required; a floor bump is a decision of the architect when a feature we need arrives, expected 0.56.0). Add 0.55.0 to the stand-in prikks' version answers only where a test needs "newer than the floor". Re-validate the seven read verbs and `extract` as the 0.54 re-baseline did (the same script, ceilings at 1.5×) and report any row that moved by more than 5 %; update `MEASURED_ON` to (0, 55, 0) only if the table test still holds.

## T2 — The extract driver learns two ids (the driver changes; compatible with 0.54.x)

0.55.0 adds the refusal ids **`claim-parent-not-sealed`** (a claim whose parent is still pending; names the claim and its parent; replaces `damaged-repository` for that case) and **`destination-inside-repository`**. Add both to the closed enum (the list of ids is then 15, with the catch-all `Other` unchanged for anything unknown). Behaviour: `claim-parent-not-sealed` becomes its own outcome `ClaimParentNotSealed { claim, parent }` (parse the two ids from the message **only if** `refusal-v1` carries them as fields — look; if they are only in the free-text message, keep the text verbatim and do not parse it); the 0.54.x mapping of `damaged-repository` for a **claim** to `ClaimParentNotSealedOrDamaged` stays (the floor is 0.54.0) and its doc comment says "0.55.0 and later report `claim-parent-not-sealed`". `destination-inside-repository` is unreachable by construction (the driver never passes such a destination) and is a named driver bug-class error if it ever appears. Provoke both on the real 0.55.0 (a pending-parent claim as in the 0.54 test; a destination inside a throwaway repository, raw, not through the driver) and add the tests (they skip with a visible reason below 0.55.0). Update `docs/extract.md`.

## T3 — Measure `sync accept` on 0.55.0 (prikk asked for the number)

In your own scratch area, under a memory-limited scope (`systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0`), with real prikk 0.55.0 and the **same method as part 4a/4b**: artifacts of 1,000, 2,500, 5,000 and 10,000 one-file patches on a store that starts empty, and once more at 5,000 on a store holding 5,000 pending; one 100 MiB blob; report wall time, ms per object, peak `VmData` and resident set, **with host load recorded for every row** and a quiet-disk reference run (nothing else of yours or the machine's running: say how you know). Compare with 0.54.1 **run again in the same session** (keep a 0.54.1 binary: `cargo install prikk --version 0.54.1 --root <scratch>`, or the release asset), not with the numbers of part 4b: the disk and load differ between days. Report the ratio and whether the object bound (5,000) and the 600 s deadline still have the margin RFC 004 R-3 asks for (a third of the deadline at the loaded end). **Do not change any default**; the architect sets them from the table. A short letter to prikk with the number is the architect's.

## Non-change scope

No change to the cost models or defaults, to authorization, the HTTP surface or any page; no floor change; no new dependency; no `unsafe`; no release.

## Acceptance criteria

T1's drift report (or "none"); T2's tests green on 0.55.0 and skipping visibly on 0.54.1; T3's table with load recorded and the same-session 0.54.1 comparison; all earlier checks pass; every gate green except the two supply-chain gates while IS-54 is open (report their exits as they are, on a fresh database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No default changed from a loaded-host number; no 0.54.1 comparison taken from an earlier day; no parsing of free text for ids; no unmemory-limited measurement run.

## Required evidence and review-request format

Commits (hash → what, by task); the drift report; the provoked-id transcripts; the T3 table with load and the same-session baseline; scripts' location; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
