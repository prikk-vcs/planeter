# Glossary — Japanese (`ja`)

The Japanese column of [`../glossary.md`](../glossary.md): the terms the interface uses, the messages that carry a term or a trust label, and the rules the wording follows. One file per language (`docs/glossary/<tag>.md`; `docs/messages-and-languages.md`, "Adding a language"); the English concept table stays in `glossary.md`.

## Status

**Status.** **Signed off** by its reviewers, the owner and a colleague, on 2026-10-31 (the status and its record are data on the language, `crates/planeter-i18n/src/languages/ja.rs`, with the baselines `crates/*/locales/ja.signed`); shipped like English and offered without any operator setting. The reviewed sheet is generated from the catalogs
(`cargo run -p planeter-i18n-sheet -- --lang ja`, `docs/messages-and-languages.md`). **The English meaning of every trust label is fixed; the Japanese
keeps it exactly.** A later correction is an edit of this section and of `ja.rs` together, then a regenerated sheet.
The style is the polite form (です・ます); headings and badges are noun phrases; Japanese has no plural, so a counted message
says the same for every count; numbers group with commas (`1,200`).

## Terms

*Shown* says whether the interface uses the term. **Hard term** = a term the reviewers decide (decided at the sign-off, 2026-10-31; the column keeps the classification the generated review sheet is built from).

