# RFC 010 — Maintenance gates — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 010 decisions D-2 (one floor, checked), D-3 (scheduled supply-chain gate) and D-5 (pinned build chain). |
| Status | Inherited from RFC 010 — **Proposed**: not assignable until the owner accepts it. |
| Basis | [`../../proposed/010-upstream-and-supply-chain-maintenance.md`](../../proposed/010-upstream-and-supply-chain-maintenance.md) D-2, D-3, D-5; `docs/dependency-policy.md` rules 1, 3, 6; threat model C-5d, C-11, INV-7; issue register IS-17, IS-22. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

Three facts (RFC 010 F1–F3): CI runs only on push and pull request, so advisories that arrive without a code
change go unseen; the prikk floor lives in three files and moves together only by procedure; and our
workflows pin third-party actions by mutable tag. This handoff turns each into a gate that fails by
construction.

## Change scope

- **T1 — `tools/floor-check`.** A std-only tool (like `tools/boundary-check`), a workspace member with its
  tests under `tests/`. It reads `MIN_PRIKK_VERSION` from `crates/planeter-prikk/src/cli.rs`,
  `PRIKK_VERSION` from `.github/workflows/release.yml`, and `ARG PRIKK_VERSION` from `Dockerfile`; exits 0
  only if all three are the same `X.Y.Z`; otherwise exits 1 and names each file and value. Wire it into
  `ci.yml` and the `gates` job of `release.yml`.
- **T2 — `tools/pin-check`.** Fails on any `uses:` in `.github/workflows/*.yml` that is not a full 40-hex
  commit SHA (a local `./` action is allowed), on a `FROM` without `@sha256:`, and on a toolchain reference
  that is a channel (`stable`, `beta`, `nightly`) rather than a version. Names file and line. Wired like T1.
- **T3 — Pin the chain.** Replace every mutable reference by its commit SHA with the human tag in a trailing
  comment, the base image by digest, and the toolchain by an explicit version that satisfies MSRV 1.85.
  **Report, per action, the upstream tag and the SHA you resolved it to**; do not guess a SHA.
- **T4 — Scheduled audit.** A new workflow `supply-chain.yml`: `schedule` (weekly) and `workflow_dispatch`;
  `permissions: contents: read`; installs `cargo-deny` and `cargo-audit` as `ci.yml`'s job does and runs
  both against a freshly fetched advisory database.
- **T5 — Documents.** `docs/UPSTREAM.md` §Re-baseline and `docs/dependency-policy.md` cite RFC 010 as their
  authority; `docs/RELEASING.md` names the two gates; CHANGELOG `[Unreleased]` records them.

## Non-change scope

No application code; no manifest or `Cargo.lock` change; no `deny.toml` or `.cargo/audit.toml` change; no
other action upgrades than the pin itself (resolve the SHA of the tag **currently** referenced).

## Required tests

Each tool: one test that plants a violation and expects exit 1 with the expected message, one that expects
exit 0 on the real repository. The planted fixtures are temporary directories, not files in the repository.

## Acceptance criteria

`floor-check` and `pin-check` pass on `main` and fail on their planted violations; `ci.yml` runs both;
`release.yml` runs both in `gates`; `supply-chain.yml` exists and a manual `workflow_dispatch` run is green
(quote the run id); `pin-check` finds no mutable reference anywhere.

## Prohibited shortcuts

Do not silence `pin-check` with an allow-list of mutable references; do not pin to a SHA you did not resolve
from the tag; do not widen any workflow `permissions`; do not add a third-party action to implement a gate.

## Security and compatibility constraints

The scheduled workflow has no secrets and `contents: read` only. The release workflow's artifacts must be
byte-for-byte the product of the same steps (pins change *which* code runs the steps, not the steps).

## Known risks

A SHA that resolves to a different action than the tag intended (check the tag's release page); a pin that
breaks a step (run the release workflow as a dry run on a branch with publishing disabled, or state that it
could not be exercised before a real tag).

## Required evidence and review-request format

Commits (hash → what); every gate's command and exit code (including both test runs and both new gates);
the table of pins (action, tag, SHA); the planted-violation outputs; the `workflow_dispatch` run id;
deviations and open questions. Nothing here authorizes a tag.
