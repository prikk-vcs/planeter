# Changelog

Releases are bare version tags; each section is the release's notes (assembled into the GitHub
release by `release.yml`). Kept by hand, per release, so nothing here goes stale by templating.

## [Unreleased]

- **API contract refinements (IS-33). Breaking for any client of 0.2.0's file endpoint.**
  `GET …/file` now serializes `content` as an object tagged by `kind`: `{"kind":"text","text":…}`,
  `{"kind":"binary","blob_id":…,"size":…}`, `{"kind":"opaque","size":…}` (it was serde's
  `{"Text":…}` / `{"Binary":{…}}` / `"NotFound"`). **A path that is not in a readable repository is now
  `404 not_found`** (it was `200` with a "not found" content value). The raw route, asked on any host but
  the content origin's, answers `404` with the new code `raw_elsewhere` (it was the generic `not_found`,
  whose hint sent callers looking at the wrong thing); the OpenAPI document names the content origin in a
  `servers` entry on the raw operation. A wrong method is `405` `method_not_allowed` with `Allow: GET`
  (it was `invalid_request`). A view that cannot be serialized is a `500 internal`, no longer an ETag over
  `null`. The web pages are unchanged.

- **Raw content is gated by host, and isolation is enforced at start-up (IS-16; RFC 003 D-4).
  Breaking.** The raw route (`/api/v1/repos/{owner}/{name}/raw`) now answers only on the content
  origin's host; on any other host it is `404` with the body of an unknown route. The effective host is
  `Host`, or the last `X-Forwarded-Host` only from a peer in `PLANETER_TRUSTED_PROXIES`. A different port
  on the same host is *not* a distinct origin. **A non-loopback bind whose content origin has the
  application's host (the default) now refuses to start**, with a message naming
  `PLANETER_CONTENT_ORIGIN`; on a loopback bind one warning is logged and nothing else changes. There is
  no override variable. A deployment that proxies a second host name to the same planeter and sets
  `PLANETER_CONTENT_ORIGIN` to it keeps working. `planeter --help` is now the one list of environment
  variables (the copy in `main.rs`'s header is gone, and a test keeps `--help` and the code in step).
  A `PLANETER_CONTENT_ORIGIN` without a host (not `scheme://host[:port]`) is refused at start.

- **A real API contract and safe cache headers (IS-28, IS-29; RFC 003 D-2, STD-3).** **Behaviour
  change: API error responses now carry a JSON body** `{"code", "message", "hint"}` with
  `Content-Type: application/json` (they were empty). Codes: `not_found` (a repository that is absent and
  one the caller may not see are byte-identical), `unavailable`, `busy` (with `Retry-After`),
  `invalid_request` (a malformed query; was an unstructured `400`), `method_not_allowed` (`405` with
  `Allow: GET`), `raw_elsewhere` (see below) and `internal`. Status
  codes and success bodies are unchanged. `GET /api/v1/openapi.json` is now a complete OpenAPI 3.0.3
  document (parameters, responses, schemas, security, the package version); `ETag` values are now a
  specified hash of the canonical JSON and **change once on upgrade**, then stay stable across restarts,
  releases of the compiler and key order. **Caching change:** every dynamic response is
  `Cache-Control: private, no-cache` with `Vary: Cookie, Authorization` (sign-in, sign-out and any
  response that sets a cookie: `no-store`); the stylesheet is `public, max-age=3600`; every response
  carries `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()`; with
  `PLANETER_TRUSTED_PROXIES` set (the app declared to sit behind TLS) responses also carry
  `Strict-Transport-Security: max-age=15552000` (no `includeSubDomains`, no `preload`). Unknown `/api/`
  paths now answer with the JSON `not_found`; other unknown paths are unchanged.

- **Sign-in work off the async workers, and constant-work sign-in (IS-24, IS-31).** Password
  verification, token and session checks, session writes and the SSO hand-shake now run on the blocking
  pool, so a sign-in flood can no longer stall the server. Password verification is bounded by its own
  gate: `PLANETER_AUTH_CONCURRENCY` (default `min(CPUs, 4)`) and `PLANETER_AUTH_QUEUE_WAIT_SECS`
  (default 5), past which the sign-in is refused with `503` and `Retry-After`. **Behaviour change
  (security fix):** sign-in for an unknown account (or one without a password) now takes as long as for a
  wrong password; before, it returned immediately, which let a client tell whether an account exists by
  timing. This weakness was present in 0.2.0.

- **One compiler, named runners, and a release dry run (RFC 010 D-5, IS-26).** CI and the manual
  supply-chain checkpoint now install the same pinned Rust toolchain (1.99.0 with rustfmt and clippy)
  that the release builds with; every workflow names its runner image (`ubuntu-24.04`) instead of
  `ubuntu-latest`. `pin-check` fails on a second toolchain version and on a `*-latest` runner. The
  release workflow can be dispatched as a dry run that builds everything and publishes nothing. The
  release `gates` job now installs rustfmt and clippy for the pinned toolchain (the action installs a
  minimal profile; a real tag would have failed `cargo fmt`).

- **Read-path concurrency (IS-4, RFC 003 D-8): an unauthenticated denial of service closed.** Every
  prikk call from the browse UI and the read API now runs on the blocking pool (`spawn_blocking`) behind
  one process-wide gate, so slow repositories no longer pin the async workers and stall sign-in and every
  other route. New `PLANETER_PRIKK_CONCURRENCY` (default `min(CPUs, 8)`) caps concurrent prikk
  subprocesses; `PLANETER_PRIKK_QUEUE_WAIT_SECS` (default 10) bounds the wait, past which the request
  is refused with `503` and `Retry-After` (existence-neutral, counted). A page's several prikk calls share
  one slot. No route, body, authorization or sandbox change.

- **`planeter --version` / `-V` and `--help` / `-h` (RFC 010 D-6, IS-1).** `--version` prints three lines
  (`planeter X.Y.Z`, `commit <sha or unknown>`, `prikk-floor X.Y.Z`) and exits 0 **without reading
  configuration or starting the server**; release builds report the tag's commit and `build-info.txt`
  carries the same lines. **Behaviour change:** any other argument is now refused with a one-line error,
  the usage and **exit 2**; before, an unrecognised argument started the server. A script that passed an
  argument and relied on the server starting must drop it.

- **Maintenance gates (RFC 010):** `tools/floor-check` (the three prikk floors must agree) and
  `tools/pin-check` (every action by commit SHA, base image by digest, toolchain by version) run in CI and
  in the release gates; a manual-only `supply-chain.yml` workflow records a deny/audit checkpoint. The
  build chain is pinned: actions by commit SHA with the tag in a comment, `debian:bookworm-slim` by digest,
  Rust 1.99.0 explicit in the release workflow. No product behaviour changes.
- **Security hygiene:** `ammonia` 4.1.4 → 4.2.3 and `yoke-derive` 0.8.3 → 0.8.4 (lock-only), clearing
  RUSTSEC-2021-0157 / -0158 and a yanked crate; the sanitizer's behaviour is pinned by regression tests.

- **prikk floor → 0.52.0** (the tested version): the PK-30 fix, the accept size bound, crash-state
  recovery, dependency-ordered `bundle import` / `sync accept` writes. The container image bakes
  prikk 0.52.0 (it baked 0.46.0). Requires **prikk ≥ 0.52.0**.
- The driver passes `bundle import --input <file>` (it passed the path positionally).
- `trust maintainer check` answers "not trusted" with exit 1 on prikk 0.52.0; the driver reads that as
  the answer when the `trust-check-v1` document says so, and still treats every other non-zero exit as
  a failure.
- The sandbox profile can carry prikk's incoming-artifact bounds (`PRIKK_EXCHANGE_MAX_BYTES`,
  `PRIKK_EXCHANGE_MAX_OBJECTS`, `PRIKK_BUNDLE_MAX_BYTES`, `PRIKK_BUNDLE_MAX_OBJECTS`); none is set yet.

