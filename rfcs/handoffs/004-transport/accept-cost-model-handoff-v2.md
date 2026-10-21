# RFC 004 — Transport — Implementation Handoff (v2, part 4b): what an accept really costs — memory by artifact shape, time, and the defaults that follow

| | |
|---|---|
| Document | Two measurements contradict the accept's cost model: **(1)** a single large blob needs far more memory than the 4.2 × rule (a 105 MiB random file in a 110 MiB artifact exceeded the 1.5 × ceiling of 677 MiB — more than 6.4 × — and was killed), and patch-shaped artifacts exceed 4.2 × from 7,500 patches on (the margin is down to 11 % at the 20,000 bound); **(2)** accept time per patch does not depend on the pending set but on the size of the artifact being accepted (≈ 15 ms at 1,000 patches, ≈ 37 ms from 5,000 on), so 20,000 patches take 672 s, over the 600 s deadline. This handoff **measures the shapes that matter, fits a conservative model, and sets provisional defaults**; the architect confirms them from the table. |
| Status | **Assigned** (2026-10-22), after part 4a and the client HTTP assessment were reviewed Approved. Finding IS-52. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-5, R-1, R-3; [`../../accepted/011-read-path-resource-model.md`](../../accepted/011-read-path-resource-model.md) (the cost table is per prikk release); `recovery-followups-review-request.md` §4 (PK-68 measured) and `client-http-assessment-review-request.md` §9 (the 105 MiB finding); `crates/planeter-transport/src/propose.rs` (`ProposeConfig::accept_cost`, `accept_ceiling`); ledger PK-59, PK-68. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — Measure (in your own scratch area; not CI; nothing committed except what T2 needs)

With real prikk, the **same call the server makes** (`sync accept`, in a sandbox with the data ceiling lifted for measuring), on a store that starts
empty and on one that already holds an unrelated 5,000 patches. For each artifact report: artifact bytes, patches, blobs, **peak resident set and peak
address space (`VmPeak`) of the accept child**, wall time, and whether the accept completes under `1.5 × (10 MiB + 4.2 × bytes)` as it is set today.
Shapes (build each as the client would, `sync build` against the forge's `have`): **(a) one random (incompressible) blob** of 1, 5, 10, 25, 50, 100 and 250 MiB;
**(b) one highly compressible blob** (zeros / repeated text) of 10, 50 and 250 MiB; **(c) many medium files**, one patch each: 100 × 1 MiB, 25 × 4 MiB,
1,000 × 100 KiB; **(d) one patch touching many files**: 1 patch with 100 and 1,000 files of 100 KiB; **(e) a large file edited many times** (a 20 MiB text
file with 50 successive small edits, one patch each); **(f)** the patch-shaped sweep of part 4a again at 5,000 and 7,500 patches **with `VmPeak`** (so the ceiling
that is enforced, a data limit, can be compared with what was measured: `RLIMIT_DATA` limits the data segment, not RSS — say which of the two the numbers
are). Where an accept fails under the current ceiling, record at which size and the failure text.
**Fit:** report `peak ≈ a + b × artifact_bytes + c × patches + d × largest_blob_bytes` by least squares on the data (say how good the fit is) **and** the
worst observed ratio `peak / artifact_bytes` per shape. Time: report ms per patch and seconds per MiB per shape. Provide the raw table, the scripts' location (not committed) and
the method in the report.

## T2 — The model and the provisional defaults (code; after T1, one commit each)

(a) **One table, one place:** `ProposeConfig::accept_cost`/`accept_ceiling` are replaced by a documented cost model with **named constants**, annotated
"measured with prikk 0.53.0, date", taken from the *worst* shape of T1 (an admission that is too small kills a legitimate accept and flags the repository; one that is
too large only queues): `cost = base + k_bytes × artifact_bytes + k_objects × object_bound`, with the object bound the configured one. The data ceiling stays `1.5 × cost`. The model and
its constants have unit tests (including: the largest allowed artifact is admitted by an idle default budget, or the start-up line says it is not, as it does today).
(b) **Provisional defaults, until the architect confirms from the table:** `PRIKK_EXCHANGE_MAX_OBJECTS` **5,000** (was 20,000; the patch-shaped accept at that bound takes ≈ 190 s,
a third of the deadline, which leaves the margin R-3 asks for, and a larger history is a repository creation, D-4); the accept deadline stays **600 s**; `PRIKK_EXCHANGE_MAX_BYTES`
stays 256 MiB **unless T1 shows the largest allowed blob cannot be accepted under the model's ceiling and the budget**, in which case lower it to the largest size T1 proves works and say so.
Update `args::USAGE`, `docs/propose.md` (the settings table and the 1.1 GiB sentence), the start-up line, the OpenAPI text of the bound.
(c) **Tests:** a real-prikk test that a one-blob artifact of the **largest allowed size** is accepted end to end through the real sandbox with the default ceiling (that is the 105 MiB case, now
passing); one just over the object bound is refused by prikk (`accept_refused`, nothing flagged); the model's unit tests; a mutation that lowers `k_bytes` is caught.

## T3 — A ceiling hit must not look like a mystery

Today a ceiling kill sets `needs_recovery`, which blocks every writer until automatic recovery has run (seconds, but a member can repeat it within the rate limit). Report, from T1, in which
cases the ceiling kill happens **before any patch was written** (the pending set unchanged, only prikk's locks left behind) and **how long automatic recovery takes** in that case on this machine
(median of ten). Do **not** change the classification; the architect decides from the numbers whether a pre-check of the artifact's declared size (prikk's `archive`/`sync` preview
verbs, if one exists that reads only the header) is worth a part of its own.

## Non-change scope

No change to the framing, the check order, the quotas, recovery, the lease classes or the HTTP surface; the read path's cost table (RFC 011) is not touched; no new dependency; no `unsafe`; no version or release change.

## Acceptance criteria

T1's table for every shape (a failure is a row); the fit and the worst ratios; T2's constants in one place with their provenance; the provisional defaults changed and documented; T2(c)'s tests; T3's numbers. All earlier checks pass: IS-14 table,
OpenAPI drift guard, ENF-2, every gate green including both supply-chain gates (fresh advisory database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No constant without a measurement behind it; no fit presented without the data; no lowering of a bound to hide a failing shape without saying which shape and why; no change of the ceiling's `1.5 ×` margin without the numbers;
no claim that a shape was tested that was not run.

## Required evidence and review-request format

Commits (hash → what, by task); the T1 table, the fit, the worst ratios, the method and the scripts' location; the model's constants with provenance; the before/after of each default; the end-to-end test transcripts; T3's numbers;
every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
