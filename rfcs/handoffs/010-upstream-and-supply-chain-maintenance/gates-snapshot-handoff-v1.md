# RFC 010 — `tools/gates.sh` must not be changed under its own feet — Fix Handoff (v1)

| | |
|---|---|
| Document | **The architect's review run on `70c6485` died twenty minutes in with `tools/gates.sh: line 349: syntax error near unexpected token ')'`.** The dev team and the architect work in **one working copy**; the dev team committed a new `tools/gates.sh` (`b61cc07`) while the architect's run was executing the old one, and a POSIX shell reads a script **a piece at a time as it runs**: when the file changed on disk the shell resumed in the middle of the new text. Nothing was wrong with either version; the run was lost, and a lost review run is twenty minutes of the machine's heat for nothing (RFC 010 Amendment 1). The same can happen to `tools/run-capped.sh` and the `tools/tests/` scripts. A script that runs for twenty minutes must not depend on a file that other people edit meanwhile. |
| Status | Assigned 2026-11-27 (SCHEDULE D-145). Small. |
| Basis | `tools/gates.sh` (its start: where it re-runs itself inside `tools/run-capped.sh`), `tools/run-capped.sh`, `tools/tests/gates.sh`, `tools/pin-check/tests/shell_tools.rs`, RFC 010 Amendment 1 principle 6. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Run from a snapshot.** At its very start, before it reads anything else of the repository, `tools/gates.sh` copies **itself and every script it will run or source** (`tools/run-capped.sh`, any helper it calls from `tools/`) into its scratch directory (a private directory it makes with `mktemp -d`, mode 0700, removed by its cleanup by that one path) and **re-executes from the copy** (`exec sh "$SCRATCH/gates.sh" --snapshot-of "$ORIG" "$@"`, with a guard so the copy does not copy again). The steps it runs inside the worktree (`cargo`, the tools of the commit under review) are the **commit's own files in the worktree it made**, which nobody edits; only the *runner* is snapshotted. It prints `gates: running from a snapshot of tools/gates.sh (<sha256 of the file>)` so that a log says which version ran. A copy that cannot be made is a stop with a sentence, not a continuation from the original.
2. **Tests** in `tools/tests/gates.sh` (no build): the script, started, is **edited on disk while a slow fake step is running** (append a syntax-breaking line, and separately replace it with a different valid script), and the run **still finishes with its own table and exit status** and cleans up; the snapshot is removed at the end, also on a failing step, `INT` and `TERM`; the printed hash equals the file's at the start. **Mutation:** run without the snapshot (the old behaviour) and show the edit-while-running test fails.
3. **Say** in one sentence in `docs/RELEASING.md`'s tiers paragraph that a run is immune to edits of the script meanwhile, and keep `GATES_EXISTING_CHECKOUT` as it is (kept: decided in review).

## Non-change scope
No product code, no step added or removed, no change to a tier's contents, no new dependency, no tag, release or upload; do not touch the ledger, SCHEDULE, STATUS or the threat model.

## Acceptance criteria
A run survives its script being rewritten on disk; the snapshot is private, removed on every exit and named in the log; the new tests and the mutation; a real review-tier run of the new script (CI green, all steps passed) and CI green on all eight jobs.

## Required evidence
Commits (hash → what, each compiling and passing the tests it touches); the edit-while-running tests' output; the mutation; one real review-tier table; deviations and open questions.
