# Changelog

Releases are bare version tags; each section is the release's notes (assembled into the GitHub
release by `release.yml`). Kept by hand, per release, so nothing here goes stale by templating.

## [Unreleased]

- **Comments and review decisions (RFC 005 part 6).** People can now talk about a proposal and say what they think of a revision. **Comments**
  (Markdown, shown through the sanitizer) sit in one-level threads and may be anchored to a patch, a path and lines; they are never edited or deleted, and a maintainer
  who administers the repository can **hide** one with a reason (its text is then shown to administrators only, everyone else sees the reason) and show it again. A reviewer
  (`write` and above) records one **decision** per revision — approve or request changes — that is shown on that revision and **never carried to a later one**: a later revision shows
  "approved on revision 1" and counts it for nothing. Nobody approves their own work. The proposal page gains one **review line** (*Awaiting review*, *Approved by 2 people*,
  *Changes requested by 1 person*) and, under *Details*, the decisions, the comments and the forms; the list shows the word too. API: `POST …/comments`,
  `…/decisions`, `…/comments/{cid}/hide` and `…/unhide` (bearer tokens only), and `review` and `comments` in the proposal's detail. Limits: 20 comments and 10 decisions per person per proposal per hour
  (`PLANETER_COMMENTS_PER_HOUR`, `PLANETER_DECISIONS_PER_HOUR`). Nothing here merges or blocks anything. See `docs/proposals.md`.

- **Landing is observed from published history only (RFC 005 D-6 amended, D-40), and three small corrections.** A proposal has *landed*
  when every one of its patches is in a block of a **published** branch of the repository; `sync pending` is no longer consulted, and
  history merely **received** here (an archive import without `--adopt`) never counts — it is shown under *Details* as a sighting
  ("Also found in history received here (not verified here)"; `received_sightings` in the JSON). A patch in a published block is shown
  as signed in even if the queue still lists it. Until a key-holding action publishes history here, a proposal usually stays open; the
  documentation says so. A closed proposal now shows its reason beside the state (cut at 200 characters; the full text under *Details*).
  **Recovery no longer stops for prikk 0.54's "unreached object" warning** when it names no repair: it is a note in the plan and in the record
  of the `doctor` step, and the server recovers by itself (any other warning or error still stops it). **The server refuses to start
  with a prikk below 0.54.0**, with the pin's own sentence. The propose route's check that a declared patch is already held now reads all of a
  ref's history (prikk's own default of the newest 20 blocks missed older patches). `tokio` 1.53.1 → 1.53.3 (lock only; RUSTSEC-2026-0386).

- **Proposals you can see, and the word "proposal" everywhere (RFC 005 part 5).** What a person puts forward for review is now a
  **proposal** in every sentence and every name a person or a program reads: the pages, the API (`proposal_id`, `created_proposal`,
  `unknown_proposal`, `proposal_not_open`, `not_your_proposal`, the quotas `open_proposals` and `revisions_per_proposal`), the messages in both
  languages, and the documentation. (*Change* keeps its glossary meaning, a recorded unit of history; the code's internal name for a
  proposal stays *change*, and nothing stored is renamed.) **This is a rename of the propose API's fields and error codes that no release
  has carried yet**, so nothing outside the repository breaks.
  New: `GET /api/v1/repos/{owner}/{name}/proposals` and `…/proposals/{id}` (anyone who may read), `POST …/proposals/{id}/close` (the
  opener, or whoever may close; bearer token; a reason is required), and `GET …/proposals/{id}/revisions/{seq}/artifact` (for whoever holds
  `review` or `merge-seal`: the accepted files byte for byte, with `Range`); the pages `/{owner}/{name}/proposals` and `/{owner}/{name}/proposals/{id}`
  (the default view is a title, who proposed it, the state, one sentence, a one-line summary and the actions you may take; everything else is
  under *Details*); the repository navigation shows *Proposals* when there are some or you may propose.
  **A proposal now lands only when the repository says so:** every one of its patches has left `sync pending` **and** is in a block `log`
  lists (a patch that merely left the pending list is not landed), observed with one `sync pending` per repository per state, never
  one call per item. **A landed or closed proposal keeps its files for 30 days** (`PLANETER_PROPOSAL_GRACE_DAYS`), after which a retention
  sweep (`PLANETER_RETENTION_SWEEP_MINUTES`, hourly, also at start) removes the files nothing else needs and never the records; the download
  then answers `410 artifact_removed` with the date. See `docs/proposals.md`.

- **The `extract` driver and the `verify` line of prikk 0.54 (no page, no API yet).** planeter can now ask prikk for the files of a
  point of history (`prikk extract`) into a working directory it makes itself, outside the repository, and removes afterwards:
  prikk 0.54 does not refuse a destination inside the repository, so the driver never takes one from a caller. The extracted tree is
  treated as untrusted and is never read by the driver. prikk's `refusal-v1` document (13 ids) and `extract-report-v1` are read
  strictly; a claim whose parent is still pending is not reported as damage. `verify` now sees prikk's new `unreached objects`
  line and array (a warning, recorded and not shown). The memory cost of `extract` is measured and admitted like a read (about
  twice the largest file, up to 8.6 × for a file edited many times; admitted at 9 × the extraction bound + 8 MiB). See `docs/extract.md`.

