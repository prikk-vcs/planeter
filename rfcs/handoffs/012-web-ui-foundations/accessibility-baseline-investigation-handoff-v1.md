# RFC 012 — Web UI foundations — Investigation Handoff (v1): the accessibility and language baseline

| | |
|---|---|
| Document | An **investigation** handoff under the accepted RFC 012 (workflow Phase 1–2; RFC 010 D-8). It measures the current UI against RFC 012 D-2, D-3, D-5 and D-6 and settles unknown U1. It builds small proofs of concept in scratch directories; it changes nothing in the repository. Issues IS-27, IS-30. |
| Status | **Assigned** (2026-10-20). |
| Basis | [`../../accepted/012-web-ui-foundations.md`](../../accepted/012-web-ui-foundations.md) (D-1…D-8, U1…U3); the architect's UI/API audit findings in `docs/STATUS.md` (IS-27, IS-30); `crates/planeter-web/templates/*.html`, `static/app.css`, `ui.rs`, `errors.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`) with the raw outputs. |
| Scope | Scratch directories only (under `.git-exclude/tmp/<name>/` or `/tmp`, removed when finished). **Non-change scope:** nothing in the repository except the review request; no product code; no manifest change; no commit to `main`. Install tools only in the scratch directory, name each tool and version, and treat their downloads as untrusted. |

## What this baseline can and cannot say

No person who uses assistive technology is available to the project (RFC 012 O-3). This baseline therefore
measures what tools can and **says so in the report**: every finding is labelled *tool-measured*, and the report
ends with a list of what a human pass would still have to check. Do not describe anything as "screen-reader
tested" unless a person using a screen reader did it.

## Rows

| ID | Measure | Record |
|---|---|---|
| B1 | **Render every page and state.** Using the existing test harness or a running server with the test fixtures, save the HTML of every page and error state the UI can produce: home, `/go` with good and bad input, login (fresh, failed, locked, CSRF failure, SSO on and off), repository (readable, no README, verify ok / warn / bad / unavailable), history, change, file (text, binary, opaque, too large, missing), tree/listing, refs, verify, not found, unavailable, busy, too large, 405. | The list of states, the saved HTML, and any state you could not produce (and why). |
| B2 | **Structure checks, by script.** Over B1's HTML: number of `h1` per page; heading-level skips; landmarks present (`header`, `nav` with an accessible name, `main`, `footer`); a skip link first; every form control's accessible label; `alt`/text alternative for every image or icon (emoji count as text that screen readers read aloud: list each); `lang`; `<title>` uniqueness and pattern; tables with headers; links whose text is not descriptive ("here", a bare id); any element with `aria-hidden` that is focusable. | A per-page table of pass/fail with the offending element; counts per rule. |
| B3 | **Contrast, by script.** Parse the CSS custom properties and the rules that use them; compute the WCAG contrast ratio of every foreground/background pair actually used (text, large text, link, muted text, borders and focus indicators, badges, buttons, form fields) in the **light** and the **dark** theme, and in the forced-colors case if the CSS handles it. | A table: pair, where used, ratio, required ratio, pass/fail, per theme. |
| B4 | **Operation checks.** From the HTML and CSS: the keyboard order of focusable elements per page (DOM order and any `tabindex`); whether focus is visible (any `outline: none`/`0` rules; the focus style in use); target sizes of links and buttons in CSS px against 24×24 (compute from the CSS where possible); reflow at 320 px width and 400% zoom (render in a headless browser if one is available and report the tool; otherwise say it was not measured); text-spacing overrides; use of motion; whether anything depends on hover or on colour alone (the assurance badges, the verify page). | A per-page table; "not measured" where it was not, with the reason. |
| B5 | **The accessibility tree.** If a headless browser is available in your environment, dump the accessibility tree of the main pages (repository, verify, login, an error page) and compare it with B2's expectations; if not, say so. | The dumps or the statement that none could be produced. |
| B6 | **Language inventory.** List every user-visible string (templates, `ui.rs` labels, `errors.rs`, form hints, `<title>`s, error pages) with its location, whether it is a whole sentence, whether it is built by concatenation or has a count/plural, and any string that embeds a name, a number, a size or a date. Count them. | A table (key candidate, text, file:line, kind: sentence / fragment / label / plural / formatted-value). Flag every concatenated fragment (they must be rewritten as whole sentences, RFC 012 D-3e). |
| B7 | **Words inventory (D-4/D-6 input).** From B6, list every term that is jargon or an implementer's word, every gendered word or assumed-name pattern, and every violent, blaming or ableist metaphor; propose nothing, only list. Start the glossary as a table of *current term → where used → what it means in the code*. | The two lists and the glossary skeleton. |
| B8 | **API messages.** Confirm that every `ErrorCode` message and hint is a whole sentence with no placeholders, list the OpenAPI descriptions that are user-facing prose, and say how `Accept-Language` could be read at the existing response layer without touching handlers (a finding, not a design). | A short table and the finding. |
| B9 | **Catalog mechanism feasibility, no code in the repository.** In a scratch project, prove the shape D-3 describes with `askama` as the repository uses it: a catalog module with stable keys, one catalog per language compiled in, a completeness test that fails on a missing or orphan key, plural rules for English and Japanese (note that Japanese has no grammatical plural), and a template that looks messages up by key. State how many lines the mechanism took, whether `askama`'s compile-time checking helps or hinders, and whether any new dependency was needed. | The scratch project's source (kept in the review request's logs directory), the line count, and the answer to "no new dependency?". |

## Ground rules

Record tool names and versions; report counts and the full lists, not summaries alone; a negative result is a
result; if a measurement contradicts something RFC 012 states as fact (F1–F7), stop and hand back; do not fix
anything, and do not open or edit any file in the repository other than reading it.

## Required evidence and review-request format

For each row: the commands or script source (kept in the logs directory), the raw output, and a short "what this
says" separating **tool-measured** from **inferred**. End with: (1) a ranked list of the failures by how much they
block a person (blocking / serious / moderate / minor, with the rule each violates); (2) the list of what only a
human pass could check; (3) the string count and the share that are concatenated fragments; (4) the answer to
B9. Do not propose a design beyond that: the architect writes the implementation handoffs from your findings.
