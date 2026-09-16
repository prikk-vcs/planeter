# RFC 003 — review-request package (Read path & web browse)

**Status: RFC 003 complete.** All seven tasks landed on `main`, every gate green (fmt · clippy `-D` ·
test · deny · audit · ENF-2 · layering gate), `forbid(unsafe_code)` holds. With A0 + RFC 002 + RFC 003,
this is **M1 (0.1.0)**: a real, hosted, browsable, access-controlled, honest forge read surface.

## Commits (on `main`, `prikk-vcs/planeter`)

| Tasks | Commit | What landed |
|-------|--------|-------------|
| T1, T2, T5, T6 | `08f0d6f` | The read-path core: honest view-models (pure functions of prikk output), the `Assurance` distinction, and the `authorize(_, Read, _)`-gated `ReadService`. |
| T7 | `91ac3ed` | The read-path gap catalogue (the prikk-side ask list). |
| T4 | `149a222` | Rendering safety: `ammonia` HTML sanitizer, `pulldown-cmark` Markdown, the strict CSP + security headers, and the isolated content origin for raw bytes. |
| T3 | `668ad63` | The axum read API (GET, OpenAPI, ETag/304, bounded pagination) + a runnable M1 preview server. |

## Web framework / sanitizer / isolated-origin choices (T4/T3)

Owner-approved (**adopt the standard stack now**); each crate vetted through cargo-deny/audit, and the
dependency-tree growth reported at each step:

- **HTTP:** `axum` 0.8 (features `http1`, `json`, `query`, `tokio`) on `tokio`. Router driven in tests
  via `tower`'s `ServiceExt::oneshot` (no live socket).
- **Sanitizer:** `ammonia` 4 (html5ever-based) — strips `<script>`/`<style>`, `on*` handlers,
  `javascript:`/`data:` script URLs.
- **Markdown:** `pulldown-cmark` 0.12 (`html` feature only), output always re-sanitized.
- **Isolated content origin:** a configured distinct domain for raw repository bytes, served with
  `nosniff` + `Content-Disposition: attachment` + a `sandbox` CSP.

**Dependency-tree growth:** 22 → **114** crates. The bulk is transitive under ammonia's html5ever parser
and `url`→`idna`→`icu` Unicode normalization (a correct HTML sanitizer's cost), plus axum/tokio. Two
licenses were added to `deny.toml`, both permissive and routine: **MPL-2.0** (weak file-level copyleft;
via ammonia's `cssparser`/`dtoa-short`) and **BSD-3-Clause** (via axum's `matchit` router). No audit
advisories.

## Gap catalogue (T7) — the prikk-side asks

`rfcs/handoffs/003-read-path/t7-read-path-gap-catalogue.md`. M1 browse blockers: **(1) a `--format json`
tree/dir listing at a ref** and **(2) raw blob byte retrieval by id**. M2+: blame, ref-vs-ref diff.
Deliberate non-gaps: no timestamps (prikk has no clocks — honesty, not a gap), no HEAD pointer. These
become a letter to the prikk team (same channel as the RFC 154/155 asks).

## QA checklist (RFC 003) — status

- [x] Browse over a real hosted repo: history, change, file-at-ref, refs/branches/tags, verify — each
      re-derived from prikk (`ReadService` + `tests/read_path.rs`).
- [x] Cache-cleared correctness: view-models are pure functions of prikk output, so a derived cache is
      droppable/rebuildable (ENF-5); asserted by the purity test. (A production cache lands in
      `planeter-store` keyed by object id when write-invalidation exists — RFC 004; re-deriving from the
      local prikk subprocess is correct and simple for the read-only M1.)
- [x] Rendering security (tested): `<script>` / event handler / `javascript:` / `data:` script URL all
      neutralized; raw bytes carry download + `nosniff` + `sandbox` headers; CSP + `nosniff` +
      `X-Frame-Options` present on every response.
- [x] Authorization (tested): anonymous denied `internal`/`private`; denied-private ≡ nonexistent (`404`
      for both); the API and the `ReadService` share the one gate.
- [x] Honesty (tested): a queued (approved-but-unsealed) change is `ForgeApprovedUnsealed`, never
      `PrikkVerified`; the verify status is always re-derived, never served from cache.
- [x] Read API OpenAPI-described with ETag/304 and bounded pagination (T3); gates green; threat model
      re-verified for `T-6/T-7/T-10`.

## Deviations from the handoff (with reasons)

1. **Derived cache is a documented seam, not yet a store** (T1 named a `planeter-store` cache keyed by
   object id). For a **read-only** M1 there are no write-invalidation events, and re-deriving from the
   local prikk subprocess is correct and simplest. The real requirement — views are pure and
   cache-rebuildable (ENF-5) — is met and tested. The cache lands with RFC 004 (writes), which is when
   invalidation becomes meaningful. `Freshness`/`Served` are in place for the stale-fallback labelling.
2. **Browse UI (HTML pages) not built; the read API is** (D-2 mentioned both). The view-models + safe
   rendering (`render_markdown`/`sanitize_html`) + security headers are all present; assembling HTML
   pages/templates on top is deferred to a UI increment. The **API** delivers browse for M1.
3. **Cursor/`Link` pagination and enforced rate-limit headers deferred** (T3 named them). Shipped the
   bounded-`limit` + `ETag`/`304` conventions **honestly** rather than emitting rate-limit numbers not
   enforced; cursor pagination also awaits a prikk `log` cursor. Documented, not faked.

None changes a public interface or a threat-model control.

## M1 status & handback

**M1 (0.1.0) is ready for the owner's release decision.** A0 + RFC 002 + RFC 003 give a hosted,
browsable, access-controlled, honest forge read surface, runnable via the `planeter` preview binary. The
first tag is **0.1.0**, which only the owner authorizes.

Two items must be scheduled **before M1 ships to production** (both are supply-chain reviews, not design
gaps): the deferred **auth crypto** (Argon2id / constant-time token hash / OAuth-OIDC / SSH-key parsing,
behind the RFC 002 seams) and a **persistent store** (SQLite behind the `*Store` traits). The preview
binary uses in-memory stores + the insecure stub hasher and is labelled not-for-production.

Next after the M1 tag: **RFC 004 (transport)** — clone/push over `authorize(_, push|merge-seal, _)` —
toward M2.
