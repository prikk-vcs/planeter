# RFC 012 follow-on — Japanese draft: consistency fixes before the reviewers read it — Implementation Handoff (v1)

| | |
|---|---|
| Document | The architect's pre-review of the Japanese catalog (`.git-exclude/reviewed/japanese-pre-review-v1.md`, 2026-10-29; findings 1–5) found words used for two things, five words for *check*, and a few inconsistencies. They are fixed in the draft so the owner and the colleague review a consistent text. **This changes the draft only; the Japanese stays preview-only and unsigned (`JA_REVIEWED` false) — the reviewers' sign-off is untouched and still required.** |
| Status | **Assigned** (2026-10-29). Do it **together with or after** `honest-wording-handoff-v1.md` (same files; one tree). |
| Basis | The pre-review note; `crates/planeter-web/src/catalog/ja.rs`; `docs/glossary.md` §5.1/§5.2; `tests/glossary_ja.rs`, `tests/retired_words.rs`; the reviewers' sheet and its addenda (`.git-exclude/review-requests/japanese-review-sheet*.md`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

1. **`管理者` has one meaning.** Every "このサーバーの管理者" (12 messages) becomes "このサーバーを運用している人" (the form already used in 8 messages; in API hints for developers "運用者" is allowed, as the glossary says); `管理者` remains **only** for an administrator of a repository and always with its qualifier ("リポジトリの管理者"), including `error.forbidden_hide.message` ("…できるのは管理者だけです" → "…できるのはリポジトリの管理者だけです"). Update the glossary row. A test: no message contains "サーバーの管理者" (planted case), and "管理者" never appears without "リポジトリの" before it.
2. **One word per kind of check.** `検査` → `検証` where it means *verify* (`created.verified`, `stage.verifies.yes/no`, `error.creation_verify.message`) and → `チェック` where it names the individual stages (`stage.error.verify`: "失敗したチェック：{conditions}。"); `点検` stays **only** for `doctor` (`recovery.blocker.doctor_problems`, `recovery.note.unreached_objects`) and gets its own glossary row ("リポジトリの点検 = what prikk's `doctor` does; it is not the verification"). A test: `検査` does not occur in any message (planted case).
3. **`ドキュメント`** everywhere (`error.invalid_request.name_hint` becomes: "この応答の field と reason に、断られた値と理由が示されています。API ドキュメントの項目の通りに直して、もう一度送ってください。"). 4. **Glossary:** add the `マージ` row, the `点検` row, and the `運用者` row. 5. **`ID は`** (the space).
Every changed row is listed in the next addendum as REWORDED with the old text beside, **and the sheet says these were the architect's consistency changes so the reviewers may overrule them**. The glossary's status cells for the changed terms stay "Hard term — reviewers decide".

## Non-change scope
No English change; no key renamed; no new message; no sign-off, no `JA_REVIEWED` change; no other word; no version or release change.

## Acceptance criteria
The five fixes in the catalog, glossary and tests (with planted cases both ways); `glossary_ja`, `retired_words`, `plain_words`, `i18n`, `catalog_checks` pass; the pages pass the WCAG checker in both languages; every gate green (fmt, floor, pin, clippy, both test runs, ENF-2, doc; deny/audit as they are while IS-54 is open), CI.

## Prohibited shortcuts
No change of a trust label; no weakening of a test; no unlisted change of a Japanese message.

## Required evidence and review-request format
Commits (hash → what); the before/after table of every changed message key; the addendum rows; every gate with command and exit code; deviations and open questions.
