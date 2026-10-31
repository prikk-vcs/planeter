# RFC 012 follow-on — Two sentences that the sign-off made untrue — Implementation Handoff (v1)

| | |
|---|---|
| Document | The review of `japanese-final-handoff-v1` found one catalog text that became false when Japanese was signed off and shipped: `planeter-client`'s `help.main`, English **and** Japanese, still says "`--lang en\|ja` the language of the messages (**Japanese is a draft**)". The previous handoff forbade touching it; this one authorizes exactly that change. |
| Status | **Done, reviewed Approved (2026-10-31; commit `ed43dff`).** (Originally: Assigned 2026-10-31.) |
| Basis | `japanese-final-review-request.md` §6.1; `crates/planeter-client/src/catalog/{en,ja}.rs` (`help.main`); the sign-off record `JA_SIGNOFF`; `tools/ja-sheet` (the sheet lists both rows). |
| Audience | Dev team. Return a short review-request package (`.git-exclude/review-requests/`). |

## Change scope
- **T1.** In `help.main` (English): remove the parenthesis "(Japanese is a draft)" and keep the rest of the line as it is ("`--lang en|ja` the language of the messages"); in the Japanese: remove the corresponding 「（日本語は草案です）」-type parenthesis. Nothing else in either row changes. Update the client help tests that compare the text, regenerate the review sheet and run `--check`; the changed rows are listed in the report with their old text. The `JA_SIGNOFF` record keeps naming the commit of the final corrections; add to the report the new catalog fingerprint (do not edit the record, the architect decides whether a second line is needed).
- **T2.** Grep both catalogs, `docs/` and `README.md` for any other sentence that calls Japanese a draft, a preview or unsigned (`draft`, `preview`, `草案`, `プレビュー`, `未確認`, `PLANETER_PREVIEW_LANGUAGES` outside the one place that explains the variable's new meaning) and list every hit with a decision; fix only documents (not catalog text) and report catalog hits to the architect.

## Non-change scope
No other message; no code; no version or release change.

## Acceptance criteria
T1 as written with the tests; T2's list; every gate green (fmt, floor, pin, clippy, both test runs named, deny and audit 0, ENF-2, doc), CI green.

## Required evidence and review-request format
Commits (hash → what); the changed rows (old → new); the T2 list; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
