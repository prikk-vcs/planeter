# RFC 004 — Transport — Implementation Handoff (v2, part 4a): what the recovery review found

| | |
|---|---|
| Document | Small follow-up to part 4: **the administrator's repair step must be reachable and tested**, **the foreign-process witness must see a person's prikk by its working directory and open files**, and **the accept-time-per-patch growth must be measured against the object bound** so the deadline and the bound can be set on facts. |
| Status | **Assigned** (2026-10-22), after part 4 was reviewed Approved. |
| Basis | [`recovery-handoff-v2.md`](recovery-handoff-v2.md) and its review; `crates/planeter-transport/src/recovery.rs` (`evaluate`, `run_steps`, `ProcessTable`); `crates/planeter-web/tests/recovery.rs` (`doctor_problems_stop_the_unattended_run_and_an_administrator_may_run_only_the_repairs_doctor_names`); [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-5.4, D-6, R-3; ledger PK-59, PK-68 (the new row). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — The repair step must be reachable, and the test must say what it tests

**Found:** `evaluate` adds `doctor_problems` as a **non-overridable** blocker whenever `doctor` is not clean, and `stopping` stops on any
non-overridable blocker before `run_steps` reaches the repair step. The administrator's repair code (and `docs/propose.md`, which says
the run executes the repairs `doctor` names) can therefore never run; the test named *"…an administrator may run only the repairs doctor
names"* asserts the opposite (`Outcome::Stopped`), and the review request's statement that the repair path is tested with a stand-in is
not so. Fix, keeping the safe defaults:
(a) `doctor_problems` becomes **overridable by an administrator only when every problem `doctor` reports is covered by a named repair that
is on the idempotent allow-list** (`--repair-wal-tail`, `--repair-index`, `--repair-pointer-index-tail`, `--repair-tails`); one error
without such a repair (or any data-losing repair named) keeps it non-overridable for everyone, and an **unattended run is never given
the override**. (b) Tests, using the existing stand-in: the administrator's run executes exactly the named repair, in a step written before
it is taken, with its output (and any run id) in the step text; `doctor` is run again and must be clean; then the rest of the procedure
completes and the flag is cleared; an unattended run on the same state still stops and repairs nothing; a stand-in whose second
`doctor` is still dirty stops after the repair with the flag set; a named repair **not** on the allow-list is never run, and shows only as
text in the plan. (c) Rename the test to what it asserts, split in two. (d) Correct `docs/propose.md` where it differs from what the code
does.

## T2 — The foreign-process witness looks at more than command lines

`names_repository` reads `argv` only: a person who has `cd`'d into the repository and runs `prikk status` has no path in the command line
and is invisible. Add two readings of `/proc/<pid>/`: the **working directory** (`cwd`) and the **open file descriptors** (`fd/*` link
targets) — a process whose `cwd` is the repository directory or inside it, or that holds a file open inside the repository's `.prikk`, is a
foreign process unless it is a descendant of this server. A process whose `cwd`/`fd` cannot be read (another user's) is **not** treated as
safe or unsafe silently: the plan lists "n processes could not be inspected" as a **warning** that stops an unattended run but not an
administrator's. Tests: a real child that only `cd`s into the repository and sleeps; one that holds a file of `.prikk` open; the server's own
sandbox child is not foreign; an unreadable entry (a trait-level test double) behaves as stated. Document in the runbook what the witness
can and cannot see (another PID namespace, another host).

## T3 — Measure: how many patches fit under the object bound, and what the deadline must be

PK-68 (new): the accept of **20,000 patches took 677.6 s (≈ 34 ms a patch)** against ≈ 14 ms at 1,000 — over the 600 s deadline; per-patch
cost grows with the number of patches already pending. What is not known is the relation between **objects** (what `PRIKK_EXCHANGE_MAX_OBJECTS`
bounds, default 20,000) and **patches**, so we cannot say how long the worst accepted artifact takes. In your own scratch area (not CI, not
committed), with real prikk: (a) for artifacts of 1, 2, 5, 10 and 20 thousand patches **report the object count `sync accept` sees** (from the
refusal text at a low bound, or the report) and whether the default bound admits them; (b) **accept wall time at 1,000, 2,500, 5,000, 7,500
and 10,000 patches** on a store that starts empty, and once more at 5,000 on a store that already holds 10,000 pending (the growth is the
pending set, not the artifact); (c) the memory peak of each (the 4.2 × artifact rule still hold?); (d) the same ten-patch artifacts in a
store with 20,000 pending already. Report a table and the largest patch count the default object bound allows. **Do not change a default in
this handoff**; the architect sets the bound and deadline from the table (RFC 004 D-5.4 amendment).

## T4 — Operator words in the glossary

The recovery texts use *flag*, *lease*, *accept*, *intent*, *lock* in operator-facing sentences. Add the **operator-facing terms** to
`docs/glossary.md` as a separate section "Words for the person who runs the server" with a plain definition each (flag = the mark a repository
carries while it waits for recovery; lease = the right to write to a repository now, held by one thing at a time; accept = the server taking in a
proposal; intent = the server's note that it started taking one in; lock = prikk's own file that stops two writers; recovery = the steps that make a
flagged repository writable again). The glossary test must cover the section (every term used in a `recovery.*` or `error.*` operator
sentence appears in it). English only here; the Japanese rows are the reviewers' to decide, in a fifth addendum.

## Non-change scope

No change to the framing, the check order, the quotas, the defaults of any bound or deadline, the lease classes or the HTTP surface; no new
dependency; no `unsafe`; no version or release change.

## Acceptance criteria

T1's tests including the three negative ones; T2's four tests; T3's table; T4's section and test. All earlier checks pass: IS-14 table,
OpenAPI drift guard, ENF-2, every gate green including both supply-chain gates (fresh advisory database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No repair outside the allow-list and outside an administrator's run; no override of `doctor_problems` that an unattended run can reach; no
witness that treats "cannot read" as "safe" without saying so; no change of a default; no hard-coded user-visible text outside the catalog.

## Required evidence and review-request format

Commits (hash → what, by task); for T1 a transcript of the administrator's run (the steps and their outcomes) and of the refusing cases;
for T2 the four tests and the plan with the warning; for T3 the table, the method and the scripts' location (not committed); for T4 the section;
every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
