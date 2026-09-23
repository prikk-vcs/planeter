# RFC 002 — Authorization model: principals, roles, and per-ref permissions

**Status.** Done — shipped in planeter 0.1.0 (2026-09-23; accepted 2026-09-15) — fills the `authorize()` seam RFC 001 established (D-6). The design is
settled and the implementer may build the authorization service and the identity/membership model
against it per the handoff. Defines *who a principal is*, *what they may do to which resource*, and *how
a forge decision maps onto prikk without ever becoming a prikk signature*.
Handoff: [`../handoffs/002-authorization/authorization-handoff-v1.md`](../handoffs/002-authorization/authorization-handoff-v1.md).
**Tracks.** ROADMAP Phase A1 → M1 (Track A). Requirements `CAP-3`, `SEC-1/3/5`, `STD-2/3`; external
design `AUTH-01…05`; internal design `AZ-1…4`, `LAY-2`; threat model `T-2/T-3`, `INV-4`, `C-2/C-3`.
**Touches.** `planeter-auth` (principals, membership, tokens/keys — the inputs to a decision) and
`planeter-core` (the `authorize()` service — the decision). Every upper-layer surface calls it. **Not
here:** the authentication *cryptography and flows* (OAuth/OIDC/SAML/LDAP mechanics are standard per
`STD-2` — implementation detail in the handoff, not redesigned); the read enforcement *call sites*
(RFC 003) and the write ones (RFC 004/005) — they consume this contract.

## Summary

RFC 001 made "authorization is consulted on every path" a *structural* property by placing a single
`authorize()` seam in `planeter-core` (`LAY-2`/`INV-4`). This RFC gives that seam its **contract**: the
principal model, the role/permission/membership model it resolves, the per-repository and **per-ref**
permissions a forge needs, and the mapping onto prikk. The one non-negotiable, from `SEC-1`: **planeter
authorization decides *whether* an operation may proceed; prikk decides *whether the result verifies*;
the two are never merged.** Authorizing a push does not sign anything — the patches are already
author-signed and prikk-verified on their own (RFC 004); authorizing a merge does not seal — the seal is
the maintainer's own signature (RFC 001 §OQ-1 a).

## The constraints that scope this design

