# RFC 003 follow-on — API contract refinements — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 003 (D-2: the read API is OpenAPI-described; STD-3: one error shape). It closes four contract gaps found when the architect reviewed the API-contract (IS-28), raw-origin (IS-16) and read-view code together: **IS-33**. |
| Status | **Assigned** — included in release 0.3.0 by the owner's decision D-14 (2026-10-17). |
| Basis | [`../../done/003-read-path.md`](../../done/003-read-path.md) D-2, D-6; requirements STD-3; the architect's reviews `.git-exclude/reviewed/api-contract-and-headers-review-v1.md` (follow-ups 1–5) and `.git-exclude/reviewed/raw-origin-gating-review-v1.md` (follow-up 2); `crates/planeter-web/src/{errors,openapi,api}.rs`, `crates/planeter-core/src/read.rs` (`FileContentView`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

The API now has one error shape and a real OpenAPI document. Four places still say something other than
what happens, or say it in a shape a client will not expect. 0.3.0 is the cheapest minor to fix them: no
client depends on the contract yet, and the error bodies already change in this release.

## Change scope

- **T1 — A method error says so.** Add the code `method_not_allowed` (405, message "That method is not
  supported here.", hint "Use GET.") and send `Allow: GET`. Add it to the OpenAPI error enum and to the
  routes' responses. `invalid_request` keeps 400.
- **T2 — A missing file is a 404.** `GET …/file` for a path that is not in a readable repository answers
  `404 not_found` (same body as any other), never `200` with a "not found" content value. Remove
  `NotFound` from `FileContentView`'s serialized forms (the view type may keep an internal marker). The UI
  page for a missing file keeps its current wording (a UI change is out of scope).
- **T3 — Content is a tagged object a client expects.** `FileContentView` serializes as
  `{ "kind": "text", "text": "…" }`, `{ "kind": "binary", "blob_id": "…", "size": N }`,
  `{ "kind": "opaque", "size": N }`. Update the OpenAPI schema (a `oneOf` with a `kind` discriminator), the
  UI's use of the view, and the drift test. **This is a success-body change** (the reason for D-14).
- **T4 — Raw bytes say where they live.**
  - On a host other than the content origin's, the raw route answers `404` with a new code
    `raw_elsewhere` (message "Raw file bytes are not served from this address.", hint "Use the link on the
    file's page, or the content origin named in the OpenAPI document."). The gate stays first, before any
    lookup, so the response still depends on nothing about any repository.
  - The OpenAPI document gets a path-level `servers` entry on the raw operation naming the configured
    content origin (OpenAPI 3.0.3 allows `servers` on a path item), and its description says raw bytes are
    served only there.
- **T5 — ETag cannot be computed from a failed serialization.** A view that fails `to_value` is an
  `internal` error, not an ETag over `null`.
- **T6 — Tests:** a 405 has the code and `Allow`; a missing file is 404 and a present one is 200; each
  `kind` serializes as documented (extend the drift guard); the app host's raw answer is `raw_elsewhere`
  and carries nothing about whether the repository exists (identical for a hidden, an absent and a public
  repository); the document's `servers` entry equals the configured content origin.

## Non-change scope

No route, parameter, authorization or header-policy change; no UI wording change; no change to who is
refused (the raw gate's decision rule is untouched, only its answer's code); no new dependency.

## Required documentation updates

CHANGELOG `[Unreleased]` (**file responses change shape; a missing file is 404; two new error codes**),
RELEASING (what an API client of the raw route should do), STATUS IS-33 closed.

## Acceptance criteria

T6's tests pass; the OpenAPI drift guard still passes and covers the new codes and the `kind` union; all
gates green including both supply-chain gates, both test runs named; ENF-2.

## Prohibited shortcuts

Do not make `raw_elsewhere` conditional on the repository, the path or the principal; do not leave a `200`
that says "not found"; do not keep both the old and the new content shapes; do not add the content origin
to any response other than the OpenAPI document and the existing UI links.

## Known risks

The UI renders from the same view type (T3): a template that matches on the old shape must be found by
compiling, not by reading. A client written against 0.2.0's `{"Text": …}` breaks: the CHANGELOG says so, the
reason being that 0.3.0 already changes every error body.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the T6 test names; one captured response per
new code and one OpenAPI excerpt (`servers`, the `kind` union); deviations and open questions.
