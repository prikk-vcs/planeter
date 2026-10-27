# Proposals, revisions and the intent journal

| | |
|---|---|
| Document | The design note for the data and the rules behind a **proposal** (RFC 005 D-1…D-3, D-6, D-9, D-10; RFC 004 D-3, D-6), and — from RFC 005 part 5 — what a person sees of it: landing observed from facts, the read API and pages, closing, the download of an accepted artifact, and the retention sweep; and — from part 6 — **review**: comments, decisions and moderation (the sections at the end). Records and pure rules were built in RFC 004 part 3a. |
| Where | Records: `crates/planeter-store/src/change.rs`. Rules: `crates/planeter-core/src/change.rs`. Tests: `crates/planeter-store/tests/change_store.rs`, `crates/planeter-core/tests/change_model.rs`; for review `crates/planeter-store/tests/review_store.rs`, `crates/planeter-core/tests/review_model.rs`, `crates/planeter-transport/tests/reviews_unit.rs`, `crates/planeter-web/tests/reviews_http.rs`. |
| **Words** | **The code's internal name for a proposal is *change*** (`ChangeRecord`, the `changes` table, `ChangeStore`, `change_id` columns): they were named before the vocabulary was settled, and nothing stored is renamed. In every word a person or a program reads — the pages, the API, the messages, the documentation — it is a **proposal**, its versions are **revisions**, and *change* keeps the glossary's meaning, a recorded unit of history (prikk's block). `docs/glossary.md` lists the words. |

## What is stored

* A **proposal** (internally a *change*) has a title, a description (text; shown only through the sanitizer, later), the ref the person
  hopes it will reach (advisory), who opened it, and a state — `open`, `landed` or `closed` (with a reason). It leaves `open`
  once and never comes back. When it leaves, `retain_until` is set (30 days on by default): the earliest time its files may
  be reclaimed.
* A **revision** is one accepted artifact of a proposal: its digest and size, the base the person says it was built on, the
  claims it carried and what accept said about each, who sent it, and two lists of **patch ids**: the ones this accept
  *added*, and — if the person's helper sent them — the ones they *declare* the artifact contains. A push by plain prikk has
  no declaration, and the record says so (the screen will not claim a completeness it does not have). Identical artifacts
  are one revision (`UNIQUE(repo, digest)`); a proposal numbers its revisions 1, 2, … once (`UNIQUE(change, seq)`).
* An **intent** is the journal entry written *before* an accept starts: the artifact's digest, who is sending it, and the
  patch ids that were already pending **at the first attempt**. It is `open` until the accept has finished and its revision
  is recorded (`complete`), or someone gives up (`abandoned`). A retry of the same artifact *resumes* the open intent
  (`attempts + 1`); the first list is never overwritten.
* A **repository flag** says `needs-recovery`, with a reason and a time, after an accept that did not finish.

**What is never stored:** patch content, block content, history — only ids (NG-6). The forge holds the artifact itself in the
spool/store; these tables only point at it. No counters either: how many open proposals someone has, how many bytes were
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
5. **Landing is observed, not asserted.** A proposal has *landed* only when **every** patch of its current patch set is found in
   a block that `log` lists for a **published** ref of the repository (D-40; see "Landing, observed from facts" below). The current patch set is **the latest declared revision's declared set plus the patches every later revision
   added** (all the added patches, if no revision is declared): a declared patch never silently drops out because a later push
   was undeclared. A superseded patch that nobody seals keeps such a proposal open, and the page says so.
6. **Closing needs a reason** and a person already allowed to close. Landed and closed are final.
7. **Quotas** (members / any-authenticated): open proposals 10 / 3, accepted bytes 512 / 64 MiB, revisions per proposal 20 / 10,
   and a repository ceiling of 4 GiB of **retained** bytes (the artifacts of open proposals and of proposals still within their 30-day grace: what occupies the disk). At the limit is allowed; one over is refused, naming the quota, the
   limit, the current use and what to do. The per-push bounds belong to the transport edge (part 3b).
8. **A patch is "signed into the history" only if a block of a published ref, read from `log`, contains it** — and that wins over
   `sync pending`, which an import can leave listing the patch (D-40 reversed part 3a's "pending wins"). Otherwise it is "held" if
   `sync pending` lists it, and "not listed" if neither does. `queued` is never read: a pending patch reports `queued: false`.

## What part 3a did not do

No route, no screen, no API, no call to prikk, no comments or review decisions, no deletion of expired artifacts, no change to
who may do what. **Two open intents for different artifacts can exist at this layer**; that they never run at once is the job
of the repository's write lease (part 3b takes it before opening an intent). `open_intents` lists them so a caller can
recover.

# What a person sees of a proposal (RFC 005 part 5)

## Landing, observed from facts

A proposal moves from `open` to `landed` only because the repository says so. The rule (RFC 005 D-6 as amended by D-40, applied to
the proposal's current patch set, rule 5 above):

* **every patch is present in a block that `log` lists for a published ref** — `heads/*`, as `branch --all` reports them as
  branches (closed ones too) — and nothing else.

Why only that. `sync pending` is **not consulted**: a patch can be at once in the queue and in an imported published block, and a patch
that merely left the queue (discarded, or the repository was restored) is not in the history. A **received** ref (`remotes/*`) is
**not evidence**: it is last-writer-wins and unverified (PK-67), and a landed proposal never changes again, so history that was merely
*received* here cannot make a state final. On this forge a published ref advances only when a key-holding action puts history there
(which D-8 will define), so until then **a proposal usually stays open**, and the page says only what is true.

A patch in a block of a published ref is shown as signed in (*Sealed*), even if `sync pending` still lists it; a patch that
is listed there and in no published block is shown as held; a patch in neither shows no badge ("Not listed here"). A **received**
ref that holds any patch of an *open* proposal is shown under *Details* as a **sighting** ("Also found in history received here (not
verified here): `remotes/…`", and that a received history can be replaced); the JSON has `received_sightings: [{ref}]`. Nothing is
derived from a sighting, and failing to read the received logs means no sightings, never an error.

How often prikk is asked is bounded and does not grow with the number of proposals:

* **one `sync pending` per repository per state fingerprint** (`ExportService::state`: the revision counter and the files cross-check);
  while the fingerprint is the same, every list, detail and landing pass in that state shares the answer;
* **one `branch` and one `log` per published ref per fingerprint**, read only when some proposal is open (every open proposal is a
  candidate) or a detail needs them; up to 100,000 blocks a ref. (prikk's own default is the newest 20 blocks, so the limit is passed; a
  log that fills the limit may have been cut short, and then nothing lands on it.) The logs of the **received** refs are read only for
  the detail of an open proposal, once per fingerprint;
* **no prikk call per item**: a list of fifty proposals makes the same calls as a list of one; with no proposal open, a list reads no log.

The transition is a **compare-and-set**: it applies only if the proposal is still open **and** has exactly the number of revisions the
observation was made on. A revision recorded meanwhile wins and the observation is discarded; the next one starts from the new set. The
transition starts the grace (`retain_until`, below). Landed is final.

If the repository cannot be read (no prikk, a damaged store, a busy gate) the records are still listed as they stand and the patches
carry no badge (nothing is claimed about a patch whose history could not be read); the next request tries again.

The observation's memory ceiling is at least 64 MiB. Measured on prikk 0.54.1 with 200 sealed blocks: under 1 MiB with nothing
pending, under 6 MiB with 5,000 patches pending (about 1 KiB a patch), so 64 MiB holds some 60,000 pending patches; each call takes
a few hundredths of a second.

## The API

All under `/api/v1/repos/{owner}/{name}/proposals`; JSON; the English or Japanese `message`/`hint` follow `Accept-Language` like every error.

| Route | Who | What |
|---|---|---|
| `GET …/proposals?state=open\|landed\|closed\|all&limit=&after=` | anyone who may read the repository | newest-updated first, `open` by default, `limit` at most 100 (default 50), `next` is the cursor for `after` |
| `GET …/proposals/{id}` | the same | the proposal in full: description (as sent and as sanitised HTML), revisions (who, when, claims as the server found them, key ids, the declared base, the digest, whether the file is still kept), at most 1,000 patches of the current set with `assurance` (`forge-approved-unsealed`, `sealed` or `null`), `still_pending`, the retention, and one plain sentence: *this server does not merge anything itself* |
| `POST …/proposals/{id}/close` `{"reason": "…"}` | the proposal's opener (with a credential that allows closing), or whoever may close (`write` and above); **bearer token only** — a cookie is not a credential here | the reason is required and at most 1,000 characters; `422 reason_required`, `422 proposal_final` (it says which state), `403 forbidden` with the hint for closing |
| `GET …/proposals/{id}/revisions/{seq}/artifact` | whoever holds `review` or `merge-seal`; a reader who holds neither gets `403 forbidden` with its own hint | the accepted artifact, byte for byte, `attachment`, `ETag` = the digest, `Range` and `If-Range`; `410 artifact_removed` (with `kept_until`) after the sweep |

Every route first goes through the one read gate: a repository the caller may not read, and one that does not exist, are the same `404`; an id
of another repository and an unknown one are the same `404`. The download is served from the artifact store and **never under a repository
lease**: the file is immutable.

## The pages

`/{owner}/{name}/proposals` (the list, with a filter between *Open*, *Landed*, *Closed* and *All*) and `/{owner}/{name}/proposals/{id}`.
The repository navigation shows *Proposals* only when the repository has at least one proposal or the viewer may propose to it; the
repository page says only how many are open.

The default view of one proposal shows: the title, who proposed it, the state (*Open*, *Landed*, *Closed*, always the neutral badge), one
sentence of what that means ("Held for review: not yet part of the project's history."), for a closed proposal one line "Closed: <reason>" (escaped, the reason cut at 200 characters), a one-line summary ("3 patches in 2 revisions"),
"The file-by-file view is not available yet.", and the actions the viewer may take — the download of the latest revision for a
reviewer, and the close form (a reason, the consequence stated: *Closing keeps the record and the files; nobody can reopen it.*) for whoever
may close. **Everything else is under one *Details*:** the full reason a proposal was closed, the received-history sightings, the description (through the sanitizer), the
revisions, the patches with their badges and the explanation of each, the claims and signature outcomes, key ids, the base each revision
was built on, the digests, and what is left of the files. No review state and no checks are shown: they would be statements about things that
do not exist yet. No script is needed; the close form is an ordinary form with a CSRF token, and a refusal (no reason, already final, not
allowed) is a page with a way back.

## Closing

Closing is for an **open** proposal and needs a reason. The record and the files stay (the grace starts); nobody can reopen a closed
proposal — a new one is opened for further work. Who may: the person who opened it, or anyone `authorize(Close)` allows. A read-only
credential of the opener cannot close; a stranger who can read gets the plain refusal.

## Retention and the sweep

A proposal that leaves `open` keeps its files for a grace (30 days by default, `PLANETER_PROPOSAL_GRACE_DAYS`). After it, the **retention
sweep** may remove the **files** (never the records): it runs at start and every `PLANETER_RETENTION_SWEEP_MINUTES` (default 60), looks at
a bounded batch of expired revisions at a time (and goes on from where it stopped), and removes a file only if nothing needs it —
no revision of an open proposal, none within its grace (in any repository: one artifact sent twice is one file), and no open intent. The
boundary is inclusive: at `retain_until` itself the file is still kept. A removal is a rename into a `trash/` directory, a look at the records again, and
then the unlink (or the rename back); a kill between the two leaves a file in the trash that the next start clears. Only one sweep runs
at a time. After it, the record says *removed* and the download is `410 artifact_removed` with the date it was kept until.

| Variable | Default | Meaning |
|---|---|---|
| `PLANETER_PROPOSAL_GRACE_DAYS` | 30 | days a landed or closed proposal keeps its files |
| `PLANETER_RETENTION_SWEEP_MINUTES` | 60 | minutes between sweeps |

# Review: comments, decisions and moderation (RFC 005 part 6)

People talk about a proposal, and reviewers say what they think of a **revision**. Nothing here merges, blocks or lands anything: the
state of a proposal still comes from the repository (landing) and from a person who closes it, and never from a comment or a decision.

## The rules

The pure rules are `crates/planeter-core/src/review.rs`; the service that applies them to the records is `planeter_transport::ReviewService`
(no prikk call, and no counter: the limits are computed from the records). Each row is pinned by a test, and each has a negative.

| # | Rule | Test (a negative in each) |
|---|---|---|
| 1 | **Who.** A comment needs `authorize(Comment)` (any signed-in caller who can read the repository, a read-only token included); a decision `authorize(Review)` (`write` and above, not a read-only token); hiding `authorize(RepoAdmin)`. All three read the repository through the one read gate first: a stranger is `404`. | `reviews_http.rs`: `who_may_comment_…`, `only_reviewers_decide_…`, `a_hidden_comments_text_is_released_to_administrators_only` |
| 2 | **When.** Comments are accepted on an open proposal and, until its `retain_until` (inclusive; 30 days after it finished by default), on a landed or closed one. Decisions only while it is open. | `review_model.rs`: `comments_are_accepted_while_open_…`, `decisions_are_accepted_only_while_the_proposal_is_open`; `reviews_http.rs`: `comments_close_when_the_grace_…` |
| 3 | **No self-approval.** The submitter of a revision cannot approve it, and nobody who submitted any revision of the proposal can approve its latest. Asking for changes and commenting are allowed. | `review_model.rs`: the two `…cannot_approve…` tests; `reviews_http.rs`: `nobody_approves_their_own_work_…` |
| 4 | **A decision belongs to one revision**, the latest unless named; naming an older one is `revision_not_current`. Recording another for the same reviewer and revision replaces it (the replaced one stays in a history). | `review_model.rs`: `naming_another_revision_is_refused_…`; `review_store.rs`: `a_second_decision_replaces_…` |
| 5 | **Never carried over.** What a later revision shows of an earlier approval is `approved_on: [1]`; it counts for nothing. | `review_model.rs`: `a_stale_approval_never_counts_…`; `reviews_http.rs`: `an_approval_of_revision_one_is_shown_on_revision_two_…` |
| 6 | **The review state** is a function of the current decisions on the **latest** revision and of nothing else: `changes_requested` if any reviewer's current decision is `request-changes`; else `approved` if at least one is `approve`; else `awaiting_review`. Counts are of distinct reviewers. | `review_model.rs`: the `review_standing` tests |
| 7 | **A comment** has a body (1 to 16 KiB of Markdown, shown through the sanitizer), and may be anchored to a patch of the proposal's *current* set (kept as written), a path (free text, never opened, at most 1,024 characters) and lines (positive, `from ≤ to`, only with a path). A thread is one level deep, inside one proposal. | `review_model.rs`: the `check_comment` tests; `reviews_http.rs`: `every_comment_rule_has_its_named_refusal` |
| 8 | **Comments are immutable and never deleted**: no edit, no delete by the author (a typo is a new reply). SQLite refuses an edit or a delete itself (triggers). | `review_store.rs`: `sqlite_refuses_an_edit_or_a_delete_…` |
| 9 | **Hiding** is the only moderation: `RepoAdmin`, with a reason (at most 500 characters), reversible, and logged. A hidden comment's text goes to administrators only (HTML and JSON alike); everyone else sees "Hidden by a maintainer: *reason*". | `reviews_http.rs`: `a_hidden_comments_text_is_released_…`, `a_hidden_comments_text_is_never_in_a_page_…` |
| 10 | **Limits**, computed from the records: at most `PLANETER_COMMENTS_PER_HOUR` (20) comments and `PLANETER_DECISIONS_PER_HOUR` (10) decisions per principal per proposal per hour (a replaced decision still counts); `429` with `Retry-After`. A request refused for a rule was never recorded, so it never counts. | `review_model.rs`: `the_rate_limit_…`; `reviews_unit.rs`: `the_comment_limit_…`, `the_decision_limit_…`; `reviews_http.rs`: the two `…_429_…` tests |

## The records

`comments` and `decisions` (plus `decision_history` and `comment_moderation`, both append-only) sit beside the change tables in the same database, created
idempotently the first time the store is opened (`planeter_store::review`). A proposal's id is the key (`change_id`). The database enforces what
it can: the `CHECK`s on lengths, lines and the all-or-none moderation columns; a trigger that refuses any edit of a comment but its moderation columns and any delete;
a trigger that keeps a thread one level deep within one proposal; the primary key `(proposal, revision, reviewer)` that makes "one current decision". Recording a decision
is one transaction (the replaced decision moves to the history, the new one is written), so two writers at once leave exactly one current decision.

## The API

Writes are **bearer-token only** (a cookie is not looked at), like `propose` and `close`. All under `/api/v1/repos/{owner}/{name}/proposals/{id}`:

| Route | Who | What |
|---|---|---|
| `POST …/comments` `{body, patch_id?, path?, line_from?, line_to?, parent?}` | `Comment` | `201` with the comment |
| `POST …/decisions` `{decision: "approve"\|"request-changes", note?, revision?}` | `Review` | `200` with the decision and the review as it now stands |
| `POST …/comments/{cid}/hide` `{reason}` and `…/unhide` | `RepoAdmin` | `200` with the comment as an administrator sees it |
| `GET …/proposals/{id}?limit=&after=` | anyone who may read | gains `review` (state, revision, counts, current decisions with reviewer names and notes, `approved_on` / `requested_changes_on`, the decisions on earlier revisions) and `comments` (`total`, a page of threads oldest first with their replies nested, `next`) |
| `GET …/proposals` | the same | each open proposal gains `review` (the same three words) |

Refusals have their own codes: `comments_closed`, `decisions_closed`, `revision_not_current`, `self_approval`, `comment_state`, `rate_limited` (with `Retry-After`), and
`invalid_request` with a `reason` for each rule of a comment, a decision and a hide.

## The pages

The default view of an **open** proposal gains one line — *Awaiting review*, *Approved by 2 people* or *Changes requested by 1 person* — with its plain meaning ("Approved here
means reviewers said they are happy with revision 3. It does not merge anything."), and, when the only approvals were for earlier revisions, "Approved on an earlier
revision; revision 3 has not been reviewed." Everything else is under *Details*: the decisions (who, which revision, the note), the decisions on earlier revisions (history), the thread of comments with
their anchors, and the forms the viewer may use — a comment form for any signed-in reader (a patch is named by the first characters of its id), a reply form under each thread, a decision
form for reviewers (the submitter's *Approve* is disabled, with the reason), hide and show for administrators. Each form says its consequence beside its button; each is an ordinary form with a CSRF token; a refusal is a page with a way back.
The proposals list shows the review word of each open proposal. An anonymous reader sees the comments and no forms.

| Variable | Default | Meaning |
|---|---|---|
| `PLANETER_COMMENTS_PER_HOUR` | 20 | most comments one person may add to one proposal in an hour |
| `PLANETER_DECISIONS_PER_HOUR` | 10 | most review decisions one person may record on one proposal in an hour |
