# RFC 005 — Change/review — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 005 (Change proposal, review, and merge; issues). |
| Status | Inherited from RFC 005 — **Accepted**. |
| Basis | [`../../accepted/005-change-review.md`](../../accepted/005-change-review.md) (D-1…D-9); RFC 002 (authz), RFC 003 (re-derived diff), RFC 004 (accepted-pending claims); threat model `INV-1/2`, `T-1/T-10`. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | **Phase A3 → M3.** Change/review model, merge via the keyless fallback, issues. **Out of scope:** the one-click seal mechanism (RFC 008/UD-6 — leave the seam), CI execution (RFC 006 — wire the check-gate seam). |

## Task breakdown (PR plan)

- **T1 — The change model (D-1/D-2/D-7).** `planeter-core`: a `change` record
  `{change-id, repo, source claim-ids/proposed-ref, target ref, author, status, review-state}` keyed to
  prikk claim ids; opens from a push (RFC 004 accepted-pending targeting a protected/other ref) or
  explicitly. The diff is prikk's re-derived `show`/patch (RFC 003) — **not** a forge-computed diff.
  Lifecycle open → merged | closed; nothing silently discarded.
- **T2 — Review (D-3).** Inline comments, threads, approvals, requested-changes; review-state that feeds
  the protected-ref gate. Authorized via RFC 002 (`review`/`comment`).
- **T3 — Merge = seal, keyless fallback (D-4/D-6, INV-2).** The merge flow: verify preconditions (D-5),
  then the **maintainer's own prikk** produces the seal via the fallback (`WR-5b`: fetch the pending
  claim → `sync seal` locally → push the sealed block), which planeter stores and re-verifies against
  trusted maintainer keys. **The forge holds no key** (ENF-2 must still pass). **Leave a clean seam** so
  RFC 008's one-click mechanism drops in without changing the model. Surface prikk's `merge-evidence`/
  `merge-plan` as the preview and prikk's `merge` outcome faithfully — no Git rebase/squash imposed;
  where prikk refuses, show prikk's refusal.
- **T4 — Merge preconditions as authorization (D-5).** A merge proceeds only if `authorize(_, merge-seal,
  ref)` passes: required approvals, required checks (CI seam — wired when RFC 006 lands; M3 gates on
  approvals + protected-ref rules, checks optional-if-present), no force-replace. A merge lacking them is
  **denied at the authz layer** — the seal step is never offered. Test the denial with no surface bypass.
- **T5 — Issues (D-8).** Forge-owned tracker: issues, labels, milestones, assignees, comments;
  cross-references to changes and prikk objects **by prikk id**. Authorized via RFC 002. Shares the
  review comment/notification machinery.
- **T6 — Honesty (D-9, INV-1).** Open change = **forge-approved, unsealed** (never prikk-verified); a
  merged change shows **approval** and **prikk-verification** as distinct facts, the latter re-derived
  from `verify` (RFC 003 D-5). A forge approval is never rendered as or convertible into a prikk
  signature.

## QA checklist (from RFC 005 acceptance)

- [ ] A change opens from a push and explicitly; its diff is prikk's re-derived `show`/patch.
- [ ] Review works; review-state drives the protected-ref merge gate.
- [ ] **Merge via the keyless fallback:** the target ref advances by a seal from the **maintainer's own
      key**; planeter stores + re-verifies; **ENF-2 still passes**. A merge lacking approvals/checks is
      **denied at the authorization layer** (tested).
- [ ] prikk's merge outcome (incl. refusals) is surfaced, not overridden.
- [ ] **Honesty (tested):** an open change never renders as prikk-verified; a merged change shows
      approval and prikk-verification distinctly (re-derived).
- [ ] Issues: create/label/assign/comment/close + cross-reference a change and a prikk object by id.
- [ ] Gates green; threat model re-verified (`INV-1`, `INV-2`); `planeter-03` updated if a data flow
      changed.

## Definition of done & handback

Done when the checklist is green — **M3 (0.3.0)**: the full collaboration loop, merge keyless. Return the
review-request package naming the PRs and the merge-flow seam left for RFC 008. The architect proceeds to
**RFC 008 (one-click seal / UD-6)** — the ergonomics upgrade to this merge — and then **RFC 006 (CI)**.
All v0.x; 1.0 is the owner gate.
