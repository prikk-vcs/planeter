# RFC 012 follow-on — Japanese: the final consistency fixes, the error responses that bypass the catalog, and the sign-off — Implementation Handoff (v1)

| | |
|---|---|
| Document | The owner reviewed and approved the generated sheet and asked for a last check; the architect's findings are in `.git-exclude/reviewed/japanese-final-check-v1.md`. This handoff (1) applies the **25 text corrections** of the table (one wording, one standard; one misleading sentence; typography), (2) fixes the **responses that bypass the catalog and the one error shape** (found by a live probe), (3) records the **sign-off** and turns Japanese on, with the integration that implies. |
| Status | **Assigned** (2026-10-31). |
| Basis | `japanese-final-check-v1.md`; `crates/planeter-web/src/catalog/{ja,mod}.rs`, `crates/planeter-client/src/catalog/ja.rs`; `docs/glossary.md` §5; `tools/ja-sheet`; `docs/messages-and-languages.md` (the path to the sign-off); RFC 004 D-11 (one error shape), RFC 012; `JA_REVIEWED`/`JA_SIGNOFF`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — The 25 text corrections (one commit; exactly these keys)

Rules behind them (add to the glossary, section 5, with the date): the **verb** *change* is 変わる/変える, **変更** is the noun (and the pair 変更を依頼する / 変更依頼あり); *sign in* is ログイン; *refused* is 拒否; *closed* is 閉じている; *confirm* is 確定; a space between a numeral or placeholder and its counter; full-width brackets. `explain.queued` must **not** promise a signature (no まだ: the English has no "yet" by decision D-50). Document also (no text change): 取り込み = take into the repository (the server's accept and an import), 保持 = held / kept as a received copy, 保管 = kept until a date, 保存 = saved, 確認 = a person checks or confirms, 検証 = verify, チェック = a stage, 点検 = `doctor`; dates are ISO 8601 UTC in every language.

| # | catalog | key | before | after |
|---|---|---|---|---|
| A | web | `propose.held` | あなたの作業はこのサーバーでレビュー待ちとして保持されており、どのブランチも変更されていません。 | あなたの作業はこのサーバーでレビュー待ちとして保持されており、どのブランチも変わっていません。 |
| A | web | `propose.already_recorded` | このアーティファクトはすでに記録済みのため何も追加されておらず、どのブランチも変更されていません。 | このアーティファクトはすでに記録済みのため何も追加されておらず、どのブランチも変わっていません。 |
| A | web | `recovery.blocker.output_unreadable` | リポジトリのツールの出力を読み取れなかったため、何も変更していません。 | リポジトリのツールの出力を読み取れなかったため、何も変えていません。 |
| A | web | `recovery.outcome.stopped` | 復旧は停止し、これ以上は何も変更していません。フラグはまだ立っています。 | 復旧は停止し、これ以上は何も変えていません。フラグはまだ立っています。 |
| A | web | `hide.refused.heading` | コメントは変更されませんでした | コメントは変わりませんでした |
| A | web | `decision.form.consequence` | これはリビジョン {revision} だけに記録され、このリポジトリを読める全員に見えます。提案が開いている間は変更できます。あとのリビジョンには引き継がれず、何もマージしません。 | これはリビジョン {revision} だけに記録され、このリポジトリを読める全員に見えます。この判断は、提案が開いている間は変えられます。あとのリビジョンには引き継がれず、何もマージしません。 |
| B | web | `error.forbidden.hint` | メンテナーに書き込み権限を依頼するか、サインインしている全員に提案を開放するよう依頼してください。 | メンテナーに書き込み権限を依頼するか、ログインしている全員に提案を開放するよう依頼してください。 |
| B | web | `error.forbidden_comment.message` | このリポジトリは読めますが、コメントするにはサインインが必要です。 | このリポジトリは読めますが、コメントするにはログインが必要です。 |
| B | web | `error.forbidden_comment.hint` | サインインするか、アクセストークンを付けてリクエストを送ってください。 | ログインするか、アクセストークンを付けてリクエストを送ってください。 |
| B | web | `new.visibility.public` | 誰でも（サインインの有無を問わない） | 誰でも（ログインの有無を問わない） |
| B | web | `new.visibility.internal` | サインインしている人なら誰でも | ログインしている人なら誰でも |
| B | web | `new.proposals.any` | 読めるサインイン済みの人なら誰でも | 読めるログイン済みの人なら誰でも |
| B | web | `new.signin` | リポジトリを作成するには、サインインしてください。 | リポジトリを作成するには、ログインしてください。 |
| C | cli | `server.refused` | サーバーが断りました：{message} {hint} | サーバーが拒否しました：{message} {hint} |
| C | cli | `server.refused_plain` | サーバーがリクエストを断りました（{status}）が、理由は示されませんでした。もう一度お試しください。続く場合は、このサーバーを運用している人に伝えてください。 | サーバーがリクエストを拒否しました（{status}）が、理由は示されませんでした。もう一度お試しください。続く場合は、このサーバーを運用している人に伝えてください。 |
| C | web | `error.invalid_request.name_hint` | この応答の field と reason に、断られた値と理由が示されています。API ドキュメントの項目の通りに直して、もう一度送ってください。 | この応答の field と reason に、拒否された値と理由が示されています。API ドキュメントの項目の通りに直して、もう一度送ってください。 |
| D | web | `refs.closed` | クローズ済み | 閉じている |
| E | web | `error.stage_limit.message` | 確認待ちのアーカイブがすでに {limit} 件あります。 | 確定待ちのアーカイブがすでに {limit} 件あります。 |
| F | web | `explain.queued` | このパッチは、このサーバーで保持されています。メンテナーがプロジェクトの履歴にまだ署名していないため、検証済みの記録には含まれていません。 | このパッチは、このサーバーで保持されています。メンテナーがプロジェクトの履歴に署名していないため、検証済みの記録には含まれていません。 |
| G | web | `proposal.patch.unknown_explain` | このサーバーは、このパッチが署名されて履歴に入ったかどうかを判断できません(履歴が、サーバーが読む範囲より長いためです)。 | このサーバーは、このパッチが署名されて履歴に入ったかどうかを判断できません（履歴が、サーバーが読む範囲より長いためです）。 |
| H | web | `explain.patch` | パッチとは、記録された編集1件のことです。 | パッチとは、記録された編集 1 件のことです。 |
| H | web | `quota.open_proposals.message` | このリポジトリで開いている提案がすでに{used}件あり、上限は{limit}件です。 | このリポジトリで開いている提案がすでに {used} 件あり、上限は {limit} 件です。 |
| H | web | `quota.revisions.message` | この提案にはすでに{used}件のリビジョンがあり、上限は{limit}件です。 | この提案にはすでに {used} 件のリビジョンがあり、上限は {limit} 件です。 |
| H | web | `error.rate_comments.message` | この1時間に、この提案へ何度もコメントしています。 | この 1 時間に、この提案へ何度もコメントしています。 |
| H | web | `error.rate_decisions.message` | この1時間に、この提案へ何度も判断を記録しています。 | この 1 時間に、この提案へ何度も判断を記録しています。 |

Tests: extend `ja_consistency.rs` (planted cases both ways): no `サインイン` outside `シングルサインオン`; no `クローズ`; no `断` + (りました|られた); no `変更されて`/`変更して(い)` as a verb for branches (the planted forbidden list: `変更されていません`, `何も変更していません`); `確認待ち` not present; no half-width parenthesis next to Japanese; no digit/placeholder directly before 件/時間; `explain.queued` does not contain まだ. The glossary tests and `retired_words` follow.

## T2 — Responses that bypass the catalog (code, with tests)

Found by a live probe with `Accept-Language: ja`: (a) **extractor rejections** are the framework's own English plain text (`400 text/plain "Invalid URL: Invalid UTF-8 in \`owner\`"` on an API path); (b) a **body over the framework's limit** answers **400 with an empty body** on `POST /login`, `/new`, `/api/v1/repos` and a proposal-close; (c) **unknown non-API addresses and wrong methods on page routes** answer **404/405 with an empty body** (`/no-such-page`, `DELETE /`, `GET /new/import`, `/a/b/?x=%ff`). Everything else probed was Japanese and in the one shape.
One fix at the edge, not per handler: a response layer (or the router's fallback and the extractors' rejection handling) that turns any framework-generated rejection into the project's own: for paths under `/api/` the **one JSON shape** `{code, message, hint}` with the catalog's text in the caller's language (`invalid_request` 400 for undecodable path/query/body, `payload_too_large` 413 for an oversize body — **the status is 413, not 400** —, `not_found` 404, `method_not_allowed` 405 with `Allow`), for other paths the existing message page (not-found page, "method not allowed" page, "too large" page) with the same statuses, `Content-Language` and no change to what a stranger can learn about a repository's existence (these are route-level, not repository-level). Tests: each of the probes above as a test in English and Japanese (and the pseudo-language), an API client gets JSON on every error, a browser never gets an empty body for a 4xx, and the IS-14 and OpenAPI drift guards still hold (add the new codes' use to the OpenAPI responses where an operation can now return them: 413 and 405).

## T3 — The sign-off and turning Japanese on

On the owner's approval (2026-10-31, sheet fingerprint `6ba30960fe807d98`, followed by the corrections of T1): regenerate the sheet and `--check` it; set `JA_REVIEWED` to `true` in both programs and `JA_SIGNOFF` to a record naming **the owner**, the date, the commit of the T1 corrections and the regenerated sheet's fingerprint (the architect adds the colleague's name if the owner wishes); the test that refuses one without the other must stay green. Then the integration this implies, with tests: Japanese is offered without `PLANETER_PREVIEW_LANGUAGES` (the variable is kept for future draft languages and documented); the client picks Japanese from `LC_ALL`/`LC_MESSAGES`/`LANG` when they say `ja` (`--lang` still wins; English otherwise); the language switcher shows both choices and no lone-English case remains (phase 2: remove the code path that hides/shows it, or confirm none is left); `docs/accessibility.md` gets its Japanese wording finalized (same facts as the English; no claim that assistive-technology testing was done); the startup log line no longer says "preview"; `docs/messages-and-languages.md`, `docs/RELEASING.md` (operator note: the language is on by default), CHANGELOG `[Unreleased]` ("Japanese is now available; English remains the default for callers who do not ask for Japanese"), README.

## Non-change scope

No English text change; no trust label change beyond the table; no new message except those T2 needs (English + Japanese, added to the sheet through its generator and context table); no new dependency; no `unsafe`; no version or release change (the architect decides the version).

## Acceptance criteria

T1's 25 corrections exactly (a test lists the keys), T1's planted-case tests; T2's probes as tests; T3 as written; the regenerated sheet passes `--check`; every gate green (fmt, floor, pin, clippy, both test runs named, deny and audit 0, ENF-2 on both binaries, doc), CI green on all three systems.

## Prohibited shortcuts

No change of a Japanese text outside the table; no sign-off record without the regenerated sheet's fingerprint; no per-handler patching of the error cases (one edge fix); no weakening of a test.

## Required evidence and review-request format

Commits (hash → what, by task); the before/after probe transcript (status, content type, first bytes) for every probe; the regenerated sheet's header; the sign-off record; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
