# RFC 010 follow-on — A declared minimum compiler that is true and checked — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** handoff of the accepted RFC 010 (D-5: the build chain is pinned and nothing in it is mutable; D-1/D-2 floors are checked). It corrects a finding of the maintenance-gates v2 review: **IS-32**. |
| Status | **Held for the owner's decision D-13** (SCHEDULE; include in release 0.3.0). |
| Basis | RFC 010 D-5 (amended); the maintenance-gates v2 review request, note 8; `Cargo.toml` (`rust-version = "1.85"`, edition 2024); let-chains in `planeter-web/src/principal.rs`, `planeter-core/src/egress.rs`, `planeter-auth/src/oidc.rs`, `planeter-auth/src/throttle.rs` (verified by the architect). |
| Audience | Dev team. |

## Purpose and background

The workspace declares `rust-version = "1.85"` but uses let-chains, which need a later compiler. Nothing
builds with 1.85, and no gate would notice. A declared minimum that is false makes `cargo install planeter`
fail with a compiler error instead of a clear message, and the claim has no evidence behind it.

## Change scope

- **T1 — Measure.** Find the lowest stable compiler that builds the whole workspace with `--locked`
  (start at 1.88, where let-chains are stable, and step up). Report the version and, if a dependency raises
  it above the language need, which one.
- **T2 — Declare it.** Set `rust-version` (workspace) to that version. Edit nothing else in the manifests.
- **T3 — Check it.** One CI job `msrv`: install exactly that version through the pinned toolchain step
  and run `cargo check --workspace --locked`. The job's version must come from one place: `pin-check`
  (or `floor-check`) fails when the job's toolchain differs from `rust-version`. If a second toolchain
  version in the workflows conflicts with the "one compiler" rule of pin-check, make the rule's exception
  explicit and named (the MSRV job), not implicit.
- **T4 — Tests** for the new check under `tools/*/tests/`: a planted mismatch fails and names the files.

## Non-change scope

No code change to make an older compiler work (we do not rewrite let-chains); no dependency change; no
change to the pinned build compiler (1.99.0).

## Required documentation updates

`docs/dependency-policy.md` (what the declared minimum means and what proves it), `docs/RELEASING.md`
(the checklist), CHANGELOG `[Unreleased]` (**the declared minimum compiler rises**), STATUS IS-32 closed.

## Acceptance criteria

`msrv` job green in CI on the commit; `pin-check` red on a planted mismatch; every other gate green,
including both supply-chain gates.

## Prohibited shortcuts

Do not declare a version that was not built; do not "fix" the declaration by deleting the field; do not
pin the MSRV job to `stable` or `latest`.

## Alternative recorded (not recommended)

Declare `rust-version = 1.99.0` (the one tested compiler) and add no MSRV job. Simplest and true, but it
refuses toolchains that would in fact build; the architect recommends T1–T3.

## Required evidence and review-request format

The measurement (which versions failed, how); commits; every gate with command and exit code; the CI run id
for `msrv`; deviations.
