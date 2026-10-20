# RFC 003 follow-on — A failed verification is shown as a failed verification — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 003 (T5: the repository's verify status is the honesty anchor, always re-derived) and RFC 001 D-3 (the prikk boundary). It fixes **IS-39**, found by the accessibility baseline (finding 2): `prikk verify` exits **1** when verification fails and prints the failing `verify-report-v1` on stdout, but the boundary treats any non-zero exit as an error, so a repository whose verification **failed** is shown as *Temporarily unavailable* ("could not be read right now") and the repository badge says *verify unavailable*. The `unverified` badge and the failed-conditions table can never be produced by real prikk. |
| Status | **Assigned** (2026-10-21). |
| Basis | `crates/planeter-prikk/src/cli.rs` (`verify`, `run_json`, `trust_check` — which already handles exit 1 with a document — and `failure`/ceiling classification from RFC 011), `crates/planeter-core/src/read.rs` (`VerifyView::from_verify`, `ReadService::verify`, the repository page's use), `crates/planeter-web/src/ui.rs`; the baseline review request (private, finding 2 and `states-html/verify-bad`, `verify-unverified` with the three real corruptions and the stand-in that returns the real failing stdout); ledger PK-50 (a keyless forge's `verify` exits 1 on publication-trust issues until keys are adopted); RFC 011 D-2 (a ceiling hit is never a verification verdict). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Why this matters (verified facts and inference)

*Verified (baseline):* real prikk exits 1 with the report on stdout for three different corruptions; the UI then
says "unavailable". *Inferred:* the same happens for PK-50's "publication-trust issues" on a keyless forge once
history is received (RFC 004/005), so the wrongly worded outage would become the normal state of a repository.
It is **not** unsafe (nothing is shown as verified), but it is dishonest in the other direction: it hides the one
fact the user needs, and tells them to retry.

## Change scope

- **T1 — The boundary.** `verify()` reads its result like `trust_check` does: success → parse; **exit status 1
  whose stdout parses as `verify-report-v1` with `verdict.ok == false`** → return that report as `Ok`. Any other
  failure stays an error via the existing `failure` path. **A ceiling hit must stay a ceiling hit:** the RFC 011
  classification (exit 134 / allocation failure / `verify` exit 1 with "did not complete every stage") is checked
  **first**; only a run that is not a ceiling hit may be read as a verdict. A report whose stages are not all
  evaluated is **not** a verdict (it is the partial-report case) and stays an error. A timeout stays an error.
- **T2 — The read service and the pages.** `ReadService::verify` and the repository/history pages receive the
  report and render the existing `unverified` badge and failed-conditions table; the repository page keeps
  rendering without `verify`-dependent parts when verify itself is *unavailable* (budget refusal, spawn failure),
  as tested today. Nothing else on those pages changes; no wording change (RFC 012 owns wording).
- **T3 — Tests** (under `tests/`, real prikk where possible, stand-in where not):
  - with real prikk: for each of the three corruptions from the baseline (trust-policy container, patch container,
    block container; take the fixture builders from the baseline's `scripts/fixtures.sh`), `/verify` is `200` and
    shows `unverified` with its failed conditions; the repository badge is `unverified`; **none of them says
    "Temporarily unavailable"**;
  - with a stand-in prikk: exit 1 with a valid failing report → `Ok(not ok)`; exit 1 with garbage, empty output or
    another schema → error; exit 1 with a **partial** report (stages not evaluated) → `CeilingExceeded` when a
    ceiling was set and an error otherwise, **never** a verdict; exit 134 → `CeilingExceeded`; exit 124 (timeout)
    → error; exit 0 with `verdict.ok == true` → `ok`;
  - the honesty invariant (property-style, small): no input ever yields the `ok` badge unless the report's verdict
    is `ok` **and** every stage is evaluated;
  - the API: `/api/v1/…/verify` returns the report with `ok: false` and status `200` (the resource is readable;
    the verdict is its content). Check what the API does today for the same fixture and state the change.
- **T4 — Mutation evidence** for the three decisive branches: exit 1 not read as a verdict; the ceiling check
  after the verdict read; the partial-report guard removed.

## Non-change scope

No wording change; no new error code; no change to the cost table or the gate; no `unsafe`; no new dependency; no
version or release change. Do not change how any other verb treats exit 1.

## Required documentation updates

CHANGELOG `[Unreleased]` (**a repository whose verification fails is now shown as unverified, with the failed
conditions, instead of as temporarily unavailable**), the `cli.rs` doc comment, STATUS IS-39 closed on review. The
architect updates the dependency ledger.

## Acceptance criteria

T3's tests pass and fail under T4's mutations; with real prikk the three corruptions render `unverified`; the RFC
011 ceiling tests still pass unchanged; every gate green including both supply-chain gates (fresh advisory
database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No parsing of stderr to guess a verdict; no treating exit 1 as a verdict without a parsed, fully evaluated report;
no mapping a partial report to a verdict; no weakening of the RFC 011 ceiling tests; no UI wording edits.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code and the advisory-database HEAD; for each corruption,
the exit code and the first lines of the real stdout; the API response before and after; test names and
self-skips; mutation table; deviations and open questions.
