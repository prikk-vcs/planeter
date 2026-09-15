# RFC 008 — One-click merge (UD-6) — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 008. Covers **D-2 (UD-6a, CLI one-click)** — B1's buildable increment — plus the coordination items for D-3/D-4. |
| Status | Inherited from RFC 008 — **Accepted**. |
| Basis | [`../../accepted/008-seal-one-click-merge.md`](../../accepted/008-seal-one-click-merge.md) (D-1…D-6); RFC 004 (client helper, transport), RFC 005 (merge flow + seam); threat model `INV-2`, `RR-2`. |
| Audience | Dev team (for D-2) + architect/owner (for the D-4 companion prikk RFC and the D-6 scope call). |
| Scope | **Build D-2 now** (keyless CLI one-click). **D-3 (web one-click)** and **D-4 (prikk-side ask)** are coordination/design items, not code in this increment. |

## Task breakdown

- **T1 — `planeter merge <change>` (D-2).** Extend the client helper (RFC 004 `TX-05`) with a single
  command that orchestrates the keyless seal: fetch the change's accepted claims → `sync accept` into the
  maintainer's (or an ephemeral) local prikk → `sync seal <target-ref> --claim` (the **maintainer's own
  key**) → push the sealed block; planeter stores + re-verifies + advances the ref. Uses **existing prikk
  verbs only** — no prikk change.
- **T2 — Wire it into RFC 005's merge flow (D-1).** Replace the guided fallback's manual steps with this
  one command at the seam RFC 005 left; the **model is unchanged** (merge = maintainer's seal). The web
  UI's "merge" for now guides the maintainer to this client command, pending D-3.
- **T3 — Local seal context (D-2).** Decide and implement where the client seals: the maintainer's
  existing clone, or an ephemeral working copy the helper creates and discards. Protect the key
  client-side; the helper never transmits it.
- **T4 — Verify + keyless guarantee (INV-2).** planeter re-verifies the returned sealed block against
  trusted maintainer keys and advances the ref **only if it verifies**; an absent/invalid seal fails the
  merge, never a forge substitute. **ENF-2 must still pass** (no forge-seal path in the default build).
- **T5 — Raise the companion prikk RFC (D-4)** — *architect/owner action, not code.* Take to the prikk
  team/owner: (i) confirm the seal-locally-and-push semantics T1 relies on; (ii) whether prikk adds a
  portable **sealable-claim** affordance to make D-3 lighter. Track as a Track-B dependency (UD-6);
  **T1 does not wait on it.**
- **T6 — D-3 (web one-click) — design-only, pending owner D-6.** Do **not** build a browser signer in
  this increment. Record the client-signer/local-agent design sketch for the owner's D-6 scope ruling.

## QA checklist

- [ ] `planeter merge <change>` merges keyless-for-the-forge end-to-end; the sealed ref-advance verifies;
      **ENF-2 still passes**.
- [ ] The merge model is unchanged (RFC 005): a merge lacking approvals/checks is still denied at the
      authorization layer; the seal is still the maintainer's own key.
- [ ] An invalid/absent seal fails the merge (no forge substitute).
- [ ] Gates green; `planeter-03` re-verified (`INV-2` holds; if T3's ephemeral-clone path adds a flow,
      note it — the seal surface is client-side).
- [ ] The **companion prikk RFC (T5) is raised** with the owner/prikk team; the D-6 sketch (T6) recorded.

## Definition of done & handback

Done when `planeter merge` gives a maintainer one-command, keyless merge and RFC 005's flow uses it.
Return the review-request package naming the PRs, the local-seal-context choice (T3), and the status of
the companion prikk RFC (T5) and the D-6 sketch (T6). This is **B1** (ships within 0.x). All v0.x; 1.0 is
the owner gate.
