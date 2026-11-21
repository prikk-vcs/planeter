# Changelog

Releases are bare version tags; each section is the release's notes (assembled into the GitHub
release by `release.yml`). Kept by hand, per release, so nothing here goes stale by templating.

## [0.8.0] — unreleased until tagged — landing by delivery, and prikk 0.58.0

0.8.0 lets reviewed work **become the project's branch without the forge holding a key** (RFC 008). A maintainer seals the work with their own prikk and their own key, runs **`planeter-client deliver`**, and the server checks the repository they send against a written rule, says in plain words what would change, and — only when the maintainer confirms — makes the maintainer's branch the repository's `heads/main`. The open proposals stay open (their pending work is made again from the artifacts the server kept), the ones whose work is now in the project's branch show as **landed**, and a landed proposal's revision can be looked at file by file. The server signs, seals, merges and closes nothing at any step. The release also moves the prikk floor to **0.58.0** (a breaking prerequisite, said first below), fixes the per-clone freshness record of `planeter-client` (IS-62), and pins the container build's BuildKit image to v0.34.2 (IS-60).

### Upgrading from 0.7.0

- **Install prikk 0.58.0 or later** before you start the new server (the container image carries it). A server on prikk 0.57.0 refuses to start with `unsupported prikk version 0.57.0; planeter requires >= 0.58.0`, and `planeter-client` stops with its own message. Why the floor moved: `prikk sync accept` reads the artifact object by object instead of holding it whole, so its memory follows the largest object and not the artifact, and it takes 4 fsyncs a patch instead of 10; `sync seal`, the maintainer's side, read 5.5 GB for 1,000 claims on 0.57.0 and reads 23 MB on 0.58.0. Existing hosted repositories keep working; new ones are format 8 as before. Building from source still needs Rust 1.89.0.
- **The memory an accept is admitted at changed, so operators may now admit more.** The default 100 MiB proposal is admitted at about 467 MiB of the memory budget (it was 720 MiB; 0.46 GiB against 0.7 GiB), so a **3 GiB host now admits the default bound** (a 2 GiB host still does not); a 4 GiB host admits artifacts up to 195 MiB (125 MiB before); a bound raised to 256 MiB needs about 1.14 GiB (1.8 GiB before), a host of about 6 GiB. If you set `PLANETER_PRIKK_MEMORY_BUDGET_MB` or raised `PRIKK_EXCHANGE_MAX_BYTES` on 0.57.0 to fit the old cost, you may now admit more than you meant to: look at the start-up line, which says what is admitted. The cost of a read (`log`, `show`, `tree`, `cat`, `verify`, `extract`) and of a creation or a delivery is unchanged: all 77 read-cost rows were measured again on 0.58.0 and none moved by more than 5 %.
- **The database gets one new table, `deliveries`**, created idempotently at start; nothing existing is touched and there is nothing to migrate. No repository format changed.
- **Deliveries are on exactly when proposals are**: when `PLANETER_SPOOL_DIR` and `PLANETER_ARTIFACT_DIR` are set. With neither there are no delivery routes (the same not-found as for proposals). The archive bound is the import's `PLANETER_IMPORT_MAX_MB` (default 100 MiB): **a repository larger than it cannot be delivered** until the operator raises it. During a delivery the disk holds the archive, the staged repository (about the archive again) and the previous generation of the repository until the end, so leave room for about three times the archive.
- **Nothing in the spool needs clearing.** The file-by-file view's cache keys are unchanged and the entries 0.7.0 wrote are read as before (`compared.landed` is `false` for them); the first view of a revision after the upgrade is unaffected, and a landed revision's view is new and has its own entries.
- **Japanese:** 107 messages of this release have not yet been read by a reviewer and are shown as they are; English is complete. `planeter-i18n-sheet --status`: `ja (日本語): signed off by the owner on 2026-11-17; 107 message(s) newer than the sign-off (server 0 changed + 69 new, client 1 changed + 37 new); 753 unchanged.` They are the delivery's pages and API messages, `planeter-client deliver` (and the changed `help.main`), the landed view's five and the new words of the delivery route's `413`. The server logs its own count at start (`planeter: language ja: 69 message(s) are newer than its sign-off and have not been read by a reviewer`); the client's 38 are in the tool's line above.
- Nothing else has to change: no new setting, no default moved.

### For people who run a server