## [0.2.0] — the auth increment: trusted proxies, per-IP throttle, SSO

Shipped during the M2 hold (clone/push waits for prikk RFC 155 then 154, after prikk 0.49.0), after a
full documentation audit. Runtime prerequisites: prikk ≥ 0.46.0, bubblewrap, and `curl` if SSO is
enabled (the container image bundles all three).

- **Trusted reverse proxies** (`PLANETER_TRUSTED_PROXIES`): client IPs from `X-Forwarded-For` only
  behind declared proxies; a non-loopback bind is refused without them (planeter speaks plain HTTP
  behind TLS termination).
- **Per-client-IP login throttle** (20 failures across any accounts → 15-minute lock) alongside the
  per-account one.
- **SSO via OpenID Connect** (`PLANETER_OIDC_*`): authorization-code flow with PKCE, ID tokens verified
  against the provider's JWKS; provider traffic through the egress guard and a confined `curl` (new
  runtime prerequisite). Accounts are linked to `(issuer, subject)` administratively — no
  auto-provisioning.
- **Documentation**: the README describes the shipped product and how to install, run and deploy it;
  RFCs 001–003 are recorded as done; threat model v0.3 (OIDC status, residual-risk ids RR-10/RR-11);
  rustdoc builds warning-free.

## [0.1.1] — read side: browse UI, sign-in, egress guard

