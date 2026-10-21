# Messages and languages

Every word the web UI and the API's error bodies say lives in **one place**, the message catalog
(`crates/planeter-web/src/catalog/`), so that a language is a table and not a redesign (RFC 012 D-3). This page
is for the person who adds a message, a placeholder or a language.

## The rules

1. **A sentence is one message.** Never join two keys into one sentence; two sentences next to each other are
   two messages. Word order differs between languages, so a message takes **named placeholders** (`{name}`),
   never a position.
2. **No markup, no URL, no entity in a message** (`<`, `>`, `&`, `http://`, `https://` fail the build). A sentence
   that contains a link or emphasis takes a **typed slot**: `Slot::Code`, `Slot::Muted` or `Slot::Link`. The
   catalog supplies the words; the **code** supplies the label and the `href`. Text, code and muted values are
   HTML-escaped; a link becomes an element only for a local path (`/…`) or an `http(s)` URL.
3. **Never `|safe` on catalog output.** `Msg::html()` returns a `Rich` that askama prints without a second
   escape; nothing else needs `|safe`. The only `|safe` in the templates is the sanitised Markdown of a README,
   listed in `tests/literals.allow`.
4. **Counts use plural messages.** A message with a count has a singular and an other form and `{count}`
   (`plural_msg`); the caller writes `.n(count)` and never names a category. English has `one` and `other`;
   Japanese will have only `other`. A language that needs more categories (`few`, `many`) is where a
   fluent-style library is reconsidered (RFC 012 D-3).
5. **Values are not words.** A user name, a path, an id, a size's number, prikk's own text (stage names, commit
   messages, failed-condition texts) are data, passed as placeholders. The product name `planeter` is a value
   (`T::product`).
6. **A missing key is never a crash and never a silent English fallback**: it renders as `[the.key]` and is
   logged; the tests fail before it can ship.

## Change a message's words

**Wording changes go through the glossary** ([`glossary.md`](glossary.md)). It is the owner-approved list of the
words the interface uses; its section 2 has a row for each message it governs, with the English text now in
force. Change the row and the catalog in the same commit: `tests/glossary.rs` fails when they disagree, and
`tests/retired_words.rs` fails when a word the plain-words work retired (`unsealed`, `forge-approved`,
`prikk-verified`, `not_evaluated`, `evaluated`, `refs`, `SSO`, or an old navigation label) comes back (a
legitimate use goes in `tests/retired_words.allow`, with its reason). A message the glossary does not list is
new wording: propose it to the architect, do not choose it silently.

## Add a message

1. Add a variant to `keys!` in `catalog/mod.rs` (`PageNewThing => "page.new_thing"`): the identifier is stable
   and dotted (`area.name`).
2. Add the entry to every shipped language table (`en.rs` now): `msg("page.new_thing", "…")`, or
   `plural_msg(…)` for a counted one.
3. Use it: in a template, `{{ t.get(Key::PageNewThing) }}` for plain text, or
   `{{ t.msg(Key::X).with("name", value).html() }}` for a sentence with slots; in Rust, `c.t.get(Key::…)`.
4. `cargo test -p planeter-web --test catalog_checks` tells you what is missing, unused or malformed.

## Add a placeholder

Write `{name}` in the English text and fill it at the call site (`.with("name", v)`, `.code(…)`, `.muted(…)`,
`.link("name", label, href)`, `.n(count)` for `{count}`). Every language's text for that key must use the same
names; the order and the words around them are the translator's.

## Add a language

1. A table `xx.rs` with an entry for **every** key, in the English order (the build's `audit` lists what is
   missing; the order lets the reviewers' corrections map back one to one).
2. A `LangDef` (tag, endonym — the language's name in itself, shown on the switcher —, `dir`, plural rule, number
   separators, table). Japanese is `JA_DEF`: one plural form (`one == other`), `1,200` grouping.
3. **Sign-off before shipping (RFC 012 D-3f).** A new language is a *draft*: it is **not** in `shipped()`. It is
   reached only through the operator's preview (`PLANETER_PREVIEW_LANGUAGES=<tag>`, listed in `previewable()`;
   off by default; logged at start). Steps: write the draft; add the language's section to the glossary (terms,
   hard terms flagged, a message table that `tests/glossary_ja.rs`-style tests compare with the catalog); give
   the reviewers **the sheet** — one row per message (key | where it appears | English | draft | placeholders |
   note | their correction), the trust labels first, then error pages, then the rest, every row `needs-review`;
   integrate their corrections (the sheet's rows map to catalog keys one to one); then, in one commit, set
   `JA_REVIEWED = true` **and** record the sign-off in `JA_SIGNOFF` (a test refuses one without the other, and
   refuses a language in `shipped()` that is not signed off). The accessibility statement names the reviewers.
4. The checks cover a draft too: the catalog audit, the retired-words test (the English retired words may not
   appear in a Japanese message, and a short list of forbidden Japanese terms: loan words for *seal*, blaming and
   gendered wording), the glossary's Japanese tables, the structural checker over every page state (the
   `…_in_japanese` tests; `STRUCTURE_LANG=ja` renders every state of every structure test in Japanese), and the
   stylesheet tests.
5. **The language switcher** is in the footer of every page, the sign-in page included: one `POST /lang` form per
   available language, no script. It is a CSRF-protected form like the others; it sets the `planeter_lang` cookie
   (a language code and nothing else; `Path=/; Secure; SameSite=Lax`) and redirects to the page the visitor was on
   (only a local path is accepted back). A request for a language that is not available is refused. The route is
   in the IS-14 table as public (it names no repository). `Accept-Language` negotiation (`ja`, `ja-JP`) works as
   before.

## What is deliberately not in the catalog (`tests/literals.allow`)

* **Developer-facing text:** the OpenAPI document (English by decision; it says so), log lines, `expect()`
  messages, header values.
* **Tokens and routes:** CSS class names, route paths, template names, media types, the stable machine codes of
  the API's errors, the verify stage status tokens the code matches on (the word shown is the catalog's).
* **Data owned by prikk or by users:** repository, branch and file names, commit messages, prikk's stage names
  and failed-condition texts (prikk's English, shown as it came), the `encoding` token (`text`, `binary`).
* **Punctuation and icons:** the `·` and `/` separators and the 📁 📄 icons (the structural-accessibility
  handoff replaces the icons).
* **Planeter-owned English text outside the catalog, in the API's JSON only:** the `detail` of a verify stage
  that is `not_evaluated` (`blocked by <stage>`) or `halted` (`after <stage>`) is built in `planeter-core`. The
  page shows neither: it builds "not run (blocked by {check})" and "stopped early (after {check})" from the
  stage's `blocked_by` and `after` fields, through the catalog.

Every line of that file has a reason, and the test fails on an entry nothing uses.

## The checks (`tests/catalog_checks.rs`, `tests/i18n.rs`, `tests/pages_in_two_languages.rs`, `tests/glossary.rs`, `tests/retired_words.rs`)

A key missing in a shipped language; a key the enum does not have; a duplicate; differing placeholder names;
a counted message whose forms do not fit the language; markup, an entity or a URL in a message; a key nothing
uses; user-visible literal text in a template or in a source file outside the allowlist; `|safe` on anything but
the allowlisted Markdown; a placeholder left unfilled in a rendered page; a glossary row whose *Current* text is
not the catalog's; a retired word in a message. Each has a planted case in the tests that proves it can fail.
