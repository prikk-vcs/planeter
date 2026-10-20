# RFC 012 — Web UI foundations — Implementation Handoff (v1): presentation and operation

| | |
|---|---|
| Document | Implements RFC 012 **D-2** (visible focus, target size, reflow, text spacing, colour scheme, forced colours) and **D-3(c)** (logical CSS properties), plus the presentation half of **D-5**: stylesheet checks as tests. D-7 step (3), second part, after [`structure-and-semantics-handoff-v1.md`](structure-and-semantics-handoff-v1.md) (done). Issue IS-41. |
| Status | **Assigned** (2026-10-23). |
| Basis | [`../../accepted/012-web-ui-foundations.md`](../../accepted/012-web-ui-foundations.md) D-2, D-3(c), D-5; the baseline review request (private): **B3** (every colour pair, light and dark, with ratios), **B4** (focus, target sizes, reflow at 320 px, text spacing, motion, physical-direction CSS: 4 declarations), the render and browser tooling and the 76 states (Chromium 154, axe-core 4.14.0, puppeteer-core: reuse the scripts and the approach); the structure checker under `crates/planeter-web/tests/structure/`; `crates/planeter-web/static/app.css`; `docs/accessibility.md`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Findings this handoff answers (tool-measured, from the baseline)

7 pages scroll sideways at 320 px (the change pages even at 1280 px: a 64-hex id in `<h1><code>`; history pages: a
long unbroken token; the not-ok verify page); targets of 20–22.5 px high pass only by the spacing exception; no
focus rule (browser default; Chromium's ring measured fine, other browsers unmeasured); dark theme leaves form
controls light (no `color-scheme`); no `forced-colors` rule; four physical-direction declarations; decorative borders
at 1.36:1 / 1.49:1; header links not underlined; with the WCAG 1.4.12 text-spacing override 10 pages overflow at 320 px.

## Change scope

- **T1 — Stylesheet checks first (as tests, shown able to fail).** Under `crates/planeter-web/tests/`, no new
  dependency (hand-written CSS token reading is acceptable; the stylesheet is small and written by us): (a) a
  **contrast test** that reads the custom properties and the rules that use them, computes the WCAG ratio of
  every foreground/background pair actually used in **both** themes (text ≥ 4.5:1, large text ≥ 3:1, and the
  non-text rule below), and fails on any pair below its threshold; (b) a **direction test** that fails on
  `margin-left/right`, `padding-left/right`, `left/right`, `border-left/right`, `text-align: left|right`, `float`
  left/right, in the stylesheet (logical properties only); (c) a **focus test** that fails if any rule removes an
  outline (`outline: none|0`) without a replacement `:focus-visible` style of at least the required contrast;
  (d) a **motion test** (no animation or transition without a `prefers-reduced-motion` guard); (e) a **target-size
  test** from the CSS (every standalone link/button/input has `min-height` and `min-width` of at least 24 CSS px).
  Run each against the **current** stylesheet first and report which findings each reproduces (they must fail),
  then pass after the fixes. Planted-failure fixtures prove each can fail.
- **T2 — Colours and the border rule.** The text pairs already pass; keep them. For borders: state in
  `docs/accessibility.md` the rule for which borders count as WCAG 1.4.11 component boundaries (those that are the
  only visual identification of an input, button or focus), give those at least 3:1 in both themes, and leave
  purely decorative rules (`--line` between table rows, the header rule) alone **or** raise them if it costs
  nothing visually: report the choice. Underline header navigation links on hover **and** focus, or give them a
  non-colour cue that does not depend on position alone (state which).
- **T3 — Focus.** A single explicit `:focus-visible` style for links, buttons, inputs and the skip link (outline of
  at least 2 CSS px, offset, using a token whose contrast against the background is ≥ 3:1 in both themes, with a
  contrasting inner ring where the background varies), never removed. Verify in Chromium by pixel measurement as
  the baseline did, and state that only Chromium was measured.
- **T4 — Target size.** Standalone targets (header and repository navigation links, form controls, buttons, the
  skip link, file-list links) are at least 24×24 CSS px by padding/min-size, not by the spacing exception; inline
  links in running text are exempt. Keep layouts compact (use padding, not large fonts).
- **T5 — Reflow, text spacing and long tokens.** At a 320 CSS px viewport and at 400% zoom, **no page needs
  two-dimensional scrolling** except the explicitly scrollable regions (`<pre>` code, and wide tables inside a
  labelled scroll container that is keyboard-focusable: `tabindex="0"`, `role="region"`, `aria-labelledby` the
  table's caption). Long ids/tokens wrap (`overflow-wrap:anywhere` on `<code>` ids, titles and names; `min-width:0`
  in flex/grid children). Apply the WCAG 1.4.12 text-spacing override (line-height 1.5, paragraph spacing 2×,
  letter spacing 0.12em, word spacing 0.16em) and show no clipped or overlapping content. Measure in headless
  Chromium over **all rendered states** (reuse the baseline harness; the structure checker's fixtures give the
  states) and report per state the horizontal overflow before and after.
- **T6 — Colour scheme and forced colours.** `color-scheme: light dark` so form controls follow the theme; add
  a `@media (forced-colors: active)` block so badges, the focus ring, borders and buttons remain distinguishable
  with system colours (use `CanvasText`/`Highlight`/`ButtonBorder` etc.; a badge keeps its text and a border). Verify
  with Chromium's emulation (`Emulation.setEmulatedMedia` with `forced-colors: active`; `prefers-color-scheme` both)
  and save screenshots; state what was **not** measured (Windows high-contrast with a real screen, other
  browsers).
- **T7 — Logical properties.** Replace every physical-direction declaration (the baseline lists four:
  `header .who{margin-left:auto}`, `nav.repo a{margin-right}`, `.crumbs a{margin-right}`, `td,th{text-align:left}`
  and any added since) with logical equivalents (`margin-inline-start`, `margin-inline-end`, `text-align:start`).
  Check with the test-only right-to-left catalog language already registered by the catalog tests: render a few
  pages with `dir="rtl"` and report anything that visibly breaks.
- **T8 — Documentation.** `docs/accessibility.md`: the border rule (T2), what is now checked automatically (the
  tests of T1), what was measured and by which tool/version, and what remains unmeasured (the baseline's human
  list, forced-colors on a real system, other browsers, print). No conformance claim.

## Non-change scope

No structure or wording changes (landmarks, headings, labels, texts: the previous handoff and the next); no
Japanese or switcher; no JavaScript; no web fonts, images or third-party requests; no new dependency (dev or
otherwise); no `unsafe`; no version or release change.

## Required documentation updates

`docs/accessibility.md` (T8), CHANGELOG `[Unreleased]` (operator-visible: the page layout changes slightly: larger
click areas, a visible focus ring, pages wrap at narrow widths, dark form controls), STATUS IS-41 (presentation
part closed on review).

## Acceptance criteria

T1's tests fail on the old stylesheet for the findings they cover and pass after (show both); **zero** states
overflow horizontally at 320 px outside the labelled scroll regions, with and without the text-spacing override;
all text pairs ≥ 4.5:1 and the border rule satisfied in both themes; the focus ring measured in both themes; forced
colours and colour-scheme screenshots saved; no physical-direction declaration remains; the structure checker and
all earlier tests still pass; every gate green including both supply-chain gates (fresh advisory database), both
test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No `outline:none`; no `overflow:hidden` to hide overflow; no `!important` to win a specificity fight (restructure
the CSS); no fixed pixel widths that break reflow; no test that reads its own expectations from the stylesheet
without an independent threshold; no weakening of a threshold to pass.

## Known risks

Larger touch areas change the look (keep it modest and report before/after screenshots); a custom focus ring can
fail on some backgrounds (the contrast test and the pixel measurement are the guard); forced-colors rendering
cannot be fully verified without a real system (said so in the statement).

## Required evidence and review-request format

Commits (hash → what); the first-run failures of T1's tests on the old stylesheet and the final run; the
per-state horizontal-overflow table before/after at 320 px (with and without text spacing); the contrast table
(pair, where used, ratio, threshold, theme); the focus-ring measurements; the forced-colors and colour-scheme
screenshots (kept in the logs directory); every gate with command and exit code and the advisory-database HEAD;
deviations and open questions.
