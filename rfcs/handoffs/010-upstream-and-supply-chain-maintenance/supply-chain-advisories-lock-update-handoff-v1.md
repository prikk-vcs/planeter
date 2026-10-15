# Interim — supply-chain advisories: lock-file update — Implementation Handoff (v1)

| | |
|---|---|
| Document | A small handoff to turn `cargo deny` and `cargo audit` green again: two `ammonia` advisories and one yanked crate, all fixable by a **lock-file update**. Found by the 0.52.0 re-baseline review request (unresolved item 1) and confirmed by the architect on 2026-10-15. An instance under RFC 010 ([`../../accepted/010-upstream-and-supply-chain-maintenance.md`](../../accepted/010-upstream-and-supply-chain-maintenance.md)). |
| Status | Inherited from RFC 010 — **Accepted** (2026-10-15): **assignable now.** An instance of D-4 (the triage ladder, step 1). It must land **before** the re-baseline commits are pushed, so that the first push to `main` has green supply-chain gates. |
| Basis | `docs/dependency-policy.md` rules 1, 3, 6 (measure and report first; advisories are not ignored; `--locked`); threat model T-6, C-6b, T-11; issue register IS-22; `cargo audit` on 2026-10-15: `ammonia 4.1.4` RUSTSEC-2021-0157 and RUSTSEC-2021-0158 (fixed in `>= 4.2.3`, or `>= 4.1.7, < 4.2.0`), `yoke-derive 0.8.3` yanked. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |
| Scope | `Cargo.lock` (and `crates/planeter-web/Cargo.toml` only if the requirement `ammonia = "4"` must change, which it should not). A regression test for the sanitizer. **Non-change scope:** no new crate, no feature change, no sanitizer configuration change (`sanitize_html` keeps calling `ammonia::clean`), no advisory ignore, no other upgrade. |

## What the architect established (so that the task is not re-litigated)

- **Exploitability, tested on the locked 4.1.4 with the default `ammonia::clean`:** `style` attributes are
  stripped entirely (including `url(…)` values), and `<svg>…<animate*>` is removed with its subtree. The
  advisories' conditions are not reachable through planeter's configuration. This is a *finding*, not a
  reason to ignore: a fix exists, the gates are release gates (`release.yml`), and a sanitizer is a
  component we do not want to explain. **The fix is the lock update, not an ignore (rule 3).**
- The 0.52.0 change set is otherwise approved; this handoff exists only so that it is pushed green.

## Task breakdown

- **T1 — Measure.** Record the lock before: `grep -c 'name = ' Cargo.lock`. Run
  `cargo update -p ammonia` and `cargo update -p yoke-derive` (nothing else; `--locked` everywhere
  afterwards). Record the lock after and the crate delta, in the form dependency-policy rule 1 requires:
  "N → M crates (+K); deny: …; audit: …; new licenses: …". Prefer the newest `ammonia` that satisfies
  `"4"` and the workspace MSRV (1.85). **If the update needs a `Cargo.toml` change, a new crate, a new
  licence or an MSRV bump: stop and hand back with the numbers.**
- **T2 — Sanitizer regression tests.** Under `crates/planeter-web/tests/` (not inline; project rule): add
  tests that pin the two advisory shapes and the neighbours the architect probed, asserting on the
  *output*: a `style` attribute with `url('javascript:…')` and with `url("")` leaves no `style`; an
  `<svg>` carrying `animateMotion`/`animate attributeName=href values=javascript:…` leaves nothing; a
  `<math><mtext><style><img onerror=…>` leaves nothing; `<a href="javascript:…">` keeps its text and loses
  the href. They must pass before **and** after the update (they pin behaviour, not the fix).
- **T3 — Gates.** All of: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D
  warnings`, `cargo test --workspace --locked` with prikk on `PATH` and once with prikk genuinely **off**
  `PATH` (use the real toolchain binaries behind the shim: `rustup which cargo`; CONTRIBUTING now says
  so), `cargo deny check`, `cargo audit`, `tools/enf2-symbol-check/check.sh`, `RUSTDOCFLAGS="-D warnings"
  cargo doc --workspace --no-deps --locked`. **`cargo deny` and `cargo audit` must be exit 0.**
- **T4 — Records.** `docs/dependency-policy.md` ledger: one row (date, crates, cost, "advisory fix,
  lock-only"); lock count in its "As of" line; STATUS IS-22 closed. Push `main` (the five re-baseline
  commits and this one) **only when T3 is all green**; the owner's tag and publish are separate.

## QA checklist

- [ ] The lock delta is reported before the commit, with deny and audit results and any new licence.
- [ ] No `Cargo.toml` changed, or the handoff stopped.
- [ ] The T2 tests exist, are under `tests/`, and pass on the old and the new version (state how you showed
      the "before").
- [ ] Both supply-chain gates exit 0; every other gate exit 0, both test runs named.
- [ ] No advisory ignore was added anywhere (`deny.toml`, `.cargo/audit.toml`).

## Definition of done & handback

Done when the checklist is complete and `main` is pushed green. The review request carries: the commits,
every gate with its command and exit code, the lock delta, the T2 test names, and anything surprising
about the new `ammonia` (changed defaults, new transitive crates). Nothing here authorizes a tag.
