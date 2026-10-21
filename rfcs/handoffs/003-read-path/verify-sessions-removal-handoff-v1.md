# RFC 003 follow-on / RFC 012 — Remove the "sessions" count from the verification view — Implementation Handoff (v1)

| | |
|---|---|
| Document | Resolves the **held** message `verify.sessions` of the plain-words work, and the API fields behind it, on prikk's answer of 2026-10-21 (ledger PK-62): `active_sessions` in the verify report is **not** a tally of work. `count` is the number of session folders (always exactly one, `default`, in every repository prikk creates); `verified_count` is a **fixed 1**, "not computed", a scope statement for a future with several sessions. "N of M sessions verified" therefore says nothing true about anything and, next to a green badge, suggests progress that does not exist. Issue **IS-49**. |
| Status | **Assigned** (2026-10-21). |
| Basis | prikk's letter `2026-10-21-reply-active-sessions-is-not-waiting-work.md` (private) and ledger PK-62; the dev team's own data points (the 0.53.0 re-baseline: three queued commits ⇒ `{count 1, verified_count 1}`; A12: pending patches are not "sessions"); `crates/planeter-core/src/read.rs` (`VerifyView`: `verified_sessions`, `total_sessions`), `crates/planeter-web/{templates/verify.html,src/openapi.rs,src/catalog/en.rs}`, `docs/glossary.md` (the held row), `tests/glossary.rs`, `tests/catalog_checks.rs`, `tests/i18n.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Decision (architect; it follows the owner's philosophy: a number that cannot be true must not be shown, and less is more)

Remove the count everywhere planeter exposes it — the page, the JSON of the verify endpoint and the OpenAPI
document — instead of rewording it. prikk advises "say nothing about sessions" for readers and offers to review
wording if we want to state what `verify` covers; we do **not** add a scope sentence in this handoff (a new
trust-adjacent sentence goes to prikk for review first). The count of *waiting* work, if it is ever wanted, comes
from `prikk status --format json` (`queue.count`, `queue.target_ref`), a different field with a different meaning,
and is a separate design (RFC 005/queued-state), not this handoff.

## Change scope

- **T1 — The view and the API.** Remove `verified_sessions` and `total_sessions` from `VerifyView` (and from the JSON
  `GET …/verify` body). Keep deserializing prikk's `active_sessions` in `planeter-prikk` (the driver models prikk's
  document; the field is simply no longer used by the view). **This is a breaking change to the verify endpoint's
  success body** (two fields removed); acceptable before 1.0 and recorded in the CHANGELOG.
- **T2 — OpenAPI and its drift guard.** `VerifyView` loses the two properties (and `required` entries); the drift
  guard (`every_view_serializes_to_exactly_the_declared_fields`) and the "properties and required agree" test follow.
- **T3 — The page.** `verify.html` drops the sentence built from `Key::VerifySessions`; the paragraph keeps
  `verify.intro` only. Remove the catalog key `verify.sessions` (it is the plural message of the held row), its
  `Key` variant, its English entry and every reference (`catalog/mod.rs`, `en.rs`, the tests that name it:
  `catalog_checks.rs`, `i18n.rs`, `glossary.rs`). Plural machinery stays (it serves `history.patch_count`); if a
  test used `verify.sessions` as its only plural example, move it to `history.patch_count`.
- **T4 — Glossary.** In `docs/glossary.md` the held row `verify.sessions` becomes **Removed** with the reason in
  one sentence (prikk: always one queue; `verified_count` fixed at 1); the glossary test no longer expects "exactly one
  held row"; the concept row "Queued, unsigned work counted by `verify`" is rewritten: *not counted by `verify`; the
  number of waiting commits is `status`'s `queue.count`, not used today*. The retired-words check gains `session`
  (as a UI word) with the sign-in "session" **not** on the list: check that no user-facing message uses "session"
  for the verify concept; the sign-in session never appeared in a user message after the plain-words rewrite — if it
  does anywhere, report.
- **T5 — Tests.** The verify page and API no longer contain the removed fields/sentence (a negative assertion each);
  all earlier checks stay green (structure rules over the 61 states, retired words, glossary agreement, catalog
  completeness/orphans, OpenAPI drift). A mutation: re-adding the sentence to the template fails a test.
- **T6 — Records.** CHANGELOG `[Unreleased]` (**the verify endpoint no longer returns `verified_sessions` and
  `total_sessions`; the verification page no longer shows "N of M sessions verified"**), STATUS IS-49 closed on
  review. The architect updates the ledger and the design set.

## Non-change scope

No new sentence about what `verify` covers; no use of `status`/`queue.count`; no change to the verdict, the stages, the
badge or the explanation blocks; no dependency change; no `unsafe`; no version or release change.

## Acceptance criteria

The two fields and the sentence are gone from page, JSON and OpenAPI; the key is gone from the catalog and nothing
orphaned remains; the glossary page agrees with the catalog; every gate green including both supply-chain gates
(fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No rewording instead of removal; no leaving the API fields "for compatibility" with a different meaning; no new
catalog text; no weakening of the glossary or catalog checks to accommodate the removal beyond T3/T4.

## Required evidence and review-request format

Commits (hash → what); before/after of the verification page and of the verify API body; the list of references
removed; the mutation; every gate with command and exit code and the advisory-database HEAD; deviations and open
questions.
