# planeter — words the interface uses

**Status.** Read and approved by the owner (2026-10-21) and **in force** in the English catalog
(`crates/planeter-web/src/catalog/en.rs`) since the plain-words work (RFC 012 D-4) and its refinements
(2026-10-21: the dev team's questions Q1–Q9 were answered by the architect's review, and the change page's badge
became **Signed into the history**, IS-44). The test
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

**Status column (section 2).** *Approved* = the text the owner approved, or a sentence the dev team wrote where this
page gave none and the architect's review accepted (including the one-sentence form of the raw-route hint, which an
API test requires). *Derived* = the approved case rule applied to a label this page did not list, or a sentence a handoff specified (IS-45).

**Case.** Standalone labels, headings, titles, links and badges in a heading are in sentence case ("Sign in",
"History · {repo} · {product}", "Verified"). **In-cell tags and status words are lowercase** ("rollback",
"current", "closed", "folder", "completed", "problem found"): they sit inside a row or beside a name and are not
labels of their own.

## 1. Concepts

| Concept (what it is) | Was | Current | Why | Certainty |
|---|---|---|---|---|
| A change that a maintainer has **signed into the project's history** (prikk: a *sealed* Block) | "sealed", "prikk-verified" | on the change page a sealed patch is labelled **Signed into the history** (a plain badge, JSON `"sealed"`); the badge **Verified** is shown only where a verification ran and passed (the repository, history and verification pages: when the whole repository checks out) | "sealed" is prikk's verb for a maintainer signing queued work into history; "signed" is the word a reader knows. The change page runs no verification, so it must not say **Verified** (IS-44); its explanation says what is known there: the patch is signed in, and the Verification page says whether the repository checks out (`explain.signed`) | Confirmed (`prikk` data model: *seal* creates a signed Block from queued Patches); IS-44 |
| A patch **held on this server but not yet signed in** (prikk: a *queued* Patch awaiting a seal) | badge "forge-approved, unsealed", then "Approved, awaiting signature" | badge **Held here, not signed in**; explanation `explain.queued` | The first badge used two private words. The second said *approved*, which nobody does to a patch a keyless forge holds: *approved* belongs to a reviewer's decision about a revision, and on one proposal page one word would mean two things. *Held* is what is true (accepted, unsigned); *not signed in* is the trust-bearing half. The explanation is the part that protects the reader (RFC 005 D-10). History: 2026-10-29: reworded (D-50). | Confirmed meaning; wording by the architect on the owner's delegation (D-50) |
| The repository's check **failed** (prikk `verify`: not ok) | badge "unverified" | badge **Verification failed**; the Verification page lists what was found | "unverified" reads as "not checked". Since IS-39 the page can really say the check found problems. | Confirmed (IS-39) |
| The check **could not run** (outage, budget, timeout) | badge "verify unavailable" | badge **Could not be checked now**; explanation "Try again in a little while." | Separates "failed" from "could not check", which the old wording blurred | Confirmed |
| `verify` | nav "verify", page "Verification" | nav **Verification**; page **Verification** | One word everywhere (sentence case) | Owner's choice |
| `refs` (branches and tags) | nav "refs", titles "refs · …" | nav **Branches and tags**; page **Branches and tags** | "refs" is internal | Confirmed |
| `code` (the file view) | nav "code" | nav **Files** | "code" is wrong for non-code repositories | Owner's choice |
| `history` | nav "history" | nav **History** | Sentence case, as the headings | Owner's choice |
| A recorded unit of history (prikk: *Block*) | "change", "block" | **change** | One word. The page already said "change". | Confirmed |
| What a person **puts forward for review** (the code calls it a *change*, internally) | "change" (RFC 005's working word; the propose API, the quota and error texts said it) | **proposal** | *Change* is taken: it is the one word for a recorded unit of history. One word for two things is the confusion this page exists to remove, and nothing was released yet (D-38). Japanese 提案, the word the draft already used for *propose*. | Drafted by the dev team (RFC 005 part 5 T0) — to be approved by the architect before merge, and flagged for the owner, who approved this page |
| One accepted push of a proposal (prikk artifact: one `sync accept`) | "revision" | **revision** | Unchanged: a proposal's versions are revisions, numbered 1, 2, … (Japanese リビジョン, as in the draft). | Drafted (as above) |
| A proposal whose every patch is **in a block of a published ref of the repository** (D-40; `sync pending` is not consulted) | "landed" | **landed** (state *Landed*): "Landed" means *every patch is in a block of a published branch of this repository*, which only a key-holding action puts there; nothing is merged by the server, and history merely **received** here is never evidence (it can be replaced). | Observed from facts, not asserted: neither absence from the pending list nor a received history is enough (D-40). Japanese 反映済み (the draft's word in `error.proposal_not_open`). | Drafted (as above) |
| What a person writes **about a proposal** (RFC 005 part 6) | (none) | **comment**; a reply to a comment is a **reply** (a thread is one level deep) | Plain; "note" is kept for the line of text beside a decision. A comment cannot be edited or deleted; a maintainer can **hide** it, and anyone who administers the repository can show it again. | Drafted by the dev team (RFC 005 part 6 T5) — to be approved by the architect before merge, and flagged for the owner, who approved this page |
| What a reviewer says about a **revision** | (none) | a **decision**: **approve** or **request changes**; the proposal's **review** is one of *Awaiting review*, *Approved by N people*, *Changes requested by N people* | "Approved" here is only ever a reviewer's decision about one revision; it merges nothing, and it is never carried to a later revision ("Approved on an earlier revision"). It is a different thing from the badge **Approved, awaiting signature** on a patch, which is about work held on this server and is untouched (the owner is asked about that wording separately). `tests/language_rules.rs` (rules in `tests/i18n/en.rules`) keeps "approved" out of every sentence about a proposal that is not about a decision. | Drafted (as above) |
| A comment a maintainer **hid** | (none) | **hidden**: "Hidden by a maintainer: *reason*"; the text is shown only to people who administer the repository | The only moderation. It is recorded, with who, why and when, and it is reversible. | Drafted (as above) |
| The file a maintainer exports to hand over a whole repository (prikk: a `PREPO001` file, `archive export`) | (none) | **archive**: "From an archive a maintainer gave you" | prikk's own word, and plain: one file with all of a repository's history. RFC 004 part 7. | Drafted (architect to approve) |
| A key an archive lists as its maintainers' (prikk: a carried MAINTAINER key, "carried, not adopted") | "adopted key", "carried key" | **maintainer key**: "Maintainer keys in the archive" | Says whose key it is. The archive's list is its own claim and proves nothing; the page says so beside the fingerprints. | Drafted |
| What a creator compares to know a key is the maintainer's (prikk: the key's public key, 64 hexadecimal characters; `prikk key public`) | (none) | **fingerprint**: "Fingerprint to compare:", shown in groups of eight and never filled into a field | It is the one thing a maintainer can read off their own machine, so the creator can compare it through a channel the archive does not control. The API's `fingerprint` and `public_key` are the same text. | Drafted (see the review request: the handoff asked for a fingerprint distinct from the public key; prikk offers none) |
| To make prikk believe a key (prikk: `trust maintainer add`, the *adopt* of `--adopt`) | "adopt" | **trust a key**: "Do not trust this key", "Trusted keys: …", "Create the repository and trust the keys I typed" | *Adopt* is the implementer's word. The API keeps the field `adopt` (developer-facing). Trust is the creator's act: only the keys whose fingerprints they typed. | Drafted |
| A branch the repository holds but does not publish (prikk: a ref under `remotes/`, "received") | "received ref" | **received copy**: "kept as a received copy named remotes/heads/main" | It is in the repository, shown under *Details*, and counts for nothing until a key is trusted. | Drafted |
| The archive on this server after the upload and before the creator confirms (no prikk word) | (none) | not a noun on the pages ("the archive is checked and kept for one day"); the API says `stage` | An implementation word; the person sees what it means. | Drafted |
| A proposal that is **accepted and waiting**: its patches are in this server's pending list, nobody has signed them | "forge-approved-unsealed" (machine value only) | **held for review** — "Held for review: not yet part of the project's history." | Says what is true and no more: the server holds the work; nothing is approved for merging. Japanese レビュー待ちで保持 (the draft's `propose.message`). | Drafted (as above) |
| The block a person says their work was built on (RFC 005 D-3's *declared base*) | "base", "declared base" | **built on** | Plain; the API field stays `base` (developer-facing). Japanese 土台とした (to be reviewed). | Drafted (as above) |
| The smallest edit inside a change (prikk: *Patch*) | "patch" | **patch**, explained on the change page: "A patch is one recorded edit." | It is prikk's own word and users of a version-control forge know it | Owner's choice (alternative: "edit") |
| A change that undoes an earlier one (prikk: *rollback*) | "rollback" | **rollback**, explained on the change page: "A rollback is a change that undoes an earlier change." | Familiar word; the explanation was what was missing. The page cannot say "This change undoes an earlier change." because it does not know whether the change is a rollback, so the explanation is a definition. | Owner's choice |
| The **number of sessions** in prikk's verify report (JSON `active_sessions`: `count`, `verified_count`) | "{n} of {m} session(s) verified." | **Removed (IS-49).** Not shown anywhere: prikk says `count` is the number of session folders (always one, `default`) and `verified_count` a fixed 1, a scope statement and not a tally. It is **not** waiting work: the number of waiting commits is `status`'s `queue.count`, which is not used today. | A number that cannot be true must not be shown (the sentence next to a green badge suggested progress that does not exist). prikk's own advice: say nothing about sessions. | Confirmed (prikk's letter of 2026-10-21, ledger PK-62) |
| A verification **stage** and its status (`evaluated`, `failed`, `not_evaluated`, `halted`) | "Stage", "evaluated", "failed", "not_evaluated", "halted" | column **Check**; status **completed**, **problem found**, **not run (blocked by {check})**, **stopped early (after {check})** (the two check names come from the stage's `blocked_by` and `after` fields) | "evaluated" does **not** mean "passed" (a stage can complete and still add a problem); "completed" says only that it ran | Confirmed that `evaluated` ≠ passed; wording is the owner's choice |
| The **findings** of a failed check (prikk conditions such as `stage-failure`, `item-failure`, `publication-trust`) | "{id}: {message}" in prikk's English | prikk's id and message as the detail, under a plain heading **What was found** | The text is prikk's and cannot be translated by us; the heading can | Confirmed |
| `SSO` | "Sign in with SSO" | **Sign in with single sign-on** | The abbreviation is not universal | Owner's choice |
| `User` field label | "User" | **Username** | It is the account name | Owner's choice |
| Raw file download address (`content origin`) | operator hints only | no user message names it; operator hints keep "content origin" | The user never needs the term | Owner's choice |
| `read memory budget` | operator hint | the operator hint keeps it (`error.too_large.hint`, the API); the page says "This is too large for this server to show." | Operator vocabulary stays with the operator | Confirmed |
| A file or folder that is **not in a repository the reader may read** | "There is no repository at this address, or you do not have access to it." (the repository-level sentence, for everyone) | to a reader of the repository: "This file or folder was not found in this repository. Check the path, or go back to the files." with a link to the files; the repository-level sentence stays for a repository that is missing, hidden or unreadable | Existence-neutrality protects the **repository**; once the reader may read it, a missing path is not a secret (IS-45). The API answers both exactly alike. | Architect's decision (IS-45) |
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
| `title.too_large` | "Too large · {path} · {product}" | "Too large to show here · {path} · {product}" | Approved |
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
| `no_such_path.body` | — | "This file or folder was not found in this repository. Check the path, or go back to the files." | Derived |
| `no_such_path.link` | — | "Go to the files" | Derived |
| `error.too_large.raw_hint` | "This server does not serve files that large; ask the operator." | "This server does not serve files that large; ask the person who runs this server." | Approved |
| `repo.location` | "at {point}" | "Showing {point}" | Approved |
| `repo.location_branch` | "at {point} · branch {branch}" | "Showing {point} · branch {branch}" | Approved |
| `tree.at` | "at {point}" | "Showing {point}" | Approved |
| `file.binary` | "Binary content — {download}." | "This file is not text, so it cannot be shown here. {download}." | Approved |
| `history.empty` | "No sealed history on this ref yet." | "There are no signed changes on this branch yet." | Approved |
| `page.csrf.body` | "The form could not be checked, so nothing was done." | "The form could not be confirmed as yours, so nothing was done. Go back, reload the page and try again." | Approved |
| `page.form_incomplete.body` | "Some required fields were missing, so nothing was done." | "Some fields were empty, so nothing was done. Go back and fill them in." | Approved |
| `page.sso_off.body` | "This server has no single sign-on provider configured." | "This server has no single sign-on provider configured. Try signing in with your username instead." | Approved |
| `page.sso_down.body` | "The sign-in provider could not be reached." | "The sign-in provider could not be reached. Try signing in with your username instead." | Approved |
| `page.sso_incomplete.body` | "The sign-in provider's reply was incomplete." | "The sign-in provider's reply was incomplete. Try signing in with your username instead." | Approved |
| `page.sso_mismatch.body` | "The sign-in could not be matched to the request that started it." | "The sign-in could not be matched to the request that started it. Try signing in with your username instead." | Approved |
| `page.sso_expired.body` | "The sign-in took too long and has expired." | "The sign-in took too long and has expired. Try signing in with your username instead." | Approved |
| `page.sso_refused.body` | "The sign-in provider did not accept the sign-in." | "The sign-in provider did not accept the sign-in. Try signing in with your username instead." | Approved |
| `page.sso_unlinked.body` | "This single sign-on identity is not linked to a planeter account." | "This single sign-on identity is not linked to a planeter account. Ask the person who runs this server." | Approved |
| `page.server_error.body` | "The server could not complete the sign-in." | "Something went wrong on the server, and you were not signed in. Try again in a little while." | Approved |

### Badges and their explanations

| Key | Was | Current | Status |
|---|---|---|---|
| `assurance.verified` | "prikk-verified" | "Verified" | Approved |
| `assurance.sealed` | — | "Signed into the history" | Approved |
| `assurance.queued` | "Approved, awaiting signature" (until 2026-10-29: reworded, D-50) | "Held here, not signed in" | Approved |
| `assurance.unverified` | "unverified" | "Verification failed" | Approved |
| `assurance.verify_unavailable` | "verify unavailable" | "Could not be checked now" | Approved |
| `explain.summary` | — | "What does this mean?" | Approved |
| `explain.queued` | "This change was approved on this server. A maintainer has not yet signed it into the project's history, so it is not part of the verified record." (until 2026-10-29: reworded, D-50) | "This patch is held on this server. A maintainer has not signed it into the project's history, so it is not part of the verified record." | Approved |
| `explain.unavailable` | — | "Try again in a little while." | Approved |
| `explain.verified` | — | "prikk ran all of its checks on this repository just now, and they found no problems." | Approved |
| `explain.signed` | — | "A maintainer signed this patch into the project's history. The Verification page shows whether the whole repository checks out." | Approved |
| `explain.failed` | — | "prikk checked this repository and found problems. The Verification page lists what was found." | Approved |
| `explain.failed_here` | — | "prikk checked this repository and found problems. They are listed below." | Approved |
| `explain.patch` | — | "A patch is one recorded edit." | Approved |
| `explain.rollback` | — | "A rollback is a change that undoes an earlier change." | Approved |

### The verification page

| Key | Was | Current | Status |
|---|---|---|---|
| `verify.intro` | "Re-derived from {command} on this request — never served from a cache." | "These checks were run just now, when you opened this page. They are never reused from an earlier visit." | Approved |
| `verify.findings` | — | "What was found" | Approved |
| `verify.col.stage` | "Stage" | "Check" | Approved |
| `verify.status.evaluated` | "evaluated" | "completed" | Approved |
| `verify.status.failed` | "failed" | "problem found" | Approved |
| `verify.status.not_evaluated` | "not_evaluated" | "not run (blocked by {check})" | Approved |
| `verify.status.not_run` | — | "not run" | Approved |
| `verify.status.halted` | "halted" | "stopped early" | Approved |
| `verify.status.halted_after` | — | "stopped early (after {check})" | Approved |

**Removed messages.** `verify.sessions` ("{verified} of {count} session(s) verified.") was held for prikk's confirmation and is **removed** (IS-49): the verification page and the verify API no longer carry it or the fields behind it (`verified_sessions`, `total_sessions`).

## 3. Open

1. ~~The owner's reading of sections 1 and 2~~ — done, approved (2026-10-21).
2. ~~Confirm with prikk what `active_sessions` means~~ — answered (PK-62): it is not waiting work; the message is removed (IS-49).
3. ~~Who reads the Japanese (RFC 012 O-2)~~ — **the owner and a colleague** (named by the owner, 2026-10-21); Japanese is [`glossary/ja.md`](glossary/ja.md) (signed off 2026-10-31) and the generated review sheet.

## 4. How this is used

The catalog follows this page. A change to a user-facing message is a change to this page first (then the
catalog, in the same commit); `tests/glossary.rs` fails if a *Current* cell and the catalog disagree, and
`tests/language_rules.rs` (rules in `tests/i18n/en.rules`) fails if a retired word (`unsealed`, `forge-approved`, `prikk-verified`, `not_evaluated`,
`evaluated`, `refs`, `SSO`, or the old navigation labels) comes back. The Japanese translation starts from this
English.

## 5. Other languages

Each language other than English keeps its terms, its message table and its wording rules in its own file, `docs/glossary/<tag>.md` (Japanese: [`glossary/ja.md`](glossary/ja.md)), with the same three parts: **Status** (draft, or signed off — by whom and when), **Terms** (the term the interface uses for each concept above, which are hard terms the reviewers decide, and the alternatives), and **Messages that carry a term or a trust label** (the catalog's text, character for character; `tests/glossary_lang.rs` compares it). The English meaning of every trust label is fixed; a translation keeps it exactly. Adding a language starts with its file: `docs/messages-and-languages.md`, "Adding a language".

## 6. Words for the person who runs the server

The messages of the recovery plan and of the proposal errors are read by whoever runs the server, and they use a few words that are
not everyday ones. They are defined here, in plain words, once. (English only; the Japanese rows are for the reviewers to decide.)
`tests/glossary_operator.rs` holds this section to the catalog: every one of these words that an operator sentence uses is defined here.

| Word | In plain words | Where you meet it |
|---|---|---|
| **flag** | The mark a repository carries while it waits for recovery. While it is set, the repository takes no new proposals; reading is not affected. | the recovery plan, `needs_recovery` |
| **lease** | The right to write to a repository right now. One thing holds it at a time (an accept, an export, a recovery); the others wait, or are told to try again. | the recovery plan |
| **accept** | The server taking in a proposal: it hands the artifact to prikk, which stores its patches in the repository. | the proposal errors, the recovery plan |
| **intent** | The server's note, written before it starts, that it began taking a proposal in. It lets a proposal that was cut off be finished later from the same starting point. | the recovery plan, the API's field names |
| **lock** | A file of prikk's own, in the repository's hidden directory, that stops two writers at once. A write that was killed leaves its locks behind; recovery clears them only when it is sure nothing is using the repository. | the recovery plan |
| **recovery** | The steps that make a flagged repository writable again: check that nothing is using it, clear the stale locks, check the repository, and clear the flag. Every step is recorded. | the recovery plan and its door |