- **The prikk floor moves to 0.54.0 (the 0.54 re-baseline).** planeter now requires prikk **0.54.0 or later** (the container image carries it;
  `MIN_PRIKK_VERSION`, the release workflow and the Dockerfile agree). Why: 0.54.0 fixes `show <block-id>` for a block that edits a
  file three or more times, which made the change page and API answer "temporarily unavailable" for such a block (known as IS-46,
  affecting 0.3.0 and 0.4.0 against prikk 0.52.0/0.53.0), and ships `extract` with the three things planeter asked for. An older prikk
  is refused when a repository is opened, with "unsupported prikk version X; planeter requires >= 0.54.0".

- **The default size of a proposal is 100 MiB (RFC 004 part 4c).** `PRIKK_EXCHANGE_MAX_BYTES` defaults to 104857600 (was 256 MiB):
  with the measured memory model the largest accept at 256 MiB needs about 1.8 GiB of the memory budget, which only a host with about
  8.1 GiB of usable memory admits at the default budget; at 100 MiB it needs about 0.7 GiB, which a 4 GiB host admits (up to 125 MiB).
  Operators with more memory raise it. The start-up log now says, in one sentence, what the configured memory budget admits and
  whether the configured bound fits. A proposal over the bound is the usual 413; the usual cause is a file edited many times (prikk
  keeps a full copy of the file for every edit it records), and `docs/propose.md` says so.

- **The accept's memory model is measured (RFC 004 part 4b, IS-52).** A single large blob needed 7.0 × its size in the
  accept's data segment, not the 4.2 × the admission and the ceiling assumed, so a proposal carrying one file of about 22 MiB or
  more was killed at its ceiling and left the repository waiting for recovery. The cost of an accept is now
  **10 MiB + 7.0 × the artifact + 2 KiB × the object bound** (the constants, each with its measurement, are in one place,
  `cost_model` in `planeter-transport`), the ceiling is still 1.5 × that; an artifact of the largest allowed size (256 MiB) is
  accepted end to end through the real sandbox. The price is a larger share of the memory budget for every proposal: the largest
  accept now needs about 1.8 GiB of the budget (was 1.1 GiB), so the default budget admits it on a host with about 8.1 GiB or more
  of usable memory; the start-up log says whether it does. **`PRIKK_EXCHANGE_MAX_OBJECTS` now defaults to 5000** (was 20000):
  the accept's time is one disk flush per patch or blob, 15 to 50 ms on the test machine, so 20,000 of them could pass the 600 s
  deadline. The bound counts patches and blobs separately.

- **Recovery follow-ups (RFC 004 part 4a).** An administrator's recovery run can now actually repair: when `doctor` lists problems
  and **every one of them names** one of the idempotent repairs (`--repair-wal-tail`, `--repair-index`,
  `--repair-pointer-index-tail`, `--repair-tails`), the run executes exactly those repairs (each step written first, with prikk's
  output), requires `doctor` to be clean afterwards, and goes on; one problem that names no repair, or any repair off the list,
  stops everyone and the repair appears only as text in the plan. Before this the repair step could not be reached (IS-51). An
  unattended run still never repairs. The process check for recovery now also finds a `prikk` that merely sits in the repository's
  directory or holds a file of its store open (not only one whose command line names it); processes it is not allowed to inspect are
  counted and stated in the plan (`processes_uninspected`), which stops an unattended run and not an administrator's. The plan's
  JSON gains `processes_uninspected` and `doctor.repair_refusal`. `docs/glossary.md` has a section of plain definitions for the
  words an operator meets (flag, lease, accept, intent, lock, recovery).

- **Recovery: a repository left unfinished by a write becomes writable again by a written, recorded procedure (RFC 004 part 4),
  and an accepted-but-unrecorded artifact is now counted (IS-50).** A repository that proposals flagged `needs_recovery` is
  recovered **by the server itself** — at start and right after the write that did not finish — only when every witness agrees:
  the flag is one of ours, the lease table has only dead holders, no process that is not the server's names the repository,
  `doctor` reports nothing, `unlock` lists only the locks an accept leaves inside the repository, and the open proposals are the
  ones the lease table accounts for. It then clears the stale locks (never on prikk's own liveness line, which is wrong inside our
  sandbox), requires a fresh `verify` to pass, removes the dead lease records and clears the flag. Anything else stays flagged for
  an administrator: `GET /api/v1/repos/{owner}/{name}/recovery` shows the plan (the flag and its cause, the holders, the processes,
  the locks, `doctor`'s verdict, what disagrees, and the exact commands); `POST` of the same path runs it (and may abandon named
  open proposals). Both need the repository-administration action and a bearer token of the `full` scope. Every run writes a
  **recovery record** (new tables `recoveries` and `recovery_steps`: who, when, each step and the text of each prikk call), each
  step before it is taken. New setting: `PLANETER_AUTOMATIC_RECOVERY` (`on` by default; `off` leaves recovery to an administrator),
  and `PLANETER_PROPOSER_QUOTA_MB`. **A quota's meaning changed:** an artifact whose declaration was refused is already accepted by
  prikk and cannot be removed, so it is now **kept and owed** — while its proposal is open its bytes count against the submitter's
  accepted bytes and the repository's retained bytes (the `intents` table gains `artifact_bytes`; existing rows read as 0); a
  corrected re-send records the revision and the bytes are counted once. The hints of three API errors are reworded (the
  `needs_recovery` hint is now two sentences) and the OpenAPI document says in plain words what `forge-approved-unsealed` means.
  Runbook: `docs/propose.md`.

