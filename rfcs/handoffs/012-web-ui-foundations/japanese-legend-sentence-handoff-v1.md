# RFC 012 follow-on — One legend sentence in plain Japanese — Implementation Handoff (v1)

| | |
|---|---|
| Document | The owner read the 12 flagged Japanese messages and found the word 保持 strange. Of those 12, exactly one contains it: `proposal.patch.not_listed_explain`, the legend of the badge 「一覧にありません」 (my D3 wording: 「このパッチは、このサーバーが保持している未署名のパッチにも、プロジェクトの履歴にもないため、何も言えません。」). The owner's ruling on the word is **contextual**: 保持 is not wrong as such (the badge 「ここで保持中・未署名」 and its explanation, and the other signed-off uses, stay exactly as they are), and 預かる is sometimes unnatural (a server does not "take into custody" a patch), so it is **not** introduced. This handoff changes the one sentence. It supersedes `japanese-held-words-handoff-v1` (withdrawn). |
| Status | Assigned 2026-11-14. |
| Basis | `crates/planeter-web/src/catalog/{en,ja}.rs` (`proposal.patch.not_listed_explain`); `docs/proposals.md`; the sheet. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
Exactly one Japanese message (English does not change):

| Key | Now | New |
|---|---|---|
| `proposal.patch.not_listed_explain` | このパッチは、このサーバーが**保持している**未署名のパッチにも、プロジェクトの履歴にも**ない**ため、何も言えません。 | **このパッチに該当するものは、このサーバーの、未署名のパッチにもプロジェクトの履歴にも見つかりません。そのためパッチの状態を表示できません。** |

**The owner dictated this text (2026-11-14); use it exactly as written, including the two sentences and the final 。.** It replaces both the 保持 (not natural here) and 「何も言えません」 ("I cannot say anything": not natural; the page does not fail to speak). The phrase 何も言えません is used nowhere else in the catalogs (checked). The badge beside it (「一覧にありません」) is unchanged.

- Update `docs/glossary/ja.md` for this row, if it quotes the sentence (the "not listed" legend row), with the date and "the owner found 保持 strange in this sentence, 2026-11-14; 保持 stays elsewhere".
- Regenerate the sheet with `planeter-i18n-sheet --lang ja` and run `--check`. The key was already among the 12 to read, so the count stays **12**; report the new fingerprint and the old → new row.
- Do **not** touch `locales/ja.signed`, any other message, any rule other than what a rules file needs if it quotes the old sentence (list it), any code or version.

## Acceptance criteria
The diff of both catalogs is exactly that one line; the sheet header still reads 12 messages to read; `--check` passes; all gates green (fmt, floor, pin, clippy, tests with prikk 0.57.0 and without prikk, ENF-2, doc; deny/audit as the advisory handoff allows: say what is red and why).

## Required evidence and review-request format
Commit (hash → what); the row old → new; the sheet header and fingerprint; every gate with command, exit code, prikk version and advisory-database HEAD; deviations and open questions.