- **Breaking, first: the prikk floor is 0.58.0** (see above). `MIN_PRIKK_VERSION`, `release.yml`, `ci.yml` and the `Dockerfile` all say 0.58.0; `planeter-floor-check` holds the four equal.
- **Deliveries.** A delivery replaces the repository's `heads/main` by a maintainer's sealed history, after a written rule over history prikk verified and a key this repository already trusts signed. **Who may deliver:** a maintainer or an administrator of the repository, with a token whose scope is `full`; a `write` token is capped below merge-seal and gets `403`; a stranger and a hidden repository get the one `404`. **The limits:** one archive of up to `PLANETER_IMPORT_MAX_MB`, a 600 s deadline for the check, 1,024 MiB of staged archives in all, at most 2 deliveries at once for a person and one in flight per repository; a delivery that is not confirmed within a day is removed (an hourly sweep). **The start-up line:** `planeter: deliveries on: an archive of up to 100 MiB, 600 s deadline, 1024 MiB of staged archives, at most 2 at once, one per repository; a delivery not confirmed in a day is removed`; at stop the server prints the counters (`planeter: deliveries: accepted_reports=… done=…`; they carry no file, branch or key name). See `docs/deliveries.md` for the steps, the refusals and what to do about each.
- **An unfinished delivery is finished, not rolled back.** The install is: check the rule again under the repository's write lease, record the intent (durably, before anything moves), exchange the staged repository for the live one, bump the revision counter, make the open proposals' pending work again from the artifacts the server kept, `verify`, done. A kill before the exchange abandons the delivery and the old repository is untouched; a kill at or after it is **completed at the next start** (or by an administrator's recovery run, which clears the stale locks of a killed prikk call with `unlock --force` after its witnesses agree, never `--yes`). The repository is flagged `a write did not finish (delivery_unfinished)` meanwhile and `GET …/recovery` shows why. A failure after the exchange is flagged, loud, and keeps the previous generation for an administrator. The exchange is two ordinary renames (no `unsafe`, no new dependency): the repository's directory is absent for about 60–90 µs, and a request that opens it then is answered `503 unavailable` and can be retried.
- **What a delivery costs.** The check is `archive verify` (3.0 × the archive) plus `archive import` (6.0 ×), admitted at the creation cost model from the heavy pool, in a scratch repository on the repositories' own filesystem (a repository on another filesystem than the stage cannot be delivered: the answer is `unavailable`). The confirmation holds the repository's write lease for the whole install: 0.3 s for a small repository, about 1.4 s for a 63 MiB archive with a 20 MiB proposal, about 6 s (12 s on a loaded host) with a 1,000-patch revision; during it an accept or a second delivery is `busy` and reads are not delayed. It grows with the number and size of the open proposals, not with the delivery.
- **`sync seal`, the maintainer's side, as measured** (prikk 0.58.0; the server does not run it). Each claim adds one new file, so the tree ends with N paths. Measured on an encrypted btrfs disk: 100 claims 1.0 MiB; 1,000 claims 7.1 MiB; 10,000 claims 183.6 MiB and 670 s; it reads about 23 KB a claim (1,000 claims took 104 s and read 23 MB; 0.57.0: 139 s and 5.5 GB; 10,000 claims read 233 MB). **The memory of `sync seal` grows with the number of claims. With a tree that stays the same size it grows linearly (68 MiB at 10,000 claims). With a tree that gains a file per claim it grows faster than linearly (181 MiB at 10,000 claims).** The time depends on the disk: about 90 s on tmpfs, 670 s on an encrypted btrfs disk for the same work. No speed is claimed for 0.58.0: its fsyncs fell, the wall time per patch of an accept did not on the measuring machine (about 19 ms a patch for 1,000 patches on both versions).
- **A debug build of the server has a test hook, `PLANETER_FAILPOINT`, that stops the engine at a named point; a release build does not contain it** (the ENF-2 script fails if the release binary contains the name). It is not a setting.
- **The container image's BuildKit builder is pinned to v0.34.2** (IS-60: the two advisories that named v0.33.1, GHSA-2fqp-xg6f-v7gj and GHSA-w26x-xg27-7qjc, are fixed in it). Only the release build is affected, not the running server.
- **The gate tools stay pinned** as in 0.7.0: `cargo-deny` 0.20.2 and `cargo-audit` 0.23.0.

### For people who use it

- **A maintainer: `planeter-client deliver https://host/owner/name`.** Run it in your own prikk repository (or name it with `--repo DIR`) after you have sealed the work with your own key. It exports your repository with your own prikk, sends it, and prints what the server found; you confirm with `y` (there is no default yes), or pass `--yes`; `--dry-run` stops after the report and cancels. A real report:

  ```
  Exporting your repository…
  Exported your repository (21.5 KiB).
  Sending 21.5 KiB to 127.0.0.1 to be checked…
  Delivering replaces this repository's branch heads/main with yours. It is accepted only if yours continues from the current one; nothing is signed by this server.
  To carry it out, confirm it. To give it up, cancel it. A delivery that is not confirmed is removed after a day.
  The project's branch would go from 76914127b56e to 7dc412b26b08.
  Your delivery adds 5 changes to this repository's history.
  Open proposals that stay as they are: 2 (#c-5922edb59ef3b8e43d9bdce5 "P3"; #c-e28a2ef02ce25d1c6701aeb2 "P2").
  Proposals whose work is already in your branch, so they will show as landed: 1 (#c-7b2a8d75ebebb715c22b567c "P1").
  ```

  A delivery is accepted only if yours **continues from the current branch** and every change is signed by a key this repository already trusts. An older, divergent or unrelated repository, a key the repository does not trust, a deleted branch and an archive too large are each refused in one sentence that says what to do (`docs/deliveries.md`); a refusal changes nothing on the server. A `write` token cannot deliver (`403`: it needs full access, from a maintainer). The delivery is carried out by the server: if you stop the program it goes on, and the program prints the delivery's id and address before it waits. Exit codes: 0 done (or nothing to deliver, a dry run, a declined prompt), 1 the server refused or the delivery did not go through, 2 the command line (no terminal without `--yes` is refused before anything is sent), 3 network, 4 your prikk, 5 local files.
- **Everyone: the repository page and the history page say who delivered.** "Delivered by {name} on {date}: signed by a key this repository trusts and checked by prikk.", "The project's branch is heads/main." and, under a `<details>`, the keys the repository trusted when the delivery was checked. A repository that never had a delivery shows nothing new. `GET …/refs` gains `followed_ref` and `last_delivery`.
- **Landing counts `heads/main` only.** A proposal lands when its patches are in a block of the project's branch; being in another branch a delivery added lands nothing. A landed proposal stays landed.
- **The changes of a landed revision can be looked at.** Its page and its JSON show the files that the project's branch gained from it, built from the sealed history (the blocks of `heads/main` that carry the revision's patches), with the statement "These changes have landed: they are part of the project's branch." and `compared.landed: true`. If the revision's work is **partly** in the project's branch and partly still waiting, the answer is the new reason `partly_landed` (409): "Some of this revision's work has landed in the project's branch and some is still waiting here, so its changes cannot be shown as one change." A revision whose work is all there but not in one run of consecutive changes is `not_one_change`, as before. **Known limit (IS-63):** a proposal built on top of another proposal loses its file-by-file view when the one under it lands, unless it was looked at before.
- **`planeter-client fetch`, an older clone and a fresh one.** A fresh `clone` has the project's branch exactly. An older clone's `fetch` lands the new tip as a received copy and never touches your own branches; after a fetch that landed a moved project branch the client says so in one sentence: "The project's branch moved on the server. A fresh clone has it exactly; to follow it in this clone, merge it yourself with prikk (this makes your own commit, not the server's)." **IS-62 is fixed:** the client's freshness record is per clone, no longer per server address, so a second clone of the same repository is no longer told "Nothing changed on the server since the last fetch" about a state it never saw. An old record is used only by the one clone that can have written it.
- **Languages.** English and Japanese. 107 Japanese messages of this release have not yet been read by a reviewer and are shown as they are; English is complete (`ja (日本語): signed off by the owner on 2026-11-17; 107 message(s) newer than the sign-off (server 0 changed + 69 new, client 1 changed + 37 new); 753 unchanged.`). The new Japanese term for a delivery is 引き渡す / 引き渡し.