- **Propose: a change can be put on the forge (RFC 004 part 3b).** With `PLANETER_ARTIFACT_DIR` set (and the export spool), an
  authenticated principal who may propose (`write` and above; any signed-in reader if the repository's `proposals` setting is
  `any-authenticated`) can `POST …/transport/propose` an artifact built by `prikk sync build`: it is checked, accepted under the
  repository's write lease with an intent journaled first, and recorded as a revision of a change. **Nothing is sealed, merged or
  published: the work is held for review and no branch changes.** The request is framed (`PLANETER-PROPOSE-1`, a metadata length,
  JSON metadata, the artifact), authenticated by bearer token only, bounded before its body is read, and streamed to disk. An accept
  that does not finish (killed, past its deadline, past its memory ceiling) leaves the repository **waiting for recovery**: writes are
  refused with `needs_recovery`, reads are not affected, and the same artifact sent again resumes it once an operator has cleared the
  stale locks (automatic recovery is the next part). **A server killed in the middle of a write now flags the repository when it
  starts.** New operator settings: `PRIKK_EXCHANGE_MAX_BYTES`, `PRIKK_EXCHANGE_MAX_OBJECTS`, `PLANETER_ACCEPT_DEADLINE_SECS`,
  `PLANETER_PROPOSE_CONCURRENCY`, `PLANETER_PROPOSE_LEASE_WAIT_SECS`, `PLANETER_PROPOSAL_CAPACITY_MB`. New API error codes (see the
  OpenAPI document): `forbidden`, `length_required`, `payload_too_large`, `needs_recovery`, `quota_exceeded`, `base_unknown`,
  `declaration_misses_added`, `declared_not_held`, `no_new_content`, `unknown_change`, `change_not_open`, `not_your_change`,
  `accept_refused`, `intent_pending`, `incomplete`. The capacity ceiling now counts every artifact still retained (open changes
  and changes within their 30-day grace), and a change's current patch set keeps a declared patch that a later undeclared push
  would have dropped. Design note: `docs/propose.md`.

- **The database gains the tables behind proposed changes (RFC 004 part 3a; nothing uses them yet).** Opening an existing
  database now also creates `changes`, `revisions`, `intents` and `repo_flags` (idempotent; every earlier table is
  untouched, and reopening changes nothing). Nothing writes to them until the transport's write side is built, so a
  server updated to this version behaves as before. The design note is `docs/proposals.md`. The export sandbox now mounts
  only a working directory of its own, never the whole spool (`<spool>/work/<id>`, created and removed per export;
  leftovers from a crash are cleaned when the server starts).

- **The verification page no longer shows "N of M sessions verified", and the verify endpoint no longer returns
  `verified_sessions` and `total_sessions` (IS-49) — a breaking change to the success body of
  `GET …/verify`.** prikk's `active_sessions` is not a tally of work (`count` is the number of session folders,
  always one; `verified_count` a fixed 1), so the sentence next to the badge suggested progress that does not exist.
  The checks, the verdict, the stages and the badge are unchanged.

- **The transport's read side: advertise and fetch (RFC 004 part 2), and a graceful shutdown.** With an export spool
  configured (`PLANETER_SPOOL_DIR`), a reader of a repository can ask what it holds
  (`GET /api/v1/repos/{owner}/{name}/transport/advertise`) and fetch it as one whole, **verified** artifact
  (`…/transport/fetch?kind=archive|bundle[&ref=…]`): `200`, `304` for an unchanged state, `206`/`416` for byte ranges,
  `503 busy` with `Retry-After` when exports are at capacity, `429` past the rate limit, and named refusals (unknown ref
  or kind, a bundle of a received ref, an artifact that did not verify — never served —, an export that failed or hit its
  deadline or memory ceiling). New operator settings: `PLANETER_EXPORT_CONCURRENCY`, `PLANETER_EXPORT_LEASE_WAIT_SECS`,
  `PLANETER_EXPORT_DEADLINE_SECS`, `PLANETER_EXPORT_MEMORY_MB`, `PLANETER_TRANSPORT_RATE_PER_MIN`. Reads of a repository
  are never blocked by an export. **The server now stops gracefully on SIGTERM and SIGINT**: it finishes the requests in
  flight (at most `PLANETER_SHUTDOWN_DRAIN_SECS`, default 30), removes its lock file and exits 0; a second signal stops it at
  once. **A CI job is never allowed to propose a change.** The dependency tree gains two small crates for the signals
  (`signal-hook-registry`, `errno`; deny and audit clean; see `docs/dependency-policy.md`).

- **A Japanese draft and a language switcher (RFC 012 D-3; IS-27) — Japanese is not shipped yet.** The footer of every
  page, the sign-in page included, now has a **language switcher**: a button for each available language, written in
  its own language (English, 日本語), the current one marked; choosing one remembers it in a cookie
  (`planeter_lang`, a language code only) and returns to the same page; no script. The Japanese catalog is a **draft
  for its two reviewers** (the owner and a colleague) and is offered **only** when the operator sets
  `PLANETER_PREVIEW_LANGUAGES=ja` (off by default, logged at start); without it Japanese is not offered and the
  switcher shows English only. English wording is unchanged. The Japanese typography uses system fonts only. New
  checks cover Japanese (completeness, placeholders, retired and forbidden terms, the glossary's Japanese tables,
  every page state), and a test pins that a language not signed off is not shipped.

