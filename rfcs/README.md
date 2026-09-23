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
[`../ROADMAP.md`](../ROADMAP.md) §The RFC plan.

**Where it stands (2026-09-24).** **001–003 are done**: shipped as M1 (planeter 0.1.0, 2026-09-23) and
extended by the 0.1.1 / 0.2.0 read-side increments (browse UI, sessions, trusted proxies, OpenID Connect).
**004 is held** — measured on prikk 0.46.0, a keyless forge cannot build incremental fetch artifacts and
has no servable canonical branch until prikk ships RFC 155 then RFC 154 (after prikk 0.49.0); see
`handoffs/004-transport/`. 005–007 wait on 004. The two prikk-side asks (008 §D-4, 009 §D-4) were
conveyed and answered: prikk RFC 154 (trusted fast-forward adoption) and RFC 155 (the repository-complete
artifact) are accepted by the prikk owner, so 008 and 009 are gated on those shipping.

| # | RFC | Serves (phase) | State |
|---|---|---|---|
| 001 | [Foundations: prikk-integration layer, the layering gate, multi-repo hosting & identity](done/001-foundations.md) | A0→A1 | **done** (0.1.0) |
| 002 | [Authorization model (`authorize()`, roles, per-ref permissions)](done/002-authorization.md) | A1 | **done** (0.1.0; SSO in 0.2.0) |
| 003 | [Read path & web browse (re-derivation, caching, safe rendering)](done/003-read-path.md) | A1 | **done** (0.1.0; UI in 0.1.1) |
| 004 | [Transport: clone/push as a ferry over prikk's artifact exchange + client helper](accepted/004-transport.md) | A2 | **accepted** (held until prikk RFC 155 + 154 ship) |
| 005 | [Change proposal, review, and merge (and issues)](accepted/005-change-review.md) | A3 | **accepted** |
| 006 | [Continuous integration and the runner protocol](accepted/006-ci-runners.md) | A4 | **accepted** |
| 007 | [Package and artifact registry](accepted/007-registry.md) | A5 | **accepted** |
| 008 | [*(joint prikk)* One-click merge without a forge key (UD-6)](accepted/008-seal-one-click-merge.md) | B1 | **accepted** (gated) |
| 009 | [Hosted-format durability: hosting prikk repos across an unstable format](accepted/009-hosted-format-durability.md) | B2 | **accepted** (gated) |

## Current state

| State | RFCs |
|---|---|
| Proposed | *(none — pipeline fully accepted)* |
| Accepted | [004 — Transport](accepted/004-transport.md), [005 — Change/review](accepted/005-change-review.md), [006 — CI + runners](accepted/006-ci-runners.md), [007 — Registry](accepted/007-registry.md), [008 — One-click merge / UD-6](accepted/008-seal-one-click-merge.md), [009 — Hosted-format durability](accepted/009-hosted-format-durability.md) |
| Done | [001 — Foundations](done/001-foundations.md), [002 — Authorization](done/002-authorization.md), [003 — Read path](done/003-read-path.md) — shipped in 0.1.0 (2026-09-23) |
| Archived | *(none yet)* |

Governing design set: [`../docs/src/`](../docs/src/) — `planeter-01` requirements, `-02` external
design, `-03` threat model, `-04` internal design. Commons frame:
[forge-commons](https://github.com/kos-commons/forge-commons). Upstream:
[prikk](https://github.com/prikk-vcs/prikk).
