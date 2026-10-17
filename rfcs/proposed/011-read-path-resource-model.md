# RFC 011 — The read-path resource model: bounding memory, size and lifetime of what planeter asks prikk to do

**Status.** Proposed (2026-10-17) by the architect, from the owner's question whether running prikk as a
subprocess makes the server more stable (SCHEDULE D-16) and the measurements that answered it. **Direction
accepted by the owner 2026-10-17 (O-1) and O-3 granted; O-2 (the budget default, the variable's name, the
status of `too_large`) awaits the owner's answer to the architect's recommendations,** after which the RFC
moves to `accepted/`. Handoffs are written after acceptance.
**Tracks.** Issue register IS-36 (the gate does not bound memory; the `file` route is unbounded) and IS-37
(a killed server leaves its prikk tree running); threat model T-9 / C-9a / C-9b, C-4c, TB-4; RFC 001 D-3 and
its "Alternatives considered"; RFC 003 D-8 (the gate); dependency ledger PK-46, PK-51, PK-52.
**Touches.** `planeter-prikk` (`sandbox.rs`, the call path), `planeter-web` (`gate.rs`, `api.rs`, `errors.rs`,
`openapi.rs`), one new operator variable, the CHANGELOG. **Not here:** the process model itself (kept), any
write path (RFC 004), per-tenant fairness and rate limits (IS-25, C-9a), metering (C-9c), prikk's own changes.

## Summary

planeter reads every repository by running prikk in a sandboxed subprocess. The gate (RFC 003 D-8) bounds
**how many** such processes run, and `timeout` bounds **how long**. Nothing bounds **how much memory** they
use, and the server cannot see it. We measured it (D-16): eight concurrent `verify` calls on a 1 GiB
repository used 16.1 GB, half the host's RAM, while the server showed 3.5 MB; the `file` route reads a whole
file into JSON in prikk and again in the server with no size bound; and a killed server leaves its prikk tree
running to the end of the call. None of this depends on the process model: linking prikk in-process would
move the same memory into the server (measured) and lose the sandbox (RFC 001). This RFC makes the resource
model explicit and enforced: **a memory budget with weighted admission, a per-call ceiling, a size bound on
file content, and a child lifetime tied to the server**, with honest errors for each.

## Evidence (verified facts)

All from the measurement review `read-process-model-measurement-review-v1` (private) and the ledger rows
PK-51 and PK-52; prikk 0.52.0; a shared, non-idle development host (so latencies are upper bounds).

- **F1. The gate counts processes.** A call is four processes (`timeout`, two `bwrap`, `prikk`). The gate
  default is `min(CPUs, 8)`.
- **F2. Peak child memory scales with the repository, not the answer:** `log` ≈ 9.5 MB at any size;
  `tree`/`cat` ≈ 1.05 × the repository; `verify` ≈ 2.05 ×; the patch-plan `file` path ≈ 3.1 × the file, with
  the server holding another ≈ 3.1 × while it parses the JSON. 8 × `verify` on 1 GiB = 16.1 GB (50.6% of
  RAM); never above 80% in any run; the server's RSS stayed 3–5 MB.
- **F3. `GET …/file` has no size bound** (200 MiB text file: 628 MB in prikk + 623 MB in the server per
  request). The raw route is capped at 4 MiB; when it exceeds the cap it answers `Unavailable`, which the API
  reports as `unavailable` ("could not be read right now"): a wrong message, since retrying cannot help.
- **F4. A per-process address-space limit reaches prikk through `bwrap`** (`prlimit --data|--as`, or a
  wrapper in `BwrapConfig.timeout`), with no measurable latency cost. Below a call's need prikk aborts
  (exit 134, 40–400 ms) or prints a **partial** `verify` report and exits 1 ("did not complete every stage").
  Lowest ceilings that worked on a 200 MiB repository: `log` 16 MiB, `tree` 256 MiB, `verify` 512 MiB, `file`
  of a 200 MiB text file 1 GiB.
- **F5. Under cgroup memory pressure the kernel killed the sandboxed child, never the server** (7 runs),
  because the child was the largest process. Not tested: a server larger than its child; a system-wide OOM.
- **F6. A timed-out call leaves nothing behind. A `SIGKILL` of the server does:** the whole
  `timeout`/`bwrap`/`prikk` tree ran on for 3.55 s (bounded by 120 s). `--die-with-parent` binds `bwrap` to
  `timeout`; nothing binds `timeout` to the server. The repository was unchanged in every case.
