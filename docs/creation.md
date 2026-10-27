# Creating a repository — empty, or from a maintainer's archive

**Status.** RFC 004 part 7 (built 2026-10-27, awaiting review). What is promised here is held by tests that run in the gates
(`crates/planeter-core/tests/creation_*.rs`, `crates/planeter-store/tests/creation_store.rs`,
`crates/planeter-transport/tests/creation_unit.rs`, `crates/planeter-web/tests/{creation_http,multipart}.rs`,
`crates/planeter/tests/creation_kill.rs`). The measurements behind the numbers are in the part 7 review request
(`repository-creation-logs/t0/`).

A repository is made by a person, through the pages or the API. There are two ways, and **neither publishes anything**: the forge is
keyless (INV-2). What a repository holds when it is made is either nothing, or the history a maintainer already published and signed.

## 1. The two ways

| | Empty | From an archive |
|---|---|---|
| What it is | A repository with no history; you are its administrator. | A repository that starts with all the history of a maintainer's repository, taken from the file `prikk archive export` writes. |
| Steps | One. | Two: **stage** (upload and look), then **confirm** (type the fingerprints of the keys you trust). |
| Page | `/new` | `/new` → `/new/import/{stage}` |
| API | `POST /api/v1/repos` | `POST /api/v1/repos/import`, `GET /api/v1/repos/import/{stage}`, `POST /api/v1/repos/import/{stage}/confirm` |
| Needs | nothing but the database and prikk | `PLANETER_ARTIFACT_DIR` (the staged archives wait in its `stages/`) |

Who may create where: **in your own namespace** (the owner is your username) or **in an organization you belong to**
(`authorize(CreateRepo)`: any member). Every other owner — someone else's name, an organization you are not in, one that does not exist —
is **one refusal** (`403 forbidden`), byte for byte, so the answer tells a stranger nothing about which organizations exist. A taken name
(`409 name_taken`) is said only to someone who may create there. The API takes **bearer tokens only** (a session cookie is what another
site's request would carry); the pages take sessions with a CSRF token. A read-only token creates nothing.

Names: lowercase letters, digits, `.`, `_`, `-`; start with a letter or a digit; at most 64; no `..`; not ending in `.` or `.git`;
**not a word the server uses for itself**. Uppercase is refused so that two names can never differ only by case. The reserved words are
**derived from the router** (the first segment of every route, and any word standing where another route has a parameter), never listed
by hand: `new`, `api`, `login`, `logout`, `static`, `lang`, `go`, `import` today. A route added tomorrow reserves its word tomorrow, and
`tests/creation_http.rs` fails if a name a route owns could be created.

Limits: a **quota** per owner (50 repositories for a user, 500 for an organization; a repository being made counts), **10 creations or
stagings per person per hour**, and for archives the limits of §3.

## 2. A repository is made in one order, and an unfinished one is removed whole

```
reserve the name  →  lay out the directory  →  prikk init  →  …the rest of the way…  →  the creator becomes administrator  →  ready
 (record: creating)                                                                                            (record: ready)
```

* The name is **reserved first** (a record in the state `creating`), checked before any byte is written; the record is unique with the
  ready ones, so two creations of one name cannot both succeed.
* A `creating` repository is **invisible and unroutable** everywhere: the store answers for it exactly as for an absent one (`get`,
  `resolve`, `list_by_owner`), so the read gate says `404`, the registry's probe covers it, and it is never in a listing. It counts in its
  owner's quota (a reservation holds its place), and in nobody else's.
* `ready` is the **last** step. Any failure, timeout or kill before it removes the repository **whole**: the directory, then the record
  (in that order, so a record never outlives the loss of its directory while a directory outlives its record), then the creator's grant.
  While the process lives a guard does it (an error, an early return and a panic are all covered); after a kill the next start does.

### The start-up sweep

At every start, **before** anything reads the repositories or the lease records, the server removes and **logs** each of:

1. every repository whose record is `creating` (its directory, its record, its grants, and the lease record the killed process left);
2. every directory under the repository root that no record owns;
3. every stage whose confirmation was running when the server stopped, every expired stage, and every stage directory no record owns.

Its guards, because it deletes directories (all tested, among them with a symbolic link planted in the repository root and in the stage
root):

* it removes only a path **inside** the repository root, resolved through the file system (a shard that is a link to somewhere else
  resolves outside the root and is refused), and it **never follows a link** (a link is unlinked for a `creating` repository and left alone otherwise);
* it descends only into entries named like a shard (two characters of `0-9a-z_`) and removes only entries named like a repository id
  (`0-9a-z`, starting with the shard); anything else under the root is not planeter's and is left;
* it removes a record's directory only where the layout puts that record's id;
* **an empty store with directories on disk is not swept**: that is a different database, not an unfinished creation, and the server says so.

The log lines (stderr): `start-up sweep: removed the unfinished creation of owner/name (id)`, `… removed <path>, a directory no
repository record owns`, `… removed stage <id> (…)`, `… cleared the lease record of the unfinished creation <id>`,
`… left <path> alone: <why>`, and a summary line.

*Runbook.* Nothing is to be done after a kill: start the server. If a `left … alone` line appears, an operator looks at that path (a link, a
file, a directory that is not where the layout puts it) and removes it by hand; the sweep will not. If the store is empty and the
repository root is not, the database is the wrong one.

## 3. From an archive

### Step 1 — stage and verify

The archive is written to the stage's directory (`<PLANETER_ARTIFACT_DIR>/stages/<id>/in/archive`, mode 0600), **outside every
repository**, hashed as it arrives and never held in memory. The size is checked before the first byte is read
(`PLANETER_IMPORT_MAX_MB`, default 100 MiB; `413`), the person's stage limit before the first byte is read too (at most 2 stages; a total
budget `PLANETER_STAGE_BUDGET_MB`, default 1024 MiB), and who may create there and whether there is room are decided from the metadata
**before any byte of the archive is read**.

