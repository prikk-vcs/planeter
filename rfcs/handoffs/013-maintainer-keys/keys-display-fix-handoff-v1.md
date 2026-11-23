# RFC 013 — The slots sentence reads wrongly for one key — Fix Handoff (v1)

| | |
|---|---|
| Document | Your package `keys-display-review-request.md` (`04e1e82`) is approved except for one sentence the architect found in your own saved page `stage-slots-en.html`: with **one** carried key the creation page says **"Adopting all 1 leaves 63; adopting fewer leaves more."** "All 1" is wrong English, and "adopting fewer" than one key is nothing: a person is told something that reads as a mistake on the page where they decide what the repository will trust. It is the same kind of fault as IS-64 (a counted message with no singular), here on a page that **has** the facility: the web catalog has `plural_msg` (`history.patch_count` is one). The owner has not read this sentence yet, so it is changed now and the reading file regenerated. |
| Status | Assigned 2026-11-23 (SCHEDULE D-116). |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **`stage.key.slots` becomes a plural message** (`plural_msg`, as `history.patch_count`), chosen by the number of carried keys `n`:
   - **one key:** "This repository can trust at most 64 keys. Keys cannot be removed yet, so each key you add uses one for the repository's whole life. Adopting this key leaves {m}." (m = 64 − 1)
   - **two or more:** the present sentence, unchanged ("… Adopting all {n} leaves {m}; adopting fewer leaves more.").
   The Japanese has a natural sentence for each case (the one-key case must not say 1 個すべて): "…この鍵を信頼すると、残りは {m} 個です。" and the present Japanese for two or more. If the facility's Japanese form takes one text for both categories, use two catalog keys chosen by `n` instead and say which you did. The wording rules are the same as before (the glossary; no 保持 or 預かる).
2. **Tests:** the creation page for **1, 2 and 64** carried keys in both languages (the saved pages `stage-slots-{one,two,64}-{en,ja}.html`), the structural rules over them, the word test of D-1 (`keys_words.rs`) still green, and the catalog tests that count plural messages.
3. **The owner's reading file** `keys-0-9-0-reading-order.md` regenerated: the count of messages stays 36 if it is one plural message (say the number the sheet gives), the creation group shows both forms.

## Non-change scope
Nothing else. No other message, page, JSON field or document; no change to `ja.signed`, the `SignOff` or a baseline; no tag, release or upload.

## Two requests that are not part of the change
* **Every commit should build.** `1042612` does not compile alone (its doc test names a function the next commit adds), which breaks `git bisect` across it. Do not rewrite pushed history; from here on order commits so each one compiles and passes `cargo test` for what it touches.
* **A test that failed in my gates and not in yours, unrelated to this work:** `a_light_read_never_waits_behind_a_waiting_heavy_one` (`crates/planeter-web/tests/admission_gate.rs:401`) asserted a wall-clock wait under 100 ms and measured 102 ms in my run without prikk on PATH, while the host's load average was above 70; 30 of 30 runs immediately after passed. It is old and untouched by this work. **Do not change it now**: the architect records it with the other wall-clock assertions (IS-69) to be changed together to ordering assertions.

## Acceptance criteria
The page reads correctly for 1, 2 and 64 keys in both languages; all gates green; CI green on all eight jobs.

## Required evidence
Commits (hash → what); the saved pages for 1, 2 and 64 keys in both languages; the sheet's `--status` line and the regenerated reading file; the gates in a clean worktree (fmt, floor, pin, clippy, tests with prikk 0.58.0 **three times in a row** and without, `cargo deny`, `cargo audit` on a fresh database, ENF-2, doc, msrv); deviations and open questions.
