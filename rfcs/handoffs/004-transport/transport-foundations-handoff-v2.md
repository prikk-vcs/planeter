# RFC 004 — Transport — Implementation Handoff (v2, part 1): foundations

| | |
|---|---|
| Document | The first of the RFC 004 v2 series (sequence below). It builds the **foundations every later part stands on and none of the user-visible transport**: the handler-enumeration test that must exist before any write handler (IS-14, ENF-4), the `propose` action (RFC 004 D-3, RFC 005 D-9), the repository lease (D-7), the artifact store and spool primitives (D-2), and the two extra sandbox mounts (D-6). Nothing here is reachable from the network; no route is added. |
| Status | **Done, reviewed Approved (2026-10-21; commits `83f8696`…`d9c6ec9`).** IS-14 closed. Part 2: `advertise-and-fetch-handoff-v2.md`. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-2, D-6, D-7 and its measured facts U-12…U-17; [`../../accepted/005-change-review.md`](../../accepted/005-change-review.md) D-9; RFC 002 (`authorize()`, default-deny, `Action::ALL` property test); RFC 011 (admission, `CallLimits`, child lifetime: done); `docs/src/planeter-04-internal-design-v0.1.md` ENF-4; `crates/planeter-core/src/{authorize,hosting,layout,admission}.rs`, `crates/planeter-prikk/src/sandbox.rs`, `crates/planeter-store`, `crates/planeter-transport` (a skeleton), `crates/planeter-web/src/{gate,budget}.rs`; ledger PK-37, PK-39, PK-59, PK-60; IS-14. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## The RFC 004 / 005 series (so you can see where this part sits)

1. **Foundations** (this handoff): IS-14 test, `propose`, lease, artifact store, sandbox mounts.
2. **Advertise and fetch**: the HTTP envelope for advertise/fetch, export under the X lease, verify gate, cache, ETag/304,
   instrumentation; creation from an archive (D-4).
3. **Accept and the intent journal**: edge bounds and rate limits, the accept with its own deadline and memory admission,
   the intent journal and `needs-recovery` (D-3, D-5, D-6) — together with RFC 005's data model (change, revision,
   quotas).
4. **Recovery**: the written procedure, automated only where the lease, the journal and the process table agree (D-6).
5. **SSH** (D-9), **the client helper** (D-10), then the threat-model update and the release candidate.
RFC 005's parts (data model and quotas; review API and UI; the file-level preview after prikk 0.54.0) are interleaved from
part 3. Each part is its own handoff and review; nothing is startable before its predecessor is approved.

## Change scope

- **T1 — IS-14 first: the handler-enumeration test (ENF-4).** Before any other task: a test that **enumerates every
  route and handler the web surface registers** (the UI router, the API router, and the static/health routes) and
  asserts that each one either calls `authorize()` (through the read gate or the principal path) or is on a short,
  reasoned **public allowlist** (login page, static stylesheet, OpenAPI document, health). A new route without an
  entry fails the build. Mechanism: whatever keeps this honest without runtime reflection (for example a single
  registration function every route goes through, which records its authorization class in a table the test
  reads); report the mechanism and its limits. The same table will later carry the transport routes and the SSH verbs:
  design it for that. Shown able to fail (a planted unauthorized route; a mutation removing a check).
- **T2 — The `propose` action.** Add `Action::Propose` to RFC 002's actions (`Action::ALL` grows to 10): a
  repository-level action, default-deny, granted by default to `write` and above, never to anonymous, and (per RFC 005 D-9)
  to any authenticated principal when the repository setting `proposals` is `any-authenticated`
  (`members` by default). The setting is a new column/record on the repository with its migration (`apply_schema`,
  idempotent), a default of `members`, and a validated parser (an unknown value is refused, not defaulted). `Propose`
  is **sensitive** (audited, `is_sensitive`). Update the default-deny property test, the permission matrix and the audit
  tests. No route uses it yet.
