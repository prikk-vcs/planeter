# RFC 012 follow-on — 「保持」 is not a word for ordinary readers — Implementation Handoff (v1)

| | |
|---|---|
| Document | While reading the 12 flagged messages the owner found the word 保持 strange in the sentences that use it (2026-11-14), and accepted all the others. 保持 ("to hold, to retain") is a formal, technical word; it appears in 19 messages for two different ideas. This handoff replaces it with the plain words that say each idea, in every message that uses it (including messages signed off on 2026-10-31: the owner's new ruling supersedes their earlier approval of that word, and the two `required-pair` pins that said "this wording stays" are updated by this handoff). |
| Status | **WITHDRAWN the same day (2026-11-14), before any work: do not apply it.** The owner ruled that 保持 "is not necessarily bad" and 預かる "is sometimes unnatural": it depends on the context. Of the 12 messages the owner had read, only one contains 保持 (`proposal.patch.not_listed_explain`, written in `prikk-0-56-rebaseline-handoff-v2` D3); the other 18 uses are signed-off text the owner accepted. The architect had over-reached by retiring the word everywhere. Replaced by `japanese-legend-sentence-handoff-v1.md` (one sentence). |
| Basis | `crates/planeter-web/src/catalog/ja.rs`, `crates/planeter-client/src/catalog/ja.rs`; `crates/planeter-web/tests/i18n/ja.rules` (lines 11, 48, 64); `docs/glossary/ja.md` (rows at lines 28, 45, 79, 85 and the word list at line 122); `grep -n 保持` over the repository. |
| Audience | Dev team. Return a short review-request package. |

## The two ideas and their words (the architect's decision)
- **The server keeps someone's work for review or signing ("held")** → **預かる** (預かっています / 預かり中), and from the contributor's side **預ける** (預けられています). Everyday, warm, and exactly the meaning: kept in custody, not decided, not owned.
- **Something is stored as a received copy ("kept as a received copy")** → **保存** (保存します / 保存されます), the plain word for saving a copy. The glossary already keeps 保管 for "kept until a date".
- 保持 is retired from the Japanese catalogs (no replacement of it by 持つ/保有).

## Change scope — exactly these texts (English does not change)
| Key (file) | Japanese now → new (only the changed part shown in bold) |
|---|---|
| `assurance.queued` (web) | ここで**保持中**・未署名 → ここで**預かり中**・未署名 |
| `explain.queued` (web) | このパッチは、このサーバーで**保持されています**。 → このパッチは、このサーバーで**預かっています**。(the rest of the sentence unchanged) |
| `propose.held` (web) | あなたの作業はこのサーバーでレビュー待ちとして保持されており、どのブランチも変わっていません。 → **このサーバーは、あなたの作業をレビュー待ちとして預かっており**、どのブランチも変わっていません。 |
| `proposal.what.open` (web) | レビュー待ちで**保持中**：まだプロジェクトの履歴には含まれていません。 → レビュー待ちで**預かり中**：… |
| `proposal.patches.pending` (web) | {count} 件のパッチは、まだこのサーバーが**保持しており**、署名されていません。 → …まだこのサーバーが**預かっており**、… |
| `proposal.patch.not_listed_explain` (web) | …このサーバーが**保持している**未署名のパッチにも… → …このサーバーが**預かっている**未署名のパッチにも… |
| `error.stage_budget.message` (web) | このサーバーは、保持できる数のアーカイブをすでに預かっています。 → **このサーバーが預かれるアーカイブの数は、すでに上限に達しています。** |
| `new.proposals.any_explain` (web) | …その人たちの作業はレビュー待ちで**保持され**、それだけでブランチが変わることはありません。 → …その人たちの作業はレビュー待ちとして**預かり**、それだけでブランチが変わることはありません。 |
| `stage.ref.kept`, `stage.ref.received`, `stage.none.consequence`, `created.branch.kept`, `created.branch.kept_plain` (web) | 受信したコピーとして**保持**（されます／します／されています） → 受信したコピーとして**保存**（されます／します／されています） — the same inflection, 保持 → 保存 |
| `help.propose` (client) | 作業はレビュー待ちとして保持されます。 → 作業は**レビュー待ちとして、サーバーに預けられます**。 |
| `help.clone` (client) | すべてを受信したコピーとして**保持**し → …**保存**し |
| `propose.done_new`, `propose.done_revision` (client) | 作業はこのサーバーでレビュー待ちとして保持されます。 → 作業は**このサーバーに、レビュー待ちとして預けられています**。 |
| `clone.no_adopt` (client) | すべてを受信したコピーとして**保持**しました。 → …**保存**しました。 |

Also: the comment at the top of `planeter-client/src/catalog/ja.rs` (line 3, a list of terms), `docs/glossary/ja.md` (the rows named in Basis and the word-list line: 預かる = held (pending, on this server); 保存 = saved / stored as a received copy; 保管 = kept until a date; 保持 retired, with the date and "the owner found it strange, 2026-11-14"), and `tests/i18n/ja.rules`: update the reason text of line 11 and the two `required-pair` lines (48, 64) to the new sentences (their reason becomes "the owner's ruling of 2026-11-14: 保持 is retired"), and add **`forbidden	*	保持`** with that reason and one planted bad and one good example. The rules self-test must pass.

## Non-change scope
No other message; no English; no code; `locales/ja.signed` and the sign-off data untouched (the changed messages become "changed since the sign-off" by the baseline and are flagged for the owner to read, as intended).

## Acceptance criteria
The diff of both catalogs is exactly the lines above (show it, old → new, and `grep -rn 保持` over code, tests and docs: only the glossary's retirement note and the rule remain); the new `forbidden` rule fails once when a 保持 is put back (show it); the sheet is regenerated with `planeter-i18n-sheet --lang ja` and `--check` passes; **its header reports the new count of messages to read** (expected: the 12 flagged ones plus the messages above that are not among them; report the number and the new fingerprint); all gates green (fmt, floor, pin, clippy, tests with prikk 0.57.0 and without prikk, ENF-2, doc, deny/audit as the state of the advisory handoff allows: say what is red and why).

## Required evidence and review-request format
Commit (hash → what); every changed row old → new; the failing-once evidence; the `grep`; the sheet header and fingerprint; every gate with command, exit code, prikk version and advisory-database HEAD; deviations and open questions.