- **Foundations for the transport (RFC 004, part 1) — nothing is reachable from the network that was not before.**
  (1) **A test that every route authorizes (IS-14).** Every route of the web surface is registered in one table with
  its authorization class — it checks `authorize()` or is on a short list of public routes, each with a written
  reason — and a test fails the build for a route that is not in it, or that is in it but leaks. (2) **A new
  permission, `propose`** (`Action::ALL` now has ten entries): granted to `write` and above; never to an anonymous
  visitor; to any signed-in user only when a repository's new setting `proposals` is `any-authenticated` (default
  `members`). The database gets a `proposals` column on first start, safely repeatable. No page or API uses it yet.
  (3) **A lease for each repository** (reads / exports / writes) with bounded waits, and a **record of who holds an
  exclusive lease**, so a crash is visible at the next start. (4) **Only one server per database:** a second server
  on the same database file is refused at start (`<database>.lock`; see RELEASING). (5) **An artifact store and an
  export spool** (`PLANETER_ARTIFACT_DIR`, `PLANETER_SPOOL_DIR`, `PLANETER_SPOOL_BUDGET_MB`; optional, unset by
  default): content-addressed, written atomically, the spool bounded and evicting its oldest. (6) **The sandbox can
  take two optional mounts** (accepted artifacts read-only; the export spool read-write); without them the
  sandbox is byte for byte what it was.

- **A missing file or folder is now said to be missing (IS-45).** In a repository you may read, a file or folder that
  is not there used to show "There is no repository at this address, or you do not have access to it." It now says
  **"This file or folder was not found in this repository. Check the path, or go back to the files."** with a link to
  the files. A repository that is missing, private or unreadable still gets the old sentence, and **nobody who may not
  read a repository can tell it apart from one that is not there**. **The API is unchanged**: a missing path is the
  same `404 not_found` (same headers and body) as a missing or hidden repository, and `GET …/tree` with a prefix that
  has nothing under it still answers `200` with no entries.

- **The change page no longer says "Verified" for a sealed patch (IS-44) — and the change API's `assurance`
  changes with it.** The change page showed the badge **Verified** for any patch that is signed into the history,
  although no verification runs on that page. A sealed patch is now labelled **Signed into the history** in a plain
  (not green) badge, and **Verified** appears only where a verification ran and passed: the repository, history and
  verification pages. **API (breaking for a client of `GET …/change/{target}`):** the `assurance` of a sealed
  patch is now `"sealed"` instead of `"prikk-verified"`; `"forge-approved-unsealed"` (queued) is unchanged, and
  `"prikk-verified"` remains a value of the schema but no change response carries it. A stage of `GET …/verify`
  that is `halted` now carries `after` (the id of the stage it stopped after), as a `not_evaluated` one carries
  `blocked_by`; `detail` is unchanged. Wording: the Verification page's failed explanation says "They are listed
  below."; the history page explains the *rollback* tag when it shows one; a stopped check reads "stopped early
  (after …)"; "Showing … · branch …" and the "Too large to show here" title follow their neighbours.

- **The pages use plain words (RFC 012 D-4; IS-30).** The wording follows `docs/glossary.md`, which the owner
  approved. The four trust badges: *prikk-verified* → **Verified**; *forge-approved, unsealed* → **Approved,
  awaiting signature**; *unverified* → **Verification failed**; *verify unavailable* → **Could not be checked
  now**. The repository menu: *code* → **Files**, *history* → **History**, *refs* → **Branches and tags**,
  *verify* → **Verification** (page titles follow). Every page that shows a badge has a **"What does this
  mean?"** block under its heading (a native expand/collapse control, no script) that says what the badge
  claims and what it does not; the change page also says what a *patch* and a *rollback* are. The verification
  table says **Check** instead of *Stage*, and **completed / problem found / not run (blocked by …) / stopped
  early** instead of *evaluated / failed / not_evaluated / halted*; a failed check lists what prikk found under
  **What was found**. Sign-in says **Username** and **Sign in with single sign-on**, and a failed sign-in,
  a missing page, an unreadable or too-large repository, a rejected form and each single-sign-on failure now say
  what to do next. **API:** `GET …/verify` adds `blocked_by` (the id of the blocking check) to a `not_evaluated`
  stage and omits it otherwise; every existing field, including `detail`, is unchanged; the raw route's
  too-large hint now says "ask the person who runs this server". One wording is **held**: "N of M sessions
  verified." stays as it was until prikk confirms what its `active_sessions` count means.

- **The pages look slightly different: bigger click areas, a visible focus ring, wrapping at narrow widths, dark
  form controls (RFC 012 D-2, D-3; IS-41).** Links, buttons and fields are at least 24 × 24 CSS px (rows are a few
  pixels taller); keyboard focus shows a 2 px ring in both themes; header links underline on hover and focus;
  long ids, file names and display names wrap, so **no page scrolls sideways at 320 px** (the change and history
  pages did, even at desktop width for a change id); form controls follow the dark theme (`color-scheme`) and
  have a visible border; a forced-colours (high-contrast) block keeps badges, borders and the focus ring
  distinguishable; layout uses logical CSS properties (a right-to-left language will mirror). No page structure,
  wording or API changed. `docs/accessibility.md` now states the border rule and what is tested automatically
  (a new stylesheet test) and what was measured.

- **Pages have a proper structure for keyboards and screen readers (RFC 012 D-2, D-5; IS-41, IS-30).** Every
  page now starts with a **"Skip to content"** link, and has a header, a main area and a **footer**
  (the product name and version) as landmarks; the repository menu is a named navigation block. Every page has
  one top-level heading (the file and folder pages and the refs page had none or two), the headings of a
  README or other Markdown file are shown one level lower than before, tables have captions and header cells,
  the folder/file icons (📁 📄) are replaced by the word "folder" in the size column, and the history rows are
  linked by the change's first message line (the short id stays beside it) instead of by a bare id. The "raw"
  link now reads "Download *file name*". Page **titles** follow one pattern (`page · context · planeter`), long
  ids are shortened to 12 characters, and the busy page, the unavailable page and a missing page no longer
  share one title; **a busy server now has its own page** ("The server is busy"), still with `Retry-After`.
  **Sign-in:** a failed sign-in is announced as an alert tied to the form, and **a locked-out account now says
  "Too many sign-in attempts. Wait a few minutes and try again."** (status 429, unchanged) — the same for a
  name that does not exist, so it tells no one which names do. **The home page's address field has a visible
  label, and a bad address in `/go` now shows the home page again with the error beside the field (status
  400)** instead of silently redirecting home. **The signed-in header shows your display name** with the
  account id beside it (the id alone when there is no name). The words are in the catalog; the API is
  unchanged. `docs/accessibility.md` is a first draft of the accessibility statement (no claim of conformance).

