# RFC 003 follow-on — Raw-content origin: gate by host and say what is enforced — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 003 D-4 (raw repository bytes are served from an isolated content origin). Issue **IS-16**: isolation is operator configuration, the default content origin equals the app origin, and the raw route also answers on the app host, while SECURITY.md and the threat model say raw bytes are served "only from an isolated origin". |
| Status | **Assignable** (architect authorization, SCHEDULE plan 1; candidate-release item). |
| Basis | [`../../done/003-read-path.md`](../../done/003-read-path.md) D-4; threat model T-6 C-6c; `SECURITY.md`; `crates/planeter-web/src/api.rs` (`raw_handler`, `build_raw`), `ui.rs` (raw links built from `content_origin`), `crates/planeter/src/main.rs` (`PLANETER_CONTENT_ORIGIN`, default `http://<addr>`); `docs/RELEASING.md`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

The raw route (`/api/v1/repos/{owner}/{name}/raw`) is on the same router as the app and has no host check.
The headers it sends (attachment, `nosniff`, a sandbox CSP) make a download inert wherever it is fetched, so
exploitation needs a browser to render an attachment: the practical risk is low. But the *stated control* is
isolation from the app origin, and today nothing enforces or even reports whether it exists.

## Change scope

- **T1 — Gate the raw route by host.** When the configured content origin's host differs from the app's:
  the raw route answers **only** for requests whose effective host equals the content origin's host, and
  returns `404` (existence-neutral, same body as an unknown route) on any other host. The effective host is
  the `Host` header, or `X-Forwarded-Host` **only** when the TCP peer is a configured trusted proxy
  (the same rule as the client-IP rule). **"Distinct" means a different host.** A different *port* on the same
  host is **not** isolation: cookies are host-scoped and ignore the port, so a page on `host:8443` still
  receives the app's cookies. A port-only difference counts as the same origin for T1 and T2.
- **T2 — Safe by default.** *(Revised 2026-10-15: the release is 0.3.0, a minor, so a safe default is
  allowed to be a breaking one; owner philosophy: clean, safe, secure.)* When planeter is bound to a
  **non-loopback** address (the production shape, which already requires trusted proxies) and the content
  origin's host equals the app's (the default), **refuse to start** with one plain message naming the
  variable and the fix: `planeter: refusing to start: raw repository content would be served from the
  application's own host, which breaks the isolation the documentation promises. Set
  PLANETER_CONTENT_ORIGIN to a different host (for example https://raw.example.org).` On a loopback
  bind (development) log one warning instead. There is no escape-hatch variable: a second host name for the
  same proxy is all it takes.
- **T3 — Make the wording true.** `SECURITY.md`, threat model C-6c status and `docs/RELEASING.md` say:
  raw bytes are always inert downloads (attachment, `nosniff`, sandbox CSP); *isolation from the app origin*
  holds when the operator configures a distinct content origin, in which case the app host refuses the raw
  route; planeter warns when it is not configured. (The architect updates `planeter-03`; the dev team
  updates SECURITY.md and RELEASING and quotes the final text in the review request.)
- **T4 — Tests** under `crates/planeter-web/tests/`: distinct hosts → raw on the content host 200, on the
  app host 404, with a spoofed `X-Forwarded-Host` from an untrusted peer 404, from a trusted proxy 200;
  same host on loopback → raw 200 on the app host and one warning; same host on a non-loopback bind, and a
  port-only difference on a non-loopback bind → start refused with the message above; the UI links still
  point at the content origin.
- **T5 — One list of variables.** `args::USAGE` is the single source of the environment-variable list: remove
  the hand-copied list from `main.rs`'s header comment in favour of a pointer to `--help`, and add a test
  that every `PLANETER_*` name read in `main.rs` appears in `USAGE` (so the two can never drift).

## Non-change scope

No change to the raw route's response headers or body, to authorization, to the UI pages or to any other
route. No new dependency, no new environment variable.

## Acceptance criteria

T4's tests pass; the refusal and the warning appear exactly as stated and not otherwise; T5's drift test passes;
SECURITY.md and RELEASING carry the true wording; all gates green (both supply-chain gates), both test runs
named; IS-16 closed.

## Prohibited shortcuts

Do not trust `X-Forwarded-Host` from an arbitrary peer; do not add an escape-hatch variable; do not
rewrite the raw headers; do not weaken the CSP of any page to accommodate the content origin.

## Security and compatibility constraints

A deployment that proxies two hostnames to one planeter keeps working unchanged. Two behaviours change and go
in the CHANGELOG and the release notes as **breaking**: a production-shaped deployment with no distinct
content host now refuses to start, and a deployment that served raw bytes from the app host while
configuring a distinct origin gets 404 there.

## Known risks

A reverse proxy that rewrites `Host` to the upstream's: the operator must preserve it or send
`X-Forwarded-Host` and declare the proxy trusted (document in RELEASING).

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the test names; the final wording of the
three documents; deviations and open questions.