### Under the hood

- **RFC 008, in four lines.** A delivery is a repository archive and nothing else. It is staged in a scratch repository that trusts exactly the repository's adopted keys (never the archive's), checked against a written rule, reported, and only then installed by replacing the repository with its staged successor. The forge's `heads/main` is then the delivered `heads/main`, so clones, fetches, reads, the proposal base, landing and the changes view keep working with no new concept. The forge adds no signature (ENF-2 scans both binaries).
- **The rule** (the staged repository is the only input; the first failure is the answer): `heads/main` is a local branch in the stage and nothing is received; every local ref's tip is sealed by an adopted key and `verify` has no publication-trust issue; for every published ref the live repository has, the live tip is a block of the staged ref's chain; no ref the live repository has is missing; and **the delivery changes at least one ref** (a ref moves, or a ref the live repository lacks is added). The closed reasons are `no_main`, `unadopted_key`, `not_a_successor`, `older_or_diverged`, `unrelated`, `branch_missing`, `nothing_to_deliver` and `damaged_archive`, beside the errors of the call (`too_large`, `stage_limit`, `busy`, `needs_recovery`, …). The chain is read whole, never with a fixed limit (up to 200,000 blocks). A refusal leaves the live repository byte-identical (tested by tree hash for every archive of the measurement).
- **The delivery's states** are nine and closed: `staged → reported → confirmed → exchanged → reaccepting → done`, with `failed`, `abandoned` (never after the exchange) and `expired`.
- **The API.** `POST /api/v1/repos/{owner}/{name}/deliveries` (the import's framing and edge limit; `201` with the delivery, `202` when the check takes longer than 25 s), `GET …/deliveries/{id}`, `POST …/deliveries/{id}/confirm` (`200` final, `202` while it runs), `DELETE …/deliveries/{id}`; two new error codes, `delivery_busy` and `delivery_not_confirmable`, beside `stage_limit`, `stage_expired`, `needs_recovery` (503) and the existing ones; the changes routes gain `compared.landed` and the reason `partly_landed`; the OpenAPI document carries all of it. The delivery route's `413` has its own words ("That archive is larger than this server accepts.") with the limit in MiB in the hint.
- **Landing and the view of landed work.** What a revision *is now* is asked first from facts: while all its claims are pending it is shown as in 0.7.0; otherwise its patches (the ids it declared together with the ids it added) are asked of the blocks of `heads/main` with the landing observation's own function. One run of consecutive blocks is its change set (cached under the two block ids, content-addressed, never stale); the counters `landed_ok` and `partly_landed` are new.
- **`planeter-client` internals.** `deliver` exports into a private scratch (mode 0700, the archive 0600) that is removed on every exit path; after a `SIGKILL` the run's advisory lock tells the next run that the directory is nobody's. It never retries a refused answer, follows no redirect, speaks https only, and calls on prikk only `archive export` and one `log` read (a source scan holds it: no `seal`, `merge`, `branch close`, `--allow-non-fast-forward`, and no `--yes` on prikk).
- **The JSON-reliance test.** `planeter-prikk`'s `unknown_fields` test reads 19 real prikk 0.58.0 documents and holds what the readers do with unknown fields, open and closed values, absent versus null and large integers. When a prikk release changes a shape it fails on purpose; its header says what to do.
- **The accept constants** moved with prikk 0.58.0 (a blob holds about 4.0 × its size on `sync accept`, 7.0 × on 0.57.0; many-patch artifacts 0.01 to 0.2 ×). An import records the history it confirmed, so the changes view of a repository **created from an archive on 0.58.0** is no longer "provisional" (a repository created on 0.57.0 stays provisional). `unlock --yes` no longer clears a lock whose holder is `unknown` on 0.58.0; planeter passes only `--force`, on every version.

## [0.7.0] — 2026-11-18 — the file-by-file view of a proposal, and the gate tools pinned

0.7.0 adds the **file-by-file view of a proposal's revision** (RFC 005 part 6): a page that lists the files a revision changes, a page for each file's diff, and two API routes that give the same answers as data. The files are rebuilt by prikk from the patches the server holds; nothing in the view is signed. It also pins the two tools that gate every push and every release. The prikk floor stays **0.57.0** and the minimum compiler **1.89.0**; it adds no setting, changes no default, no database table and no repository format.

### Upgrading from 0.6.0

- **Nothing has to change.** No new setting; the prikk floor is still 0.57.0 and building from source still needs Rust 1.89.0. The database and the hosted repositories are untouched (no table and no format changed), and the view stores its answers only in the spool (see below), where nothing needs clearing: the view's cache is new in this release, and an entry written by a development build of it is never read, because every key carries a format version.
- **Japanese:** the Japanese of this release — the pages and the API messages of the file-by-file view, the two cut-marker messages, and the client's `help.propose` (changed) and `propose.revision_keeps` (new), 95 messages in all — was **read and accepted by the owner on 2026-11-17**, and the 24 of them that were then reworded (for consistency and naturalness) were read again and passed the same day (the record is data: `crates/planeter-i18n/src/languages/ja.rs` and both `locales/ja.signed` baselines). `planeter-i18n-sheet --status`: `ja (日本語): signed off by the owner on 2026-11-17; 0 message(s) newer than the sign-off (server 0 changed + 0 new, client 0 changed + 0 new); 754 unchanged.` The count of messages newer than the sign-off is **0**, so the server logs no such line at start (as in 0.6.0).

### For people who run a server

- **The view needs the spool.** It is part of proposals, which need `PLANETER_ARTIFACT_DIR` and `PLANETER_SPOOL_DIR`; with no spool there are no proposals and no view. The start-up line "planeter: the file-by-file view of a revision: at most 30 s and 64 MiB of files per view, answers kept in the spool (up to 64 MiB)" says it is on.
- **What a view costs.** A revision's view is two extractions under one admission: the same cost as an `extract` read, 9.5 × the 64 MiB bound + 8 MiB (616 MiB of the heavy pool at the defaults), with a 30 s wall; a busy server answers `busy` (503, `Retry-After`), a run past the wall `timed_out`. The answers are kept in the spool (`changes-cache/`, disposable, least recently used out past 64 MiB) and a cached answer needs no admission. **At most eight requests wait for one revision's view; the ninth is told the server is busy at once.** No new setting.
- **The tools that gate every push and every release are pinned (IS-58).** `ci.yml`, `release.yml` and `supply-chain.yml` install `cargo-deny` 0.20.2 and `cargo-audit` 0.23.0 by exact version with `--locked` (they used to install the newest); `planeter-pin-check` now fails on a gate tool without an exact version or `--locked`, on any other `cargo install` in a workflow, and on a workflow that names another version. A pin moves at the pins review of each release.

### For people who use it

- **Each revision on the proposal page links to "See the changes in revision N"** (the old sentence "The file-by-file view is not available yet" is gone). The **changes page** lists the files a revision changes in one table (File, Change, Lines, Notes) with a summary line, and says in plain sentences what the files are (rebuilt from the patches; nothing here is signed), whether the history was checked (it may only be replayed), and what they are compared with; a **file page** shows one file's diff as one table with a visible "+" / "−" and a hidden word on each row, and a symbolic link as a sentence.
- **What it does not show, said in words.** A text file is shown up to 256 KiB a side and 5,000 diff lines a file; a file that is not UTF-8 text or is binary is listed with its reason, not shown; a symbolic link is shown as a link, never followed; a line over 2,000 characters and a name over 1,024 bytes are cut, and the page says how much was left out, in the reader's language. A file moved to a new name is a deletion plus an addition. Every reason the view can be unavailable (the revision cannot be shown file by file, it is too large, the server is busy, it took too long, it failed) is a plain page with what to do next, in English and Japanese.
- **Adding a revision with `planeter-client propose --proposal ID` works.** In the 0.5.0 and 0.6.0 clients **the server refused it** (the client sent the target branch beside the proposal id, which the server does not accept), so a person who needed a second revision had to open a new proposal; **upgrade the client**. The first revision fixes a proposal's title, description and target, so `--title` or `--description-file` with `--proposal` is now refused by the client itself, at once, in one sentence; the server's `reason` for a refused request shows under `--verbose`. No change to the server or the API.
- **Safe to read.** Names and text from a proposal reach a page only after three looks (the engine's display function, the page's, the template's escaping); nothing scrolls sideways at 320 CSS px; added and removed rows differ by a solid and a dashed border as well as by colour. Accessibility was checked by the structural and stylesheet tests and by one browser measurement (Chromium: no sideways scroll, no target under 24 × 24 CSS px, a visible focus ring on every focusable element); **no assistive technology has been tried**, and the diff table with a screen reader is the first thing to try (`docs/accessibility.md`).
- **Languages.** English and Japanese. The Japanese of this release — the file-by-file view (pages and API), the client's `help.propose` and `propose.revision_keeps` — was read and accepted by the owner on 2026-11-17, the 24 that were then reworded being read again and passed the same day; the count of messages newer than the sign-off is 0 (`ja (日本語): signed off by the owner on 2026-11-17; 0 message(s) newer than the sign-off (server 0 changed + 0 new, client 0 changed + 0 new); 754 unchanged.`). The English sentence "Compared with … (the block the first patch continues from)" says "the point in history" instead of the internal word.

### Under the hood

- **The API** (RFC 005 part 6, first part). `GET …/proposals/{id}/revisions/{seq}/changes` lists the files a revision changes and `…/changes/{file_id}` gives one file's diff, compared with the revision's own base and rebuilt by prikk from the proposal's patches (nothing is signed). New error `changes_unavailable` with a closed `reason`: `claim_not_held` (409), `not_one_change` (422), `tree_too_large` (422), `busy` (503, `Retry-After`), `timed_out` (503), `failed` (503). Twelve new messages in each language (a message and a hint for each of the six reasons). **Cut text is data, not words:** a line over 2,000 characters and a name over 1,024 bytes are returned as their head and nothing else, with `cut_chars` (on the diff line) and `path_cut_bytes` (on the file entry) saying how much was left out, both omitted when 0 and both additive in the OpenAPI document; the engine writes no English into the data, and a name is cut at a character boundary. The change-set cache has a version in its key (`v2`), so no earlier entry is read.
- **The engine.** Paths and text are display-safe (invalid UTF-8, controls, bidirectional and zero-width characters are shown as visible escapes); a diff is bounded; the two pages ask the same functions as the API, so the two cost the same. About 80 new messages in each language for the pages and two for the cut text.
- **IS-58.** The gate tools are pinned as described above; `planeter-pin-check` enforces it.

## [0.6.0] — 2026-11-14 — prikk 0.57.0, Japanese signed off, languages as data, two advisories closed

0.6.0 moves the prikk floor to **0.57.0**, makes the Japanese pages, API messages and `planeter-client` available by default (signed off by the owner, twice), turns "a language" into a row of data in a shared crate, makes a failed upload judged by evidence instead of by one fact, and updates two dependencies for advisories (`ammonia` 4.2.5, `rusqlite` 0.41.1 with SQLite 3.54.1). It adds no feature to proposals, review or repository creation, and changes no default.

### Upgrading from 0.5.0

- **Install prikk 0.57.0 or later** before you start the new server (the container image carries it). The server refuses to start on an older prikk with the pin's own sentence; `planeter-client` stops at its own floor message. Existing hosted repositories keep their format (the driver never runs `format upgrade`); new ones are format 8.
- **Building from source needs Rust 1.89.0** (it was 1.88.0); the binaries and the image are unaffected.
- **Databases need no migration.** The tables are created idempotently at start and none changed. The bundled SQLite moves from 3.46.0 to 3.54.1, and this was rehearsed rather than assumed: a database written by the released 0.5.0 server (a real proposal with a comment and a decision, accounts, tokens, sessions, a repository flagged for recovery) opened and was served identically by the new build after a clean stop **and** after a SIGKILL that left a `-wal` file (every comment the killed server had acknowledged was there); a database the new build had written was then opened, read and written by 0.5.0 again, so a rollback to 0.5.0 works. `PRAGMA integrity_check` was `ok` and the journal mode `wal` in every direction. Not covered by the rehearsal: large databases, a `-wal` of a different page size, a power loss, other platforms.
- Nothing else has to change: no default setting moved in this release.

### For people who run a server

- **Breaking: the prikk floor moves to 0.57.0 (the 0.56/0.57 re-baseline).** planeter now requires prikk **0.57.0 or later**, for the server and for `planeter-client`
  (the container image carries it; `MIN_PRIKK_VERSION`, `planeter-client`'s floor, the release workflow, CI and the Dockerfile agree). Why: from 0.57.0 every new
  repository is **format 8**, which 0.56.0 and earlier refuse, so a server on 0.57.0 and a client on 0.56.0 could not work together; one floor for both. Existing
  hosted repositories keep their format (the driver never runs `format upgrade`). What 0.56.0/0.57.0 changed that planeter now reads: `sync pending` lists a patch
  only once a claim names it (so a killed or failed accept leaves **no pending prefix**, only unreached patch and blob objects, which `doctor` reports as one
  information finding, `PRIKK-DOCTOR-UNREACHED-CONTENT`, and recovery counts as clean); `extract --claim` works over a chain of pending claims and reports the chain; rollbacks,
  diverged histories and histories that do not connect are refused by default on import (the driver never passes `--allow-non-fast-forward`; planeter's creation flow meets
  only a first import); `unlock` records when the holder started (read, never trusted). Typed readers for `sync pending --format json` and for an import's refusal are in
  place and not used yet.
- **Building from source needs Rust 1.89.0.** The declared minimum compiler rises from 1.88.0 to 1.89.0 (`rusqlite` needs it); the `msrv` job of CI proves it.
- **`PLANETER_PREVIEW_LANGUAGES` now means "draft languages", and none exists today.** A code that is already offered (Japanese) is accepted, needs no setting, and the server logs that it
  is not needed; a draft language named there is offered only on the operator's request and logged once as a preview.
- **The server answers in its own words when the framework would not (RFC 012).** A request body over 2 MiB, an address the server cannot decode, an address
  nobody serves and a method a page does not take used to get the framework's plain-text reply or an empty page. Under `/api/` they are now the API's one JSON
  shape (`invalid_request` 400, `payload_too_large` **413**, `not_found`, `method_not_allowed`, each with `code`, `message` and `hint`), in the caller's language; elsewhere they are a page with a heading,
  one sentence and a link, with the same status. The proposal upload and the archive staging keep their own bounds. The OpenAPI document lists the 413 where a body
  is read.
- **The language is negotiated by BCP 47, and the switcher becomes a menu from four languages on.** A caller's request is matched exactly, then by script, then by language (`zh-TW` gets `zh-Hant`, `pt-PT` gets `pt-BR` when only that ships); the footer
  shows a button per language and, from four languages on, one menu.
- **Start-up lines.** The server logs, once, a language count only when a signed-off language has messages newer than its sign-off (for Japanese there are none: the line is gone); it logs the
  `PLANETER_PREVIEW_LANGUAGES` lines described above.
- **Memory cost of an `extract` read: 9.5 × the extraction byte bound + 8 MiB (was 9.0 ×).** On prikk 0.57.0 a file edited 128 times reached 9.16 × its size, above the old multiple; at the default bound (64 MiB) the cost is 616 MiB of the heavy pool (584 MiB before). No page or API of this release calls `extract`.

### For people who use it

- **Japanese is now available (RFC 012 D-3f).** The Japanese texts of the pages, the API's error messages and `planeter-client` were reviewed by their reviewers and signed off by the owner on 2026-10-31; the owner read the later messages and the Japanese accessibility statement again on 2026-11-14, wrote the final wording of one sentence (the legend of the badge 「一覧にありません」) and accepted all the others. A caller who asks for Japanese (`Accept-Language: ja`, the `planeter_lang` cookie, or the button in the footer of
  every page) gets it, with no operator setting; **a caller who does not ask is English, as before**. `planeter-client` speaks Japanese when `LC_ALL`, `LC_MESSAGES`
  or `LANG` says `ja` (and with `--lang ja`; `--lang` wins). The first review found and fixed 25 inconsistencies of wording before the sign-off (one word for one thing:
  the verb *change* is 変わる, *sign in* is ログイン, *refused* is 拒否), and one sentence that promised a signature the keyless forge cannot promise. The `planeter-client` help line and its wrong-`--lang` refusal now name the shipped languages from the registry (the help
  prints `--lang en|ja` exactly as before; the refusal lists them: "Use one of: en, ja.").
- **"Not listed here" no longer says something untrue.** After a patch arrives only as received sealed history (prikk 0.56.0 and later: it is then not pending), the page said "neither held
  by this server nor in the history it holds" one line above "found in history received here". The legend now says "not one of the unsigned patches this server holds, and not in the
  project's history" (English), and the Japanese badge and legend follow (一覧にありません; the explanation likewise, in the owner's own words).
- **A failed upload is judged by evidence.** When a proposal's upload fails after the server began storing it (a write error: disk full, permissions, a failing disk), the server no longer answers "refused, nothing was left" unless it can show that nothing changed; otherwise it flags the repository for recovery, keeps the upload, and a resend finishes. Nothing about a person's data changes; the technical account is below.

### Under the hood

- **Languages are data: any number of languages, with a record of who signed what (RFC 012 D-3, amended 2026-11-04).** A new shared crate, `planeter-i18n` (the eleventh published crate, published first),
  holds the only list of languages: each language's tag, name in itself, direction, plural rule, separators and **sign-off record** (who read it, when, which
  commit and review sheet). Nothing else changes for English and Japanese. What it makes possible: plural categories beyond `one` and `other` (Russian, Polish,
  Arabic and Welsh rules are in and tested), matching by BCP 47, and one review-sheet tool for any language (`planeter-i18n-sheet`, replacing the Japanese-only one). **A message
  written or changed after a language was signed off ships and is flagged until a reviewer has read it**: the server logs how many at start, the sheet marks every row, and the
  release notes list the count. For Japanese the count is **0** (`planeter-i18n-sheet --status`: 661 messages unchanged since the 2026-11-14 sign-off).
