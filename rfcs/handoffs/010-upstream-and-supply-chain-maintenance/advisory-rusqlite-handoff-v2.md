# RFC 010 — Advisory RUSTSEC-2026-0459: adopt the `rusqlite` upgrade (continues v1) — Implementation Handoff (v2)

| | |
|---|---|
| Document | `advisory-rusqlite-handoff-v1` was delivered on the local branch `advisory-rusqlite` (`ba47d6a`, reviewed Approved, D-70) and waited for one decision, which is now made. The owner prefers the upgrade to any ignore and authorized the release plan. **Decision (architect, on the owner's philosophy): (1) the licence Zlib is allowed for the crate `foldhash` only**, as a `[[licenses.exceptions]]` entry like `webpki-roots` (permissive, OSI-approved, compatible with Apache-2.0; it enters through `rusqlite → hashlink → hashbrown`, not avoidable from our manifests); **(2) the version is `rusqlite` 0.41.1, not 0.42.0.** Both bundle SQLite 3.54.1, declare Rust 1.89.0, cost the same lock delta and need 0 source lines. 0.41.1 is 17 days old with about 1.9 million downloads, and the project that reported the advisory (`localcache`) adopted it; 0.42.0 is three days old with about 98 thousand. For a long-term, stable and secure choice the proven one wins; the next dependency round can take 0.42. |
| Status | Assigned 2026-11-14. |
| Basis | the branch `advisory-rusqlite` (`ba47d6a`) and its review request; `deny.toml` (the `webpki-roots` exception is the model); `docs/dependency-policy.md` rules 1–3; current `main` (floor 0.57.0, `ammonia` 4.2.5). |
| Audience | Dev team. Return a review-request package. |

## Tasks
1. **Rebase onto current `main`** (the branch is behind: ammonia, the floor move). Resolve `Cargo.lock` with `cargo update`, not by hand. Do not rewrite `main`; push the result as a fast-forward to `main` only after the architect's review (as before: work on the branch, report the head).
2. **Use `rusqlite` 0.41.1**: `rusqlite = { version = "0.41", features = ["bundled"] }` in both manifests; the lock must show `rusqlite` 0.41.1 and `libsqlite3-sys` 0.39.1; state the bundled `SQLITE_VERSION` from the source file as before (expected 3.54.1) and re-measure "N → M crates (+K), deny, audit, new licenses" against the rebased `main`. The declared minimum compiler stays at 1.89.0 with the documentation of `ba47d6a`.
3. **`deny.toml`**: add the exception for `foldhash` with `allow = ["Zlib"]` and a comment in the style of the `webpki-roots` one (what it is, how it enters: `rusqlite → hashlink → hashbrown`, the licence's nature, that it covers that one crate). Add a line to `docs/dependency-policy.md` rule 2's list of licences and its triage record. `cargo deny check` must exit **0** with no advisory ignore.
4. **Upgrade rehearsal (new, the one risk worth testing).** The new SQLite must open the database files that the released 0.5.0 server wrote, and a rollback must not corrupt them:
   - Install the released `planeter` 0.5.0 (`cargo install planeter --version 0.5.0 --locked --root <scratch>`), start it on a fresh data directory with a database, and create real content: a user (or sign-in), a repository, a proposal with a comment and a decision, a recovery flag if it can be set; stop it cleanly. Also make a second database by killing the 0.5.0 server with SIGKILL during activity, so a `-wal` file is left.
   - Start the **new** build on copies of both; verify it opens them, serves the same content (the proposal, the comment, the decision, the sign-in), and runs the migrations it needs without error; report `PRAGMA integrity_check` and `PRAGMA journal_mode` of each database before and after.
   - Start the 0.5.0 server again on a copy that the new build has written to; say whether it still opens and reads (the rollback path), and what it says if it does not.
   Report every step with commands and outputs; any failure is a STOP (report, change nothing).
5. **Gates on the exact commit**, with the fresh advisory database: fmt, floor, pin, clippy, tests with prikk 0.57.0 (the host's) and without prikk, ENF-2, doc, `cargo audit` **and `cargo deny check` both exit 0**, `cargo +1.89.0 check --workspace --locked`, and the publish dry-run (which fails until the version moves; say so and show the failure is only that).
6. `CHANGELOG.md` `[Unreleased]`: the line of `ba47d6a` updated to 0.41.1; nothing else.

## Non-change scope
No other dependency update unless the rebase forces it (list it). No `ignore` anywhere. No source change unless the compiler asks (it did not on 0.42; if 0.41.1 needs any, list each line). No version change, no tag.

## Acceptance criteria
The rebased commit; `rusqlite` 0.41.1 and `libsqlite3-sys` 0.39.1 in the lock; bundled SQLite 3.54.1 shown; `deny` and `audit` exit 0; the rehearsal (task 4) complete and clean; every other gate green; CI's `supply-chain` job green.

## Required evidence and review-request format
Commits (hash → what); measurement line; the rehearsal as a table (database → integrity check → journal mode → content served → result) for the three directions (0.5.0 → new, killed 0.5.0 with `-wal` → new, new → 0.5.0); every gate with command, exit code, prikk version and advisory-database HEAD; deviations and open questions.
