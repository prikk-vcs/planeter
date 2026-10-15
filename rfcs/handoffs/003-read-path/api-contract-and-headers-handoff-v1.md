# RFC 003 follow-on — API contract and response headers — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 003 (D-2: the read API is OpenAPI-described; STD-3: one error shape, `ETag` conditional requests) and RFC 003 D-4/D-6 (private content must not leak). It corrects four gaps found by the architect's audit of the shipped API (issues IS-28, IS-29). |
| Status | **Assigned** — included in release 0.3.0 by the owner's decision D-12 (2026-10-15). |
| Basis | [`../../done/003-read-path.md`](../../done/003-read-path.md) D-2, D-4, D-6; requirements STD-3; forge-commons *HTTP API* and *Transport Security and the Web*; `crates/planeter-web/src/api.rs` (`error_response`, `etag_of`, `openapi_doc`), `security.rs` (`app_security_headers`); the private audit `.git-exclude/reviewed/ui-api-audit-v1.md` (A1–A5). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

An API client today receives a bare status with an empty body on every error, an OpenAPI document that
lists paths and one-line summaries only, validators that depend on a compiler's hash implementation, and
responses with no cache directives, so a private repository's pages may be stored by a browser or a
caching proxy. 0.3.0 is the cheapest minor in which to fix the contract: no client depends on it yet.

## Change scope

- **T1 — One error shape.** Every non-2xx/304 response of the read API carries
  `application/json` `{ "code": "<stable snake_case>", "message": "<one plain sentence>", "hint":
  "<what the caller can do next>" }`. Codes: `not_found` (404; **identical for hidden and absent**),
  `unavailable` (503 when prikk cannot be read), `busy` (503 from the gate, with `Retry-After`),
  `invalid_request` (400/422: a malformed `path`, `ref` or `limit`), `internal` (500). Messages and hints are
  written once in one table (they will be catalogued for translation later); they never include a path,
  a repository's existence, or prikk output. The UI's pages are unchanged.
- **T2 — A real OpenAPI document.** Hand-built (no new crate): for each route its parameters (name, `in`,
  required, schema, default and bound for `limit`), its 200 response schema (from the view types), 304, the
  error responses with the T1 schema, `security` (a bearer token, and the session cookie), and
  `info.version = env!("CARGO_PKG_VERSION")`. Add a test that **every route registered in the API router
  appears in the document and vice versa**, and that a sample of each view serializes to the declared
  required fields.
- **T3 — Stable validators.** Replace `DefaultHasher` in `etag_of` by a hash whose output is specified
  (SHA-256, truncated to 128 bits, hex, quoted strong validator). **Use a hash already in the dependency
  tree** (via a re-export or an existing helper); if that needs a manifest change, **stop and report** with
  the delta (dependency-policy rule 1).
- **T4 — Cache and browser-policy headers.** Every dynamic response (UI and API, including errors) carries
  `Cache-Control: private, no-cache` (revalidate, never shared) and `Vary: Cookie, Authorization`; every
  response that sets or clears a cookie, and the login and logout responses, carry `Cache-Control: no-store`;
  the single stylesheet carries `Cache-Control: public, max-age=3600`. Add
  `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()`. Add
  `Strict-Transport-Security: max-age=15552000` **only when `PLANETER_TRUSTED_PROXIES` is configured** (the
  production shape: the app is declared to sit behind TLS), no `includeSubDomains`, no `preload`.
- **T5 — Tests** under `crates/planeter-web/tests/`: the error body for each code, hidden and absent
  repositories byte-identical; the OpenAPI drift guard; the ETag is unchanged across two runs and across a
  re-ordered JSON object; every header above on a UI page, an API response, an error, a login response and the
  stylesheet; HSTS present only with trusted proxies.

## Non-change scope

No route, query parameter, status code or success body changes; no authorization change; no new dependency
without stopping; no UI wording changes (a separate RFC owns the web UI's vocabulary and language); no
rate-limit headers or `Link` pagination (T12).

## Required documentation updates

`docs/RELEASING.md` (the HSTS note and that the app sets cache directives), CHANGELOG `[Unreleased]`
(**the error bodies are now JSON** and the caching change are operator-visible), the OpenAPI document is its
own reference; issue register IS-28 and IS-29 closed.

## Acceptance criteria

T5's tests pass; the OpenAPI document and the router cannot drift (a deliberately removed route fails the
test); a private repository's response has `private, no-cache` and `Vary`; the validators are stable; all
gates green including both supply-chain gates, both test runs named; ENF-2.

## Prohibited shortcuts

Do not put `Cache-Control: public` on any dynamic response; do not distinguish a hidden repository from an
absent one in code, message, hint, status or header; do not generate the OpenAPI document by adding a crate
without stopping; do not leak prikk's raw error text into `message` or `hint`.

## Security and compatibility constraints

`no-cache` keeps conditional GETs working (a 304 still saves the body); `no-store` is only for credentials
flows. Error bodies are new; a client that ignored bodies is unaffected.

## Known risks

An operator's proxy that strips or overrides `Cache-Control` (document: planeter's directives are a
default, the proxy's policy wins); an over-long `hint` table becoming untranslatable prose (keep one
sentence).

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the T5 test names; one captured response per
error code and one OpenAPI excerpt; the ETag stability evidence; deviations and open questions.
