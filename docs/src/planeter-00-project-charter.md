# planeter — Project Charter: background, goals, and governance

| | |
|---|---|
| Document | The project's background and goals in one place — why planeter exists, what "done" means, what it will not do, who decides what — for anyone joining the project. The requirements (`planeter-01`) are the contract; this is the frame around it. |
| Version | v1.0 |
| Date | 2026-09-24 (written after the 0.2.0 release; history below is as of that date) |
| Basis | prikk `ROADMAP.md` hosting direction (2026-09-06) and prikk RFC 145 (Shape D); forge-commons; the planeter design set v0.1 (2026-09-15) and its revisions; the owner's rulings recorded in `ROADMAP.md` and `docs/STATUS.md`. |

## 1. Background

**prikk** is a verifiability-focused version-control system: a lean, offline, single-repository CLI
with Ed25519 author and maintainer signatures, TOFU maintainer trust, offline `verify`, and artifact
based exchange (`bundle`, `sync`). By settled design it has no network, no accounts, no multi-repository
namespace, no ref authorization and no multi-user concurrency — and its owner ruled that full forge
hosting is *not prikk's to build*. RFC 145 resolved the wanted shape: prikk ships plumbing (a
machine-readable `--format json` read surface, artifact exchange); the ecosystem builds views over the
CLI, never over prikk's unstable crate.

The ecosystem around prikk follows one pattern: each companion carries a weight prikk refuses, so prikk
stays small. **stikk** explains prikk to humans (a front-end that owns no authority). **brygge** carries
the *dependency* weight (history import from Git and others). **forge-commons** is the vendor-neutral
standards, proposals and guidelines commons the forge is built against. **planeter** (Norwegian:
*planets* — bodies orbiting a common star) carries the *hosting* weight: the forge.

## 2. Goal

**Be a real, familiar forge for prikk** — the web service that hosts many prikk repositories over the
network and provides the collaboration loop a developer recognizes from GitHub, GitLab or Forgejo:
clone and push, propose and review changes, issues, CI, packages, teams and permissions, over a web UI
and an API (requirements PU-1). "Familiar" is a requirement, not a slogan: a person who has used a forge
should recognize planeter as one.

The four commitments that make that goal *this* project rather than any forge:

1. **Carry the hosting weight so prikk stays lean** (PU-2, BN-4). Nothing planeter needs may push a
   network stack, an account model, ref authorization or a dependency back into prikk.
2. **Build on prikk's stable CLI surface, never its internals** (PU-3, INT-1). prikk is driven as a
   sandboxed subprocess and stays the source of truth for every repository fact.
3. **Hold authority without weakening prikk's cryptography** (PU-4, SEC-1). planeter decides who may
   read, push and merge; every write still lands as prikk objects that prikk's own offline `verify`
   accepts. *The forge holds no history-signing key* (owner-ruled 2026-09-15): a compromised planeter
   can deny history but can never forge it — the property a prikk forge exists to offer over a Git one.
