# RFC 010 follow-on — Advisory RUSTSEC-2026-0374 (basic-toml): trim `askama`'s features — Implementation Handoff (v2)

| | |
|---|---|
| Document | Supersedes [`advisory-basic-toml-handoff-v1.md`](advisory-basic-toml-handoff-v1.md), whose T1 evidence the dev team delivered and which found that the architect's triage was incomplete: in every `askama_derive` version `basic-toml` is an **optional** dependency behind the `config` feature, which `askama`'s default features enable and `planeter-web` does not use (no `askama.toml`). RFC 010 D-4 puts a manifest change (rung 2) **before** an ignore (rung 3). Decision (architect): **option B**. Issue **IS-42**. |
| Status | **Assigned** (2026-10-23). `main` stays red until this lands. |
| Basis | RFC 010 D-4; the dev team's stop report (`advisory-basic-toml-review-request.md`, T1(a–d), `finding-askama-without-config.txt`, `option-B-trim-askama-features.patch`); the architect's re-verification on 2026-10-23: the patch applies to `HEAD`, `cargo deny check` reports `advisories ok`, `cargo audit` (fresh database, 1,345 advisories) no vulnerability and no warning, `basic-toml` gone from the lock (213 → 212 crates), `cargo test -p planeter-web` 154 passed. |
| Audience | Dev team. Return a short review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — Apply option B.** `askama = { version = "0.13", default-features = false, features = ["derive", "std"] }`
  in `crates/planeter-web/Cargo.toml` (apply the dev team's patch or equivalent), `Cargo.lock` as `--locked`
  requires (expected: `basic-toml` removed and the `serde`/`serde_derive` edges only it pulled in). **Report the
  complete lock diff**; if anything other than those edges changes, stop. State whether any template feature
  (filters, `urlencode`, `serde_json`, whitespace control) that the default features used is lost and show that
  nothing in the templates or catalog uses it (the compiler and the whole test suite are the proof; name them).
- **T2 — Keep the gate honest.** A comment on the dependency line saying why the features are trimmed, that
  re-enabling `config` brings RUSTSEC-2026-0374 back, and the revisit condition ("re-enable `config` only if an
  `askama.toml` is needed and the advisory is resolved"). Add a one-row mention to `docs/dependency-policy.md`
  (the dependency, the trim, the date and the evidence), RELEASING step 1 needs no change, STATUS IS-42 closed on
  review, CHANGELOG only if an operator-visible fact changes (it should not).
- **T3 — Gates.** Fresh advisory database: `cargo deny check` exit 0 with no advisory error; `cargo audit` exit 0
  with no `basic-toml` warning; the full set (fmt, floor-check, pin-check, clippy `-D warnings`, test with and
  without prikk, ENF-2, doc); the release binary builds and `planeter --version` runs; CI `supply-chain` green on
  the commit. No `deny.toml` or `audit.toml` change (no ignore).

## Non-change scope

No ignore; no version change of `askama`; no template or catalog change; no other dependency change; no
`unsafe`; no release or version change.

## Acceptance criteria

The lock diff is limited as T1 says; `cargo deny check` and `cargo audit` exit 0 on a fresh database with no
`basic-toml` finding; CI green on the commit; the whole test suite passes unchanged.

## Prohibited shortcuts

No scoped ignore as well as the trim; no enabling extra features to "make something compile" (report instead); no
stale advisory database.

## Required evidence and review-request format

Commit (hash → what); the lock diff; every gate with command and exit code and the advisory-database HEAD; the CI
run id; deviations and open questions.