Then, in a **scratch repository inside the stage's directory** (read-only mount of the archive, nothing else), the forge runs:

1. `prikk archive verify` — structure, closure, every signature against the archive's own carried material. An archive prikk refuses is
   `422 archive_refused` with **prikk's own words** behind our sentence, and **nothing is kept**;
2. to learn *where each of the archive's branches would land*, it trusts every carried key **in the scratch repository only** and runs
   `archive import --adopt` and `verify` there (prikk's `archive verify` lists the keys but not the branches; the import report is the only
   place prikk says which branches an archive holds and why one would not be published). The scratch repository is deleted when the stage
   is made; it is never a hosted repository and nothing trusts it.

The response is the **stage**: its id, a day's expiry, the archive's size and SHA-256, "internally consistent" (not a statement of trust),
the **carried maintainer keys**, the branches and, for each, whether it would be published and prikk's reason if not, the object counts, and
whether the repository made with every key trusted would pass prikk's checks.

> **The fingerprint.** prikk gives a maintainer key a short id (`ed25519-` and 16 hex characters) and the public key (64 hex characters,
> what `prikk key public` prints on the maintainer's machine). The fingerprint this forge shows and asks for is **the whole public key**: it
> is the one thing a maintainer can read off their own machine, which is the only way a creator can compare it through a channel the
> archive does not control. The API's `fingerprint` and `public_key` are the same text (the creator compares one; `trust maintainer add`
> takes the other), and `fingerprint_grouped` is the same in groups of eight, for the page.

A stage is kept **24 hours**, then removed (at start and every hour; and before a new stage is made). It is **never served back**. Only
the principal who staged it can read it or confirm it; anyone else, an unknown id and a removed stage are the same `404`.

### Step 2 — confirm

`POST …/import/{stage}/confirm` with `{adopt: [{key_id, fingerprint}], leave_out?: [key_id], no_adopt?: bool}`, by the principal who staged it.

> **Trust is the creator's act, never the archive's.** The fingerprints must be **typed or pasted by the creator**: the page's form lists
> the carried keys *beside* empty fields and never prefills one, and after a refusal the fields are empty again. Each must equal the carried key's
> whole fingerprint (ignoring case, spaces and colons); a prefix, the short id or another key's text is refused, naming the key and **never
> showing what was expected**. **Every carried key must be adopted or explicitly left out** (`leave_out`, or `no_adopt` for all); nothing
> chosen is not a choice. No key is ever trusted that was not typed, and `archive import --adopt` is never run unless a matching
> `trust maintainer add` was recorded for exactly the typed keys.

The sequence, as one operation under the **new repository's W lease** (own deadline, `PLANETER_CREATE_DEADLINE_SECS`, default 600 s; memory
admitted from the heavy pool by §4):