- **Every word the UI and the API's error bodies say now comes from a message catalog; pages that were bare
  strings are pages; sizes show KiB/MiB (RFC 012 D-3, D-5; IS-27).** There is still one language (English),
  and the pages say what they said, with these exceptions: sizes read `2.0 KiB` / `5.0 MiB` instead of raw
  bytes (`223 B` below 1 KiB is unchanged); `patch(es)` and `session(s)` become `1 patch` / `2 patches` and
  `1 of 1 session verified` / `1 of 2 sessions verified`. The server now **negotiates a language**: a
  `planeter_lang` cookie (a language code only; it is read but nothing sets it until a second language
  ships) beats `Accept-Language`, which beats English; `<html lang>` and `dir` follow it, and every HTML
  page and every API error carries `Vary: Accept-Language` and `Content-Language` (a caching proxy must
  honour `Vary`, as it must for the existing `Cookie` and `Authorization`). The API's error `message` and
  `hint` follow the same negotiation; `code`, status and shape never change, and the OpenAPI document stays
  English (it says so). **The eleven responses that used to be a bare string** (a failed form check,
  an incomplete form, a missing address on `/go`, and the single-sign-on errors) are now pages with a
  heading, one sentence and one link back, **with the same status codes**; the single-sign-on provider's
  own error text is no longer shown (it is logged). English API bodies are byte-for-byte unchanged.

- **A repository whose verification fails is now shown as unverified, with the failed conditions, instead of
  as "temporarily unavailable" (IS-39).** `prikk verify` exits 1 and prints the failing report when a
  repository does not verify; planeter used to treat every non-zero exit as an outage, so the verify page
  and the repository badge said the repository "could not be read right now" (and the API answered `503
  unavailable`). They now show the verdict: the verify page and `GET …/verify` answer `200` with the
  failed conditions and stages (`ok: false` in the API), and the repository and history badges say
  `unverified`. Only a report in which every stage ran to an outcome is read this way: a run that stopped
  for lack of memory (the per-read memory ceiling), was cut short, timed out or printed nothing usable is
  still an error, never a verdict. A page whose own content cannot be read (damaged history or file
  objects) is still `503`; no page ever says "verified" unless prikk's verdict is ok and every stage was
  evaluated. A repository with a damaged ref pointer index (stages blocked behind a failed `refs` stage) is
  likewise shown as unverified now, not as unavailable. No wording changed.

## [0.4.0] — 2026-10-19 — the read-path resource model: memory-weighted admission, a per-call ceiling, children that die with the server

### Upgrading from 0.3.0

1. **`prlimit` (util-linux) is now a required program**, and planeter **refuses to start** without it (the
   container image has it). `timeout` (coreutils) is **no longer used**.
2. **New variable `PLANETER_PRIKK_MEMORY_BUDGET_MB`** (MiB): the memory budget of reads. Unset, it is a
   quarter of the memory the process may use (its cgroup limit, else physical memory), **at least 256 MiB and
   at most 8 GiB**; a value you set is never capped. The budget is split in a small light pool and a heavy
   pool; size it so that 7/8 of it is at least 4.1 × the largest repository you expect to `verify`
   (`docs/RELEASING.md`). The start-up log states the budget, its source and both pool sizes.
3. **Reads can now be answered `503 busy`** (no budget or slot within the bounded wait) **or `422 too_large`**
   (new error code: the read can never fit its pool of the budget, or the file is above its bound).
4. **`GET …/file` refuses files above 4 MiB** with `422 too_large`; **the raw route keeps its 25 MiB cap**
   (above it, `422 too_large` instead of `503 unavailable`).
5. **A killed server no longer leaves its prikk processes running** (`bwrap` is the server's direct child and
   dies with it).
6. **The sandbox wall-time bound is now a `SIGKILL` at the deadline** (no SIGTERM grace); the answer is the
   same `503`.

### Changes

