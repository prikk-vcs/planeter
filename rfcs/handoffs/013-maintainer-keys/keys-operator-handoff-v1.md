# RFC 013 — The operator's act: `planeter operator key add` (D-2) — Implementation Handoff (v1)

| | |
|---|---|
| Document | Second of three handoffs for RFC 013. It builds **the one way out when no maintainer key can sign**, and the first command of the `planeter operator` family: the person who runs the server, with the server **stopped**, adds one maintainer key to one repository. It is never silent (the key event it records is shown on the repository page by handoff 3), it adds and never removes (D-1), and it cannot be run beside a live server. It stands on `keys-foundation-handoff-v1` (the bound `room_for`, the `repo_key_events` table, `KeyEventStore`, `repo_keys`, `detail_for_record`) and on `process_lock` (RFC 004 D-7). |
| Status | Assigned 2026-11-22 (SCHEDULE D-113), after the `url` advisory move is approved. |
| Basis | `rfcs/accepted/013-maintainer-keys.md` (D-1, D-2, D-4, D-5, D-6, "Threats"); ledger PK-90, PK-91 (prikk adds a key with no signature from an adopted key; **removing one fails `verify`**), PK-92; `crates/planeter/src/args.rs` (`parse`, `USAGE`), `crates/planeter/src/main.rs` (the configuration the server reads, `ProcessLock::acquire(format!("{db_path}.lock"))`, `exit_releasing_lock`), `crates/planeter-core/src/process_lock.rs`, `crates/planeter-core/src/lease.rs` (the W lease and its durable holder record), the delivery engine's failpoints and its kill tests (`crates/planeter/tests/delivery_real_server.rs`, `failpoint.rs`), the existing recovery for a flagged repository, `crates/planeter-client` for how a command line picks its language. |
| Audience | Dev team. Return a review-request package. |

## What the command is

```
planeter operator key add OWNER/NAME --key-id ID --public-key HEX --reason TEXT [--lang en|ja]
planeter operator --help
```

`--help` says, in both languages: **"For the person who runs this server. It needs the server's data and cannot be run from the web. Stop the server first."** `planeter --help` gains one line naming `operator`. An unknown subcommand prints the usage on stderr and exits 2. The command reads **only** the settings it needs (`PLANETER_DB`, `PLANETER_REPOS_ROOT`, the prikk and sandbox settings the server uses to run prikk) and **ignores every other variable** (a secret such as `PLANETER_OIDC_CLIENT_SECRET` is never read, never printed: a test sets a canary in it and holds that it appears nowhere). The server's configuration reading is **shared, not copied**: factor what both need out of `main.rs` without changing what the server does (its tests pass untouched).

## Change scope

**T1 — The command line.** `args::parse` learns the first word `operator`; a small `operator` module owns the rest. `--reason` is required, 1 to 200 characters, and a reason with a control character or longer than 200 is **refused with a sentence** (the operator typed it; it is never silently cut); the public key is 64 lowercase hexadecimal characters (uppercase refused with the sentence that says what is expected) and the id is the one prikk derives from it (use the same function creation uses; a mismatch is refused with both values shown). The messages are **translated**: add them to the server's catalog under `operator.*` in English and Japanese, chosen by `--lang`, else by `LC_ALL`/`LC_MESSAGES`/`LANG` as `planeter-client` does, else English; they are written in the project's plain words (RFC 012; one sentence each, with what to do), and the Japanese follows the glossary (引き渡す for deliver is not needed here; do not use 保持 or 預かる in new text). **These Japanese messages ship flagged until the owner has read them** (the usual rule): do not touch `ja.signed` or `SignOff`.