1. the stage is **claimed** (once at a time: a second confirmation is `409 stage_in_use`; after a success the stage is gone: `404`);
2. who may create there, the quota and the name are asked again (a membership may have changed), and the staged file is hashed again — the
   bytes imported are the bytes that were verified;
3. reserve → `init` → `trust maintainer add` for exactly the typed keys → `archive import` (`--adopt` only when a key was typed) → `verify` →
   the creator becomes the administrator → `ready`;
4. the stage is removed.

**The rule for the final `verify`.** A repository whose every carried key was adopted must `verify` clean (`verified: true`). When the creator
left a key out (or typed none), a branch signed by a key left out lands under `remotes/` and `verify` exits 1 with **`publication-trust` and
nothing else**: that is the consequence of the choice, reported as `verified: false` with the page's plain sentence that *nothing is verified or
trusted until a key is trusted* (RFC 004 Q-2). Any other failed condition — or `publication-trust` when every key was adopted, which means the
archive carries a branch signed by a key it does not list — removes the repository (`422 creation_verify_failed`, with the failed conditions)
and the stage with it. The stage tells you beforehand (`verifies_with_all_keys`, `verify_failures`).

**This version cannot trust a key after the repository exists** (advancing a published branch is RFC 004 D-8, undecided); the result page says so.

When a step fails for a reason that is not the archive's (busy, timed out, a ceiling, something broken) the stage is released for another
try; when it is the archive's (prikk refused it, it does not verify) the stage is removed with the repository.

## 4. Memory, time and what prikk does not bound

Measured with prikk 0.54.1 on eleven archives from 0.12 MiB to 256 MiB (the table is in the review request). The memory of both calls
follows the **largest single blob**, which nothing before prikk reads the archive can tell, so — as in part 4b — the whole archive is assumed
to be one:

| Call | Data segment | Admission cost | Ceiling (× 1.5) |
|---|---|---|---|
| `archive verify` | 3.00 × the largest blob (0.03 to 0.15 × for archives of many files) | 10 MiB + 3.0 × the archive | `verify_ceiling` |
| `archive import` (and the `verify` after it) | 6.00 × the largest blob (0.07 to 0.24 ×) | 10 MiB + 6.0 × the archive | `import_ceiling` |

One staging, or one confirmation, is admitted once, for the largest of its calls (the import's), from the heavy pool; at the default 100
MiB bound that is 610 MiB, and the start-up line says whether the budget admits it. The constants live in **one place**,
`planeter_transport::creation::cost_model`, with their provenance, and are re-measured at each prikk re-baseline. **Time** follows the
number of objects (3.3 ms each unloaded, 7.7 ms median under load, 12 ms for a history of many small blocks): a history of tens of
thousands of objects needs a longer `PLANETER_CREATE_DEADLINE_SECS`.

**prikk bounds nothing of this itself:** `PRIKK_BUNDLE_MAX_*`, `PRIKK_EXCHANGE_MAX_*` and `incoming.max-object-bytes` are ignored by
`archive verify` and `archive import` (measured). The edge size bound, the memory ceiling and the deadline are all the protection there is.
A call that hits the ceiling aborts (`SIGABRT`, "memory allocation of N bytes failed") and is reported as a named refusal
(`create_too_big`), never as damage; one that hits the deadline is killed and the repository removed whole.

## 5. The settings

| Variable | Default | What |
|---|---|---|
| `PLANETER_ARTIFACT_DIR` | unset | Where staged archives wait (`stages/`). Unset: creation from an archive is off (empty creation still works). |
| `PLANETER_IMPORT_MAX_MB` | 100 | The largest archive. |
| `PLANETER_CREATE_DEADLINE_SECS` | 600 | The wall-time bound of one staging and of one confirmation. |
| `PLANETER_STAGE_BUDGET_MB` | 1024 | The most MiB of staged archives the server holds (and at most 2 per person, each 24 hours). |
| `PLANETER_REPO_QUOTA_USER` | 50 | Repositories one user may own. |
| `PLANETER_REPO_QUOTA_ORG` | 500 | Repositories one organization may own. |
| `PLANETER_CREATIONS_PER_HOUR` | 10 | Creations or stagings one person may start in an hour. |

## 6. What is not here

No push or landing of newer history into an existing repository (D-8), no second archive into one, no rename, transfer or deletion of a
repository, no organization or account management, no SSH, no client helper. Nothing here changes `propose`, recovery or review.
