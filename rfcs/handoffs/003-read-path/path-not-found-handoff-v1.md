# RFC 003 follow-on — A missing path is said to be missing; the queued-only repository is pinned — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **follow-on** of the done RFC 003 (the read path) and of RFC 012 D-4 (plain words), from two observations in the prikk 0.53.0 re-baseline review request (T3): (1) a missing **file** in a repository the reader can see shows "There is no repository at this address, or you do not have access to it." — the existence-neutral sentence for a repository, wrong for a path; (2) the behaviour of a repository whose only content is queued work is unpinned by any test. Issue **IS-45**. |
| Status | **Done, reviewed Approved (2026-10-21; commits `f6d36ce`, `3c39e5d`).** IS-45 closed. |
| Basis | The re-baseline review request (private), T3 tables and its two "existing oddities"; `crates/planeter-core/src/read.rs` (`ReadError`, `file()` with its `tree` pre-check, `raw_file` and `is_missing_path`), `crates/planeter-web/src/{api,ui}.rs` (error mapping, `not_found` page), `catalog/en.rs`, `docs/glossary.md`; the existence-neutrality rule (a stranger must not learn whether a private repository exists). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## The principle

Existence-neutrality protects **the repository**: whoever may not read it gets the same answer whether it exists or
not. Once the reader is **authorized** to read the repository, a missing *path* inside it is not a secret and should
be said plainly. The authorization check always runs first.

## Change scope

- **T1 — A distinct error after authorization.** Add `ReadError::NoSuchPath` in `planeter-core`, produced only
  **after** `authorize_read` succeeded: by `file()` (empty `tree --prefix` pre-check), by `raw_file` (prikk's own
  "path … does not exist", exit 1 — the existing `is_missing_path` test, unchanged), and by `tree` for a prefix that
  has no entries when the point is a sealed one **and prikk can tell** (measure first: does an empty listing for a
  non-root prefix always mean "no such directory" in prikk, which stores files not empty directories? Report the
  evidence; if it can ever mean "an existing, empty directory", do not map `tree` and say so). A repository that is
  unreadable, absent or hidden stays `NotFound`, byte-identical as today.
- **T2 — The API stays neutral in shape.** `NoSuchPath` maps to the **same** `404 not_found` body as before
  (`code`, `message`, `hint`): a client cannot tell them apart and nothing is learned by a stranger. Test: for a hidden
  repository, an absent repository, and a readable repository with a missing path, the API responses are identical
  (status, headers, body) — and for the authorized reader, a missing path on a readable repository is also
  identical to them. (The UI is where the difference is shown, and only to readers.)
- **T3 — The UI says it.** For `NoSuchPath` the file and folder pages render a page (404) titled "Not found"-pattern
  with the sentence **"This file or folder was not found in this repository. Check the path, or go back to the
  files."** and a link to the repository's file list (catalog keys, plain words, one sentence plus one link); the
  repository-level `not_found.body` is unchanged and is used only for `NotFound`. Titles keep the distinct-state rule
  (`Not found · /alice/app/src/nope · planeter` is already unique per path). Add the new keys to the glossary page
  (Status: Derived) and let the glossary/catalog agreement test cover them.
- **T4 — The queued-only repository is pinned.** A regression test (real prikk; self-skips with a reason without it)
  for a repository whose only content is queued, unsealed commits, as built in the re-baseline's `queued_only.py`:
  history is empty with the "no signed changes" sentence; tree and refs answer 200 with empty content; `file` and
  raw give `NoSuchPath`/`not_found` and **never reach `checkout --patch-plan`** (assert that with a recording
  stand-in or by the call log); the mixed repository (sealed + queued) shows the sealed content only. This documents
  today's behaviour; it does not change it.
- **T5 — Record the other oddity, do not fix it.** The empty-but-"Verified" repository page for queued-only work is
  **not** changed here (the architect decides it with the queued-state design: `branch --format json` has
  `unpublished_current_branch`, a data field). Add one line to `docs/accessibility.md`'s or the glossary's "known
  gaps" so it is not lost.

## Non-change scope

No change to who may read what; no change to the API's codes, shapes or statuses; no new error code in the API; no
prikk-text matching beyond the existing `is_missing_path`; no wording change outside the new sentence; no new
dependency; no `unsafe`; no version or release change.

## Required documentation updates

`docs/glossary.md` (the new rows), CHANGELOG `[Unreleased]` (operator-visible: a missing file in a readable
repository is now said to be missing, not "no repository at this address"), STATUS IS-45 closed on review.

## Acceptance criteria

T2's identity tests pass (hidden = absent = readable-but-missing, in the API) and fail under a mutation that makes
`NoSuchPath` distinguishable; the UI shows the new sentence only to a reader who may read the repository and never to
an anonymous visitor of a private one (test both); T4's test passes on prikk 0.53.0; the structure checker, the
retired-words, glossary, stylesheet, catalog and OpenAPI drift checks stay green; every gate green including both
supply-chain gates (fresh advisory database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No `NoSuchPath` before `authorize_read`; no difference in status, headers or body between hidden, absent and
path-missing in the **API**; no matching new prikk text; no change to the repository-level sentence.

## Required evidence and review-request format

Commits (hash → what); the T1 evidence for `tree`; the identity-test output; before/after of the file page for a
missing path (reader) and the same URL as a stranger of a private repository; the T4 test; the mutation table;
every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
