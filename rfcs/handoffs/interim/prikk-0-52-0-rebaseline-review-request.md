# Interim — prikk 0.52.0 re-baseline — review request

| | |
|---|---|
| Document | The dev team's review-request package for [`prikk-0-52-0-rebaseline-handoff-v2.md`](prikk-0-52-0-rebaseline-handoff-v2.md) (instance 0.52.0; no RFC of its own). |
| Status | **Implemented; one gate red for a pre-existing reason; not pushed.** Awaiting architect review. |
| Date | 2026-10-15, against `prikk --version` = **0.52.0** |
| Branch | `main` at `a55168e` (4 local commits on top of `4542e05`; **nothing pushed**) |

## Summary

T1–T6 are implemented and verified on the real 0.52.0 binary. No `schema_version` changed; no field
planeter reads was removed. The driver now passes `bundle import --input`, reads prikk 0.52.0's
exit-1 "not trusted" answer, and the sandbox profile carries the four incoming-bound variables (none
set). The floor is raised to 0.52.0 in all three places. **`cargo deny check` and `cargo audit` are red
for reasons outside this change (two ammonia advisories, section "Unresolved"); no manifest or lock file
changed.**

## Commits (on `main`, not pushed)

| Commit | Tasks | What landed |
|---|---|---|
| `f82b378` | T3 | `sandbox::IncomingBounds` on `BwrapConfig`; `tests/sandbox_env.rs` (4 argument-vector tests) |
| `7189562` | T1, T2, T4, T5 | `bundle import --input`; `trust_check` reads the document on exit 1; shared JSON parse helper; `tests/trust_check.rs`; five new tests in `tests/driver.rs` |
| `038d53c` | T6 | floor 0.52.0 in `cli.rs`, `release.yml`, `Dockerfile`; README, ROADMAP, STATUS, CHANGELOG `[Unreleased]`; ledger v1.7 |
| `a55168e` | T6 | the same floor in `docs/dependency-policy.md` and two ledger notes |

## T1 — schema verdict (measured on 0.52.0)

Method: a sealed scratch fixture (isolated key directory) with one queued edit; every verb dumped raw
with `--format json`; the key paths compared with the structs in `model.rs`; and
`driver::every_schema_the_driver_reads_parses_on_the_installed_prikk` through the sandbox.

| Id | Verb | `schema_version` | Fields planeter deserializes | Note |
|---|---|---|---|---|
| `log-report-v1` | `log` | unchanged | unchanged | |
| `show-report-v1` | `show <block>` | unchanged | unchanged | |
| `verify-report-v1` | `verify` | unchanged | unchanged | |
| `branch-list-v1` | `branch --all` | unchanged | unchanged | **new field `unpublished_current_branch`** (ignored by serde, not modelled) |
| `tag-list-v1` | `tag` | unchanged | unchanged | list was empty |
| `status-report-v1` | `status` | unchanged | unchanged | `provisional_worktree`, `interrupted_materialization` were null |
| `worktree-status-report-v1` | `worktree-status` | unchanged | unchanged | exits **1** on a dirty worktree, JSON still written |
| `patch-plan-content-v1` | `checkout --patch-plan --content-path` | unchanged | unchanged | |
| `tree-listing-v1` | `tree` | unchanged | unchanged | |
| `path-content-v1` | `cat --format json` | unchanged | unchanged | |
| `key-status-v1` | `key status` | unchanged | unchanged | **raw dump only**: through the keyless sandbox `key status` exits 1 (no key directory), by design |
| `trust-list-v1` | `trust maintainer list` | unchanged | unchanged | |
| `trust-check-v1` | `trust maintainer check` | unchanged | unchanged | exit code changed, see T5 |
| `diff-report-v1` | `diff` | unchanged | not read | nothing in the code reads it |

