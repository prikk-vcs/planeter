# RFC 001 — Foundations — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 001 (Foundations). Task breakdown + PR plan + QA checklist. |
| Status | Inherited from RFC 001 — **Accepted** (the implementer may build against it). |
| Basis | [`../../accepted/001-foundations.md`](../../accepted/001-foundations.md) (decisions D-1…D-7, acceptance criteria); internal design `planeter-04` (§1–§4, §11); project rules (gates, English, Apache-2.0). |
| Audience | The dev team (Mid-Capability Model). Return a **review-request package** (entry-point path) to the owner and the architect when A0 is green. |
| Scope | **Phase A0 only:** the workspace skeleton, the layering gate, `planeter-prikk`, the `planeter-store` repository records, and the `planeter-core` hosting model + security seams. **Out of scope** (later RFCs): the `authorize()` *contract* (RFC 002), the read path/web (RFC 003), transport (RFC 004). Build the *seams* now, not their contents. |

## What you are building, in one paragraph

The bottom of planeter: a Cargo workspace whose layering is a CI gate, a single sandboxed
subprocess boundary to the prikk CLI, and the data model that turns anonymous `.prikk` repositories into
identified hosted ones — plus three empty security seams (authorize, forge-seal-off, egress guard) so the
invariants hold structurally before there is anything to secure. No network surface ships here.

## Crate skeleton (D-1)

Create the workspace with these crates; dependencies point **downward only**:

| Crate | Layer | A0 responsibility |
|---|---|---|
| `planeter-prikk` | lower | the `PrikkRepo` trait + a subprocess CLI implementation (T2–T4) |
| `planeter-store` | lower | the repository/identity records + store abstraction (T5) |
| `planeter-core` | core | the hosting model (create/open repo) + the three security seams (T6) |
| `planeter-web`, `planeter-transport`, `planeter-ci`, `planeter-registry` | upper | **empty placeholders** (crate + lib.rs stub) so the layering gate has real targets |
| `planeter` (bin), `planeter-runner` (bin) | — | **empty placeholders** |

Conventions (project rules): Rust 2024, MSRV pinned, `#![forbid(unsafe_code)]` in every crate (no FFI
yet), English, Apache-2.0 (`LICENSE` present; add `NOTICE` if needed). Gates that must be green on every
PR: `fmt` · `clippy -D warnings` · `test` · `cargo-deny` · `cargo-audit`, all `--locked`.

## Task breakdown (PR plan — build in this order)

- **T1 — Workspace + layering gate (D-1/D-6).** Create the workspace and all crates (real lower/core,
  placeholder upper/bins). Add a **layering-gate** check to CI: read `cargo metadata`, assert no
  production dependency edge from a lower/core crate into an upper crate, **fail naming the edge**.
  *Test:* a deliberately-planted upward edge fails the gate; the clean graph passes. (Mirror prikk's
  RFC 149 `LAYER` rule.)
- **T2 — The `PrikkRepo` trait (D-2).** Define the typed operation surface over prikk's real verbs:
  reads (`log`, `show`, `status`, `branch`, `tag`, `verify`, `worktree_status`,
  `checkout_patch_plan_content`) and exchange (`bundle_export/import`,
  `sync_summary/have/build/accept/pending/seal`). Model each command's `--format json` output as typed
  structs and prikk's exit codes as typed errors (never panic on a non-zero exit). No behaviour yet — the
  trait + types.
- **T3 — The subprocess implementation (D-2/D-3).** Implement `PrikkRepo` by shelling out to the prikk
  binary against a repo's on-disk `.prikk`. Include the **version pin** (D-3): read `prikk --version` and
  the repo `FORMAT`, compare to a declared supported range, **refuse** out-of-range (typed error), do not
  guess. Parse JSON into T2's types; map exit codes.
