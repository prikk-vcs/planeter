# RFC 010 — Release 0.10.0: the Japanese reading, its corrections and its record — Implementation Handoff (v1)

| | |
|---|---|
| Document | **The architect has read the 160 Japanese messages newer than the sign-off of 2026-11-25**, on the owner's delegation of 2026-11-28 ("please take charge of reviewing the Japanese dictionary … if you encounter any items that you are unable to decide on yourself … isolate those"). **No item was left that the architect could not decide.** 125 are accepted as they are; **35 are corrected**; and **3 messages that were signed** are corrected for one word (IS-73). This handoff has the corrections (exact), the rules that keep them from coming back, and the record of the reading. |
| Status | Assigned 2026-11-28 (SCHEDULE D-150). **Do it in the same re-roll as `release-0-10-0-candidate-fix-handoff-v1`**: one new candidate, one release-tier run. Commit order: that handoff's commits, then **A** (the corrections and the rules), then **B** (the record). |
| Basis | `docs/glossary/ja.md` (the terms, "One wording, one standard"); `crates/planeter-web/tests/i18n/ja.rules`; `docs/messages-and-languages.md` ("The record of Japanese"); `release-0-9-0-signoff-and-tag-handoff-v1.md` (the shape of A and B); `japanese-0-10-0-reading/corrections.tsv` (**the exact texts; read it**). |
| Audience | Dev team. Return a review-request package. |

## What was decided, and why (so that you can apply it with understanding)

