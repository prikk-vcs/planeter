# RFC 012 — Web UI foundations — Implementation Handoff (v1): structure and semantics, with the structural checks

| | |
|---|---|
| Document | Implements RFC 012 **D-2** (structure, labels, links, titles, accessible forms) and **D-6(a)** (names), and the structural half of **D-5**: the rendered-page checks. D-7 step (3), first part. The presentation half (focus, target size, reflow, colour scheme, forced colours, logical CSS properties, the contrast test) is the next handoff. Issues IS-41, IS-30. |
| Status | **Done, reviewed Approved (2026-10-23; commits `d359e09`, `e808ac5`, `092e1f1`, `162b7b7`).** IS-41 (structure) closed; presentation and operation remain (next handoff). |
| Basis | [`../../accepted/012-web-ui-foundations.md`](../../accepted/012-web-ui-foundations.md) D-2, D-5, D-6; the baseline review request (private): **B1** (76 states and how they were rendered: reuse its render program), **B2** (the per-page rule grid and `b2-per-page.md`: every offending element), **B5** (accessibility trees); [`message-catalog-handoff-v1.md`](message-catalog-handoff-v1.md) (done): every new text goes through the catalog; `docs/messages-and-languages.md`; `crates/planeter-web/templates/*.html`, `ui.rs`, `catalog/`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The checker first (so every fix is proved).** A test module under `crates/planeter-web/tests/` that renders
  **every page and state** the baseline's B1 lists (reuse the render program's fixture builders; the states that
  need real prikk self-skip *with* a printed reason, and a stand-in-based subset always runs) and runs structural
  rules over the HTML. Parse with a parser **already in the dependency tree** as a *dev-dependency* of
  `planeter-web` (the lock already holds `html5ever`/`markup5ever` via `ammonia`; confirm and name the crate and
  version); **if that needs any crate not already in `Cargo.lock`, stop and report.** Rules (each with a planted
  failure proving it can fail): `lang`/`dir` present; exactly one `<h1>` in the page's own markup and **none other
  in rendered content**; no skipped heading level; a skip link is the first focusable element and targets the
  `<main>` id; landmarks `banner`, `main`, `contentinfo`, and every `<nav>` has an accessible name; every form
  control has an associated visible `<label>` (placeholder never counts); every `<table>` has a `<caption>` and
  `<th scope>` cells; no `<img>`/icon without a text alternative or `aria-hidden` decorative role; no focusable
  `aria-hidden`; link text is not a bare identifier and not in a deny-list of vague words (the list in a file,
  reviewed like code); unique `<title>` per distinct state (a map state → title; no two states share one unless
  they are the same page); no `<form>` inside phrasing-only content; no inline script or style. Report which of
  B2's findings each rule reproduces on the *current* tree (they must fail first), then pass after the fixes.
- **T2 — Landmarks and skip link.** A skip link as the first element (text from the catalog), a `<main id>` target,
  `<header>`, `<nav aria-label>` (catalog text, unique per nav), and a `<footer>` (`contentinfo`) that carries only
  what is true and useful today (the product name and version; nothing else). Fix the `<form>` inside `<span class="who">`
  by restructuring the signed-in header into valid content.
- **T3 — Headings.** Every page has one `<h1>` naming the page. Pages that had none (file, tree) get one; refs gets
  one `<h1>` and `<h2>`s. Rendered Markdown (README, file view) is **demoted by one level** at render time
  (its `h1` becomes `h2`, `h2`→`h3`, … capped at `h6`) inside the existing sanitizing renderer; the sanitizer's
  tests and the injection tests stay green and a new test shows hostile heading markup stays inert.
- **T4 — Tables.** Listing, history, refs, change and verify-stage tables get a `<caption>` (catalog text; visually
  hidden only if the visible heading already says the same, using a small utility class in the stylesheet) and
  `<th scope="col">`/`row` cells with catalog header texts. No layout tables.
- **T5 — Icons.** Replace the 📁/📄 emoji with a text label from the catalog (a visible word) — or, if the dev team
  prefers, an inline SVG with `aria-hidden="true"` next to a text label. Either way a screen reader reads the name
  once, never "file folder". (No new image files or third-party assets.)
