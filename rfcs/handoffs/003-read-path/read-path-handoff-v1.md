# RFC 003 — Read path & web browse — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 003 (Read path). Task/PR plan + QA checklist. |
| Status | Inherited from RFC 003 — **Accepted**. |
| Basis | [`../../done/003-read-path.md`](../../done/003-read-path.md) (decisions D-1…D-8); RFC 001 (prikk driver, hosting), RFC 002 (`authorize(_, read, _)`); `STD-3/6`, `INT-4/5`, `INV-6`; threat model `T-6/T-7/T-10`. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | **Phase A1, read slice — reaching M1 with A0 + RFC 002.** The browse UI + read API (GET only). **Out of scope:** any prikk write verb, transport (RFC 004), the change/review model (RFC 005). Read-only. |

## Task breakdown (PR plan — build in this order)

- **T1 — Read model + cache (D-1/D-3).** In `planeter-core`: resolve repo → `.prikk` → `planeter-prikk`
  read → typed view. Cache results in `planeter-store` **keyed by the prikk object id** they derive from;
  a repo write (later) invalidates the touched refs' entries. **The cache is never the authority** — it
  must be droppable and rebuildable from prikk (prove it in T-QA). On an unavailable prikk subprocess,
  serve last-known cache **labelled stale**; never fabricate.
- **T2 — Browse surfaces (D-2).** Map each to its prikk `--format json` verb and build the view model:
  history (`log`), a change's content/effect (`show`), a file at a ref (`checkout --patch-plan
  --content-path`), refs/branches/tags (`branch`/`tag`), verify status (`verify`). Bounded/paginated
  `log`; single-file content; a bounded number of prikk calls per page (D-8).
- **T3 — Read API (D-2, `STD-3`).** Expose the same as **GET** endpoints, **OpenAPI-described**, with
  `Link`-header pagination (bounded default page size), `ETag`/`If-None-Match` → `304`, and rate-limit
  headers. Establish these conventions here; later RFCs extend the API.
- **T4 — Safe rendering — the security core (D-4, `STD-6`).** Route all Markdown/HTML through a **strict
  sanitizer** (strip scripts, event handlers, `javascript:`/`data:` script URLs) before it reaches a
  page. Serve **raw repository bytes** (raw file view, downloads, avatars, hosted content) from an
  **isolated content origin** (a distinct domain from the app). Author a **strict CSP** with the UI (no
  inline script, tight sources) plus `nosniff`, `frame-ancestors`, `Referrer-Policy`. Non-negotiable and
  built in now.
- **T5 — Honesty display (D-5, `WEB-06`/`SEC-1`).** Every history/change view shows prikk's `verify`
  status and marks **prikk-verified** vs **forge-approved** distinctly; unsigned/unverifiable/approved-
  but-unsealed reads as exactly that. The **security verify status is re-derived, not served from a
  possibly-stale cache** (coordinate with T1's stale-labelling).
- **T6 — Read authorization (D-6).** Every read (UI and API) calls `authorize(principal, read, resource)`
  (RFC 002 T4). A denied private resource is **indistinguishable from nonexistent** (`T-7`). No
  unauthenticated back door to private content; UI and API share the one gate.
- **T7 — Gap catalogue (D-7, `UD-1`).** Where a view needs data prikk doesn't expose in machine-readable
  form (raw blob bytes beyond `show`/`checkout` reports; blame/annotate), render **"pending a prikk
  increment"** — never reach into prikk internals (`LAY-3`). Produce the catalogue of gaps as the
  prikk-side ask list.

## QA checklist (from RFC 003 acceptance)

- [ ] Browse works over a real hosted repo: history, change view, file-at-ref, refs/branches/tags,
      verify status — each re-derived from prikk (T2).
- [ ] **Cache-cleared correctness:** dropping the derived cache and rebuilding from prikk yields
      identical views (`ENF-5`); a write invalidates affected entries.
- [ ] **Rendering security (tested):** a repo file with `<script>` / an event handler / a `javascript:`
      URL is neutralized; raw bytes served only from the isolated content origin; CSP + `nosniff` +
      `frame-ancestors` present.
- [ ] **Authorization (tested):** anonymous denied `internal`/`private`; denied-private ≡ nonexistent;
      UI and API enforce the same gate.
- [ ] **Honesty (tested):** an approved-but-unsealed change never renders as prikk-verified; the security
      verify status is not served stale.
- [ ] Read API OpenAPI-described with pagination/ETag/rate-limit (T3); gates green; threat model
      re-verified for `T-6/T-7/T-10`.

## Definition of done & handback

Done when the checklist is green — at which point **A0 + RFC 002 + RFC 003 = M1 (0.1.0)**: a real,
hosted, browsable, access-controlled, honest forge. Return the review-request package naming the PRs, the
web framework/sanitizer/isolated-origin choices (T4), and the gap catalogue (T7). The architect then
writes RFC 004 (transport) to open clone/push for M2. All v0.x; 1.0 is the owner gate.
