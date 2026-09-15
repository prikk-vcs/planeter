# RFC 008 — One-click merge without a forge key (UD-6): the sealing ergonomics problem

**Status.** Accepted (2026-09-15) — **Track B, joint planeter + prikk, gated.** The planeter design is
settled; the implementer may build the D-2 (CLI one-click) increment against it per the handoff. Carries
a **prikk-side ask** (D-4) to be raised as a companion prikk RFC with the prikk project/owner, and a
web-one-click scope decision (D-6) for the owner. Upgrades the *mechanism* of RFC 005's merge without
changing its model.
Handoff: [`../handoffs/008-seal-one-click-merge/seal-one-click-merge-handoff-v1.md`](../handoffs/008-seal-one-click-merge/seal-one-click-merge-handoff-v1.md).
**Tracks.** ROADMAP Phase B1 (Track B — never blocks Track A; RFC 005's keyless fallback ships the merge
meanwhile). Requirements `UD-6`, OQ-1 (a); external design `GATED-1`, `TX-05`; internal design `WR-5a`;
threat model `INV-2`, `RR-2`.
**Touches.** The client helper (RFC 004 `TX-05`), `planeter-transport`/`planeter-web` (the merge action),
and a **companion prikk RFC** for any prikk-side affordance. **Not here:** the merge *model* (RFC 005,
unchanged), CI (RFC 006).

## Summary

Merging seals accepted claims onto a ref, and a seal needs the maintainer's Ed25519 key. Under OQ-1(a)
the **forge holds no key**, so the seal is the maintainer's own — and RFC 005 already delivers that via a
keyless fallback (fetch → seal locally → push). This RFC is about making it **one click without ever
putting a key in the forge**. Its central, honest finding: **the forge staying keyless is easy; the hard
part is the *browser → user-key* bridge**, because a web page cannot hold or use a maintainer's prikk
signing key. Mainstream forges dodge this by signing web-merges with the *forge's own* key — exactly the
option (b) planeter refuses as a default (`INV-2`/`RR-2`). So this RFC's job is to get as close to
one-click as possible **while keeping the key on the maintainer's side**, and to name precisely what (if
anything) prikk should add to help.

## The constraints that scope this design

- **The forge holds no signing key** (`INV-2`), except the explicit per-repo opt-in (b). This RFC exists
  to make (b) unnecessary for a good merge UX.
- **A web browser cannot produce a prikk seal.** A prikk seal is an Ed25519 signature over a block; a
  browser has no access to the maintainer's prikk key, and **WebAuthn/passkeys sign challenges, not
  arbitrary block data**, so they cannot substitute. A user-key web merge therefore needs a **client-side
  signer** (a local agent / the client tool holding the key), not browser crypto.
- **Sealing needs the accepted-claim state + the key together.** prikk's `sync seal <ref> --claim`
  operates where the accepted claim lives. Today that is the server (which has no key) or the
  maintainer's local clone (which has the key after a fetch). Bridging the two without a forge key is the
  problem.
- **prikk's exact sync/seal semantics are prikk's to state.** Whether a maintainer can seal locally and
  push the sealed ref-advance (the fallback assumes yes), and whether a compact "sealable claim" can be
  handed to a client and its sealed result re-ingested, are **questions for the prikk team** (D-4), not
  facts this RFC may assume.

## Decisions

- **D-1 — The property, stated as ergonomics not trust.** Goal: the maintainer merges in **one action**,
  the seal is produced by **their own key**, and the **forge holds no key** at any point. Everything
  below is about reducing the maintainer's steps, never about relocating the key to the forge.
- **D-2 — UD-6a — CLI one-click, buildable now (no prikk change).** The client helper (RFC 004 `TX-05`)
  orchestrates the fallback behind a single command: `planeter merge <change>` → fetch the change's
  claims → `sync accept` into the maintainer's (or an ephemeral) local prikk → `sync seal` (the
  maintainer's key) → push the sealed block; planeter stores + re-verifies + advances the ref. This makes
  merge **one command for a maintainer at a terminal**, keyless-for-the-forge, using only existing prikk
  verbs. **This is B1's first, committed increment.**
- **D-3 — UD-6b — web one-click via a client-side signer (the frontier).** For the browser "merge"
  button, the click triggers the maintainer's **local signer** — a small local agent, or the client tool
  invoked through a registered handler — which performs the seal (D-2's steps, or D-4's compact form) and
  returns the sealed block; planeter stores/verifies/advances. **The key never enters the browser or the
  forge.** This is the genuinely hard UX; it is designed here as a *client/agent* problem, and pursued
  incrementally. It is **not** required for merge to work (D-2 covers that) and its depth for v0 is an
  owner scope call (D-6).
