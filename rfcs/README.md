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

**RFC 001 is accepted** (2026-09-15) — the implementer may build A0 against it; its
[handoff](handoffs/001-foundations/foundations-handoff-v1.md) is written. **RFC 002 (authorization) and
RFC 003 (read path) are also accepted, with handoffs** — with A0 they complete the **M1 (0.1.0)** design
layer. **RFCs 004 (transport, M2), 005 (change/review, M3), and 008 (one-click merge / UD-6, Track B) are
accepted** (handoffs written). **RFC 006 (CI + runners) is proposed** for M4. The rest are planned in
priority order below (see [`../ROADMAP.md`](../ROADMAP.md) §The RFC plan) — remaining: 007 (registry) and
009 (durability).

| # | RFC | Serves (phase) | State |
|---|---|---|---|
| 001 | [Foundations: prikk-integration layer, the layering gate, multi-repo hosting & identity](accepted/001-foundations.md) | A0→A1 | **accepted** |
| 002 | [Authorization model (`authorize()`, roles, per-ref permissions)](accepted/002-authorization.md) | A1 | **accepted** |
| 003 | [Read path & web browse (re-derivation, caching, safe rendering)](accepted/003-read-path.md) | A1 | **accepted** |
| 004 | [Transport: clone/push as a ferry over prikk's artifact exchange + client helper](accepted/004-transport.md) | A2 | **accepted** |
| 005 | [Change proposal, review, and merge (and issues)](accepted/005-change-review.md) | A3 | **accepted** |
| 006 | [Continuous integration and the runner protocol](proposed/006-ci-runners.md) | A4 | **proposed** |
| 007 | Package registry | A5 | planned |
| 008 | [*(joint prikk)* One-click merge without a forge key (UD-6)](accepted/008-seal-one-click-merge.md) | B1 | **accepted** (gated) |
| 009 | Hosted-format durability policy | B2 | planned (gated) |

## Current state

| State | RFCs |
|---|---|
| Proposed | [006 — CI + runners](proposed/006-ci-runners.md) |
| Accepted | [001 — Foundations](accepted/001-foundations.md), [002 — Authorization](accepted/002-authorization.md), [003 — Read path](accepted/003-read-path.md), [004 — Transport](accepted/004-transport.md), [005 — Change/review](accepted/005-change-review.md), [008 — One-click merge / UD-6](accepted/008-seal-one-click-merge.md) |
| Done | *(none yet)* |
| Archived | *(none yet)* |

Governing design set: [`../docs/src/`](../docs/src/) — `planeter-01` requirements, `-02` external
design, `-03` threat model, `-04` internal design. Commons frame:
[forge-commons](https://github.com/kos-commons/forge-commons). Upstream:
[prikk](https://github.com/prikk-vcs/prikk).
