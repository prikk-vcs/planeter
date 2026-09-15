# RFC 005 — Change proposal, review, and merge (and issues)

**Status.** Proposed (2026-09-15) — the collaboration loop, M3. Draft for review; on acceptance the
implementer builds the change/review model, the merge flow, and the issue tracker per the handoff.
Defines *how a proposed change is reviewed and merged* — where a "pull request" is prikk's own
accepted-but-unsealed claim set, and a "merge" is a maintainer's own seal, never the forge's.
Handoff: forthcoming (`../handoffs/005-change-review/`).
**Tracks.** ROADMAP Phase A3 → M3 (Track A). Requirements `CAP-5`, `CAP-4`, `SEC-1`, `NG-3/6`; external
design `WEB-02/03/06`, `FL-04/05`; internal design `WR-4/5`, `DM-2/4`, `RD-3`; threat model `T-1/T-10`,
`INV-1/2`. Consumes RFC 002 (`authorize` merge-seal/review), RFC 003 (re-derived diff), RFC 004
(accepted-pending claims).
**Touches.** `planeter-core` (the change/review + issue model, the merge flow) and `planeter-web` (the
review and issue UI + API). **Not here:** the **one-click seal mechanism** (RFC 008/UD-6 — this RFC
uses the keyless fallback and is written to accept either), **CI execution** (RFC 006 — merge *gates on*
its checks), and packages (RFC 007).

## Summary

RFC 004 leaves pushed work as **accepted, verified, pending** claims (author-signed, ingested, not
sealed onto a ref). This RFC turns that state into the familiar collaboration loop: **an open change is
those pending claims plus forge review metadata; a merge is the maintainer sealing them onto the target
ref with their own key.** The non-negotiable, from `SEC-1`/`INV-1`/`INV-2`: **the forge holds no signing
key and seals nothing** — it presents the change (re-derived from prikk, RFC 003), gates the merge at the
authorization layer (RFC 002), and the *maintainer's own prikk* produces the seal. "Approved on planeter"
and "sealed & prikk-verified" stay distinct at every step (`WEB-06`). Issues (a forge-owned tracker) ride
alongside, cross-referencing changes and prikk objects by prikk id.

## The constraints that scope this design

- **A change is not a forge invention — it is prikk's accepted-unsealed state** (`DM-2`, RFC 004 D-4):
  the patches live in prikk; the forge holds only the review wrapper, keyed to prikk claim ids.
- **A merge is a seal, and the seal is the maintainer's** (OQ-1 a, `INV-2`): the forge never signs.
  Today's keyless mechanism is fetch-seal-push locally (`WR-5b`); the one-click upgrade is RFC 008 (UD-6).
- **prikk's merge model is its own, not Git's** (survey: `merge-evidence`/`merge-plan`/`merge`, block-
  oriented patch theory): planeter surfaces **prikk's** merge, it does not impose Git rebase/squash
  semantics on a VCS that does not share them.
- **Honesty is a security property** (`INV-1`, `T-1`): an open (unsealed) change must never read as
  prikk-verified, and a forge-approval must never look like a prikk signature.

## Decisions

- **D-1 — A change is accepted-pending claims + review metadata.** A **change** record:
  `{ change-id, repo, source (the pending claim ids / proposed ref), target ref, author, status ∈
  {open, merged, closed}, review state }`, stored forge-side keyed to prikk **claim ids** (`DM-2`). The
  patches themselves stay in prikk (RFC 004); the forge never copies history (`NG-6`).
- **D-2 — Opening a change.** A change opens from a push whose accepted-pending claims target a protected
  or non-writable ref (RFC 004 D-4), or explicitly against a target ref. Its "diff" is **prikk's own
  view** — `show`/patch content re-derived via RFC 003 — not a forge-computed diff.
- **D-3 — Review.** Inline comments, threaded discussion, **approvals** and **requested-changes**, and
  the required-approval / required-check state. Review metadata is forge-owned and authorized via
  RFC 002 (`review`, `comment` actions). Approvals and checks feed the protected-ref rules that gate the
  merge (D-5).
- **D-4 — Merge = seal, by the maintainer's own key (WR-5, INV-2).** Merging seals the change's accepted
  claims onto the target ref. The forge orchestrates and stores/verifies; **the seal is produced
  client-side by the merging maintainer's own prikk** (OQ-1 a). Two mechanisms, and the flow accepts
  either:
  - **M3 default — the keyless fallback (`WR-5b`):** the maintainer's prikk fetches the pending claim
    (RFC 004), `sync seal`s it locally, and pushes the sealed block, which planeter stores and re-verifies.
    Works today, no prikk change; it is a guided flow, not yet one click.
  - **The one-click upgrade (`WR-5a`, RFC 008/UD-6):** planeter hands the maintainer's client a sealable
    package; their prikk seals it in one action and returns the sealed block. Ships within 0.x when UD-6
    lands; **the merge flow here is written so adopting it changes the mechanism, not the model.**
  - **Never:** a forge-held key sealing on the maintainer's behalf, except the explicit per-repo opt-in
    (option b) behind the off-by-default feature (RFC 001 `LAY-4`, threat model `RR-2`).