| English term | Japanese term used | Shown | Why, and alternatives | Status |
|---|---|---|---|---|
| repository | リポジトリ | yes | The usual loan word; on every page (`home.*`, `layout.nav_repository`, the not-found sentences). | Hard term — reviewers decide |
| branch | ブランチ | yes | The usual loan word (`nav.refs`, `refs.*`, `history.empty`). Alternative: 枝 is not used in software. | Hard term — reviewers decide |
| tag | タグ | yes | The usual loan word. | Proposed |
| patch | パッチ | yes | prikk's own word is *patch*; the interface explains it (`explain.patch`: 記録された編集 1 件). Alternative: 編集 would drop prikk's word. | Hard term — reviewers decide |
| change | 変更 | yes | The one word for a recorded unit of history (prikk: *block*), as in English. Counted with 件 (`history.patch_count`). | Hard term — reviewers decide |
| history | 履歴 | yes | The usual word for a repository's history. | Proposed |
| proposal | 提案 | yes | What a person puts forward for review (RFC 005 part 5, D-38): the page headings, the state names and the API messages (`proposals.*`, `proposal.*`, `error.*_proposal.*`). The draft's word for *propose* already; *change* (変更) stays for the recorded unit of history. Counted with 件. | Hard term — reviewers decide |
| revision (of a proposal) | リビジョン | yes | One version of a proposal, a re-sent artifact (`proposal.revisions.*`). Alternative: 版. | Hard term — reviewers decide |
| landed (state) | 反映済み | yes | The state of a proposal whose patches are all in the project's history (`proposal.state.landed`, `error.proposal_not_open`). Alternatives: 取り込み済み (but 取り込み is the server's *accept*, below). | Hard term — reviewers decide |
| held for review | レビュー待ちで保持中 | yes | The one sentence of an open proposal (`proposal.what.open`): the work is on this server and not yet in the history. | Hard term — reviewers decide |
| built on | 土台とした履歴 | yes | The history a revision says it was made from (`proposal.revisions.col.built_on`). | Proposed |
| close (a proposal) | 閉じる | yes | The action and the state 閉じている (`proposal.close.*`, `proposal.state.closed`). | Proposed |
| comment | コメント | yes | What a person writes about a proposal (`comment.*`, `comments.*`); a reply is 返信. Counted with 件. | Hard term — reviewers decide |
| review / decision | レビュー / 判断 | yes | The proposal's *review* is レビュー (`review.*`); what one reviewer says is a *decision*, 判断 (`decision.*`, `review.decision.*`). | Hard term — reviewers decide |
| approve | 承認する | yes | A reviewer's decision about one revision (`decision.form.approve`, `review.line.approved`). Used for nothing else about a proposal (`tests/retired_words.rs`). Alternative: 了承する. | Hard term — reviewers decide |
| request changes | 変更を依頼する | yes | The other decision (`decision.form.request_changes`). Here 変更 is the ordinary word for *a change to the work*, not the glossary's *change* (a recorded unit of history). Alternative: 修正を依頼する. | Hard term — reviewers decide |
| hide (a comment) | 非表示にする | yes | `hide.*`, `comment.hidden`; the opposite is 再表示する (`unhide.*`). | Proposed |
| archive | アーカイブ | yes | The usual loan word for one file that holds a whole repository (`new.import.*`, `stage.*`). | Hard term — reviewers decide |
| maintainer key | メンテナーの鍵 | yes | `stage.keys.*`; メンテナー as in the existing sentences. | Proposed |
| fingerprint | フィンガープリント | yes | The usual loan word for what is compared to know a key (`stage.key.*`). Alternative: 指紋 (unusual in software). | Hard term — reviewers decide |
| trust (a key) | 鍵を信頼する | yes | Plain: 信頼する / 信頼しない (`stage.key.skip`, `created.trusted`). prikk's *adopt* is not shown. | Hard term — reviewers decide |
| received copy | 受信したコピー | yes | A branch kept but not published (`created.branch.kept`, `stage.ref.kept`); the English *received copy* made plain. | Proposed |
| rollback | ロールバック | yes | The usual loan word; explained in place (`explain.rollback`: 以前の変更を取り消す変更). | Hard term — reviewers decide |
| verify / verification | 検証 | yes | The page and the navigation say 検証 (also "prikk の検証に通ります"; 検査 is no longer used, D-50); the checks are チェック (`verify.col.stage`, `stage.error.verify`), so a *check* and the *verification* stay two words, as in English. Alternative: 確認 (used for *could not be checked*, 現在は確認できません). | Hard term — reviewers decide |
| Verified (badge) | 検証済み | yes | A trust label. English: prikk ran all its checks just now and found no problems. 検証済み says *has been verified*; the explanation (`explain.verified`) says *just now*. The reviewers confirm the label does not read as a lasting guarantee. | Hard term — trust label, reviewers decide |
| Signed into the history (badge) | 署名して履歴に記録済み | yes | A trust label. English: a maintainer signed this patch into the project's history. The draft says both parts, *signed* and *recorded in the history*. Alternatives: 履歴に署名済み, 署名済み. It must not read as *verified* (it is not). | Hard term — trust label, reviewers decide |
| Held here, not signed in (badge) | ここで保持中・未署名 | yes | A trust label. English: held on this server, not signed by a maintainer, so not part of the verified record. Reworded 2026-10-29 (D-50; was 承認済み・署名待ち, which said *approved*); approved at the sign-off, 2026-10-31. 未署名 keeps *not signed*. | Hard term — trust label, reviewers decide |
| Verification failed (badge) | 検証に失敗 | yes | A trust label. prikk checked and found problems (not *was not checked*). | Hard term — trust label, reviewers decide |
| Could not be checked now (badge) | 現在は確認できません | yes | A trust label. The check could not run (not *failed*). 確認 rather than 検証 so that it does not claim a verification happened. | Hard term — trust label, reviewers decide |
| seal / sealed / unsealed | (not used) | no | The interface never says *seal* (the plain-words work retired it; the English reader sees *signed into the history*). **prikk documents no Japanese, so there is no usage to follow.** If a document ever needs the word, the draft would propose 封印 / 封印済み / 未封印 or the loan word シール; both are on the forbidden list for the messages (`tests/retired_words.rs`). | Hard term — reviewers decide (not shown to the reader) |
| single sign-on | シングルサインオン | yes | The usual katakana form; the abbreviation SSO is retired (`login.sso`, `page.sso_*`). | Hard term — reviewers decide |
| sign in / sign out | ログイン / ログアウト | yes | The usual words. Alternative: サインイン / サインアウト. | Proposed |
| username | ユーザー名 | yes | `login.user`. | Proposed |
| maintainer | メンテナー | yes | The usual loan word (`explain.signed`, `explain.queued`). | Proposed |
| check (a verification stage) | チェック | yes | `verify.col.stage`; the statuses are 完了 / 問題あり / 未実行 / 途中で停止. | Proposed |
| the person who runs this server | このサーバーを運用している人 | yes | `unavailable.body`, `too_large.body`, `error.too_large.raw_hint` and the other operator-facing messages. Reversed 2026-10-29 (D-50, the architect's consistency change; the reviewers may overrule): the draft said このサーバーの管理者, which is also what a repository's administrator would be called. | Hard term — reviewers decide |
| operator (in API hints for developers) | 運用者 | yes | The same person, in the short form the API hints use, as the English hints say *operator*. | Hard term — reviewers decide |
| an administrator of a repository | リポジトリの管理者 | yes | Always with the qualifier, never 管理者 alone (`error.forbidden_hide.message`, `review.hidden.*`, `new.empty.consequence`). Not the person who runs the server. Added 2026-10-29 (D-50). | Hard term — reviewers decide |
| the repository check that prikk's `doctor` does | 点検 | yes | `recovery.blocker.doctor_problems`, `recovery.note.unreached_objects`. Only for `doctor`; it is a different operation from the verification, as the English keeps *repository check* and *verification* apart. 検査 is not used. Added 2026-10-29 (D-50). | Hard term — reviewers decide |
| merge | マージ | yes | The usual loan word in software (`proposal.*`, `review.*`: "何もマージしません"). Added 2026-10-29 (D-50). | Proposed |

## Messages that carry a term or a trust label

The Japanese cell is the catalog text, character for character (`tests/glossary_lang.rs` compares it, and the English cell, with
the two catalogs). A reviewer's correction changes this cell and `ja.rs` together.

| Message | English | Japanese |
|---|---|---|
| `nav.code` | "Files" | "ファイル" |
| `nav.history` | "History" | "履歴" |
| `nav.refs` | "Branches and tags" | "ブランチとタグ" |
| `nav.verify` | "Verification" | "検証" |
| `layout.sign_in` | "Sign in" | "ログイン" |
| `layout.sign_out` | "Sign out" | "ログアウト" |
| `layout.language` | "Language" | "言語" |
| `login.sso` | "Sign in with single sign-on" | "シングルサインオンでログイン" |
| `login.user` | "Username" | "ユーザー名" |
| `login.password` | "Password" | "パスワード" |
| `assurance.verified` | "Verified" | "検証済み" |
| `assurance.sealed` | "Signed into the history" | "署名して履歴に記録済み" |
| `assurance.queued` | "Held here, not signed in" | "ここで保持中・未署名" |
| `assurance.unverified` | "Verification failed" | "検証に失敗" |
| `assurance.verify_unavailable` | "Could not be checked now" | "現在は確認できません" |
| `explain.summary` | "What does this mean?" | "これはどういう意味ですか？" |
| `explain.verified` | "prikk ran all of its checks on this repository just now, and they found no problems." | "prikk がこのリポジトリに対してすべてのチェックをたった今実行し、問題は見つかりませんでした。" |
| `explain.signed` | "A maintainer signed this patch into the project's history. The Verification page shows whether the whole repository checks out." | "メンテナーがこのパッチをプロジェクトの履歴に署名して記録しました。リポジトリ全体に問題がないかどうかは、「検証」ページで確認できます。" |
| `explain.queued` | "This patch is held on this server. A maintainer has not signed it into the project's history, so it is not part of the verified record." | "このパッチは、このサーバーで保持されています。メンテナーがプロジェクトの履歴に署名していないため、検証済みの記録には含まれていません。" |
| `explain.failed` | "prikk checked this repository and found problems. The Verification page lists what was found." | "prikk がこのリポジトリをチェックしたところ、問題が見つかりました。見つかった内容は「検証」ページに一覧があります。" |
| `explain.failed_here` | "prikk checked this repository and found problems. They are listed below." | "prikk がこのリポジトリをチェックしたところ、問題が見つかりました。内容は下に一覧で示します。" |
| `explain.unavailable` | "Try again in a little while." | "しばらくしてから、もう一度お試しください。" |
| `explain.patch` | "A patch is one recorded edit." | "パッチとは、記録された編集 1 件のことです。" |
| `explain.rollback` | "A rollback is a change that undoes an earlier change." | "ロールバックとは、以前の変更を取り消す変更のことです。" |
| `history.rollback` | "rollback" | "ロールバック" |
| `history.patch_count` | "{count} patch" / "{count} patches" | "パッチ {count} 件" |
| `history.empty` | "There are no signed changes on this branch yet." | "このブランチには、署名済みの変更がまだありません。" |
| `refs.branches` | "Branches" | "ブランチ" |
| `refs.tags` | "Tags" | "タグ" |
| `refs.current` | "current" | "現在" |
| `refs.closed` | "closed" | "閉じている" |
| `verify.heading` | "Verification" | "検証" |
| `verify.findings` | "What was found" | "見つかった内容" |
| `verify.status.evaluated` | "completed" | "完了" |
| `verify.status.failed` | "problem found" | "問題あり" |
| `verify.status.not_evaluated` | "not run (blocked by {check})" | "未実行（{check} が原因）" |
| `verify.status.not_run` | "not run" | "未実行" |
| `verify.status.halted` | "stopped early" | "途中で停止" |
| `verify.status.halted_after` | "stopped early (after {check})" | "途中で停止（{check} の後）" |
| `verify.col.stage` | "Check" | "チェック" |
| `home.open` | "Open repository" | "リポジトリを開く" |
| `home.address_label` | "Repository address" | "リポジトリのアドレス" |
| `unavailable.body` | "This repository could not be read right now. Try again in a little while. If it keeps happening, tell the person who runs this server." | "今はこのリポジトリを読み込めません。しばらくしてから、もう一度お試しください。それでも解決しない場合は、このサーバーを運用している人にお知らせください。" |
| `not_found.body` | "There is no repository at this address, or you do not have access to it. Check the address, or sign in if the repository is private." | "このアドレスにはリポジトリがないか、アクセスする権限がありません。アドレスを確認してください。非公開のリポジトリの場合は、ログインしてください。" |

## One wording, one standard (the final check, 2026-10-31)

Rules the Japanese catalogs follow, each enforced by `crates/planeter-web/tests/i18n/ja.rules` (planted cases both ways, `tests/language_rules.rs`) where it can be: the **verb** *change*
is 変わる / 変える (「どのブランチも変わっていません」); **変更** is the noun *change* only (and the fixed pair 変更を依頼する / 変更依頼あり, which the
reviewers approved). *Sign in* is ログイン everywhere (シングルサインオン keeps its own name). *Refused* is 拒否 (not 断る / 断られた). *Closed* is
閉じている (a proposal and a branch alike; 締め切られました for "comments closed on {date}" is a deadline, a different sense). *Confirm* is 確定.
A space stands between a numeral or placeholder and its counter (`{used} 件`, `1 時間`). Brackets are full-width （ ）. `explain.queued` does **not**
promise a signature (no まだ): the English has no "yet" by decision D-50, because a keyless forge cannot promise that anyone will sign.

Words with one family and more than one audience or sense, **deliberately left as they are**: 取り込み = take into the repository (the server's
*accept* and an *import*); 保持 = held (pending) or kept as a received copy; 保管 = kept until a date; 保存 = saved; 確認 = a person checks or confirms
(and "could not be checked now", on purpose); 検証 = verify; チェック = one stage of a verification; 点検 = what `doctor` does. Dates and times are
ISO 8601 in UTC in every language (`2026-10-31 14:05 UTC`); sizes use KiB / MiB / GiB; the identifiers `ref`, `field`, `reason`, `Retry-After` and
`Content-Length` stay in Latin letters in developer texts.