- **T4 — Sandbox the invocation (D-3 / INV-3 / C-4c).** Wrap every prikk invocation in confinement.
  **First cut:** a restricted execution context — a filesystem view limited to the repo dir + a scratch
  temp, **no network**, and CPU/memory/wall-time `rlimit`s; refuse on limit hit. **Recommended
  mechanism:** a user-namespace sandbox (e.g. `bwrap`/bubblewrap) bind-mounting only the repo dir and
  scratch, `--unshare-net`, with rlimits — clean and dependency-light on Linux; a plain restricted-user +
  rlimits fallback is acceptable for the first pass with the namespace hardening tracked as a follow-up.
  The **contract is fixed** (a prikk parser fault is contained, no network, bounded resources); the exact
  tool is your pragmatic choice, recorded in the PR. *Test:* an invocation cannot read outside the repo
  dir, cannot open a socket, and is killed at the resource ceiling.
- **T5 — Repository records + store (D-4/D-5).** In `planeter-store`: the **repository record**
  `{ repo_id, owner_ref, name, visibility ∈ {public,internal,private}, created_at,
  prikk_format_version, path }`, the `owner/name → repo_id` resolution, and CRUD + rename/transfer
  (rename/transfer change the record, **never** move on-disk bytes — the path is keyed by `repo_id`).
  Behind a store trait; **SQLite** first implementation (IQ-4). Visibility is stored as data only; do not
  enforce it here (RFC 002).
- **T6 — Hosting model + security seams (D-4/D-6) in `planeter-core`.**
  - **Create/open a hosted repo:** allocate `repo_id`, lay out `<repos-root>/<shard>/<repo_id>/.prikk`
    (D-4), run `prikk init` via `planeter-prikk` for a new repo; open resolves record → path → driver.
  - **`authorize()` seam:** a single function/trait every future surface must call; A0 ships the seam
    (signature + a default-deny stub), RFC 002 fills the contract. This is what makes `LAY-2`/`INV-4`
    structural from the start.
  - **`forge-seal` cargo feature, off by default (LAY-4/ENF-2):** reserve it; **no seal code exists yet.**
    Add the **ENF-2 CI check**: the default build's binary contains **no** forge-signer symbol (a `nm`/
    symbol grep). It passes trivially now (nothing to find) and stays as the guard that keeps INV-2 true
    as write code arrives later.
  - **egress-guard seam (LAY-5):** a single outbound-fetch entry point stub (SSRF-filter contract, no
    callers yet) so RFC 004/webhooks route through one point.
- **T7 — The `stikk-prikk` evaluation (D-7 / IQ-1).** Evaluate building T3 on stikk's existing
  `stikk-prikk` layer versus the independent driver, **against the `PrikkRepo` trait**. Record the
  decision (reuse vs. independent) and the reasoning in the PR; either way callers are unaffected.

## Acceptance / QA checklist (from RFC 001)

- [ ] Workspace builds; `forbid(unsafe_code)` holds; `fmt`/`clippy -D`/`test`/`cargo-deny`/`cargo-audit`
      green (`--locked`).
- [ ] Layering gate runs in CI; **fails a planted upward edge**, passes the clean graph.
- [ ] `planeter-prikk`: refuses an out-of-range prikk version; runs `log --format json` → typed data;
      runs a `bundle`/`sync` command; all inside T4's confinement (verified: no out-of-dir read, no
      socket, killed at ceiling).
- [ ] `planeter-store`: create/open/rename/resolve a repository record; rename does **not** move bytes.
- [ ] `planeter-core`: create a hosted repo (`prikk init` under the D-4 layout) and re-derive a fact from
      it via `planeter-prikk` **with the forge metadata store cleared** (proves prikk is source of truth,
      ENF-5).
- [ ] **ENF-2 symbol-absence check** passes on the default build.
- [ ] Threat model re-verified: A0 touches the prikk boundary (parser confinement) — confirm `INV-3`/
      `C-4c` controls are present; nothing here adds a signing path (`INV-2`). If a new data flow was
      introduced, update `planeter-03`.

## Definition of done & handback

A0 is done when the checklist is green and the layering gate + ENF-2 check are wired into CI. Then return
a **review-request package** to the owner and architect — a short entry-point document naming: the PRs,
the sandbox mechanism chosen (T4), the `stikk-prikk` decision (T7), and any deviations from this handoff
with reasons. The architect then proceeds to **RFC 002 (authorization)** and **RFC 003 (read path)** on
this base toward **M1 (0.1.0)**. Everything remains **v0.x**; 1.0 is an explicit owner gate.
