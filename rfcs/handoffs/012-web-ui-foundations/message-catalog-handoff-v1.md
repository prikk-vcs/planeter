# RFC 012 — Web UI foundations — Implementation Handoff (v1): the message catalog and the extraction of every string

| | |
|---|---|
| Document | Implements RFC 012 **D-3** (the catalog mechanism, language negotiation, API messages by language) and the first half of **D-5** (the catalog checks), and performs D-7 step (2): every user-visible string moves into the catalog, **in English, without changing what the pages say** except where a fragment must become a whole sentence. It is the first implementation handoff of RFC 012; the structural accessibility fixes (D-2) follow in the next, and the plain-word rewrite (D-4) and Japanese after that. Issues IS-27, IS-30. |
| Status | **Done, reviewed Approved (2026-10-22; commits `caf86d1`, `98a56a3`, `90d3f84`).** IS-27 closed for the mechanism and the extraction. |
| Basis | [`../../accepted/012-web-ui-foundations.md`](../../accepted/012-web-ui-foundations.md) D-3, D-5, D-7; the baseline review request (private): **B6** (the 104-string inventory with file:line and the 36 fragments), **B8** (where `Accept-Language` can be read), **B9** (the scratch catalog proof: `Key` enum, one table per language, plural rules, `fill`, negotiation, tests, and the **design gap** of markup inside sentences) — reuse its source; `crates/planeter-web/templates/*.html`, `ui.rs`, `errors.rs`, `headers.rs`, `openapi.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The catalog module** (in `planeter-web`, or a small new workspace crate if the layering gate prefers it;
  no new external dependency — B9 proved none is needed). Stable keys (an `enum`, one line each), one compiled-in
  table per language (English now; the table type must make adding Japanese a catalog task), `Lang`, plural rules
  per language (English one/other; Japanese always other; the scheme must be extendable to CLDR categories later
  without rewriting callers), named `{placeholders}`, locale-aware number and size formatting (sizes as KiB/MiB
  with the unit in the catalog, replacing the raw `B`), and a lookup that takes the request's language. Messages
  are **whole sentences**; no key is ever concatenated with another to form a sentence.
- **T2 — Markup inside sentences (the B9 gap): design it safely.** A sentence that contains a link or emphasis
  (the home page's "…by its owner/name", "Binary content — download.") is a catalog message with **named slots**
  (`{download}`) filled by typed fragments: text parts are HTML-escaped, a link slot is built from an escaped label
  and an href the **code** supplies (never the catalog), and the result is a type askama renders without a second
  escape. `|safe` on catalog output is **forbidden** (a test scans templates for it). A test proves that a hostile
  value in a text or link slot cannot inject markup or an attribute, and that a catalog entry cannot introduce a
  tag, an attribute or a URL by itself.
- **T3 — Extract everything.** Every string of B6 that is ours (104) becomes a catalog key and the templates and
  `ui.rs`/`errors.rs` look it up; **no hard-coded user-visible text remains** (data values, prikk-owned text and
  punctuation separators are the documented exceptions — list them). Wording stays as it is, with these changes only:
  the 36 fragments become whole sentences with placeholders using the same words; the hard-coded plurals
  (`patch(es)`, `session(s)`) become plural messages; `message + hint` pairs stay two messages. Where one English
  word means two things (B7: "session"), use two keys now and let the glossary decide the words later. List every
  place where you had to change a word and why.
- **T4 — Language negotiation (D-3b).** `Accept-Language` with q-values, overridden by a `lang` cookie (a language
  code only, `SameSite=Lax`, no `HttpOnly` need, long-lived but not tracking: RFC 012 D-6f); `<html lang>` and `dir`
  follow the page's language; `Vary: Accept-Language` is added where the language varies the response (the
  existing response-policy layer). With one shipped language the switcher is **not** built yet (it appears with
  Japanese); the cookie and negotiation work and are tested with a second, test-only catalog.
- **T5 — API messages by language (D-3d, B8).** `message` and `hint` follow `Accept-Language` through the
  catalog at the single place error bodies are built; `code` and shapes are unchanged; the `TooLarge` code's two
  hints become two keys chosen at the call site (not matched by text). The OpenAPI document's developer-facing
  prose stays English and says so.
- **T6 — The checks (D-5, catalog half).** Tests under `tests/` that fail the build on: a key missing in any
  shipped language; an orphan key no template or Rust path uses (B9's scanner); a placeholder set that differs
  across languages; a template or Rust source that contains user-visible literal text outside the documented
  exceptions (a scan with an allowlist file kept in the repository); `|safe` on catalog output; a plural key whose
  languages disagree on the forms they define; a catalog message containing markup. Each check is shown able to
  fail (planted mutation, then restored).
- **T7 — Pages that are not pages.** The 11 bare-string responses (B1: CSRF failures, SSO errors, axum
  rejections) become real pages through the same layout with catalog messages, a unique title and a next step
  where one exists; they keep their status codes. **No internal error text is shown** (the SSO provider error that
  embeds a fetch failure is logged, not rendered). This is the only place the visible content grows; keep it to
  one sentence plus one link per page.

## Non-change scope

No Japanese yet; no language switcher; no structural accessibility fixes (headings, landmarks, skip link, tables,
focus, reflow: the next handoff — **do not fix them opportunistically**, because each is a reviewed change with its
own test); no wording rewrite beyond T3's list; no change to the API's codes, shapes or statuses; no new external
dependency (if one seems needed, stop and report); no `unsafe`; no version or release change.

## Required documentation updates

`docs/` (a short "Messages and languages" page: how to add a message, a language, a placeholder; the rule that
sentences are whole; the allowlist of exceptions), CHANGELOG `[Unreleased]` (operator-visible: the `lang`
cookie, `Accept-Language` handling, sizes shown in KiB/MiB, the new pages for errors that were bare strings),
README only if a user-visible fact changes, STATUS IS-27 progress.

## Acceptance criteria

Every B6 string is a catalog key or is on the documented exception list; the pages render the same words as before
except as T3 lists; T6's checks pass and each fails under its mutation; the injection test passes; a test-only
second language proves negotiation, the cookie, `lang`/`dir`, `Vary`, plural and placeholder handling, and the
API's `message`/`hint`; the 11 states are pages; no raw error text reaches a page; every gate green including both
supply-chain gates (fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No `|safe` on catalog output; no string concatenation across keys; no catalog entry holding markup or a URL; no
silent English fallback for a missing key in a shipped language (the test fails first; at run time a missing key
is a visible, logged, non-crashing marker); no new dependency; no opportunistic accessibility or wording changes.

## Known risks

~100 messages × (enum line + one line per language) is manageable by hand at two languages; the check scanner for
"user-visible literal text" can produce false positives (the allowlist is the remedy, reviewed like code); askama
cannot check placeholders or argument types (T6's tests do).

## Required evidence and review-request format

Commits (hash → what, one per T); every gate with command and exit code and the advisory-database HEAD; the
string accounting (B6's 104 → keys, the exceptions list, the words you changed and why); the injection-test
output; each T6 check with its planted failure; before/after samples of three pages (same words); the API error
body in English and in the test language; deviations and open questions.
