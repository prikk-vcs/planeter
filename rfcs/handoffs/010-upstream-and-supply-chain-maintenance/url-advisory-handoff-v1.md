# RFC 010 — RUSTSEC-2026-0592: `url` 2.5.8 → 2.5.9, lock-only — Implementation Handoff (v1)

| | |
|---|---|
| Document | A new advisory (RUSTSEC-2026-0592, dated 2026-11-20, in the database since 2026-11-22): **`url` 2.5.8, stack overflow in `Url::origin()` on nested `blob:` URLs; patched in ≥ 2.5.9** (published 2026-11-20 23:03 UTC, not yanked). `cargo deny check` and `cargo audit` fail on the lock of `main` against a fresh database (the architect's run on `0a6d3c8`, database HEAD `248aea09…`, 1,560 advisories; the release 0.8.0 was checked against 1,555 advisories that did not yet contain it). The project's rule is no ignore: **move the lock**, as for `ammonia` on 2026-10-18 (`docs/dependency-policy.md`, the table of lock changes). |
| Status | Assigned 2026-11-22 (SCHEDULE D-112). **Do this before anything else on `main`: the CI `supply-chain` job turns red at the next push.** |
| Basis | RFC 010 (supply-chain maintenance: gates run against a fresh database); `docs/dependency-policy.md` (the precedent row of 2026-10-18); `cargo tree -i url --locked`: `url` is reached through `ammonia` 4.2.5 (the sanitizer in `planeter-web`) and `planeter-core` (`egress.rs`: `url::Url::parse`). |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Read the advisory** (RustSec and the `url` repository's release notes for 2.5.9) and say in one paragraph **whether planeter can reach it**: `grep` shows planeter's own code never calls `Url::origin()`; check whether `ammonia` 4.2.5 or any other crate in the tree calls it on a value that can come from a person (a link in sanitized text, an issuer or webhook URL, the client's server address), and state what you found with the file and line. The answer does not change the fix (no ignore either way); it goes in the record.
2. **Move the lock only:** `cargo update -p url --precise 2.5.9` (no manifest change; if the resolver wants to move anything else, stop and report). The diff of `Cargo.lock` is the `url` version and checksum lines and nothing else (show it). The crate count stays 214.
3. **A row in the table of lock changes** of `docs/dependency-policy.md`, in the style of the 2026-10-18 `ammonia` row (date, what, count before and after, the one-line diff, deny and audit results on a freshly fetched database).
4. **Gates** in a clean worktree: fmt, floor, pin, clippy, tests with prikk 0.58.0 and without, `cargo deny check` and `cargo audit` (0.23.0) against a **freshly fetched** database, **both 0 with no ignore**, ENF-2, doc, msrv 1.89.0; CI green on all eight jobs. (`cargo publish` dry run is not meaningful at the released 0.8.0.)

## Non-change scope
No manifest change, no other crate moved, no source change, no pin, no version change, no release, no upload. Do not touch `docs/SCHEDULE.md`, `docs/STATUS.md`, the ledger, the threat model.

## Acceptance criteria
`Cargo.lock` differs from `main` in the two `url` lines only; deny and audit are 0 on a fresh database; all gates and CI green; the reachability paragraph names what was checked.

## Required evidence and review-request format
Commits (hash → what); the `Cargo.lock` diff; the reachability paragraph; the policy row; every gate with command, exit code, tool versions and the advisory-database HEAD and its advisory count; deviations and open questions.
