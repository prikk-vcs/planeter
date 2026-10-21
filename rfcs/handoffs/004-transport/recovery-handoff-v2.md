# RFC 004 — Transport — Implementation Handoff (v2, part 4): recovery, and the two things part 3b's review found

| | |
|---|---|
| Document | Part 4 of the RFC 004 v2 series: **a repository that `propose` flagged `needs_recovery` becomes writable again by a written, tested procedure** (RFC 004 D-6), run automatically only when every witness agrees and otherwise by an administrator with exact commands; plus the fix of IS-50 (an accept whose declaration fails is counted by no quota) and the small items the part 3b review left. |
| Status | **Done, reviewed Approved (2026-10-22; commits `2516bce`…`5ccf9a4`).** IS-50 closed. Follow-ups (IS-51 and two small items): `recovery-followups-handoff-v2.md`. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-6, D-7, D-3.5, U-13, U-16; [`propose-and-accept-handoff-v2.md`](propose-and-accept-handoff-v2.md) and `docs/propose.md` (the flag, the intent, `reconcile_at_start`, the test hook of T6); `docs/changes.md`; ledger PK-59 (accept resumable, not atomic), PK-60, U-16 (`unlock`'s PID advisory is wrong inside our PID namespace); the lease module (`lease_holders`, orphans) and the sandbox. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T0 — What the part 3b review decided (first, each its own commit)

(a) **IS-50 — an accepted-but-unrecorded artifact must be counted.** When a declaration does not hold, prikk has already accepted the
patches and they cannot be removed; today nothing counts them, so a member can add disk use without limit, one bad declaration at a time (only
the rate limit bounds it). Decision: **the intent carries the artifact's size and the submitter** (`artifact_bytes`, written when the
intent is opened; a migration, idempotent over reopens, existing rows read as 0), the declaration-failure path **retains** the artifact
(like an incomplete run; the corrected re-send needs only the same bytes), and the usage queries count every **open intent that has no revision**
in `accepted_bytes` (per submitter) and in `repo_retained_bytes`. An open intent is therefore a debt that a re-send, an abandonment (T4) or
its expiry (below) clears. Test: ten different artifacts each with a bad declaration reach the cumulative limit and the eleventh is
refused with the quota text; a corrected re-send of one of them records its revision and the bytes are counted **once**. (b) **Open
intents expire:** an open intent older than the retention period (30 days, the same operator variable as changes) may be abandoned by
the recovery command of T3 (never silently by the server); its artifact leaves the retained set; the accepted patches stay in the
repository. (c) **Wording** (catalog, English and the Japanese draft rows, and a fourth addendum): `error.needs_recovery.hint` becomes
two sentences ("Try again later. Reading is not affected, and the person who runs this server can see why in the server log.");
`error.declaration_misses_added.hint` and `error.declared_not_held.hint` say what is true: "Your artifact is already on this server;
send the same artifact again with a corrected list."; `error.invalid_request.metadata_hint` keeps its second clause in Japanese. (d)
**The machine value `forge-approved-unsealed` stays** (it is in the released 0.4.0 API contract); the OpenAPI description and
`docs/propose.md` say in plain words what it means ("accepted and held by this server, not yet signed into the history; nobody has
approved it for merging"). The human texts already say "held for review". (e) **Tests the 3b review found missing:** the
propose-specific `transport_off` hint over HTTP; a memory-ceiling kill of a propose classified incomplete (a test double of the driver
is acceptable; say so); and **one measurement**: `sync pending` with a repository holding about 20,000 pending patches (build once in
your own scratch area, outside CI; report time and memory; do not commit the repository).

## Change scope

- **T1 — The recovery procedure, as a function with a plan.** `recover(repo, mode)` where `mode` is `Plan` (read-only: what it
  would do, why, and whether it may run unattended) or `Run`. Steps, in prikk's order with our lease as the authority: (1) **witnesses**:
  the flag is set; the durable lease table has only dead holders for this repository (a holder whose process is this process is *not*
  dead); no process on the host has a prikk or `bwrap` command line naming the repository's directory; the open intents are
  listed. (2) take the **W lease** under a recovery operation name (bounded wait; a live holder ends the attempt with
  "not now"). (3) `prikk doctor` (read) and `prikk unlock` (list only); parse both with strict readers in `planeter-prikk` (unknown
  shape ⇒ stop with the text kept verbatim, nothing changed). (4) For each lock `unlock` lists: `unlock --lock <path> --force` (**the
  PID advisory is ignored**, U-16; the decision rests on step 1's witnesses). (5) `doctor` repairs only for what `doctor` names, each repair's
  run id recorded; `ref complete` where `doctor` names an interrupted publication. (6) **`verify`** (a fresh, un-cached run) must exit 0.
  (7) delete the dead lease records, clear the flag, bump the state fingerprint, release. Each step is written to a **recovery record**
  (table `recoveries`: id, repository, actor or "automatic", start, end, the steps with their outcomes and the text of each prikk call,
  the outcome) **before** it is taken where it changes something and completed after. A step that fails stops the procedure, leaves the
  flag set and names the exact commands for an operator in the message. Idempotent: running it on a healthy flagged repository only
  clears the flag after `verify`.
- **T2 — When it runs by itself.** Only when **every witness agrees**: the flag's cause is one of ours (`accept_deadline`,
  `accept_ceiling`, `accept_failed`, `accept_unreadable`, `accept_not_recorded`, or the start-up reason "the server stopped during a
  write"); every lease holder is dead; no foreign prikk process; `doctor` reports nothing beyond stale locks; open intents ≤ the number
  of proposals the leases account for. Then at **start-up** (after `reconcile_at_start`) and **right after an incomplete accept**
  (the child is confirmed gone) it runs `Run`. Anything else stays flagged for an administrator (T3). Automatic recovery never abandons
  an intent and never touches a repository twice at once.
- **T3 — The administrator's door.** (a) `GET /api/v1/repos/{owner}/{name}/recovery` returns the flag, its cause in words, the open
  intents (id, submitter, age, bytes, attempts), the last recovery records, and the plan; (b) `POST` of the same path runs it (and
  optionally `abandon_intent` ids). Both are registered through the route registry with **the repository-administration action the
  registry already has; if none fits, stop and ask the architect before adding an `Action`**. A caller who may not administer learns
  nothing beyond what the 404/403 rules already give. (c) The web UI shows nothing in this part. (d) The server log line at start-up names
  the repository and the call.
- **T4 — Retry converges.** After recovery the **same artifact** sent again resumes the intent and records exactly one revision
  (the test hook of part 3b is removed: the real procedure replaces it, and the part 3b kill test uses it).
- **T5 — Messages and documentation.** The recovery messages are catalog texts (English and the Japanese draft rows, fourth addendum).
  `docs/propose.md` gains the runbook: how an operator reads the plan, what each witness means, the exact commands when it stays
  flagged, the meaning of a recovery record, and how to restore (`--recovery-restore`) after a repair that was wrong. CHANGELOG,
  STATUS. The architect updates the threat model.

## Non-change scope

No client helper, no SSH, no creation from an archive, no review UI, no comments or decisions, no publication, no `extract`, no
change to the framing or the check order of part 3b (T0 only adds what is listed), no new external dependency, no `unsafe`, no
version or release change.

## Acceptance criteria

T0 done with its tests and the measurement. A kill test over a **smaller** artifact (about 300 patches, real server, real prikk, real
sandbox) at several points of the run (roughly 15, 40, 70 and 90 % of the accept, found by watching `sync pending` grow) shows, for
each: reads work; a new proposal is refused `needs_recovery`; **automatic recovery at the next start clears the flag** and `verify` and
`doctor` are clean; the same artifact sent again records exactly one revision derived from the first attempt's list; the recovery
record shows the steps. A deliberately **disagreeing** witness (a live foreign process; a lock `doctor` does not name; an unknown
`unlock` output) leaves the flag set, changes nothing, and the plan names the commands. Recovery run twice at once is serialized.
The administrator door refuses a caller without the action and tells a stranger nothing. All earlier checks pass: IS-14 table,
OpenAPI drift guard, ENF-2, every gate green including both supply-chain gates (fresh advisory database), both test runs named,
`cargo doc`.

## Prohibited shortcuts

No `unlock --force` without the witnesses of T1 step 1; no trust in the PID advisory; no clearing the flag without a fresh `verify`;
no repair that is not named by `doctor`; no automatic abandonment of an intent; no silent change of a quota's meaning (T0(a) is documented
and in the CHANGELOG); no recovery step without its record; no hard-coded user-visible text outside the catalog.

## Known risks

Recovery is the one place where we deliberately remove locks of the storage engine: every removal must be justified by evidence the
record shows. A PID-namespace or container-restart surprise (the lease holder's process table differs after a restart) is exactly why
the lease table, not the process id alone, is the authority; test the restart case. Do not let the recovery race an accept: both take W.

## Required evidence and review-request format

Commits (hash → what, by task); the witness table (what each one reads, what disagreement looks like, the test); the four-point
kill-test log (patches pending at the kill, what recovery did, the retry's result); a sample recovery record and plan (JSON);
the 20,000-patch `sync pending` measurement; the quota test of T0(a); every gate with command and exit code and the advisory-database
HEAD; deviations and open questions.