Work during the M2 hold (clone/push waits for prikk 0.48.0+).

- **Browse web UI**: repository home (directory listing, sanitized README, prikk-verified badge), tree,
  file (Markdown sanitized, text escaped, binary via the isolated content origin), history, change,
  refs, verify — the same read service and authorization gate as the API.
- **Sign-in / sign-out** with `HttpOnly`/`SameSite=Strict`/`Secure` session cookies, CSRF protection on
  every POST, and a per-account login throttle (5 failures → 15-minute lock). No MFA yet.
- **Egress guard**: the SSRF filter every outbound feature will route through (no callers yet).
- Threat model v0.2 records the implementation status of the auth, rendering and egress controls.

## [0.1.0] — M1: host + browse + auth

The first usable hosted forge over prikk (ROADMAP milestone M1 = A0 + RFC 002 + RFC 003).

### Hosting (RFC 001)
- Create and serve prikk repositories, laid out on disk by a stable opaque id so rename/transfer never
  moves bytes. Every prikk invocation runs in a **bubblewrap sandbox** (repo-dir-only writes, no
  network, wall-time bound, no operator keys).
- **Keyless forge:** planeter holds no history-signing key. The `forge-seal` cargo feature is reserved
  and off; CI proves no forge-signer symbol exists in the default build (ENF-2).
- Requires **prikk ≥ 0.46.0** (the version pin refuses older binaries).

### Authorization & authentication (RFC 002)
- A single pure, default-deny `authorize()`: visibility (public / internal / private), per-repo roles
  (read ⊂ write ⊂ maintain ⊂ admin) via direct, team, and org grants, credential scopes
  (`user-permissions ∩ scope`), protected refs, and no existence leak for hidden repositories.
- Local passwords hashed with **Argon2id**; scoped access tokens (SHA-256, constant-time compare);
  **ed25519** SSH public keys (fingerprints identical to `ssh-keygen -lf`). OIDC/SSO is a seam for a
  later release.
- Persistent **SQLite** for every store (`PLANETER_DB`).

### Browse (RFC 003)
- A GET read API (OpenAPI-described; `ETag`/`304`; bounded pages) over the honest view-models:
  history, change, file, **directory listing** (`prikk tree`), **raw file download** (`prikk cat`),
  refs, and verify status. Approved-but-unsealed changes are never shown as prikk-verified.
- Rendering safety: a strict HTML/Markdown sanitizer, a strict CSP, and raw bytes served only as
  inert downloads from an isolated content origin.

### Not in this release
- Clone/push (M2), review/merge (M3), CI (M4), packages (M5); blame/annotate (awaiting a prikk verb).
