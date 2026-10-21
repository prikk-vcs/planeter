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
branch has changed. A pending patch is `forge-approved-unsealed`: **never** `sealed` (that needs a block read from `log`), and
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
| `PRIKK_EXCHANGE_MAX_OBJECTS` | 20000 | the most objects an artifact may declare (prikk refuses a larger one) |
| `PLANETER_ACCEPT_DEADLINE_SECS` | 600 | wall-time bound of one accept (never the 120 s read bound) |
| `PLANETER_PROPOSE_CONCURRENCY` | 2 | proposals uploading or accepting at once |
| `PLANETER_PROPOSE_LEASE_WAIT_SECS` | 5 | the wait for an upload slot and the write lease before `503 busy` |
| `PLANETER_PROPOSAL_CAPACITY_MB` | 4096 | artifacts a repository retains for open and recently finished changes |
| `PLANETER_PRIKK_MEMORY_BUDGET_MB` | see `--help` | the budget an accept draws from, like a read, always from the **heavy** pool |

An accept of an artifact of *A* bytes is admitted at 10 MiB + 4.2 × *A* (measured, U-12) and given an address-space ceiling of 1.5
times that. At the default bound (256 MiB) that is about 1.1 GiB: the start-up log says whether the heavy pool admits it, and the
operator raises the budget or lowers the bound if it does not.

## Limits worth knowing

* A declaration that fails after a successful accept leaves its patches in the repository: an accepted patch cannot be removed
  from a prikk store. They are counted by nothing until a revision records the artifact; rate limits and the repository's disk are
  what bound a hostile repeat of this.
* Two proposals of the same artifact by different people while one is unfinished: the second gets `409 intent_pending`.
* The accept's wall time follows the number of patches, not the bytes (about 14 s for 1,000 patches on the test machine).
