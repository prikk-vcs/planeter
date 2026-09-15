# RFC 001 — Foundations: the prikk-integration layer, the layering gate, and multi-repo hosting

**Status.** Proposed (2026-09-15) — the first planeter RFC, and the substrate every later surface sits
on. Draft for review; on the owner's acceptance it moves to `accepted/` and the dev team builds the
workspace skeleton per the handoff. Defines *how planeter talks to prikk*, *how the crates are layered
and gated*, and *how many anonymous prikk repositories become identified, hosted repositories*.
Handoff: forthcoming (`../handoffs/001-foundations/`).
**Tracks.** ROADMAP Phase A0 → A1 (Track A — depends only on prikk's existing CLI). Internal design
§1 (`CR-*`), §2 (`LAY-*`), §3 (`PKI-*`), §4 (`DM-*`), §11 (`ENF-*`); external design `PK-*`, `BD-03`;
requirements `INT-1…6`, `BN-1…5`, `CAP-1`; threat model `INV-2/3/4/6`, `TB-4`, `C-4c`.
**Touches.** The Cargo workspace skeleton and its layering; a new `planeter-prikk` crate (the prikk CLI
driver); `planeter-store` (the repository/identity records only, here); `planeter-core` (the layering
seams and the hosting model); and a `boundary-check`-style layering gate in CI. **Not here:** the
authorization model (RFC 002), the read path and web (RFC 003), transport (RFC 004) — all built *on*
this. No network surface ships in this RFC.

## Summary

planeter's value flows through prikk: it hosts prikk repositories and re-derives every repository fact
from prikk (`INT-4`). This RFC fixes the three things everything else depends on, so they are right once
rather than reworked later: (1) **the prikk boundary** — a subprocess-driven, version-pinned, sandboxed
integration layer, the *only* code that talks to prikk; (2) **the layering** — a crate structure whose
dependency direction is a CI-enforced gate, so the security invariants are properties of the build graph
(the discipline prikk settled in its own RFC 149); and (3) **multi-repo hosting & identity** — the model
by which planeter gives anonymous prikk repositories the owner/name/URL identity a forge needs, *above*
prikk, never inside it (RFC 145 §7).

The one idea this RFC turns on: **planeter carries the hosting weight so prikk stays lean, and it does so
over prikk's stable CLI — not its unstable crate API** (`INT-1`, RFC 145 §8d). The boundary is a process
boundary; prikk's internals never enter the forge, and prikk's churn cannot break it.

## The constraints that scope this design

From the 2026-09-15 prikk survey and the design set:

- **prikk is a single-repository, offline, file-based CLI** with a machine-readable read surface
  (`--format json`) and artifact-based exchange (`bundle`/`sync`). It has **no network, no multi-repo,
  no accounts** — those are planeter's to build (`§0a` of requirements).
- **The crate API is explicitly unstable and is not the substrate** (RFC 145 §8d). The CLI JSON surface
  is. planeter links no prikk crate (`INT-1`, `BD-03`).
- **prikk parses untrusted bytes** on any push (`TB-3`), so the process that runs prikk is an
  untrusted-input parser and must be confined (`C-4c`).
- **prikk's format is unstable pre-1.0** (`UD-3`); planeter must pin which versions it supports and
  refuse the rest rather than guess (`OP-04`).
- **prikk repositories are anonymous by settled design** (RFC 145 §7); a view must not invent identity
  *inside* prikk. planeter's identity lives above.
- **The forge must be able to hold no signing key by construction** (`INV-2`, OQ-1 a): the layering must
  make "the default build cannot seal history" checkable, from the very first commit.

## Decisions

- **D-1 — The workspace is layered, and the layering is a gate.** planeter is a Cargo workspace
  (Rust 2024, MSRV-pinned, `forbid(unsafe_code)` outside any isolated FFI). Three layers, dependencies
  pointing **downward only**:
  - **lower:** `planeter-prikk` (the CLI driver, D-2) and `planeter-store` (persistence; here only the
    repository/identity records).
  - **core:** `planeter-core` (the hosting model D-4/D-5, and — RFC 002 — the `authorize()` service).
  - **upper (later RFCs):** `planeter-web`, `planeter-transport`, `planeter-ci`, `planeter-registry`.
  A **layering gate** runs in CI (a `boundary-check`-style check over `cargo metadata`): a production
  dependency edge from a lower/core crate into an upper crate **fails, naming the edge**. This is
  RFC 149's `LAYER` discipline adopted from day one (`LAY-1`).
- **D-2 — `planeter-prikk` is the sole prikk boundary, behind a trait.** It exposes a typed
  `PrikkRepo` trait over prikk's real verbs — reads (`log`, `show`, `status`, `branch`, `tag`, `verify`,
  `worktree-status`, `checkout --patch-plan --content-path`, all `--format json`) and exchange
  (`bundle export/import`, `sync summary|have|build|accept|pending|seal`). The first implementation
  **shells out to the prikk binary** per hosted repo and parses its JSON into typed values. No other
  crate invokes prikk or parses its output (`LAY-3`). Errors and prikk exit codes map to typed results;
  a non-zero prikk exit is a typed failure, never a panic.
