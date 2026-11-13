# RFC 004 / 005 — prikk 0.56.0 is released: measure it against the driver, change nothing yet — Implementation Handoff (v1)

| | |
|---|---|
| Document | prikk 0.56.0 is on crates.io and installed on the host (since 2026-11-09). The prikk project promised (letters of 2026-10-31, recorded in ledger PK-77) a delivery classification with default refusal of rollback / diverged / does-not-connect, `extract --claim` over chains of pending claims, `sync pending` membership by claim, unreached patch and blob counts, and `archive verify` key `status` as one field with four fixed strings. With 0.56.0 on PATH, five tests that drive a real prikk fail (the two `propose_kill` tests, `extract::real_prikk_0_55_names_a_claim_whose_parent_is_pending`, `verify_unreached::real_prikk_after_received_remove_…`, `transport_propose::history_received_by_import_…`). With prikk 0.55.0 on PATH, and without prikk, all 1,207 tests pass (architect's run of `03fca11`). **This handoff is measurement only.** The floor stays 0.54.0, no driver code changes, no test is edited. The architect decides the re-baseline from your report. |
| Status | Assigned 2026-11-13. |
| Basis | ledger PK-70…PK-77 (`docs/src/planeter-prikk-dependency-ledger.md`); the prikk letters in the architect's private notes are not available to you: use the prikk 0.56.0 source and its own documentation in `~/.cargo/registry/src/*/prikk-*-0.56.0/` and `prikk --help`; `crates/planeter-prikk/`; the five failing tests. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Setup
Put prikk **0.55.0** in a private directory (`cargo install prikk --version 0.55.0 --locked --root <dir under .git-exclude/tmp>`) and keep it for comparison. The host's 0.56.0 is the other side. Every measurement below says which one it ran on. Scratch is under `.git-exclude/tmp/` or `/tmp` and is deleted when done.

## Tasks
1. **For each of the five failing tests:** the assertion that fails, the value on 0.55.0 and on 0.56.0, and one sentence: is it an intended 0.56.0 change (name which promise it is), a timing change (faster accept, PK-75), or something unexplained. Unexplained ones are reported first.
2. **Delivery classification.** Run `bundle preview`, `bundle import` and `archive import` on 0.56.0 for each of the six classes (`no-local-history`, `already-at-tip`, `fast-forward`, `rollback`, `diverged`, `does-not-connect`). Report the exact JSON and exit code of each (refusal-v1 where it refuses), and say which of the planeter driver's calls reach each class (accept path, repository creation from an archive, `received` import, recovery). Say whether any current planeter flow can now be refused that was accepted before.
3. **`sync pending --format json`.** Report the shape on 0.56.0 (the `claims`, `target_block_id`, membership), the text form's header line and order, and how it differs from 0.55.0 around `sync accept` and around `received remove`. Does the before/after comparison in PK-71 still work unchanged?
4. **`extract --claim` over a chain** of two and three pending claims on a receiver that never seals: the ids, `chain[]`, `chain_signature` (`unverifiable` on a receiver without the key), and what a claim whose parent is pending now returns (the failing `extract` test). Two claims with the same `target_block_id`: what `extract` says (`claim-chain-shared-block`).
5. **`doctor` / `verify`:** unreached `patch` and `blob` objects after `received remove`: the code (expected to be unchanged, `PRIKK-DOCTOR-UNREACHED-OBJECT`), the wording, the JSON `unreached_objects` entries.
6. **`archive verify`:** the key `status` strings for the four situations (adopted; carried, not adopted; carried, conflicts with an adopted key; carried, not checked against a repository). Compare the exact strings with the four fixed ones.
7. **Cost:** repeat the `sync accept` timing of PK-75 on 0.56.0 for the 300- and the 1,000-patch cases (paired and interleaved with 0.55.0, same host), and say whether the cost model constants (7.0×, 2 KiB per object, 10 MiB) still bound it. The two `propose_kill` tests need a kill window: say what window the 0.56.0 timing leaves.
8. **Floor reading:** run the existing `floor-check` and the `MIN_PRIKK_VERSION` logic against 0.56.0 and say what happens (it should only warn or pass, never refuse).

## Non-change scope
No code, no test, no floor, no version, no doc outside the report. If a measurement shows prikk behaving differently from the ledger, say so; do not "fix" the driver.

## Acceptance criteria
Every task answered with commands and outputs (full outputs in the logs directory, the review request quotes the decisive lines); both prikk versions named for every number; unexplained differences listed first; no repository file changed except the report package.

## Required evidence and review-request format
A table for task 1; a table for task 2 (class → command → exit code → refusal code or JSON key → which planeter flow); the other tasks as short sections; logs; deviations and open questions.