- **A failed accept is called "refused" only on positive evidence that it changed nothing (found by the 0.56 re-baseline).** Since prikk 0.56.0 the claims are written
  last, so an accept that fails after it has stored the patches and blobs (a write error: disk full, permissions, a failing disk) leaves `sync pending` as it was while the
  store changed. planeter used to read that as "prikk refused it and nothing was left": the proposal was answered *refused*, the intent closed and the artifact discarded,
  with orphaned objects in the store and no recovery flag. It now compares a **store signature** (the size and modification time of every file under `.prikk`, leaving out
  lock files) taken before the accept and after the failure, besides the pending set: only when both are as they were is the answer *refused*; anything else, or a signature
  that cannot be read, is *incomplete* (the repository is flagged, the intent and the artifact are kept, recovery runs, and a resend finishes). Refusals that change nothing
  (a bad signature, a truncated artifact, an over-limit object) are answered as before.
- **Security-relevant dependency update: `ammonia` 4.2.4 → 4.2.5 (RUSTSEC-2026-0533, mutation XSS through embedded MathML and SVG).** `ammonia` cleans every README, description and comment the server shows. planeter's configuration enables none of the tags the advisory needs (no `math`, no `svg`, none of the raw-text tags), so it was not exposed; it is updated anyway, and a regression test keeps the advisory's shape inert and the sanitizer's output a fixed point.
- **Security-relevant dependency update: `rusqlite` 0.32 → 0.41.1 (RUSTSEC-2026-0459).** The bundled SQLite was older than 3.54.0 and could return uninitialised page-cache memory when a process opens a database together with a `-wal` file of a different page size; the bundled SQLite is now 3.54.1. planeter opens only the database file the operator names (`PLANETER_DB`), so it was not reachable from a request; it is updated anyway. **The declared minimum compiler rises from 1.88.0 to 1.89.0** (`rusqlite` needs it). The new dependency `foldhash` (through `rusqlite` → `hashlink` → `hashbrown`) is licensed Zlib, a permissive licence; it is allowed for that one crate only, by an exception in `deny.toml`. `cargo deny` and `cargo audit` report nothing, and no advisory is ignored.

