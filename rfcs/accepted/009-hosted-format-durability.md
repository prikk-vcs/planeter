# RFC 009 — Hosted-format durability: hosting prikk repos across an unstable format

**Status.** Accepted (2026-09-15) — **Track B, owner-gated (OQ-6).** The design is settled; the
implementer may build against it per the handoff. The owner accepted the **migrate-forward** policy (D-2)
in accepting this RFC; the prikk-side **migration dependency** (D-4) and prikk-1.0 timing (D-6) remain
coordination items. Required before 1.0.
Handoff: [`../handoffs/009-hosted-format-durability/hosted-format-durability-handoff-v1.md`](../handoffs/009-hosted-format-durability/hosted-format-durability-handoff-v1.md).
**Tracks.** ROADMAP Phase B2 (Track B). Requirements `UD-3`, `OQ-6`, `OPS-3`; external design `OP-04`,
`GATED-3`; internal design `CON`, `PKI-4`; threat model `RR-4`. Builds on RFC 001's version pinning
(D-3) and the repository record's `prikk_format_version` (D-5).
**Touches.** `planeter-prikk` (version support range), `planeter-core`/`planeter-store` (per-repo format
tracking, migration orchestration), and operations (`OPS-3` backup). **Not here:** prikk's own format
design or migration tooling (prikk's); the exact backup mechanism (`OPS-3`).

## Summary

planeter hosts **the largest repositories in some projects' lives** in prikk's on-disk format — and that
format is **explicitly unstable before prikk 1.0** ("internal to the CLI, may change without notice").
That is a real durability risk: a prikk format change could strand hosted repositories. This RFC makes
the risk **managed and explicit** rather than latent: planeter **pins the prikk format versions it
supports**, **refuses to host an unsupported one**, **migrates hosted repos deliberately** (with a
backup and a rehearsed restore), and **never adopts a new prikk version for hosting until a migration
path exists**. It proposes the durability *policy* for the owner to rule (OQ-6) and names the prikk-side
**migration dependency**. It is the reason hosting stays honest through prikk's pre-1.0 churn — and part
of why planeter itself is v0.x.

## The constraints that scope this design

- **prikk's format is unstable pre-1.0** (`UD-3`, survey): it stabilizes at prikk 1.0; before that a
  minor may change it. planeter cannot assume a fixed format.
- **A hosted repo must always be re-verifiable** (`INV-6`/`OP-04`): planeter must never silently hold a
  repo in a format it can no longer read/verify.
- **Migration is prikk's mechanism, not planeter's** (`BN-4`/`LAY-3`): planeter drives prikk; it does not
  reimplement prikk's format or its migration. Whether prikk offers an in-place migration is a
  **prikk-side dependency**.
- **The policy is the owner's** (`OQ-6`): what planeter *promises* hosted repos across format changes is
  a product-durability decision, not the architect's to set unilaterally.

## Decisions

- **D-1 — Pin the supported prikk format versions (formalizing RFC 001 D-3).** `planeter-prikk` declares
  a **supported format-version range**; `planeter-store` records each repo's `prikk_format_version`
  (RFC 001 D-5). planeter **refuses to host or operate on** a repo whose format is outside the range —
  a refusal with a reason, never a guess (`OP-04`).
- **D-2 — The durability policy (proposed; owner rules OQ-6).** The recommended policy: **support a
  bounded window of prikk format versions, and migrate hosted repos forward deliberately** when planeter
  adopts a newer prikk. Concretely: (i) a new prikk version is adopted for hosting only after D-3's gate;
  (ii) on adoption, hosted repos are migrated (D-4); (iii) planeter documents the supported window so
  operators know what a given planeter version can host. *(Alternatives the owner may prefer: a wider
  "support many versions read-only" stance, or — pre-1.0 — an explicit "host at operator's risk, no
  migration promise" stance. Recommended is migrate-forward.)*
- **D-3 — The adoption gate: never risk hosted repos.** planeter does **not** upgrade the prikk version
  it hosts with until: the new format is in D-1's supported range; a **migration path exists** (D-4); and
  a **backup + rehearsed restore** (`OPS-3`) is in place. Absent any of these, planeter stays on the
  current prikk version for hosting. The forge's durability is never gambled on an unvetted format bump.
- **D-4 — Migration mechanism (a prikk dependency).** When prikk introduces a new format, planeter
  migrates each hosted repo by **driving prikk's own migration** (if prikk provides one — the named
  prikk-side dependency), per-repo, **serialized** (`CON-1`) and **after a backup**. If prikk offers no
  in-place migration for a given step, the fallbacks are (a) hold on the prior prikk version within the
  supported window, or (b) a re-import path — both slower, both explicit. planeter targets whatever
  prikk's migration story turns out to be; it does not invent one (`BN-4`).
- **D-5 — Operator-facing honesty (OP-04/OP-06).** planeter surfaces each repo's `prikk_format_version`
  and the planeter version's supported window; a repo that would fall out of support is flagged **before**
  an upgrade, not stranded after. planeter never presents a repo it cannot re-verify as healthy.
- **D-6 — Alignment with prikk 1.0 and planeter's v0→v1 gate.** This risk is inherent to hosting a
  pre-1.0 VCS: prikk's format firming at prikk 1.0 is what lets planeter's durability promise firm too.
  Until then, hosted-format is treated as **versioned and migratable**, and this shared pre-1.0 risk is
  part of why **planeter is v0.x** (ROADMAP owner gate). planeter's own 1.0 durability promise should not
  outrun prikk's format stability.

## What "done" means (acceptance criteria)

- planeter **refuses** a repo outside the supported format range with a reason; records and surfaces each
  repo's `prikk_format_version` (D-1/D-5).
- The **adoption gate (D-3)** is enforced: a prikk version-bump for hosting is blocked until format
  support + migration path + backup/restore rehearsal are in place (tested as a gate, e.g. in CI/release
  tooling).
- A **migration run (D-4)** over hosted repos is per-repo, serialized, backup-first, and **verifiable**
  (re-derive/verify after migration); a failed migration leaves the repo on its prior good state, never
  half-migrated.
- The **owner's OQ-6 ruling** is recorded here (which policy of D-2); the **prikk migration dependency**
  is raised with the prikk team.
- Threat model re-verified: `RR-4` is now a *managed* risk with a stated policy, not a latent one.

## Alternatives considered

- **No durability promise pre-1.0 ("host at operator's risk").** A valid owner choice (D-2) — simplest,
  but it pushes the risk onto operators and weakens the "your history is safe here" proposition. Recorded
  as an option; migrate-forward is recommended.
- **planeter reimplements prikk's format / migration.** Rejected (`BN-4`/`LAY-3`): that pushes weight into
  the forge and forks prikk's format understanding. Drive prikk's migration; name the dependency.
- **Support every historical format forever.** Rejected: an unbounded obligation; a bounded, documented
  window with migrate-forward is the sustainable posture (D-2).

## Open questions & dependencies

- **OQ-6 (owner)** — the durability policy of D-2 (migrate-forward vs. wider read-only support vs.
  at-operator's-risk). This RFC proposes migrate-forward; the owner rules.
- **prikk migration (prikk team)** — whether prikk offers an in-place format migration planeter can drive
  (D-4); a companion prikk ask, like brygge's UD- dependencies. planeter adapts to the answer.
- **prikk 1.0 timing (D-6)** — planeter's 1.0 durability promise is coupled to prikk's format stabilizing;
  coordinated via the owner.

## Sequencing & handoff

Track B, owner-gated; it does not block Tracks A's feature milestones but is **required before planeter
1.0**. On acceptance (and the owner's OQ-6 ruling), the architect writes
`handoffs/009-hosted-format-durability/` (the version-range declaration, the adoption gate as
release/CI tooling, the per-repo migration orchestration with backup/verify, and the operator surfacing)
and raises the prikk migration dependency. With this, the RFC pipeline (001–009) is complete: planeter
has an accepted design and RFC set from foundations through the path to 1.0. All v0.x; 1.0 is the owner
gate.
