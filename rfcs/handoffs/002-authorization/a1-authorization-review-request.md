# RFC 002 — review-request package (Authorization)

**Status: RFC 002 complete.** All seven tasks landed on `main`, every gate green (fmt · clippy `-D` ·
test · deny · audit · ENF-2), `forbid(unsafe_code)` holds, and no new external dependency was added.
`authorize()` is now the single, pure, default-deny decision every future surface calls.

## Commits (on `main`, `prikk-vcs/planeter`)

| Tasks | Commit | What landed |
|-------|--------|-------------|
| T1, T3–T7 | `8ea76e8` | The authorization **decision**: role/membership inputs (`planeter-store`), resolution + the pure `authorize()` + the audit seam (`planeter-core`). |
| T2 | `81d4992` | Principals & authentication (`planeter-auth`): account/credential model, authentication → `Principal`, hashing/OIDC **seams**. |

## What each task delivered

- **T1 — decision core.** `authorize(principal, action, resource, ctx) -> Decision`, pure and
  default-deny. `Action` is a small explicit set; `Resource` is org/repo/ref/item; `Decision` is
  `Allow | Deny(reason)` with an existence-neutral `NotAuthorized` for visibility/role denials (T-7).
- **T2 — principals & authentication.** `Anonymous | User | MachineAsUser{scope} | CiJob{scope}`.
  Password (Argon2id **seam**), OIDC (**seam**), scoped access tokens (stored hashed), SSH keys, and
  repo-scoped deploy keys, each resolving to a principal. See the mechanisms note below.
- **T3 — roles, teams, membership.** `Read ⊂ Write ⊂ Maintain ⊂ Admin` + org roles; `resolve_repo_role`
  takes the max privilege across personal ownership, org-admin, direct grants, and team grants.
- **T4 — visibility read gate.** `authorize(_, Read, repo)`: public → anyone (incl. anonymous),
  internal → any authenticated user, private → explicit grant only.
- **T5 — protected refs.** Evaluated inside `authorize(_, Push|MergeSeal, ref)`: no-force-replace,
  required approvals, required checks, restricted merge-seal set — denied at the authorization layer,
  reachable by no surface.
- **T6 — tokens & scope.** Effective capability = `user-permissions ∩ credential-scope`: a read-only
  token is blocked from writes, a single-repo credential from another repo, an expired credential
  entirely.
- **T7 — audit hooks.** `audit_if_sensitive` records push/merge-seal/admin decisions to an `AuditSink`
  (null + in-memory now; off-box shipper behind the trait). Records carry identity, never secrets.

## Authentication mechanisms wired (T2)

Per the owner's ruling (**model + seams now, vetted crypto later**), T2 shipped the identity/credential
**model** and the authentication **seams** with **zero new dependencies**:

- `PasswordHasher` / `TokenHasher` traits — the production **Argon2id** password hasher and
  constant-time token hasher install behind these. Only a clearly-marked `InsecureStubHasher`
  (non-cryptographic, tests only) ships now.
- `OidcVerifier` trait + an `UnconfiguredOidc` stub — the OAuth2/OIDC provider client is a configured
  integration behind this seam. SAML/LDAP and TOTP/WebAuthn are further seams, not M1 blockers.

**Deferred, needs a dependency decision before production:** the concrete Argon2id, constant-time
token-hash, OAuth2/OIDC, and SSH-key-parsing crates. Until they land behind the seams, **no real secret
should pass through `planeter-auth`.** This is the one open item from RFC 002 and it is a supply-chain
review, not a design gap — the seams are settled and caller-invisible when the crates arrive.

## QA checklist (RFC 002) — status

- [x] `authorize()` pure and default-deny; no-grant → Deny; private-denied ≡ nonexistent-denied (T-7).
- [x] Principal model, membership resolution, and scope intersection unit-tested, incl. read-only-token,
      cross-repo-credential, and anonymous-vs-internal/private.
- [x] Protected-ref evaluation denies a force-replace and a merge lacking approvals/checks — at the
      authorization layer, no surface involved.
- [x] A property test enumerates every `(action, resource)` and asserts default-deny for an ungranted
      principal.
- [x] Audit records emitted for sensitive decisions (D-9).
- [x] Gates green; `forbid(unsafe_code)`; `INV-4` (single decision path) and `INV-2` (no signing path)
      re-verified — `authorize()` never signs or seals.

## Deviations from the handoff (with reasons)

1. **T2 crypto deferred behind seams** (handoff named Argon2id/OAuth/SSH). Owner-ruled: introduce the
   model + seams dep-free now, land vetted crates after a supply-chain review, consistent with the
   ecosystem's tiny-audited-surface ethos. No interface changes when the crates arrive.
2. **Store backends remain in-memory** (as in A0): SQLite is a caller-invisible follow-on behind
   `RepositoryStore` / `MembershipStore` / `AccountStore` / `CredentialStore`.

## Handback

RFC 002 is done. Next is **RFC 003 (read path)**, which consumes `authorize(_, Read, _)` for its browse
gate (D-5), then **RFC 004 (transport)** for `push`/`merge-seal` (D-6/D-8) — together reaching
**M1 (0.1.0)**. The deferred auth-crypto dependency review should be scheduled before M1 ships to
production. All v0.x; 1.0 is the owner gate.
