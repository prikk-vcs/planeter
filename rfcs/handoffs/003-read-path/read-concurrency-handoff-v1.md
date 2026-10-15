# RFC 003 follow-on — Read-path concurrency and bounds — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 003, carrying out its unmet acceptance of D-8 (a bounded number of prikk calls per page; correctness never depends on the call model) and RFC 001 D-3's bound on invocations. It fixes issue IS-4. Listed in RFC 003's Status as follow-on work. |
| Status | **Assignable** (architect authorization under SCHEDULE plan 1, step 3; the issue register entry IS-4 was accepted by the owner on 2026-10-15). It needs no prikk change and no design decision beyond the minor concretizations named below. |
| Basis | [`../../done/003-read-path.md`](../../done/003-read-path.md) D-3, D-8; RFC 001 D-3; threat model T-9, C-9b; issue register IS-4 (re-rated 2026-10-15: an unauthenticated denial of service on any public repository); `crates/planeter-prikk/src/cli.rs` (`std::process::Command::output()`), `crates/planeter-web/src/api.rs` and `ui.rs` (async handlers calling the synchronous `ReadService`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

Every read handler is an `async fn` that calls the synchronous `ReadService`, which spawns a sandboxed prikk
and blocks until it exits (up to the 120 s wall-time bound). On the default multi-thread runtime each
in-flight call pins a worker thread, so as many slow requests as there are workers stall the whole server,
including sign-in. Nothing caps concurrent prikk subprocesses. A public repository makes this reachable
without authentication.

## Change scope

- **T1 — Move blocking work off the async workers.** Every call from an async handler into the read service
  runs through `tokio::task::spawn_blocking` (one helper, so that no handler can forget it). Join errors
  and cancellation map to the existing `ReadError::Unavailable`/500 behaviour; no new status codes.
- **T2 — Cap concurrent prikk invocations.** A process-wide `tokio::sync::Semaphore` acquired *before*
  spawning blocking work. Configuration `PLANETER_PRIKK_CONCURRENCY` (positive integer); **default:
  `min(available_parallelism, 8)`** (a minor concretization by the architect, adjustable by the owner).
  Waiting is bounded by `PLANETER_PRIKK_QUEUE_WAIT_SECS` (default 10): past it, the request is refused with
  **503** and `Retry-After`, existence-neutral, and a counter is incremented. The cap and the wait are
  logged once at start.
- **T3 — Tests** under `crates/planeter-web/tests/` (and `planeter-core/tests/` for the helper), not inline:
  with a stand-in `prikk` that sleeps, *N + 1* concurrent requests: the first *N* run, the extra waits and
  then succeeds or gets 503 as configured; **a cheap route (the OpenAPI document) answers in under one second
  while all *N* slots are busy**; a cancelled request releases its permit.

## Non-change scope

No change to routes, response bodies, authorization, the sandbox profile, the wall-time bound, caching or
any crate other than `planeter-web`, `planeter-core` (the helper) and `planeter` (config). No new
dependency (`tokio` already provides both primitives; confirm with `cargo tree`, and **stop** if a feature
flag would add crates). No use of `block_in_place`.

## Required documentation updates

`crates/planeter/src/main.rs` header (the two variables), `docs/RELEASING.md` (deployment variables),
CHANGELOG `[Unreleased]`, issue register IS-4 closed with the commit.

## Acceptance criteria

- No async handler calls blocking prikk work directly (a test or a grep-gate in the review request shows
  it); T3's four behaviours pass; defaults and bounds documented.
- All gates green, **including both supply-chain gates** (they are green by then, RFC 010's lock update),
  both test runs named, ENF-2.

## Prohibited shortcuts

Do not raise the runtime's worker count to "fix" it; do not cap by dropping requests silently; do not
convert `ReadService` to async by wrapping it in a mutex; do not add a global lock around prikk calls.

## Security and compatibility constraints

The 503 must not reveal repository existence or prikk output. Behaviour at the default cap must be
indistinguishable from today for a single user. Compatibility: additive environment variables only.

## Known risks

A too-low default starves legitimate bursts (mitigated by the 10 s wait and the configurable cap);
`spawn_blocking`'s own thread pool is large and is *not* the limiter: the semaphore is.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the T3 test names and a short log of the
responsiveness test; the grep or test proving no direct blocking call remains; deviations and questions.