**T2 — The sequence, under the process lock.** In this order, and each refusal changes nothing and exits 1:
1. validate the arguments (T1; exit 2 on a malformed command line);
2. read the configuration; **take the process lock** (`{db}.lock`, as the server does). If a server holds it: the sentence **"The server is running. Stop it, run this command again, then start it."** and exit 3. A stale lock is handled exactly as the server handles it; an unclear lock is the server's own message;
3. open the database; resolve `OWNER/NAME` (unknown: one sentence, exit 1);
4. **refuse a repository that needs recovery, has an unfinished delivery, or has a lease holder record left by a dead process:** "Start the server and let recovery finish first." (exit 1);
5. read the trust list with prikk. If the key is **already trusted**: when the table holds **no event** for it, **record the event now** (this completes an act that was interrupted after the add) and say so; when it holds one, refuse "This key is already trusted." (exit 1);
6. `room_for(trusted, 1)`: at the bound, the sentence of D-6 ("This repository already trusts the most keys planeter supports: 64. Keys cannot be removed yet, so no more can be added.") and exit 1;
7. take the repository's **W lease** (operation name `operator key add`; its holder record is the intent: a kill leaves a record the next server's start finds and recovers like any write that did not finish);
8. `prikk trust maintainer add`; then **`verify`** (exit 0 and no publication-trust issue). If `verify` fails after the add (it should not): the add **cannot be undone** (D-1), so flag the repository `needs_recovery` with the reason "an operator's key add did not verify", say so loudly, exit 4;
9. bump the repository's revision counter the way every other write does;
10. record the event (`source = operator`, `detail = reason` through `detail_for_record`, `delivery_id` empty, `backfilled = 0`). If recording fails after the add succeeded, say plainly **"The key is trusted but the record of it was not written. Run the same command again to record it."** (step 5 completes it) and exit 4;
11. release the lease and the lock; print the result sentence: **"{fingerprint, eight groups} is now trusted by OWNER/NAME. Added by this server's operator on {date}: {reason}"** and "Start the server." — the same words the page will show; exit 0.
It **never** touches history, a proposal, a delivery or a token; it **never** calls `trust maintainer remove` (the D-1 scan covers this crate too).

**T3 — Exit codes and output.** 0 done (or the interrupted act completed); 1 refused (the state or the key); 2 the command line; 3 the server is running or the lock cannot be taken; 4 prikk or the record failed after a change; 5 local files or the database. Messages to stderr; the result sentence to stdout; no colour; nothing printed that names a secret, a token or another repository. `--help` lists the codes.

**T4 — Tests (real prikk, real sandbox, the built binary).**
- the add: a repository created with key A, key B made with prikk, the command run; `prikk trust maintainer list` shows both, `verify` exit 0, the event row (`operator`, the reason, not backfilled), the revision counter moved by one;
- **the whole story:** the server started on that data, the maintainer's delivery signed by **B** accepted and the same delivery **before** the add refused `unadopted_key` (as today);
- refusals, each leaving the tree and the database as they were (hash the `.prikk` tree and dump the tables): a malformed key, a mismatched id, a key already trusted with an event, the bound (a repository at 64), a flagged repository, an unfinished delivery, an orphan lease record, an unknown repository, an empty or 201-character reason, a reason with a control character;
- **a real server process holding the lock**: exit 3 with the sentence, nothing changed; after the server is stopped, the same command works; a stale lock file of a dead process is replaced;
- **kills**: with the debug failpoint mechanism the delivery engine uses (a debug build only; the ENF-2 check keeps the name out of the release binary — extend its list if it names points), SIGKILL the command at each point — after the lease, after the add, after `verify`, after the bump, after the event — then start the real server: the repository is flagged and recovered where the lease record says so, ends `verify` clean, and **re-running the command** ends in the same final state (the key trusted, one event); a kill after the add and before the event shows the key as "Source not recorded" in `repo_keys` until the re-run;
- **the secret canary** (above); **the language**: the same refusal in English and in Japanese by `--lang` and by `LC_ALL`;
- **mutations** to show failing once and then reverted: the add before the process lock is taken; the lease skipped; `verify` skipped; the event not recorded; the bound not checked.

## Non-change scope
No page, no JSON field, no creation form, no `docs/keys.md` (handoff 3), nothing about D-3 (a keys file in a repository; gated: do not start it), no `remove` of any kind, no new route, no socket, no new dependency, no `unsafe`. No change to the server's behaviour (its tests pass untouched), the delivery rule or the stage. No change to `ja.signed`, the `SignOff` or any baseline. No tag, no release, no upload; `cargo publish` only `--dry-run`. The architect writes the threat-model and ledger changes.

## Acceptance criteria
The command does exactly the sequence above and nothing else; every refusal leaves everything as it was; a kill at any point recovers to a consistent state and the command is re-runnable; the server is untouched by the existing tests; the new messages exist in both languages and are counted as "newer than the sign-off" by `planeter-i18n-sheet --status` (give the number); all gates green; CI green on all eight jobs.

## Required evidence and review-request format
Commits (hash → what, by task); `git diff --stat`; the transcript of a real run (the add, the refusal for a running server, the refusal for the bound, the re-run after a kill) in English and in Japanese; the table of kill points and what each left and what the re-run did; the mutations (failing output, then clean); the new catalog keys and `--status`; the grep proving the D-1 scan covers the new module; the gates in a clean worktree with commands, exit codes, tool versions, the advisory-database HEAD and its advisory count (fmt, floor, pin, clippy, tests with prikk 0.58.0 **three times in a row** and without, `cargo deny`, `cargo audit`, ENF-2 with the release binary, doc, msrv); deviations and open questions (say plainly what you could not show).