- **D-3 — The prikk subprocess is version-pinned and sandboxed.** `planeter-prikk` declares a supported
  **prikk CLI/format version range**, checks it (via `prikk --version` and the repo `FORMAT`), and
  **refuses** an unsupported version rather than guessing (`OP-04`, `PKI-4`). Every invocation runs
  **confined** — a restricted user, a filesystem view limited to that repo's directory plus a scratch
  artifact dir, no ambient credentials, and CPU/memory/time bounds — so a prikk parser fault on hostile
  pushed bytes is contained in a subprocess, not the forge (`C-4c`, `INV-3`; concrete confinement
  mechanism is a handoff detail, the *requirement* is stated here).
- **D-4 — A hosted repository is one `.prikk` under a planeter-managed root.** Layout:
  `<repos-root>/<shard>/<repo-id>/.prikk`, where `repo-id` is planeter's stable internal id and `shard`
  keeps directory fan-out bounded at scale (the human `owner/name` is *not* the on-disk path — renames
  and transfers must not move bytes). Creating a repository runs `prikk init` in a fresh dir; importing
  one lands a decoded/`bundle import`ed `.prikk`. One repository = one `.prikk`; planeter never packs
  many repos into one prikk store (`DM-1`).
- **D-5 — Identity is planeter's, above anonymous prikk.** `planeter-store` holds a **repository
  record**: `{ repo-id, owner (user|org) ref, name, visibility ∈ {public, internal, private},
  created_at, prikk_format_version, → on-disk path }`, plus the owner/name → repo-id resolution that
  gives each repository its URL. prikk sees only an anonymous `.prikk`; **planeter never writes
  owner/name/URL into prikk** (RFC 145 §7, `INT-3`). Visibility is stored here as data; *enforcing* it is
  the authorization service's job (RFC 002). The record keys future forge metadata (issues, reviews) to
  prikk object ids, never duplicating history (`INT-4`, `NG-6`).
- **D-6 — The security seams are established now, before there is anything to secure.** Three structural
  hooks, set up empty in A0 so later RFCs fill them rather than retrofit them:
  - an **`authorize()` seam** in `planeter-core` that every surface must call (RFC 002 defines the
    contract; A0 provides the seam so `LAY-2`/`INV-4` hold structurally from the start);
  - a **`forge-seal` cargo feature, off by default**, reserved as the *only* place a forge-held-key seal
    could ever live (`LAY-4`). No seal code exists yet; the feature and an **ENF-2 CI symbol-absence
    check** (the default build contains no forge-signer symbol) are established now, so `INV-2` is a
    construction guarantee from commit one;
  - a **single egress guard** in `planeter-core` for every outbound fetch (`LAY-5`), so RFC 004's
    transport and later webhooks/mirroring route through one SSRF-filtering point, not many.
