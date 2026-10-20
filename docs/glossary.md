# planeter — words the interface uses

**Status.** Read and approved by the owner (2026-10-21) and **in force** in the English catalog
(`crates/planeter-web/src/catalog/en.rs`) since the plain-words work (RFC 012 D-4). One row is **held** for prikk's
confirmation (the `session(s)` count, section 1). Rows marked *Awaiting the owner* are sentences the dev team had to
write because this page gave none; they are in force until the owner changes them. The test
`crates/planeter-web/tests/glossary.rs` reads the *Current* cell of every row in section 2 and fails if it is not
exactly the English catalog text: **change the words on this page and in the catalog together**. Nothing here
changes what a label *means*: a label that carries trust (what is verified, what is not) keeps its exact meaning
and has plainer words around it.

**Rules (RFC 012 D-4, D-6).** Say what happened and what to do next. Prefer the reader's word to the
implementer's. No jargon without a one-line explanation reachable in place (a `<details>` or a linked help page; not a
tooltip). Short sentences, active voice, no idioms. The same word for the same thing everywhere. Calm and respectful
tone; never blame the reader. No gendered words. The product names `planeter` and `prikk` are names, not jargon.

**Certainty column (section 1).** *Confirmed* = checked against prikk's own documentation or planeter's code.
*Confirm with prikk* = the architect's reading, to be checked in the next letter before it is final. *Owner's
choice* = a wording decision.

**Status column (section 2).** *Approved* = the text the owner approved. *Derived* = the approved rule applied by
the dev team to a message the page did not list (sentence case; the approved sentence kept as one sentence where an
existing API test requires one; a next step chosen per page). *Awaiting the owner* = written by the dev team
because no sentence was given. *Held* = unchanged on purpose.

## 1. Concepts

| Concept (what it is) | Was | Current | Why | Certainty |
|---|---|---|---|---|
| A change that a maintainer has **signed into the project's history** (prikk: a *sealed* Block) | "sealed", "prikk-verified" | **signed into the history**; badge **Verified** (on the repository, history and verification pages: when the whole repository checks out) | "sealed" is prikk's verb for a maintainer signing queued work into history; "signed" is the word a reader knows. On the change page a signed patch also carries **Verified**, but that page does not re-check the repository, so its explanation says only what is known there: the patch is signed in (`explain.signed`) | Confirmed (`prikk` data model: *seal* creates a signed Block from queued Patches) |
| A change **approved on this server but not yet signed in** (prikk: a *queued* Patch awaiting a seal) | badge "forge-approved, unsealed" | badge **Approved, awaiting signature**; explanation `explain.queued` | The old badge used two private words. The explanation is the part that protects the reader (RFC 005 D-10). | Confirmed meaning; wording is the owner's choice |
| The repository's check **failed** (prikk `verify`: not ok) | badge "unverified" | badge **Verification failed**; the Verification page lists what was found | "unverified" reads as "not checked". Since IS-39 the page can really say the check found problems. | Confirmed (IS-39) |
| The check **could not run** (outage, budget, timeout) | badge "verify unavailable" | badge **Could not be checked now**; explanation "Try again in a little while." | Separates "failed" from "could not check", which the old wording blurred | Confirmed |
| `verify` | nav "verify", page "Verification" | nav **Verification**; page **Verification** | One word everywhere (sentence case) | Owner's choice |
| `refs` (branches and tags) | nav "refs", titles "refs · …" | nav **Branches and tags**; page **Branches and tags** | "refs" is internal | Confirmed |
| `code` (the file view) | nav "code" | nav **Files** | "code" is wrong for non-code repositories | Owner's choice |
| `history` | nav "history" | nav **History** | Sentence case, as the headings | Owner's choice |
| A recorded unit of history (prikk: *Block*) | "change", "block" | **change** | One word. The page already said "change". | Confirmed |
| The smallest edit inside a change (prikk: *Patch*) | "patch" | **patch**, explained on the change page: "A patch is one recorded edit." | It is prikk's own word and users of a version-control forge know it | Owner's choice (alternative: "edit") |
| A change that undoes an earlier one (prikk: *rollback*) | "rollback" | **rollback**, explained on the change page: "A rollback is a change that undoes an earlier change." | Familiar word; the explanation was what was missing. The page cannot say "This change undoes an earlier change." because it does not know whether the change is a rollback, so the explanation is a definition. | Owner's choice |
| Queued, unsigned work counted by `verify` (prikk JSON: `active_sessions` count and `verified_count`) | "{n} of {m} session(s) verified." | **Held — unchanged** until prikk confirms what `active_sessions` means; the proposal is **Waiting work: {n} of {m} checked.** (plural forms by language rules) | "session" means the sign-in session on one page and prikk's queue of unsigned work on another. This is the architect's reading of prikk's `active_sessions`. | **Confirm with prikk** |
| A verification **stage** and its status (`evaluated`, `failed`, `not_evaluated`, `halted`) | "Stage", "evaluated", "failed", "not_evaluated", "halted" | column **Check**; status **completed**, **problem found**, **not run (blocked by {check})**, **stopped early** | "evaluated" does **not** mean "passed" (a stage can complete and still add a problem); "completed" says only that it ran | Confirmed that `evaluated` ≠ passed; wording is the owner's choice |
| The **findings** of a failed check (prikk conditions such as `stage-failure`, `item-failure`, `publication-trust`) | "{id}: {message}" in prikk's English | prikk's id and message as the detail, under a plain heading **What was found** | The text is prikk's and cannot be translated by us; the heading can | Confirmed |
| `SSO` | "Sign in with SSO" | **Sign in with single sign-on** | The abbreviation is not universal | Owner's choice |
| `User` field label | "User" | **Username** | It is the account name | Owner's choice |
| Raw file download address (`content origin`) | operator hints only | no user message names it; operator hints keep "content origin" | The user never needs the term | Owner's choice |
| `read memory budget` | operator hint | the operator hint keeps it (`error.too_large.hint`, the API); the page says "This is too large for this server to show." | Operator vocabulary stays with the operator | Confirmed |
| `Retry-After`, `OpenAPI`, `GET` | API hints | unchanged (developer-facing) | The API's readers are developers | Owner's choice |

