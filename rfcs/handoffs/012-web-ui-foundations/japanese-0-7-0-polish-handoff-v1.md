# RFC 012 follow-on — Japanese polish of the 0.7.0 messages: 24 messages, exact text — Implementation Handoff (v1)

| | |
|---|---|
| Document | The owner asked the architect to read the 95 messages of the 0.7.0 candidate once more against four criteria (natural in a forge; natural for a patch-based VCS; one term translated one way; no other unnatural Japanese) and **accepted the architect's recommendation to apply the findings before the tag** (2026-11-17). This handoff applies **exactly 24 changed Japanese messages**, with the text below, nothing else. English does not change. The owner then re-reads only these 24 (the sheet will flag exactly them); a second small handoff records that reading as the sign-off and the architect tags that commit. (The architect's note said 26; two of the states, 追加 and 削除, did not change: it is 24.) |
| Status | Assigned 2026-11-17 (SCHEDULE D-87). |
| Basis | `crates/planeter-web/src/catalog/ja.rs`, `crates/planeter-client/src/catalog/ja.rs`; `crates/planeter-web/tests/i18n/ja.rules`; `docs/glossary/ja.md`; the tests that quote Japanese texts (`changes_e2e.rs`, `pages_in_two_languages.rs`, `language_rules.rs`, the client `flows.rs`); the sheet. Current head: the third sign-off record `5c724f4` (or later). |
| Audience | Dev team. Return a short review-request package. |

## The 24 messages (key → new Japanese; use the text exactly, including 。 and the placeholders)

| # | key | new text |
|---|---|---|
| 02 | `error.changes_unavailable.claim_not_held.hint` | このリビジョンの作業はもうレビュー待ちとしてここに残っていないため、プロジェクトの履歴を見るか、リビジョンのファイルをダウンロードして、自分のコンピューターで**読んで**ください。 |
| 03 | `error.changes_unavailable.not_one_change.message` | このリビジョンは**一直線の作業ではない**ため、ファイルごとに**表示できません**。 |
| 04 | `error.changes_unavailable.not_one_change.hint` | **表示できるのは、枝分かれのない一直線の作業だけです。**リビジョンのファイルをダウンロードして、自分のコンピューターで読んでください。 |
| 05 | `error.changes_unavailable.tree_too_large.message` | このリビジョンはファイルが多すぎるか大きすぎるため、ファイルごとに**表示できません**。 |
| 06 | `error.changes_unavailable.tree_too_large.hint` | リビジョンのファイルをダウンロードして、自分のコンピューターで**読んで**ください。 |
| 07 | `error.changes_unavailable.busy.message` | 現在、サーバーはファイルごとの表示**で**混み合っています。 |
| 09 | `error.changes_unavailable.timed_out.message` | ファイルごとの表示に時間がかかりすぎたため、中止しました。 |
| 10 | `error.changes_unavailable.timed_out.hint` | あとでもう一度お試しいただき、何度も起きる場合は、リビジョンのファイルをダウンロードして、自分のコンピューターで**読んで**ください。 |
| 11 | `error.changes_unavailable.failed.message` | ファイルごとに**表示できませんでした**。 |
| 16 | `changes.statement.rebuilt` | ここに表示するファイルは、このリビジョンのパッチから再構築したものです。**ここに表示した内容は、署名されたものではありません。** |
| 24 | `changes.ids.after` | このリビジョンの**終点**：{id} |
| 33 | `changes.col.change` | **変更の種別** |
| 37 | `changes.status.modified` | **変更** |
| 39 | `changes.status.type_changed` | **種類の変更** |
| 49 | `changes.earlier` | **ここより前のファイル** |
| 50 | `changes.more` | **ここより後のファイル** |
| 53 | `changes.file.status` | **変更の種別**：{status} |
| 79 | `changes.file.prev` | **前の変更されたファイルへ** |
| 80 | `changes.file.next` | **次の変更されたファイルへ** |
| 83 | `changes.unavailable.claim_not_held.body` | このリビジョンの作業は、もう提案としてここ**には残っていません**。たとえば、署名された履歴として届いた場合です。代わりに、リポジトリの履歴をご覧ください。 |
| 85 | `changes.unavailable.not_one_change.body` | このリビジョンのパッチは**一直線につながっていない（枝分かれや合流がある）**ため、ファイルごとに表示できません。提案した人に、**一直線につながる形で**送り直してもらってください。 |
| 87 | `changes.unavailable.tree_too_large.body` | このリビジョンはファイルが多すぎるか、大きすぎるため、ファイルごとに表示できません。代わりに、**自分の**コンピューターで読んでください。 |
| 91 | `changes.unavailable.timed_out.body` | 変更点の表示に時間がかかりすぎたため、中止しました。しばらくしてからもう一度お試しいただき、何度も起きる場合は、**自分の**コンピューターで読んでください。 |
| 95 | `propose.revision_keeps` (client) | リビジョンを追加するときは、追加先の提案のタイトルと説明がそのまま使われます。--proposal と --title や --description-file はいっしょに使えません。**--title や --description-file を付けずに実行するか、新しい提案を開くなら --proposal を付けずに実行してください。** |

