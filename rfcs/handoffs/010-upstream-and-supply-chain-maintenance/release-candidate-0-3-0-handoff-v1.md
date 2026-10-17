# RFC 010 follow-on — Release-candidate commit for 0.3.0 and the dry run — Implementation Handoff (v1)

| | |
|---|---|
| Document | The dev-team step **R4** of release 0.3.0 (SCHEDULE §5, authorized by decisions D-11, D-12, D-13, D-14). It uses RFC 010's release-implication machinery (D-5 pins, D-6 build identity, the dry-run mode of `release.yml`) and `docs/RELEASING.md`; it adds no capability. |
| Status | **Assigned** (2026-10-17): R1, R2, R2b and R3 are done and approved. |
| Basis | `docs/SCHEDULE.md` §5 (objective, content, entry and exit criteria, steps R1–R7); `docs/RELEASING.md` ("Before the tag", "Dry run"); `CHANGELOG.md` `[Unreleased]`; [`maintenance-gates-handoff-v2.md`](maintenance-gates-handoff-v2.md). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). **You do not tag, publish, or change package visibility: those are the owner's (R6).** |

## Change scope

- **T1 — Version.** `[workspace.package] version = "0.3.0"`. Update `Cargo.lock` only as `--locked` requires;
  if any path-dependency version strings need the same bump, make them.
- **T2 — CHANGELOG.** Turn `[Unreleased]` into `## [0.3.0] — <short title>` (date 2026-10-17 or the commit's
  date) and leave an empty `## [Unreleased]` above it. Keep every entry; do not shorten operator-visible
  warnings. Put a short **"Upgrading from 0.2.0"** list first, one line each, in this order: the prikk
  prerequisite (≥ 0.52.0); the declared minimum compiler (1.88); a non-loopback bind now needs a distinct
  content host (refuses to start); an unknown argument exits 2; API errors are JSON; file `content` is
  `kind`-tagged and a missing file is 404; caching headers; the new `PLANETER_AUTH_*` variables. Check each line
  against the code, not against this list.
- **T3 — Checks from RELEASING "Before the tag", steps 1–5.** Run all of them on the candidate commit. For step 5
  (pins against upstream releases and advisories) report each pin's pinned version, upstream's latest and
  whether you changed it; **an update is a reviewed change that names the upstream diff**. If you find nothing to
  change, say so per pin.
- **T4 — Dry run.** Dispatch `release.yml` with `dry_run: true` on the candidate commit **after it is on
  `main`**. Verify, as in the gates-v2 review, that nothing was published (releases, tags, crates.io unchanged;
  login, push and attestation steps skipped). Check that `build-info.txt` and the artifact's own `--version`
  show `planeter 0.3.0`, the candidate commit and `prikk-floor 0.52.0`.
- **T5 — Records.** STATUS "current state" and ROADMAP's 0.3.0 row say "release candidate ready"; do not mark it
  released.

## Non-change scope

No code or workflow behaviour change except what T1/T3 require; no tag; no push of a tag; no crates.io
action; no ghcr visibility change; no new dependency.

## Acceptance criteria

Every gate green on the candidate commit including `msrv`, both supply-chain gates and ENF-2; the dry run
green with nothing published; the CHANGELOG `[0.3.0]` section exists and the workflow's release-notes `awk`
extraction yields it (show the extracted text); version, `--version`, build-info and the CHANGELOG agree.

## Prohibited shortcuts

Do not run a non-dry dispatch or push any tag, even to a fork or a test tag name; do not edit `release.yml`
to get a green run (report the failure instead); do not drop a CHANGELOG entry to shorten it.

## Known risks

The first real tag-run exercises steps the dry run skips (login, push, attestation, `gh release create`): list
them in the report so the owner knows what is first exercised at R6.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code; CI run id for the candidate commit; the dry-run
run id and the published-nothing evidence; the extracted release notes; the pin table (T3 step 5); deviations
and open questions.
