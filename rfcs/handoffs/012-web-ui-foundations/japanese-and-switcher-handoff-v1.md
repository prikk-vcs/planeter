# RFC 012 — Web UI foundations — Implementation Handoff (v1): Japanese, the language switcher and the accessibility statement

| | |
|---|---|
| Document | Implements RFC 012 **D-3** (a second shipped language, the visible switcher) and D-7 steps (5) and (6). The English wording is final (plain-words work, approved); the reviewers named by the owner on 2026-10-21 (RFC 012 O-2) are **the owner and a colleague**, both fluent. Two phases: (1) the dev team builds the Japanese catalog **as a draft for the reviewers**, the switcher and the checks; (2) after the reviewers return their corrections, the dev team integrates them. A Japanese catalog never ships without the reviewers' pass (RFC 012 D-3f). Issue IS-27. |
| Status | **Phase 1 done, reviewed Approved (2026-10-21; commits `5358129`, `4a28b05`, `da5fe1f`).** Waiting for the reviewers (the owner and a colleague) to return the sheet; phase 2 is a small follow-up the architect writes then. |
| Basis | [`../../accepted/012-web-ui-foundations.md`](../../accepted/012-web-ui-foundations.md) D-3, D-4, D-5, D-6; `docs/glossary.md` (the English words in force; certainty column; the trust labels); `docs/messages-and-languages.md`; `docs/accessibility.md`; `crates/planeter-web/src/catalog/{mod,en}.rs`, `tests/{catalog_checks,i18n,pages_in_two_languages,glossary,retired_words,structure,stylesheet}`; the baseline review request (private), B6 (the string inventory). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`) **and the reviewer's sheet** (T3). |

## Change scope

- **T1 — The Japanese catalog (draft).** `catalog/ja.rs`: every key of the English catalog, written as natural Japanese
  (not word-for-word), following the glossary's meaning for every concept and **preserving the meaning of every trust
  label exactly** (Verified; Approved, awaiting signature; Signed into the history; Verification failed; Could not be
  checked now — and their explanation sentences, the error pages and the sign-in messages). Japanese has no grammatical
  plural: plural messages carry `one == other`. Sizes use the catalog units; numbers group per the language
  (`1,200`). Politeness: polite form (です・ます), consistent; no gendered or blaming wording (RFC 012 D-6 applies to the
  Japanese text; the deny-list test gets a Japanese list: items the dev team proposes and the reviewers amend). A Japanese
  text may be drafted with machine help, but it ships only after the reviewers' pass; mark every message
  `needs-review` in the sheet. The catalog is compiled in; `shipped()` includes Japanese **only behind the reviewers'
  sign-off** (see T6: until phase 2 it is registered like the test language, not shipped).
- **T2 — The Japanese glossary.** A section in `docs/glossary.md`: for each concept the Japanese term the draft uses and
  why, with the **hard terms flagged for the reviewers' decision**: *seal / sealed / unsealed* (prikk's own Japanese
  usage, if prikk documents one — check; otherwise propose), *verify / verification*, *branch*, *patch*, *change*,
  *rollback*, *signed into the history*, *single sign-on*, *repository*. The terms must be used identically everywhere.
- **T3 — The reviewers' sheet.** A single file `.git-exclude/review-requests/japanese-review-sheet.md` (private, plain
  Markdown, easy to edit by hand): a table with one row per message — **key | where it appears (page/state) | English |
  Japanese draft | placeholders and what they stand for | note/context for the reviewer | reviewer's correction (empty)** —
  grouped by page, the trust labels and their explanations first, then error pages, then the rest, with a one-page preface
  (how to review; what a placeholder is; the terms to decide; how to return it). No code needed to read it. The architect
  does not edit it. Keep the rows in the catalog's key order inside each group so corrections map back mechanically.
- **T4 — Negotiation, the switcher and `lang`.** Japanese negotiable by `Accept-Language` (`ja`, `ja-JP`) and by the
  `planeter_lang` cookie (existing machinery); `<html lang="ja" dir="ltr">`; `Vary: Accept-Language`, `Content-Language`.
  **The switcher** (RFC 012 D-3b; placement decided by the architect: the **footer of every page and the sign-in
  page**): endonyms ("English", "日本語"), each entry a real form button/link with `lang` set to its own language, the
  current language marked (`aria-current`), no script; selecting a language sets the cookie (`SameSite=Lax; Secure;
  Path=/`, a language code only) with a `POST` to a small route (CSRF-protected like the other forms) that redirects back
  to the same page; the route is in the IS-14 enumeration table. Text and landmarks come from the catalog (the structure
  checker's rules must pass: labelled, keyboard-operable, target size ≥ 24 px).
- **T5 — Typography for Japanese.** `app.css`: confirm the `font-family` falls through to system fonts that render
  Japanese (no web fonts, no third-party requests: RFC 012 D-6d), line-height suitable for CJK, `overflow-wrap` rules do
  not break Japanese text mid-way badly (use `overflow-wrap: anywhere` only where it is already needed for identifiers),
  the stylesheet tests stay green, and **render every state in Japanese** (the structure checker's `STRUCTURE_LANG`
  switch) to confirm reflow at 320 px, no overlaps, with and without the WCAG text-spacing override (Chromium, as the
  presentation handoff did; report).
- **T6 — The checks cover Japanese.** The catalog checks (completeness, orphans, duplicates, placeholder sets,
  plural forms for a language with only `other`, no markup in a message), the retired-words check (a Japanese retired
  list: the draft's words that the glossary forbids), the glossary/catalog agreement test (a *Japanese* column), the
  structural checker over all states in Japanese, the stylesheet tests. **A test pins the rule "a language not signed off
  is not in `shipped()`"**: `shipped()` lists Japanese only when a constant `JA_REVIEWED` is true, set in phase 2.
  Until then Japanese is registered for negotiation in a *preview* mode enabled by an operator setting (default off),
  so the reviewers can see it in the running product (see T7).
- **T7 — A preview for the reviewers.** Reviewers read the sheet; they also want to see pages. With the operator setting
  `PLANETER_PREVIEW_LANGUAGES=ja` (documented in `--help` and RELEASING, off by default, logged at start) Japanese is
  negotiable; without it, it is not offered and the switcher shows English only. Add the variable to `args::USAGE` (the
  usage/main drift test guards it).
- **T8 — The accessibility statement.** Update `docs/accessibility.md`: languages (English; Japanese in preview, reviewed by
  the owner and a colleague once signed off), what the Japanese checks cover, and still plainly that no assistive-technology
  user has tested the product. The final statement is completed in phase 2.
- **T9 — Documentation.** `docs/messages-and-languages.md` (how a language is added and signed off: the sheet, `needs-review`,
  `JA_REVIEWED`), RELEASING (the preview variable), CHANGELOG `[Unreleased]`, STATUS IS-27 progress.

## Non-change scope

No change to English wording; no new English message beyond the switcher's own labels and the preview setting's log line;
no other language; no machine translation at run time; no JavaScript; no web fonts or third-party requests; no new
dependency; no `unsafe`; no API change except `Content-Language` on error bodies in Japanese (already built).

## Acceptance criteria

All English keys exist in Japanese with identical placeholder sets; the catalog, retired-words, glossary, structural and
stylesheet checks pass for Japanese; every state renders in Japanese without sideways scroll at 320 px; the switcher works
(cookie set, same page, CSRF, keyboard); the sheet is complete, readable and ordered as T3 says; a test proves Japanese is
**not** shipped or offered without the preview setting and `JA_REVIEWED`; every gate green including both supply-chain
gates (fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No Japanese in `shipped()` before sign-off; no trust label whose Japanese meaning is weaker or stronger than the English
(if unsure, flag it in the sheet instead of choosing); no word-for-word translation of an idiom; no hard-coded Japanese
outside the catalog; no weakening of a check to admit the new language.

## Known risks

A trust label that reads confidently in Japanese when the English hedges (or the reverse) is the worst failure here:
review those rows first and in full. Terms for *seal/verify* may have no settled Japanese equivalent in prikk's own
documentation: the reviewers decide; the draft must not pretend.

## Required evidence and review-request format

Commits (hash → what); the sheet's path and size; the terms flagged for decision; renders of three pages in Japanese
(screenshots at 320 px and 1280 px, light and dark); the switcher's behaviour; the checks and mutations (a missing
Japanese key; a changed placeholder; Japanese shipped without sign-off); every gate with command and exit code and the
advisory-database HEAD; deviations and open questions.
