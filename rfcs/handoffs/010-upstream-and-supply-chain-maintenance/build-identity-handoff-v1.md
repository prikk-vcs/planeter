# RFC 010 — Build identity — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 010 decision D-6 (the binary identifies itself and never starts a server by accident). |
| Status | Inherited from RFC 010 — **Proposed**: not assignable until the owner accepts it. |
| Basis | [`../../proposed/010-upstream-and-supply-chain-maintenance.md`](../../proposed/010-upstream-and-supply-chain-maintenance.md) D-6; issue register IS-1; `crates/planeter/src/main.rs` (configuration is read from environment at the top). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

The `planeter` binary has no `--version`, and an unrecognised argument starts the server (IS-1). An
operator or a release check cannot ask a build what it is.

## Change scope

- **T1 — Argument handling.** In `crates/planeter/src/main.rs`, before configuration is read or any socket
  opened: `--version` / `-V` print three lines — `planeter X.Y.Z`, `commit <sha or unknown>`,
  `prikk-floor X.Y.Z` — and exit 0; `--help` / `-h` print usage (the environment variables the binary reads,
  as listed at the top of `main.rs`) and exit 0; **any other argument** prints a one-line error and the usage
  and exits 2. No argument runs as today.
- **T2 — Values.** Version from `CARGO_PKG_VERSION`; floor from `planeter_prikk::cli::MIN_PRIKK_VERSION`
  (do not duplicate the constant); commit from `option_env!("PLANETER_GIT_COMMIT")`, `unknown` when absent.
  **No build script.** The release workflow sets `PLANETER_GIT_COMMIT` for the build and writes the same
  three fields into `build-info.txt`.
- **T3 — Tests.** Under `crates/planeter/tests/`: run the built binary with each argument and assert stdout
  and exit status, **with the environment cleared so that it cannot start a server by accident**, and with a
  short timeout that fails the test if a server starts.

## Non-change scope

No change to configuration variables, defaults, routes or any crate other than `crates/planeter` (and the
release workflow's build step and `build-info.txt` lines). No new dependency (no argument-parsing crate; the
three flags are matched by hand, per dependency-policy rule 1).

## Acceptance criteria

The six invocations (`--version`, `-V`, `--help`, `-h`, an unknown argument, none) behave as T1 states;
`planeter --version` from a release artifact shows the tag's commit; `build-info.txt` carries the same
fields; gates green, both test runs named; no manifest change.

## Prohibited shortcuts

Do not read configuration before handling arguments; do not use a build script or add `git` to the build;
do not hard-code the floor.

## Security and compatibility constraints

A script that passed an unknown argument and relied on the server starting will now fail with exit 2: record
it in CHANGELOG. The output must not include paths, hostnames or environment values.

## Known risks

`option_env!` is evaluated at compile time: a cached build can embed a stale commit (state it in the doc
comment; the release workflow builds clean).

## Required evidence and review-request format

Commits; every gate with command and exit code; the output of the six invocations; the `build-info.txt`
lines; deviations and open questions.
