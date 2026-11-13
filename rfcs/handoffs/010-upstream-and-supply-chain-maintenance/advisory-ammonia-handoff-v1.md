# RFC 010 — Advisory RUSTSEC-2026-0533 (`ammonia` 4.2.4, mutation XSS) — Implementation Handoff (v1)

| | |
|---|---|
| Document | The advisory database gained **RUSTSEC-2026-0533** on 2026-11-12 (after your last clone): `ammonia` before 4.2.5 handles MathML text integration points and SVG `foreignObject` wrongly, so a sanitized document can parse differently the second time (mutation XSS). The advisory's own condition is that the application allows a raw-text tag (`title`, `textarea`, `xmp`, `iframe`, `noembed`, `noframes`, `plaintext`, `noscript`, `style`, `script`) together with `math`/`svg`; planeter's sanitizer (`crates/planeter-web/src/sanitize.rs`) is expected not to, but the policy is to **update, not to argue**. `ammonia` is the sanitizer for every README, description and comment the server shows. |
| Status | Assigned 2026-11-14. |
| Basis | `docs/dependency-policy.md` rules 1 and 3; `crates/planeter-web/src/sanitize.rs`; `crates/planeter-web/tests/sanitize_advisories.rs` (the earlier advisory shapes); the advisory text in a fresh clone of the advisory database (`crates/ammonia/RUSTSEC-2026-0533.md`). |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **Fresh advisory-db clone first** (the previous one was stale); report its HEAD.
2. **Measure (rule 1):** `cargo update -p ammonia --precise 4.2.5` (or the newest 4.2.x that the advisory marks patched; say which). Report "N → M crates (+K); deny: …; audit: …; new licenses: …". Only `Cargo.lock` should change; if the manifest or another crate must move, stop and report.
3. **Regression test**, in `tests/sanitize_advisories.rs` beside the earlier ones: the advisory's shape (`<math><mi><svg>…</svg></mi></math>` with a raw-text gadget such as `<title>`/`<style>`/`<textarea>` inside), through `sanitize` and through the Markdown path, asserting the output is inert (the file's existing `assert_inert`) **and** that sanitizing the output a second time gives the same text (a mutation check: the output is a fixed point). Show it **failing on 4.2.4** (once, with the lock temporarily at 4.2.4) and passing on the patched version. If it passes on 4.2.4, say so plainly in the report: it means planeter was not exposed, and the test stays as the guard.
4. State, from the code, which tags and attributes the sanitizer enables and whether any of the raw-text tags above is among them. This is evidence, not a reason to skip tasks 2 and 3.
5. `docs/dependency-policy.md` triage-record row and one `CHANGELOG.md` `[Unreleased]` line.
6. Gates on the exact commit with the fresh database: fmt, floor, pin, clippy, both test passes (with prikk **0.55.0** first on PATH, privately installed, and without prikk), ENF-2, doc, `cargo audit`, and `cargo deny` (expected: **exit 0 for advisories**; while the Zlib decision of the `rusqlite` branch is open, `cargo deny` on a base that has not got it will still pass licences, because this change adds none).

## Non-change scope
No `ignore`. No other dependency. No change to the sanitizer's configuration unless the test fails on the patched version (then stop and report). Base the work on `main`; the unpushed `advisory-rusqlite` branch is separate and will be rebased by you afterwards, resolving `Cargo.lock` with `cargo update`, not by hand.

## Acceptance criteria
`cargo audit` exit 0 for this advisory; the lock shows `ammonia` ≥ 4.2.5; the test exists, is shown failing on 4.2.4 (or the plain statement above); the tag list of task 4; gates green; CI's `supply-chain` job no longer reports `ammonia`.

## Required evidence and review-request format
Commits (hash → what); the measurement line; the test and its failing-once evidence; the tag list; every gate with command, exit code, prikk version and advisory-database HEAD; deviations and open questions.
