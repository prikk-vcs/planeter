# RFC 003 — Read path & web browse: re-derive from prikk, render safely, show the truth

**Status.** Proposed (2026-09-15) — the read half of M1. Draft for review; on acceptance the implementer
builds the browse surfaces and the read API against it per the handoff. Defines *how planeter shows a
repository* — always re-derived from prikk, cached carefully, rendered safely, and honest about what
prikk verified versus what the forge merely approved.
Handoff: forthcoming (`../handoffs/003-read-path/`).
**Tracks.** ROADMAP Phase A1 → M1 (Track A). Requirements `CAP-8`, `STD-3/6`, `INT-4/5`, `NG-6`; external
design `WEB-01/05/06`, `API-01…03`, `RD-1…3`; internal design `RD-1…3`, `DM-3`, `ENF-5`; threat model
`T-6/T-7/T-10`, `INV-6`, `C-6`.
**Touches.** `planeter-web` (the read UI + read API/GET endpoints, rendering, the isolated content
origin) and `planeter-core` (the read model + cache), over `planeter-prikk` (reads) and RFC 002's
`authorize()` (the read gate). **Not here:** write/transport (RFC 004), the change/review model
(RFC 005), CI/registry. This RFC is read-only; it never invokes a prikk write verb.

## Summary

A forge's read surface is where most requests land and where most of its XSS and confidentiality risk
lives, because it turns attacker-controllable repository content into rendered pages. This RFC fixes
three things so they are right from the start: **(1)** every repository fact is **re-derived from prikk**
and cached, never treated as authoritative in the forge database (`INT-4`/`INV-6`); **(2)** content is
**rendered safely** — sanitized, with raw bytes on an isolated origin under a strict CSP (`STD-6`); and
**(3)** the UI **tells the truth about trust** — prikk-verified and forge-approved are shown as distinct
facts (`WEB-06`/`SEC-1`). The read path is the first user-visible product (M1): host a prikk repo, sign
in, browse it — with access control (RFC 002) and honesty built in.

## The constraints that scope this design

- **prikk is the source of truth; the forge store is not** (`INV-6`/`NG-6`). Any fact shown must be
  reconstructible by re-invoking prikk over the hosted `.prikk`; a corrupted forge cache must never be
  able to present false history (`T-10`).
- **The prikk boundary is a subprocess with per-call latency** (RFC 001 D-2). The read path must cache to
  be usable, but the cache is a performance aid, never the authority.
- **Rendered content is untrusted** (`T-6`): READMEs, file contents, issue text, filenames — all
  attacker-controllable. XSS here is account takeover.
- **Some reads prikk does not yet expose in machine-readable form** (`UD-1`): raw blob bytes are reached
  only via `show`/`checkout` content reports (blob access is `pub(crate)` in prikk, not a CLI surface),
  and there is no blame/annotate. A view needing these is *pending a prikk increment*, not faked
  (`INT-5`/`PK-6`).

## Decisions

- **D-1 — Re-derive, then cache; prikk is the authority.** A read resolves repo → `.prikk` (RFC 001 D-4)
  → the relevant `planeter-prikk` command (D-2) → typed JSON → view/serialization. Results are cached in
  `planeter-store` **keyed by the prikk object id** they derive from; because prikk ids are
  content-addressed, a cache entry is valid until the object changes. A write (RFC 004) invalidates the
  affected refs' entries. The cache can always be dropped and rebuilt from prikk (tested — `ENF-5`).
- **D-2 — The browse surfaces and their prikk sources.** Each surface maps to a prikk read verb
  (`--format json`):
  - **history / log** → `log`; **a change's content and effect** → `show`; **a file's content at a ref**
    → `checkout --patch-plan --content-path`; **refs / branches / tags** → `branch`, `tag`; **repository
    verification status** → `verify`; **worktree/queued state** (where relevant) → `worktree-status`.
  - The read **API** exposes the same as GET endpoints, described in **OpenAPI** with `Link`-header
    pagination, `ETag`/`If-None-Match` conditional responses, and rate-limit headers (`STD-3`). This RFC
    establishes those conventions for reads; later RFCs extend the API for their surfaces.
- **D-3 — Caching, invalidation, and honest degradation.** Cache keys are prikk object ids (D-1); a repo
  write invalidates the touched refs' derived entries. If the prikk subprocess is unavailable for a repo,
  reads serve the last-known cache **labelled stale** and never fabricate (`OP-06`); a security-relevant
  status (D-5) is **not** served from cache when stale — it is re-derived or withheld.
- **D-4 — Safe rendering is the security core (STD-6).** Markdown/HTML is passed through a **strict
  sanitizer** (scripts, event handlers, dangerous URL schemes stripped) before it reaches a page. **Raw
  repository bytes** — a raw file view, downloads, avatars, any hosted content — are served from an
  **isolated content origin** (a distinct domain from the app), so a malicious file executes, at worst,
  in a throwaway origin with no session cookie and no access to the forge DOM. A **strict CSP** is
  authored with the UI (no inline script, tight source lists), plus `nosniff`, `frame-ancestors`,
  `Referrer-Policy`. These are non-optional and designed in now, not retrofitted.
