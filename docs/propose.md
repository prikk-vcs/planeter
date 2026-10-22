# Propose: putting an artifact on the forge as a change revision

| | |
|---|---|
| Document | The write side of the transport (RFC 004 part 3b; D-3, D-5, D-6, D-7): the request, the order of the checks, the life of an intent, the operator's settings, and what the client and the operator see when something goes wrong. The records and rules it uses are in [`changes.md`](changes.md). |
| Where | `crates/planeter-web/src/transport.rs` (the route), `crates/planeter-transport/src/propose.rs` (the service), `crates/planeter-transport/src/artifacts.rs` (the upload area). Tests: `crates/planeter-web/tests/transport_propose.rs`, `crates/planeter/tests/propose_kill.rs`. |

**Nothing here seals, merges or publishes.** A proposal makes the work *held for review* on the forge; no branch changes.

## The request

`POST /api/v1/repos/{owner}/{name}/transport/propose`, authenticated by a **bearer token only** (a session cookie is not looked
at, so another site cannot make the request on a browser's behalf), authorized for **propose**: `write` and above; any signed-in
reader if the repository's `proposals` setting is `any-authenticated`; never a CI job; a token limited to reading never.

The body is framed without multipart:

```
PLANETER-PROPOSE-1\n          19 bytes
M                             8 bytes, big-endian: the length of the metadata
metadata                      M bytes of JSON
artifact                      the rest of the body: `prikk sync build` output (PEXCH002)
```

`Content-Length` is required (`411`); `Content-Type`, if sent, is `application/octet-stream`. `M` is at most 4 MiB, the artifact
at most `PRIKK_EXCHANGE_MAX_BYTES` (default 256 MiB): the declared length is checked **before a byte of the body is read**
(`413`). The artifact is streamed to disk and hashed as it arrives, never held in memory; a client that goes quiet for 30 seconds,
or takes more than 15 minutes, is cut off.

Metadata (unknown fields are refused): `change` (an existing change's id, to add a revision; absent opens a change), `title`
(required for a new change, at most 200 characters), `description` (at most 64 KiB), `target_ref` (advisory), `base` (the block id
the artifact was built against), `declared_patches` (the 64-hex patch ids the artifact contains; absent for a push by plain
prikk), `client` (name and version). An existing change takes no `title`, `description` or `target_ref`.

The answer is `201` (a change was opened) or `200` (a revision was added, or the artifact was already recorded), with the change
and revision ids, the claims and what accept said of each, how many patches the push added and the first 1,000 of them with their
assurance, whether a declaration was kept, and a one-sentence message in the caller's language: the work is held for review and no
branch has changed. A pending patch is `forge-approved-unsealed` (in plain words: **accepted and held by this server, not yet signed into the history; nobody has approved it for merging** — the word *approved* is part of a machine value that is in the 0.4.0 contract and is not a review decision): **never** `sealed` (that needs a block read from `log`), and
`queued` is never read (U-10, IS-47).

## The order of the checks

Everything before the accept is decided without writing anything; the first rule that fails names the refusal.

| # | Check | Refusal |
|---|---|---|
| 1 | the rate limit (per address and per signed-in user; keyed by the caller only, so it says nothing about any repository) | `429 rate_limited` + `Retry-After` |
| 2 | the read gate: a repository the caller may not read, or that does not exist | `404 not_found` (identical for both) |
| 3 | `propose` authorization: a reader who may not propose | `403 forbidden` |
| 4 | the artifact store is configured | `503 transport_off` |
| 5 | the repository is not waiting for recovery | `503 needs_recovery` (reads are unaffected) |
| 6 | the framing and the size: length, content type, magic, the metadata's length, the artifact's size | `411`, `400 invalid_request` (+ `reason`), `413 payload_too_large` |
| 7 | the metadata: JSON, no unknown field, every field's shape | `400 invalid_request` (+ `reason`) |
| 8 | the change it names exists here, is open, and is the caller's own or the caller holds `write` | `422 unknown_change`, `422 change_not_open`, `403 not_your_change` |
| 9 | the declared base is listed by `log` of a ref the repository holds (never `show <base>`, PK-57) | `422 base_unknown` |
| 10 | a free upload slot (at most `PLANETER_PROPOSE_CONCURRENCY`), then the body is read to disk | `503 busy` + `Retry-After` |
| 11 | **a repeat**: the artifact's digest already has a revision | `200`, that revision, nothing recorded |
| 12 | the quota (RFC 005 Q-2: open changes, accepted bytes, revisions per change, repository capacity) | `422 quota_exceeded` (+ `quota`, `limit`, `used`, `unit`) |
| 13 | admission against the memory budget (heavy pool, cost 10 MiB + 4.2 × the artifact), then the repository's write lease | `422 too_large`, `503 busy` + `Retry-After` |
| 14 | the accept itself (below) | see the failure table |

The quota is worked out at step 7 but **held back until step 12**: a repeat of an artifact that is already recorded must be
answered with its revision even when the caller is now at a limit (a client that lost the answer and sends the same artifact
again). Two consequences are deliberate: a request that breaks both the metadata and the quota gets the metadata's error; a
request that breaks the base and the quota gets the base's.

## The life of an intent

```
                 open_or_resume_intent                                  complete_intent_with_revision
   (none) ───────────────────────────────▶ open ───── exit 0, derived, recorded ──────────────▶ complete
                                              │  ▲                                                (revision_id)
                    prikk refused,            │  │ the same artifact sent again
                    nothing changed           │  │ (attempts + 1; the FIRST attempt's
                    or "nothing new"          │  │  pending_before is kept)
                                              ▼  │
                                          abandoned   (also: killed, timed out, unreadable, not recorded → stays open + needs_recovery)
```

* The intent is written **before** `sync accept` starts, holding the patches that were pending at the **first** attempt.
* Exit 0 **and** a readable report **and** a derivable revision → one transaction records the revision (and the change) and closes
  the intent. Nothing else ever creates a revision.
* A declaration that does not hold (`declaration_misses_added`, `declared_not_held`) leaves the accepted patches where they are
  (an accepted patch cannot be removed), leaves the intent **open** and the repository **not** flagged: a corrected re-send derives
  from the first attempt's list and records one revision.
* An accept that **added nothing** (`no_new_content`) closes the intent and records nothing.
* The flag `needs_recovery` is set by anything that may have changed the repository without a record: killed, past its deadline,
  past its memory ceiling, a crash or lock left behind, output that could not be read, a record that could not be saved. A server
  killed in the middle of a write could not set it, so **at the next start** every repository with a write-lease record left by
  another process is flagged (`reconcile_at_start`). **Recovery itself is part 4**; until then an operator clears the stale prikk
  locks (`prikk unlock --lock <path> --force`, `prikk doctor`), the flag and the stale lease record, and the client sends the same
  artifact again.

## What the client and the operator see

| Cause | Client | Operator (log, counters) |
|---|---|---|
| prikk refused the artifact and changed nothing (over its bounds, damaged, base unknown to prikk) | `422 accept_refused`, prikk's text verbatim in `detail` | `refusals{accept_refused}`; intent abandoned; **no flag** |
| the accept hit its deadline (`PLANETER_ACCEPT_DEADLINE_SECS`) and was killed | `503 incomplete`, `cause: accept_deadline`, `intent` | flag set; intent open; `proposals_incomplete`; a log line with the intent id |
| the accept hit its memory ceiling | `503 incomplete`, `accept_ceiling` | the same |
| the accept crashed, was killed, left a lock, or failed after changing the repository | `503 incomplete`, `accept_failed` | the same |
| the accept exited 0 but its output could not be read | `503 incomplete`, `accept_unreadable` | the same |
| the accept finished but its record could not be saved | `503 incomplete`, `accept_not_recorded` | the same; the retry resumes the intent |
| the server was killed during the accept | the connection drops | the next start flags the repository; the intent is open with the first attempt's list |
| the repository is flagged | `503 needs_recovery` | the flag's reason is in the `repo_flags` table and the start-up log |
| another export or write holds the repository | `503 busy` + `Retry-After` | `lease_wait` counters |

In every `503 incomplete` case the hint is the same: **send the same artifact again once the repository has recovered.** The
uploaded artifact is kept in the store; the retry sends it again and resumes the intent.

## The operator's settings

| Variable | Default | Meaning |
|---|---|---|
| `PLANETER_ARTIFACT_DIR` | unset | the artifact store; **required** for proposals (unset: `503 transport_off`) |
| `PRIKK_EXCHANGE_MAX_BYTES` | 268435456 | the largest artifact: refused at the edge and again by prikk |
| `PRIKK_EXCHANGE_MAX_OBJECTS` | 5000 | the most patches, and the most blobs, an artifact may declare (prikk refuses a larger one; nothing is accepted). Was 20000 before part 4b: the accept of 20,000 patches took 11 minutes, over the 600 s deadline |
| `PLANETER_ACCEPT_DEADLINE_SECS` | 600 | wall-time bound of one accept (never the 120 s read bound) |
| `PLANETER_PROPOSE_CONCURRENCY` | 2 | proposals uploading or accepting at once |
| `PLANETER_PROPOSE_LEASE_WAIT_SECS` | 5 | the wait for an upload slot and the write lease before `503 busy` |
| `PLANETER_PROPOSAL_CAPACITY_MB` | 4096 | artifacts a repository retains for open and recently finished changes |
| `PLANETER_PRIKK_MEMORY_BUDGET_MB` | see `--help` | the budget an accept draws from, like a read, always from the **heavy** pool |

An accept of an artifact of *A* bytes under an object bound of *N* is admitted at **10 MiB + 7.0 × *A* + 2 KiB × *N*** and given a
data-segment ceiling (`RLIMIT_DATA`) of 1.5 times that. The constants are in one place, `cost_model` in
`crates/planeter-transport/src/propose.rs`, each annotated with its measurement (prikk 0.53.0, 2026-10-22, part 4b): a single
large blob needs 7.0 × its size whatever it holds, artifacts of many files or patches 4.0 to 4.3 ×, and each patch about 1.7 KiB more.
At the default bounds (256 MiB, 5,000 objects) that is about **1.8 GiB** of the budget and a ceiling of about 2.7 GiB: the start-up log
says whether the heavy pool admits it (it does on a host with about 8.1 GiB or more of usable memory at the default budget), and the
operator raises the budget or lowers `PRIKK_EXCHANGE_MAX_BYTES` if it does not. A proposal larger than the heavy pool is refused as
too large *before* anything is accepted; one that is admitted is not killed by its own ceiling.

## Limits worth knowing

* A declaration that fails after a successful accept leaves its patches in the repository: an accepted patch cannot be removed
  from a prikk store. They are counted by nothing until a revision records the artifact; rate limits and the repository's disk are
  what bound a hostile repeat of this.
* Two proposals of the same artifact by different people while one is unfinished: the second gets `409 intent_pending`.
* The accept's wall time follows the number of objects (patches and blobs), not the bytes: about one disk flush each, so 15 to 50 ms on the test machine's disk, depending on what else the host is doing (and well under a millisecond on a RAM disk): 5,000 patches in 70 to 230 s, and 20,000 could pass the 600 s deadline. The default bound keeps the worst accept inside it; the measurements are in the part 4b review request.

## Recovery: the runbook (RFC 004 part 4)

A repository the write side flagged `needs_recovery` refuses writes (`503 needs_recovery`) and still answers reads. The server
recovers it **by itself** when every witness agrees, at start and right after an accept that did not finish; otherwise an
administrator does, through `GET` and `POST /api/v1/repos/{owner}/{name}/recovery` (the `RepoAdmin` action: the owner and
administrators of the repository; a token must have the `full` scope; bearer token only). `PLANETER_AUTOMATIC_RECOVERY=off`
turns the unattended runs off; the door works either way.

### The witnesses

| Witness | What it reads | What disagreement looks like | Unattended | Administrator's run |
|---|---|---|---|---|
| the flag's **cause** | the reason stored with the flag | anything but `accept_deadline`, `accept_ceiling`, `accept_failed`, `accept_unreadable`, `accept_not_recorded`, `server_stopped` (blocker `cause_not_ours`) | stops | may go on |
| the **lease table** | the durable holder records of the repository; a record written by another boot of the server is a dead holder (the server runs once per database, so that process is gone), a record of this boot is a lease held now | a live holder (`lease_held`); the run is *not now* and may be tried again | stops | stops |
| **other processes** | `/proc`: any `prikk` or `bwrap` process that is not a descendant of the server and that (a) names the repository's directory on its command line, **or (b) has its working directory in the repository, or (c) holds a file of the repository's `.prikk` open** (a person who `cd`s into the repository and runs `prikk status` is found by (b)) | `foreign_process` (a second server, an operator's own `prikk`, a survivor of the killed one) | stops | stops |
| **processes it may not inspect** | the same tools whose working directory or open files the system would not let the server read (another user's), and any process it cannot even identify | `processes_uninspected`: stated as a warning with the count; it is neither "safe" nor "unsafe" | stops | may go on (a person who knows the host decides) |
| **`doctor`** | its report (strictly read: it must add up to its own summary) | any error or warning (`doctor_problems`); an unknown shape (`output_unreadable`, prikk's text kept) | stops | stops, **except** when *every* problem the report lists names a repair on the idempotent allow-list (`--repair-wal-tail`, `--repair-index`, `--repair-pointer-index-tail`, `--repair-tails`): then the run executes exactly those repairs, and `doctor` must be clean afterwards. One problem that names no repair, or names any repair off the list (even beside an allowed one: `--discard-damaged-commits`, `--restore-queue-target`, `--cancel-import`, `--rebuild-pointer-index`, `--repair-main-ref`), stops everyone and no repair is run; it shows only as text in the plan |
| **`unlock`** | the listing (strictly read); only locks of the kinds an accept leaves (`active`, `container:object-store`) that resolve to a `.lock` file inside the repository's own `.prikk` directory | any other kind or place (`unexpected_lock`); an unknown shape (`output_unreadable`) | stops | stops |
| the **open intents** | the repository's unfinished proposals | more than the dead accepts the lease table accounts for plus 8 (`too_many_open_intents`; a refused declaration leaves an open intent that is not a crash) | stops | may go on |

`unlock`'s own liveness line ("does not appear to be running") is **kept in the record and never trusted**: inside our PID
namespace it is wrong (U-16). The decision rests on the table above.

### What a run does, in order

1. read the witnesses (**step `witnesses`**); 2. take the repository's **write lease** (`lease`; a live holder ends the run as
*not now*, and two runs at once are serialized by it: the second finds the flag cleared and does nothing); 3. read the witnesses
again under the lease (`witnesses-under-lease`) and `doctor` and `unlock`'s listing (`doctor`, `unlock-list`); 4. an
administrator's run only, and only when every problem `doctor` lists names an allow-listed repair: those repairs, each in a step written before it is taken, with prikk's output (and any run id) in its text (`repair`), then `doctor` again, which must be clean; 5. `unlock --lock
<path> --force` for each listed lock (`unlock-force`, then `unlock-list`, which must be empty); 6. a **fresh `verify`**, which
must pass (`verify`): **no flag is cleared without it**; 7. the intents an administrator named are abandoned
(`abandon-intent`; their accepted patches stay in the repository and their bytes stop counting); 8. the dead lease records are
removed, the flag is cleared and the state fingerprint moves (`clear-lease-records`, `clear-flag`). A healthy flagged
repository goes through the same steps with nothing to unlock. A step that fails stops the run, leaves the flag set, and the
plan names the commands to run by hand.

### What the process witness can and cannot see

It reads `/proc` of the host the server runs on. It **can** see a `prikk` or `bwrap` process of the same host that names the
repository, sits in it, or holds one of its store's files, unless it descends from the server (the server's own reads). It **cannot**
see a process in **another PID namespace** (another container that mounts the same directory), a process on **another host** that
shares the directory over a network file system (prikk documents nothing about shared file systems; the server is meant to be the
repository's only writer), or the inside of another user's process (that case is counted and stated as `processes_uninspected`, not
guessed). Where the directory is shared, the operator's own check, under "When it stays flagged", is the one that counts.

### The record

Every run has a **recovery record** (table `recoveries`, one row per step in `recovery_steps`): who ran it (`automatic` or the
administrator), when, the steps with their outcomes and the **text of each prikk call**. A step is written *before* it is taken
and completed after, so a record whose last step has no end shows where a run that was itself interrupted stopped. `GET …/recovery`
shows the last ten. Outcomes: `recovered`, `stopped`, `not_now`, `nothing_to_do`.

### When it stays flagged: the commands

`GET …/recovery` returns the plan: the flag and its cause in words, the lease holders (`dead` or not), the foreign processes, the
open intents (id, submitter, age, bytes, attempts, `expired` past 30 days), the locks, `doctor`'s verdict, the **blockers** (what
disagrees, and whether an administrator's run may go on regardless), the commands, and whether the server could run it by itself.
By hand, in the repository's directory: look for processes (`ps -eo pid,args | grep -E 'prikk|bwrap'`), read `prikk doctor`,
list with `prikk unlock`, clear a lock you have confirmed is stale with `prikk unlock --lock <path> --force`, and check with
`prikk verify`; then `POST …/recovery` to clear the flag (it repeats the witnesses and the `verify`).

### After a wrong repair

`prikk doctor --recovery-list` lists the saved entries grouped by repair run; `prikk doctor --recovery-restore <run id>
[--plan-only]` undoes one run (it refuses unless every file still holds what the run left). The run id of a repair planeter ran
is in the text of its `repair` step.

### What an open intent costs

An accept whose declaration is refused has already put its patches in the repository, and they cannot be removed. **Its artifact
is kept, and its bytes are owed**: while the intent is open (and has no revision) they count against the submitter's accepted
bytes and the repository's retained bytes, so different artifacts with bad declarations reach the limit. A corrected re-send
records its revision and the bytes are counted once; an administrator may abandon the intent (it then stops counting).
