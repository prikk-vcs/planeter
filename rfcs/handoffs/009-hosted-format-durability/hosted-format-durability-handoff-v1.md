# RFC 009 — Hosted-format durability — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 009 (Hosted-format durability). |
| Status | Inherited from RFC 009 — **Accepted** (owner accepted the migrate-forward policy). |
| Basis | [`../../accepted/009-hosted-format-durability.md`](../../accepted/009-hosted-format-durability.md) (D-1…D-6); RFC 001 (version pin D-3, `prikk_format_version` D-5); `OPS-3` (backup), `CON-1` (serialization). |
| Audience | Dev team + architect/owner (for the prikk migration dependency and prikk-1.0 coordination). |
| Scope | **Phase B2 (Track B), required before 1.0.** Does not block Track A milestones. |

## Task breakdown (PR plan)

- **T1 — Supported format-version range (D-1).** `planeter-prikk` declares the supported prikk
  format-version range; startup and per-repo checks **refuse** an out-of-range repo with a reason.
- **T2 — Per-repo format tracking + surfacing (D-1/D-5).** Record `prikk_format_version` per repo
  (RFC 001 D-5); surface it and the planeter version's supported window; flag a repo that would fall out
  of support **before** an upgrade, never strand it after.
- **T3 — The adoption gate (D-3).** Encode as release/CI tooling: a prikk version-bump for hosting is
  **blocked** unless (i) the new format is in the supported range, (ii) a migration path exists (T4), and
  (iii) a backup + rehearsed restore (`OPS-3`) is in place. Test the gate fails when any is missing.
- **T4 — Migration orchestration (D-4).** On adopting a newer prikk, migrate each hosted repo by
  **driving prikk's own migration** (the named prikk dependency), **per-repo, serialized (`CON-1`),
  backup-first**, and **verify** after (re-derive/verify). A failed migration leaves the repo on its
  prior good state — never half-migrated. If prikk offers no in-place migration for a step, fall back to
  holding on the prior version or a re-import path (both explicit).
- **T5 — Raise the prikk migration dependency (D-4)** — *architect/owner action.* Take to the prikk
  team/owner: does prikk offer an in-place format migration planeter can drive? Coordinate prikk-1.0
  timing (D-6), to which planeter's 1.0 durability promise is coupled. Track as a Track-B dependency.

## QA checklist (from RFC 009 acceptance)

- [ ] planeter refuses an out-of-range repo with a reason; records/surfaces `prikk_format_version` (T1/T2).
- [ ] **Adoption gate (tested):** a hosting prikk version-bump is blocked until support + migration path +
      backup/restore rehearsal are present.
- [ ] **Migration (tested):** per-repo, serialized, backup-first, verifiable; a failed migration leaves
      the prior good state, never half-migrated.
- [ ] The **prikk migration dependency is raised** with the owner/prikk team; prikk-1.0 coupling noted.
- [ ] Threat model re-verified: `RR-4` is now a managed risk with a stated policy.

## Definition of done & handback

Done when the gate + migration are in place and the prikk dependency is raised — **the durability
prerequisite for 1.0**. With this, the RFC pipeline (001–009) is fully accepted and handed off. Return
the review-request package naming the PRs and the status of the prikk migration dependency. planeter
stays **v0.x**; 1.0 remains an explicit owner gate, coupled to prikk's format stabilizing.
