# Deliveries: how reviewed work becomes the project's branch (RFC 008, 0.8.0)

A **delivery** is how a maintainer's own sealed history becomes the repository's branch `heads/main` on the forge. The maintainer seals the work with their own prikk and their own key; the forge **holds no key and signs nothing**, at any step. It checks what it is sent against a written rule, says in plain words what would change, and — only when the maintainer confirms — replaces the repository by the successor the maintainer sent. This page is for two readers: a **maintainer** who delivers, and an **operator** who runs the server. The decisions are in `rfcs/accepted/008-seal-one-click-merge.md`; the client's side is in `docs/client.md`; the API is in the OpenAPI document (`/api/v1/openapi.json`).

## For a maintainer

You deliver with `planeter-client deliver https://host/owner/name`, run inside your own prikk repository (or with `--repo DIR`), after you have sealed the work you want to land. What a delivery is:

* It sends **the whole repository** (what `prikk archive export` writes), not a patch. It is the same file creation takes. The whole history goes each time.
* The server accepts it **only if yours continues from the project's branch** and every change in it is signed by a key this repository already trusts. A delivery can move the branch forward, and add branches; it cannot rewind, delete a branch or bring in a key.
* Nothing happens until you confirm. The report says what would change; `Deliver it? [y/N]` has no default yes; `--yes` skips the question; `--dry-run` stops after the report and cancels the delivery on the server.
* Who may deliver: a **maintainer** (or an administrator) of the repository, with a token whose scope is **`full`**. A `write` token is capped below merge-seal and cannot deliver: the client says so (`403`).

### The steps

