# Changelog

Releases are bare version tags; each section is the release's notes (assembled into the GitHub
release by `release.yml`). Kept by hand, per release, so nothing here goes stale by templating.

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
