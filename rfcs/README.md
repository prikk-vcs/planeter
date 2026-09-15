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

None are written yet — planeter is at the design-set stage. The order below is the planned priority
(see [`../ROADMAP.md`](../ROADMAP.md) §The RFC plan); each is authored as a `proposed/` RFC + handoff on
the owner's go-ahead, **RFC 001 first**.

| # | RFC (planned) | Serves (phase) | State |
|---|---|---|---|
| 001 | Foundations: prikk-integration layer, the layering gate, multi-repo hosting & identity | A0→A1 | planned |
| 002 | Authorization model (`authorize()`, roles, per-ref permissions) | A1 | planned |
| 003 | Read path & web browse (re-derivation, caching, safe rendering) | A1 | planned |
| 004 | Transport envelope + client helper (fetch/push over ferried artifacts) | A2 | planned |
| 005 | Change/review model (accepted-unsealed-as-PR, review, keyless merge) | A3 | planned |
| 006 | CI + runner protocol | A4 | planned |
| 007 | Package registry | A5 | planned |
| 008 | *(joint prikk)* Seal / UD-6 — one-click client-sealable merge | B1 | planned (gated) |
| 009 | Hosted-format durability policy | B2 | planned (gated) |

## Current state

| State | RFCs |
|---|---|
| Proposed | *(none yet)* |
| Accepted | *(none yet)* |
| Done | *(none yet)* |
| Archived | *(none yet)* |

Governing design set: [`../docs/src/`](../docs/src/) — `planeter-01` requirements, `-02` external
design, `-03` threat model, `-04` internal design. Commons frame:
[forge-commons](https://github.com/kos-commons/forge-commons). Upstream:
[prikk](https://github.com/prikk-vcs/prikk).
