# RFC 010 follow-on — Advisories RUSTSEC-2026-0386 (tokio) and RUSTSEC-2026-0387 (crypto-bigint) — Implementation Handoff (v1)

| | |
|---|---|
| Document | Two new advisories (2026-10-13, 2026-10-23) turn `cargo deny check` and `cargo audit` red on the lock of `main`; CI's `supply-chain` job has failed since `f370ab5`. Following RFC 010 D-4's ladder, in order: **(1)** a lock-only update for `tokio` (fix exists in the same minor); **(3)** for `crypto-bigint 0.5.5` no lock or manifest upgrade fixes it (verified by the architect 2026-10-27: `jsonwebtoken` 11.1.0 and 10.4.0 still depend on `p256 ^0.13`/`p384 ^0.13`, i.e. `elliptic-curve 0.13` → `crypto-bigint 0.5`; `elliptic-curve 0.14.3` uses `crypto-bigint ^0.7.8`, but no `jsonwebtoken` uses it), so the candidate is to **remove the crate path**: `jsonwebtoken`'s `aws_lc_rs` backend instead of `rust_crypto` takes `p256`, `p384`, `elliptic-curve`, `crypto-bigint` **and** `rsa` (the standing ignore RUSTSEC-2023-0071) out of the tree. **Measure it; do not adopt it in this handoff.** **(4)** an ignore is the last rung and is the owner's ruling (policy rule 3), not part of this handoff. |
| Status | **Assigned** (2026-10-27). Independent of `landing-rule-correction-handoff-v2.md`. |
| Basis | [`../../accepted/010-upstream-and-supply-chain-maintenance.md`](../../accepted/010-upstream-and-supply-chain-maintenance.md) D-4; `docs/dependency-policy.md` (rules 1–3, the `rsa` ignore); `.git-exclude/review-requests/advisory-crypto-bigint-2026-0387-report.md` (the dev team's stop report, correct and complete); the advisory texts (database HEAD `3279c99`, 2026-10-27); the same-minor precedent `advisory-ammonia-4-2-4-handoff-v1.md`; the client-stack decision (D-37: `rustls` with the `aws-lc-rs` provider) — one crypto provider for server and client is a reason to prefer this backend if the numbers allow. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — tokio (RUSTSEC-2026-0386): a lock-only update

`cargo update -p tokio --precise 1.53.3` (or the newest `1.53.x` that is not yanked). Report the **complete lock diff** (expected: `tokio` alone, perhaps `tokio-macros`; if anything else moves, stop and report). `git grep AsyncFd -- crates` stays empty (state it). Gates: the full set including both supply-chain gates on a fresh advisory database — `cargo deny check` will still fail on RUSTSEC-2026-0387 until T2's outcome is decided; show that **the tokio advisory is gone** from both outputs. Its own commit; lock only.

## T2 — Measure `jsonwebtoken` with the `aws_lc_rs` backend (scratch only; no repository change)

In a scratch copy of the workspace (`~/scratch/…`, never committing its manifests or lock), switch `planeter-auth`'s `jsonwebtoken` from `rust_crypto` to `aws_lc_rs` (the current `10.x` and `11.1.0` both, if the API differs say how) and report, per rule 1 ("N → M crates (+K); deny: …; audit: …; new licences: …"):
(a) the lock delta by crate name (added, removed — confirm `p256`, `p384`, `ecdsa`, `elliptic-curve`, `crypto-bigint`, `rsa`, `primeorder` and their exclusive dependencies are gone); (b) `cargo deny check` and `cargo audit` on the fresh database for the scratch lock: **which advisories remain, and whether the `rsa` ignore (RUSTSEC-2023-0071) would be unnecessary**; new licences, if any; (c) whether every OIDC algorithm planeter accepts still works with the backend (RS256/384/512, ES256/384, EdDSA, and that HS* is still refused where it is today): run the **whole existing `oidc.rs` test set** and the sign-in tests; show the algorithm-by-algorithm result including a token signed with each and a tampered token for each; (d) native build requirements (a C compiler; `cmake`/`nasm`/`perl` needs, as in the client assessment: test with `CC=/bin/false`), MSRV 1.88.0 build, **release binary size and clean-build time** against `main`, **the container build** (`docker build` of the project's `Dockerfile`, or the nearest you can run, and what it needs added); (e) whether `aws-lc-sys` changes the ENF-2 symbol check or the `unsafe` posture of the workspace (planeter crates keep `forbid(unsafe_code)`; count `unsafe` lines in the added crates as in the client assessment, labelled as a grep); (f) the **regression pin** RFC 010 D-4 asks for `jsonwebtoken` (untrusted input): which test pins the advisory shape and does it still pass.
Close with a table "stay (ignore, owner-ruled) vs switch (backend)" of facts only, and your recommendation clearly separated from the facts.

## Non-change scope

No `deny.toml`, `audit.toml` or manifest change; no ignore; no change to `planeter-auth`'s code in the repository (the scratch copy may change what it must to compile; show the diff); no release.

## Acceptance criteria

T1 committed with its lock diff and the tokio advisory gone from both tools; T2's report complete for (a)–(f); every claim with its command and output in the logs; nothing but T1's lock change in the repository.

## Prohibited shortcuts

No ignore entry; no editing the gate script or the advisory database to get a green; no counting from documentation instead of from a build; no adoption of the backend without the owner's and the architect's decision.

## Required evidence and review-request format

Commits (T1); the lock diff; the T2 table and logs; the fresh advisory database HEAD for every run; deviations and open questions.
