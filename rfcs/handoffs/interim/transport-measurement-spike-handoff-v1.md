# Interim — transport and CI read-side measurement spike — Implementation Handoff (v1)

| | |
|---|---|
| Document | A **measurement-only** handoff: it turns the assumptions under RFC 004 v2 (transport) and RFC 006 v2 (CI) into evidence, so that those RFCs and the owner's ruling on M2's shape rest on measured behaviour, not on documents. It writes no product code. Supporting work without an RFC of its own (workflow §6.6). Theme **T13** in `docs/SCHEDULE.md`. |
| Status | **Authorized by the owner 2026-10-15 (SCHEDULE D-7); held until precondition (3) holds (the re-baseline v2 is done), so not yet assignable.** Preconditions: (1) the owner authorizes it — **done**; (2) prikk **0.52.0** is installed locally (`prikk --version`) — **met 2026-10-15**; (3) the re-baseline handoff v2 is done, so the dev team has one measured cadence on that binary. |
| Basis | Dependency ledger v1.5 §D (the rows marked *prikk-measured* or *ASSUMED*: PK-33, PK-37–PK-41, PK-43 and the candidates below); prikk letters 2026-10-13 and 2026-10-15; prikk RFC 155 §9.3–§9.6; `docs/dependency-policy.md` rule 1 (no manifest changes here). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-request/`) and the raw logs. |
| Scope | Run the measurements below on a prikk 0.52.0 binary, **in a scratch directory outside the repository**, and report results with their commands, exit codes and peak memory. **Non-change scope:** no change to any file in the repository except the review request; no product code; no manifest changes; no prikk patches; no letters to prikk (the architect drafts them from the results). |

## Ground rules

- **Scratch only.** One new, empty directory per fixture; scripts live in a different directory from the data
  they read; treat every archive, bundle and artifact as untrusted data (never execute or `cd` into one).
- **Keyless means keyless.** The forge-side repository is created with `env -i PATH=… prikk init` (no `HOME`,
  no key directory, no seed). The client-side repository uses `prikk setup` with its own keys.
- **Record, per measurement:** `prikk --version`, the exact command, exit status, the relevant output
  lines, and peak resident set (`/usr/bin/time -v`) where memory is named. A negative result is a result.
- **If a measurement contradicts a ledger row marked CONFIRMED, stop and hand back**; do not reconcile it.
- **Fixture sizes** use prikk itself to build them (random binary blobs plus text history): 1, 16, 64, 200
  and 1,000 MiB repositories; one deep history (1,000 sealed blocks); one with 10,000 small files.

## Part A — transport (RFC 004 v2)

| ID | Measure | Record / learn | Ledger row |
|---|---|---|---|
| A1 | Keyless receiver: `init` → `archive verify` → `trust maintainer add` (public key) → `archive import --adopt` of a maintainer-built archive; `branch list`; `verify`. Then the same without the `trust maintainer add`. | Exit codes; `heads/main` published or `remotes/`; any key directory created | PK-33 re-measured |
| A2 | Push path: client `sync build <ref> --have <forge have>` → keyless `sync accept` at 1/16/64/200 MiB artifacts; peak RSS; the over-bound refusal (exit code, text) with `PRIKK_EXCHANGE_MAX_BYTES` set through `env`; idempotent re-accept; a byte-flipped artifact | The memory multiplier *k* for accept; refusal keyed on exit 1 | PK-28, PK-31, PK-40 |
| A3 | `sync have <ref>` on a keyless forge in each state: fresh `init`; after `archive import --adopt`; after `sync accept` (pending only); with a received ref; for an absent ref. Sizes at 1k/10k/100k patches (`sync summary` too) | Whether an incremental push artifact is possible at all, or every push re-sends history | new: PK-44 |
| A4 | `archive export` of a repository that accepted a sync artifact: `archive verify`, `archive import` into a fresh repository, receiver `verify` | The 0.52.0 closure fix on our own corpus | PK-32 |
| A5 | `bundle export --ref` on a local sealed branch and on a received ref | Whether a received ref is exportable by bundle on 0.52.0 | PK-4 / new |
| A6 | `tree`, `log`, `show`, `diff`, `cat`, `verify` with `--ref remotes/…` on a keyless receiver | Whether received refs can be rendered | PK-29 re-measured |
| A7 | **Locks.** (i) `archive export` of a large repository while a `sync accept` is started: time to `LockConflict`; (ii) `SIGKILL` an accept mid-write: which lock files remain, `doctor`'s text, `unlock` with and without `--force`, both inside `bwrap --unshare-all` with our sandbox's argument vector and outside; then a re-accept; (iii) is `bundle export` lock-free? | Whether `unlock`'s PID advisory is misleading under our sandbox; the lease classes | PK-37, PK-39 |
| A8 | `archive export` duration and peak RSS at 50/200/1,000 MiB, cold and warm cache; spool disk used | Sizing for the spool | PK-32 |
| A9 | Prefix stability: export, `sync accept` one more artifact, export again; compare each section's bytes and each manifest entry | Evidence for prikk's append-only question (ours, not a contract) | PK-38 |
| A10 | `verify` and `tree` in a loop while an accept loop runs; count transient failures and their text | Whether readers must wait for writers | new: PK-45 |
| A11 | `archive import` into an **existing** repository (no `--adopt`, then with): what lands where; time and peak RSS | The fetch path of the client helper | PK-34 |

## Part B — CI read side (RFC 006 v2)

| ID | Measure | Record / learn | Ledger row |
|---|---|---|---|
| B1 | Snapshot by overlay: `bwrap --overlay-src <repo> --tmp-overlay <dest>` (and `--ro-overlay`); `prikk checkout --snapshot-materialize` into it; archive the result; confirm nothing persisted and no `worktree.provisional` survived; on this host, then in the container recipe (`seccomp=unconfined` as documented) | Does it work, and where; time and RSS at 100/1,000/10,000 files | PK-41 |
| B2 | `tree` + per-file `cat` over the same trees | The cost curve behind "rejected beyond toy repositories" | PK-41 |
| B3 | Throwaway seal for an open change: copy the store; scratch key made by prikk inside the sandbox; `sync seal` the claims; `tree` the new block; delete | Cost, disk, and whether any signer code is needed in planeter's own binary (expected: none) | PK-41 |
| B4 | Determinism: build the same snapshot twice; vary on-disk order; compare bytes (fixed mtime, owner 0, sorted by prikk's path order) | Whether a snapshot can be cached and checked by digest | new |
| B5 | Stampede: 50 concurrent snapshot requests for one tip against a single-flight stub | Builds, memory, latency | new |
| B6 | A rootless OCI runtime and a bubblewrap rootfs as runner isolation: start-up time; an escape checklist (socket, `/proc`, devices, network, host mounts) | Input to decision D-C4 | new |
| B7 | Workflow census: five representative workflows (cargo build and test, a Node build, a container build, a scheduled job, a matrix job) against the checklist in the RFC 006 note | The subset in black and white | new |

## Definition of done & handback

Done when every row has a result (or an explained inability, with the blocking fact) and the review request
holds the raw logs. The review request carries: a table `ID → result → proposed ledger change`; the
fixtures' sizes and how they were built; every surprise; and the questions it raises for the architect. The
architect then bumps the ledger (rows PK-44, PK-45 and the promotions), drafts any letter to prikk, writes
RFC 004 v2 and RFC 006 v2 from the evidence, and brings decisions D-8 to D-10 to the owner. Nothing here
authorizes a tag, a publish or a letter.
