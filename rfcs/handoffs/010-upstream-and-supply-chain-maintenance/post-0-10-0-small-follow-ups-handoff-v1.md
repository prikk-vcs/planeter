# RFC 010 — After 0.10.0: three small follow-ups — Implementation Handoff (v1)

| | |
|---|---|
| Document | Three small things 0.10.0 left, in this order: **(1) IS-76**, the two throw-away private keys that are string literals in the sources of earlier packages; **(2) the `sync seal` memory re-measured on prikk 0.59.0** (PK-88, PK-94's open action) and `docs/deliveries.md`'s sentences about it; **(3) IS-71, the cost of the test suite, first step only: the measurement and the coverage proof**, no deletion. None changes a product behaviour. |
| Status | Assigned 2026-11-28 (SCHEDULE D-153). Not urgent; do (1) and (2) together, (3) as its own package. |
| Basis | `docs/STATUS.md` IS-76 and IS-71; `tools/gates.sh` (the `known` list of the `package-keys` step); `docs/deliveries.md` ("`sync seal`, measured"); ledger PK-88, PK-94, PK-95; `.git-exclude/upstream/prikk/receive/2026-11-28-prikk-0-59-0-released-…md` (their figures: theirs, not ours); the measuring script `sealmem.sh` named in PK-88. |
| Audience | Dev team. Return a review-request package (one for (1)+(2), one for (3)). |

## (1) IS-76 — take the two literals out of the sources
`crates/planeter-auth/src/oidc.rs` (a unit test's `TEST_KEY_PEM`) and `crates/planeter-web/tests/oidc.rs` hold a throw-away private key as a string. They shipped in 0.9.0 and 0.10.0 packages and cannot be taken back. **Do:** make the test key without a literal in a source file that is packaged: generate it in the test with what the crate already depends on (no new dependency), or move it to a fixture under `tests/fixtures/` (which the package excludes), keeping the test's meaning and its assertions exactly. Then **empty the `known` list** of the `package-keys` step in `tools/gates.sh` and its tests (`known=""`), and show: the step passes on the new tree with no "known" line; the mutation (put a literal back) fails it and names the file. `cargo package --list` for the two crates shows nothing new. If a crate has no way to generate the key without a new dependency, **stop and say so**: do not add one.

## (2) `sync seal` on prikk 0.59.0, measured by us
prikk reports (their release letter) 183 → 31 MiB at 10,000 claims in the growing shape and 63 → 23 MiB in the fixed shape. **Those are their figures.** Measure ours: `sealmem.sh` as in PK-88 (N sealed blocks each its own claim, the maintainer's copy accepts the artifact, then `sync seal heads/main --claims FILE`, `VmData` sampled), the **growing** and the **fixed** shape, at 1,000, 3,000, 5,000, 7,500 and 10,000 claims, **0.58.0 and 0.59.0 on the same machine, alternated**, one at a time, under the cap (`tools/run-capped.sh`), nothing else running. Report both tables. **Then** `docs/deliveries.md`'s three measured sentences: they are true of 0.58.0 and say so; add what we measured of 0.59.0 **as a sentence of its own with its numbers and its date**, and say that the forge never runs `sync seal` (it is the maintainer's command). **Do not copy prikk's figures into our page as ours.** If our numbers disagree materially with theirs, say so in the report and do not edit the page until the architect has read them. The architect updates PK-88/PK-94 from the report.

## (3) IS-71 — what the test suite costs, and what it covers (a measurement, not a cut)
The review tier takes ~1,300 s and ~600 CPU-s; the suite spawns about 36,000 sandbox wrappers, and the kill-point tests dominate. **Do not delete or weaken any test.** Produce, in one report: (a) the cost by test file (wall and CPU, from one run, sorted), the top 15; (b) for each of those 15, **what it covers that no other test covers** (a short line, with the control it holds from the threat model, T-n/C-n) and whether it could run **fewer iterations or points with the same coverage**; for a kill-point test, the list of points and the argument that each is distinct; (c) a **proposal**, as a table, of what could be run less often (for example, the full set in the release tier, a reduced set in the review tier) with the coverage each reduction gives up, said plainly. **No change is made by this package.** The architect decides from it; "faster" is not a reason by itself (RFC 010 Amendment 1: no unmeasured tuning, release evidence stays strict).

## Non-change scope
No product behaviour, no catalog text, no pin, no dependency, no `unsafe`, no workflow change; no edit of `STATUS.md`, `SCHEDULE.md`, the ledgers, the threat model or a security review.

## Acceptance criteria
(1) the `known` list is empty and the gate passes; the mutation fails it; the two tests keep their assertions. (2) two tables, both versions, same machine; `docs/deliveries.md` says what is true of each version and nothing it did not measure. (3) the report has (a), (b), (c) and no test changed.

## Required evidence
Commits (hash → what); for (1) the diff of the two test files and of `tools/gates.sh`, the step's output before and after, the mutation; for (2) the script and its raw output, the two tables, the page's diff; for (3) the per-file table and the coverage lines; deviations and open questions. Final reply: the review-request path(s) only.
