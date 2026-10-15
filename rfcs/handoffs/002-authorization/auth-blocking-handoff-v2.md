# RFC 002 follow-on — Sign-in and session work off the async workers, with constant-work verification — Implementation Handoff (v2)

| | |
|---|---|
| Document | A **follow-on** handoff of the done RFC 002 (listed in its Status). It applies RFC 003's IS-4 fix to the authentication path (**IS-24**) **and** removes the account-existence timing difference that the v1 handoff's T0 found (**IS-31**). Supersedes [`auth-blocking-handoff-v1.md`](auth-blocking-handoff-v1.md), whose T0 is done. |
| Status | **Assigned** (SCHEDULE plan 1 step 3; release 0.3.0 step R2). T0 is complete and approved; start at T1. |
| Basis | [`../../done/002-authorization.md`](../../done/002-authorization.md) D-2, D-7; threat model T-2 (C-2b throttles), T-9, C-9b; the dev team's T0 report (`.git-exclude/review-requests/auth-blocking-t0-report.md`) and the architect's review of it; `rfcs/handoffs/003-read-path/read-concurrency-handoff-v1.md` (the `PrikkGate` helper in `planeter-web`); `crates/planeter-auth/src/authenticator.rs`, `crates/planeter-web/src/ui.rs` (`login_submit`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## What changed since v1 — and why (verified facts)

1. **The v1 handoff said "keep the existing dummy verification path". There is none.** That was the
   architect's premise and it was wrong: `authenticate_password` returns at once when the account is missing
   or has no password, and only hashes for an existing password account. The T0 report measured ≈ 0.00 ms
   (unknown user) against ≈ 8–9 ms (wrong password); the architect confirmed the structure in the code.
2. **So an account-existence timing oracle exists today**, although every failure returns the same
   `BadCredentials`, and `authenticator.rs`'s own doc says the cases cannot be told apart. They can, by time.
3. **T2 as written in v1 would have added a second oracle** (a 503 for existing accounts only).
4. Verified and kept: hashing does **not** run under the store mutex (the stop condition is not met); the
   throttle is consulted **before** any hash (v1's T3 is already true: keep it, test it).

## Purpose

Take the authentication path's blocking work off the async workers, bound it, and make a sign-in cost the
same whether or not the account exists, so neither the response time nor the 503 reveals existence.

## Change scope

- **T1 — Constant-work verification (new, first).** In `Authenticator`, build **one dummy PHC hash at
  construction** with the *same* `PasswordHasher` and parameters, from a random password that is never
  stored or logged. In `authenticate_password`, when the account is missing **or has no password**, verify
  the presented password against the dummy hash and then return `BadCredentials` **regardless of the
  result**. The three failure cases (unknown user, no password, wrong password) each perform exactly one
  `verify`. No other behaviour changes. Fix the `authenticator.rs` module documentation to say what is
  true (indistinguishable in response *and* in work).
- **T2 — Offload.** Every call from an async handler into `planeter-auth` that hashes a password, touches
  SQLite or verifies a token/key runs through one helper on `spawn_blocking`. A source-scan test forbids
  direct calls, like `no_handler_calls_the_read_service_directly`. The T0 report lists the call sites
  (`login_submit`, `logout`, `ctx()` / `principal_from_request` on every request, the OIDC callback).
- **T3 — A separate, small gate for password verification.** A second semaphore (**not** the prikk gate),
  `PLANETER_AUTH_CONCURRENCY` (default `min(available_parallelism, 4)`), bounded wait
  `PLANETER_AUTH_QUEUE_WAIT_SECS` (default 5), past which the sign-in is refused with **503** and
  `Retry-After`, counted. **Every `verify` takes a permit — real or dummy** — so the 503 is
  existence-neutral. Token and session checks do not take a permit.
- **T4 — Throttle before hash: keep, and test.** Already true. Add a test that a locked account or IP is
  refused (429) without any `verify` call.
- **T5 — Tests** under `tests/`, not inline:
  - a counting stand-in hasher: unknown user, account without password and wrong password each call
    `verify` **exactly once**; a right password once (deterministic: do not assert wall-clock equality);
  - N + 1 concurrent sign-ins with a slow stand-in hasher: the excess get 503 + `Retry-After`, **for
    existing and unknown usernames alike** (same status, same headers, same body);
  - a cheap route answers in under a second while the gate is full;
  - a cancelled request releases its permit;
  - the dummy never authenticates: presenting the dummy's own password for an unknown name returns
    `BadCredentials` (use the stand-in hasher's accept-everything mode to prove the result is ignored).

## Non-change scope

No change to hashing parameters, throttle thresholds, response bodies, cookies, CSRF, OIDC flows, token
checks or any authorization decision. No new dependency (`tokio` primitives already present; the dummy hash
uses the existing hasher and an existing random source: if none is available without a manifest change,
**stop and report**).

## Acceptance criteria

T5's tests pass; no async handler hashes or touches SQLite directly; the three failure cases do equal work;
defaults and bounds documented in the `main.rs` header and RELEASING; CHANGELOG `[Unreleased]` states that
sign-in for an unknown account now takes as long as for a wrong password; all gates green including both
supply-chain gates, both test runs named; IS-24 and IS-31 closed.

## Prohibited shortcuts

Do not lower Argon2id cost; do not drop sign-ins silently; do not share the prikk gate; do not make the
dummy a constant string checked by equality or a sleep (it must be the same hasher's real verification);
do not return early on a missing account anywhere in the path; do not log which case occurred.

## Security and compatibility constraints

Neither the status, the body, the headers (including `Retry-After`) nor the work may distinguish the
cases. Behaviour at defaults is unchanged for a single legitimate user; an unknown-account attempt costs
one hash (bounded by the IP throttle and the gate).

## Known risks

- **Parameter drift:** a stored hash made with older Argon2id parameters costs differently from the dummy
  (made with the current ones). Rehash-on-upgrade does not exist today; state in the report whether any
  stored-hash parameter differs in a deployment you can see, and leave it recorded as a known limit.
- A too-small default starves a burst of legitimate sign-ins (the wait and the cap are configurable).
- Moving SQLite access to blocking threads must not change transaction boundaries (state how you checked).

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the T5 test names and the counting-hasher
output; a short log of the responsiveness test; deviations and open questions.
