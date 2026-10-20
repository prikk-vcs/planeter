# RFC 012 / RFC 003 — Plain-words refinements and an honest label on the change page — Implementation Handoff (v1)

| | |
|---|---|
| Document | Implements the architect's answers to the dev team's questions Q1–Q9 in the review request for [`plain-words-handoff-v1.md`](plain-words-handoff-v1.md), and fixes **IS-44**, a trust-label finding made while answering Q3: the change page labels a sealed patch **Verified**, although no verification runs on that page. A follow-on of the done RFC 003 (T5 honesty, `Assurance`) and of RFC 012 D-4. |
| Status | **Assigned** (2026-10-21). |
| Basis | The plain-words review request (private), Q1–Q9; `crates/planeter-core/src/read.rs` (`Assurance`, `ChangeView::from_show`, `StageView`), `crates/planeter-web/src/ui.rs` (`assurance_label`), `catalog/en.rs`, `templates/explain.html`; RFC 003 T5 and the RFC 005 D-10 honesty type; `docs/glossary.md`; ledger (what `show` reports). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## The decisions (architect; the owner's glossary and philosophy are the authority)

- **Q3 / IS-44 — a label must not claim a check that did not run.** `Assurance::PrikkVerified` means "the repository's
  verification passed with every stage evaluated". The change page gives it to any non-queued patch straight from
  `show`, without running `verify` (the code comment says the repository-level verify is the authority). That is an
  over-claim in the one place the project promised never to blur (SEC-1, WEB-06). **Fix:** a patch that `show` reports
  as sealed (not queued) is labelled **Signed into the history**, a new assurance value, and the word **Verified**
  appears only where a verification actually ran and passed (the repository, history and verification pages).
  The explanation sentence is the dev team's Q3 text.
- **Q1, Q2** accepted as written; on the Verification page itself the second sentence of `explain.failed` points at
  the page you are on, so that page gets a variant (T3).
- **Q4** accepted (a definition, not a claim about this change). The history page's `rollback` tag gets the same
  explanation (T4).
- **Q5, Q6, Q7** accepted as built.
- **Q8:** `repo.location_branch` follows `repo.location` ("Showing {point} · branch {branch}"); `title.too_large`
  follows its heading ("Too large to show here · {path} · {product}"). In-cell tags (`rollback`, `current`, `closed`,
  `folder`) and status words stay lowercase: the rule is written down in the glossary (T6).
- **Q9:** `halted` gets an `after` field exactly as `blocked_by` was added (additive, absent when not applicable), so
  the table has no prikk-side English left.

## Change scope

- **T1 — The assurance value (IS-44).** Add `Assurance::Sealed` (serialized `"sealed"`) in `planeter-core`; `from_show`
  gives it to non-queued patches; `PrikkVerified` is produced only by the repository-level verification path
  (`VerifyView`, the repository and history pages' badge), still only when `ok` and every stage `evaluated`.
  Update the enum's doc comments to say exactly that. Check every place that matches on `Assurance`.
- **T2 — The label.** Catalog key `assurance.sealed` = **Signed into the history**; the UI renders it as a plain
  badge (the neutral style, not the green one: it must never look like the verified badge); the explanation block on
  the change page uses `explain.signed` (Q3's text) for it. The change page no longer contains the word "Verified"
  as a badge for any patch (tests).
- **T3 — Page-specific failure sentence.** On the Verification page the failed explanation reads "prikk checked this
  repository and found problems. They are listed below." (new key `explain.failed_here`); other pages keep
  `explain.failed`.
- **T4 — Rollback on the history page.** When a history page shows at least one `rollback` row, its explanation block
  includes the rollback sentence (one paragraph, once).
- **T5 — Q8 texts and the Q9 field.** The two catalog texts above; `halted` stages carry `after: Option<String>`
  (`skip_serializing_if`), filled from the report's own field (as `blocked_by` is), the page builds "stopped early
  (after {check})" through the catalog (new key) and leaves that row's detail cell empty; `detail` unchanged for API
  compatibility; OpenAPI `Stage.after` (not in `required`) and the drift guard extended as for `blocked_by`.
- **T6 — API, docs, records.** The change endpoint's `assurance` enum in OpenAPI gains `sealed` (and the drift guard
  and the "one plain sentence" tests cover it); CHANGELOG `[Unreleased]`: **the change API's `assurance` for a sealed
  patch is now `"sealed"` instead of `"prikk-verified"`** and the label wording; `docs/glossary.md`: the new row,
  the Q8 rule for tag/status case, status of Q1–Q9 changed to Approved; `docs/messages-and-languages.md` untouched
  unless a rule changed; STATUS IS-44 closed on review.
- **T7 — Tests.** (a) A sealed patch is `Sealed`, a queued one `ForgeApprovedUnsealed`, and no patch ever
  `PrikkVerified` (core unit test over `from_show`); (b) the property-style honesty invariant of `VerifyView` still
  holds unchanged; (c) change page: for sealed + queued + both, the badges and explanations are exactly the expected
  ones and the word "Verified" appears in no badge; (d) the repository, history and verification pages still show
  **Verified** only when verification passed; (e) T3/T4/T5 texts and the `after` field present/absent; (f) retired-words,
  glossary-agreement, structure, stylesheet, catalog and OpenAPI drift checks all stay green (the glossary page and the
  catalog agree for the new and changed keys). Mutations: `from_show` returning `PrikkVerified` again (caught by a),
  the neutral badge replaced by the green one (caught by c).

## Non-change scope

No Japanese; no switcher; no change to the held `verify.sessions` message; no new dependency; no `unsafe`; no
version or release change; no change to `VerifyView`'s logic.

## Required documentation updates

`docs/glossary.md`, CHANGELOG `[Unreleased]` (T6), STATUS (IS-44, IS-30 progress). The architect updates the
threat model T-7/T-6 status wording if the label change touches it (flag in your report).

## Acceptance criteria

T7's tests pass and fail under their mutations; the change page never shows **Verified**; `PrikkVerified` is
constructed only on the verification path (show the `grep`); the API/OpenAPI and docs say `sealed`; every gate
green including both supply-chain gates (fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No relabelling without changing the value (the JSON must tell the truth too); no green badge for `Sealed`; no
verification run added to the change page to "earn" the word (performance and honesty are separate questions; the
page says what it knows); no hard-coded text outside the catalog; no change to a label's meaning other than this one.

## Required evidence and review-request format

Commits (hash → what); every changed key old → new; before/after of the change page for a sealed and a queued
patch (badge and explanation); the `grep` for `PrikkVerified` constructors; the API response before/after for a
sealed patch; mutation table; every gate with command and exit code and the advisory-database HEAD; deviations and
open questions.
