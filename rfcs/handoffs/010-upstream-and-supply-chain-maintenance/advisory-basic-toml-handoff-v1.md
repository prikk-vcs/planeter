# RFC 010 follow-on — Advisory RUSTSEC-2026-0374 (basic-toml unmaintained): triage and a scoped ignore — Implementation Handoff (v1)

| | |
|---|---|
| Document | An instance of RFC 010 D-3/D-4 (the supply-chain gate at a project event; the triage ladder). The `supply-chain` job of CI has been red on `main` since commit `67dab7d`: `cargo deny check` fails on **RUSTSEC-2026-0374** ("`basic-toml` is unmaintained", advisory dated 2026-09-26, in the database after 2026-10-17). `cargo audit` reports it as an allowed warning. Issue **IS-42**. |
| Status | **SUPERSEDED 2026-10-23 by [`advisory-basic-toml-handoff-v2.md`](advisory-basic-toml-handoff-v2.md)** (the dev team's T1 found a rung-2 fix; the scoped ignore is not used). T1's evidence was delivered and accepted. |
| Basis | RFC 010 D-4 (ladder: lock-only update → manifest change after measuring → a scoped ignore with reachability shown), `docs/dependency-policy.md` rule 3 (an ignore needs a reason and a revisit condition), `deny.toml` and `.cargo/audit.toml` (the existing RUSTSEC-2023-0071 entry is the pattern); the dev team's reports in the IS-39 and message-catalog review requests. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Triage (architect's, to be confirmed by your evidence)

The dependency path is `basic-toml 0.1.10` ← `askama_derive 0.13.1` (**proc-macro**) ← `askama 0.13.1` ←
`planeter-web`. `cargo tree --locked -i basic-toml -e normal` shows the same path, i.e. `basic-toml` is a build-time
dependency of the template macro, not code planeter links. Rung 1 (lock-only): no newer `askama` 0.13.x exists.
Rung 2 (manifest change): the architect checked crates.io dependency lists for `askama_derive` 0.14.0, 0.15.0 and
0.16.1 and **each still lists `basic-toml`**, so an `askama` upgrade does not remove it. That leaves rung 3.

## Change scope

- **T1 — Show it, with evidence.** (a) `cargo tree --locked -i basic-toml` (all edges, normal/build/dev) and its
  output; (b) that the released binary does not contain `basic-toml`: build `planeter` release and list symbols
  / strings for `basic_toml` (`nm -C`, `strings`) and report 0 hits (or what you find); (c) where `askama_derive`
  uses it (it parses an optional `askama.toml` configuration at compile time; confirm the repository has no
  `askama.toml`); (d) re-check that the three newer `askama_derive` versions still depend on `basic-toml` (query
  crates.io, record the commands and the date) and whether any other released templating crate in use is
  affected. **If any fact contradicts the triage, stop and hand back.**
- **T2 — The scoped ignore.** Add `RUSTSEC-2026-0374` to `deny.toml`'s `[advisories].ignore` with a `reason`, and
  to `.cargo/audit.toml`, each with a comment in the style of the existing entry that states: the advisory title,
  that it is *unmaintained* (informational, no vulnerability), the dependency path, that it is a compile-time
  proc-macro dependency absent from the shipped binary (T1b), the **revisit condition** ("drop the ignore the day
  `askama` releases a version that no longer depends on `basic-toml`, or the advisory is withdrawn; re-check at
  every release candidate per RELEASING step 1"). Keep `deny.toml` and `audit.toml` in step.
- **T3 — Record it.** `docs/dependency-policy.md` ledger/ignore list (a row with the evidence date), RELEASING
  step 1 mentions the ignore list as part of the pre-tag review, STATUS IS-42 closed on review, CHANGELOG only if an
  operator-visible fact changes (it should not).
- **T4 — Gates.** With a freshly cloned advisory database: `cargo deny check` and `cargo audit` exit 0 (audit may
  keep an allowed-warning line); then the full set (fmt, floor-check, pin-check, clippy `-D warnings`, test with and
  without prikk, ENF-2, doc); CI `supply-chain` green on the commit.

## Non-change scope

No dependency upgrade or downgrade; no `askama` change; no other ignore; no code change; no version or release
change.

## Acceptance criteria

T1's evidence shows the path and the absence from the binary; the ignore entries carry reason and revisit
condition in both files; `cargo deny check` and `cargo audit` exit 0 on a fresh database; CI green.

## Prohibited shortcuts

No ignore without T1's evidence; no blanket `unmaintained = "allow"` or similar policy change; no ignore of any
other advisory; no stale database.

## Required evidence and review-request format

Commits (hash → what); T1's four outputs; the two diffs; every gate with command and exit code and the
advisory-database HEAD; the CI run id; deviations and open questions.
