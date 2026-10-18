# RFC 011 — Read-path resource model — Implementation Handoff (v1): the prikk boundary — ceiling and lifetime

| | |
|---|---|
| Document | Implements RFC 011 **D-2** (the per-call ceiling and the honest mapping of a ceiling hit) and **D-5** (a child never outlives the server), in the prikk boundary crate. It is the first of two implementation handoffs; the second (admission, `too_large`, size bounds: D-1, D-3, D-4) follows after the owner decides the budget cap and builds on the ceiling parameter this one adds. Issues IS-36 (part), IS-37. |
| Status | **Assigned** (2026-10-18). Targets the line after 0.3.0 (0.4.0); nothing here changes the released 0.3.0. |
| Basis | [`../../accepted/011-read-path-resource-model.md`](../../accepted/011-read-path-resource-model.md) D-2, D-5 and the resolved unknowns; the private investigation report `resource-model-investigation-review-request.md` and its logs and harnesses (reuse them: they contain working versions of both mechanisms); `crates/planeter-prikk/src/sandbox.rs`; `crates/planeter-web/src/gate.rs`, `api.rs` (`read_blocking`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The ceiling parameter.** The sandbox gains an optional per-call **data ceiling** (bytes). When set, the
  production vector is prefixed with `prlimit --data=<N> --` (candidate A of the investigation). The public
  call path (what `ReadService`/`read_blocking` use) takes the ceiling as an argument; **until the admission
  handoff supplies a value, production passes `None`** (no behaviour change at defaults). Safe code only:
  `#![forbid(unsafe_code)]` stays in the crate.
- **T2 — The `prlimit` prerequisite is explicit.** At start, planeter probes `prlimit` (e.g. `prlimit --version`
  through the same lookup as the other binaries) and, if absent, **refuses to start with a message that names
  `prlimit` and util-linux** (the same discipline as the prikk floor check). Document it as a runtime
  prerequisite (README, `docs/RELEASING.md`'s prerequisites list, `release.yml`'s release-notes line and
  `build-info.txt`'s `requires:` line); state that the container image has it (verified from the pinned base
  layer by the investigation; confirm with the image's own `prlimit --version` in the dry run's image build if
  that can be done without publishing, otherwise say it is unverified).
- **T3 — A ceiling hit is never a verdict.** An abort under the ceiling (exit 134 / "memory allocation … failed")
  and a **partial `verify` report** ("did not complete every stage", exit 1 with incomplete stages) are returned
  as a distinct boundary error (e.g. `PrikkError::CeilingExceeded`), which the read service maps to
  `ReadError::Unavailable`; **never** to a verification result and never to "unverified". A test with a
  deliberately low ceiling proves it for `verify`, `tree`, `cat`. Log one line naming the ceiling and the verb
  (no paths, no tool output).
- **T4 — Lifetime.** Drop `timeout` from the production vector: `bwrap` (with `--die-with-parent`) becomes the
  server's direct child. The wall-time bound (default 120 s; configurable as today) is kept **by the server**:
  at the deadline it kills the child (SIGKILL) and returns the existing timeout error; make sure no process of
  the call remains (the investigation's D1 had 0 survivors). **Spawn and wait happen in the same thread** (the
  existing `spawn_blocking` closure shape); a comment at the spawn site states why, and a test pins it (spawn the
  call from a blocking-pool thread configured with a very short keep-alive, run a call longer than that
  keep-alive and show it completes — see investigation I3, rows A1/A1′).
  Remove the `timeout`/`timeout_secs` fields' coupling to the external `timeout` binary; keep the user-visible
  setting if one exists. `timeout` (coreutils) is no longer a runtime prerequisite: update the docs and the
  dry-run `requires:` line.
- **T5 — Tests** under `tests/` (not inline): the vector (prefix present only when a ceiling is set; no
  `timeout`); the probe's refusal message; the ceiling-hit mapping (T3); a deadline kill leaves no process
  (use a stand-in long-running program through the same vector, as the investigation did with `sleep`); **a real
  server kill** — start the built `planeter` binary against a repository root with a stand-in or real prikk and a
  long call in flight, `SIGKILL` the server, and assert that no `bwrap`/prikk process of that call remains after a
  bounded settle time (a few seconds); the one-thread pin test above. Tests that need `prikk`, `bwrap` or
  `prlimit` self-skip with a printed reason, and the report states which ran in your environment.

## Non-change scope

No admission or budget logic (the next handoff); no new error code in the API; no change to what a response
looks like; no `unsafe`; no new dependency (if one seems needed, stop and report); no change to the write
path (there is none yet); no release or version change.

## Required documentation updates

README (prerequisites), `docs/RELEASING.md` (prerequisites; the sandbox description), the `sandbox.rs` module
docs (they say rlimits are a later scope), CHANGELOG `[Unreleased]` (**`prlimit` is a new prerequisite;
`timeout` no longer is; a killed server no longer leaves its children running**), threat model status line
under T-9 (architect updates; flag it in your report), STATUS IS-37 closed on review.

## Acceptance criteria

T5's tests pass; the real-server-kill test passes and fails when the tie is removed (show the mutation); every
gate green including both supply-chain gates (refresh the advisory database first, as the last handoff did),
both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No `unsafe`; no post-spawn `prlimit(pid)`; no systemd/cgroup dependence; no thread-agnostic claims without the
test; no silent fallback when `prlimit` is missing; do not map a ceiling hit to any verification outcome.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; the T5 test names and which self-skipped; the
mutation evidence for the kill test and the one-thread test; one captured start-up refusal; deviations and open
questions.
