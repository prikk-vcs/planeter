# RFC 010 — Maintenance gates — Implementation Handoff (v2)

| | |
|---|---|
| Document | A small follow-on of the maintenance gates (v1, approved 2026-10-15): one compiler for CI and release, and a named runner image. Found by the dev team's review request (deviations 3 and 4) and ruled by the architect; RFC 010 D-5 is amended accordingly ("improvement on finding"). |
| Status | Inherited from RFC 010 — **Accepted**: assignable. |
| Basis | [`../../accepted/010-upstream-and-supply-chain-maintenance.md`](../../accepted/010-upstream-and-supply-chain-maintenance.md) D-5 (amended); issue register IS-26; `.github/workflows/ci.yml` (no toolchain step: it uses the runner's preinstalled Rust), `release.yml` (`toolchain: 1.99.0`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Purpose and background

CI is green on whatever compiler the runner image carries that day, while the release builds with the pinned
`1.99.0`: a pass on one is not a pass on the other. And `runs-on: ubuntu-latest` moves under us.

## Change scope

- **T1 — One compiler.** Add the same pinned `dtolnay/rust-toolchain` step (same SHA and tag comment as
  `release.yml`, `toolchain: 1.99.0`, `components: rustfmt, clippy`) to every job of `ci.yml` and
  `supply-chain.yml` that builds or installs Rust code. **Extend `planeter-pin-check`** to fail when the
  `toolchain:` inputs across `.github/workflows/*.yml` are not one and the same version, naming the files.
- **T2 — Named runner images.** Replace `ubuntu-latest` by `ubuntu-24.04` in every workflow (the aarch64
  job already names `ubuntu-24.04-arm`). **Extend `pin-check`** to fail on any `runs-on` ending in
  `-latest`.
- **T3 — Tests and documents.** Each new check has a planted-violation test and a passes-on-the-repository
  test, under `tests/`. `docs/dependency-policy.md` and `docs/RELEASING.md` state the two rules; CHANGELOG
  `[Unreleased]`; STATUS IS-26 closed.

## Non-change scope

No application code; no dependency change; no new action beyond the one already pinned; no change to
`cargo install cargo-deny cargo-audit` (current on purpose: the advisory database).

## Acceptance criteria

`pin-check` fails on a planted second toolchain version and on a planted `-latest`, and passes on the
repository; CI run on the branch is green with the pinned compiler (quote the run id); every gate exit 0,
both test runs named.

## Prohibited shortcuts

Do not add a version to `rust-toolchain.toml` as a substitute (it is a second source); do not silence the
new checks with an allow-list.

## Known risks

A job that relied on a preinstalled tool of the runner image (state any you find, with the fix).

## Required evidence and review-request format

Commits; every gate with command and exit code; the planted-violation outputs; the CI run id; deviations.