1. **One word for "linked": 連携 (not 結び付ける).** The signed text already says it (`page.sso_unlinked.*`: 「…アカウントに連携されていません」), and one thing has one word. So: 連携する / 連携を解除する / 連携している.
2. **A role is not "taken" or "confiscated": 取り消す** (the word the token messages already use), never 取り上げる (it reads as a punishment; the glossary's tone rule is calm and never blaming). **"A role of its own" is a role given to the account directly: 直接与えられた権限** (`docs/accounts.md`: the owner and an organization's administrators count separately). A role is always written 権限「{role}」, in every sentence that names one.
3. **No idiom** (the glossary's rule): 手を付けません → 影響しません; 届く ("reach") → アクセスできる / 対象です; 限れる → 指定できる.
4. **The date line says what it means:** 「{date} まで有効です。」 (not 「{date} までです。」).
5. **Brackets are full-width, with no space after one** (the glossary's rule): `{handle}（{name}）`, `（HANDLE）は`, `（{why}）`; and `ログインのプロバイダー経由でログインしてください` (not 「…プロバイダーでログイン…」, which says *login* twice in one breath).
6. **"count" is not 数える:** the handle comparison includes organizations' and repository owners' names (`handle_taken`).
7. **A password that is kept is kept as it is: そのまま残ります**, as the start-up line already says (not 保存されます, which says it is saved now).
8. **IS-73, settled by delegation (the Japanese part):** the person who runs the server is, in a sentence for people, **このサーバーを運用している人** (the glossary's word). Three signed sentences said このサーバーの運用者: `repo.keys.source.operator`, `operator.done`, `operator.done.recorded`. They are changed, and the corrections table says so. 運用者 stays in the two API hints for developers (the glossary's rule). **The English "single sign-on identity" of `page.sso_unlinked.body` stays**: the words name different things (the button, the provider), the glossary holds both, and a change of signed English is not worth the churn. IS-73 is closed with that sentence in SCHEDULE.
9. **Kept as written, read and accepted** (so that nobody reopens them): the four role words (読み取り / 書き込み / メンテナー / リポジトリの管理者), the three scope words (読み取り専用 / 書き込み / 完全なアクセス), the plain-sentence fallbacks, "who did it" (あなた / このサーバーを運用している人 / ログインのプロバイダー), 出来事 for *events*, ログイン中の状態 for a sign-in that is still on, ID for an identity at the provider (the signed page's word), 誰/だれ as they are (the signed text uses both).

## Change scope

**A — the corrections and the rules** (one commit):
1. **`crates/planeter-web/src/catalog/ja.rs`: the 38 messages of `corrections.tsv`**, each **character for character** as its `new` column says (the `\n` in a cell is a line break; the other characters are literal). English and the keys do not change. No other message changes.
2. **A test that is a script's work, not a person's:** read the TSV, and for each row assert (a) the catalog's value **before** the change equals `old` (run it once on the parent commit and show it) and **after** equals `new`; (b) the placeholders (`{like_this}`) of old and new are the same multiset; (c) the set of keys that differ between the parent's and the new catalog is **exactly** the 38. Keep the check as a test (`tests/ja_corrections_0_10_0.rs`) only if it stays small and costs nothing; otherwise run it, put its output in the report, and do not add it.
3. **`crates/planeter-web/tests/i18n/ja.rules`: three forbidden words, each with its reason and its planted bad and good examples** (the file's format, held by `tests/language_rules.rs`): `結び付` (linked is 連携: one word, D-150), `取り上げ` (a role is revoked, 取り消す: no punishment), `手を付` (an idiom: say what happens). The catalog must pass all three; a mutation (put one back) must fail the rule test: show it.
4. **`docs/glossary/ja.md`:** add the terms **link / unlink (an identity to an account): 連携する / 連携を解除する**, **role (given directly): 直接与えられた権限**, and say in "One wording, one standard" that the person who runs the server is このサーバーを運用している人 in every sentence for people. The test that holds that page against the catalogs must pass.
5. **The sheet and the reading order are regenerated from the catalogs** (the corrections change their text and their fingerprint, by design: **the fix handoff's line "the sheet's fingerprint must be unchanged" is superseded by this one**; the new fingerprint is reported).

**B — the record** (a second commit, after A is on `main` and its tests are green):
6. **The `SignOff` of `ja`** (`crates/planeter-i18n/src/languages/ja.rs`), both baselines (`planeter-i18n-sheet --baseline ja web|client`, by the tool, never by hand), `docs/messages-and-languages.md` ("The record of Japanese", a seventh line), `docs/glossary/ja.md` ("Status"), `README.md`'s Languages line, the CHANGELOG's Japanese bullet. **What the record must say, truthfully:**
   - `reviewers`: **"the architect of the project (an AI assistant, Claude), reading on the owner's delegation of 2026-11-28"**. Not the owner and not a colleague: the earlier six readings were the owner's and that stays true of them.
   - `date` 2026-11-28; `commit` = commit A; `sheet_fingerprint` = the regenerated sheet's.
   - `note`: one paragraph: the seventh reading; the 160 messages newer than the sign-off of 2026-11-25 (the operator's account and token commands, the account's own log, the sign-in provider's settings and lines, the page for a disabled account) were read in full against the glossary; **35 of them and 3 signed messages were corrected (corrections.tsv)**; 125 were accepted as they stood; **the reader is not a native speaker of Japanese and says so**; a reader of Japanese who finds a sentence unnatural should say so and it is a one-line fix (a catalog edit, a regenerated sheet, a new line of this record).
   - The CHANGELOG's Japanese bullet changes from "160 … ship flagged" to a sentence of the same facts: **who read them (the line above), how many were corrected, that the owner's six earlier readings stand, and that the flag no longer applies**; the status line of `planeter-i18n-sheet --status` is quoted as the tool prints it. `README.md`'s "not yet read" becomes the same sentence in one line. **No claim that the Japanese is "native-reviewed" anywhere.**
   - The server's start-up count line (`language ja: N message(s) are newer than its sign-off…`) no longer appears: show that (the report quotes the log of a start).

## Non-change scope
No English change; no key added or removed; no message other than the 38; no change to a rule's meaning, a placeholder or a plural form; no dependency; no `unsafe`; **the record names no human (it says who read, by role)**. No tag; the owner's authorization of the tag is the owner's. No edit of `SCHEDULE.md`, `STATUS.md`, the ledgers or the threat model.

## Acceptance criteria
The catalog differs from the parent in exactly the 38 keys and each equals the TSV's `new`; the three rules exist with planted examples and fail on a mutation; the glossary page and its tests pass; the generated sheet passes `--check` and says "897 + 160 − …" in its header as the tool prints it (report it); `planeter-i18n-sheet --status` says **0 messages newer than the sign-off**; the record says only what is above; nothing else changed (`git diff --stat` over A and B).

## Required evidence
Commits (hash → what); the TSV check's output on the parent and on A; the three rules' planted examples and the mutation; the new sheet's header and fingerprint; `--status` before and after; a server start's log lines (no flagged count); the diff of the CHANGELOG and README sentences; deviations and open questions. Final reply: the review-request path only.