- **prikk has no authorization model at all** — trust is *object* trust ("adopting a key never lets it
  move a ref"), no accounts, no ACLs (survey §6). So the entire access-control layer is planeter's, above
  prikk (`§0a`). Whether it *anchors* to any prikk-side notion is the owner's open **OQ-2**; the default
  here is "purely planeter's".
- **Default-deny is the only safe base** (`INV-4`, `T-3`): ambient authority and IDOR are the classic
  forge escalations.
- **Least privilege is a standard, not a nicety** (`SEC-3/5`): a token or key must carry the smallest
  capability that does its job; an unscoped credential in a CI file is the classic breach (`T-5`).
- **The decision must be pure and testable**: a forge with a sprawling, side-effecting authorization
  path cannot be audited. `authorize()` is a deterministic function of (principal, action, resource,
  the membership/permission state) with no I/O of its own beyond reading that state.

## Decisions

- **D-1 — The contract: `authorize(principal, action, resource) -> Decision`.** A single, pure,
  **default-deny** function in `planeter-core`. `Decision` is `Allow` or `Deny{reason}` (the reason is
  for the audit log and the API error, never a leak of resource existence — a private resource denies
  identically whether or not it exists, `T-7`). Every surface — web, API, transport — calls it before
  acting; there is no second authorization path (`LAY-2`).
- **D-2 — Principals.** Exactly one of: **Anonymous**; a **User** (a human account); a **Machine acting
  as a user** via a credential (a scoped personal access token, an OAuth token, or an SSH/deploy key) —
  carrying the *intersection* of the user's permissions and the credential's scope (D-7); or a **CI job**
  (an ephemeral, job-scoped principal minted by RFC 006, never a signing identity). Authentication (how a
  principal is established — token/session/key verification, OAuth/OIDC) is `planeter-auth`'s job per
  `STD-2`; its *output* is a principal, which is all `authorize()` sees.
- **D-3 — Resources and actions (a capability model).** Resources are addressable: an **org**, a
  **repository**, a **ref** (within a repo), a **change/review**, an **issue**. Actions are the verbs a
  forge grants: on a repo — `read`, `push`, `merge-seal`, `admin`; on a ref — `push` / `merge-seal`
  gated by protected-ref rules (D-6); on an org — `admin`, `create-repo`; on an issue/change —
  `comment`, `close`, `review`. The set is small and explicit; a new surface adds an action deliberately,
  never a wildcard.
- **D-4 — Roles and membership resolve to permissions.** An **owner** is a user or an org. An org has
  **teams**; a user's team memberships and direct grants resolve to **per-repository roles** —
  `read` ⊂ `write`(push) ⊂ `maintain`(merge-seal, ref protection) ⊂ `admin` — and to **org roles**
  (member, admin). `authorize()` resolves the principal's effective permission on the resource from this
  membership state (read from `planeter-auth`/`planeter-store`), then checks the action against it. Least
  privilege is the default; elevation is explicit and audited.
- **D-5 — Visibility is enforced here.** RFC 001 stored repository visibility (`public`/`internal`/
  `private`) as data; RFC 002 makes it a read gate: `read` on a `public` repo is allowed for Anonymous;
  `internal` requires any authenticated user; `private` requires an explicit grant. This is the gate
  RFC 003's browse path calls.
- **D-6 — Protected refs are authorization outcomes, not UI.** A ref may carry protection: required
  review approvals, required passing checks, no force-replace, and a restricted `merge-seal` set. These
  are evaluated inside `authorize(_, merge-seal|push, ref)` — so a push that would rewrite a protected
  ref, or a merge lacking approvals/checks, is *denied at the authorization layer*, reachable by no
  surface. (The seal itself remains the maintainer's client-side signature — D-8.)
- **D-7 — Tokens, keys, and scope.** A credential carries a **scope**; the principal's effective
  capability is `user-permissions ∩ credential-scope`, enforced **per endpoint/verb** (`STD-3`). Scoped,
  expiring **personal access tokens** and **OAuth tokens** for machines; **SSH keys** map to a user
  identity; a **deploy key** is scoped to a single repository. A read-only token cannot reach a write
  action; a single-repo credential cannot reach another repo — checked in `authorize()`, not assumed.
- **D-8 — The mapping onto prikk (SEC-1), and OQ-2.** `authorize()` gates *whether* `sync accept`
  (push) or the merge-seal *may proceed*; it never produces a prikk signature. A push lands author-signed
  patches prikk verifies on their own (RFC 004); a merge is sealed by the maintainer's own key (OQ-1 a).
  Authorization is therefore **purely planeter's, above prikk** — the current answer to **OQ-2** (whether
  to anchor per-ref authority to a prikk-side notion). The owner may later rule OQ-2 differently; the
  contract (D-1) is unaffected, only the *source* of a ref's protection state would change.
- **D-9 — Every decision is auditable.** `authorize()` outcomes on sensitive actions (push, merge-seal,
  permission and admin changes) are recorded to the off-box audit log (`SEC-5`, `T-12`) with principal,
  action, resource, and allow/deny — the record that answers "who did what".

## What "done" means (acceptance criteria)

- `authorize()` exists in `planeter-core`, pure and **default-deny**; a resource with no matching grant
  denies; a private resource denies identically whether or not it exists (`T-7`).
- The principal model (D-2), role/team/membership resolution (D-4), and scope intersection (D-7) are
  implemented and unit-tested, including: read-only token blocked from writes; single-repo credential
  blocked from another repo; anonymous blocked from `internal`/`private` (D-5).
- Protected-ref evaluation (D-6) denies a force-replace of a protected ref and a merge lacking required
  approvals/checks — **tested at the authorization layer, independent of any surface**.
- A property test enumerates every defined `(action, resource)` and asserts a default-deny for a
  principal with no grant.
- Audit records are emitted for sensitive decisions (D-9).
- Threat model re-verified: `INV-4` (default-deny, single path) is structurally satisfied; no signing
  path is introduced (`INV-2` untouched).

## Alternatives considered

- **Enforce authorization in each surface handler.** Rejected: that is exactly the drift `LAY-2`/`INV-4`
  forbid; a single missed handler is an escalation. One core service, called everywhere.
- **Anchor per-ref authority to a prikk-side concept now.** Deferred to the owner (OQ-2): prikk has no
  such concept, and inventing a prikk change to host authority would push weight into prikk (`BN-4`). The
  default is planeter-owned authority; the contract does not depend on the answer.
- **Rich ABAC/policy-language authorization.** Rejected for v0: a small, explicit capability model
  (D-3/D-4) is auditable and sufficient for a familiar forge; a policy engine is complexity the owner's
  philosophy leans against. Revisit only against a demonstrated need.

## Open questions & dependencies

- **OQ-2 (owner)** — whether per-ref authority anchors to a prikk-side notion or stays purely planeter's
  (D-8). Default: planeter-owned. Does not block A1.
- **OQ-4 (owner)** — the exact set of grantable resources/actions at the v1 edge (e.g. discussions,
  boards) may extend D-3; the core set here is sufficient for M1.
- Authentication mechanism choices (OAuth/OIDC provider set, session store) are `planeter-auth`
  implementation detail per `STD-2`, settled in the handoff — not this RFC.

## Sequencing & handoff

Proceeds on RFC 001's base (the `authorize()` seam, `planeter-auth`/`planeter-store`). On acceptance, the
architect writes `handoffs/002-authorization/` (the role/permission schema, the membership-resolution
algorithm, the token/scope model, the protected-ref evaluation, the audit hooks, and the tests above).
RFC 003 (read path) then calls `authorize(_, read, _)` for its browse gate (D-5); RFC 004 (transport)
calls it for `push`/`merge-seal` (D-6/D-8). Together they reach **M1 (0.1.0)**. All v0.x; 1.0 is the
owner gate.