## [0.5.0] — 2026-10-31 — propose, review, create: the first release in which a person can put work to the forge, and the forge still holds no key

0.5.0 is the first release in which a person can **propose work** to the forge, have it **reviewed** (comments and decisions), and **create a repository** (empty, or from a maintainer's archive), with the forge still holding no key: nothing a proposal carries is sealed, merged or published by the server, and nobody approves a patch a keyless server holds. It also ships `planeter-client`, a separate program for a person with their own prikk and their own keys, and a release chain that runs the image it ships. **Breaking changes are listed first** under "For people who run a server" (the verify API's removed fields, the change API's `assurance`, the required prikk 0.54.0, the new defaults of the proposal bounds, the image base).

**Distribution of `planeter-client`:** 0.5.0 ships it through crates.io (`cargo install planeter-client`) and as source; release-asset binaries of the client are not part of this release.

### For people who run a server

- **The verification page no longer shows "N of M sessions verified", and the verify endpoint no longer returns
  `verified_sessions` and `total_sessions` (IS-49) — a breaking change to the success body of
  `GET …/verify`.** prikk's `active_sessions` is not a tally of work (`count` is the number of session folders,
  always one; `verified_count` a fixed 1), so the sentence next to the badge suggested progress that does not exist.
  The checks, the verdict, the stages and the badge are unchanged.

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

