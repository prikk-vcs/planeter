# planeter — Roadmap

How planeter gets from a design set to a real, familiar forge for prikk. This is a **direction with
milestones**, not a dated schedule: work ships when it is correct, tested, secure, and honest.
Requirement and design ids (e.g. `CAP-2`, `WR-5`, `INV-2`, `LAY-2`) refer to the design set in
[`docs/src/`](docs/src/) — `planeter-01` requirements, `-02` external design, `-03` threat model,
`-04` internal design. The commons frame is [forge-commons](https://github.com/kos-commons/forge-commons);
the upstream is [prikk](https://github.com/prikk-vcs/prikk).

## The wide perspective (read this first)

**planeter is a familiar forge — and that is a large build.** Host repositories people clone and push,
propose and review changes, track issues, run CI, publish packages, manage teams and permissions, over
a web UI and an API. The ambition is Forgejo-shaped, even if leaner; the roadmap is honest that reaching
1.0 is a multi-milestone journey, not a single release.

**One structural fact shapes the whole plan:** planeter builds on **prikk's stable CLI surface**, not
its internals (RFC 145 Shape D; the stikk precedent). So **almost all of planeter can be built now,
without changing prikk** — prikk is already a working single-repo VCS with a machine-readable read
surface and artifact-based exchange. Only a small, well-named set of things need *joint* prikk work, and
they are decoupled so they never block the bulk.

**Two tracks, deliberately decoupled** (the brygge pattern):

- **Track A — the forge over prikk-as-is.** The bulk of planeter. Depends only on prikk's existing CLI
  (`log`/`show`/`verify`/… JSON) and artifact exchange (`bundle`/`sync`). Buildable today, milestone by
  milestone, to a usable increment at each step.
- **Track B — the joint prikk work.** A short list of prikk-side capabilities planeter needs
  (one-click client-side merge, hosted-format durability). Each is a joint planeter+prikk RFC, owner-
  gated, advancing in parallel; Track A ships with a keyless fallback until each lands.

**The arc, at a glance** — each milestone is usable; 1.0 is *familiar-complete*:

```
 A0 foundations → A1 host+browse+auth → A2 clone+push (keyless) → A3 review+issues+merge
      → A4 CI → A5 packages ─────────────────────────────────────────────► 1.0 familiar forge
                                                                              │
 Track B (parallel, gated): B1 one-click merge (UD-6) · B2 format durability ─┘
 Later (deferred, per forge-commons verdicts): federation · AI (passive) · portable identity
```

## Guiding rules (constant across the roadmap)

- **Design before implementation** (project rules). Requirements → external → threat model → internal
  design → **RFC + handoff** → implementation → tests. Never inverted.
- **Carry the hosting weight; keep prikk lean** (PU-2/BN-4). Nothing planeter needs pushes weight back
  into prikk's five-dependency, offline core. If a capability cannot be built without changing prikk's
  posture, it is a Track-B joint RFC or it is deferred.
- **The forge signs nothing, by construction** (INV-2/ENF-2). The default build contains no
  forge-held-key seal path; a push `accept`s author-signed patches with no key, and the merge seal is
  the maintainer's own (OQ-1 a). No milestone ships a default build that can forge prikk-verified
  history.
- **Lean on the standards; defer the frontier** (STD-*/DEF). Federation, portable identity, and
  built-in AI are post-1.0, per forge-commons' *pilot/defer* verdicts. A familiar forge does not need
  them.
- **Authorization is a single core service** (LAY-2/INV-4), consulted by every surface, from the first
  milestone that has a surface.
- **Security is release-gating** (threat model). A release touching auth, transport, the prikk boundary,
  CI, rendered content, or the signing model **updates** `planeter-03`; others **re-verify** it.

---

## Track A — the forge over prikk-as-is

### Phase A0 — Foundations
The substrate every surface sits on; no network yet. **CR-prikk** (drive the prikk CLI as a sandboxed
subprocess, typed JSON — INT-1/PKI-*), **CR-store** (forge-metadata persistence, DB-abstracted),
**CR-core** with the `authorize()` service (LAY-2), **CR-auth** primitives, and the **layering gate**
(LAY-1, `boundary-check`-style). Delivers nothing user-facing; makes everything above it safe.

### Phase A1 — Host + browse + auth (the read/host spine)
Multi-repo hosting and forge-level identity (owner/name/URL above anonymous prikk repos — DM-1);
the **read path** (browse history/changes/contents/verify, re-derived from prikk — RD-*, safe rendering
STD-6); **authentication** (OAuth/OIDC, tokens, SSH keys) and **per-repo/per-ref authorization** (AZ).
**Outcome: a real, hosted, browsable, access-controlled forge.** Buildable with no prikk change.

### Phase A2 — Clone + push (keyless)
The **transport ferry** (TX): fetch/clone (server builds, client accepts — WR-1) and **push** (client
builds, server `accept`s author-signed patches — WR-2/WR-3), plus the **client helper** (TX-05).
**No maintainer key touches the forge.** Outcome: a live host you clone from and push to.

### Phase A3 — Review + issues + merge
The **change/review model** (WR-4): an open change *is* prikk's accepted-but-unsealed claim set, with
review metadata, inline discussion, approvals; **issues** and cross-references (CAP-4); and **merge via
the keyless fallback** (WR-5b: maintainer's own prikk seals, forge stores). Outcome: the full
collaboration loop.

### Phase A4 — CI
Pipelines and **isolated, ephemeral, separate-host runners** (CIO-*, INV-5): triggers on push/change/
schedule, secrets withheld from untrusted changes, least-privilege job tokens.

### Phase A5 — Packages
The **registry** (RG): OCI Distribution first, language formats by demand; scoped publish, immutable
versions, object-storage-backed.

## Track B — joint prikk work (gated, parallel, never blocks Track A)

### Phase B1 — One-click merge (UD-6)
The client-sealable-claim affordance (WR-5a): let a maintainer's client seal a *server-held* accepted
claim in one action, so "merge" is one click and the forge still holds no key. A **joint planeter+prikk
RFC** (needs a small prikk-side capability). Until it lands, A3's keyless fallback (WR-5b) is the merge.

### Phase B2 — Hosted-format durability (OQ-6 / UD-3)
prikk's on-disk format is unstable pre-1.0; planeter pins supported prikk format versions and refuses
the unsupported (OP-04). B2 settles the **durability policy**: version pinning, migration, and the
stability gate before hosting a given prikk version. Owner-ruled; required before 1.0.

## Later — deferred frontier (post-1.0)
Per forge-commons verdicts, and only when their triggers are met: **federation** (pilot ForgeFed
read/announce; defer cross-instance writes), **AI** (passive conventions only — no autonomous authority,
content-as-data, no default egress), **portable identity** (defer until a standard interoperates).

---

## Milestones & versions

| Milestone | Version | Contents | Track | Status |
|---|---|---|---|---|
| **M0** | 0.1.0-dev | Foundations: CR-prikk driver, store, core + `authorize`, auth, layering gate | A0 | planned |
| **M1** | **0.1.0** | **Host + browse + auth**: multi-repo hosting/identity, read path, sign-in, per-ref authz. The first usable hosted forge | A1 | planned |
| **M2** | 0.2.0 | **Clone + push (keyless)**: transport ferry, fetch, push=accept+verify, client helper | A2 | planned |
| **M3** | 0.3.0 | **Review + issues + merge**: change/review model, issues, merge (keyless fallback WR-5b) | A3 | planned |
| **M4** | 0.4.0 | **CI**: pipelines + isolated ephemeral runners | A4 | planned |
| **M5** | 0.5.0 | **Packages**: OCI + first language registries | A5 | planned |
| **B1** | ships within 0.x once UD-6 lands | **One-click merge** (client-sealable claim) | B1 | gated (joint prikk RFC) |
| **B2** | before 1.0 | **Hosted-format durability policy** | B2 | gated (owner OQ-6) |
| **1.0.0** | 1.0 | **The familiar forge, complete**: host + transport + review + one-click merge + CI + packages + web + API, hardened, backup-tested, durability policy settled | A + B | pending A0–A5, B1, B2 **+ owner confirmation** |

> **v0 → v1 is an owner gate.** All development proceeds in **v0.x**. Promoting to **1.0 requires the
> owner's clear, explicit confirmation** — green gates and feature-completeness are necessary but **not
> sufficient**. Until then, the format, APIs, and surfaces are treated as pre-1.0 (unstable, additive
> where possible), and no milestone silently crosses into 1.0.

## Release cycles

- **Milestone-driven minors.** Each phase completes a minor. A minor ships only when its gates are green
  (fmt · clippy `-D warnings` · test · supply-chain: `cargo-deny`/`cargo-audit`), its security invariants
  hold (notably **ENF-2**: the default build links no forge-seal path — CI-checked), and the threat model
  is updated if the release touched a sensitive surface.
- **The read/host spine ships early and often.** M1 is a genuinely usable product (host + browse + auth);
  subsequent minors add capability without a big-bang.
- **Track B ships when ready, within 0.x.** B1/B2 are not release blockers for their neighbours; they
  land when their joint prikk work and owner rulings complete.
- **Security releases are out-of-band.** A dependency advisory or a threat-model control failure triggers
  a prompt patch release.
- **Tags are bare versions (no `v`)**, gates CI-enforced, release mechanics mirroring the ecosystem;
  **publishing/tagging is owner-only.**

## Dependencies (on prikk and on the owner) — do not block Track A

Named so no plan silently assumes them:

- **UD-6** — client-sealable-claim affordance (one-click merge, B1). Joint prikk RFC.
- **UD-3 / OQ-6** — prikk format stability + the hosted-format durability policy (B2). Owner-ruled.
- **OQ-2** — whether per-ref authorization anchors to a prikk-side notion or is purely planeter's (shapes
  AZ-2/WR-5). Owner-ruled.
- **OQ-4** — the exact v1 feature ceiling beyond the CAP core (wikis, discussions, boards). Owner-ruled.
- **OQ-7 / IQ-1** — reuse stikk's `stikk-prikk` layer for CR-prikk, or build independently. Dev-team
  decision, architect recommendation to evaluate reuse behind a trait.
- **UD-4 / IQ-3** — prikk local-locking sufficiency beneath planeter's per-repo serialization. Confirmed
  in the transport RFC.

*OQ-1 is settled (2026-09-15, option a): the forge holds no history-signing key by default.*

---

## The RFC plan (priority and order)

Detailed per-subsystem design lives in [`rfcs/`](rfcs/) (5-folder lifecycle, per project RFC 000);
this is their **priority and order**, each mapped to the phase it serves. The architect writes each RFC
+ handoff; the dev team implements against it; the owner authorizes. **RFC 001 first** — nothing else
starts until the foundations and the layering gate exist.

| # | RFC (planned) | Serves | Priority |
|---|---|---|---|
| **001** | Foundations: prikk-integration layer (CR-prikk), the layering gate (LAY-*), multi-repo hosting & identity (DM-1) | A0→A1 | **first** |
| **002** | Authorization model — the `authorize()` contract, roles, per-ref permissions (AZ-2) | A1 | high |
| **003** | Read path & web browse — re-derivation, caching, safe rendering (RD-*) | A1 | high |
| **004** | Transport envelope + client helper — fetch/push over ferried artifacts (WR-1/2, TX-05) | A2 | high |
| **005** | Change/review model — accepted-unsealed-as-PR, review, keyless merge (WR-4/5b) | A3 | medium |
| **006** | CI + runner protocol (CIO-*) | A4 | medium |
| **007** | Package registry (RG-*) | A5 | medium |
| **008** | *(joint prikk)* Seal / UD-6 — one-click client-sealable merge (WR-5a) | B1 | parallel, gated |
| **009** | Hosted-format durability policy (OQ-6/UD-3) | B2 | before 1.0 |

**Immediate next step:** stand up planeter's `rfcs/` (done alongside this roadmap — see
[`rfcs/README.md`](rfcs/README.md)), then draft **RFC 001** on the owner's go-ahead.

## Method & roles (project rules §Workflow)

The **architect** (High-Capability Model) writes the schedule, the RFCs and their handoffs, and manages
release cycles. The **dev team** (Mid-Capability Model) implements against the handoffs and returns
review packages. The **owner** authorizes releases, tags, and the owner-only decisions (the OQs above).
Both architect and dev team are authorized to commit and push. Entry points are surfaced to the owner as
file paths, so the owner manages at low cost: this roadmap, and [`rfcs/README.md`](rfcs/README.md).
