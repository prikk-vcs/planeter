# RFC 010 follow-on — Advisory RUSTSEC-2026-0373 (ammonia): lock-only update — Implementation Handoff (v1)

| | |
|---|---|
| Document | An instance of RFC 010 D-3/D-4 (the supply-chain gate at a project event; the triage ladder). The tag run of release 0.3.0 failed its `cargo-deny` step on a **new advisory published 2026-10-16**; nothing was published. This handoff fixes the lock. Issue **IS-38**. |
| Status | **Assigned** (2026-10-18). The release decision that follows (re-tag or 0.3.1) is the owner's; this handoff does not depend on it and must not touch tags, versions or the release workflow. |
| Basis | RFC 010 D-3, D-4; `docs/dependency-policy.md`; the failed run 39067378782 (`cargo deny check` → advisories FAILED); RUSTSEC-2026-0373 "Missing case-insensitive matching for CSS url function" — affects `ammonia` 4.2.3, fixed in ≥ 4.2.4 (and ≥ 4.1.8, < 4.2.0); `crates/planeter-web/src/sanitize.rs`, `crates/planeter-web/tests/sanitize_advisories.rs`. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Triage (the architect's, to be confirmed by you)

planeter calls `ammonia::clean(html)` with the **default** configuration (`sanitize.rs`). The advisory's
trigger needs a builder that allows the `style` attribute with `filter_style_properties`; ours does not
(`grep` finds neither). So the defect is **not reachable** here. The ladder's first rung still applies: a
lock-only update within the manifest's range (`ammonia = "4"`) exists, and a red gate is fixed first.

## Change scope

- **T1 — Reachability, shown.** Confirm in the report with the exact grep and the default-configuration
  fact (and that no other crate in the tree builds an `ammonia::Builder`). Add one test beside the existing
  advisory regressions: with the default configuration, `URL("https://…")` inside a `style` attribute and
  inside a `<style>` element is stripped from the output.
- **T2 — Lock-only update.** `cargo update -p ammonia --precise 4.2.4` (or the newest allowed that the
  advisory names as fixed). **Report the complete `Cargo.lock` diff.** If any crate other than `ammonia`
  changes, list each with old/new versions and why; if more than ammonia's own dependencies move, stop and
  hand back. No manifest change.
- **T3 — Gates, with a fresh advisory database.** `cargo deny check` and `cargo audit` both exit 0 after
  refreshing the advisory database (state how); then the full set: fmt, floor-check, pin-check, clippy
  `-D warnings`, test with prikk on `PATH` and genuinely without, ENF-2, doc. The existing sanitizer tests
  must pass unchanged.
- **T4 — Records.** CHANGELOG under `## [Unreleased]` (one entry: the advisory, that it was not reachable,
  the lock update); STATUS IS-38 closed on review. Do not edit any `[0.3.0]` section, the workspace version,
  the release workflow or any tag.

## Non-change scope

No version bump; no tag action; no workflow change; no code change other than the new test; no dependency
other than the one named.

## Acceptance criteria

Lock diff limited as T2 says; `cargo deny check` and `cargo audit` exit 0 on a fresh database; the new test
fails on 4.2.3 (show it, using a temporary revert) and passes on 4.2.4; every gate green; CI green on the
commit.

## Prohibited shortcuts

Do not add the advisory to an ignore list; do not pin around the issue; do not touch the `0.3.0` tag or
version; do not skip refreshing the advisory database.

## Required evidence and review-request format

Commits (hash → what); the lock diff; every gate with command and exit code; the red-on-4.2.3 evidence; the
CI run id; deviations and open questions.
