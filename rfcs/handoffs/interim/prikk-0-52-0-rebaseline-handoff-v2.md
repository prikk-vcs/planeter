# Interim — prikk 0.52.0 re-baseline — Implementation Handoff (v2)

| | |
|---|---|
| Document | Handoff for the per-prikk-release re-baseline (ROADMAP §Release cycles: "planeter re-baselines against each prikk release", owner-confirmed 2026-09-23), instance **0.52.0**. It supersedes [`prikk-0-47-0-rebaseline-handoff-v1.md`](prikk-0-47-0-rebaseline-handoff-v1.md), which targeted 0.47.0 and stays for its reasoning. Supporting work without an RFC of its own (workflow §6.6). |
| Status | **Next assignment — assignable: the precondition was met on 2026-10-15** (prikk 0.52.0 is installed locally, `prikk --version` = 0.52.0). The version-pinned tests must run against the real binary, never be skipped into a false green: the version-pinned tests must run against the real binary, never be skipped into a false green. |
| Basis | Dependency ledger v1.5 rows **PK-18** (drift), **PK-26** (floor), **PK-30** (explicit `--ref` on an unsealed repo, fixed 0.47.0), **PK-31** (accept size bound), **PK-40** (bounds and memory), **PK-42** (items for the floor), **PK-43** (schema drift 0.47–0.52 not verified); `docs/UPSTREAM.md` §Re-baseline procedure; RFC 001 D-3 (version pin) and D-2 (the driver keys on exit status, never text); issue register IS-3, IS-21. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | The prikk boundary crate (`planeter-prikk`) and the records that pin the prikk version. **Non-change scope:** no transport code (M2 is held), no archive code, no new dependencies, no design-set edits, no change to any `schema_version` string planeter expects unless 0.52.0 changed it (then: stop and file the drift, T1). The measurement spike for the transport design is a **separate** handoff. |

## Why this release matters to planeter

- Four releases passed since v1 was written (0.47.0 to 0.52.0). No letter reports a JSON schema change, and
  **nobody has verified one** (PK-43): T1 is the first real check.
- **PK-30** (explicit `--ref` on an unsealed repository) is fixed since 0.47.0; planeter reads default to the
  bare form, so 0.2.0 is unaffected, but the trap is gone and deserves a test.
- **PK-31**: `sync accept` refuses an over-size artifact before reading it (exit 1, new wording, bound
  `PRIKK_EXCHANGE_MAX_BYTES`). The sandbox clears the environment, so planeter must set bounds explicitly
  when transport lands; this handoff wires the **seam** only.
- **PK-42, 0.52.0:** `trust maintainer check` **exits 1 for "not trusted"** (it exited 0 either way).
  `planeter-prikk::trust_check` keys on exit status, so this is a behavioural break to handle (T5, IS-21).
- **0.49.0–0.52.0 fixes that matter to a forge** (floor argument, T6): refusal of bundles whose history cannot
  be checked, dependency-ordered writes in `bundle import` and `sync accept`, linear resync over hostile
  content (a quadratic `verify` cost on hostile containers before 0.49.0), `ref complete`, the recovery log.

## Task breakdown (build in this order)

- **T1 — Verify the JSON schemas planeter reads (PK-18, PK-43).** With 0.52.0 on `PATH`, run the driver and
  read-path suites (`cargo test -p planeter-prikk -p planeter-core --locked`). The driver refuses any
  `schema_version` other than the expected one, so drift fails loudly. The fourteen expected ids:
  `log-report-v1`, `show-report-v1`, `verify-report-v1`, `branch-list-v1`, `tag-list-v1`,
  `status-report-v1`, `worktree-status-report-v1`, `patch-plan-content-v1`, `tree-listing-v1`,
  `path-content-v1`, `key-status-v1`, `trust-list-v1`, `trust-check-v1`, `diff-report-v1` (where read).
  Also record, per id, whether the *fields* planeter deserializes are unchanged (a changed field set with
  an unchanged id is drift too). **If any id or field shape changed: stop, record the exact difference and
  hand back**: accepting a schema is the architect's call.
- **T2 — PK-30 regression test.** A version-pinned driver test: on a fresh, unsealed repository an explicit
  `--ref heads/main` on `tree` and `log` answers identically to the bare form on 0.52.0. Guard it on the
  detected version so a 0.46.0 host skips it **visibly** (the skip is printed).
