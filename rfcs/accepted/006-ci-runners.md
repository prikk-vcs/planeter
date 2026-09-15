# RFC 006 — Continuous integration and the runner protocol

**Status.** Accepted (2026-09-15) — automation for M4. The design is settled and the implementer may
build the pipeline model, the runner protocol, and `planeter-runner` against it per the handoff. Defines
*how planeter runs untrusted build code safely* — on isolated, ephemeral runners in a separate trust
domain, with a job identity that can build but can never sign or merge.
Handoff: [`../handoffs/006-ci-runners/ci-runners-handoff-v1.md`](../handoffs/006-ci-runners/ci-runners-handoff-v1.md).
**Tracks.** ROADMAP Phase A4 → M4 (Track A). Requirements `CAP-6`, `SEC-4`, `OPS-4`; external design
`CI-01…04`; internal design `CIO-1…4`; threat model `T-5` (CI RCE), `C-5`, `INV-5`, `RR-6/7`. Consumes
RFC 002 (job authz), RFC 004 (a job pulls the change), and wires the check-gate seam RFC 005 left.
**Touches.** `planeter-ci` (pipeline model, orchestration, job-token minting) and `planeter-runner` (the
agent binary, deployed on separate hosts). **Not here:** the artifact/package registry (RFC 007 — CI
*produces into* it), and the merge model (RFC 005 — CI *gates* it).

## Summary

CI is the most dangerous feature a forge has, for one plain reason: **it is remote code execution the
forge invited in** — any workflow, including one from a stranger's fork, is arbitrary code planeter
agrees to run. This RFC builds it so a compromise buys an attacker as little as possible: **isolated,
ephemeral runners on separate hosts**, **secrets withheld from untrusted changes**, and a **job identity
scoped to that job that is never a signing key**. That last point ties CI to the whole trust model: a CI
job must not be able to seal or merge on a maintainer's behalf (`INV-5`, and the OQ-1(a)/`INV-2`
discipline) — CI reads code and reports results; it does not advance sealed history.

## The constraints that scope this design

- **Workflows are untrusted code** (`T-5`): fork pull requests are the classic "pwn request"; the runner
  is where compromise must be contained.
- **The runner is a different trust domain from the forge** (`INV-5`/`OPS-4`): it never shares a host,
  credentials, or a network it doesn't need with `planeter-web`/`core`.
- **A job identity is not a signing identity** (`INV-2`): giving CI the ability to seal would reintroduce
  a forge-side path to prikk-verified history — forbidden. CI's write reach is status/logs/artifacts,
  never a ref seal.
- **The action supply chain is an attack surface** (`T-11`, `RR-7`): a third-party step is code from a
  stranger in your pipeline.

## Decisions

- **D-1 — Pipelines: in-repository definitions, explicit triggers.** Workflows are defined **in the
  repository** (read via prikk, RFC 003) and trigger on **push** (RFC 004), on a **proposed change**
  (RFC 005), and on a **schedule**. `planeter-ci` resolves definitions and enqueues jobs.
- **D-2 — Workflow format: Actions-compatible, deliberately.** Adopt a **GitHub-Actions-compatible**
  workflow/step format so the large ecosystem of existing actions and authoring knowledge transfers
  (forge-commons "compatibility as strategy") — a considered choice, not a drift, and bounded by D-6's
  pinning. A leaner native format is the alternative if compatibility proves costly; decided here in
  favor of compatibility, revisited only against evidence.
- **D-3 — Runners: separate-host, containerized, ephemeral.** `planeter-runner` registers to `planeter-ci`
  over **HTTPS with a scoped registration token**, claims a job, executes it in a **fresh container**,
  streams status and logs, and is **torn down** after (or is single-use). Runners live on separate hosts
  in a distinct trust domain; be deliberately careful with Docker-in-Docker / host-socket mounts (they
  can hand a job the runner host).
- **D-4 — Least-privilege job identity (INV-5/INV-2).** Each job gets a **short-lived, job-scoped token**
  (a CI-job principal, RFC 002 D-2) able to do *that job* — read the change/ref, report status, write
  logs, publish to the registry it's authorized for — and **nothing else**. It is **never a maintainer
  signing key and can never seal or merge**. A job that wants to "merge on green" cannot; it reports a
  check result, and a maintainer's own seal merges (RFC 005/008).