- **F7. Reading one file costs the whole repository** (`cat` of a 1 MiB blob: 6 ms in a 1 MiB repository,
  84 ms in a 64 MiB one, ≈ 1.2 s and ≈ 1.03 GB in a 1 GiB one). This is prikk's cost, not ours (PK-46).
- **F8. In-process is possible but not attractive today:** `prikk-store` served `log`/`tree`/`cat`/`verify`;
  it saved 1.6–3.5 ms (1 MiB) and 13–26 ms (64 MiB) and no memory; it is not read-only by construction,
  exports signing and commit API unconditionally, and ENF-2's marker grep would not detect linked signing
  code (PK-52). Rust cannot, in-process, enforce a hard per-call memory limit (an allocation failure aborts
  the process on stable), kill a stuck call, or catch an abort or stack overflow.

## Unknowns (to be measured, not assumed)

- **U1. A cheap, trustworthy estimate of a repository's size** (the input to F2's multipliers). Candidates:
  a bounded directory walk of `.prikk` cached per repository with a short time-to-live; a stat-based
  estimate. Not measured. The handoff's first task measures the cost and accuracy of the candidates.
- **U2. How a ceiling is applied without a new unsafe block or a new prerequisite:** a wrapper command
  named in `BwrapConfig.timeout` (`prlimit` is in util-linux, present on the supported hosts but a new
  runtime prerequisite) versus `setrlimit` in a pre-exec hook (needs `unsafe`, and `planeter-prikk` declares
  `#![forbid(unsafe_code)]`, as do the other library crates). To be settled in the handoff with the project's unsafe-code rule in view.
- **U3. How to tie the child's lifetime to the server:** `PR_SET_PDEATHSIG` fires when the parent *thread*
  exits, and a tokio blocking-pool thread can exit while the process lives, which would kill healthy
  children early. A supervised process group, a dedicated spawner thread or a wall-time implementation inside
  planeter in place of `timeout` are candidates. The handoff must test with a real server kill.
- **U4. The right default budget.** Measured multipliers and a host's free memory give a safe order of
  magnitude; the default needs a rule (see D-1) and an operator override.

## Decisions

- **D-1 — Memory-weighted admission.** Besides the process count, the gate holds a **byte budget**. Each read
  is admitted with a **cost** derived from the verb and the repository size (F2, with a fixed floor for
  `log`), waits at most the existing bounded wait for budget, and is refused with `busy` and `Retry-After`
  when the wait ends. A request whose own cost exceeds the whole budget is refused at once with a distinct
  error (D-4), never queued. The budget defaults to **a quarter of the memory the process may use** (the
  cgroup limit if one is set, otherwise physical memory) and is set by one new variable,
  `PLANETER_PRIKK_MEMORY_BUDGET_MB`, validated like the existing gate variables and logged at start. The cost
  model lives in one table, covered by a test that fails when a verb is added without a cost. In-process or
  subprocess, the same budget applies (the design does not depend on the process model).
- **D-2 — Per-call ceiling.** Every sandboxed prikk call runs with an address-space or data limit of
  **cost × 1.25** (never above the budget). The limit is the enforcement the admission estimate cannot give:
  a wrong estimate costs one failed call, not the host. **A call that hits its ceiling is never reported as a
  verification result:** an abort (exit 134) maps to `unavailable` with a log line naming the ceiling, and a
  partial `verify` report (exit 1, incomplete stages) maps to `unavailable`, not to "unverified". A test
  proves this with a deliberately low ceiling.
- **D-3 — Bound the content the API returns.** `GET …/file` is limited to the same size as raw (4 MiB
  today), decided **before** reading the content where the size is known cheaply, otherwise by bounding the
  verb's output (`cat --max-bytes` exists and raw uses it). The bound is one named constant used by both
  routes. A file above it is answered by D-4's error, with a hint that points to the raw download on the
  content origin. The server's own buffer is thereby bounded by the same constant.
- **D-4 — Say what happened.** One new error code, `too_large` (the response would exceed what this server
  serves, or the repository needs more memory than the budget permits), status **`422`**, message "This is too
  large to serve here.", hint "Download it from the content origin, or ask the operator to raise the read
  memory budget." The over-cap raw route uses it too (replacing `unavailable`, which implied a retry could
  help). `busy` keeps its meaning (try again soon). The OpenAPI document and the drift guard gain the code.
