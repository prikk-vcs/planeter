# RFC 002 — Authorization — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 002 (Authorization). Task/PR plan + QA checklist. |
| Status | Inherited from RFC 002 — **Accepted**. |
| Basis | [`../../accepted/002-authorization.md`](../../accepted/002-authorization.md) (decisions D-1…D-9); RFC 001 foundations (the `authorize()` seam, `planeter-auth`/`planeter-store`); `STD-2/3`, `SEC-3/5`; threat model `INV-4`, `T-2/T-3/T-7/T-12`. |
| Audience | Dev team. Return a review-request package (entry-point path) when green. |
| Scope | **Phase A1, authorization slice.** The identity/membership inputs and the `authorize()` decision. **Out of scope:** the read call sites (RFC 003) and write ones (RFC 004/005) — they *call* this; here, build the contract and prove it in isolation. Build on RFC 001's A0. |

## Task breakdown (PR plan — build in this order)

- **T1 — The decision core (D-1/D-3).** In `planeter-core`: the resource + action enums (repo:
  read/push/merge-seal/admin; ref: push/merge-seal; org: admin/create-repo; issue|change: comment/close/
  review) and `authorize(principal, action, resource) -> Decision` where `Decision = Allow |
  Deny{reason}`. **Pure and default-deny:** no matching grant → `Deny`; the function reads
  membership/permission state passed in, performs no I/O of its own. A `Deny` for a private resource is
  **indistinguishable** from a `Deny` for a nonexistent one (`T-7`).
- **T2 — Principals & authentication (D-2, `STD-2`).** In `planeter-auth`: the account model and the
  authentication that yields a **Principal** (`Anonymous | User | MachineAsUser{user, scope} | CiJob{scope}`).
  Wire **standard mechanisms** (no home-grown crypto): session sign-in via **OAuth 2.0 / OIDC**
  (Argon2id for any local password); **scoped, hashed access tokens** and **OAuth tokens** for machines;
  **SSH public-key** registration (used by RFC 004 transport) mapping a key → user; **deploy keys**
  scoped to one repo. `authorize()` sees only the resulting Principal. (SAML/LDAP and TOTP/WebAuthn are
  configured integrations / follow-ons — stub the seams, don't block M1 on them.)
- **T3 — Roles, teams, membership (D-4).** In `planeter-store`/`planeter-auth`: owners (user|org), org
  **teams**, per-repo roles (`read ⊂ write ⊂ maintain ⊂ admin`), org roles. Implement the
  **resolution**: a principal + resource → effective permission, from team membership + direct grants.
  Least-privilege default; elevation explicit.
- **T4 — Visibility as the read gate (D-5).** `authorize(_, read, repo)` resolves the repo's visibility
  (`public`/`internal`/`private`, stored by RFC 001) to a decision: public → Anonymous allowed; internal
  → any authenticated user; private → explicit grant. This is what RFC 003 calls.
- **T5 — Protected refs (D-6).** A per-ref protection state (required approvals, required checks, no
  force-replace, restricted merge-seal set) evaluated **inside** `authorize(_, push|merge-seal, ref)`, so
  a protected-ref violation is denied at the authorization layer, reachable by no surface. (The seal
  remains the maintainer's client-side signature — this only gates *whether* it may proceed.)
- **T6 — Tokens & scope (D-7).** Effective capability = `user-permissions ∩ credential-scope`, enforced
  per verb/endpoint. Tests: read-only token blocked from a write action; single-repo credential blocked
  from another repo; expired token denied.
- **T7 — Audit hooks (D-9).** Sensitive decisions (push, merge-seal, permission/admin changes) emit an
  off-box audit record (principal, action, resource, allow/deny). Wire the sink seam RFC 001 reserved.

## QA checklist (from RFC 002 acceptance)

- [ ] `authorize()` is pure and **default-deny**; no-grant → Deny; private-denied ≡ nonexistent-denied
      (`T-7`).
- [ ] Principal model, membership resolution, and scope intersection unit-tested, incl. read-only-token,
      cross-repo-credential, and anonymous-vs-internal/private cases.
- [ ] Protected-ref evaluation denies a force-replace and a merge lacking approvals/checks — **tested at
      the authorization layer, no surface involved**.
- [ ] A property test enumerates every `(action, resource)` and asserts default-deny for an ungranted
      principal.
- [ ] Audit records emitted for sensitive decisions.
- [ ] Gates green (fmt · clippy `-D` · test · deny · audit, `--locked`); `forbid(unsafe_code)` holds;
      threat model re-verified for `INV-4` (no second authorization path) and `INV-2` (no signing path
      added).

## Definition of done & handback

Done when the checklist is green and `authorize()` is the single decision point every future surface
will call. Return the review-request package naming the PRs, the authentication mechanisms wired (T2),
and any deviations. RFC 003 then consumes `authorize(_, read, _)`; RFC 004 consumes `push`/`merge-seal`.
All v0.x.