- **T6 — Links and the sign-in/home forms.** (a) The history rows' link text is the change's summary line (data from
  prikk, escaped), with the short id as plain secondary text, not the only link text; the "raw" link text becomes a
  catalog sentence naming the file ("Download {name}"). (b) The home page's address field gets a visible
  `<label>` (catalog) and `autocomplete="off"`, `spellcheck="false"`; `/go` with invalid input **re-renders the home
  page with the error message next to the field** (status 400 for the error render), `aria-invalid`, and
  `aria-describedby` tying the message to the field. (c) Sign-in: `autocomplete="username"` / `"current-password"`
  on the fields, `<label>` for each (exists; keep), and the failure message is in a container with `role="alert"` and
  associated with the form (`aria-describedby`), focus moves to it only by the browser's natural behaviour on a new
  page load (no script). (d) **Locked-out sign-in gets its own message** ("Too many sign-in attempts. Wait a few
  minutes and try again."): the lock is keyed on the submitted name and the address whether or not the account
  exists, so this reveals nothing; **prove it with a test** that the response for a locked existing name and a locked
  unknown name are identical (status, headers, body). Wrong credentials keep the single generic message.
- **T7 — Titles.** Distinct titles for every distinct state (busy, unavailable and a missing change no longer
  share one), one pattern `{page} · {context} · planeter`, long ids shortened to 12 characters in titles. Catalog
  keys, not literals.
- **T8 — Names (D-6a).** The signed-in header shows the person's **display name** when there is one, and the
  account id as secondary text; the display name is escaped, wrapped not cut, never split or reordered; with no
  display name, the id alone. A test with names in several scripts (Latin with diacritics, Japanese, Arabic, a long
  name, an empty one) proves they render intact and in order.
- **T9 — Accessibility statement, first draft.** `docs/accessibility.md`: the standard (WCAG 2.2 AA, D-1), what has
  been checked and how (tool-measured, listing the tools), **what has not been** (no testing with people who use
  assistive technology; the baseline's "only a human pass can check" list), known gaps (the next handoffs'), and how
  to report a problem. Plain words; no claim of conformance.

## Non-change scope

No CSS-only work (focus, target size, reflow, colour scheme, forced colours, logical properties, the contrast test:
the next handoff — **do not fix them opportunistically**); no wording rewrite beyond the new texts above and the
catalog rules; no Japanese; no switcher; no change to the API; no new external dependency; no `unsafe`; no version
or release change.

## Required documentation updates

`docs/accessibility.md` (T9), `docs/messages-and-languages.md` (the new keys follow its rules; add a short
"Markup and accessibility" section only if a rule changed), CHANGELOG `[Unreleased]` (operator-visible: new
landmarks and skip link, the sign-in lock message, `/go` error behaviour, display name shown), STATUS IS-41
progress.

## Acceptance criteria

T1's rules fail on the pre-change tree for the findings they cover and pass after (show both runs); every rendered
state passes every rule or is on a written, reasoned exception list (expected: none); the lock-message test; the
names test; the Markdown demotion and injection tests; catalog checks still green (no literal text added outside
the catalog); every gate green including both supply-chain gates (fresh advisory database), both test runs named,
ENF-2, `cargo doc`.

## Prohibited shortcuts

No `aria-label` used to paper over a missing visible label; no `role` that duplicates native semantics; no hidden
text that says something the page does not show; no weakening of a rule to pass; no hard-coded text outside the
catalog; no `|safe` on catalog output; no script.

## Known risks

The README demotion touches the sanitizing path (security-sensitive: keep its tests); a footer that promises more
than is true is worse than none (T2 says what it may contain); the parser dev-dependency must be one already in
the lock.

## Required evidence and review-request format

Commits (hash → what, by change set if tasks share files); the checker's first-run failures (counts per rule on the
old tree) and the final run (all states × all rules); the lock-message and names test output; before/after samples of
three pages' landmarks and headings; every gate with command and exit code and the advisory-database HEAD; the
dev-dependency named; deviations and open questions.