(#02/#04/#06/#10 and #87/#91 keep every other word of the present text; the bold marks only what changes. Message numbers are the architect's, from the sheet order; use the keys.) All other messages, including #36 追加 and #38 削除, stay as they are.

## Tasks
1. Apply the 24 texts in `crates/planeter-web/src/catalog/ja.rs` and `crates/planeter-client/src/catalog/ja.rs` (#95 only in the client). **Do not change any English text, any key, any placeholder, any other Japanese message.** If the text of a key above is not the one the architect read (the sheet at fingerprint `57d77f1fb0d3689e`), stop and report.
2. **Rules and tests.** Run the language rules and the tests that quote Japanese. If a rule in `tests/i18n/ja.rules` or a glossary test blocks one of the new words (for example a rule about 変更 as a verb catching the bare noun 変更 in `changes.status.modified`), **do not weaken the rule**: stop and report which rule and which text. Update a test only when it pins one of the 24 old texts (list each). Add to `ja.rules` (rules as data, with a reason and a planted bad and good example each): `forbidden * ご自分のコンピューター` (the catalog says 自分のコンピューター); `forbidden * を作れません` and `forbidden * 作られません` for the view's messages are **not** added (too narrow); add `forbidden changes.* 内容が変わった` is **not** added either. Only the first rule is added.
3. **The owner's reading copy.** Regenerate the sheet at the final commit (`planeter-i18n-sheet --lang ja --out .git-exclude/review-requests/japanese-review-sheet.md`; move the previous one into `japanese-review-sheet-superseded/` with its fingerprint in the name), `--check` it, and report its header. **Do not touch the baselines or the `SignOff` data**: the sheet must read "730 unchanged, 24 changed, 0 new — 24 message(s) to read" (754 messages; if the number differs, explain). Also write `.git-exclude/review-requests/japanese-0-7-0-polish-for-the-owner.md`: a table of the 24 (key | English | before | after), generated from the catalogs (a script; keep it in `scripts/`), which the owner reads instead of the whole sheet.
4. **Gates** on the final commit: fmt, floor, pin, clippy, tests with prikk 0.57.0 and without prikk, `cargo deny` and `cargo audit` (the pinned 0.23.0) 0, ENF-2, doc, msrv, the dry run with a fresh `CARGO_TARGET_DIR`; CI green. **The server's start-up log must now say 24** messages are newer than the sign-off (the client's are not counted by the server: the server's own changed messages are 22 and the client's 2: report both numbers and the log line). The client stress target is not re-run (no client logic changed; one message did).
5. `CHANGELOG.md` `[0.7.0]`: the notes currently say the Japanese of the release "was read and accepted by the owner" (the third sign-off record). **Leave them as they are in this handoff**: the next handoff (the record of the owner's reading of the 24) changes them. Do not touch `docs/` otherwise except what a test that pins a text needs.

## Non-change scope
No English; no key; no placeholder; no other Japanese message; no code; no baseline or `SignOff` edit; no version change; no tag; no pin.

## Acceptance criteria
The diff of the two catalogs is exactly the 24 messages (show it, old → new, by key); the rule added fails on a planted bad example; the sheet reads 24 to read and passes `--check`; the owner's one-page table exists; all gates green; CI green; the final commit hash stated.

## Required evidence and review-request format
Commits (hash → what); the catalog diff by key; the tests or rules that needed an edit (each, with why); the sheet header, fingerprint, `--status` line and start-up line; the owner's table; every gate with command, exit code, tool versions and the advisory-database HEAD; deviations and open questions.
