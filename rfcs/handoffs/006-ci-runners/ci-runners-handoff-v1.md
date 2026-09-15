# RFC 006 — CI + runners — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 006 (CI + the runner protocol). Task/PR plan + QA checklist. |
| Status | Inherited from RFC 006 — **Accepted**. |
| Basis | [`../../accepted/006-ci-runners.md`](../../accepted/006-ci-runners.md) (D-1…D-9); RFC 002 (job authz), RFC 004 (a job pulls the change), RFC 005 (merge-gate seam); threat model `T-5`, `C-5`, `INV-5`, `INV-2`. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | **Phase A4 → M4.** `planeter-ci` + `planeter-runner`. **Out of scope:** the registry (RFC 007 — CI publishes into it via D-8's seam). |

## Task breakdown (PR plan)

- **T1 — Pipeline model + triggers (D-1/D-2).** `planeter-ci`: read **in-repository** workflow
  definitions (via prikk, RFC 003) in a **GitHub-Actions-compatible** format (a defined, pinned subset);
  enqueue jobs on **push** (RFC 004), **proposed change** (RFC 005), and **schedule**.
- **T2 — The runner protocol + `planeter-runner` (D-3).** The agent registers over **HTTPS with a scoped
  registration token**, claims a job, runs it in a **fresh container**, streams status/logs, tears down.
  Separate host, distinct trust domain. Guard Docker-in-Docker / host-socket use.
- **T3 — Least-privilege job identity (D-4, INV-5/INV-2).** Mint a **short-lived, job-scoped** CI-job
  principal (RFC 002 D-2): read the change/ref, report status, write logs, publish to its authorized
  registry — **and nothing else**. **It is never a signing key and cannot seal/merge** (assert in tests).
- **T4 — Fork-secret policy (D-5, C-5b).** Withhold secrets from fork-originated workflows by default;
  require **maintainer approval** to run them at all. Secrets encrypted, injected at run time, never in
  logs/definitions.
- **T5 — Action supply chain (D-6).** Make **digest-pinning** the default; warn on unpinned third-party
  actions.
- **T6 — Merge-gate wiring (D-7).** Feed a job's check result into RFC 005's required-check protected-ref
  rule (RFC 002 authz): a merge waits on green checks. CI **gates**, never **performs**, the merge.
- **T7 — Integrity custodian (D-8).** Store and verify artifact signatures/build provenance (custodian,
  not key-holder), feeding RFC 007's registry.
- **T8 — Bounds (D-9) + threat-model update.** Per-job time/resource/log limits, terminate-with-reason on
  breach. **Update `planeter-03`** for the CI execution surface (`T-5`/`INV-5`; `INV-2` untouched).

## QA checklist (from RFC 006 acceptance)

- [ ] A workflow triggers on push/change/schedule; a job runs on a separate-host, containerized,
      ephemeral runner registered over HTTPS.
- [ ] **Isolation (tested):** job can't reach the forge trust domain; runner torn down; job token
      **job-scoped, cannot seal/merge or reach another repo** (`INV-5`/`INV-2`).
- [ ] **Fork safety (tested):** fork workflow gets no secrets and requires approval.
- [ ] Unpinned third-party actions warned; digest-pinning default.
- [ ] **Merge gate (tested):** a required check must be green for a merge to be authorized; red/absent
      blocks at the authorization layer.
- [ ] **Bounds (tested):** an over-limit job is terminated with a reason.
- [ ] Gates green; `planeter-03` updated for the CI surface.

## Definition of done & handback

Done when the checklist is green — **M4 (0.4.0)**. Return the review-request package naming the PRs, the
Actions-compatible subset, and the runner isolation mechanism. The architect proceeds to **RFC 007
(registry)** for M5. All v0.x; 1.0 is the owner gate.