- **D-5 — Secrets are withheld from untrusted changes (C-5b).** Secrets are **not exposed to
  fork-originated workflows** by default, and such a workflow **requires maintainer approval to run at
  all** — closing the pwn-request. Secrets are stored encrypted, injected at run time, never written to
  logs or workflow files.
- **D-6 — The action supply chain is pinned (C-5d/T-11).** Third-party actions/steps are **pinned by
  commit digest**, not a mutable tag; planeter makes digest-pinning the default and easy, and surfaces an
  unpinned reference as a warning. Reusing an action is code you are trusting — pin it and review it.
- **D-7 — CI feeds the merge gate (RFC 005 D-5).** A job's check result feeds the **required-check**
  protected-ref rule: a merge waits on green checks, evaluated at the authorization layer (RFC 002 D-6).
  This wires the seam RFC 005 left; CI **gates** the merge, it does not **perform** it.
- **D-8 — CI as integrity custodian (SEC-4).** A job may produce artifacts and (incrementally) **build
  provenance**; `planeter-ci` **stores and verifies** signatures/provenance (custodian, not key-holder) —
  generation is the job's own doing with its own materials, feeding RFC 007's registry.
- **D-9 — Bounded jobs (T-9).** Every job has **time, resource, and log-size limits**; a job exceeding
  them is terminated with a recorded reason, never left to exhaust the host.

## What "done" means (acceptance criteria)

- A workflow (Actions-compatible, D-2) triggers on push / proposed change / schedule; a job runs on a
  **separate-host, containerized, ephemeral** runner registered over HTTPS (D-1/D-3).
- **Isolation (tested):** a job cannot reach the forge's trust domain; the runner is torn down after; a
  job token is **job-scoped and cannot seal/merge or reach another repo** (`INV-5`/`INV-2`).
- **Fork safety (tested):** a fork-originated workflow gets **no secrets** and **requires approval** to
  run (`C-5b`).
- **Supply chain:** unpinned third-party actions are warned; digest-pinning is the default (D-6).
- **Merge gate (tested):** a required check must be green for a merge to be authorized (D-7); a red/absent
  check blocks the merge at the authorization layer.
- **Bounds (tested):** a job exceeding time/resource/log limits is terminated with a reason.
- Threat model **updated** — CI is a new, high-value execution surface: confirm `T-5`/`INV-5` controls;
  `INV-2` untouched (no job-token signing path); `planeter-03` revised per the release rule.

## Alternatives considered

- **Run jobs on the forge host / in the forge process.** Rejected (`INV-5`): untrusted code must not
  share the forge's trust domain; separate-host ephemeral runners are the containment.
- **A leaner native workflow format.** Deferred (D-2): Actions-compatibility buys an ecosystem now; the
  native option is revisited only if compatibility proves costly.
- **Let a CI job seal/merge on green ("auto-merge by CI").** Rejected (`INV-2`/`INV-5`): that hands CI a
  path to sealed history. Auto-merge, if wanted, is a maintainer pre-authorizing their **own** seal
  (RFC 005/008), never CI holding a key.
- **Trust third-party actions by tag.** Rejected (`T-11`): pin by digest; a mutable tag is an
  attacker-movable supply-chain hole.

## Open questions & dependencies

- **RFC 007 (registry)** — where CI artifacts/provenance land (D-8); the storage contract is RFC 007's.
- **OQ-4 (owner)** — the CI feature edge (matrix builds, reusable workflows, required-check policy
  granularity) may refine D-1/D-7; the core here suffices for M4.
- **Runner autoscaling** and hosted-runner provisioning are operational concerns (`OPS-*`), settled in the
  handoff / a later increment; the protocol (D-3) supports horizontal, ephemeral runners.

## Sequencing & handoff

Builds on RFC 002 (job authz), RFC 004 (a job pulls the change), and RFC 005 (the merge gate seam). On
acceptance, the architect writes `handoffs/006-ci-runners/` (the Actions-compatible format subset, the
runner registration/claim/report protocol, `planeter-runner`'s container isolation, the job-token scope,
the fork-secret policy, the digest-pinning, the merge-gate wiring, and the tests above — plus the
`planeter-03` update for the CI execution surface). Delivering this reaches **M4 (0.4.0)**. Then RFC 007
(registry) adds packages for M5. All v0.x; 1.0 is the owner gate.
