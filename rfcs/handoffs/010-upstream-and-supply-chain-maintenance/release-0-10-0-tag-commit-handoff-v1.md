# RFC 010 — Release 0.10.0: the wording of the record, and the tag commit — Implementation Handoff (v1)

| | |
|---|---|
| Document | The candidate **`d33ac8d`** is approved (SCHEDULE D-152) and **the owner authorized the 0.10.0 tag** (2026-11-28). Two small commits remain, as for 0.9.0: **A** puts one sentence of the Japanese record in order; **B** is the docs-only tag commit. It changes no catalog text and no code. |
| Status | Assigned 2026-11-28 (SCHEDULE D-152). |
| Basis | `release-0-9-0-signoff-and-tag-handoff-v1.md` (the model: commit B and its gates); `release-0-10-0-japanese-review-request.md` ("Deviations": the status line's grammar); `docs/RELEASING.md` (the 0.10.0 row); `CHANGELOG.md` `[0.10.0]`. |
| Audience | Dev team. Return a short review-request package. |

## Two commits, in this order
### Commit A — the record's sentence
1. **The status line reads "…of 2026-11-28 on 2026-11-28"** because the tool joins the reviewers' string and the date with " on ". Change the `reviewers` string of the `SignOff` of `ja` (`crates/planeter-i18n/src/languages/ja.rs`) to **"the architect of the project (an AI assistant, Claude), reading on the owner's delegation"**, so the line reads *"signed off by the architect of the project (an AI assistant, Claude), reading on the owner's delegation on 2026-11-28"*. The delegation's date stays in the `note` (it is already there). `date`, `commit` (`860f3ac…`) and `sheet_fingerprint` (`7193ee88069104ca`) **do not change**, and neither does any catalog text or baseline: show that `planeter-i18n-sheet --baseline ja web` and `client` give the bytes already committed.
2. **Every place that quotes the status line** follows, character for character as the tool prints it now: `CHANGELOG.md` `[0.10.0]`, `docs/messages-and-languages.md`, `docs/glossary/ja.md` (Status), `docs/RELEASING.md`'s row, and any test that holds it (find them with `grep -rn "delegation of 2026-11-28"`; **do not weaken a test: change the expected text to the new one**). The regenerated sheet's header shows the same line; report its SHA-256 and fingerprint (the fingerprint must be `7193ee88069104ca`).
3. `git diff` of A is those files and nothing else.

### Commit B — the tag commit (docs only)
1. `CHANGELOG.md`: `## [0.10.0] — <the date of this commit, YYYY-MM-DD> — accounts you can make and read, and a sign-in provider that can be the only way in` (no "unreleased until tagged"; the rest of the heading unchanged); show that `release.yml`'s awk finds the section (the 0.9.0 report's way). *If the tag is made on a later day, the architect makes a docs-only commit that changes the date: say the date you used.*
2. `README.md`: the status line says 0.10.0 without "(release candidate)". 3. `ROADMAP.md`: the 0.10.0 row says **Released <date> (owner-authorized; SCHEDULE D-121…D-152)** in the style of the 0.9.0 row, its last cell "**released <date>**; content in `CHANGELOG.md` and `docs/accounts.md`". 4. `docs/RELEASING.md`: the 0.10.0 row headed "**0.10.0 (<date>).**", with what is already known filled (the candidates `3454e16` and `d33ac8d`, reviewed Approved D-121/D-149 and D-152, the seventh record `<commit A>`, the tag-commit placeholder; the rehearsals as already written: none, the chain did not change since `7e23768`); **the tag-time blanks stay `<…>` for the architect**.
Nothing else: `git diff` of B names exactly these four files.

## Gates on the final commit (commit B)
`tools/gates.sh --tier release --commit <B> --sheet <the sheet generated from B's catalogs>` at the default cap, one run, from its snapshot (the whole table, as in your last report); CI green on all eight jobs on B (if `client (macos-15)` is cancelled, re-run that job and say so); `git diff 7e23768..HEAD -- .github/workflows/release.yml Dockerfile .github/workflows/supply-chain.yml` empty (show it). No stress run (show that no client source changed since `d33ac8d`: `git diff d33ac8d..HEAD -- crates/planeter-client` empty).

## Non-change scope
No catalog text, no code, no pin, no version change, no rehearsal; no edit of `docs/STATUS.md`, `docs/SCHEDULE.md`, the ledgers, the threat model or any security review (mine: `docs/security-review-0.10.0.md` and the threat model v0.10 are written); **no tag, no GitHub release, no upload, no `cargo publish` other than `--dry-run`**.

## Acceptance criteria
`--status` reads "signed off by the architect of the project (an AI assistant, Claude), reading on the owner's delegation on 2026-11-28; 0 message(s) newer than the sign-off …"; the sheet's fingerprint is `7193ee88069104ca`; commit A's and B's diffs are the files named; every gate green; **state the final commit hash: it is the one the architect will tag.**

## Required evidence
Commits (hash → what); `git diff --stat d33ac8d..<B>`; the status line before and after; the baselines' byte-identity; the CHANGELOG heading and the awk output; the whole release-tier table with tool versions, the advisory-database HEAD and count; deviations and open questions. Final reply: the review-request path only.