## 2. Messages

Each row is a catalog key. *Current* is the English catalog text, character for character (placeholders
`{like_this}` included); the test compares them. *Was* is the text before the plain-words work. *Next step* is
always offered where one exists. Existence-neutral wording of "not found" is kept: it must not tell a stranger
whether a private repository exists. Messages not listed here did not change (for example `login.locked`,
`busy.body`, and the API's `error.*` texts apart from `error.too_large.raw_hint`).

### Navigation, titles and signing in

| Key | Was | Current | Status |
|---|---|---|---|
| `layout.sign_in` | "sign in" | "Sign in" | Derived |
| `layout.sign_out` | "sign out" | "Sign out" | Derived |
| `nav.code` | "code" | "Files" | Approved |
| `nav.history` | "history" | "History" | Approved |
| `nav.refs` | "refs" | "Branches and tags" | Approved |
| `nav.verify` | "verify" | "Verification" | Approved |
| `title.history` | "history · {repo} · {product}" | "History · {repo} · {product}" | Derived |
| `title.change` | "change {target} · {repo} · {product}" | "Change {target} · {repo} · {product}" | Derived |
| `title.refs` | "refs · {repo} · {product}" | "Branches and tags · {repo} · {product}" | Derived |
| `title.verify` | "verify · {repo} · {product}" | "Verification · {repo} · {product}" | Derived |
| `login.failed` | "Sign-in failed." | "The username or password was not correct. Check both and try again." | Approved |
| `login.sso` | "Sign in with SSO" | "Sign in with single sign-on" | Approved |
| `login.user` | "User" | "Username" | Approved |

### Pages a person meets most

| Key | Was | Current | Status |
|---|---|---|---|
| `home.intro` | "A forge for prikk repositories. Browse a repository by its {address}." | "planeter hosts prikk repositories. To open one, enter its address, written as {address}." | Approved |
| `home.open` | "Open" | "Open repository" | Approved |
| `not_found.body` | "There is no such repository, or you do not have access to it." | "There is no repository at this address, or you do not have access to it. Check the address, or sign in if the repository is private." | Approved |
| `unavailable.body` | "The repository could not be read right now. Nothing is shown rather than something stale or invented." | "This repository could not be read right now. Try again in a little while. If it keeps happening, tell the person who runs this server." | Approved |
| `too_large.heading` | "Too large" | "Too large to show here" | Approved |
| `too_large.body` | — | "This is too large for this server to show. You can download it, or ask the person who runs this server to raise the limit." | Approved |
| `error.too_large.raw_hint` | "This server does not serve files that large; ask the operator." | "This server does not serve files that large; ask the person who runs this server." | Derived |
| `repo.location` | "at {point}" | "Showing {point}" | Approved |
| `tree.at` | "at {point}" | "Showing {point}" | Approved |
| `file.binary` | "Binary content — {download}." | "This file is not text, so it cannot be shown here. {download}." | Approved |
| `history.empty` | "No sealed history on this ref yet." | "There are no signed changes on this branch yet." | Approved |
| `page.csrf.body` | "The form could not be checked, so nothing was done." | "The form could not be confirmed as yours, so nothing was done. Go back, reload the page and try again." | Approved |
| `page.form_incomplete.body` | "Some required fields were missing, so nothing was done." | "Some fields were empty, so nothing was done. Go back and fill them in." | Approved |
| `page.sso_off.body` | "This server has no single sign-on provider configured." | "This server has no single sign-on provider configured. Try signing in with your username instead." | Derived |
| `page.sso_down.body` | "The sign-in provider could not be reached." | "The sign-in provider could not be reached. Try signing in with your username instead." | Derived |
| `page.sso_incomplete.body` | "The sign-in provider's reply was incomplete." | "The sign-in provider's reply was incomplete. Try signing in with your username instead." | Derived |
| `page.sso_mismatch.body` | "The sign-in could not be matched to the request that started it." | "The sign-in could not be matched to the request that started it. Try signing in with your username instead." | Derived |
| `page.sso_expired.body` | "The sign-in took too long and has expired." | "The sign-in took too long and has expired. Try signing in with your username instead." | Derived |
| `page.sso_refused.body` | "The sign-in provider did not accept the sign-in." | "The sign-in provider did not accept the sign-in. Try signing in with your username instead." | Derived |
| `page.sso_unlinked.body` | "This single sign-on identity is not linked to a planeter account." | "This single sign-on identity is not linked to a planeter account. Ask the person who runs this server." | Derived |
| `page.server_error.body` | "The server could not complete the sign-in." | "Something went wrong on the server, and you were not signed in. Try again in a little while." | Approved |

### Badges and their explanations

| Key | Was | Current | Status |
|---|---|---|---|
| `assurance.verified` | "prikk-verified" | "Verified" | Approved |
| `assurance.queued` | "forge-approved, unsealed" | "Approved, awaiting signature" | Approved |
| `assurance.unverified` | "unverified" | "Verification failed" | Approved |
| `assurance.verify_unavailable` | "verify unavailable" | "Could not be checked now" | Approved |
| `explain.summary` | — | "What does this mean?" | Approved |
| `explain.queued` | — | "This change was approved on this server. A maintainer has not yet signed it into the project's history, so it is not part of the verified record." | Approved |
| `explain.unavailable` | — | "Try again in a little while." | Approved |
| `explain.verified` | — | "prikk ran all of its checks on this repository just now, and they found no problems." | Awaiting the owner |
| `explain.signed` | — | "A maintainer signed this patch into the project's history. The Verification page shows whether the whole repository checks out." | Awaiting the owner |
| `explain.failed` | — | "prikk checked this repository and found problems. The Verification page lists what was found." | Awaiting the owner |
| `explain.patch` | — | "A patch is one recorded edit." | Derived |
| `explain.rollback` | — | "A rollback is a change that undoes an earlier change." | Awaiting the owner |

### The verification page

| Key | Was | Current | Status |
|---|---|---|---|
| `verify.intro` | "Re-derived from {command} on this request — never served from a cache." | "These checks were run just now, when you opened this page. They are never reused from an earlier visit." | Approved |
| `verify.findings` | — | "What was found" | Approved |
| `verify.col.stage` | "Stage" | "Check" | Approved |
| `verify.status.evaluated` | "evaluated" | "completed" | Approved |
| `verify.status.failed` | "failed" | "problem found" | Approved |
| `verify.status.not_evaluated` | "not_evaluated" | "not run (blocked by {check})" | Approved |
| `verify.status.halted` | "halted" | "stopped early" | Approved |
| `verify.status.not_run` | — | "not run" | Awaiting the owner |
| `verify.sessions` | "{verified} of {count} session verified." / "{verified} of {count} sessions verified." | "{verified} of {count} session verified." / "{verified} of {count} sessions verified." | Held for prikk's confirmation |


## 3. Open

1. ~~The owner's reading of sections 1 and 2~~ — done, approved (2026-10-21).
2. **Confirm with prikk** (next letter): that `active_sessions` in the verify report means queued, not-yet-signed
   work, and the right user-facing name for it. Until then `verify.sessions` is held.
3. **Awaiting the owner:** the five rows marked so in section 2 (`explain.verified`, `explain.signed`,
   `explain.failed`, `explain.rollback`, `verify.status.not_run`) and the per-page choice of the next step in the
   `page.sso_*` bodies.
4. Who reads the Japanese (RFC 012 O-2): the wording is settled in English **before** translation starts.

## 4. How this is used

The catalog follows this page. A change to a user-facing message is a change to this page first (then the
catalog, in the same commit); `tests/glossary.rs` fails if a *Current* cell and the catalog disagree, and
`tests/retired_words.rs` fails if a retired word (`unsealed`, `forge-approved`, `prikk-verified`, `not_evaluated`,
`evaluated`, `refs`, `SSO`, or the old navigation labels) comes back. The Japanese translation starts from this
English.
