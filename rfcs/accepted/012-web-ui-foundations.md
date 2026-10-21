# RFC 012 — Web UI foundations: language, plain words, accessibility and inclusion

**Status.** **Accepted (2026-10-20) by the owner** (O-1 accepted; O-2: English plus Japanese; O-3: no assistive-technology
user is available now, so the pass is limited as D-5 states). Handoffs: [`../handoffs/012-web-ui-foundations/`](../handoffs/012-web-ui-foundations/). Proposed (2026-10-20) by the architect, on the owner's philosophy ("APIs and UI/UX for users not to be
confused or misunderstand") and the owner's direction that **accessibility and social inclusiveness are important**
(2026-10-20). 
**Tracks.** Issue register IS-27 (no i18n), IS-30 (jargon as default vocabulary, no next steps, polish); SCHEDULE
T14; project rule: *the GUI must support multiple languages*; the architect's UI/API audit (private
`ui-api-audit-v1`); RFC 003 (the UI as shipped); RFC 005 (the review UI that will build on this).
**Touches.** `planeter-web` (`templates/`, `static/app.css`, `ui.rs`, `errors.rs`, `openapi.rs`), a new catalog
module, tests, docs. **Not here:** new product capability (repository listing is IS-7, a feature), the review UI
itself (RFC 005), the API's wire contract (codes and shapes stay), authentication changes.

## Summary

planeter's web UI is read-only, server-rendered and script-free, which is an excellent base for people using
assistive technology, old browsers, slow connections or text-only readers. What it lacks is the rest: every
string is hard-coded English in templates and Rust, the vocabulary is the implementer's ("prikk-verified",
"unsealed", "session(s)", "refs"), nothing is marked up for assistive technology, and nothing was checked against
an accessibility standard. This RFC makes **language, plain words, accessibility and inclusion** foundations that
every later page (the review UI first) is built on, with checks that fail the build, not guidelines that drift.

## Evidence (verified facts)

- **F1.** One fixed `<html lang="en">`; all user-visible text is literal in 15 askama templates and in Rust
  (`ui.rs` assurance labels, `errors.rs` messages and hints). No i18n dependency exists in the lock file.
- **F2.** The templates contain **no `aria-*` or `role` attributes**, no skip link, no explicit focus styles, no
  `prefers-reduced-motion` rule. (`prefers-color-scheme` dark mode exists; contrast ratios have not been checked.)
- **F3.** The UI uses emoji as file and folder icons; the refs page has two `<h1>`; page titles follow two
  different patterns; `/go` swallows bad input silently; error pages give no next step.
- **F4.** The UI has **no JavaScript** and a strict CSP; every control is a native form element; raw bytes are
  never inlined. These are strengths to keep.
- **F5.** The API's error codes are stable and its messages are written once, in `errors.rs`, "so they can be
  catalogued for translation later" (RFC 003 follow-on).
- **F6.** prikk records no timestamps; the forge owns the few times it shows. Names shown are account ids and
  display names (free text, any script).
- **F7.** No accessibility audit by tool or by a person using assistive technology has been done.

## Unknowns (measured first, not assumed)

- **U1. The real accessibility baseline:** automated checks (HTML validity, labels, landmarks, headings, contrast
  of every colour pair in both themes, target sizes), the accessibility tree of every page and error state, and
  keyboard order, in both themes. **None of this replaces a person using assistive technology**, and no such
  person is available to the project today (O-3): the baseline therefore measures what tools can, and the
  accessibility statement says in plain words that the UI has **not** been tested with people who use assistive
  technology and invites them to report problems. A human pass is scheduled the day one is available.
- **U2. How many languages we can keep true.** A catalog is only as good as its translations; the cost of keeping
  each current is real (see D-3).
- **U3. What users actually find confusing.** The vocabulary decisions (D-4) rest on the audit and on the owner's
  reading; if the owner can arrange two or three readers outside the project, a short think-aloud pass over the
  repository page and the verify page is the cheapest evidence there is.

## Decisions

- **D-1 — The standard we hold ourselves to: WCAG 2.2 level AA** for every page and state the UI renders, plus the
  inclusion rules of D-6. "Conformance" is claimed only for what was checked; the project publishes an
  **accessibility statement** (what was checked, how, known gaps, how to report one) with each release.