- **D-5 — Merge preconditions are authorization outcomes (RFC 002 D-6).** A merge proceeds only if the
  protected-ref rules pass: required approvals present, required checks green (CI — RFC 006; the
  check-gate seam is ready here and wired when 006 lands), no force-replace. A merge lacking them is
  **denied at the authorization layer**, reachable by no surface — the seal step is never even offered.
- **D-6 — Merge uses prikk's merge, surfaced honestly.** planeter shows `merge-evidence`/`merge-plan`
  (read-only) as the **merge preview** and performs the merge via prikk's `merge` + seal. Where prikk's
  merge refuses or flags (its patch-theory semantics), planeter **surfaces prikk's outcome** rather than
  forcing a resolution — "where prikk refuses, planeter explains", the forge-side echo of stikk. No
  Git-style rebase/squash is imposed.
- **D-7 — Change lifecycle.** `open` → `merged` (D-4 sealed) or `closed` (without merge; may be
  superseded by a new change). A merged change records the resulting sealed block's prikk id; a closed
  one keeps its review record. Nothing is silently discarded.
- **D-8 — Issues (CAP-4), the companion forge metadata.** A forge-owned issue tracker: issues with
  labels, milestones, assignees, threaded comments, and **cross-references** to changes and prikk objects
  by prikk id (`DM-4`). Purely forge metadata (no prikk coupling beyond the cross-ref ids), authorized via
  RFC 002 (`comment`, `close`). Standard surface; it shares the review UI's comment/notification
  machinery.
- **D-9 — Honesty at every step (WEB-06/SEC-1/INV-1).** An open change is shown as **forge-approved,
  unsealed** — never prikk-verified. A merged change shows **both** "approved by … on planeter" **and**
  "sealed & prikk-verified" as distinct facts, the second re-derived from prikk's `verify` (RFC 003 D-5).
  A forge approval is never rendered as, or convertible into, a prikk signature (`T-1`).

## What "done" means (acceptance criteria)

- A change opens from a push and explicitly (D-1/D-2); its diff is prikk's re-derived `show`/patch view.
- Review works: comments, approvals, requested-changes; review state drives the protected-ref gate (D-3/D-5).
- **Merge (D-4) via the keyless fallback:** a maintainer merges; the target ref advances by a seal
  produced **by the maintainer's own key**; planeter stores and re-verifies it; **the forge holds no key
  (ENF-2 still passes)**. A merge lacking required approvals/checks is **denied at the authorization
  layer** (tested, no surface bypass).
- **prikk's merge is surfaced, not overridden:** a prikk merge refusal/flag is shown as prikk's outcome,
  not resolved by planeter (D-6).
- **Honesty (tested):** an open change never renders as prikk-verified; a merged change shows approval
  and prikk-verification as **distinct** facts, the latter re-derived (D-9, `INV-1`).
- Issues: create/label/assign/comment/close, cross-reference a change and a prikk object by id (D-8).
- Threat model re-verified: `INV-1` (no manufactured verification), `INV-2` (no forge seal path added);
  `planeter-03` updated if a new data flow was introduced.

## Alternatives considered

- **Copy the change's patches into the forge DB as the authoritative PR state.** Rejected (`NG-6`/`INV-6`):
  the patches stay in prikk (accepted-pending); the forge holds only the review wrapper keyed to prikk
  ids, so a DB compromise cannot forge history (`T-10`).
- **A forge-side merge button that seals with a forge key (default).** Rejected (`INV-2`): the default
  merge is the maintainer's own seal (D-4); forge-side signing is the explicit per-repo opt-in only.
- **Impose Git-style rebase/squash/merge-commit strategies.** Rejected (D-6): prikk has its own
  patch-theory merge; planeter surfaces it faithfully rather than pretending prikk is Git.
- **Block M3 on the one-click seal (UD-6).** Rejected: the keyless fallback (`WR-5b`) delivers merge now;
  RFC 008 upgrades the *mechanism* without changing the model (D-4).

## Open questions & dependencies

- **RFC 008 / UD-6 (joint prikk)** — the one-click client-sealable claim; upgrades D-4's mechanism.
  Not blocking M3 (the fallback ships).
- **RFC 006 (CI)** — required-check gating (D-5); the seam is ready, wired when CI lands. M3 gates on
  approvals + protected-ref rules with checks optional-if-present.
- **OQ-2 (owner)** — if per-ref authority ever anchors to a prikk-side notion, D-5's protection source
  shifts; the merge flow is unaffected.
- **OQ-4 (owner)** — the review feature edge (suggested changes, review threads model, draft changes)
  may refine D-3; the core loop here is sufficient for M3.

## Sequencing & handoff

Builds on RFC 002 (merge-seal/review authorization), RFC 003 (re-derived diff, honesty display), and
RFC 004 (accepted-pending claims). On acceptance, the architect writes `handoffs/005-change-review/` (the
change/issue data model keyed to prikk ids, the review-state → protected-ref-gate wiring, the merge flow
with the keyless-fallback steps and the UD-6-ready seam, the prikk-merge surfacing, and the tests above).
Delivering this reaches **M3 (0.3.0)** — the full collaboration loop. Then RFC 006 (CI) adds automation
for M4. All v0.x; 1.0 is the owner gate.
