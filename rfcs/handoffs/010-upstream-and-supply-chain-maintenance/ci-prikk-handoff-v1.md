# RFC 010 — CI runs the tests that need prikk, so fewer of them have to run on a developer's machine — Implementation Handoff (v1)

| | |
|---|---|
| Document | From the gate-cost measurement (`gate-cost-measurement-review-request.md`, "What CI already covers"): the tests **with prikk** (the real-server, kill-point, delivery and creation tests: the 4,000 seconds of wall) run **nowhere but on a developer's or the architect's machine**, because the `build` job has no prikk (the `client` job installs it only for `planeter-client`'s tests, on three systems). That makes the architect's review tier carry them every time, and means CI is green on a commit whose real-prikk tests it never ran. The `client` job already downloads a **pinned, checksum-verified** prikk (`PRIKK_VERSION`); the `build` job can do the same. The tests that need **bubblewrap** will still skip on the runner (Ubuntu 24.04 restricts unprivileged user namespaces: see the memory of the earlier rehearsals, and `Sandbox::Unconfined` in the stand-in tests), **but most of the real-prikk tests use `Sandbox::Unconfined`** and will run. This moves work from a laptop to a runner and makes CI a better baseline for Amendment 1's tiers. |
| Status | Assigned 2026-11-27 (SCHEDULE D-145). After `gates-snapshot-handoff-v1`. |
| Basis | `.github/workflows/ci.yml` (the `build` and `client` jobs; how `client` installs prikk: the version variable, the checksum, where it is put on `PATH`); `tools/pin-check` (its rules: the new step must satisfy them); the test files that skip without prikk or bubblewrap (`need_all!`, `need_server!`, "self-skip") ; `docs/RELEASING.md` (the tiers). |
| Audience | Dev team. Return a review-request package. |

## Change scope
1. **Install the pinned prikk in the `build` job** the way the `client` job does (same variable, same checksum verification, same location; no new pin: `PRIKK_VERSION` already equals the floor and `planeter-floor-check` keeps it so). The install happens **before** `cargo test --workspace`; a failed download or checksum stops the job loudly. **Say how long the install takes and what it downloads.**
2. **Measure before you commit to it, and report:** the `build` job's wall time **before and after** (three CI runs each, or the numbers the runs already give you); **which tests newly run** on the runner (the count of tests that previously skipped for lack of prikk and now run, and the count that still skip for lack of bubblewrap or `prlimit`: list the files); any test that **fails on the runner** and is not a product defect (a timing or environment assumption): fix it or, if it needs a runner feature the runner lacks, make it skip with a reason; **do not weaken an assertion**. If the added wall time exceeds **20 minutes** of the `build` job, stop and report with the numbers instead of committing it (the architect decides: a separate job in parallel is the likely answer).
3. **A separate job instead, if the numbers say so.** If putting it in `build` makes the job too slow or flaky, make a new job `tests-with-prikk` (pinned prikk, `cargo test --workspace`, in parallel with `build`) and keep `build` as it is; say which you chose and why.
4. **`pin-check` and `maintenance-gates`** stay green: no mutable reference, the action SHAs as before; add a test case to `planeter-pin-check` only if the install uses a shape it does not yet understand.
5. **`docs/RELEASING.md`**: the tiers paragraph's list of what CI covers is updated (tests with prikk, **without bubblewrap**), and the review tier's own list shrinks **only if** CI really runs them and you can say which: **do not remove the local run of the tests that need bubblewrap**, because CI cannot run those. The release tier is unchanged.

## Non-change scope
No product code, no test assertion weakened, no change to the release workflow or the `Dockerfile`, no new action, no new third-party download (prikk's release from the same source and checksum as the `client` job), no change to the pinned compiler or tools, no tag, release or upload.

## Acceptance criteria
CI runs the real-prikk tests that do not need bubblewrap (the count named), stays green on all jobs on three consecutive runs, the `build` job's added time is reported and within the bound (or the separate-job form is used), and `docs/RELEASING.md` says exactly what CI now covers; the review tier on the commit is green.

## Required evidence
Commits (hash → what); the install step's text; the before/after times and the three runs; the counts and lists (newly run / still skipped); anything fixed on the runner and why; the review-tier table; deviations and open questions.