- **Reads are admitted by estimated memory; `too_large`; `file` above 4 MiB is refused (RFC 011 D-1…D-4,
  D-6; IS-36). New variable, new error code, behaviour changes.** The prikk gate used to bound only the
  *number* of prikk processes, while a read's memory follows the repository's size (eight concurrent
  `verify` calls on a 1 GiB repository held 16 GB). It now also holds a **memory budget**:
  `PLANETER_PRIKK_MEMORY_BUDGET_MB` (MiB; default a quarter of the memory the process may use — its cgroup
  limit or physical memory — **at least 256 MiB and at most 8 GiB**; a value you set is never capped; the
  start-up log states the budget and its source). Each read is costed from the repository's store size and
  the verb, must fit **its pool of the budget** and a slot within the existing bounded wait, and runs under
  a memory ceiling of 1.5 × its cost (never above the budget). **The budget is split in two pools:** a
  small **light pool** (an eighth of the budget, at least 64 MiB, at most half of it) that only the
  cheapest reads draw from (`log`, refs, a small `show`; costs up to 32 MiB), and a **heavy pool** (the
  rest) for every other read — so a cheap read is never queued behind a large one that is waiting for
  memory; the start-up log states both sizes. **Answers:** a read that cannot fit its own pool (for a heavy
  read, the heavy pool: 7/8 of the budget) is **`422 too_large`**
  (new code: message "This is too large to serve here."; the UI shows the same, plainly); a wait that ends
  is `503 busy` as before; a call that hits its ceiling is `503 unavailable` and logs one line. Size your
  budget so that 7/8 of it is at least 4.1 × the largest repository you expect to `verify`. **The API's
  `file` view is now bounded at 4 MiB** and refuses a larger file with `422 too_large` (it had no bound
  before); **the raw route keeps its 25 MiB cap** (above it: `422 too_large` instead of `503
  unavailable`, with the hint "This server does not serve files that large; ask the operator."; the
  `file` and budget refusals keep "Download it from the content origin, or ask the operator to raise the
  read memory budget."). `raw` and the repository's page no longer report a **timeout as "not found"**: only a missing path is `404`. The repository page's `verify`
  badge is admitted separately, so a repository whose `verify` does not fit the budget still gets its page
  ("verify unavailable"). A repository whose store cannot be measured is `503` with a log line.
  The OpenAPI document gains the `422` response on every read operation.

- **A killed server no longer leaves its children running; `prlimit` is a new runtime prerequisite and
  `timeout` no longer is (RFC 011 D-2, D-5; IS-37).** Every prikk call now runs as `bwrap` directly under
  the server (the `timeout` hop is gone), so `--die-with-parent` ties the whole sandbox to the server: a
  `SIGKILL` of planeter removes `bwrap` and prikk within milliseconds, where before they ran to the end of
  the call (up to the 120 s bound). The wall-time bound is now kept by the server (`SIGKILL` at the
  deadline, same `503`; no SIGTERM grace). **New prerequisite:** `prlimit` (util-linux; in the container
  image and on every mainstream distribution) is checked at start and a missing one **refuses to start**
  with a message naming it. The mechanism for a per-call memory ceiling (`prlimit --data=<N> --`) and its
  honest error — a call that hits it is a `503 unavailable` that logs one line naming the verb and the
  ceiling, never a verification result — are the basis of the per-read ceiling described in the entry
  above. The image build now fails if `prlimit` is missing.

## [0.3.0] — 2026-10-17 — safe by default: non-blocking sign-in, a real API contract, isolated raw content

**Upgrading from 0.2.0**

- Needs **prikk ≥ 0.52.0** (the container image bakes 0.52.0; it baked 0.46.0).
- The declared minimum compiler rises from 1.85 to **1.88** (`cargo install planeter` on an older one is now refused up front).
- A non-loopback bind now needs a **distinct content host**: `PLANETER_CONTENT_ORIGIN` must name a different host from the application's, or planeter refuses to start (on a loopback bind it warns).
- An **unknown command-line argument exits 2** instead of starting the server (`--version`/`-V` and `--help`/`-h` are new).
- **API errors are JSON** (`{"code","message","hint"}`, were empty bodies), and ETags change once.
- File `content` is **`kind`-tagged** (`{"kind":"text","text":…}`), and a **missing file is `404`** (was `200`).
- New **caching and browser-policy headers** on every response (`Cache-Control`, `Vary`, `Permissions-Policy`, and HSTS behind trusted proxies).
- New variables: `PLANETER_AUTH_CONCURRENCY` and `PLANETER_AUTH_QUEUE_WAIT_SECS` (sign-in), and `PLANETER_PRIKK_CONCURRENCY` and `PLANETER_PRIKK_QUEUE_WAIT_SECS` (read path); defaults need no action.

- **The declared minimum compiler is now true and checked (IS-32). The minimum rises from 1.85 to
  1.88.** `rust-version` was `1.85`, but the code uses let-chains, which need Rust 1.88; nothing built on
  1.85 and no gate noticed, so `cargo install planeter` on an older compiler failed with a compiler error
  instead of a clear message. `rust-version` is now `1.88.0`, the lowest compiler that builds the
  workspace (measured), and a new CI job `msrv` runs `cargo check --workspace --locked` on exactly that
  version. `pin-check` fails if that job's compiler and `rust-version` differ. The release itself is still
  built with 1.99.0.

- **API contract refinements (IS-33). Breaking for any client of 0.2.0's file endpoint.**
  `GET …/file` now serializes `content` as an object tagged by `kind`: `{"kind":"text","text":…}`,
  `{"kind":"binary","blob_id":…,"size":…}`, `{"kind":"opaque","size":…}` (it was serde's
  `{"Text":…}` / `{"Binary":{…}}` / `"NotFound"`). **A path that is not in a readable repository is now
  `404 not_found`** (it was `200` with a "not found" content value). The raw route, asked on any host but
  the content origin's, answers `404` with the new code `raw_elsewhere` (it was the generic `not_found`,
  whose hint sent callers looking at the wrong thing); the OpenAPI document names the content origin in a
  `servers` entry on the raw operation. A wrong method is `405` `method_not_allowed` with `Allow: GET`
  (it was `invalid_request`). A view that cannot be serialized is a `500 internal`, no longer an ETag over
  `null`. The web pages are unchanged.

