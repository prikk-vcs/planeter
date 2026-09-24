# Interim — prikk 0.47.0 re-baseline — Implementation Handoff (v1)

| | |
|---|---|
| Document | Handoff for the per-prikk-release re-baseline (ROADMAP §Release cycles: "planeter re-baselines against each prikk release", owner-confirmed 2026-09-23), instance **0.47.0**. Supporting work without an RFC of its own (workflow §6.6). |
| Status | **Next assignment — held until prikk 0.47.0 is released and installed locally** (owner ruling 2026-09-24: the new team's first task). Not assignable before that: the version-pinned tests must run against the real binary, never be skipped into a false green. |
| Basis | Dependency ledger v1.4 rows **PK-18** (JSON drift is a version-gated risk), **PK-26** (the floor), **PK-30** (0.45/0.46 regression, fixed 0.47.0), **PK-31** (0.47.0 accept size bound); `docs/UPSTREAM.md` §Re-baseline procedure; RFC 001 D-3 (version pin) and D-2 (the driver keys on exit status, never text); issue register IS-3. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | The prikk boundary crate (`planeter-prikk`) and the records that pin the prikk version. **Non-change scope:** no transport code (M2 is held), no new dependencies, no design-set edits, no change to any `schema_version` string planeter expects unless 0.47.0 changed it (then: stop and file the drift, T1). |

## Why this release matters to planeter

- **PK-30** is a read-path bug in the exact state every planeter-created repository is in (nothing sealed
  until RFC 154 adoption): naming the branch explicitly on `tree`/`log`/`cat`/`worktree-status`/`diff`/
  read-only `checkout` was refused on 0.45/0.46. planeter's reads default to the bare form, so 0.2.0 is
  correct today, but the fix removes a trap for any future explicit-ref read.
- **PK-31** is the first RFC 158 stage: `sync accept` refuses an over-size artifact **before reading it**
  (0.46.0 read the whole file first — an OOM kill on a large enough push). The exit code stays 1; the
  wording changes; the bound is `PRIKK_EXCHANGE_MAX_BYTES` (default 256 MiB). The sandbox clears the
  environment, so planeter must set it explicitly when transport lands — this handoff wires the seam now.

## Task breakdown (build in this order)

- **T1 — Verify the JSON schemas planeter reads (PK-18).** With prikk 0.47.0 on PATH, run the driver
  and read-path suites (`cargo test -p planeter-prikk -p planeter-core --locked`). The driver refuses
  any `schema_version` other than the expected one, so drift surfaces as a test failure, not a silent
  change. The fourteen expected ids: `log-report-v1`, `show-report-v1`, `verify-report-v1`,
  `branch-list-v1`, `tag-list-v1`, `status-report-v1`, `worktree-status-report-v1`,
  `patch-plan-content-v1`, `tree-listing-v1`, `path-content-v1`, `key-status-v1`, `trust-list-v1`,
  `trust-check-v1`, plus `diff-report-v1` where the driver reads it. **If any id or field shape changed:
  stop, record the exact difference, and hand it back** — accepting a new schema is the architect's
  call (it may change a view-model). Non-change: the ids themselves.
- **T2 — PK-30 regression test.** Add a version-pinned driver test: on a fresh, unsealed repository,
  an explicit `--ref heads/main` on `tree` and `log` answers identically to the bare form on 0.47.0
  (on 0.46.0 it was refused). Guard it on the detected version so 0.46.0 hosts skip it *visibly*
  (the skip is printed, never silent). Required test: the new one; existing ones unchanged.
- **T3 — `PRIKK_EXCHANGE_MAX_BYTES` seam (PK-31).** The sandbox (`planeter-prikk::sandbox`) passes
  only `PATH` after `--clearenv`. Add a typed knob on the sandbox profile for the exchange byte bound,
  passed as `--setenv PRIKK_EXCHANGE_MAX_BYTES <n>` when set, and a unit test on the built argument
  vector. **Do not wire a value from the binary yet** — the accept-edge cap is RFC 004 D-7's and comes
  with the v2 transport handoff; this task only makes the seam exist and tested. Non-change: no
  behavioural change for read verbs.
- **T4 — `bundle_import --input` (IS-3).** The driver passes the artifact positionally; prikk ≥ 0.46.0
  takes `--input <path>`. Fix the argument vector and add a version-pinned round-trip test:
  `bundle export` from a sealed fixture repository → `bundle import --input` into a fresh one → the
  imported ref is visible under `remotes/` (PK-4). The refusal path keys on exit status only (D-2).
- **T5 — Pin, floor and records.** Bump `MIN_PRIKK_VERSION` to `(0, 47, 0)` **only if T1–T4 are green
  on the 0.47.0 binary** (the pin refuses out-of-range binaries, so the binary must be present — ROADMAP
  rule). Then keep the three floors identical: `crates/planeter-prikk/src/cli.rs`, `PRIKK_VERSION` in
  `.github/workflows/release.yml`, `ARG PRIKK_VERSION` in `Dockerfile`; update the "≥ 0.46.0" mentions
  in README, RELEASING, ROADMAP (§Runtime prerequisites), CHANGELOG `[Unreleased]` ("prikk floor →
  0.47.0: PK-30 fix, accept size bound"), and `docs/STATUS.md`. Ledger: bump to **v1.5** — PK-30 and
  PK-31 move to **CONFIRMED (measured on 0.47.0)** with the test names as evidence; PK-18 gains the
  line "0.47.0: no `schema_version` change" (or the drift found in T1). **The floor bump is a release
  question** (0.2.1 or the next minor) — recommend in the review request; the owner decides.

## QA checklist

- [ ] T1 ran against `prikk --version` = 0.47.0 (quote the line) and every expected id matched, or the
      drift is recorded and the handoff stopped at T1.
- [ ] T2 test passes on 0.47.0 and skips visibly on 0.46.0 (both outputs quoted).
- [ ] T3: the argument vector contains `--setenv PRIKK_EXCHANGE_MAX_BYTES <n>` only when the knob is
      set; read verbs unchanged (existing sandbox tests still pass).
- [ ] T4 round-trip passes; the refusal path is exit-status keyed.
- [ ] Gates: `cargo fmt --all --check` · `cargo clippy --workspace --all-targets --locked -- -D warnings`
      · `cargo test --workspace --locked` **with** prikk on PATH · the same **with prikk shimmed off
      PATH** (CI has no prikk) · `cargo deny check` · `cargo audit` · `tools/enf2-symbol-check/check.sh`
      · `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
- [ ] No manifest changed (no dependency delta to report). If one had to: stop and hand back.
- [ ] The three floors are identical and every "≥ 0.46.0" mention is updated (grep for `0.46.0`).

## Definition of done & handback

Done when the QA checklist is complete, the ledger is v1.5 and `docs/STATUS.md` records 0.47.0 as
re-baselined. The review request carries: commits (hash → what), every gate's command and exit code
(both test runs named), T1's schema verdict per id, T2/T4 outputs on the real binary, the floor-bump
recommendation with its rationale, deviations and open questions. Nothing here authorizes a tag, a
publish, or a letter to the prikk team; if T1 finds drift, the architect drafts the letter.