- **T3 — Exchange-bound seam (PK-31, PK-40).** The sandbox (`planeter-prikk::sandbox`) passes only `PATH`
  after `--clearenv`. Add a typed knob on the sandbox profile for `PRIKK_EXCHANGE_MAX_BYTES`,
  `PRIKK_EXCHANGE_MAX_OBJECTS`, `PRIKK_BUNDLE_MAX_BYTES` and `PRIKK_BUNDLE_MAX_OBJECTS`, each emitted as
  `--setenv NAME <n>` only when set, with a unit test on the built argument vector. **Do not wire a value
  from the binary yet** (the accept-edge cap is RFC 004 v2's). Non-change: no behaviour change for reads.
- **T4 — `bundle_import --input` (IS-3).** The driver passes the artifact positionally; prikk ≥ 0.46.0
  takes `--input <path>`. Fix the argument vector and add a version-pinned round trip: `bundle export` from
  a sealed fixture repository → `bundle import --input` into a fresh one → the ref is visible under
  `remotes/` (PK-4). The refusal path keys on exit status only. Note: since 0.49.0 import refuses a bundle
  whose history it cannot check; the fixture must carry its full closure.
- **T5 — `trust_check` and the 0.52.0 exit code (IS-21, PK-42).** Measure on 0.52.0, for a key that is
  adopted and one that is not: the exit status and whether `trust-check-v1` JSON is still written to stdout
  on exit 1. Then make the driver return a typed `TrustCheck { trusted: false }` for the "not trusted"
  answer **if and only if the JSON is present and well-formed**, keeping exit status the only signal for
  real failures (unreadable policy, usage). Support 0.46.0's behaviour (exit 0) as well while the floor
  allows it. Required tests: both outcomes on 0.52.0; the 0.46.0 shape from a recorded fixture. **If the
  JSON is not written on exit 1: stop and hand back** (the architect decides whether to ask prikk).
- **T6 — Pin, floor and records.** Raise `MIN_PRIKK_VERSION` to `(0, 52, 0)` **only if T1–T5 are green on
  the 0.52.0 binary** (the architect's recommendation: the floor is the tested version, as for 0.46.0; the
  0.49.0–0.52.0 fixes above are the argument, and the image currently bakes 0.46.0). Keep the three floors
  identical: `crates/planeter-prikk/src/cli.rs`, `PRIKK_VERSION` in `.github/workflows/release.yml`,
  `ARG PRIKK_VERSION` in `Dockerfile` (checksum-verified asset); update the "≥ 0.46.0" mentions in README,
  RELEASING, ROADMAP (§Runtime prerequisites), STATUS and CHANGELOG `[Unreleased]`. Ledger: bump to **v1.7** (v1.6 already records prikk's plan letter of 2026-10-15),
  promoting PK-30, PK-31 and PK-43 to **CONFIRMED (measured on 0.52.0)** with the test names as evidence,
  and recording T5's measurement against PK-42. **The floor bump is a release question** (0.2.1 or the next
  minor): recommend in the review request; the owner decides the version.

## QA checklist

- [ ] T1 ran against `prikk --version` = 0.52.0 (quote the line); every expected id matched, or the drift
      is recorded and the handoff stopped at T1.
- [ ] T2 passes on 0.52.0 and skips visibly on 0.46.0 (both outputs quoted).
- [ ] T3: the argument vector contains each `--setenv` only when its knob is set; read verbs unchanged.
- [ ] T4 round trip passes; the refusal path is exit-status keyed.
- [ ] T5: the measurement is quoted; both outcomes are tested.
- [ ] Gates: `cargo fmt --all --check` · `cargo clippy --workspace --all-targets --locked -- -D warnings` ·
      `cargo test --workspace --locked` **with** prikk on `PATH` · the same **with prikk shimmed off
      `PATH`** (CI has no prikk) · `cargo deny check` · `cargo audit` · `tools/enf2-symbol-check/check.sh`
      · `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
- [ ] No manifest changed (no dependency delta to report). If one had to: stop and hand back.
- [ ] The three floors are identical and every "≥ 0.46.0" mention is updated (grep for `0.46.0`).

## Definition of done & handback

Done when the checklist is complete, the ledger is v1.7 and `docs/STATUS.md` records 0.52.0 as
re-baselined. The review request carries: commits (hash → what), every gate's command and exit code (both
test runs named), T1's schema verdict per id and per deserialized field set, T2/T4/T5 outputs on the real
binary, the floor-bump recommendation with its rationale, deviations and open questions. Nothing here
authorizes a tag, a publish, or a letter to the prikk team; if T1 or T5 finds drift, the architect drafts
the letter.
