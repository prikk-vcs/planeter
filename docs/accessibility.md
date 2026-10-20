# Accessibility

**Status: a first draft.** This page says what planeter's web pages aim for, what has been checked and how,
what has not been checked, and what is known to be missing. It is **not a claim of conformance**: nobody has
audited planeter against the standard, and no person who uses assistive technology has tried it yet.

## The aim

The web UI aims for [WCAG 2.2](https://www.w3.org/TR/WCAG22/) level AA (RFC 012 D-1). Where a rule could be
checked by a program, the program is part of the test suite, so a change that breaks it fails the build.

## What has been checked, and how

Everything below was checked by tools, not by people.

- **Page structure, on every page and state the UI can produce** — a test (`crates/planeter-web/tests/structure/`)
  renders each page through the real router, parses it the way a browser does, and checks: the page language
  and text direction; exactly one top-level heading and no skipped heading level; a skip link that is the first
  thing reached by keyboard and leads to the main content; a header, main and footer landmark and a unique name
  for every navigation block; a visible label for every form field; a caption and header cells on every table
  the page itself builds; no icon or image without a text alternative; no hidden-but-focusable element; link
  text that is not a bare identifier or a word like "here" or "raw"; one title per kind of page, in one pattern,
  with long identifiers shortened; no `<form>` where HTML does not allow one; no inline script or style. Each
  rule has a deliberately broken page in the test, to prove the rule can fail.
- **Headings inside a README or other Markdown file** are moved one level down when the page is built, so the
  page keeps one top-level heading and the file's own headings nest below it. The sanitizer that makes
  repository content safe runs first; a test shows that hostile heading markup stays inert.
- **Text, words and translation** — every word on a page comes from one message catalog (RFC 012 D-3) and is
  checked by tests, which is what lets a screen reader and a translator find them.
- **A one-off baseline** (2026-10-21) measured the pages in Chromium with axe-core 4.14.0 and html-validate
  11.17.0, scripted keyboard walks with puppeteer-core 25.14.0, colour contrast computed from the stylesheet
  (lowest ratio 4.87:1, passes), and the browser's accessibility tree. That run is where the list of gaps
  below comes from. It is not repeated on every change.

## What has not been checked

- **No testing with people who use assistive technology** — screen readers, screen magnifiers, switch or voice
  control. Whether the pages are *usable* with them, as opposed to passing the checks above, is unknown.
  In particular: how a screen reader reads the status badges and the tables row by row; whether a failed
  sign-in is noticed (the failure is announced as an alert, but that has not been heard).
- **Other browsers and systems.** The tool measurements used Chromium on Linux only.
- **Forced-colours and high-contrast modes, text-only zoom to 200 %, screen magnifiers, print output.**
- **Touch-target comfort on real devices** (the baseline found targets of 20–22.5 px that pass only by the
  spacing exception).
- **Whether the wording is understood** — plain words, the sentences' tone, and any translation need a
  fluent human reader.
- **Voice control** ("label in name", WCAG 2.5.3) beyond what a script can compute.
- **Session expiry.** There is no warning before a sign-in expires, so there was nothing to measure.
- **Help (WCAG 3.2.6).** The pages offer no help or contact route of their own.

## Known gaps

These are known and planned, not fixed yet (the next steps of RFC 012):

- Pages that scroll sideways at 320 px width, the change page above all (a 64-character identifier in a
  heading).
- Focus is shown only by the browser's default; no designed focus indicator.
- No colour-scheme handling beyond the light/dark switch; form controls stay light in the dark scheme; header
  links are not underlined.
- Layout uses physical (left/right) CSS properties, which a right-to-left language will need changed.
- Target sizes below the recommended size, as above.
- A README's own links that are relative resolve against the page address, not the file's folder.
- One language (English). Japanese and a language switcher are planned.

## How to report a problem

If a page does not work for you, or you find something here that is wrong, open an issue on the project's
repository (<https://github.com/prikk-vcs/planeter/issues>) and say which page you were on, what you were
using (browser, assistive technology, settings) and what happened. You do not need to know which rule it
breaks. A vulnerability is different: see [`SECURITY.md`](../SECURITY.md) and do not open a public issue.
