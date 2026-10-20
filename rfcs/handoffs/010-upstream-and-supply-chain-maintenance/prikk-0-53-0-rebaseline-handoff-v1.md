# RFC 010 D-1 — prikk 0.53.0 re-baseline — Implementation Handoff (v1)

| | |
|---|---|
| Document | The per-prikk-release re-baseline (RFC 010 D-1, `docs/UPSTREAM.md` "Re-baseline procedure"), instance **0.53.0**. The owner installed 0.53.0 on the development host on 2026-10-21 (`prikk --version` → `prikk 0.53.0`). Follows the shape of [`prikk-0-52-0-rebaseline-handoff-v2.md`](prikk-0-52-0-rebaseline-handoff-v2.md). |
| Status | **Assigned** (2026-10-21). |
| Basis | prikk's 0.53.0 release notes (the architect's reading below); `docs/UPSTREAM.md` steps 1–7; ledger PK-18 (drift), PK-26 (floor), PK-42/PK-43, PK-51 to PK-53 (read cost and its multiples); RFC 011 D-1 (the cost table is a per-release measurement); `crates/planeter-prikk/src/cli.rs`, `crates/planeter-core/src/{read,admission}.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## What the release says (architect's reading; verify, do not trust)

Read the notes yourself (`gh api repos/prikk-vcs/prikk/releases/latest --jq .body`). The parts that touch planeter:

1. **`checkout` refusing modes** (`--snapshot-plan`, `--snapshot-materialize`, `--patch-plan`, `--patch-materialize`, …)
   keep refusing on a branch with queued, unsealed commits, but the message changes from `ref heads/main does not
   exist in this repository` to `heads/main has no sealed block yet (N commit(s) queued) — prikk seal publishes it`
   (same "precondition not met" class). **planeter's `file` route uses `checkout --patch-plan`**
   (`CliPrikkRepo::patch_plan_content`). What a user sees for a repository with only queued commits could change.
2. A **cold `commit`** is cheaper, `commit` reads the queue three times instead of four: not used by planeter
   (it never commits), recorded only.
3. prikk states `verify` is unchanged and reads are not touched ("0.53.0 does not change these verbs", letter of
   2026-10-18). **That is prikk's statement; T4 measures it.**
No format change and no JSON change is mentioned.

## Change scope

- **T1 — Schema drift (PK-18, PK-43).** With 0.53.0 on `PATH`, run the driver and read-path suites
  (`cargo test -p planeter-prikk -p planeter-core -p planeter-web --locked`), then record for **each of the 14
  expected `schema_version` ids** (list in the 0.52.0 handoff, T1) whether the *fields planeter deserializes* are
  unchanged: use the real binary on the fixtures and compare the field sets against the recorded 0.52.0 outputs
  (the investigation fixtures from the RFC 011 work are the source; rebuild them with 0.53.0). **If any id or field
  shape changed: stop, record the exact difference, hand back.**
- **T2 — Every place planeter depends on prikk's text.** Find them all (`grep` for `stderr.contains`, `contains("`,
  message matches in `crates/planeter-prikk` and `crates/planeter-core`; known: `read.rs` `does not exist` for
  `cat`, the ceiling classification in `cli.rs` — `did not complete every stage`, `memory allocation of`,
  `i/o error:` + `out of memory`; the `trust check` exit/JSON handling; the version line parse). For **each**:
  the exact text it depends on, the 0.52.0 text (from the recorded outputs), the 0.53.0 text (run the trigger),
  and equal / changed. A changed text that a match depends on is a finding: stop and report with a proposal.
- **T3 — The queued-only repository (release note 1).** Build a repository whose only content is queued,
  unsealed commits (a branch with N queued commits and no sealed block), as the note describes. On 0.53.0 record,
  through the real server (router, `ServiceExt::oneshot` as the baseline harness does): the repository page,
  history, tree, file (`patch-plan`), raw, refs and verify pages and the matching API responses — status, body, and
  which `ReadError` each maps to. State plainly what a user sees now and whether it is correct (honest, existence-
  neutral, with a next step) or misleading. If planeter's mapping of the new message differs from the old behaviour
  in a way that matters, **do not fix it in this handoff**: report it and propose (the architect decides). Note
  that the sealed history of a repository with *both* sealed and queued commits is unaffected (confirm).
- **T4 — The cost table on 0.53.0 (UPSTREAM step 6).** prikk says reads are unchanged. Confirm by measurement,
  not by trust: on the fixture set of the RFC 011 investigation (1/16/64/200 MiB many-file repositories, a
  1,000-block history, 10,000 small files, 64 MiB binary and 64/200 MiB text single-file shapes), record for
  `log`, `show`, `branch`/`tag`, `tree`, `cat`, `checkout --patch-plan` and `verify` the peak memory and the
  smallest working `RLIMIT_DATA`, and compare with `admission.rs` (cost) and the ceiling (cost × 1.5). Report the
  ratio table. **Do not change the table in this handoff:** if every cell is within the existing rule (cost never
  under-predicts, ceiling clears the smallest working limit), say "unchanged"; if any cell is outside, stop and
  report (the multiples are the architect's to change, from your numbers).
- **T5 — The floor.** Recommendation: **keep `MIN_PRIKK_VERSION` at 0.52.0**: 0.53.0 carries no verb or
  correctness fix planeter needs (procedure step 4). Report the evidence for or against (T1–T4); do **not** bump the
  floor, `release.yml`, `Dockerfile`, `README` or RELEASING. If T1–T4 show a reason to bump, stop and report.
- **T6 — Records.** `docs/STATUS.md` (the "pending upstream" table / current-state line: prikk 0.53.0 installed,
  re-baseline result), `docs/UPSTREAM.md` only if a step needed correcting, the dependency-ledger inputs
  (the architect writes the rows; give the facts: per row affected, the measurement and the outcome). A hold note
  on any handoff whose assumptions the release changed (expected: none).
- **T7 — The suite with prikk off `PATH`.** As procedure step 3: once more with prikk shimmed off, which is what
  CI sees; the prikk-dependent tests self-skip with their printed reason.

## Non-change scope

No code change unless a test fixture must follow a changed text (and then only after reporting, T2); no floor bump;
no cost-table change; no dependency change; no `unsafe`; no version or release change; no change to hosted
repositories (planeter never upgrades a hosted repository's format).

## Required documentation updates

STATUS (T6) only, plus a short facts file for the architect's ledger rows inside the review request.

## Acceptance criteria

T1's per-id drift statement for all 14 ids; T2's table of every text dependency with equal/changed; T3's behaviour
table for the queued-only repository; T4's ratio table with the verdict "unchanged" or the stop-report; T5's floor
evidence; the suite green with 0.53.0 and with prikk off `PATH` (the same test count); every gate green including
both supply-chain gates (fresh advisory database), ENF-2, `cargo doc`.

## Prohibited shortcuts

No relaxing a test to accept a changed text; no adjusting the cost table or the ceiling to make T4 pass; no floor
bump; no reading prikk's statement "reads are unchanged" as a measurement; no stale advisory database.

## Required evidence and review-request format

The commands and outputs; the four tables above (drift, text dependencies, queued-only behaviour, cost ratios);
every gate with command and exit code and the advisory-database HEAD; deviations and open questions. Keep
scratch under `.git-exclude/tmp/<name>/` and remove it when done; relative `cd` only.
