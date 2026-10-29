# RFC 010 follow-on — The image that is pushed must be the image that was run — Implementation Handoff (v1)

| | |
|---|---|
| Document | The image-glibc gate (D-51) runs the image it builds and checks the binary against its glibc before anything is pushed. Its review request states the one thing it does not prove: the pushed image comes from a **second** `buildx build` over the same inputs and cache, not the identical object. The first attempt to compare (the build metadata's config digest) failed because a manifest list carries none. For a release pipeline whose point is "what we tested is what we shipped" (owner: *robust*, *safe*), a gap that is *probably* never true is still a gap; close it by **checking the registry side**. |
| Status | **Assigned** (2026-10-30). Small. |
| Basis | `.github/workflows/release.yml` (jobs `image`: the build-and-load, run, check and push-by-digest steps; `image-publish`/`image-rehearsal`), `image-glibc-review-request.md` §4 and §8.1, `docs/RELEASING.md` ("The image gate"), `tools/pin-check`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — A post-push, pre-attest check of identity.** In the `image` job of the **rehearsal and release** modes (there is no push in a dry run, where the existing text of the docs stays true), after "Build and push by digest" and before the digest is handed on, resolve what was pushed and compare it with what was run: from the pushed digest read the index (`docker buildx imagetools inspect --raw ghcr.io/…@sha256:…` or the registry API with the job's token), select the manifest for `linux/${ARCH}` (ignore the attestation manifests whose platform is `unknown/unknown`), read **its config digest**, and require it to equal `IMAGE_ID` (the loaded image's id, `sha256:<64 hex>`, already checked by the run step). A mismatch fails the job, naming both ids. Put the comparison in a small tested script or in `tools/` (std-only, in the `glibc-check` style, with planted cases: equal passes, different fails, an index with no matching platform fails, only attestation manifests fails, malformed JSON is a usage error — never "ok"), and have `pin-check` require the step between the push and the artifact upload of the digest in the publishing jobs (planted-workflow tests as for the glibc gate).
- **T2 — If the two builds are not identical, say so and stop.** Run the **rehearsal** on a branch several times (at least three runs, not one) and report whether the config digests were equal each time. If they ever differ (for example because `apt-get install` ran twice against a moving mirror), **do not weaken the check**: report the cause and the evidence, and implement the alternative the architect then chooses; the candidate alternative is to build once, load, run, and push **that** image (`docker tag` + `docker push` of the loaded id, accepting that BuildKit's in-index provenance is replaced by the GitHub attestation the job already makes, and recording the one tag the push needs in the throwaway/real package) — report what it would change in the package before doing anything.
- **T3 — Documentation.** `docs/RELEASING.md`: the gate now proves identity (and how); the paragraph that says it does not is replaced; CHANGELOG; STATUS.

## Non-change scope

No change to the Dockerfile, the base, the binary or any crate; no real package is pushed (rehearsal path only; no tag, no release); no new action beyond what `pin-check` allows (digest-pinned).

## Acceptance criteria

T1's step and tool with tests and the `pin-check` rule with planted workflows; at least three rehearsal runs with the comparison's outcome reported per architecture; docs; every gate green (fmt, floor, pin, clippy, both test runs, deny and audit 0, ENF-2, doc), CI green and a dry-run of the release workflow green.

## Prohibited shortcuts

No comparison that can pass without reading the registry; no tolerance for a mismatch; no push to the real package; no unpinned action.

## Required evidence and review-request format

Commits (hash → what); the run links and per-run, per-architecture comparison (both ids); the tests and planted cases; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
