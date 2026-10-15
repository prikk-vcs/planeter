# RFC 002 follow-on — Sign-in and session work off the async workers — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 002 (listed in its Status). It applies RFC 003's IS-4 fix to the authentication path: the dev team's concurrency review noted that sign-in, session and token checks, Argon2id verification and the SQLite mutex run synchronously on the async workers. Issue **IS-24**. |
| Status | **Assignable** (architect authorization, SCHEDULE plan 1 step 3). Start with T0 (measure) and report before T1 if its findings change the shape. |
| Basis | [`../../done/002-authorization.md`](../../done/002-authorization.md) D-2, D-7; threat model T-2 (C-2b throttles), T-9, C-9b; `rfcs/handoffs/003-read-path/read-concurrency-handoff-v1.md` and its review (the `PrikkGate` helper in `planeter-web`); `crates/planeter-auth`, `crates/planeter-store/src/sqlite.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

After IS-4, prikk work is gated. The authentication path is the same class of problem and is reachable
without credentials: a login request runs an Argon2id verification (memory-hard, tens of milliseconds or
more) and SQLite access behind a mutex directly on a runtime worker. The per-account and per-IP throttles
bound *repeated failures from one source*; they do not bound a distributed flood or CPU already spent
before a lock is consulted.

## Change scope

- **T0 — Measure and report.** On the development host: the wall time of a login (hit and miss), whether the
  SQLite mutex is held across password hashing, and the order of operations (is the throttle consulted
  *before* the hash?). Report numbers. **If hashing runs under the store mutex, stop after T0 and hand back.**
- **T1 — Offload.** Every call from an async handler into `planeter-auth` that hashes a password, touches
  SQLite or verifies a token/key runs through one helper on `spawn_blocking`. A source-scan test forbids
  direct calls, like `no_handler_calls_the_read_service_directly`.
- **T2 — A separate, small gate for password hashing.** A second semaphore (**not** the prikk gate),
  `PLANETER_AUTH_CONCURRENCY` (default `min(available_parallelism, 4)`), bounded wait
  `PLANETER_AUTH_QUEUE_WAIT_SECS` (default 5), past which the sign-in is refused with **503** and
  `Retry-After`, counted. Only Argon2id verification takes a permit; token and session checks do not.
- **T3 — Throttle before hash.** If T0 shows a locked account or IP is still hashed, consult the throttle
  first and refuse without hashing (the refusal must remain indistinguishable from a wrong password where it
  is today).
- **T4 — Tests** under `tests/`, not inline: N + 1 concurrent sign-ins with a deliberately slow stand-in
  hasher; a cheap route answers in under a second while the gate is full; a locked account is refused without
  a hash (counted by the stand-in); a cancelled request releases its permit.

## Non-change scope

No change to hashing parameters, throttle thresholds, response bodies, cookies, CSRF, OIDC flows, or any
authorization decision. No new dependency (`tokio` primitives already present).

## Acceptance criteria

T4's tests pass; no async handler hashes or touches SQLite directly; defaults and bounds documented in the
`main.rs` header and RELEASING; all gates green including both supply-chain gates, both test runs named;
IS-24 closed.

## Prohibited shortcuts

Do not lower Argon2id cost to "fix" it; do not drop sign-ins silently; do not share the prikk gate; do not
make unknown-user and wrong-password responses distinguishable (timing included: keep the existing dummy
verification path).

## Security and compatibility constraints

The 503 must not reveal whether an account exists. Behaviour at defaults is unchanged for a single user.

## Known risks

A too-small default starves a burst of legitimate sign-ins (the wait and the cap are configurable); moving
SQLite access to blocking threads must not change transaction boundaries (state how you checked).

## Required evidence and review-request format

T0's numbers; commits (hash → what); every gate with command and exit code; the test names and a short log of
the responsiveness test; deviations and open questions.