- **Raw content is gated by host, and isolation is enforced at start-up (IS-16; RFC 003 D-4).
  Breaking.** The raw route (`/api/v1/repos/{owner}/{name}/raw`) now answers only on the content
  origin's host; on any other host it is `404` `raw_elsewhere` (see the API contract entry). The effective host is
  `Host`, or the last `X-Forwarded-Host` only from a peer in `PLANETER_TRUSTED_PROXIES`. A different port
  on the same host is *not* a distinct origin. **A non-loopback bind whose content origin has the
  application's host (the default) now refuses to start**, with a message naming
  `PLANETER_CONTENT_ORIGIN`; on a loopback bind one warning is logged and nothing else changes. There is
  no override variable. A deployment that proxies a second host name to the same planeter and sets
  `PLANETER_CONTENT_ORIGIN` to it keeps working. `planeter --help` is now the one list of environment
  variables (the copy in `main.rs`'s header is gone, and a test keeps `--help` and the code in step).
  A `PLANETER_CONTENT_ORIGIN` without a host (not `scheme://host[:port]`) is refused at start.

- **A real API contract and safe cache headers (IS-28, IS-29; RFC 003 D-2, STD-3).** **Behaviour
  change: API error responses now carry a JSON body** `{"code", "message", "hint"}` with
  `Content-Type: application/json` (they were empty). Codes: `not_found` (a repository that is absent and
  one the caller may not see are byte-identical), `unavailable`, `busy` (with `Retry-After`),
  `invalid_request` (a malformed query; was an unstructured `400`), `method_not_allowed` (`405` with
  `Allow: GET`), `raw_elsewhere` (the raw route asked on a host other than the content origin's) and `internal`. Status
  codes are unchanged, and the one change to a success body is the file endpoint's `content` shape (see "API contract refinements" above). `GET /api/v1/openapi.json` is now a complete OpenAPI 3.0.3
  document (parameters, responses, schemas, security, the package version); `ETag` values are now a
  specified hash of the canonical JSON and **change once on upgrade**, then stay stable across restarts,
  releases of the compiler and key order. **Caching change:** every dynamic response is
  `Cache-Control: private, no-cache` with `Vary: Cookie, Authorization` (sign-in, sign-out and any
  response that sets a cookie: `no-store`); the stylesheet is `public, max-age=3600`; every response
  carries `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()`; with
  `PLANETER_TRUSTED_PROXIES` set (the app declared to sit behind TLS) responses also carry
  `Strict-Transport-Security: max-age=15552000` (no `includeSubDomains`, no `preload`). Unknown `/api/`
  paths now answer with the JSON `not_found`; other unknown paths are unchanged.

- **Sign-in work off the async workers, and constant-work sign-in (IS-24, IS-31).** Password
  verification, token and session checks, session writes and the SSO hand-shake now run on the blocking
  pool, so a sign-in flood can no longer stall the server. Password verification is bounded by its own
  gate: `PLANETER_AUTH_CONCURRENCY` (default `min(CPUs, 4)`) and `PLANETER_AUTH_QUEUE_WAIT_SECS`
  (default 5), past which the sign-in is refused with `503` and `Retry-After`. **Behaviour change
  (security fix):** sign-in for an unknown account (or one without a password) now takes as long as for a
  wrong password; before, it returned immediately, which let a client tell whether an account exists by
  timing. This weakness was present in 0.2.0.

- **One compiler, named runners, and a release dry run (RFC 010 D-5, IS-26).** CI and the manual
  supply-chain checkpoint now install the same pinned Rust toolchain (1.99.0 with rustfmt and clippy)
  that the release builds with; every workflow names its runner image (`ubuntu-24.04`) instead of
  `ubuntu-latest`. `pin-check` fails on a second toolchain version and on a `*-latest` runner. The
  release workflow can be dispatched as a dry run that builds everything and publishes nothing. The
  release `gates` job now installs rustfmt and clippy for the pinned toolchain (the action installs a
  minimal profile; a real tag would have failed `cargo fmt`).

- **Read-path concurrency (IS-4, RFC 003 D-8): an unauthenticated denial of service closed.** Every
  prikk call from the browse UI and the read API now runs on the blocking pool (`spawn_blocking`) behind
  one process-wide gate, so slow repositories no longer pin the async workers and stall sign-in and every
  other route. New `PLANETER_PRIKK_CONCURRENCY` (default `min(CPUs, 8)`) caps concurrent prikk
  subprocesses; `PLANETER_PRIKK_QUEUE_WAIT_SECS` (default 10) bounds the wait, past which the request
  is refused with `503` and `Retry-After` (existence-neutral, counted). A page's several prikk calls share
  one slot. No route, body, authorization or sandbox change.

- **`planeter --version` / `-V` and `--help` / `-h` (RFC 010 D-6, IS-1).** `--version` prints three lines
  (`planeter X.Y.Z`, `commit <sha or unknown>`, `prikk-floor X.Y.Z`) and exits 0 **without reading
  configuration or starting the server**; release builds report the tag's commit and `build-info.txt`
  carries the same lines. **Behaviour change:** any other argument is now refused with a one-line error,
  the usage and **exit 2**; before, an unrecognised argument started the server. A script that passed an
  argument and relied on the server starting must drop it.

- **Maintenance gates (RFC 010):** `tools/floor-check` (the three prikk floors must agree) and
  `tools/pin-check` (every action by commit SHA, base image by digest, toolchain by version) run in CI and
  in the release gates; a manual-only `supply-chain.yml` workflow records a deny/audit checkpoint. The
  build chain is pinned: actions by commit SHA with the tag in a comment, `debian:bookworm-slim` by digest,
  Rust 1.99.0 explicit in the release workflow. No product behaviour changes.
- **Security hygiene:** `ammonia` 4.1.4 → 4.2.3 and `yoke-derive` 0.8.3 → 0.8.4 (lock-only), clearing
  RUSTSEC-2021-0157 / -0158 and a yanked crate; the sanitizer's behaviour is pinned by regression tests.

- **prikk floor → 0.52.0** (the tested version): the PK-30 fix, the accept size bound, crash-state
  recovery, dependency-ordered `bundle import` / `sync accept` writes. The container image bakes
  prikk 0.52.0 (it baked 0.46.0). Requires **prikk ≥ 0.52.0**.
- The driver passes `bundle import --input <file>` (it passed the path positionally).
- `trust maintainer check` answers "not trusted" with exit 1 on prikk 0.52.0; the driver reads that as
  the answer when the `trust-check-v1` document says so, and still treats every other non-zero exit as
  a failure.
- The sandbox profile can carry prikk's incoming-artifact bounds (`PRIKK_EXCHANGE_MAX_BYTES`,
  `PRIKK_EXCHANGE_MAX_OBJECTS`, `PRIKK_BUNDLE_MAX_BYTES`, `PRIKK_BUNDLE_MAX_OBJECTS`); none is set yet.

- **Security hygiene: `ammonia` 4.2.3 → 4.2.4 (lock-only), RUSTSEC-2026-0373 (IS-38).** The advisory, published
  2026-10-16, is a missing case-insensitive match for the CSS `url(` function; it affects an `ammonia` builder
  configured to allow inline `style` with `filter_style_properties` and a restricted `url_schemes`. planeter
  calls `ammonia::clean` with the default configuration (which drops `style` entirely), so it was **not
  reachable** here; the lock is updated so the supply-chain gates stay green. No manifest change; sanitizer
  behaviour is pinned by new regression tests.

## [0.2.0] — the auth increment: trusted proxies, per-IP throttle, SSO

Shipped during the M2 hold (clone/push waits for prikk RFC 155 then 154, after prikk 0.49.0), after a
full documentation audit. Runtime prerequisites: prikk ≥ 0.46.0, bubblewrap, and `curl` if SSO is
enabled (the container image bundles all three).

- **Trusted reverse proxies** (`PLANETER_TRUSTED_PROXIES`): client IPs from `X-Forwarded-For` only
  behind declared proxies; a non-loopback bind is refused without them (planeter speaks plain HTTP
  behind TLS termination).
- **Per-client-IP login throttle** (20 failures across any accounts → 15-minute lock) alongside the
  per-account one.
- **SSO via OpenID Connect** (`PLANETER_OIDC_*`): authorization-code flow with PKCE, ID tokens verified
  against the provider's JWKS; provider traffic through the egress guard and a confined `curl` (new
  runtime prerequisite). Accounts are linked to `(issuer, subject)` administratively — no
  auto-provisioning.