1. **Check, on your side.** The client checks that the directory is a prikk repository with a `heads/main`, that prikk is at least the floor, that there is a token, and — when you did not pass `--yes` — that there is a terminal to ask you on. Nothing has been sent yet.
2. **Export and send.** `prikk archive export` writes the archive into a private scratch directory (removed on every exit path); the client sends it with your token over https.
3. **The report.** The server stages the archive in a scratch repository that trusts **only the keys this repository already trusts** (never the archive's), applies the rule, and answers. A real report:

   ```
   Delivering replaces this repository's branch heads/main with yours. It is accepted only if yours continues from the current one; nothing is signed by this server.
   To carry it out, confirm it. To give it up, cancel it. A delivery that is not confirmed is removed after a day.
   The project's branch would go from 76914127b56e to 7dc412b26b08.
   Your delivery adds 5 changes to this repository's history.
   Open proposals that stay as they are: 2 (#c-5922edb59ef3b8e43d9bdce5 "P3"; #c-e28a2ef02ce25d1c6701aeb2 "P2").
   Proposals whose work is already in your branch, so they will show as landed: 1 (#c-7b2a8d75ebebb715c22b567c "P1").
   ```

4. **Confirm.** The server checks the rule again under the repository's write lease (the branch may have moved since the report; then the delivery is `failed` with the reason `moved` and nothing is changed), records its intent, exchanges the staged repository for the live one, and makes the open proposals' pending work again from the artifacts it kept. It runs on the server, not in your request: if you stop the program it goes on, and the program prints the delivery's id and the address to look at before it starts waiting.
5. **Done.** "The delivery is done: this repository's branch is now yours." A proposal whose work is in your branch shows as **landed** (the observation does that; the delivery does not assert it).

### The rule and its refusals

The rule has five conditions and the first failure is the answer. In order: **(1)** your archive has `heads/main`, it is a local branch there, and nothing in it is received; **(2)** every branch tip and every change is signed by a key this repository trusts, and prikk's `verify` finds no publication-trust issue; **(3)** for every published branch this repository has, its tip is a change of your branch (a fast-forward or the same tip); **(4)** no branch this repository has is missing from yours (a delivery may add branches); **(5)** the delivery **changes at least one branch** (one moves, or one is added). A refusal changes nothing on the server (tested byte for byte), leaves nothing behind, and says what to do:

| reason | what it says | what to do |
|---|---|---|
| `no_main` | Your archive has no branch named `heads/main`. | Deliver the archive of a repository that has the project's branch. |
| `unadopted_key` | Some of your history is signed by a key this repository does not trust. | The report lists the keys that can deliver. Only those can. (A key the repository does not trust cannot be added by a delivery: see "What a delivery does not do".) |
| `not_a_successor` | Your archive does not continue another branch this repository has. | Update your repository from this one, then deliver again. |
| `older_or_diverged` | Your branch does not continue from this repository's branch. | Update your repository from the project's branch, then deliver again. |
| `unrelated` | Your archive has nothing in common with this repository's history. | Deliver from a repository that was made from this one. A different history cannot replace it. |
| `branch_missing` | This repository has a branch your delivery does not have. | A delivery may add branches but never remove one. |
| `nothing_to_deliver` | Your archive changes nothing in this repository. | There is nothing new. (Exit 0, not an error.) |
| `damaged_archive` | The archive could not be read, or it does not check out. | Make the archive again with prikk, and send it again. |

Also possible, as errors of the call or of the install: `too_large` / the `413` (the archive is over the server's bound: the hint says the limit in MiB), `stage_limit`, `busy` (another delivery or another write is in progress: wait, then try again; a busy confirmation cancels your check), a delivery that waited more than a day (`410`, removed), `moved` (see above), and — only after the exchange, flagged for an administrator — `verify_failed` and `publication_trust`. The sentences of each, in both languages, are in the catalog (`delivery.*`).

### What happens to the open proposals

Every proposal that is open when you confirm **stays open**. The server makes its pending work again in the new repository from the artifact it kept for every revision, in order (claim ids are unchanged; a patch your branch already carries is a no-op). The report sorts them before you confirm:

* **stay** — open proposals whose work is not (all) in your branch;
* **land** — open proposals whose every patch is in your branch; they will show as landed;
* **cannot be kept** — a proposal whose retained artifact is gone (removed by retention): it stays readable, its changes can no longer be shown, and the report names it before you confirm. Nothing is deleted.

### What a clone does afterwards

A **fresh `clone`** has the project's branch exactly: its `heads/main` is the delivered tip. A clone made **earlier** receives the new tip through `fetch` as a **received copy** and never has its own branches touched; the client says so in one sentence. To follow it in that clone you `merge --from` with prikk, which makes **your own block**, not the forge's. (`docs/client.md`, "When the project's branch moves".) The client's freshness record is per clone, so a second clone of one repository is not told "nothing changed" about a state it never saw (IS-62).

The repository's home page and history page say who delivered last — "Delivered by {name} on {date}: signed by a key this repository trusts and checked by prikk." and "The project's branch is heads/main." — with the keys the repository trusted when the delivery was checked under a `<details>`. A repository that never had a delivery shows nothing new.

### What a delivery does not do

* It **does not add, rotate or revoke maintainer keys**: the repository's trusted keys are the creator's. A maintainer whose key is not trusted cannot deliver.
* It follows **`heads/main` only**: landing counts that branch and no other (a patch in a branch the delivery added lands nothing).
* It **sends the whole history each time**, and the archive is bounded (below); a repository larger than the bound cannot be delivered until the operator raises it. There is no delta form.
* It **cannot rewind**: undoing a delivery is a new delivery of a successor.
* A `write` token cannot deliver.
* It **signs, seals, merges, closes and forces nothing**, and never trusts a key from the archive. A source test holds the engine to that, and the ENF-2 scan covers both binaries.

## For an operator

**When deliveries are on.** Exactly when proposals are: with `PLANETER_SPOOL_DIR` and `PLANETER_ARTIFACT_DIR`. Otherwise there are no delivery routes. The start-up line says it:

```
planeter: deliveries on: an archive of up to 100 MiB, 600 s deadline, 1024 MiB of staged archives, at most 2 at once, one per repository; a delivery not confirmed in a day is removed
```

**The limits** are creation's (no new setting): the archive is bounded by `PLANETER_IMPORT_MAX_MB` (default 100 MiB, `413`); the check has a 600 s deadline; a person may have at most 2 deliveries in flight and all deliveries together stage at most 1,024 MiB; one delivery is in flight per repository; a delivery that is not confirmed in a day is removed (an hourly sweep). Every route is rate limited per address and per person.

**What it costs.** The check is `archive verify` (3.0 × the archive) plus `archive import` (6.0 ×), admitted at the creation cost model from the heavy pool. The disk holds the archive, the staged repository (about the archive again) and the previous generation of the repository until the end, on the repositories' own filesystem (a delivery whose stage would be on another filesystem is answered `unavailable`). The confirmation holds the repository's **write lease** for the whole install: 0.3 s for a small repository, about 1.4 s for a 63 MiB archive with a 20 MiB proposal, about 6 s (12 s on a loaded host) with a 1,000-patch revision, because the install makes every open proposal's pending work again; during it an accept or a second delivery is `busy`, and reads are not delayed. The time grows with the number and size of the open proposals, not with the delivery.

**The install, in order.** The write lease; the rule again over fresh readers (the archive's digest, the live tip); the intent, durably, **before anything moves**; the exchange (two ordinary renames, the previous generation kept: the repository's directory is absent for about 60–90 µs, and a request that opens it then is answered `503 unavailable` and can be retried); the revision counter; the re-accept of every open proposal's artifacts; `verify` of the new repository; done, and the delivery's directory is removed.

**Recovery.** A kill **before the exchange** abandons the delivery; the old repository is untouched. A kill **at or after the exchange** is **completed at the next start** (the intent decides), or by an administrator's recovery run: the repository is flagged `a write did not finish (delivery_unfinished)`, `GET …/recovery` (for the repository's administrator) shows why and the exact commands, the stale locks of a killed prikk call are cleared with `unlock --force` after the recovery's witnesses agree (never `--yes`), and the install is finished, not rolled back. A killed *check* is never resumed: it is thrown away and made again. A re-accept that fails, or a `verify` that fails, stops with the repository flagged and the previous generation kept for an administrator; a failing publication-trust check is logged in capitals. The procedure is `docs/propose.md`, "Recovery".

**Counters and logs.** At stop: `planeter: deliveries: accepted_reports=… done=…` (one counter per outcome). No log line names a file, a branch or a key, and none shows content (a test holds that over a delivery, three refusals, a kill and a restart).

**A debug build** of the server has a test hook, `PLANETER_FAILPOINT`, that stops the engine at a named point so that kills can be tested; a release build does not contain it (the ENF-2 script fails if the release binary contains the name). It is not a setting.

### `sync seal`, measured

The forge never runs `prikk sync seal`: it is the maintainer's command, with the maintainer's key. It is where the work of a delivery is done, so its cost is measured here as it was measured (prikk 0.58.0; each claim adds one new file, so the tree ends with N paths):

| claims | memory (peak) | time | read |
|---|---|---|---|
| 100 | 1.0 MiB | 10.2 s | 2.2 MB |
| 1,000 | 7.1 MiB | 104 s | 23 MB |
| 10,000 | 183.6 MiB | 670 s | 233 MB |

Measured on an encrypted btrfs disk; it reads about 23 KB a claim. **The memory of `sync seal` grows with the number of claims. With a tree that stays the same size it grows linearly (68 MiB at 10,000 claims). With a tree that gains a file per claim it grows faster than linearly (181 MiB at 10,000 claims).** The time depends on the disk: about 90 s on tmpfs and 670 s on an encrypted btrfs disk for the same work. (prikk 0.57.0 took 139 s and read 5.5 GB for 1,000 claims.) These are the only claims made about its memory or its speed. No speed is claimed for prikk 0.58.0 as a whole: its fsyncs fell, the wall time per patch of an accept did not on the measuring machine.

## Where it is tested

The rule against every archive of the measurement (older, divergent, unrelated, an unadopted key, a key merely carried, an unadopted block under an adopted tip, truncated and flipped archives), in both states of the forge, with the live `.prikk` tree byte-identical after every refusal; the install with a kill at every point (a real `SIGKILL` of the process, then a restart on the same data); the routes over HTTP with real prikk; the real server with the real sandbox; the real client against the real server (`crates/planeter/tests/client_deliver_e2e.rs`); the landed view against `diff -r`. See the review requests of each handoff.
