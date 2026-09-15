# planeter RFCs

Design records for planeter, one per subsystem. This directory follows the ecosystem's **RFC lifecycle
policy** (project rules `000-rfc-lifecycle-policy.md`), **5-folder variant** — because the architect
(design) and the dev team (implementation) are distinct roles, so "the design is settled" (`accepted/`)
is a real event separate from "the work shipped" (`done/`).

The **schedule, phases, release cycles, and the RFC priority/order** live in [`../ROADMAP.md`](../ROADMAP.md).
This file is the **index**: it lists every RFC by state. The folder is the source of truth for state; a
file's Status field is kept consistent with its folder.

## Folders

```
rfcs/
  README.md      ← this index
  proposed/      ← open for review (implementer should not yet start)
  accepted/      ← design settled; implementer may start; not yet shipped
  done/          ← shipped
  archive/       ← withdrawn or superseded
  handoffs/      ← optional companion execution docs, under handoffs/NNN-slug/
```

## Pipeline (planned, from the roadmap)

**The RFC pipeline (001–009) is fully accepted**, each with a handoff (2026-09-15) — the design phase is
complete. Track A: **001** foundations (A0) · **002** authorization + **003** read path (A1 → M1) ·
**004** transport (M2) · **005** change/review (M3) · **006** CI (M4) · **007** registry (M5). Track B
(gated): **008** one-click merge / UD-6 · **009** hosted-format durability. See
[`../ROADMAP.md`](../ROADMAP.md) §The RFC plan. What remains before implementation begins in earnest: the
dev team builds against the handoffs (M0→M5, Track B in parallel), and the owner conveys the two
**prikk-side asks** to the prikk team — the UD-6 seal affordance (008 §D-4) and the format-migration
dependency (009 §D-4).

| # | RFC | Serves (phase) | State |
|---|---|---|---|
| 001 | [Foundations: prikk-integration layer, the layering gate, multi-repo hosting & identity](accepted/001-foundations.md) | A0→A1 | **accepted** |
| 002 | [Authorization model (`authorize()`, roles, per-ref permissions)](accepted/002-authorization.md) | A1 | **accepted** |
| 003 | [Read path & web browse (re-derivation, caching, safe rendering)](accepted/003-read-path.md) | A1 | **accepted** |
| 004 | [Transport: clone/push as a ferry over prikk's artifact exchange + client helper](accepted/004-transport.md) | A2 | **accepted** |
| 005 | [Change proposal, review, and merge (and issues)](accepted/005-change-review.md) | A3 | **accepted** |
| 006 | [Continuous integration and the runner protocol](accepted/006-ci-runners.md) | A4 | **accepted** |
| 007 | [Package and artifact registry](accepted/007-registry.md) | A5 | **accepted** |
| 008 | [*(joint prikk)* One-click merge without a forge key (UD-6)](accepted/008-seal-one-click-merge.md) | B1 | **accepted** (gated) |
| 009 | [Hosted-format durability: hosting prikk repos across an unstable format](accepted/009-hosted-format-durability.md) | B2 | **accepted** (gated) |

## Current state

| State | RFCs |
|---|---|
| Proposed | *(none — pipeline fully accepted)* |
| Accepted | [001 — Foundations](accepted/001-foundations.md), [002 — Authorization](accepted/002-authorization.md), [003 — Read path](accepted/003-read-path.md), [004 — Transport](accepted/004-transport.md), [005 — Change/review](accepted/005-change-review.md), [006 — CI + runners](accepted/006-ci-runners.md), [007 — Registry](accepted/007-registry.md), [008 — One-click merge / UD-6](accepted/008-seal-one-click-merge.md), [009 — Hosted-format durability](accepted/009-hosted-format-durability.md) |
| Done | *(none yet)* |
| Archived | *(none yet)* |

Governing design set: [`../docs/src/`](../docs/src/) — `planeter-01` requirements, `-02` external
design, `-03` threat model, `-04` internal design. Commons frame:
[forge-commons](https://github.com/kos-commons/forge-commons). Upstream:
[prikk](https://github.com/prikk-vcs/prikk).
