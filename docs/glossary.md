# planeter — words the interface uses (proposal, awaiting the owner's reading)

**Status.** *Proposed* by the architect on 2026-10-23 for RFC 012 D-4 (plain words). It becomes binding when the
owner approves it; until then the catalog keeps today's words. Nothing here changes what a label *means*: a label
that carries trust (what is verified, what is not) keeps its exact meaning and gets plainer words around it.

**Rules (RFC 012 D-4, D-6).** Say what happened and what to do next. Prefer the reader's word to the
implementer's. No jargon without a one-line explanation reachable in place (a `<details>` or a linked help page; not a
tooltip). Short sentences, active voice, no idioms. The same word for the same thing everywhere. Calm and respectful
tone; never blame the reader. No gendered words. The product names `planeter` and `prikk` are names, not jargon.

**Certainty column.** *Confirmed* = checked against prikk's own documentation or planeter's code. *Confirm with
prikk* = my reading, to be checked in our next letter before it is final. *Owner's choice* = a wording decision.

## 1. Concepts

| Concept (what it is) | Today | Proposed | Why | Certainty |
|---|---|---|---|---|
| A change that a maintainer has **signed into the project's history** (prikk: a *sealed* Block) | "sealed", "prikk-verified" | **signed into the history** / badge **Verified** (when the whole repository checks out) | "sealed" is prikk's verb for a maintainer signing queued work into history; "signed" is the word a reader knows | Confirmed (`prikk` data model: *seal* creates a signed Block from queued Patches) |
| A change **approved on this server but not yet signed in** (prikk: a *queued* Patch awaiting a seal) | badge "forge-approved, unsealed" | badge **Approved, awaiting signature**; detail: "This change was approved on this server. A maintainer has not yet signed it into the project's history, so it is not part of the verified record." | The old badge uses two private words. The detail sentence is the part that protects the reader (RFC 005 D-10). | Confirmed meaning; wording is the owner's choice |
| The repository's check **failed** (prikk `verify`: not ok) | badge "unverified" | badge **Verification failed**; detail lists the problems found | "unverified" reads as "not checked". Since IS-39 the page can really say the check found problems. | Confirmed (IS-39) |
| The check **could not run** (outage, budget, timeout) | badge "verify unavailable" | badge **Could not be checked now**; detail: "Try again in a little while." | Separates "failed" from "could not check", which the old wording blurred | Confirmed |
| `verify` | nav "verify", page "Verification" | nav **Verification**; page **Verification** | One word everywhere (sentence case) | Owner's choice |
| `refs` (branches and tags) | nav "refs", titles "refs · …" | nav **Branches and tags**; page **Branches and tags** | "refs" is internal | Confirmed |
| `code` (the file view) | nav "code" | nav **Files** | "code" is wrong for non-code repositories | Owner's choice |
| `history` | nav "history" | nav **History** | Sentence case, as the headings | Owner's choice |
| A recorded unit of history (prikk: *Block*) | "change", "block" | **change** | One word. The page already says "change". | Confirmed |
| The smallest edit inside a change (prikk: *Patch*) | "patch" | **patch**, explained once on the change page ("one recorded edit") | It is prikk's own word and users of a version-control forge know it | Owner's choice (alternative: "edit") |
| A change that undoes an earlier one (prikk: *rollback*) | "rollback" | **rollback**, explained on the change page ("This change undoes an earlier change.") | Familiar word; the explanation is what was missing | Owner's choice |
| Queued, unsigned work counted by `verify` (prikk JSON: `active_sessions` count and `verified_count`) | "{n} of {m} session(s) verified." | **Waiting work: {n} of {m} checked.** (plural forms by language rules) | "session" meant the sign-in session on one page and prikk's queue of unsigned work on another. This is my reading of prikk's `active_sessions`. | **Confirm with prikk** |
| A verification **stage** and its status (`evaluated`, `failed`, `not_evaluated`, `halted`) | "Stage", "evaluated", "failed", "not_evaluated", "halted" | column **Check**; status **completed**, **problem found**, **not run (blocked by {check})**, **stopped early** | "evaluated" does **not** mean "passed" (a stage can complete and still add a problem); "completed" says only that it ran | Confirmed that `evaluated` ≠ passed; wording is the owner's choice |
| The **findings** of a failed check (prikk conditions such as `stage-failure`, `item-failure`, `publication-trust`) | "{id}: {message}" in prikk's English | keep prikk's id and message as the detail, under a plain heading **What was found** | The text is prikk's and cannot be translated by us; the heading can | Confirmed |
| `SSO` | "Sign in with SSO" | **Sign in with single sign-on** | The abbreviation is not universal | Owner's choice |
| `User` field label | "User" | **Username** | It is the account name | Owner's choice |
| Raw file download address (`content origin`) | operator hints only | user messages say **the download address**; operator hints keep "content origin" | The user never needs the term | Owner's choice |
| `read memory budget` | operator hint | operator hint keeps it; user message says "this is too large to show here" | Operator vocabulary stays with the operator | Confirmed |
| `Retry-After`, `OpenAPI`, `GET` | API hints | unchanged (developer-facing) | The API's readers are developers | Owner's choice |

