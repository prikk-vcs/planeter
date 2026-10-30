# RFC 010 — Release 0.5.0: the release-candidate commit — Implementation Handoff (v1)

| | |
|---|---|
| Document | Everything planned for 0.5.0 is built and reviewed (D-53). This handoff prepares **the release-candidate commit** exactly as `docs/RELEASING.md` ("Before the tag") describes, and stops before the tag: **tagging, the GitHub release, crates.io publication and making the ghcr package public are not part of it** (the architect asks the owner for the tag when this is reviewed). Version: **0.5.0** (SemVer 0.x: new features and removals that break the read API — `verified_sessions`/`total_sessions` of the verify API, IS-49 — need a minor). |
| Status | **Assigned** (2026-10-30). |
| Basis | `docs/RELEASING.md` (steps 1–5, "Dry run", "Rehearsal", "crates.io"), `CHANGELOG.md` `[Unreleased]`, `docs/SCHEDULE.md` §5 for 0.3.0/0.4.0 (the shape of a release record), `docs/security-review-0.5.0.md`, the threat model v0.6, `ROADMAP.md`, `README.md`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — Version and CHANGELOG.** `[workspace.package] version = "0.5.0"` (the lock follows, `--locked` everywhere); `planeter-client`'s version is the workspace's. Turn `[Unreleased]` into `## [0.5.0] - <date of the tag, left as the placeholder "unreleased until tagged" and filled by the architect>`: reorder the entries into reader order — **For people who run a server** (breaking changes first: the verify API's removed fields, the `PRIKK_EXCHANGE_MAX_*` defaults 100 MiB / 5,000, the new required prikk floor 0.54.0, the image base now Debian 13, new settings with defaults, the artifact directory, the spool), **For people who use it** (proposals, review, creating a repository, the client), **Under the hood** (the measured models, the output cap, aws-lc, the release gates) — keeping every existing fact, adding none that is not already in an entry, one paragraph per item as written. A short opening paragraph says what 0.5.0 is: the first release in which a person can propose work to the forge, review it, and create a repository, with the forge still holding no key. The client's distribution is stated: **0.5.0 ships `planeter-client` through crates.io (`cargo install planeter-client`) and as source; release-asset binaries of the client are not part of this release.**
- **T2 — The crates.** All crates that are published (`planeter-*`, now including `planeter-client`) have correct manifest metadata (description, license `Apache-2.0`, repository, readme, `rust-version`, `include`/`exclude` that keep tests' private fixture keys and certificates out of the package where they are not needed — report what each package contains, `cargo package --list`); run `cargo publish --dry-run` (no upload) for each in dependency order and report the results; `planeter-runner` stays unpublished. Note the order for the publication step in `docs/RELEASING.md` if `planeter-client` changes it (it has no workspace dependency).
- **T3 — The documents.** `README.md` and `ROADMAP.md` say what 0.5.0 is and what is next (prikk 0.56.0: chain extraction and the delivery guard; model B of D-8; checks/CI; SSH not planned); `docs/RELEASING.md` "Release log" gets the 0.5.0 row template (filled after the tag); check every document that names a default or a count that this release changed (`grep` the old numbers: 256 MiB, 20,000, 214 crates, 0.52.0/0.53.0 floors) and report each hit with the decision.
- **T4 — The pins review (RELEASING step 5).** For every `uses:` SHA and the base-image digest, compare the pinned tag with the upstream's latest and read the advisories; report a table (pin, pinned tag, latest, advisories read, decision). An update is its own reviewed commit that names the upstream diff and keeps `pin-check` green; do not update anything "while you are there" without the table.
- **T5 — Rehearsal and dry run on the candidate.** After T1–T4 are merged to `main`: a **rehearsal** of the release workflow (the release chain changed since 0.4.0: the image jobs, the glibc and identity gates) and record the run id; a **dry run** is implied by it. Both architectures must show: build, image run, glibc check, push, identity, manifest list and attestation (rehearsal package). Report the run link and the per-job result.
- **T6 — The gates, immediately before handing over** (RELEASING step 1): a fresh advisory database, `cargo deny check` and `cargo audit` exit 0, fmt, floor-check, pin-check, clippy, both test runs named, ENF-2 on both binaries, doc, the `msrv` job, CI green — on the final candidate commit. Run the `client` stress target once (`PLANETER_STRESS_RUNS=20`, in a memory-limited scope) and report it.

## Non-change scope

No tag; no GitHub release; no crates.io upload; no change to the ghcr package's visibility; no feature or fix beyond the files named; no dependency change; no change to a default.

## Acceptance criteria

T1–T6 as written with their evidence; the candidate commit is on `main` with everything green; the architect can read the CHANGELOG section as the release notes without editing facts.

## Prohibited shortcuts

No tag, no upload; no unreviewed pin update; no "publish" step run even as a test; no edit of a historical CHANGELOG entry; no weakening of a gate.

## Required evidence and review-request format

Commits (hash → what, by task); the `cargo package --list` summaries and the dry-run results; the pins table; the rehearsal run link and per-job results; the stress run; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