- **T3 — The repository lease (D-7).** In `planeter-core`: a lease manager per repository store with the three classes
  **R**, **X**, **W** of the RFC's table (R allowed with R, X, W; X excludes W and X; W excludes X and W), **bounded
  waits** that end in a typed "busy" error carrying a retry hint (the transport turns it into 503 `Retry-After`), fairness
  that cannot starve a writer behind a stream of readers (readers never block on a waiting writer, but a waiting writer is
  served before *new* X requests), release on drop and on panic, cancel-safe, no unbounded growth in the table, and a
  **durable holder record** for W and X (who holds it, since when, which operation) in planeter's store so that recovery
  (part 4) can see a dead holder after a restart. The lease is **in-process** (one writer process per store: the RFC's
  stated deployment shape); document that a second server process against one store is unsupported and refused at start
  (an advisory lock file next to the database, cleaned on exit, with a clear message). Tests: the full matrix of class
  pairs; bounded wait; no starvation; release on drop and panic; the durable record survives a simulated restart.
- **T4 — The artifact store and spool primitives (D-2).** A content-addressed store for accepted artifacts and a spool for
  exports, both under operator-configured directories **on a different volume from the repositories if configured so**
  (the configuration is validated: the spool must not be inside a repository root): atomic write (`<name>.partial` →
  `fsync` → rename), content-addressing by SHA-256 of the bytes, a disk budget with **LRU eviction for the spool only**
  (the artifact store never evicts: retained artifacts are governed by RFC 005's retention), and cleanup of orphaned
  `.partial` files at start. No export or accept logic yet: the primitives and their tests only (a crash between write and
  rename leaves no visible object; the budget evicts the least recently used spool entry first; eviction never touches
  the store; concurrent writers of the same digest converge).
- **T5 — Sandbox mounts (D-6).** `planeter-prikk`'s sandbox profile gains two optional mounts outside the repository:
  an **artifact-in** directory (read-only) and a **spool-out** directory (read-write), added to the argument vector only
  when configured, with unit tests on the vector (absent by default: the vector for existing read calls is **byte-identical**
  to today's) and a real-`bwrap` test that a call can read the first and write the second and cannot write anywhere else
  (including the repository's sibling directories). The `extract` destination parent (RFC 006 / ledger PK-65) will be a
  third such mount later; design the profile so a mount list generalizes, but add only these two now.

## Non-change scope

No route, no network surface, no `sync accept` or `archive export` call, no change to any existing response, no
admission-cost change, no new dependency (if one seems needed, stop and report), no `unsafe`, no version or release change.
IS-14's table covers routes that exist today; transport routes arrive with their own parts.

## Required documentation updates

`docs/RELEASING.md` or a new `docs/operating.md` section only for the configuration this part introduces (spool and
artifact-store directories, the single-process rule); CHANGELOG `[Unreleased]` for operator-visible facts (the new
configuration, the startup refusal for a second process); STATUS IS-14 closed on review; the authorization tests and the
design-set note ENF-4 (the architect updates `docs/src/` after review).

## Acceptance criteria

T1's enumeration test passes and fails under its two mutations; `Action::ALL` has 10 entries with default-deny proved by
the property test; the lease matrix, bounded wait, starvation and restart tests pass; the store/spool crash and
eviction tests pass; the existing sandbox vector is unchanged byte for byte and the new mounts work under real `bwrap`
(self-skip with a printed reason without it); every gate green including both supply-chain gates (fresh advisory
database), both test runs named, ENF-2, `cargo doc`.

## Prohibited shortcuts

No allowlist entry without a reason; no lease that can deadlock or block a reader on a writer; no spool inside a
repository; no eviction of retained artifacts; no feature flag that disables the enumeration test; no widening of the
sandbox beyond the two named mounts.

## Required evidence and review-request format

Commits (hash → what, by task); the enumeration table (route → class) and the mutation evidence; the lease matrix
results and the restart test; the store/spool crash tests; the vector before/after for an existing call; every gate with
command and exit code and the advisory-database HEAD; deviations and open questions.
