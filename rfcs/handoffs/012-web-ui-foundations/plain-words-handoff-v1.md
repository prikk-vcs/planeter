# RFC 012 — Web UI foundations — Implementation Handoff (v1): the plain-word rewrite

| | |
|---|---|
| Document | Implements RFC 012 **D-4** (plain words) and the wording side of D-6, using the owner-approved [`docs/glossary.md`](../../../docs/glossary.md) (sections 1 and 2). D-7 step (4). The structure (landmarks, headings, tables) and presentation are done; the Japanese catalog starts only from the English this handoff produces. Issue IS-30. |
| Status | **Done, reviewed Approved (2026-10-21; commits `fab79ff`, `4bb8cf8`, `1192b14`);** refinements in `plain-words-refinements-handoff-v1.md`. |
| Basis | [`../../accepted/012-web-ui-foundations.md`](../../accepted/012-web-ui-foundations.md) D-4, D-6; `docs/glossary.md` (the table of concepts, the table of messages, certainty column); `docs/messages-and-languages.md`; `crates/planeter-web/src/catalog/en.rs`, `templates/`, `ui.rs`; `crates/planeter-core/src/read.rs` (the verify view); [`message-catalog-handoff-v1.md`](message-catalog-handoff-v1.md) (the rules for keys, placeholders and checks). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The English catalog follows the glossary.** Apply section 2's proposed text to the listed keys and
  section 1's words to the navigation, badges, titles, headings, column headers and status words. Keys stay (so the
  catalog checks keep working); a key is added only where a new sentence is needed (T2, T3). Sentence case
  throughout, including the titles (`History · {repo} · planeter`). **Exceptions, do not change:**
  (a) `verify.sessions` — the glossary's row for the old "session(s)" count is **held** until prikk confirms what
  `active_sessions` means (architect's letter pending); leave the text and its plural forms exactly as they are
  and put a comment on the key saying it is held; (b) operator-facing hints (`content origin`, `read memory
  budget`, `Retry-After`, `OpenAPI`, `GET`) and the API `error.*` texts the glossary marks "unchanged".
- **T2 — Explanations in place (no tooltips, no script).** On each page that shows an assurance badge
  (repository, history, change, verify), directly after the heading, a `<details>` whose summary is a catalog
  sentence ("What does this mean?") and whose body is the badge's detail sentence from the glossary
  (Verified / Approved, awaiting signature / Verification failed / Could not be checked now). On the change page,
  one-line explanations of **patch** ("one recorded edit") and **rollback** ("This change undoes an earlier
  change.") in the same way. All text is in the catalog; the structure checker's rules still pass (the `<details>`
  summary is a native control with an accessible name).
- **T3 — The verification table in plain words.** Column "Check" (was Stage); status words **completed /
  problem found / not run (blocked by {check}) / stopped early**; the findings heading **What was found**, over
  prikk's own ids and messages (kept as prikk wrote them, in a plain list or table with a caption). For the
  "not run (blocked by …)" status the UI needs the blocker's name as **data**: the stage view currently carries only
  the English `detail` string built in `planeter-core` ("blocked by {x}"). Add an **additive** field to the stage
  view/JSON (`blocked_by`, absent when not blocked), keep the existing `detail` unchanged for API compatibility,
  have the UI build its sentence from the field through the catalog, and add the field to the OpenAPI document
  and its drift guard. (This removes the "blocked by" exception from `tests/literals.allow` if it still lists it.)
- **T4 — Retired words are checked.** A catalog check that fails if a **user-facing** message contains any of the
  retired terms: `unsealed`, `forge-approved`, `prikk-verified`, `not_evaluated`, `evaluated` (as a status word),
  `refs` (as a UI label), `SSO` (the abbreviation, in user messages), and the old nav words as labels
  (`code`, lowercase `history`/`verify`) — with an allowlist file for legitimate uses (data owned by prikk, the
  command name in operator text), each entry with a reason. Shown able to fail (planted case + mutation).
- **T5 — Update the glossary page.** Rename the glossary's *Today* column to *Was*, add a *Current* column
  showing the catalog text now in force, remove the "Proposed" status wording, keep the *Certainty* column, and
  list the one held row (`verify.sessions`) as "held for prikk's confirmation". The page and the catalog must
  agree: a small test reads the glossary's *Current* cells for the keys it names and asserts they equal the
  English catalog text (so the page cannot drift).
- **T6 — Tests.** Update the existing tests' expectations (structure checker states, page tests, i18n tests,
  `pages_in_two_languages`, API/OpenAPI drift) to the new words; add tests for T2 (the `<details>` exists, labelled,
  with the right sentence per badge state), T3 (the blocker comes from the field; a stage without a blocker has no
  field), and the glossary/catalog agreement. All earlier checks stay green: catalog completeness and orphans,
  placeholders, `|safe`, literals scan, the 17 structural rules, the stylesheet tests.

## Non-change scope

No Japanese; no language switcher; no structure, CSS or behaviour change except the `<details>` blocks and the new
stage field; no change to any API status or code, or to the wording of API `error.*` texts the glossary leaves
unchanged; no change to the held `verify.sessions` message; no new dependency; no `unsafe`; no version or release
change.

## Required documentation updates

`docs/glossary.md` (T5), `docs/messages-and-languages.md` (a line that wording changes go through the glossary),
CHANGELOG `[Unreleased]` (operator-visible: the interface words changed — list the badges and navigation labels in
one line each; the additive `blocked_by` field in the verify API), STATUS IS-30 progress.

## Acceptance criteria

Every glossary row marked approved is in the catalog or on the held list; no retired word remains in a user-facing
message (T4 passes and fails under its mutation); every page that shows a badge has the explanation block with
the right sentence for each of its four states (tests); the verify table uses the plain status words and the
blocker field; the glossary and catalog agree (test); the OpenAPI drift guard covers `blocked_by`; all earlier tests
and checks green; every gate green including both supply-chain gates (fresh advisory database), both test runs
named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No wording invented beyond the glossary (a sentence the glossary does not give is proposed in the review request as
a question, with the key, not silently chosen); no change to what a trust label means; no tooltip, script or
`title` attribute carrying meaning; no hard-coded text outside the catalog; no edit of the held message.

## Known risks

Rewording touches trust labels: the detail sentences are the part that protects the reader, so their tests assert
the exact meaning (approved vs signed vs failed vs could-not-check) and not only the words; the `<details>` adds a
control to every badge page (the structure checker and the page tests guard it).

## Required evidence and review-request format

Commits (hash → what); a table of every changed key (old text → new text) so the owner can read the result in one
place; the before/after text of the four badge states with their explanation blocks; the T4 check with its planted
failure and mutation; the glossary/catalog agreement test; every gate with command and exit code and the
advisory-database HEAD; questions for any sentence the glossary did not provide.
