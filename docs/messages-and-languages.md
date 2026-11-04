# Messages and languages

Every word the web UI and the API's error bodies say lives in **one place**, the message catalog
(`crates/planeter-web/src/catalog/`; the command-line program's are in `crates/planeter-client/src/catalog/`), so that a language is
a row of data and a table and not a redesign (RFC 012 D-3, amended 2026-11-04). This page is for the person who adds a message, a
placeholder or a language. The languages themselves — their tag, direction, plural rule, separators and **sign-off record** — are data in
one shared crate, `crates/planeter-i18n` (`languages/`), the only list of languages; both programs read it.

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
4. **Counts use plural messages.** A message with a count has one form per plural category its language uses and `{count}` in each
   (`plural_msg` for English's `one` and `other`; `Entry::with_forms` for more); the caller writes `.n(count)` and never names a
   category. The categories are CLDR's (`zero`, `one`, `two`, `few`, `many`, `other`); `planeter-i18n` has the rule functions of
   English, Japanese, Russian, Polish, Arabic and Welsh, tested, and `audit` checks that an entry has **exactly** the forms its language
   declares. A language whose rule is not there is where a fluent-style library is reconsidered (five languages or more).
5. **Values are not words.** A user name, a path, an id, a size's number, prikk's own text (stage names, commit
   messages, failed-condition texts) are data, passed as placeholders. The product name `planeter` is a value
   (`T::product`).
6. **A missing key is never a crash and never a silent English fallback**: it renders as `[the.key]` and is
   logged; the tests fail before it can ship.

## Change a message's words

**Wording changes go through the glossary** ([`glossary.md`](glossary.md)). It is the owner-approved list of the
words the interface uses; its section 2 has a row for each message it governs, with the English text now in
force. Change the row and the catalog in the same commit: `tests/glossary.rs` fails when they disagree, and
`tests/language_rules.rs` (rules in `tests/i18n/en.rules`) fails when a word the plain-words work retired (`unsealed`, `forge-approved`,
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

## Adding a language (a checklist; no code change outside data)

A language is **a row of data, a catalog in each program, a rules file, a glossary file, and — when the reviewers have read it — a
sign-off record**. Nothing in a handler or a template names a language. Follow the steps in order; each one is checked by a test, and a
language that skips one fails the build with a sentence that says which.

1. **The tag.** A BCP 47 tag: `de`, `zh-Hant`, `pt-BR`. Matching is by BCP 47 (`planeter_i18n::tag`): exact (case-insensitive) → the same
   language and script → the same language (the first registered) → the source language. `zh-TW` gets `zh-Hant`, `zh-CN` gets `zh-Hans`,
   `pt-PT` gets `pt-BR` when only that ships. Register `zh-Hans` and `zh-Hant` as two languages, not one `zh`.
2. **The row.** `crates/planeter-i18n/src/languages/<tag>.rs`: a `Language` — tag; **endonym** (the language's name in itself, shown on the
   switcher); direction (`Dir::Ltr` or `Dir::Rtl`); the **plural categories** it uses and the **rule** that picks one (CLDR:
   <https://cldr.unicode.org/index/cldr-spec/plural-rules>; `plural::{en, ja, ru, pl, ar, cy}` exist and are tested — add yours with
   test vectors for 0, 1, 2, 3, 5, 11, 21, 100, 101 and a million); the group and decimal separators; and `status: Status::Draft`. Add it to
   `languages::LANGUAGES` (after the source language; the order is the switcher's). `check_registry` fails an unsound row.
3. **The catalogs.** `crates/planeter-web/src/catalog/<tag>.rs` and `crates/planeter-client/src/catalog/<tag>.rs`, each with
   `pub static DEF: LangDef = LangDef::new(&planeter_i18n::languages::<tag>::<TAG>, CATALOG)` and an entry for **every** key in the English
   order, with exactly the plural forms the language declares; add the catalog to `CATALOGS` (and `baseline_text` once it is signed off).
   `cargo test -p planeter-web --test languages` and `-p planeter-client --test catalog` say what is missing.
4. **The rules file.** `crates/planeter-web/tests/i18n/<tag>.rules` (both programs read it): the terms the language must not use, its style
   conventions (full-width or half-width punctuation, the space before a counter…), and the wording that must stay; one rule per line, each
   with planted examples that prove it can fail (`planeter_i18n::rules`). The source language's retired words apply to every language. An
   empty file is allowed, with a comment that says so; a missing one fails `language_rules`.
5. **The glossary file.** `docs/glossary/<tag>.md` with the three parts the tests read: **Status**, **Terms** (each concept's term, which
   are hard terms the reviewers decide, the alternatives) and **Messages that carry a term or a trust label** (the catalog's text,
   character for character; `tests/glossary_lang.rs`). The English meaning of every trust label is fixed; a translation keeps it exactly.
6. **The sheet's instructions and notes.** `tools/i18n-sheet/data/<tag>/instructions` — an English section and a section in the language
   itself, written by whoever adds the language (`{{draft wording|signed-off wording}}` follows the status) — and optionally
   `data/<tag>/notes.tsv`; add a line to `LANGUAGE_DATA`.
7. **Draft → preview → sheet → named reviewers → sign-off.** A draft is **not** shipped: it is offered only through the operator's preview
   (`PLANETER_PREVIEW_LANGUAGES=<tag>`; logged at start) so that the reviewers can see the pages. Generate the sheet
   (`cargo run -p planeter-i18n-sheet -- --lang <tag> --out FILE`), give it to the reviewers — **at least one fluent reader, recorded by
   name or role** — integrate their corrections (one commit per group), regenerate and `--check` it. Then, in one commit, change the status
   to `Status::SignedOff(SignOff { reviewers, date, commit, sheet_fingerprint, note })` (a test refuses a sign-off without a reviewer, a date,
   a commit and a 16-hex fingerprint) and write the **baseline** files `crates/planeter-web/locales/<tag>.signed` and
   `crates/planeter-client/locales/<tag>.signed` with `planeter-i18n-sheet --baseline <tag> web|client` (a test refuses a signed-off language
   without them). The accessibility statement names the reviewers and gets the language's version of it.
8. **A right-to-left language** additionally needs a **real** right-to-left page read by a fluent reader before it ships: the logical-property
   checks of the stylesheet and the test language `qaa` prove that nothing overlaps, not that the page reads right, and the accessibility
   statement says today that no right-to-left language has been rendered for real. Numbers and units inside a right-to-left sentence
   (`223 B`) in particular need that reader.
9. **The switcher scales by itself.** Up to three languages: one inline button each. Four or more: one `<details>` "Language" listing the
   endonyms, each still a no-script form with `lang` on its button, the current one marked and the navigation block keeping its name
   (`tests/language_switching.rs`, `tests/language_menu.rs`).
10. **Who may propose a translation.** Any person, as a proposal to this project (a catalog, a rules file and a glossary file are ordinary
    files); the **sign-off is the maintainers'** — the people the project names as reviewers of that language — never the author's alone.

### After a sign-off: what is flagged until a reviewer has read it

A sign-off covers the messages **as they were signed**: the baseline file lists `key<TAB>hash` of each. A message written or changed
afterwards **ships in that language** (the dev team writes it, the glossary's rules apply) **and is flagged until a reviewer has read it** —
nothing is silently counted as reviewed. The sheet marks every row `unchanged`, `changed since sign-off` or `new since sign-off`; the
header counts them; the server logs the count once at start ("language ja: 10 message(s) are newer than its sign-off…"); and
`planeter-i18n-sheet --status` prints one line per signed-off language for the **release notes** (RELEASING says to list it). When the
reviewers have read the flagged rows, the baselines are regenerated and the record gets a new line.

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

## The checks (`tests/catalog_checks.rs`, `tests/languages.rs`, `tests/i18n.rs`, `tests/pages_in_two_languages.rs`, `tests/glossary.rs`, `tests/glossary_lang.rs`, `tests/language_rules.rs`)

A key missing in a shipped language; a key the enum does not have; a duplicate; differing placeholder names;
a counted message whose forms do not fit the language; markup, an entity or a URL in a message; a key nothing
uses; user-visible literal text in a template or in a source file outside the allowlist; `|safe` on anything but
the allowlisted Markdown; a placeholder left unfilled in a rendered page; a glossary row whose *Current* text is
not the catalog's; a retired word in a message. Each has a planted case in the tests that proves it can fail.

## The review sheet (generated, for any language; there is one per language, and nobody writes an addendum)

The reviewers read **one sheet, generated from the live catalogs**, so that what is reviewed is exactly what ships. For Japanese it was a
first sheet plus thirteen addenda whose rows replaced one another; those are kept, out of the reviewers' way, in
`.git-exclude/review-requests/japanese-review-sheet-superseded/`.

```sh
cargo run -p planeter-i18n-sheet -- --lang ja --out .git-exclude/review-requests/japanese-review-sheet.md   # write it
cargo run -p planeter-i18n-sheet -- --check .git-exclude/review-requests/japanese-review-sheet.md           # exit 1, naming each difference, if it drifted
cargo run -p planeter-i18n-sheet -- --status                                                               # for the release notes
cargo run -p planeter-i18n-sheet -- --baseline ja web | client                                             # the baseline of a catalog as it stands
```

* **What is in it.** Every message of the server's catalog and of `planeter-client`'s, once: key · where it appears · English · the language's
  text · placeholders · a note for the reviewer · (for a signed-off language) **since sign-off** · an empty "reviewer's correction". The
  English and target cells are the catalogs' text (escaped for a table cell; `tests/sheet.rs` of `tools/i18n-sheet` fails if a cell differs, a
  message is missing, a key is not in the catalogs, a row is repeated, in the wrong group or has a stale status). The header carries the
  language, the commit, the date, the count per group, the counts since the sign-off and a **fingerprint** of all the wording (it changes
  when any shown text changes, not when a note or a status does; it is not a cryptographic hash).
* **Groups** (the reviewer may stop after any): **A** the words that carry trust (badges and their explanations, the review line and
  decisions, the states of a proposal, the key-trust screens of repository creation); **B** the pages a person reads, page by page, in the
  order they are met; **C** the command-line program; **D** developer- and operator-facing texts (`error.*`, `quota.*`, `recovery.*`, the
  answer to a proposal) — **optional for the sign-off**. The first page says in English and in the language itself what to do (from
  `data/<tag>/instructions`; "a draft" while the language is a draft, "the current text" once it is signed off), lists the hard terms of
  `docs/glossary/<tag>.md` as rows to decide first, and what happens next.
* **Maintained tables** live next to the tool (`tools/i18n-sheet/data/`): `contexts.tsv` (shared) maps a key (or a prefix ending in `*`; the
  longest match, an exact key first) to its group, page and "where it appears" — **a new message that matches no line makes the generator
  fail until a line is added**; `<tag>/notes.tsv` holds the notes for the reviewer (a note for a key that no catalog has fails).
  The next feature that adds a message adds its line, regenerates the sheet and runs `--check`.
* **Keeping group D in English.** `ENGLISH_ONLY_KEYS` in `catalog/mod.rs` is a list of keys that are English in every language: such a key
  is looked up in the English table whatever the caller's language (and with English plural rules), the other tables do not hold it,
  `audit` fails if the list names a key English lacks or a key another table also holds, and the tests that compare the key sets allow
  exactly that list. **It is empty.** Turning group D off is naming its keys there (and deleting them from the other catalogs): a one-line
  change for the architect once the owner has said so; the generated sheet then has no group D, because it lists the target catalog's messages.

**From corrections to the sign-off.** (1) Each group's corrections become edits of the language's catalogs (both programs) and of its
glossary file, **one commit per group**; a term decided at the top of the sheet is applied wherever it is used, in the commit that decides it.
(2) The sheet is regenerated and `--check`ed. (3) The language's status in the registry becomes `SignedOff` with its record — who reviewed, the
date, the commit and the sheet fingerprint — and the baselines are written, in one commit. (4) **Phase 2** (for Japanese, done): the
switcher always renders one button per available language (no code path hides a lone one), and `accessibility.md` has the language's
version. The first page of every generated sheet lists these steps.
