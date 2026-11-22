# RFC 013 — `planeter operator key add`: one defect in step 5 (a key trusted since creation must not become "added by the operator") — Fix Handoff (v1)

| | |
|---|---|
| Document | Your package `keys-operator-review-request.md` (`c1c0b6c`) is approved except for one defect the architect found by reading step 5 of `crates/planeter/src/operator.rs` against RFC 013 D-4 ("the page never claims what was not recorded"). The rest is accepted as built, including step 7b, which was a real hole in the architect's own handoff (the start-up pass would have labelled a new key "creation" after a kill between the add and the record): thank you for finding it. |
| Status | Assigned 2026-11-23 (SCHEDULE D-114). |
| Audience | Dev team. Return a short review-request package. |

## The defect
Step 5 treats "trusted but `NotRecorded`" as **an act interrupted after the add** and completes it by writing an `operator` event. But `repo_keys` reports **every** trusted key as `NotRecorded` in a repository that has **no key record at all** (a repository the start-up pass has not yet filled: for example the first command run on a 0.9.0 binary before the server has ever started, or a repository whose pass was skipped). In that repository **a key trusted since the repository's creation** is `NotRecorded`, `completing` is true, step 7b skips it (`filter(|k| k.public_key != cmd.public_key)`), and the command **writes an `operator` event for it**: the page would then say "Added by this server's operator on {date}: {reason}" about a key that was there from the start. That is false provenance, the one thing D-4 forbids.

An interrupted act of this command can never leave a repository with no record at all (step 7b writes the older keys' records in one transaction **before** the add; a kill after it leaves rows), so "no row at all" and "completing" cannot both be true.

## Change scope
1. **Test first, and show it failing on `c1c0b6c`:** a repository with **no key record at all** (made, then its `repo_key_events` rows deleted, or made before the table existed) trusting key A; `planeter operator key add OWNER/NAME --key-id <A> --public-key <A> --reason x` must exit **1** ("This key is already trusted."), change **nothing** (no event, counter unmoved, no lease record) and write **no** `operator` event. On `c1c0b6c` it exits 0 and writes one: show that output, then the fix.
2. **The fix:** step 5 completes an interrupted act **only when the repository already has at least one key record** (`events.has_any(&repo)`); in a repository with none, an already trusted key is refused as already trusted. Nothing else in the sequence changes. Keep the order of 7b.
3. **Pin the other side:** a repository **with** rows, a key trusted and unrecorded (the state a kill after the add leaves) → `completing` still completes (your kill-table test covers it; name it in the report). Add one test for the third state, so the behaviour is written down and not accidental: rows exist for A, key B was added out-of-band with prikk (never recorded), `key add B` **completes and records it as `operator`** with the operator's reason — the command cannot tell this from an interrupted act, and the operator who runs it with a reason **asserts** it. (Handoff 3's `docs/keys.md` says so in one sentence: "If you record a key that was added earlier, say so in the reason: the reason is shown to everyone.")
4. **A mutation** to show failing once and then reverted: `completing` ignoring `has_any`.

## Non-change scope
Nothing else: no message, no catalog text (the 27 Japanese messages are unchanged and still flagged), no page, no sequence step other than the condition above, no other test. No tag, release or upload.

## Acceptance criteria
The new test fails on `c1c0b6c` and passes after; the two other states are pinned; the mutation fails once; all gates green; CI green on all eight jobs.

## Required evidence
Commits (hash → what); the failing output before and the passing output after; the mutation; the gates in a clean worktree (fmt, floor, pin, clippy, tests with prikk 0.58.0 **three times in a row** and without, `cargo deny`, `cargo audit` on a fresh database, ENF-2, doc, msrv) with commands, exit codes and the advisory-database HEAD; deviations and open questions.