- **D-2 — Structure and operation (the WCAG part), as concrete rules:** one `<h1>` per page and no skipped heading
  levels; landmarks (`header`, `nav` labelled, `main`, `footer`) and a **skip link** first in the page; every
  control has a visible, programmatically associated label; **visible focus** on every focusable element, never
  removed; everything operable by keyboard in a logical order; no information by colour alone (the assurance
  badges already carry text; keep it so); contrast ≥ 4.5:1 for text and ≥ 3:1 for large text and component
  boundaries, in **both** themes; touch/click targets ≥ 24×24 CSS px (WCAG 2.2 target size); content reflows at
  320 px width and at 400% zoom without two-dimensional scrolling; text spacing overrides do not break layout;
  `prefers-reduced-motion` and `prefers-color-scheme` respected (nothing animates today; keep it); form errors
  are identified in text next to the field with a suggestion; **accessible authentication** (WCAG 2.2: no
  cognitive-function test, paste into password fields allowed, password managers work, SSO offered); a session
  that is about to expire is not silently lost; each page has a unique, descriptive `<title>`; every image has
  text alternatives (emoji icons are replaced by inline SVG with `aria-hidden` plus a text label, or by text);
  links say where they go; tables have headers.
- **D-3 — Language is a first-class concern.** (a) **Mechanism:** message catalogs keyed by stable identifiers,
  one catalog per language, compiled into the binary; no runtime file lookup; **no new dependency in the first
  version** (a small catalog module and per-language plural rules; the project's dependency policy applies, and a
  fluent-style library is reconsidered only if the number of languages makes it pay for itself). A test fails
  the build when a key is missing in any shipped language or a catalog has a key no template uses. (b) **Choice
  of language:** `Accept-Language` negotiation, overridable by a visible language switcher whose choice is kept in
  a cookie (no account needed), with endonyms ("日本語", "English") and a `lang` attribute on the switcher's
  entries and on any text in another language; `<html lang>` and `dir` follow the page's language. (c) **Right to
  left readiness:** layout uses logical CSS properties (`margin-inline-start`, not `margin-left`) from now on, so
  adding such a language is a catalog task, not a redesign. (d) **The API:** codes and shapes do not change;
  `message` and `hint` follow `Accept-Language` (English by default), so clients may also localize by code.
  (e) **Never concatenate sentence fragments**; messages are whole sentences with named placeholders so that word
  order can differ; plural forms per language rules; numbers, sizes and dates are formatted per locale (dates only
  where we show one, with an unambiguous form). (f) **Which languages ship first** is an owner decision (O-2);
  a language is shipped only when a fluent speaker has reviewed it, and the accessibility statement says which.
- **D-4 — Plain words.** A **glossary** (one page in the repository) fixes the user-facing term for every
  concept, with the reason, and the catalogs use only glossary terms. Rules: say what happened and what to do
  next; prefer the reader's word to the implementer's ("signature check", not "verify stage"; "not yet confirmed",
  not "forge-approved, unsealed"); no jargon without a one-line explanation available in place (a `<details>` or
  a linked help page, not a tooltip); short sentences, active voice, no idioms or metaphors that do not
  translate; the same word for the same thing everywhere; empty states and errors always name a next step. The
  assurance labels are rewritten first because they carry the most weight and the most confusion (they must still
  never present an unconfirmed change as verified: RFC 005 D-10). Where the exact technical term matters, the
  plain word comes first and the term is available on demand.
- **D-5 — Safe by construction.** The checks are tests, not advice: the build fails on a missing label, a skipped
  heading level, a second `<h1>`, a missing `alt`, a missing language key, a hard-coded colour pair below its
  contrast ratio (computed from the CSS tokens in a test), `margin-left`/`padding-right` and other physical
  properties in the stylesheet, and an `aria-hidden` element that is focusable. A written **checklist**
  (keyboard order, accessibility-tree review, 200%/400% zoom, forced-colors mode, one language other than English;
  and a screen-reader pass **by a person** when one is available) is part of each release's candidate step; the
  result is recorded in the review request, and a step that was not done by a person is labelled as such.