4. **Clean, safe, secure and robust before rich** (PU-5, owner's standing design philosophy). Lean on
   standards, keep each feature's trust surface small, defer the frontier, and prefer refusing a
   capability to shipping it half-safe.

## 3. What "done" means

**1.0 is the familiar forge, complete**: hosting, transport, change review and one-click merge (the
maintainer seals with their own key, the forge adopts the trusted fast-forward — prikk RFC 154), issues,
CI on isolated ephemeral runners, a package registry, web and API — hardened, backup-tested, with the
hosted-format durability policy settled. **Promotion to 1.0 is an explicit owner decision**; green gates
and feature-completeness are necessary, not sufficient. Until then everything is 0.x, additive where
possible, honest about what is not there.

Each milestone is usable on its own: M1 (host + browse + auth, shipped 0.1.0) → M2 (clone + push,
keyless) → M3 (review + issues + merge) → M4 (CI) → M5 (packages) → 1.0. Track B (the canonical branch
by adoption, and hosted-format durability) is joint work with prikk, gated on prikk shipping RFC 155 and
RFC 154, and ships within 0.x when it can.

## 4. What planeter will not do (the non-goals, as firm as the goals)

Not a Git server (Git only by import and mirror); not a change to prikk (a needed capability is a
named prikk-side dependency, never a prikk patch planeter writes); not a holder of authority it cannot
map onto prikk (forge-approved and prikk-verified stay distinct on every page); not the frontier by
default (federation, portable identity, built-in AI are post-1.0 per forge-commons); not a reinvention
of standards; not a second source of truth; not a lock-in (export plus unconditional access to the
prikk repositories). Requirements NG-1…NG-7.

## 5. Constraints planeter accepts

- **Linux only.** The sandbox is bubblewrap; planeter makes no platform claim it cannot run (prikk RFC
  107's lesson). Runtime prerequisites are prikk at the pinned floor, bubblewrap, and curl for SSO.
- **prikk moves fast and its JSON is pre-1.0.** planeter pins a floor, re-baselines at every prikk
  release, and treats schema drift as a version-gated risk (dependency ledger PK-18).
- **The write path depends on prikk's schedule.** Keyless fetch and the canonical branch need prikk RFC
  155 and 154 (after prikk 0.49.0). planeter designs to the accepted direction and waits for the binary
  rather than inventing a prikk wire protocol.
- **Supply chain is reviewed per crate**: measured growth, allow-listed licenses, no unscoped advisory
  ignores, a preference for confined subprocesses over in-process protocol stacks
  (`docs/dependency-policy.md`).
- **Security is release-gating.** A release touching auth, transport, the prikk boundary, CI, rendered
  content or the signing model updates the threat model (`planeter-03`, a first-class deliverable).

## 6. Who decides what

- **The owner** (nabbisen) decides direction, priorities, scope, milestones, the owner-only questions
  (requirements §10 OQ-), every tag, publish and release, letters to the prikk team, and the 1.0
  promotion.
- **The architect** (a high-capability AI agent) writes requirements, design, RFCs, handoffs, schedule
  and records; reviews every implementation; drafts upstream letters; recommends releases.
- **The dev team** (a mid-capability AI agent) implements from handoffs and returns evidence.
- **The prikk team** is upstream: its rulings and schedule are recorded in the dependency ledger; asks
  go by letter through the owner (`docs/UPSTREAM.md`).

The workflow, the RFC lifecycle and the handoff conventions are in `CONTRIBUTING.md`,
`rfcs/done/000-rfc-lifecycle-policy.md` and `rfcs/handoffs/README.md`.

## 7. History to date

| Date | Event |
|---|---|
| 2026-09-06 | prikk's owner names hosting as a separate project's job |
| 2026-09-15 | Design set v0.1 (requirements, external design, threat model, internal design); RFC 001–009 accepted with handoffs; OQ-1 ruled: no forge-held signing key |
| 2026-09-16 | A0 foundations begin; prikk RFC 154 and RFC 155 accepted upstream (the keyless multi-maintainer question answered) |
| 2026-09-22 | prikk 0.46.0 ships the read verbs co-designed with planeter (`tree`, `cat`, `diff`); floor set to 0.46.0 |
| 2026-09-23 | **0.1.0** (M1: host + browse + auth) and **0.1.1** (browse UI, sign-in, egress guard) released; the 0.46.0 transport measurement puts M2 on hold; owner rules proxy-terminated TLS + host OpenSSH |
| 2026-09-24 | **0.2.0** (trusted proxies, per-IP throttle, OpenID Connect SSO) released after a full documentation audit; the team migrates to the two-agent workflow |

## 8. Where to read next

`docs/STATUS.md` (where things stand) → `ROADMAP.md` (milestones, release cycles, dependencies) →
`docs/SCHEDULE.md` (sequence, windows, prospects, risks) → `docs/src/planeter-01…04` (the design set)
→ `rfcs/` (the decisions) → `docs/src/planeter-prikk-dependency-ledger.md` (what prikk really does).
