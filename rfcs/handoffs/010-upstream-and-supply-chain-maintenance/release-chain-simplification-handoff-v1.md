# RFC 010 (amendment D-9…D-11) — A release chain that trusts only GitHub-owned actions — Implementation Handoff (v1)

| | |
|---|---|
| Document | Implements RFC 010's amendment **D-11** (with **D-9**'s procedure for the pins that remain), behind the rehearsal built by [`release-rehearsal-handoff-v1.md`](release-rehearsal-handoff-v1.md) (done, approved 2026-10-21). It retires the five lagging pins and the toolchain action instead of updating them (IS-35). |
| Status | **Done, reviewed Approved (2026-10-21; commits `17a07d0`…`ed7e3b2`).** IS-35 closed. |
| Basis | RFC 010 amendment D-9, D-10, D-11; the dev team's rehearsal findings (review request `release-rehearsal-review-request.md`: runners have Docker 28.0.4 and buildx v0.38.0 with the `docker` driver on both architectures; the job token creates a private package; `actions/attest` v4.2.2 is the form for new use, SHA `1e69f48acb82d1966a394da916b4c1698aa569d6`, verified by the dev team as the upstream tag's commit); `.github/workflows/{release,ci,supply-chain}.yml`; `Dockerfile`; `tools/pin-check`; `docs/RELEASING.md`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Target state (the end of this handoff)

The only `uses:` in any workflow are from the `actions/` owner (`checkout`, `upload-artifact`, `download-artifact`,
`attest`). The Rust toolchain is installed by the runner's own `rustup` at the pinned version. The image is built
natively per architecture with the runner's `docker`/`buildx` and joined into one manifest list; there is no QEMU
and no `docker/*` or `dtolnay/*` action. Image content, tags and attestations are what the current chain
produces.

## Change scope — in this order, each change its own commit, each followed by a rehearsal run

- **T1 — The toolchain by `rustup`.** Replace `dtolnay/rust-toolchain` in `ci.yml` (all jobs, including `msrv`),
  `release.yml` (gates and both build jobs) and `supply-chain.yml` with a run step that installs the pinned
  version with the runner's `rustup` (`--profile minimal` plus `rustfmt` and `clippy` where the job needs them,
  `--no-self-update`, then make it the default). Extend `pin-check` so it reads the version from that command exactly
  as it reads `toolchain:` today (one build compiler across workflows; the `msrv` job exception; a channel or
  `latest` refused), with planted-failure tests. Check that the arm runner has `rustup`; report if not.
- **T2 — The attestation action.** Replace both `actions/attest-build-provenance` uses (tarballs, image; and the
  rehearsal copies) with `actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6 # v4.2.2` (re-verify that SHA is
  the upstream tag's commit and still the newest release before you use it, per D-9, and record the check).
  Keep `subject-path` for tarballs; for the image `subject-name`, `subject-digest`, `push-to-registry: true`. Set
  **`create-storage-record: false`** (least privilege; do not add `artifact-metadata: write`) unless the rehearsal
  shows the action cannot work without it, in which case stop and report. The attestation must still verify with
  `gh attestation verify` for tarballs, and the image's record must exist for the manifest-list digest.
- **T3 — The container job, natively.** Build each image architecture on its own runner (the existing x86_64 and
  `ubuntu-24.04-arm` build jobs, or a matrix over the same runners), without QEMU, and publish one manifest list with
  the tags the real release uses. Logging in is `docker login ghcr.io -u "$GITHUB_ACTOR" --password-stdin` with
  the job token piped on standard input (never on the command line, never echoed). The builder: **prefer** a
  `docker-container` buildx builder whose BuildKit image is referenced **by digest** (a new outside artifact,
  pinned like the base image, and checked by `pin-check`), with each architecture pushed **by digest** and joined
  by `docker buildx imagetools create`; the alternative is per-architecture temporary tags merged by
  `imagetools create`, which leaves extra tags in the package and is acceptable only if the preferred route
  fails in the rehearsal (say why). Decide by measurement in the rehearsal and report both attempts. The result
  must show: a manifest list with `linux/amd64` and `linux/arm64`; the same `Dockerfile` and build args; BuildKit's
  own provenance/SBOM attestations at least as present as before (state what the old chain attached and what the
  new one does); the image attestation by `actions/attest` for the **manifest-list digest**; the real path
  unchanged in what it publishes (tags, `latest`). Remove the rehearsal-only measurement steps added for T4 of the
  rehearsal handoff (they have served).
- **T4 — The gate.** `pin-check` fails on any `uses:` whose owner is not `actions/`; on a `docker-container`
  builder image or any `docker run`/`docker pull` of an image not referenced by `sha256:` digest; and on the
  token appearing in a command line (`--password` rather than `--password-stdin`). Planted-failure tests.
  Keep the rehearsal rules of the previous handoff (rehearsal never reaches the real image).
- **T5 — Rehearse and show.** After each of T1, T2 and T3 and after the whole: a `mode: rehearsal` run, green,
  with the evidence of the previous handoff (rehearsal package updated, manifest list shape, attestations created,
  nothing else published); a `mode: dry-run` once at the end; CI green. If you have a token with `read:packages`,
  verify the rehearsal image attestation with `gh attestation verify oci://…`; if not, say so as before.
- **T6 — Documentation.** `docs/RELEASING.md` (what the chain trusts, the rehearsal rule of D-10 now covers T1–T3
  changes, how a pin or the BuildKit digest is updated per D-9), `docs/dependency-policy.md`, the workflow header
  comments, `docs/UPSTREAM.md` if it lists the actions, STATUS IS-35, the dependency ledger if it lists the
  actions, CHANGELOG only if an operator-visible fact changes (it should not).

## Non-change scope

No change to what a release publishes (tarball names, the image's tags, build-info, release notes); no change to
`Dockerfile` content or its base-image digest; no tag, no release, no crates.io action; no new secret or token; no
`actions/checkout`/`upload-artifact`/`download-artifact` change except through D-9's procedure if you find a newer
release with a reason to take it (otherwise leave them).

## Acceptance criteria

The target state holds (`grep` the workflows for `uses:` and show the list); every rehearsal run green with the
evidence; the manifest list shape and the tag set equal the current real path's; `pin-check` rules and their
planted-failure tests; all gates green including both supply-chain gates (fresh advisory database), both test runs
named, ENF-2.

## Prohibited shortcuts

No action outside `actions/`; no tag-based or mutable reference to the BuildKit image or any other image or
action; no weakening of a `pin-check` rule to pass; no token on a command line; no pin update without D-9's
record; no first run of any new step on a real tag; no change to the real release steps that the rehearsal did
not first exercise.

## Known risks

Shell steps are new code in a security-sensitive workflow: keep them short, `set -euo pipefail`, quote every
variable, no `eval`. The rehearsal records accumulate (labelled, accepted). If the preferred builder route fails
and the fallback leaves extra tags in the real package on a real release, that must be reported before use.

## Required evidence and review-request format

Commits (hash → what, one per T); the run ids of each rehearsal and of the dry run; the manifest-list shape
(`docker buildx imagetools inspect` output from a runner log); the attestation evidence; the D-9 records for each
SHA or digest introduced; the planted-failure outputs; the final `uses:` list; the old-versus-new statement of what
BuildKit attaches; every gate with command and exit code and the advisory-database HEAD; deviations and open
questions.