- **D-4 — The prikk-side ask (the joint part): a portable *sealable claim*.** To make D-3 clean — so the
  client signs a **compact package** rather than reproducing a full fetch/accept/clone — planeter asks
  prikk to consider: (i) confirming that a maintainer may seal a claim locally and push the sealed
  ref-advance (the fallback's assumption); and (ii) optionally, a **portable sealable-claim** capability —
  export a claim's minimal sealable state from the server, seal it on a client, and re-ingest the sealed
  block. This is **prikk's/owner's to design** (a companion prikk RFC, the analogue of brygge's UD-
  dependencies); planeter **names the requirement and targets whatever prikk settles**. If prikk declines
  (ii), D-2/D-3 still work via the full fetch-based path — just heavier.
- **D-5 — The rejected shortcut, named so it stays rejected.** A forge-held key signing web-merges
  (option b) is what mainstream forges do and is the easy "green button." It is **not** planeter's default
  (`INV-2`); it remains only the explicit, per-repo, key-isolated, forge-attributed opt-in (`RR-2`). This
  entire RFC exists to make the *keyless* path good enough that (b) is a deliberate exception, not a
  convenience default.
- **D-6 — Scope ladder for v0 (owner call).** B1 delivers **D-2 (CLI one-click)** — keyless, buildable,
  no prikk change. **D-3/D-4 (web one-click + the prikk affordance)** are the frontier increment; the
  owner decides how far to push them for 1.0 versus leaving the web merge as "guided" (trigger the local
  signer with a couple of confirmations) until a clean client-signer exists. The forge is keyless at
  every rung.

## Security (INV-2, RR-2)

The seal is **always** the maintainer's own key; the forge only ever **stores and re-verifies** the
returned sealed block against trusted maintainer keys, and **advances the ref only if it verifies**. A
missing or invalid seal fails the merge — the forge never substitutes its own. The client-side signer (D-3)
must protect the key on the maintainer's side (its threat surface is the maintainer's machine, not the
forge). No rung of the ladder adds a forge-held signing path to the default build (ENF-2 holds
throughout).

## What "done" means

- **D-2 (UD-6a) end-to-end:** `planeter merge <change>` merges keyless-for-the-forge via the client
  helper; the sealed ref-advance verifies; **ENF-2 still passes**. RFC 005's merge flow adopts this as
  its mechanism with no model change.
- **D-3/D-4 specified, not necessarily built:** the client-signer design and the prikk-side ask are
  written and the **companion prikk RFC is raised**; whether D-3 ships for v0 is recorded per D-6.
- Threat model re-verified: `INV-2` holds at every rung; `RR-2` (the opt-in b) unchanged; if D-3's local
  agent introduces a new flow, `planeter-03` notes it (the agent's surface is client-side).

## Alternatives considered

- **Forge-key web merge as default (option b).** Rejected (`INV-2`/D-5): it is the mainstream shortcut and
  the thing a *prikk* forge exists to avoid; it stays an explicit opt-in.
- **A browser-held key (WebAuthn/passkey) signing the seal.** Rejected: passkeys sign challenges, not
  arbitrary block data (constraints §2); they cannot produce a prikk seal. A device-held prikk key via a
  local agent (D-3) is the viable path.
- **Require a full local clone for every merge (the pre-008 fallback only).** Not rejected — it *is* the
  fallback and D-2 automates it; D-4's portable sealable-claim would make it lighter if prikk provides it.

## Open questions & dependencies

- **Companion prikk RFC / owner (D-4)** — confirm the seal-locally-and-push semantics; decide whether
  prikk adds a portable sealable-claim affordance. planeter adapts either way; **B1 (D-2) does not wait on
  it.**
- **Owner (D-6)** — how far to push web one-click (D-3) for v0 vs. leaving it guided until a clean
  client-signer exists.
- **Client-signer design (D-3)** — the local agent / registered-handler mechanism and its client-side key
  protection; a handoff/companion design once D-6 is ruled.

## Sequencing & handoff

Track B, parallel to Track A; RFC 005's keyless fallback ships the merge regardless. On acceptance, the
architect (a) writes `handoffs/008-seal-one-click-merge/` for **D-2 (CLI one-click)** — the client-helper
orchestration and the tests — as B1's buildable increment, and (b) raises the **companion prikk RFC** for
D-4 and drafts the D-3 client-signer design for the owner's D-6 ruling. This upgrades the merge ergonomics
without ever giving the forge a key. All v0.x; 1.0 is the owner gate. Next in the main line: **RFC 006
(CI)**.
