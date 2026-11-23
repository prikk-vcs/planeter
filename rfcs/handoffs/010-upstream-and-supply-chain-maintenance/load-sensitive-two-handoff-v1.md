# RFC 010 / IS-69 — the last two load-sensitive tests: the export deadline and the server-kill start — Implementation Handoff (v1)

| | |
|---|---|
| Document | Two tests the IS-69 work found or met and left, rightly, for a decision (SCHEDULE D-118). **(1)** `transport_fetch.rs::under_the_real_sandbox_the_export_writes_into_the_spool_mount_and_nowhere_else` failed 1 in 100 loaded runs: its **one-second export deadline** can end before the sandboxed call has started, and the server answers `unavailable` where the test expects `export_failed`. The stand-in already sleeps 120 s, so the deadline's value is not what the test holds (it holds "the write lands in the spool mount and nowhere else" and that an export that does not finish is `export_failed`). **(2)** `server_kill.rs:157` ("the server never accepted connections") failed once in the dev team's interrupted gate run while the host was loaded. |
| Status | Assigned 2026-11-24 (D-118). Small; before the 0.9.0 candidate. |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **(1)** Raise that test's export deadline to **10 s** (the stand-in outlives any deadline), keep the assertions, say what the total test time became. Mutation: make the export write outside the spool mount (the property under test) and see it fail.
2. **(2)** Find why the start was not accepted: read the failing log in `glibc-step-logs/gates-interrupted/` and the test's wait (`server_kill.rs` near line 157). If it waits a fixed time for the port, make it wait until the server says it is listening **and then retry the connection** (as `key_records_real_server.rs` already does since `c19921a`), with a generous guard (60 s). Show the failure under load before (`tools/repeat-under-load.sh 100`) if it reproduces; if it does not, say so and make the change anyway on the evidence of the log.
3. **Run `tools/repeat-under-load.sh 100` on both files after the change: 0 failures**, and report any other failure the runs show (do not fix others silently: list them).

## Non-change scope
No product code, no catalog text, no dependency, no tag, release or upload. Do not touch `docs/SCHEDULE.md`, `docs/STATUS.md`, the ledger, the threat model.

## Acceptance criteria
Both tests pass 100 loaded runs; their properties unchanged (mutation for (1)); all gates green, tests with prikk three times in a row; CI green on all eight jobs.

## Required evidence
Commits (hash → what); before/after loaded-run results; the mutation; the gates in a clean worktree with commands, exit codes, advisory-database HEAD and count; deviations.
