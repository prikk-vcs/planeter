# RFC 012 follow-on — One voice for "nothing was done" in two new Japanese pages — Implementation Handoff (v1)

| | |
|---|---|
| Document | While checking the ten messages that are newer than the Japanese sign-off against the owner's philosophy (2026-11-13), the architect found one inconsistency. The signed-off pages say "nothing was done" in the passive and without a subject: `page.csrf.body` «…何も行われていません。», `page.empty_field.body` (same). The two new pages say it with the address or the content as the subject: «このアドレスは、…何も行いませんでした。» and «送られた内容が、…何も行いませんでした。» — an address or a piece of content does not "do" anything, and one fact is now said in two voices. This handoff aligns the two new pages with the signed-off ones. |
| Status | **Done, reviewed Approved (2026-11-13; commit `03fca11`).** (Originally: Assigned 2026-11-13.) |
| Basis | `crates/planeter-web/src/catalog/ja.rs` (`page.method_not_allowed.body`, `page.payload_too_large.body`); the signed-off pages `page.csrf.body` and the empty-field page (grep `何も行われていません`); `docs/glossary.md` §5.3. |
| Audience | Dev team. Return a short review-request package (`.git-exclude/review-requests/`). |

## Change scope
Exactly two Japanese texts. English does not change.

| Key | Now | New |
|---|---|---|
| `page.method_not_allowed.body` | このアドレスは、その種類のリクエストを受け付けないため、**何も行いませんでした**。前のページに戻って、ページのリンクやボタンから操作してください。 | このアドレスは、その種類のリクエストを受け付けないため、**何も行われていません**。前のページに戻って、ページのリンクやボタンから操作してください。 |
| `page.payload_too_large.body` | 送られた内容が、このサーバーが一度に受け付ける大きさを超えているため、**何も行いませんでした**。小さくして送り直すか、このサーバーを運用している人に尋ねてください。 | 送られた内容が、このサーバーが一度に受け付ける大きさを超えているため、**何も行われていません**。小さくして送り直すか、このサーバーを運用している人に尋ねてください。 |

- Add a rule to `tests/i18n/ja.rules` (rules as data): no Japanese message contains `何も行いませんでした`; the "nothing was done" sentences of the pages use `何も行われていません`. Keep the rule's reason in the file, one line.
- Regenerate the review sheet with `planeter-i18n-sheet --lang ja` and run `--check`. Both keys are already "new" (not yet read by the owner), so the count stays **10 messages to read**; report the new fingerprint and the two old → new rows.
- Do **not** touch the baseline `locales/ja.signed`, the `SignOff` data, any other message, any code, or any version.

## Acceptance criteria
The two rows as above and nothing else changed in any catalog (diff of both catalogs shows exactly those two lines); the new rule fails if the old wording comes back (show it failing once); the sheet header still reads "651 unchanged, 2 changed, 8 new — 10 message(s) to read"; every gate green (fmt, floor, pin, clippy, both test runs, deny and audit 0, ENF-2, doc), CI green.

## Required evidence and review-request format
Commit (hash → what); the two rows old → new; the failing-once evidence; the sheet header and fingerprint; every gate with command, exit code and advisory-database HEAD; deviations and open questions.
