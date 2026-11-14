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
| `proposal.patch.not_listed_explain` | このパッチは、このサーバーが**保持している**未署名のパッチにも、プロジェクトの履歴にも**ない**ため、何も言えません。 | このパッチは、このサーバーが**持っている**未署名のパッチにも、プロジェクトの履歴にも**見当たらない**ため、何も言えません。 |

Why this wording: 持っている is the everyday verb for "has" (nothing to be held in custody); 見当たらない ("cannot be found") is also the honest tense of the claim: the server looked and did not find the patch there, which is true in the received-history case too, where the patch exists but not as one of these two things. The badge beside it (「一覧にありません」) is unchanged.

- Update `docs/glossary/ja.md` for this row, if it quotes the sentence (the "not listed" legend row), with the date and "the owner found 保持 strange in this sentence, 2026-11-14; 保持 stays elsewhere".
- Regenerate the sheet with `planeter-i18n-sheet --lang ja` and run `--check`. The key was already among the 12 to read, so the count stays **12**; report the new fingerprint and the old → new row.
- Do **not** touch `locales/ja.signed`, any other message, any rule other than what a rules file needs if it quotes the old sentence (list it), any code or version.

## Acceptance criteria
The diff of both catalogs is exactly that one line; the sheet header still reads 12 messages to read; `--check` passes; all gates green (fmt, floor, pin, clippy, tests with prikk 0.57.0 and without prikk, ENF-2, doc; deny/audit as the advisory handoff allows: say what is red and why).

## Required evidence and review-request format
Commit (hash → what); the row old → new; the sheet header and fingerprint; every gate with command, exit code, prikk version and advisory-database HEAD; deviations and open questions.
