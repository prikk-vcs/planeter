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

1. A table `xx.rs` with an entry for **every** key (the build's `audit` lists what is missing).
2. A `LangDef` (tag, endonym, `dir`, plural rule, number separators, table) and a line in `shipped()`.
3. A fluent speaker reviews it **before** it ships (RFC 012 O-2); the accessibility statement names the
   reviewer and the language's status.
4. The language switcher and the setter of the `planeter_lang` cookie arrive with the second language; the
   cookie holds a language code and nothing else (RFC 012 D-6f). `Accept-Language` already works.

## What is deliberately not in the catalog (`tests/literals.allow`)

* **Developer-facing text:** the OpenAPI document (English by decision; it says so), log lines, `expect()`
  messages, header values.
* **Tokens and routes:** CSS class names, route paths, template names, media types, the stable machine codes of
  the API's errors, the verify stage status tokens the code matches on (the word shown is the catalog's).
* **Data owned by prikk or by users:** repository, branch and file names, commit messages, prikk's stage names
  and failed-condition texts (prikk's English, shown as it came), the `encoding` token (`text`, `binary`).
* **Punctuation and icons:** the `·` and `/` separators and the 📁 📄 icons (the structural-accessibility
  handoff replaces the icons).
* **One planeter-owned English text outside the catalog:** the `detail` of a verify stage that is
  `not_evaluated` (`blocked by <stage>`) is built in `planeter-core` and is part of the API's JSON; it is shown
  as it comes until the wording handoff restructures it.

Every line of that file has a reason, and the test fails on an entry nothing uses.

## The checks (`tests/catalog_checks.rs`, `tests/i18n.rs`, `tests/pages_in_two_languages.rs`)

A key missing in a shipped language; a key the enum does not have; a duplicate; differing placeholder names;
a counted message whose forms do not fit the language; markup, an entity or a URL in a message; a key nothing
uses; user-visible literal text in a template or in a source file outside the allowlist; `|safe` on anything but
the allowlisted Markdown; a placeholder left unfilled in a rendered page. Each has a planted case in the
tests that proves it can fail.