## 2. Messages to rewrite (the pages a person meets most)

Each row replaces the English catalog text. Placeholders (`{like_this}`) are unchanged. *Next step* is always
offered where one exists. Existence-neutral wording of "not found" is kept: it must not tell a stranger whether a
private repository exists.

| Key | Today | Proposed |
|---|---|---|
| `home.intro` | "A forge for prikk repositories. Browse a repository by its {address}." | "planeter hosts prikk repositories. To open one, enter its address, written as {address}." |
| `home.open` | "Open" | "Open repository" |
| `login.failed` | "Sign-in failed." | "The username or password was not correct. Check both and try again." |
| `login.locked` | "Too many sign-in attempts. Wait a few minutes and try again." | unchanged (already plain) |
| `not_found.body` | "There is no such repository, or you do not have access to it." | "There is no repository at this address, or you do not have access to it. Check the address, or sign in if the repository is private." |
| `unavailable.body` | "The repository could not be read right now. Nothing is shown rather than something stale or invented." | "This repository could not be read right now. Try again in a little while. If it keeps happening, tell the person who runs this server." |
| `busy.body` | "Too many requests are being handled right now. Try again in a little while." | unchanged |
| `too_large.heading` / body | "Too large" + the API sentence | heading "Too large to show here"; body "This is too large for this server to show. You can download it, or ask the person who runs this server to raise the limit." (the raw-route variant: "This server does not serve files that large. Ask the person who runs this server.") |
| `file.binary` | "Binary content — {download}." | "This file is not text, so it cannot be shown here. {download}." |
| `history.empty` | "No sealed history on this ref yet." | "There are no signed changes on this branch yet." |
| `repo.location` / `tree.at` | "at {point}" | "Showing {point}" |
| `verify.intro` | "Re-derived from {command} on this request — never served from a cache." | "These checks were run just now, when you opened this page. They are never reused from an earlier visit." (`{command}` is operator vocabulary: dropped from the user text) |
| `page.csrf.body` | "The form could not be checked, so nothing was done." | "The form could not be confirmed as yours, so nothing was done. Go back, reload the page and try again." |
| `page.form_incomplete.body` | "Some required fields were missing, so nothing was done." | "Some fields were empty, so nothing was done. Go back and fill them in." |
| `page.sso_*` bodies | provider-oriented sentences | keep the meaning; each ends with the next step ("Try signing in with your username instead." where password sign-in exists; "Ask the person who runs this server." otherwise) |
| `page.server_error.body` | "The server could not complete the sign-in." | "Something went wrong on the server, and you were not signed in. Try again in a little while." |
| `error.not_found.message`/`hint` (API) | "No such resource." / "Check the owner, the repository name and the path, and that your token may read it." | unchanged (developer-facing, already a whole sentence with a next step) |

## 3. Unresolved before the wording handoff

1. The owner's reading of sections 1 and 2 (anything marked *Owner's choice*, and the two badge sentences above all).
2. **Confirm with prikk** (next letter): that `active_sessions` in the verify report means queued, not-yet-sealed
   work, and the right user-facing name for it.
3. Who reads the Japanese (RFC 012 O-2): the wording must be settled in English **before** translation starts.

## 4. How this is used

After approval the architect writes the wording handoff: the dev team changes the English catalog to these texts
(keys unchanged, checks unchanged), adds the in-place explanations (`<details>`) for *patch*, *rollback* and the
badges, updates the glossary's *Today* column to *Current*, and the Japanese translation then starts from the final
English.