- **The container image is now based on Debian 13 (trixie) and is run before it is shipped (D-51).** The image used `debian:bookworm-slim` (glibc 2.36),
  but the release binary — built on a newer system — needs glibc 2.38 (amd64) or 2.39 (arm64) and could not start there (observed). The base is now
  `debian:trixie-slim` (glibc 2.41), pinned by digest. Operators who run the image see no other change; bubblewrap is 0.12 and util-linux 2.41 in it.

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

- **The database gains the tables behind proposed changes (RFC 004 part 3a; nothing uses them yet).** Opening an existing
  database now also creates `changes`, `revisions`, `intents` and `repo_flags` (idempotent; every earlier table is
  untouched, and reopening changes nothing). Nothing writes to them until the transport's write side is built, so a
  server updated to this version behaves as before. The design note is `docs/proposals.md`. The export sandbox now mounts
  only a working directory of its own, never the whole spool (`<spool>/work/<id>`, created and removed per export;
  leftovers from a crash are cleaned when the server starts).

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

- **A cap on what the server reads from a prikk call (IS-55).** The memory ceiling bounded prikk, not the server's own buffer: a hostile archive with
  very many refs could make `archive import --format json` print a report as large as the archive. Every prikk call is now read up to a limit per
  verb (a table with its measurements, each limit at least twice the largest legitimate output measured on prikk 0.54.1) and **killed beyond it**; the
  call fails as a named error, is counted by verb and logged, and is never read as an empty or a partial report. A read is then *unavailable*, an accept
  *incomplete*, a creation refused (the repository removed whole), recovery's witness *unknown*. The start-up probe and the outbound fetch are bounded the
  same way. **Landing is observed from the newest `PLANETER_OBSERVATION_BLOCKS` blocks of each published branch (default 10,000; 100 to 100,000):** a
  proposal whose patches are all found lands; a patch not found in a log that filled the window is shown as *unknown* ("This server cannot tell whether
  this patch has been signed in"), never as held, and the proposal stays open; a landed proposal shows its patches as signed in without reading the log
  again. The counters (`output_too_large` by verb, `sightings_unavailable`) are in the retention sweep's log line and at shutdown. See `docs/proposals.md`.

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

### For people who use it

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

- **Comments and review decisions (RFC 005 part 6).** People can now talk about a proposal and say what they think of a revision. **Comments**
  (Markdown, shown through the sanitizer) sit in one-level threads and may be anchored to a patch, a path and lines; they are never edited or deleted, and a maintainer
  who administers the repository can **hide** one with a reason (its text is then shown to administrators only, everyone else sees the reason) and show it again. A reviewer
  (`write` and above) records one **decision** per revision — approve or request changes — that is shown on that revision and **never carried to a later one**: a later revision shows
  "approved on revision 1" and counts it for nothing. Nobody approves their own work. The proposal page gains one **review line** (*Awaiting review*, *Approved by 2 people*,
  *Changes requested by 1 person*) and, under *Details*, the decisions, the comments and the forms; the list shows the word too. API: `POST …/comments`,
  `…/decisions`, `…/comments/{cid}/hide` and `…/unhide` (bearer tokens only), and `review` and `comments` in the proposal's detail. Limits: 20 comments and 10 decisions per person per proposal per hour
  (`PLANETER_COMMENTS_PER_HOUR`, `PLANETER_DECISIONS_PER_HOUR`). Nothing here merges or blocks anything. See `docs/proposals.md`.

- **Creating a repository — empty, or from a maintainer's archive (RFC 004 part 7).** A person can now make a repository through the pages (`/new`) or the
  API. **Empty**: one step, `POST /api/v1/repos`. **From an archive** (the file `prikk archive export` writes): two steps — upload it and see what it holds
  (the maintainer keys it lists, its branches and where each would land, whether it would pass prikk's checks), then **type the fingerprints of the keys you
  trust** (the whole public key, which the maintainer can read off their own machine; the page never fills them in). Only the keys you typed are trusted; the
  rest are explicitly left out, and their branches are kept as received copies. The repository is built in one order — name reserved, directory, `init`, trust, import,
  `verify`, you as administrator, ready — and **a failure, a timeout or a kill at any step removes it whole**; at every start the server removes (and logs) what a killed
  one left, never following a link and never outside the repository root. Your own namespace or an organization you belong to; one refusal for every other owner; quotas
  per owner (50 / 500); 10 creations an hour per person; names are lowercase and the server's own words (`new`, `api`, `login`, `static`, `import`, … derived from its routes)
  are reserved. A staged archive is kept 24 hours, at most 2 per person. The pages upload with a bounded `multipart/form-data` reader of our own (no new dependency).
  Settings: `PLANETER_IMPORT_MAX_MB`, `PLANETER_CREATE_DEADLINE_SECS`, `PLANETER_STAGE_BUDGET_MB`, `PLANETER_REPO_QUOTA_USER`, `PLANETER_REPO_QUOTA_ORG`,
  `PLANETER_CREATIONS_PER_HOUR`. Measured on prikk 0.54.1: `archive verify` holds 3.0 × the largest blob and `archive import` 6.0 ×, and **prikk bounds neither**.
  See `docs/creation.md`.

- **`planeter-client`: clone, fetch and propose from your own computer (RFC 004 part 9).** A new program, separate from the server, for a person
  with their own prikk and their own keys. `clone` downloads a repository, checks it with your prikk and shows the maintainer keys it carries
  in full; **only the keys you type are trusted**. `fetch` lands the server's newer state as received copies and never touches your branches;
  it refuses a server that shows two histories under one name. `propose` builds your work against what the server already holds and sends it as
  a proposal, then says in plain words that it is held for review and that no branch has changed. It holds no key and signs nothing, speaks
  https only (no `--insecure`, no redirects followed), takes its token from `PLANETER_TOKEN` or a private file (never the command line), and
  says each TLS failure as one sentence with the next step. Linux, macOS and Windows; English, with a Japanese draft. See `docs/client.md`.
  The server gains one read route for it, below.

- **The have-list over HTTP (RFC 004 part 8).** `GET …/transport/have?ref=heads/NAME` serves what the repository already holds of a branch (the
  `PSYNCHV1` file of `prikk sync have`, 32 bytes a patch) so that a client can build an incremental push; the advertisement offers it for each
  published branch. It is made once per state of the repository, under a read lease, checked and served from the spool; a tag and a received
  ref have none (`no_have_list`). See `docs/propose.md`.

- **The advertisement says what a client needs before it sends or fetches (RFC 004 part 10).** `GET …/transport/advertise` now carries `limits`
  (the largest artifact and the most objects a proposal may carry; `null` when proposals are off) and, per branch, `update_seq` (the tip's RefState
  sequence). `planeter-client` uses them: work over a bound is refused before any upload with the server's number in the sentence, and a fetch
  refuses a lower `update_seq`, or the same number with another state, before downloading anything. Nothing existing changed (two new fields).

- **`planeter-client` tells the truth when prikk cannot be started (RFC 004 part 11).** "prikk was not found" is said only when there is no such
  program; a prikk that is there but cannot be started (no permission, a directory, a script whose interpreter is gone, `Text file busy`) is told
  with the system's reason and the next step, and one that starts but does not say its version is told what it said. The program also passes
  `HOME` and `XDG_CONFIG_HOME` on to the prikk it starts explicitly (the same values it always inherited). The `flows` tests no longer write
  executables, and the test server closes connections gracefully; both were causes of failures under load.

- **Two badge and line texts no longer say "approved" (D-50).** The badge on a patch the server holds but a maintainer has not signed in reads
  **Held here, not signed in** (it said "Approved, awaiting signature"), its explanation says the patch is held on this server, and the line
  under a closed proposal reads **Reason: …**. Nobody approves a patch a keyless server holds; "approved" now belongs to a reviewer's decision.
  Nothing in the API changed (`forge-approved-unsealed` is still the machine value).

- **A missing file or folder is now said to be missing (IS-45).** In a repository you may read, a file or folder that
  is not there used to show "There is no repository at this address, or you do not have access to it." It now says
  **"This file or folder was not found in this repository. Check the path, or go back to the files."** with a link to
  the files. A repository that is missing, private or unreadable still gets the old sentence, and **nobody who may not
  read a repository can tell it apart from one that is not there**. **The API is unchanged**: a missing path is the
  same `404 not_found` (same headers and body) as a missing or hidden repository, and `GET …/tree` with a prefix that
  has nothing under it still answers `200` with no entries.

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

- **A Japanese draft and a language switcher (RFC 012 D-3; IS-27) — Japanese is not shipped yet.** The footer of every
  page, the sign-in page included, now has a **language switcher**: a button for each available language, written in
  its own language (English, 日本語), the current one marked; choosing one remembers it in a cookie
  (`planeter_lang`, a language code only) and returns to the same page; no script. The Japanese catalog is a **draft
  for its two reviewers** (the owner and a colleague) and is offered **only** when the operator sets
  `PLANETER_PREVIEW_LANGUAGES=ja` (off by default, logged at start); without it Japanese is not offered and the
  switcher shows English only. English wording is unchanged. The Japanese typography uses system fonts only. New
  checks cover Japanese (completeness, placeholders, retired and forbidden terms, the glossary's Japanese tables,
  every page state), and a test pins that a language not signed off is not shipped.

### Under the hood

- **The release is built with Rust 1.99.1.** The one compiler of CI and release moved from 1.99.0 to 1.99.1, a patch release that fixes a miscompilation of FFI `bool` return values on x86_64. The minimum supported compiler is unchanged (1.88.0).

- **OIDC sign-in verifies ID tokens with `aws-lc-rs` instead of the RustCrypto crates (D-50).** No behaviour changes: the same 30 cases (six algorithms valid, tampered in
  payload and in signature; a key of the wrong kind; HS*, PS* and `none` refused; wrong audience; expired) give the same answers on both backends. The two advisories that
  came with the RustCrypto backend (RUSTSEC-2023-0071 `rsa`, RUSTSEC-2026-0387 `crypto-bigint`) are gone because the crates are, not because they are ignored:
  `cargo deny` and `cargo audit` pass with no ignore at all. The price is a C library (`aws-lc-sys`) in the server binary, about 2 MB more, and a longer clean build.
  Building from source needs `cmake` and a C compiler on the build host (Linux x86_64 and aarch64 were measured).

- **prikk 0.55.0 re-baseline (the floor stays 0.54.0).** Nothing in 0.55.0 is required, so the floor does not move. Re-validated on 0.55.0: the suite (one test drifts, by design: see
  the `extract` entry below), the cost table's seven read verbs and `extract` (no ceiling tripped; memory figures moved by at most 1 MiB), and `sync accept`, which is about
  2.6 to 3.8 times faster for artifacts of many small patches and unchanged in memory. No default (object bound, deadline, ceilings) is changed by this release.

- **The `extract` driver knows prikk 0.55.0's two refusals.** `claim-parent-not-sealed` (a claim whose parent is accepted but not sealed yet; 0.54.x said
  `damaged-repository`) is its own outcome, `ClaimParentNotSealed`, with prikk's text kept verbatim; `destination-inside-repository` is a named driver bug
  (the driver never passes such a destination). The list of ids is 15; the floor stays 0.54.0, so the 0.54.x mapping stays. See `docs/extract.md`.

- **The `extract` driver and the `verify` line of prikk 0.54 (no page, no API yet).** planeter can now ask prikk for the files of a
  point of history (`prikk extract`) into a working directory it makes itself, outside the repository, and removes afterwards:
  prikk 0.54 does not refuse a destination inside the repository, so the driver never takes one from a caller. The extracted tree is
  treated as untrusted and is never read by the driver. prikk's `refusal-v1` document (13 ids) and `extract-report-v1` are read
  strictly; a claim whose parent is still pending is not reported as damage. `verify` now sees prikk's new `unreached objects`
  line and array (a warning, recorded and not shown). The memory cost of `extract` is measured and admitted like a read (about
  twice the largest file, up to 8.6 × for a file edited many times; admitted at 9 × the extraction bound + 8 MiB). See `docs/extract.md`.

- **The container image can no longer be shipped dead (D-51).** The release workflow now loads the image it built, runs it (`planeter`, `prikk`,
  `bwrap`, `prlimit`) and checks the release binary's glibc needs against the image's, in every mode, before anything is pushed or attested; a
  new check (`tools/glibc-check`) and `planeter-pin-check` enforce it.

- **The pushed image is checked to be the image that was run (D-51).** After each architecture's image is pushed, the release workflow reads the
  registry's own index, takes the `linux/<arch>` manifest and requires its config digest to equal the id of the image that was run; a mismatch
  fails the job before the digest is handed on. A new tool (`tools/image-identity`) does the reading and `planeter-pin-check` requires the step.

- **The Japanese draft is consistent with itself (D-50).** `管理者` now means only the administrator of a repository (the person who runs the
  server is `このサーバーを運用している人`), one word per kind of check, `ドキュメント` throughout. Still a draft; still not shipped.

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
