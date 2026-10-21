# Changes, revisions and the intent journal

| | |
|---|---|
| Document | The design note for the data and the rules behind a **proposed change** (RFC 005 D-1…D-3, D-6, D-9, D-10; RFC 004 D-3, D-6). Built in RFC 004 part 3a: **records and pure rules only** — no route, no screen, no `sync accept` call yet (part 3b wires them; part 4 is recovery). |
| Where | Records: `crates/planeter-store/src/change.rs`. Rules: `crates/planeter-core/src/change.rs`. Tests: `crates/planeter-store/tests/change_store.rs`, `crates/planeter-core/tests/change_model.rs`. |

## What is stored

* A **change** is one proposal: a title, a description (text; shown only through the sanitizer, later), the ref the person
  hopes it will reach (advisory), who opened it, and a state — `open`, `landed` or `closed` (with a reason). It leaves `open`
  once and never comes back. When it leaves, `retain_until` is set (30 days on by default): the earliest time its files may
  be reclaimed.
* A **revision** is one accepted artifact of a change: its digest and size, the base the person says it was built on, the
  claims it carried and what accept said about each, who sent it, and two lists of **patch ids**: the ones this accept
  *added*, and — if the person's helper sent them — the ones they *declare* the artifact contains. A push by plain prikk has
  no declaration, and the record says so (the screen will not claim a completeness it does not have). Identical artifacts
  are one revision (`UNIQUE(repo, digest)`); a change numbers its revisions 1, 2, … once (`UNIQUE(change, seq)`).
* An **intent** is the journal entry written *before* an accept starts: the artifact's digest, who is sending it, and the
  patch ids that were already pending **at the first attempt**. It is `open` until the accept has finished and its revision
  is recorded (`complete`), or someone gives up (`abandoned`). A retry of the same artifact *resumes* the open intent
  (`attempts + 1`); the first list is never overwritten.
* A **repository flag** says `needs-recovery`, with a reason and a time, after an accept that did not finish.

**What is never stored:** patch content, block content, history — only ids (NG-6). The forge holds the artifact itself in the
spool/store; these tables only point at it. No counters either: how many open changes someone has, how many bytes were
accepted, how many are pending are **asked of the records each time**, so they cannot drift.

## How the lists are kept

Patch ids and claim ids are 64 hexadecimal characters; they are stored as **32 raw bytes each in one blob** (half the size,
order kept, and a malformed id is refused before it is written). Claim outcomes and author key ids are short, so they are
JSON text. An intent at the largest accept the transport allows (20,000 objects) holds 640,000 bytes of ids; writing it took
about 50 ms (one short transaction on the shared connection).

## The rules, in plain words

1. **A revision is derived from an intent record, never from two loose lists, and only while the intent is open.** What the
   accept added is "pending after, minus the pending-before of the *first* attempt". So a killed accept records nothing, and
   the retry — which finds a longer pending list — still derives exactly the patches the artifact added, once.
2. **A declaration is checked.** It must include everything the accept added, and name only patches the forge holds;
   otherwise the push is refused with the patches named. An accept that added nothing records nothing.
3. **The same artifact is the same revision.** A repeat finds it.
4. **A base counts only if the forge holds it** (listed by `log` of a ref it holds); no base is recorded as "no base", and
   its results never count. A result is stale when the base is no longer the target's tip; an approval belongs to the
   revision it was given for and is not carried to the next.
5. **Landing is observed, not asserted.** A change has *landed* only when every patch of its current patch set has left the
   pending list that `sync pending` just reported. The current patch set is **the latest declared revision's declared set
   plus the patches every later revision added** (all the added patches, if no revision is declared): a declared patch never
   silently drops out because a later push was undeclared. A superseded patch that nobody seals keeps such a change open,
   and the page will say so.
6. **Closing needs a reason** and a person already allowed to close. Landed and closed are final.
7. **Quotas** (members / any-authenticated): open changes 10 / 3, accepted bytes 512 / 64 MiB, revisions per change 20 / 10,
   and a repository ceiling of 4 GiB of **retained** bytes (the artifacts of open changes and of changes still within their 30-day grace: what occupies the disk). At the limit is allowed; one over is refused, naming the quota, the
   limit, the current use and what to do. The per-push bounds belong to the transport edge (part 3b).
8. **A patch is "signed into the history" only if a block read from `log` contains it**, and "pending" only if `sync pending`
   lists it. `queued` is never read: a pending patch reports `queued: false`.

## What this part does not do

No route, no screen, no API, no call to prikk, no comments or review decisions, no deletion of expired artifacts, no change to
who may do what. **Two open intents for different artifacts can exist at this layer**; that they never run at once is the job
of the repository's write lease (part 3b takes it before opening an intent). `open_intents` lists them so a caller can
recover.