- **Documentation**: the README describes the shipped product and how to install, run and deploy it;
  RFCs 001–003 are recorded as done; threat model v0.3 (OIDC status, residual-risk ids RR-10/RR-11);
  rustdoc builds warning-free.

## [0.1.1] — read side: browse UI, sign-in, egress guard

Work during the M2 hold (clone/push waits for prikk 0.48.0+).

- **Browse web UI**: repository home (directory listing, sanitized README, prikk-verified badge), tree,
  file (Markdown sanitized, text escaped, binary via the isolated content origin), history, change,
  refs, verify — the same read service and authorization gate as the API.
- **Sign-in / sign-out** with `HttpOnly`/`SameSite=Strict`/`Secure` session cookies, CSRF protection on
  every POST, and a per-account login throttle (5 failures → 15-minute lock). No MFA yet.
- **Egress guard**: the SSRF filter every outbound feature will route through (no callers yet).
- Threat model v0.2 records the implementation status of the auth, rendering and egress controls.

## [0.1.0] — M1: host + browse + auth

The first usable hosted forge over prikk (ROADMAP milestone M1 = A0 + RFC 002 + RFC 003).

### Hosting (RFC 001)
- Create and serve prikk repositories, laid out on disk by a stable opaque id so rename/transfer never
  moves bytes. Every prikk invocation runs in a **bubblewrap sandbox** (repo-dir-only writes, no
  network, wall-time bound, no operator keys).
- **Keyless forge:** planeter holds no history-signing key. The `forge-seal` cargo feature is reserved
  and off; CI proves no forge-signer symbol exists in the default build (ENF-2).
- Requires **prikk ≥ 0.46.0** (the version pin refuses older binaries).

### Authorization & authentication (RFC 002)
- A single pure, default-deny `authorize()`: visibility (public / internal / private), per-repo roles
  (read ⊂ write ⊂ maintain ⊂ admin) via direct, team, and org grants, credential scopes
  (`user-permissions ∩ scope`), protected refs, and no existence leak for hidden repositories.
- Local passwords hashed with **Argon2id**; scoped access tokens (SHA-256, constant-time compare);
  **ed25519** SSH public keys (fingerprints identical to `ssh-keygen -lf`). OIDC/SSO is a seam for a
  later release.
- Persistent **SQLite** for every store (`PLANETER_DB`).

### Browse (RFC 003)
- A GET read API (OpenAPI-described; `ETag`/`304`; bounded pages) over the honest view-models:
  history, change, file, **directory listing** (`prikk tree`), **raw file download** (`prikk cat`),
  refs, and verify status. Approved-but-unsealed changes are never shown as prikk-verified.
- Rendering safety: a strict HTML/Markdown sanitizer, a strict CSP, and raw bytes served only as
  inert downloads from an isolated content origin.

### Not in this release
- Clone/push (M2), review/merge (M3), CI (M4), packages (M5); blame/annotate (awaiting a prikk verb).
