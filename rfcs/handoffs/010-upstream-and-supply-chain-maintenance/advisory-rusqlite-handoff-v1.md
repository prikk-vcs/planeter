# RFC 010 — Advisory RUSTSEC-2026-0459 (`libsqlite3-sys` via `rusqlite`) — Implementation Handoff (v1)

| | |
|---|---|
| Document | `cargo deny check` and `cargo audit` fail on `main` since the advisory database moved (2026-10-30 advisory, seen 2026-11-13): `libsqlite3-sys` 0.30.1, bundled SQLite before 3.54.0 can return uninitialized page-cache memory when a process opens a database together with a sibling `-wal` file whose page size differs (a crafted pair of local files). It enters through `rusqlite` 0.32 (`planeter-auth`, `planeter-store`, feature `bundled`). CI's `supply-chain` job is red on the last two commits. Patched: `libsqlite3-sys >= 0.39.0` (`rusqlite` 0.41). |
| Status | Assigned 2026-11-13. |
| Decision | The architect, by the owner's standing rule (dependency policy, rule 3: **no standing ignore**; the owner's philosophy: stable and secure, long term): **fix by upgrading, do not ignore.** Reachability is low (planeter opens only its own database files in its own data directory; an attacker would already need write access there) and that is **not** the reason to wait. |
| Basis | `docs/dependency-policy.md` rules 1 and 3; `crates/planeter-auth/Cargo.toml`, `crates/planeter-store/Cargo.toml`; `crates/planeter-auth/src/sqlite.rs`, `crates/planeter-store/src/*.rs`; RUSTSEC-2026-0459 text; `rusqlite` 0.41 and 0.42 release notes. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope
1. **Measure first (dependency policy rule 1), in a scratch worktree, before any commit that adopts it:** for `rusqlite` 0.41 and for the newest release (0.42.0 on crates.io today), report "N → M crates (+K); deny: …; audit: …; new licenses: …", the SQLite version each bundles (must be **>= 3.54.1** or the version that fixes the advisory; show how you know), the declared Rust version of `rusqlite` and `libsqlite3-sys`, and the number of source lines each needs to change. Use a fresh advisory-db clone.
2. **Adopt the newest of the two that needs the fewest source changes and clears the advisory.** If the choice is not 0.42, say why in one paragraph. If it needs a higher `rust-version` than the workspace's 1.88.0, raise `rust-version` to exactly what the chosen release declares, nothing more, and update everything that names it (`floor-check`, the `msrv` CI job, docs, CHANGELOG). The compiler pin (1.99.1) does not change.
3. Fix the code for any API change, in `planeter-auth` and `planeter-store` only. No behavior change: the existing tests are the proof, and **no test is edited to make it pass** except for a signature that no longer exists (list each such edit).
4. **Confirm the reachability statement with a grep, as evidence, not as an excuse:** list every `Connection::open*` call and say which file it opens and who can write that file. Report anything that opens a path an outsider can influence (an upload, an archive, a user-named path).
5. Record the decision in `docs/dependency-policy.md` ("Supply-chain triage record": the advisory, the version, the bundled SQLite version, the lock count before and after, no ignore) and in `CHANGELOG.md` under unreleased (a security-relevant dependency update, one line).
6. Re-run the whole gate set on the exact commit with a fresh advisory database, including both test runs, ENF-2 and `cargo publish --workspace --exclude planeter-runner --dry-run --locked`.

## Non-change scope
No `deny.toml` or `.cargo/audit.toml` ignore. No other dependency update unless the new `rusqlite` forces it (then list it). No change to the schema, the file layout or the WAL settings. No tag, no release, no version change.

## Acceptance criteria
`cargo deny check` and `cargo audit --db <fresh clone> --no-fetch` exit **0** with no ignore; the lock shows `libsqlite3-sys >= 0.39.0`; the bundled SQLite version is stated with evidence; all existing database tests pass unedited (apart from listed signature edits); the reachability list is present; every gate green except those listed in "Known red" below; CI's `supply-chain` job green.

**Known red, not part of this handoff:** five tests that drive a real prikk fail when `prikk` 0.56.0 is on PATH (the host now has it). Run the with-prikk test pass with prikk **0.55.0** on PATH (install it privately, e.g. `cargo install prikk --version 0.55.0 --root <dir>`, and put `<dir>/bin` first) and say which prikk version each pass used. The without-prikk pass is unaffected.

## Required evidence and review-request format
Commits (hash → what, the measurement commit separate from the adoption commit if there is one); the measurement table for 0.41 and 0.42; the Rust version change if any; the reachability list; every gate with command, exit code, the prikk version of the with-prikk pass and the advisory-database HEAD; deviations and open questions.
