# RFC 010 (amendment D-9…D-11) — Release rehearsal mode — Implementation Handoff (v1)

| | |
|---|---|
| Document | Implements RFC 010's amendment of 2026-10-20, **D-10** (a release rehearsal) on the **current chain and pins**, and measures the three unknowns the amendment names. It is the first of two handoffs for IS-35; the second (D-11, the simplified chain) follows after this is reviewed, and is built and checked behind this rehearsal. |
| Status | **Assigned** (2026-10-20), after the owner accepted D-9 to D-11 (O-4) and the rehearsal's footprint (O-5). |
| Basis | [`../../accepted/010-upstream-and-supply-chain-maintenance.md`](../../accepted/010-upstream-and-supply-chain-maintenance.md) "Proposed amendment (2026-10-20)" (now accepted) D-9, D-10, D-11; `.github/workflows/release.yml` and its dry-run mode ([`maintenance-gates-handoff-v2.md`](maintenance-gates-handoff-v2.md)); `tools/pin-check`; `docs/RELEASING.md`; the 0.3.0 and 0.4.0 tag runs. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — A `mode` input.** `release.yml`'s `workflow_dispatch` takes `mode` (a choice: `dry-run`, the default,
  and `rehearsal`) in place of the boolean `dry_run`. A tag push is unchanged. A dispatch is refused unless it
  runs from `main`. Update RELEASING and the existing dry-run documentation.
- **T2 — What the rehearsal does.** The gates; both tarball builds; the image build; then, **only in rehearsal**,
  `docker/login-action` to ghcr, a build-and-push of the image to **`ghcr.io/prikk-vcs/planeter-rehearsal:rehearsal`**
  (one tag, overwritten each time), and an **attestation of that image** (subject name
  `ghcr.io/prikk-vcs/planeter-rehearsal`). Tarballs: attest **copies renamed `rehearsal-<name>`** (not the release
  file names) so the permanent records are labelled; those copies are not uploaded as release assets. The rehearsal
  never creates a GitHub release or tag, never runs the `publish` job, never writes `ghcr.io/prikk-vcs/planeter`
  (any tag, including `latest`) and never touches crates.io. The dry run (publishes nothing at all) is unchanged.
- **T3 — The rehearsal cannot reach the real package, structurally.** Give the rehearsal its own steps: the
  real `Build and push` step keeps its tags and is conditioned on `github.event_name == 'push'`; the rehearsal's
  step has only the rehearsal tag and is conditioned on `inputs.mode == 'rehearsal'`. Extend `tools/pin-check`
  (with tests) so it **fails** when: a step that names `ghcr.io/prikk-vcs/planeter:` lacks
  `github.event_name == 'push'` in its `if`; a step that names `planeter-rehearsal` also names the real
  image; or a rehearsal-conditioned step names a `release`, `tag` or crates.io action/command. Planted-failure
  tests for each rule.
- **T4 — Measure the unknowns (report, do not design).** In the rehearsal run: (a) does the job token
  (`packages: write`) **create** the new package on first push, and what are its visibility and its link to the
  repository afterwards; (b) print `docker version` and `docker buildx version` on **both** runner types (x86_64
  and `ubuntu-24.04-arm`) and whether `docker buildx build --platform` works natively on each; (c) read the
  current README and inputs of `actions/attest-build-provenance` at its latest release and of `actions/attest`,
  and report the form a new implementation should use, the permissions it needs, and the full SHA of each
  candidate release tag with the check that it is the tag's commit in the upstream repository (D-9). **Do not
  change any pin in this handoff.**
- **T5 — Run it and show it.** Dispatch `mode: rehearsal` on the commit that contains this change, after it is on
  `main`. Report: run id; every job's result; the rehearsal package's existence and visibility; whether the image
  attestation verifies (`gh attestation verify oci://ghcr.io/prikk-vcs/planeter-rehearsal:rehearsal --repo
  prikk-vcs/planeter` — if your token lacks `read:packages`, say so and what you used instead); evidence that
  nothing else was published (no new release, no new tag, crates.io versions unchanged, the real package's tags
  not written: by construction from T3 and, if you can read them, by listing).
  Also dispatch `mode: dry-run` once to show it is unchanged.
- **T6 — Documentation.** `docs/RELEASING.md` (a "Rehearsal" section: when to run it per RFC 010 D-10, what it
  writes, how to delete the rehearsal package version when no longer wanted — a manual owner step), the
  dependency-policy note, STATUS IS-35 (progress, not closed).

## Non-change scope

No pin change (not even to a GitHub-owned action); no change to the real release path's behaviour; no tag, no
release, no crates.io action; no new secret or personal access token; no change to `Dockerfile` content; no
simplification of the chain yet (D-11 is the next handoff).

## Required documentation updates

RELEASING (mode input, rehearsal), `docs/dependency-policy.md` (rehearsal footprint: one private package and
labelled attestation records), the workflow's header comment.

## Acceptance criteria

`pin-check` fails on each planted violation of T3 and passes on the repository; the rehearsal run is green and
shows what T5 lists; the dry run is unchanged and green; the real release path's text is unchanged except for the
`mode` plumbing (show the diff of the real steps); every gate green including both supply-chain gates (fresh
advisory database), both test runs named, ENF-2.

## Prohibited shortcuts

No rehearsal step that can run on a tag push; no use of the real image name in a rehearsal path; no skipped
attestation to avoid the footprint (the footprint is accepted: O-5); no edit of a pin to "see if it works"; no
deletion of anything from the real package.

## Known risks

A rehearsal leaves permanent labelled attestation records and one private package (accepted, O-5). If the job
token cannot create the package, report it with the exact error: the owner may need to pre-create it (an owner
step), and that is a finding, not a failure.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code and the advisory-database HEAD; the run ids; the
T4 measurements (a), (b), (c) with the candidate SHAs and checks; the planted-failure outputs; the diff of the real
release steps; deviations and open questions.
