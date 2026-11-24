# RFC 010 / IS-70 — "planeter listening on …" must be printed after the socket is bound — Implementation Handoff (v1)

| | |
|---|---|
| Document | Found by the dev team while fixing `server_kill` (`load-sensitive-two-review-request.md`): `crates/planeter/src/main.rs` prints `planeter listening on http://{addr} (db: …)` (line ≈1045) **before** `planeter_web::serve_until` binds the socket (`lib.rs:134`). Two honest consequences: (1) a supervisor, a script or a test that starts the connection the moment it reads the line can be **refused**; 23 of 100 loaded runs of `server_kill` failed this way; (2) if the bind **fails** (the port is taken), the operator reads "listening" and then "server error": the first line is false. The architect's decision (SCHEDULE D-119, by the owner's philosophy: clean, honest, robust): the line says what is true when it is printed. Test retries stay (they are cheap and some tests start in other ways), but they stop being the thing the product depends on. |
| Status | Assigned 2026-11-24 (D-119). Small; before the 0.9.0 candidate. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Split binding from serving.** `planeter_web::serve_until` keeps its signature for its callers (tests) and gains a sibling that takes an **already bound** `tokio::net::TcpListener` (e.g. `serve_on(state, listener, shutdown)`); `serve_until` binds and calls it. In `main.rs`: bind first; on failure print `planeter: cannot listen on {addr}: {e}` (a plain sentence, no "listening"), release the lease and the process lock as the existing error path does, exit 1; on success print the **same line as today, character for character** (`planeter listening on http://{addr} (db: {db_path})`; many tests and the operator documents read it), then serve. If `PLANETER_ADDR` has port 0 the line must still show the **bound** address as before (use `listener.local_addr()` for what is printed only if today's line already does; otherwise keep `{addr}`: say which and keep behaviour).
2. **Where the bind sits:** after all start-up work that can fail (the same place the line is today), so the order of every other start-up line is unchanged. The key-records thread and the recovery thread still start as today (they run after the server listens; moving the bind earlier than them is fine, say what you did).
3. **Tests:** (a) a real-server test: start the built server on a port that is **already bound** by the test: the output has **no** `listening` line, has the new sentence, exit code 1, and the process lock file is **removed**; (b) a test that connects **at once** (no retry, no sleep) the moment the line is read, 200 times in a loop against fresh servers if that is cheap enough (otherwise 20), all accepted; (c) the existing tests pass untouched. Mutation: put the print back before the bind and see (b) fail under `tools/repeat-under-load.sh` (say how many of N) and (a) fail.
4. Say whether any document names the line's timing (`docs/`, README, RELEASING); correct only a wrong statement.

## Non-change scope
No other product change, no catalog text, no new dependency, no `unsafe`, no change to start-up order beyond the bind, no tag, release or upload. Do not touch `docs/SCHEDULE.md`, `docs/STATUS.md`, the ledger, the threat model.

## Acceptance criteria
The line is printed only after a successful bind; a failed bind prints one honest sentence and exits 1 with the lock removed; (b) passes; all gates green with tests with prikk three times in a row; CI green on all eight jobs.

## Required evidence
Commits (hash → what); the failing-bind transcript; (b)'s result and the mutation's; the gates in a clean worktree with commands, exit codes, advisory-database HEAD and count; deviations.
