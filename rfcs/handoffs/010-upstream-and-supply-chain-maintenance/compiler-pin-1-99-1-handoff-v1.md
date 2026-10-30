# RFC 010 — The compiler pin 1.99.0 → 1.99.1 before the 0.5.0 tag — Implementation Handoff (v1)

| | |
|---|---|
| Document | The release candidate's pins review found one pin that matters: the single compiler of CI and release is **1.99.0**; **1.99.1** (2026-10-16) fixes a **miscompilation of FFI `bool` return values on x86_64**, and the release binaries link C (aws-lc, SQLite, libc). Nobody has shown that the tree is affected (the C APIs in use return `int`), but a release built with a compiler that has a known, fixed miscompilation, when a patch release exists and the update is one reviewed pin, is not what *robust* and *safe* mean. Decision (D-55): update, rehearse, then the candidate is final. BuildKit stays at v0.33.1 (no advisory affects it; v0.34.1 is 10 days old; a builder change is its own rehearsed commit later, noted in STATUS). |
| Status | **Done, reviewed Approved (2026-10-31; commit `68be90a`, the final 0.5.0 candidate).** (Originally: Assigned 2026-10-30; blocked the tag.) |
| Basis | `release-0-5-0-candidate-review-request.md` §5 and §8.1; RFC 010 D-5 (one compiler, pinned; `pin-check` enforces it), `docs/RELEASING.md` step 5; `.github/workflows/{ci,release,supply-chain}.yml` (`rustup toolchain install 1.99.0 …`), `docs/dependency-policy.md` line on the compiler, `docs/STATUS.md` IS-26 note; the 1.99.1 release notes. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The pin.** Every place that names `1.99.0` becomes `1.99.1` (the `rustup toolchain install …` and `rustup default …` lines in `ci.yml`, `release.yml`, `supply-chain.yml`; the documentation sentences; the tests of `pin-check` that name the version, if any) — grep for `1.99.0` across the repository, list every hit and its decision (history rows in STATUS/SCHEDULE/dependency-policy stay history). `rust-version` (MSRV) **stays 1.88.0** and the `msrv` job stays on 1.88.0. `Cargo.lock` must not change; if the new compiler changes any lint outcome (clippy), fix the lint the minimal way and report each.
- **T2 — Reproducibility note.** The release binaries are built by this compiler: record in the CHANGELOG `[0.5.0]` "Under the hood" the one fact (the release is built with Rust 1.99.1) and in `docs/RELEASING.md` the pin's review date; no other CHANGELOG edit.
- **T3 — Evidence on the commit.** All gates (fmt, floor, pin, clippy `-D warnings`, both test runs named, deny and audit **0**, ENF-2 both binaries, doc, the `msrv` job, CI green on all three systems), **and a rehearsal** of the release workflow on the resulting commit, with the run link and per-job results (both architectures: build, image run, glibc check, push, identity, manifest list, attestation). Report the binary sizes against 0.5.0's candidate (`404614e`) for the record.

## Non-change scope

No other pin; no BuildKit change; no dependency change; no feature; no tag, release or upload.

## Acceptance criteria

T1–T3 as written; the result is **the final 0.5.0 candidate commit** — the architect reviews it and then asks the owner for the tag.

## Prohibited shortcuts

No `@stable`, no unpinned toolchain; no change of `rust-version`; no lock change; no skipping the rehearsal.

## Required evidence and review-request format

Commits (hash → what); the `1.99.0` grep with decisions; every gate with command and exit code and the advisory-database HEAD; the rehearsal run link and per-job results; sizes; deviations and open questions.
