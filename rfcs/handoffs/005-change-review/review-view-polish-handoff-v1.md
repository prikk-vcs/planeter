# RFC 005 part 6 — Review view polish before 0.7.0 — Implementation Handoff (v1)

| | |
|---|---|
| Document | The pages of the review view were reviewed Approved (gates green, 1,318 tests, the hostile cases, the browser measurement). Two small things found in the review should be fixed before the release, because each is cheap now and a wire-format change later: **(1)** the engine writes an **English** marker inside the data (`…(N more characters)` in a cut line, `…(N more bytes)` in a cut name) and the page recognises it by its text (dev team deviation 8): a file line that ends with that text triggers a false sentence, and the JSON API carries English in a field every language reads; **(2)** one English sentence uses the internal word *block* ("the block the first patch continues from") while the Japanese already says 履歴上の地点. |
| Status | Assigned 2026-11-17 (SCHEDULE D-81). |
| Basis | `crates/planeter-transport/src/changes/{display,model,engine}.rs`; `crates/planeter-web/src/ui/changes_pages.rs`, `api.rs`, `openapi.rs`; the catalogs (`changes.statement.compared`, `changes.file.cut_lines`); `review-view-pages-review-request.md` deviation 8. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Cut text is data, not words.** In the engine's output a cut line carries the **first 2,000 characters of the line with no marker** and `cut_chars: N` (the number of characters left out; omitted when 0) on the diff line object; a cut name carries the first 1,024 bytes (cut at a character boundary, display-safe) with **no marker** and `path_cut_bytes: N` on the file entry (omitted when 0; the file id is still the hash of the **whole** path bytes). Remove every English marker from the engine and the cache. The **pages** add the marker in the reader's language from the field (a new message per language with the number, plural-aware, in the cell after the text and in the name's heading; plus the existing sentence `changes.file.cut_lines` is shown when any line of the file has `cut_chars`). The **JSON API** documents both fields in OpenAPI (additive, `required` false). **The cache format changes:** bump the key schema (a version component in the cache key) so no earlier entry is read; old entries age out by the LRU.
   Tests: a line of a file that ends with the old English marker text is shown as itself and does **not** produce the cut sentence (this was the bug); a 2,500-character line shows 2,000 characters, `cut_chars` 500, and the page's marker in English and Japanese; a name over 1,024 bytes (hand-made answer; prikk may refuse it) with `path_cut_bytes`; the JSON examples updated; mutation: the page ignores `cut_chars` → fails; the engine puts the marker back → fails.
2. **No "block" in a sentence a reviewer reads.** English `changes.statement.compared`: "Compared with {before} (the point in history the first patch continues from)." Nothing else changes in English or Japanese; the Japanese row stays exactly as it is (so no new row to read). Check by grep that the English catalog has no other *block* in a page sentence of the review view; report what you find (do not change other pages).

## Non-change scope
No other behaviour; no new setting; no sign-off data; `locales/ja.signed` untouched; no version change.

## Acceptance criteria
The two changes with their tests and mutations; the sheet regenerated and `--check` passing, with the **count of messages to read** reported (expected: the 91 plus the new cut-marker message(s); the English-only change adds none); all gates green (fmt, floor, pin, clippy, tests with prikk 0.57.0 and without, deny/audit 0 with the pinned tools, ENF-2, doc, msrv, the dry run with a fresh `CARGO_TARGET_DIR`), CI green.

## Required evidence and review-request format
Commits (hash → what); the changed JSON (before → after) for a cut line and a cut name; the new messages (key → English → 日本語); the tests and mutations; the sheet header and fingerprint; every gate with command, exit code, tool versions and advisory-database HEAD; deviations and open questions.