Not exercised by the fixture (unverified beyond the model's unit tests): non-null
`provisional_worktree` / `interrupted_materialization`, non-empty `declarations`, `received`, `tags`.

## T2–T6 evidence

- **T2.** `driver::explicit_current_branch_on_an_unsealed_repo_matches_the_bare_form`: `tree` and `log`
  with `--ref heads/main` are equal to the bare form (`PartialEq` on the typed reports) on 0.52.0. It
  prints `SKIPPED (PK-30 is fixed in prikk 0.47.0; installed: …)` below 0.47.0. **The 0.46.0 skip output
  is not quoted: no 0.46.0 binary is installed.** The skip path is a few lines and is not itself tested.
- **T3.** Four variables, each `--setenv NAME n` only when set, before the `--` separator, after
  `--clearenv`. End to end through bubblewrap:
  `driver::sandbox_bound_reaches_prikk_and_refuses_an_oversize_artifact`. Unconfined mode cannot carry
  the knob (it has no profile); it is dev-only.
- **T4.** `driver::bundle_round_trip_lands_under_remotes`: export from a sealed repo, `import --input`
  into a fresh one, the ref is under `remotes/heads/main`, no local `heads/main` appears; a missing
  artifact is a typed `Command` error. With the old positional argument this test fails on 0.52.0
  (mutation checked).
- **T5.** Measured: adopted key → exit 0, `trusted: true`; unknown key → **exit 1, document written**
  (`trusted: false`, `public_key: null`). `trust_check` returns the typed answer only for exit **1** with
  a well-formed `trust-check-v1` that says `trusted: false`; usage exit 2, an absent/garbage/foreign-schema
  document, and a contradictory `trusted: true` on exit 1 all stay `PrikkError::Command`. Tests:
  `tests/trust_check.rs` (eight shapes via a stand-in `prikk`) and
  `driver::trust_check_answers_for_adopted_and_unknown_keys`. **The 0.46.0 shape (exit 0 + the same
  document) is a stand-in built from the ledger and prikk's letter, not a recording from a 0.46.0
  binary.** With the exit-1 branch disabled the live-binary test fails (mutation checked).
- **T6.** Floors: `cli.rs` `(0, 52, 0)`, `release.yml` `PRIKK_VERSION: "0.52.0"`, `Dockerfile`
  `ARG PRIKK_VERSION=0.52.0`. The four 0.52.0 Linux assets (`x86_64`/`aarch64`, tarball and `.sha256`)
  answer HTTP 200 at the URLs the Dockerfile builds. **The image itself was not built.**
- **Promotion basis for PK-31** (the handoff's tasks do not measure it, so I did): on 0.52.0,
  `PRIKK_EXCHANGE_MAX_BYTES=64 prikk sync accept` on a 4,096-byte file exits 1 with
  `sync exchange artifact at … is 4096 bytes, over the limit of 64 bytes … set by
  $PRIKK_EXCHANGE_MAX_BYTES`; without the variable the same file is refused for its content
  (`invalid patch-exchange artifact magic`).

## Gates

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | exit 0 (after `cargo fmt --all` reformatted my new tests only) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --locked`, prikk 0.52.0 on `PATH` | exit 0, 139 passed, 0 failed |
| the same, prikk genuinely **off** `PATH` | exit 0, 139 passed, 0 failed (the 9 driver tests print `skipping: prikk not on PATH`) |
| `cargo deny check` | **exit 1** — `advisories FAILED` (bans, licenses, sources ok) |
| `cargo audit` | **exit 1** — 2 vulnerabilities, 1 allowed warning |
| `tools/enf2-symbol-check/check.sh` | exit 0 (`ENF-2 OK … 2 artifact(s) scanned`) |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` | exit 0 |

No manifest or `Cargo.lock` changed; no dependency delta.

## Unresolved issues

1. **Supply-chain gates are red; pre-existing, not caused by this change.**
   - `ammonia 4.1.4` (planeter's HTML sanitizer, `planeter-web`): **RUSTSEC-2021-0157** (CSS
     `url("")` escaping in `style` attributes) and **RUSTSEC-2021-0158** (XSS via SVG `animate*` tags);
     solution `>= 4.2.3` or `>= 4.1.7, < 4.2.0`.
   - `yoke-derive 0.8.3` is yanked (`cargo update -p yoke-derive`).
   - `sanitize_html` calls `ammonia::clean` (the default configuration). **I did not assess
     exploitability**; whether the default allowlist reaches the advisory conditions is the architect's
     call. Upgrading is a manifest change, which the handoff forbids, so I stopped. CI's
     `supply-chain` job will be red on `main` for the same reason. Not pushed on that account.
2. **Sandbox path binding (design note for RFC 004 v2).** The sandbox binds only the repository root
   read-write, so any artifact passed to `bundle export/import` or `sync accept` must live inside it
   (the tests copy files in). A transport design needs a staging path there.
3. **CONTRIBUTING's "shim prikk off PATH" recipe does not work as written here.** Symlinks to
   `cargo`/`rustc`/`rustup` resolve to rustup proxies, which re-add `~/.cargo/bin` to the child PATH, so
   the "shimmed" run still found prikk (no skips, 0.62 s). I used the real toolchain binaries
   (`rustup which cargo`) behind the shim, then saw all nine skips. The recipe should say so.

## Deviations from the handoff

- **Count.** T1 says "fourteen" ids; the driver reads thirteen (`diff-report-v1` is not read).
- **Test placement.** All new tests are under `tests/`, not inline, following
  `project-instructions-rust.md`. The existing inline `#[cfg(test)]` modules (25 source files, including
  `cli.rs` and `sandbox.rs`) were left alone; the one existing inline test that asserts the floor
  (`version_ordering_meets_the_floor`) was updated to 0.52.0. Whether to migrate the inline modules is
  still the architect's ruling.
- **Refactor.** The JSON parse step of `run_json` is now `parse_schema_json`, shared with `trust_check`
  (behaviour-preserving). `trust_check` avoids let-chains: the workspace MSRV is 1.85 and let-chains need
  1.88 (the local 1.99 compiler would not have caught it).
- **Records beyond the handoff's list:** `docs/dependency-policy.md` and two ledger notes (live floor
  statements); the STATUS "Active handoff" line and the IS-3 / IS-21 dispositions.
- **Left stale on purpose** (design set or history, "no design-set edits"):
  `planeter-04` PKI-1 (`MIN_PRIKK_VERSION = 0.46.0`), the charter history, `rfcs/README.md`, the 0.1.0
  `CHANGELOG` section, ROADMAP's historical 0.46.0 bullets.
- **Image bakes a different prikk.** The container now bundles 0.52.0 (was 0.46.0). Not built or run.

## Known limitations

- `trust_check` and `bundle_import` are not called outside the driver today; the changes are
  correctness work ahead of the transport.
- The seam sets no value: planeter still runs with prikk's defaults (256 MiB, 100,000 objects) until
  RFC 004 v2 chooses the accept-edge caps. IS-4 and IS-15 (concurrency and sandbox limits) are untouched.

## Floor recommendation (the owner decides the version)

Raise the floor to 0.52.0 **now** (done in the branch); release it as **0.2.1**, not a minor: no
capability, API or schema changes, and the operator-visible change is the prerequisite. The argument for
the tested version is the 0.47.0 PK-30 fix and accept bound, the 0.49.0 crash-state recovery and
dependency-ordered import/accept writes, and 0.52.0's archive closure fix; the image was baking none of
them. I recommend **not tagging until the ammonia advisories are resolved**: a sanitizer advisory is itself
a reason for an out-of-band patch release (ROADMAP §Release cycles), and a release with a red
`cargo audit` fails its own `release.yml` gates.

## Requested review focus

1. Confirm the `trust_check` rule ("exit 1 and a well-formed document saying `trusted: false`") is the
   intended reading of PK-42.
2. Decide the ammonia question (section "Unresolved", item 1) and whether it goes before the floor release.
3. Rule on the inline-test migration so the next handoff touching `cli.rs`/`sandbox.rs` is unambiguous.
4. Confirm the sandbox-binds-only-the-root consequence for RFC 004 v2 is already in the scoping notes.

## Artifacts

Raw per-verb JSON dumps and the gate logs are in the session scratchpad (not committed):
`/tmp/claude-1000/-home-nabbisen-Desktop-prikk-workspaces-planeter/f47f66a6-8273-4b14-8992-6e65b8b8a80c/scratchpad/t1/out/`
and `…/scratchpad/g-*.txt`.