- **D-7 — The stikk-`stikk-prikk` reuse decision is a swappable implementation, not a fork in the road.**
  Because D-2 defines `planeter-prikk` behind the `PrikkRepo` trait, whether the shelling-out
  implementation is planeter's own or built on stikk's existing `stikk-prikk` layer (`IQ-1`/`OQ-7`) is a
  choice of *implementation*, deferrable without reworking callers. The RFC's recommendation: implement
  planeter's own first-pass driver, evaluate `stikk-prikk` reuse against the trait, and record the
  outcome in the handoff.

## What "done" means (acceptance criteria)

- The workspace builds with the three lower/core crates present and empty upper-crate placeholders;
  `forbid(unsafe_code)` holds; the gates (fmt · clippy `-D warnings` · test) are green.
- The **layering gate** runs in CI and fails a deliberately-introduced upward edge (tested).
- `planeter-prikk` can, against a real on-disk prikk repo: run a pinned-version check and refuse an
  out-of-range version; execute a read (`log --format json`) and return typed data; run a
  `bundle`/`sync` command; and do all of it inside the confinement of D-3 (tested with a restricted
  invocation).
- `planeter-store` can create, open, rename, and resolve a repository record (D-5), and `planeter-core`
  can create a hosted repository (`prikk init` under the D-4 layout) and re-derive a fact from it via
  `planeter-prikk` — with the forge metadata store cleared, proving prikk is the source of truth
  (`ENF-5`).
- The **ENF-2 symbol-absence check** passes on the default build (no forge-signer symbol present).

## Alternatives considered

- **Link `prikk-store` for in-process reads (no subprocess).** Rejected: the crate API is explicitly
  unstable (RFC 145 §8d) and has no read-only facet (`RefStore` carries write authority, RFC 145 §8c);
  linking it couples planeter to prikk's churn and forfeits the process-boundary containment (`C-4c`).
  The subprocess is slower per call but correct and safe; caching (RFC 003) addresses the cost.
- **One prikk store hosting many repositories (namespacing inside prikk).** Rejected: prikk is
  single-repository by design; forcing multi-repo into it would push hosting weight down into prikk
  (`BN-4`) and invent identity prikk forbids (RFC 145 §7).
- **Enforce layering by convention/review instead of a gate.** Rejected: prikk's own RFC 149 shows a
  layer holds only when it is a checked rule; convention drifts.

## Open questions & dependencies

- **IQ-1 / OQ-7** — reuse `stikk-prikk` or build the driver independently (D-7). Dev-team decision behind
  the trait; not blocking.
- **IQ-4** — the `planeter-store` backend (SQLite-first, PostgreSQL path). This RFC fixes only the
  repository-record *shape* and the store abstraction boundary; the engine choice is deferrable.
- **UD-4 / IQ-3** — prikk local-locking sufficiency beneath planeter's per-repo write serialization is
  confirmed in RFC 004 (transport), not here; A0/A1 are read-and-create only.
- Concrete **confinement mechanism** for D-3 (user namespaces / seccomp / a wrapper) is a handoff choice;
  the RFC fixes the *requirement*, the handoff picks the *how* for the target platform.

## Sequencing & handoff

This RFC is the first, and blocks the others (ROADMAP §RFC plan): nothing starts before the workspace,
the layering gate, and the prikk boundary exist. On acceptance, the architect writes
`handoffs/001-foundations/` (crate-by-crate task breakdown, the confinement mechanism, the stikk-prikk
evaluation, the layering-gate and ENF-2 check implementations, and the acceptance tests above); the dev
team builds A0 against it; then RFC 002 (authorization) and RFC 003 (read path) proceed on this base to
reach **M1 (0.1.0)** — the first usable hosted, browsable, access-controlled forge. All of it is v0.x;
1.0 is an explicit owner gate (ROADMAP).