- **D-5 — A child never outlives the server by more than its call.** The mechanism is chosen in the handoff
  (U3); the requirement is a test that kills the server during an in-flight call and observes that no
  `timeout`, `bwrap` or `prikk` process of that call remains after a bounded settling time (a few seconds),
  and that the wall-time bound and the 5 s kill-after still hold.
- **D-6 — Visibility.** The gate counts admitted, waited, refused-busy, refused-too-large and the in-flight
  and high-water budget, and logs a refusal with the cost and the budget. No endpoint or periodic output is
  added (OPS-5 owns metrics); the counters are what metrics will read. A counting allocator is **not**
  introduced: the budget accounts for children by estimate and ceiling, and the server's own buffers are
  bounded by D-3.
- **D-7 — The process model stays as is; the door stays closed, with named conditions.** RFC 001's rejection
  of linking prikk crates stands for the reasons measured in F8 and the security reasons in C-4c. It is
  reopened only if prikk offers a stable library facet that is read-only by construction and carries no
  signing code **and** our measurements show process start matters; before any prikk crate becomes a
  dependency, ENF-2's check is strengthened to detect signing code by what is linked, not by one marker name.
- **D-8 — What we ask prikk (the owner conveys; granted O-3).** The 0.54.0 "files at a point" command
  (PK-46) should cost **the point it writes, not the repository's store** (F7); let us learn the point's file
  count and bytes before paying for it; refuse past a limit, all-or-nothing and before writing; and state its
  memory need. Two questions about `cat`/`tree`: is the whole-repository cost inherent, and is the memory
  multiple a documented, stable property. Drafted as a letter for the owner to convey.

## Alternatives considered

- **Do nothing; trust `timeout` and the gate.** Rejected: F2 shows a half-RAM spike from a legal workload.
- **Cgroup per call.** Strongest enforcement, but needs delegated cgroups (systemd/containers) planeter must
  not assume. A deployment that has them can apply `MemoryMax` to the whole unit as a second line.
- **A counting allocator in the server.** Measures the server's own buffers only; they are small once D-3
  holds. Revisit if an in-process design is ever reopened (D-7).
- **Refuse by repository size alone.** Simple and wrong: `log` is cheap on any repository; the cost depends
  on the verb.
- **Worker processes that link `prikk-store`.** Gives in-process speed with hard ceilings, at the price of
  PK-52's problems in a second binary. Not now.

## Risks

- **A wrong size estimate** admits too much (the ceiling then fails one call) or refuses too much (a
  legitimate read answered `busy`). Mitigation: the ceiling, the over-estimate bias, and the counters that
  show it.
- **Ceilings fail large repositories every time** (F4). That is the intended trade: a deterministic
  `too_large` that tells the operator which knob to turn, instead of a host-wide incident.
- **A new runtime prerequisite** (`prlimit`) if U2 resolves that way; mitigated by a start-up check with a
  clear message, like the existing prikk floor check.
- **Behaviour change in an API client's eyes:** a large `file` that used to be served is `422`. The API is
  days old and this ships with the 0.4.0 contract notes.

## Schedule and relation to other work

Not part of 0.3.0 (the exposure needs operator-placed content until a write path exists). **A precondition of
RFC 004's first write path** and of any public deployment guidance for repositories above a few hundred MiB.
Proposed as SCHEDULE theme T15, before RFC 004's acceptance; its handoffs (a size/cost investigation U1–U3
first, then implementation) follow acceptance.

## Verification plan

Tests under `tests/`: the cost table is complete; a request above the whole budget is `too_large` at once; a
second request waits for budget and is served when the first finishes; the budget is released on completion,
timeout, cancellation and abort; a low ceiling yields `unavailable` and never a verification verdict; `file`
and raw refuse above the shared constant with `too_large`; killing the server leaves no child (D-5); the
OpenAPI drift guard covers `too_large`. Evidence includes a repeat of the C2 worst case showing the
descendants' memory held at the budget.

## Open questions

1. Is a quarter of available memory the right default budget, or should the server refuse to start without an
   explicit one on hosts above a size?
2. Should D-3's bound be a variable? (Recommendation: no; one constant, like raw's, until a deployment needs
   otherwise.)
3. Status `422` versus a 4xx/5xx of your preference for `too_large`.

## Owner decisions

- **O-1.** Accept the direction (D-1 to D-8) and place theme T15 before RFC 004's acceptance.
- **O-2.** The default budget rule (D-1) and the variable's name; the status for `too_large` (D-4).
- **O-3.** Convey D-8 to prikk with the requirements note.
