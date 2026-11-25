# RFC 014 — Two last things in the account log work: the scope words in Japanese, and a race in the terminal test — Fix Handoff (v2)

| | |
|---|---|
| Document | `accounts-event-details-fix-review-request.md` (`a0bf90d`) is approved except for two things found by the architect. **(1)** In Japanese the token scope words `write` and `full` are left in English (`operator.token.list.write` = "write", `…full` = "full") while `read-only` is 読み取り専用: the log sentence reads `トークン「ci」（write、1 件のリポジトリ、…）`, the same fault the fix itself removed (an English word inside a Japanese sentence), and inconsistent within one parenthesis. **(2)** `with_the_real_stty_what_is_typed_is_not_shown` (`operator_account.rs:1708`) failed once in the architect's whole-workspace run (and passed in 10 loaded repeats of the file and in the dev team's runs): the transcript shows the typed password **before** the "Password:" prompt, i.e. the test wrote its input to the pseudo-terminal **before the command had turned echo off**, and the terminal echoed it. It is a race in the test (a person types after seeing the prompt), not a defect of the command; but it is exactly the kind of test that must not fail a release run. |
| Status | Assigned 2026-11-25 (SCHEDULE D-132). Small. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Scope words in Japanese: words that describe, not values that are typed.** The values a person *types* (`--scope read-only|write|full`, in usage lines and in the sentence that says which values are allowed) stay literal in every language. The words that *describe* a token's scope in a sentence or a table become Japanese: **read-only → 読み取り専用 (unchanged), write → 書き込み, full → 完全なアクセス** (the last is the client's own wording for "full access" in `deliver.error.forbidden`: 完全なアクセスを許可するトークン). Change `operator.token.list.write` and `.full` in `ja.rs` and check **every place** a scope is shown in words (the token list table, the create result, `account_events::scope_word`, the transcripts) for the same fault; list each place and its decision. English is unchanged. The reading file is regenerated (the messages stay counted as new; give `--status`).
2. **The pseudo-terminal test:** the test must **read until the prompt has appeared** ("Password:", and again "Password again:") **before it writes each entry**, with a generous guard (30 s, failing with what it did receive), so the order is: prompt printed → echo already off → input sent. Do the same in every other test in that file that types into the pty (the fake-`stty` guard tests), and say which. Then run `tools/repeat-under-load.sh 50` on `operator_account` and, to reproduce the failure's real condition, **three whole-workspace test runs with prikk on PATH** (the failure showed only under the load of the whole workspace): 0 failures. If you can make the old test fail on purpose (type before the prompt: a deliberately early write), show it failing once so the new test's ordering is shown to be what holds it.

## Non-change scope
Nothing else; no product code outside the two Japanese words (the command, the event rendering and the stored forms are unchanged), no schema, no other message, no sign-off record, baseline, `ja.signed` or `SignOff`, no new dependency or `unsafe`, no tag, release or upload.

## Acceptance criteria
Both Japanese words changed and every place that describes a scope in words checked; no pty test types before its prompt; 50 loaded repeats and three whole-workspace runs clean; all gates green; CI green on all eight jobs.

## Required evidence
Commits (hash → what, each compiling and passing its tests); the list of places and decisions; the deliberately-early-write failure if shown; the repeat results; `--status` and the regenerated reading file; the gates in a clean worktree with commands, exit codes and the advisory-database HEAD; deviations and open questions.
