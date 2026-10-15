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
  (the same rule as the client-IP rule). Ports are compared when the origin states one.
- **T2 — Say it at start-up.** When the content origin equals the app origin (the default), log one
  `planeter: warning: raw content is served from the application origin; set PLANETER_CONTENT_ORIGIN to a
  separate origin for production`, **and** state it in the same start-up line block that reports the
  concurrency gate. Do **not** refuse to start (a patch release must not break existing deployments).
- **T3 — Make the wording true.** `SECURITY.md`, threat model C-6c status and `docs/RELEASING.md` say:
  raw bytes are always inert downloads (attachment, `nosniff`, sandbox CSP); *isolation from the app origin*
  holds when the operator configures a distinct content origin, in which case the app host refuses the raw
  route; planeter warns when it is not configured. (The architect updates `planeter-03`; the dev team
  updates SECURITY.md and RELEASING and quotes the final text in the review request.)
- **T4 — Tests** under `crates/planeter-web/tests/`: distinct origins → raw on the content host 200, on the
  app host 404, with a spoofed `X-Forwarded-Host` from an untrusted peer 404, from a trusted proxy 200;
  same origin → raw 200 on the app host (unchanged); the UI links still point at the content origin.

## Non-change scope

No change to the raw route's response headers or body, to authorization, to the UI pages, to any other
route, or to the default behaviour of a same-origin deployment (except the warning). No new dependency.

## Acceptance criteria

T4's tests pass; the warning appears exactly once at start in the same-origin case and not otherwise;
SECURITY.md and RELEASING carry the true wording; all gates green (both supply-chain gates), both test runs
named; IS-16 closed.

## Prohibited shortcuts

Do not trust `X-Forwarded-Host` from an arbitrary peer; do not refuse to start; do not rewrite the raw
headers; do not weaken the CSP of any page to accommodate the content origin.

## Security and compatibility constraints

A deployment that proxies both hostnames to one planeter keeps working unchanged. A deployment that serves
raw bytes from the app host *while configuring a distinct origin* now gets 404 there: that is the point,
and it goes in the CHANGELOG as a behaviour change.

## Known risks

A reverse proxy that rewrites `Host` to the upstream's: the operator must preserve it or send
`X-Forwarded-Host` and declare the proxy trusted (document in RELEASING).

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the test names; the final wording of the
three documents; deviations and open questions.
