# A0 — review-request package (RFC 001 foundations)

**Status: A0 complete.** All seven tasks (T1–T7) landed on `main`, every gate green, the layering gate
and the ENF-2 check wired into CI. This is the entry-point document RFC 001's definition-of-done asks
for: the commits, the sandbox choice (T4), the `stikk-prikk` decision (T7), and deviations.

## Commits (on `main`, `prikk-vcs/planeter`)

| Task | Commit | What landed |
|------|--------|-------------|
| T1 | `f7bc98e` | Cargo workspace (Rust 2024, MSRV 1.85, resolver 3, 11 crates), layered skeleton, the layering gate (`tools/boundary-check`). |
| T2 | `47e8e58` | The `PrikkRepo` trait + typed models of prikk's `--format json` (faithful to prikk 0.43.0). |
| T3 | `3005fc2` | The subprocess driver `CliPrikkRepo` + the version pin (refuses prikk < 0.43.0). |
| T4 | `b879291` | Bubblewrap confinement of every prikk invocation. |
| T5 | `177398b` | Repository/identity records + the swappable `RepositoryStore` (in-memory backend; SQLite is the follow-on). |
| T6 | `cd4d63d` | The hosting model (`create_repo`/`open`) + the three security seams (authorize, egress, `forge-seal`/ENF-2). |
| T7 | *(this handoff)* | The `stikk-prikk` reuse evaluation — decision recorded in `t7-stikk-prikk-evaluation.md`. |

## Sandbox mechanism chosen (T4)

**Bubblewrap (`bwrap`)**, owner-ruled 2026-09-16 over a plain restricted-user fallback. Every prikk
invocation runs `timeout … bwrap …`: a read-only view of `/usr` (with usr-merge symlinks) so prikk finds
its libraries, the repository directory as the **only** writable bind, `--proc`/`--dev`, `--unshare-all`
(no network), `--die-with-parent`, `--new-session`, and `--clearenv` (no `HOME` → no operator keys reach
the subprocess, which is keyless-forge-correct). Wall-time is bounded by `timeout`; memory/CPU/fsize
rlimits and the artifact size/count/ratio bounds are a documented follow-on enforced by planeter
*before* handing bytes to prikk (RFC 004). Rationale and the contract are in
`crates/planeter-prikk/src/sandbox.rs`.

## `stikk-prikk` decision (T7)

**Independent driver, kept — no code dependency on stikk.** The surfaces differ (planeter needs the
bundle/sync exchange verbs stikk lacks; stikk's reads return TUI-shaped `stikk-model` types), reuse
would couple two *peer* front-ends and force lockstep prikk rebaselines, and planeter's untrusted-input
confinement + keyless property have no analogue in a local TUI. The `PrikkRepo` trait preserves the
freedom to revisit. Full reasoning and an escape hatch (a future vendor-neutral `prikk-cli-common` if a
third consumer appears) in `t7-stikk-prikk-evaluation.md`.

## QA checklist (RFC 001) — status

- [x] Workspace builds; `forbid(unsafe_code)` holds; `fmt` / `clippy -D` / `test` / `cargo-deny` /
      `cargo-audit` green (`--locked`).
- [x] Layering gate runs in CI; **fails a planted upward edge**, passes the clean graph
      (`tools/boundary-check/tests/layering.rs`).
- [x] `planeter-prikk`: refuses an out-of-range prikk version; runs reads → typed data; runs
      `bundle`/`sync` verbs; all inside the bwrap confinement (verified by `tests/driver.rs`, self-skips
      without prikk).
- [x] `planeter-store`: create/open/rename/resolve a record; rename/transfer does **not** move bytes
      (`memory::tests::rename_and_transfer_keep_repo_id_and_path`).
- [x] `planeter-core`: creates a hosted repo (`prikk init` under the D-4 layout) and re-derives a fact
      from it via `planeter-prikk` **with the store bypassed** — proving prikk is the source of truth
      (ENF-5; `tests/hosting.rs::create_lays_out_by_id_and_prikk_is_the_source_of_truth`).
- [x] **ENF-2 symbol-absence check** passes on the default build (`tools/enf2-symbol-check`, CI `enf2`
      job): no forge-signer symbol present (INV-2).
- [x] Threat model re-verified: A0 touches the prikk boundary (parser confinement) — `INV-3`/`C-4c`
      controls present (T4); nothing here adds a signing path (`INV-2`, guarded by ENF-2). No new data
      flow requiring a `planeter-03` update.

## Deviations from the handoff (with reasons)

1. **T5 backend: in-memory first, SQLite deferred** (handoff said "SQLite first"). The handoff's own
   IQ-4 fixes the *abstraction* as T5's deliverable; the concrete engine sits behind the
   `RepositoryStore` trait. A0 ships the trait + a real thread-safe in-memory backend (std-only, no new
   dependencies) with full invariant tests; the SQLite backend is a clean, caller-invisible follow-on.
   No interface changes when it lands.
2. **T4 rlimits: wall-time only in the first cut** (handoff allowed a first pass; memory/CPU/fsize
   tracked as hardening). Recorded in `sandbox.rs`; the security-relevant bounds for untrusted input
   (size/count/ratio) are RFC 004's job, before bytes reach prikk.

Neither deviation changes a public interface or a threat-model control.

## Handback

A0 is done. The architect proceeds to **RFC 002 (authorization)** — filling the `authorize()` seam —
and **RFC 003 (read path)**, toward **M1 (0.1.0: host / browse / auth)**. No release or tag during A0
(pre-release; the first tag is M1/0.1.0).