- **D-5 — The honesty display (WEB-06/SEC-1).** Every history/change view carries prikk's **`verify`
  status** and distinguishes **prikk-verified** (a maintainer's seal that prikk accepts) from
  **forge-approved** (a planeter review decision) — never conflating them. An unsigned, unverifiable, or
  approved-but-unsealed object reads as exactly that. The verify status shown for a security display is
  **re-derived, not trusted from a possibly-stale cache** (D-3). This is the read-side of "the forge
  cannot manufacture verification" (`T-1`/`INV-1`).
- **D-6 — Reads are authorized (RFC 002 D-5).** Every read calls `authorize(principal, read, resource)`:
  a `public` repo is readable by Anonymous; `internal`/`private` require the appropriate grant; a denied
  private resource is indistinguishable from a nonexistent one (`T-7`). The read API and the UI share the
  one gate; there is no unauthenticated back door to private content.
- **D-7 — Read-surface gaps are shown, not faked (UD-1/INT-5).** A view needing data prikk does not
  expose in machine-readable form today — raw blob bytes beyond `show`/`checkout` reports, a blame/
  annotate — is rendered as **"pending a prikk increment"**, never synthesized by reaching into prikk's
  internals (`LAY-3` forbids it). These gaps are catalogued in the handoff and become prikk-side asks.
- **D-8 — Performance posture.** The subprocess cost is absorbed by D-1's cache and by keeping reads
  scoped (paginated `log`, single-file content, bounded page sizes — `STD-3`); a browse page issues a
  bounded number of prikk calls. Connection/process reuse for `planeter-prikk` is an optimization the
  handoff may pursue, but correctness never depends on it.

## What "done" means (acceptance criteria)

- Browse works over a real hosted repo: history, a change view, file content at a ref, refs/branches/
  tags, and verify status — each re-derived from prikk (D-2).
- **Cache-cleared correctness:** dropping the derived cache and rebuilding from prikk yields identical
  views (`ENF-5`); a repo write invalidates the affected entries (D-3).
- **Rendering security (tested):** a repository file containing `<script>`/an event handler/a `javascript:`
  URL is neutralized in the rendered page; raw file bytes are served only from the isolated content
  origin; the CSP, `nosniff`, and `frame-ancestors` headers are present (D-4).
- **Authorization (tested):** anonymous is denied `internal`/`private`; a denied private repo is
  indistinguishable from nonexistent; the read API and UI enforce the same gate (D-6).
- **Honesty (tested):** an approved-but-unsealed change never renders as prikk-verified; the security
  verify status is not served from a stale cache (D-5).
- The read API is OpenAPI-described with pagination/ETag/rate-limit conventions (D-2); threat model
  re-verified for `T-6`/`T-7`/`T-10`.

## Alternatives considered

- **Treat the forge database as the authoritative read model (index history into the DB).** Rejected:
  it breaks `INV-6` — a DB compromise could then present false history (`T-10`) — and duplicates what
  prikk already is. Re-derive + cache keeps prikk the single truth.
- **Render raw content from the app origin with sanitization alone.** Rejected: sanitization has bugs;
  the isolated content origin is defense-in-depth that turns an escaped payload into a harmless one
  (`STD-6`). Both, not either.
- **Fill read-surface gaps by linking `prikk-store` for blob access.** Rejected (`LAY-3`, RFC 001): the
  crate API is unstable and this would breach the process boundary; gaps are named and become prikk CLI
  increments (D-7/`UD-1`).

## Open questions & dependencies

- **UD-1 (prikk)** — the read-surface completeness gaps catalogued in D-7 (raw blob access as a CLI
  surface; a blame/annotate equivalent) become prikk-side asks; M1 ships the views prikk supports today
  and marks the rest pending. Does not block M1's core browse.
- **OQ-4 (owner)** — the exact browse feature edge (e.g. rendered diffs style, file-tree depth) may
  refine D-2; the core surfaces here are sufficient for M1.
- The web framework / templating choice is `planeter-web` implementation detail, settled in the handoff.

## Sequencing & handoff

Builds on RFC 001 (the prikk driver, hosting model) and RFC 002 (the read gate). On acceptance, the
architect writes `handoffs/003-read-path/` (the surface→prikk-command map, the cache/invalidation design,
the sanitizer + isolated-origin + CSP specifics, the honesty-display rules, the read-API/OpenAPI
conventions, the gap catalogue, and the tests above). With A0 + RFC 002 + this, planeter reaches **M1
(0.1.0)** — a real, hosted, browsable, access-controlled, honest forge. Then RFC 004 (transport) opens
clone/push. All v0.x; 1.0 is the owner gate.
