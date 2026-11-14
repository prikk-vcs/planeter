# RFC 010 — IS-58: pin `cargo-deny` and `cargo-audit` — Implementation Handoff (v1)

| | |
|---|---|
| Document | `ci.yml`, `release.yml` and `supply-chain.yml` install the two tools that gate every push and every release at their **newest** versions: `cargo install cargo-deny cargo-audit --locked`. `planeter-pin-check` says in its header that this is "deliberately current and out of scope". The 0.6.0 security review (IS-58) records the cost of that choice: a tool update can change a gate's verdict, or a hostile one could report a pass; the `gates` job has only `contents: read`, so the damage is a false verdict, not a leak, but a false verdict is exactly what the gate exists to prevent. The reason for "current" was real (an old tool can fail to read a new advisory database); the answer is to pin and to move the pin deliberately, because that failure is **loud** (the gate goes red) while a wrong verdict is silent. First item of 0.7.0 (D-78). |
| Status | Assigned 2026-11-14. |
| Basis | the three workflow files; `tools/pin-check/src/lib.rs` (header and the rules); `docs/RELEASING.md` ("What the release chain trusts", step 5 pins review); `docs/dependency-policy.md`; `docs/security-review-0.6.0.md` (IS-58). |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Find the versions in use.** From the logs of the last green `ci.yml` and `release.yml` runs (the rehearsal 43575291310 and the release 43579591208, or newer), read which versions `cargo install` built, and report them with the run ids. Pin **those** (the versions that have actually produced the verdicts we rely on), unless the pins review (next point) finds a reason not to.
2. **Read before pinning (one row each):** the tool's release notes since the pinned version up to the newest on crates.io, any advisory (RustSec, GitHub) on the tool or on its dependencies, whether the newest reads the current advisory database differently from the pinned (run both against a fresh clone of the database and the repository's `Cargo.lock`: same verdict, same advisory count), and the crate's license (it is a tool, not a dependency: no `cargo deny` entry, but state it).
3. **Pin in all three workflows:** `cargo install cargo-deny --version X.Y.Z --locked` and `cargo install cargo-audit --version A.B.C --locked` (two commands, or one with both `--version`-pinned forms; whichever the pin-check can read). Comment each with the date and "moved at the pins review of each release".
4. **`planeter-pin-check`:** a new rule: in every workflow, a `cargo install` of `cargo-deny` or `cargo-audit` without an exact `--version X.Y.Z` (no range, no `^`, no `latest`) and without `--locked` fails the check; every other `cargo install` also fails (there is none today: the rule is a closed list, the same style as the others). All three workflows must name the **same** versions. Add the new rule's tests (a passing file, and a failing one per violation), update the header sentence ("deliberately current and out of scope" is no longer true), and mention it in `docs/dependency-policy.md` and in `docs/RELEASING.md` step 5 (the pins table gains two rows: the tools).
5. **Rehearsal.** The release chain changed (`release.yml`): run `gh workflow run release.yml --ref main -f mode=rehearsal` after the change is on `main` and record the run id; the gates job must show the pinned versions in its log. Do not run it for reassurance alone: this is a chain change, which RFC 010 D-10 names.
6. **Evidence of the failure mode we accept:** show, in a scratch copy, that an older `cargo-audit` than the pinned one either reads the current advisory database or fails loudly (not silently passes): install the oldest version that still builds with the repository's compiler, run it against a fresh database clone, and report the exit and the message. This documents why the pin is safe to hold for a release cycle.

## Non-change scope
No dependency of the product changes; no other pin moves; no version change; no tag.

## Acceptance criteria
The versions pinned in all three workflows and equal; `pin-check` fails on each violation and passes on the tree; a rehearsal run green with the pinned versions in the log; the pins review rows; the old-tool evidence; all gates green (fmt, floor, pin, clippy, tests with prikk 0.57.0 and without, deny and audit 0 with the pinned tools, ENF-2, doc), CI green.

## Required evidence and review-request format
Commits (hash → what); the versions and the run ids they came from; the pins review rows; the pin-check tests; the rehearsal run link and the log lines; the old-tool evidence; every gate with command, exit code, tool versions and the advisory-database HEAD; deviations and open questions.
