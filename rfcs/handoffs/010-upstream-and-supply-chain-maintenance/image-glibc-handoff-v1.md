# RFC 010 follow-on — The container image must be able to run the binary it carries — Implementation Handoff (v1)

| | |
|---|---|
| Document | **A release defect found by the aws-lc switch's review (D-51).** The release binaries are built on `ubuntu-24.04` runners (glibc 2.39) and copied into an image whose base is `debian:bookworm-slim` (glibc 2.36). The dev team read the ELF version-need tables of the rehearsal artifacts (`advisory-aws-lc-switch-logs/t4/ci/verneed-*.txt`): the **x86_64 binary needed nothing newer than GLIBC_2.34 before the switch and needs GLIBC_2.38 after it** (`__isoc23_strtol`, `__isoc23_sscanf` from `aws-lc-sys`, compiled against glibc ≥ 2.38 headers), and the **aarch64 binary already needed GLIBC_2.38/2.39 before**. Either way the dynamic loader on bookworm would refuse to start the binary. It was derived from the tables, not observed (no container runtime in the dev environment). Nothing in the release workflow *runs* the image, which is why a broken image could be built, attested and pushed. Owner's terms: *robust* — a release artifact that cannot start is the worst kind of failure, and the pipeline must make it impossible to ship; *safe/secure* — move to a base that still receives security updates for the whole life of the release. |
| Status | **Assigned** (2026-10-30). Release-critical: no release is tagged before this is done. |
| Basis | `Dockerfile` (base `debian:bookworm-slim@sha256:7c7b2c96…`, `PRIKK_VERSION` 0.54.0), `.github/workflows/release.yml` (jobs `image`, `image-publish`; native `ubuntu-24.04` and `ubuntu-24.04-arm` builders; `mode: rehearsal`), `tools/pin-check` (digest pins, the rehearsal-only registry rule), RFC 010 D-5, D-9, D-10, D-11; `docs/RELEASING.md`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — Observe first (no change).** Run the **rehearsal** of the release workflow on the current `main` and add, in a throwaway branch, one step per image job: after the image is built for its native architecture, **run it**: `docker run --rm --platform linux/<arch> <image> --version` (and `prikk --version` through `--entrypoint`). Report the real result for both architectures on the current bookworm base (the expected failure is `version 'GLIBC_2.38' not found`; say what it actually is). This turns the derived claim into an observed one.
- **T2 — The base.** Change the base to **`debian:trixie-slim`** (Debian 13, glibc 2.41; current stable, supported longest) pinned **by digest** (a multi-arch index digest; obtain it from the registry and put the human tag in a comment, as `pin-check` requires), and keep the `apt-get install` of `bubblewrap ca-certificates curl` and the `prlimit --version` build check. Re-verify the baked prikk release (`PRIKK_VERSION`, checksum-verified) runs on it, the sandbox prerequisites exist (bubblewrap's version on trixie; `prlimit`), and note any package version change that matters (`curl`, `ca-certificates`) in the report. Update `docs/RELEASING.md` (the base, the user-namespace notes if bubblewrap's behaviour differs).
- **T3 — The gate that makes it impossible to ship a dead image (permanent).** In `release.yml`, in **every mode** (dry-run, rehearsal and release), after each architecture's image is built and **before** it is pushed or attested: (a) run it as in T1 and require exit 0 and the expected `planeter X.Y.Z` line; (b) a static check as a second, cheaper line of defence: a small script `tools/glibc-check` (Rust or shell, in the pin-check style, with its own tests and planted cases) that reads the highest **strong** `GLIBC_` version need of the binary (`objdump -T` or `readelf -V`; weak symbols reported separately) and fails if it exceeds the base image's glibc (read from the image: `ldd --version` output in a `docker run`, not a constant), run in the same job. Both steps are covered by `pin-check` (it must fail if a push or attestation step is not preceded by them).
- **T4 — Consequences for what was already published.** State, from the observed T1 result, whether the **0.3.0 and 0.4.0 images on ghcr** (private) were runnable (x86_64 before the switch needed 2.34 so bookworm should have run it; aarch64 may not have been): read their version-need tables from the release tarballs (the releases keep the binaries) and run the old images if the registry is reachable with your credentials; if not reachable, say so. This is information for the architect's note to the owner; do not republish anything.
- **T5 — Documentation.** CHANGELOG (operator-visible: the image base changes to trixie), `docs/RELEASING.md`, STATUS, the dependency policy's note on the base image.

## Non-change scope

No change to the binary or to any crate; no release, tag, or push of a real package (rehearsal path only, as RFC 010 D-10 allows); no change of the release targets; no new action beyond what `pin-check` already allows (pin any new `uses:` by digest).

## Acceptance criteria

T1's observed result for both architectures on bookworm and on trixie; T2's pinned base; T3's two steps in every mode with `pin-check` coverage and tests (a planted workflow without the step fails `pin-check`); a successful **rehearsal** run on the branch showing both images started and printed their version; T4's statement; every gate green (fmt, floor, pin, clippy, both test runs, deny and audit both 0, ENF-2, doc), CI green.

## Prohibited shortcuts

No claim that an image runs without running it; no unpinned base; no pushing before the run step; no silencing of a weak-symbol finding without reporting it.

## Required evidence and review-request format

Commits (hash → what, by task); the rehearsal run links and per-architecture results; the version-need tables before/after; the digest and where it came from; the T4 statement; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
