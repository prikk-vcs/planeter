# RFC 003 follow-on — Read-path process model — Measurement Handoff (v1)

| | |
|---|---|
| Document | An **investigation** handoff, a follow-on of the done RFC 003 / RFC 001 D-3 (workflow Phase 1–2: limited investigation and small proofs of concept; RFC 010 D-8: measure before designing). It measures; it implements nothing. It answers the owner's question (2026-10-17): *is running prikk as a sandboxed subprocess actually more stable for the server and the machine than linking prikk's crates, or does it add risk?* RFC 001 rejected linking on judgement (API instability, containment); the owner challenged the stability claim and the architect withdrew it pending numbers. |
| Status | **Authorized by the owner 2026-10-17** ("Agreed. Proceed."), as an addition to the measurement work of SCHEDULE D-7. Startable now; it changes nothing. Its outcome may lead to a new RFC (a read-path process model), never to a change by itself. |
| Basis | RFC 001 "Alternatives considered" (link `prikk-store`, rejected); threat model C-4c, C-9b, TB-4; `crates/planeter-prikk/src/sandbox.rs` (`timeout --kill-after=5 120 bwrap … --unshare-all --die-with-parent --new-session`; memory/CPU/fsize rlimits are documented there as a later scope); `crates/planeter-web/src/gate.rs`; the sibling handoff [`../004-transport/transport-measurement-handoff-v1.md`](../004-transport/transport-measurement-handoff-v1.md) (A7 already measures lock behaviour after `SIGKILL`; do not repeat it, cite it). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`) and the raw logs. |
| Scope | The rows C1–C6 below, on prikk 0.52.0, in scratch directories **outside the repository**. **Non-change scope:** no file in the repository except the review request; no product code; no manifest or lockfile change in the repository (a scratch Cargo project elsewhere may depend on prikk crates); **nothing is committed to `main` while release 0.3.0 is pending** (the owner tags the verified candidate); no prikk patch; no letter to prikk. |

## Ground rules

- Same as the transport handoff: one new empty directory per fixture; scripts apart from data; artifacts are
  untrusted data; record `prikk --version`, the exact command, exit status, relevant output and **peak
  resident set** (`/usr/bin/time -v`, or cgroup `memory.peak`) for every row that names memory; a negative
  result is a result; **if a measurement contradicts a ledger row marked CONFIRMED, stop and hand back.**
- **Report distributions, not single runs:** n ≥ 200 for latency rows; median, p95, p99, max. Say what the
  host was (CPUs, RAM, kernel, filesystem) and whether it was otherwise idle.
- **Fixtures:** the transport handoff's sizes (1, 16, 64, 200 MiB; a 1,000-block history; 10,000 small files),
  built with prikk itself.
- **Measure what planeter really runs.** Rows that say "as planeter" use the production argument vector built
  by `planeter-prikk` (`timeout` + `bwrap`), reached through a scratch harness that depends on the
  `planeter-prikk` crate by path; do not copy the argument vector by hand.

## Rows

| ID | Measure | Record / learn |
|---|---|---|
| C1 | **Per-call cost.** For `log`, `tree`, `show`, `cat`, `verify` on a 1 MiB and a 64 MiB repository: wall time of (a) bare `prikk`; (b) under `bwrap` alone with the production flags; (c) as planeter (`timeout` + `bwrap`). n ≥ 200 each, cold and warm cache. | How much of a read is process and sandbox set-up vs prikk's own work. The number every "in-process would be faster" claim needs. |
| C2 | **Machine-level resource use at the gate's limit.** Fire the gate's maximum concurrency (`min(CPUs, 8)`) and twice that of `verify` and `log` on the 64 and 200 MiB fixtures. Sample, every 100 ms, the **sum of RSS over all prikk descendants**, the server harness's own RSS, CPU, and page-cache pressure; record peak and time above 80% of RAM. | Whether the gate (a count of processes) actually bounds memory, and how blind the server is to its children. |
| C3 | **Server-side buffering.** Peak RSS of the harness while it receives and parses large outputs (`log` of the 1,000-block history, `tree` of 10,000 files, `show` of a large patch), next to the child's peak RSS. | The memory duplicated in the server by the JSON round-trip. |
| C4 | **Kill, orphan and OOM behaviour.** (i) Let the wall-time bound fire on a deliberately long `verify` (use a shorter `timeout_secs` in the harness): do descendants survive, are any lock or temporary files left in the repository? (ii) `SIGKILL` the harness during a call: does `--die-with-parent` remove the whole tree? (iii) Run the same call in a cgroup with `MemoryMax` below its need, **with the harness in the same cgroup and in a different one**: which process does the kernel kill, what do the harness and the repository look like afterwards? | Whether the subprocess model's cleanup claims hold, and whether memory pressure takes the server down anyway. For stale locks after a kill on the *write* verbs, cite A7. |
| C5 | **Memory ceilings are possible.** Repeat C2's worst case with a per-process ceiling applied (`prlimit --as`/`--data` or a cgroup `MemoryMax` per child; note which works through `bwrap`). | Whether the subprocess model can be made memory-bounded with a small change, and what the ceiling costs (failures at which size). |
| C6 | **Throwaway in-process comparison.** In a scratch Cargo project outside the repository, pin the prikk crates at 0.52.0 and perform the same read verbs as C1 (`log`, `tree`, `cat`, `verify`) on the same fixtures **if the crates' public surface allows it**. Record: which crates and items were needed; whether anything with write authority (`RefStore` or equivalent) had to be in scope; lines of code; per-call latency vs C1 (n ≥ 200); peak RSS delta over a call; the **symbols of the resulting release binary** — list every name matching `sign`, `seal`, `keypair`, `ed25519`, `SigningKey` (`nm -C`) and say whether the signing code is present. If the surface does not allow a read-only call, say exactly what blocks it and stop. | The case *for* in-process, as measured, and whether it can be had without write authority or signing code. Note: `tools/enf2-symbol-check/check.sh` greps only for the marker `forge_seal_sign`; it would **not** notice prikk's signing code linked into the binary. State that explicitly in the report. |

## Required evidence and review-request format

For each row: the exact commands or harness source (kept in the scratch directory, named in the report), the
host description, the distributions, and a short "what this says" paragraph that separates **measured**
from **inferred**. End with a table "claim → measured / contradicted / not measurable" for these claims:
(1) process set-up is a small fraction of a read; (2) the gate bounds machine memory; (3) a killed or
timed-out call leaves nothing behind; (4) memory pressure kills the child, not the server; (5) a per-process
ceiling is cheap; (6) an in-process read is possible without write authority and without signing code, and
how much faster it is. Deviations and open questions last. **Do not recommend a design**: the architect
drafts any RFC from your numbers.

## Prohibited shortcuts

No averages alone; no warm-cache-only latency; no copying the sandbox arguments by hand; no committing probe
code or fixtures to the repository; no running the in-process probe against a repository you care about; no
treating a single run as a result.