- **D-6 — Social inclusiveness, as rules.** (a) **Names:** a person's name is one free-text field in any script;
  never split into first and last, never truncated without a way to read it, wrapped not cut; the UI shows the
  display name the person gave and the account id, nothing inferred. (b) **No assumptions about people:** no
  gendered words, titles or pronouns in any message (messages are written to need none); no field asks for
  gender; examples and placeholders use neutral, culturally varied names; icons and metaphors are checked for
  cultural neutrality. (c) **Words:** inclusive technical vocabulary (allowlist/blocklist, primary/default branch
  as prikk names it, no violence or ableist metaphors in messages: "stuck", not "hung/dead"; "invalid", not
  "crippled"). (d) **Access conditions:** the UI keeps working without JavaScript, on slow or metered connections
  (small pages, one stylesheet, no web fonts, no third-party requests), on old browsers, in text-only readers and
  in print; a page's meaning never depends on hover. (e) **Tone:** respectful and calm, never blaming ("this
  could not be read", not "you entered an invalid path"); errors explain without alarm. (f) **Privacy:** the
  language cookie is the only preference stored for anonymous visitors, and it holds a language code.
- **D-7 — Order of work.** Foundations first, features after: (1) the baseline measurement (U1); (2) the catalog
  mechanism, the checks of D-5 and the extraction of every existing string, in English, with no wording change;
  (3) the accessibility fixes of D-2 on the existing pages; (4) the glossary and the plain-word rewrite (D-4);
  (5) the first additional language(s) (O-2); (6) the accessibility statement. RFC 005's review UI is built on
  the result and must not add a hard-coded string or a page that fails a D-5 check.
- **D-8 — What this RFC does not do.** It does not build the repository list or the home page's discovery (IS-7,
  a feature that needs its own design), does not change who can see what, and does not rename API codes.

## Alternatives considered

- **Add translations later, per page.** Rejected: each page added before the mechanism multiplies the debt
  (RFC 005's UI would double it).
- **A fluent/ICU-based library now.** Powerful and heavy; the project's rule is a new dependency only when it
  pays for itself, and two or three languages do not need it. Reconsidered at the fifth language or the first
  language whose grammar the small plural-and-placeholder scheme cannot express.
- **Auto-translate catalogs.** Rejected for user-facing text: unreviewed machine output would put confusion back
  in through the front door. Machine drafts may help a human translator, never ship alone.
- **A client-side language switcher with script.** Rejected: the UI has no script by design (F4).
- **An accessibility overlay or widget.** Rejected: it does not fix markup and often breaks assistive technology.

## Risks

- **False confidence from automated checks.** Mitigation: the manual checklist and the person-run passes are
  release gates, and the statement names what was and was not checked.
- **Translation drift.** A language falls behind the English catalog. Mitigation: the completeness test fails the
  build; a language that cannot be kept current is withdrawn and the statement says so.
- **Plain words that are less exact.** Mitigation: the technical term is one step away, and the glossary records
  the mapping; trust-bearing labels keep their exact meaning (D-4).
- **Cost.** This is real work before any new feature; it is cheaper now than after the review UI.

## Verification plan

Tests under `tests/`: catalog completeness and orphan keys; every rendered page and state passes the structural
checks of D-5 (one `h1`, heading order, labels, `alt`, landmarks, skip link, unique titles, `lang`/`dir`);
contrast ratios computed from the CSS tokens for both themes; no physical-direction CSS; language negotiation and
the cookie override; API `message`/`hint` follow `Accept-Language` and fall back to English; no catalog message
contains a gendered pronoun (a deny-list test over the shipped catalogs). Evidence per release: the manual
checklist result and the accessibility statement.

## Open questions

1. How should the language switcher be placed so it is findable without being noisy? (Footer plus the sign-in
   page is the proposal.)
2. Which help mechanism carries the one-line explanations (`<details>` inline, or a small help page)? The
   proposal is `<details>` for terms and a help page for concepts.

## Owner decisions

- **O-1.** Accept the direction (D-1 to D-8) and place theme T14 before RFC 005's UI.
- **O-2 (settled 2026-10-20; reviewers named 2026-10-21).** English plus Japanese. A language ships only when a fluent speaker has reviewed
  it; **the Japanese reviewers are the owner and a colleague of the owner** (2026-10-21). They review a sheet the dev team prepares
  and may see the pages in a preview mode; Japanese is not offered to anyone else until they sign it off.
- **O-3 (settled 2026-10-20).** No outside readers and no assistive-technology user are available now. The
  project proceeds with tool-based measurement and the labelled statement of D-1/D-5, and revisits when a person
  is available (U3 stays open).
