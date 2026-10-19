# RFC 010 follow-on — Release-candidate commit for 0.4.0 and the dry run — Implementation Handoff (v1)

| | |
|---|---|
| Document | The dev-team step **R4** of release 0.4.0 (SCHEDULE §5, authorized by decision D-18, 2026-10-19). It uses RFC 010's release machinery (D-5 pins, D-6 build identity, the dry-run mode of `release.yml`) and `docs/RELEASING.md`; it adds no capability. The content of the release is RFC 011 (done, approved). |
| Status | **Assigned** (2026-10-19). |
| Basis | `docs/SCHEDULE.md` §5 "0.4.0" (objective, content, entry and exit criteria, steps); `docs/RELEASING.md` ("Before the tag", "Dry run"); `CHANGELOG.md` `[Unreleased]`; the 0.3.0 handoff [`release-candidate-0-3-0-handoff-v1.md`](release-candidate-0-3-0-handoff-v1.md) and its lessons (the tag run catches a new advisory; refresh the advisory database). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). **You do not tag, publish or change package visibility.** |

## Change scope

- **T1 — Version.** `[workspace.package] version = "0.4.0"`, and the path-dependency version strings that carry it;
  `Cargo.lock` changes only as `--locked` requires (the workspace crates' own versions).
- **T2 — CHANGELOG.** Turn `[Unreleased]` into `## [0.4.0] — <date> — <short title>` and leave an empty
  `## [Unreleased]` above it. Keep every entry unshortened; fold the entries so that the section reads as one
  release (RFC 011's several entries stay separate bullets, but remove statements that a later entry
  corrected: for example the raw-route reduction, if any trace remains). Put an **"Upgrading from 0.3.0"** list
  first, one line each: `prlimit` (util-linux) is now a required program and planeter refuses to start without
  it (the container image has it); `timeout` (coreutils) is no longer used; the new variable
  `PLANETER_PRIKK_MEMORY_BUDGET_MB` and its default rule (a quarter of usable memory, between 256 MiB and 8 GiB;
  an explicit value is not capped; a sizing rule in RELEASING); reads can now be answered `503 busy` for lack of
  budget or `422 too_large`; `GET …/file` refuses files above 4 MiB (raw keeps its 25 MiB cap); a killed server no
  longer leaves its prikk processes running; the sandbox wall-time bound is now a SIGKILL at the deadline.
  Check each line against the code and against the CHANGELOG entries, not against this list.
- **T3 — Checks from RELEASING "Before the tag", steps 1–5.** Run them on the candidate commit, **with a freshly
  cloned advisory database** for `cargo deny` and `cargo audit` (state the database's HEAD), and report them. For
  step 5 (pins against upstream releases and advisories) report each pin as in 0.3.0's table; **do not change a
  pin in this handoff** (IS-35 is separate); a pin with a *published advisory* is the exception: stop and report.
- **T4 — Dry run.** Dispatch `release.yml` with `dry_run: true` on the candidate commit **after it is on `main`**.
  Verify, as for 0.3.0, that nothing was published (releases, tags, crates.io unchanged; login, push and
  attestation steps skipped). Check that `build-info.txt` and the artifact's own `--version` show
  `planeter 0.4.0`, the candidate commit and `prikk-floor 0.52.0`, and that `requires:` names `prlimit`. Show that
  the image build executed the `prlimit --version` step for both architectures.
- **T5 — Notes.** Show the release notes the workflow's `awk` extracts for `0.4.0` (the list first).
- **T6 — Records.** STATUS "current state" and the ROADMAP row say "release candidate ready"; do not mark it
  released.

## Non-change scope

No code or workflow behaviour change except what T1 requires; no tag (not even a test tag); no crates.io action;
no ghcr visibility change; no pin change; no new dependency.

## Acceptance criteria

Every gate green on the candidate commit including `msrv`, both supply-chain gates (fresh database) and ENF-2; CI
green; the dry run green with nothing published; the CHANGELOG `[0.4.0]` section exists, its extraction is shown, and
version, `--version`, build-info and the CHANGELOG agree.

## Prohibited shortcuts

No non-dry dispatch; no tag; no edit to `release.yml` to get a green run (report the failure instead); no dropped
CHANGELOG entry; no stale advisory database.

## Known risks

The tag run's first-time steps (registry login and push, attestations, `gh release create`) were exercised by 0.3.0
and are unchanged; the new Dockerfile `prlimit` step is exercised by the dry run. A new advisory may still appear
between your run and the tag: the architect re-runs both gates immediately before tagging.

## Required evidence and review-request format

Commits (hash → what); every gate with command and exit code and the advisory-database HEAD; the CI run id for the
candidate commit; the dry-run run id and the published-nothing evidence; the extracted release notes; the pin table
(T3 step 5); deviations and open questions.
