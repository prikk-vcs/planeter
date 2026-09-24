# planeter handoffs — structure, storage and status

Companion execution documents to the RFCs, per the 5-folder lifecycle
([`../done/000-rfc-lifecycle-policy.md`](../done/000-rfc-lifecycle-policy.md)). An RFC defines and
justifies a change; a **handoff** directs its implementation and must not silently redefine its RFC.

## Layout and naming

- One directory per RFC: `NNN-slug/` (`001-foundations/` … `009-hosted-format-durability/`).
  Supporting work without an RFC of its own (workflow §6.6: explicit and approved) lives under
  `interim/`, one file per work package, named by its subject and date or prikk version.
- Files: `<slug>-handoff-vN.md` (the handoff), `<slug>-review-request*.md` (the implementer's package
  back), `t<N>-*.md` (task-level records such as evaluations or gap catalogues). A **v2 supersedes v1
  and both stay**: the v1 keeps the reasoning that the measurement overturned.
- Every document opens with the header table — Document · Status · Basis · Audience · Scope — and the
  handoff body has three fixed parts: **task breakdown in build order** (`T1…`, each with purpose,
  applicable RFC decisions, change scope, non-change scope, required tests), the **QA checklist** drawn
  from the RFC's acceptance criteria, and the **definition of done and handback** (which gates, which
  evidence, what the review request must contain).
- Evidence is by reference: commit hashes on `main`, CI run ids, and gate outputs quoted with their exit
  codes. A claim without a reference is not evidence.

## Status of every handoff (2026-09-24)

| Directory | Handoff | Status |
|---|---|---|
| `001-foundations/` | v1 + A0 review request + T7 stikk-prikk evaluation | **done** — shipped in 0.1.0 |
| `002-authorization/` | v1 + A1 review request | **done** — shipped in 0.1.0; SSO landed in 0.2.0 |
| `003-read-path/` | v1 + review request + T7 gap catalogue | **done** — shipped in 0.1.0; UI in 0.1.1; the gap catalogue is closed except blame |
| `004-transport/` | v1 (2026-09-15), hold note 2026-09-23 | **held, not assignable** — T1/T2 superseded by the owner's TLS/SSH ruling; D-2's keyless-fetch assumption corrected by PK-27; **v2 is written when prikk ships RFC 155 then 154** |
| `005-change-review/` … `009-hosted-format-durability/` | v1 (2026-09-15) | **not assignable** — written before any prikk measurement; each is re-issued as v2 (re-checked against the dependency ledger and the shipped prikk) before assignment |
| `interim/prikk-0-47-0-rebaseline-handoff-v1.md` | v1 (2026-09-24) | **next assignment** — held until prikk 0.47.0 is released and installed (owner ruling) |

**Quarantine rule.** A handoff whose Status row does not say *assignable* (or *next assignment* with its
precondition met) is not implemented from. The implementer's first check is this table; the architect's
first duty at each prikk release and RFC disposition is to update it.

## What a review request carries back

The implementation summary; the commits (hash → what landed); every gate with its command and exit
code, including the test run **with** prikk and the run **with prikk shimmed off PATH**; the
dependency-tree delta with `cargo deny` / `cargo audit` results when a manifest changed; deviations from
the handoff and why; known limitations; open questions for the architect. The three existing review
requests (001–003) are the worked examples.
