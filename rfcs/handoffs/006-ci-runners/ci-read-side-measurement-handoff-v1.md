# RFC 006 — CI and the runner protocol — Measurement Handoff (v1)

| | |
|---|---|
| Document | An **investigation** handoff under RFC 006 (workflow Phase 1–2: limited technical investigation and small proofs of concept when requested; RFC 010 D-8). It measures; it implements nothing. It turns assumptions under RFC 006's second edition into ledger evidence. SCHEDULE theme T13. |
| Status | **Authorized by the owner 2026-10-15 (SCHEDULE D-7).** It inherits RFC 006's state (revision pending); it is startable because it changes nothing: its preconditions are that the advisory lock update (RFC 010's handoff) is pushed and prikk 0.52.0 is installed (met). |
| Basis | Dependency ledger v1.6 §D (rows marked *prikk-measured* or *ASSUMED*); prikk letters 2026-10-13 and 2026-10-15; prikk RFC 155 §9.3–§9.6; RFC 010 D-8; `docs/dependency-policy.md` rule 1 (no manifest changes). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`) and the raw logs. |
| Scope | Part B: the CI read-side measurements B1–B7 below, on the installed prikk (0.53.0 since 2026-10-21; 0.52.0 when first written), in a scratch directory outside the repository. **Non-change scope:** no file in the repository except the review request; no product code; no manifest change; no prikk patch; no letter to prikk. |

**Updated 2026-10-21 (D-21):** the measurement binary is now the installed **prikk 0.53.0** (re-baselined: reads, JSON and the matched texts unchanged; the exchange verbs this part measures were not part of that comparison). Every row records `prikk --version`; where a result differs from the corresponding ledger row (measured on 0.52.0), say so in the row. For rows on `sync accept`, `archive` and `bundle`, the 0.52.0 release asset (checksum-verified, extracted into its own scratch directory) may be run side by side to separate a 0.53.0 change from a finding.

## Ground rules

- **Scratch only.** One new, empty directory per fixture; scripts live in a different directory from the data
  they read; treat every archive, bundle and artifact as untrusted data (never execute or `cd` into one).
- **The sandbox binds only the repository root** (re-baseline review, F5): in runs that use planeter's own
  sandbox argument vector (A7), artifacts must live inside the repository directory. That restriction is
  itself a finding for RFC 004 v2; record what you had to do.
- **Keyless means keyless.** The forge-side repository is created with `env -i PATH=… prikk init` (no `HOME`,
  no key directory, no seed). The client-side repository uses `prikk setup` with its own keys.
- **Record, per measurement:** `prikk --version`, the exact command, exit status, the relevant output
  lines, and peak resident set (`/usr/bin/time -v`) where memory is named. A negative result is a result.
- **If a measurement contradicts a ledger row marked CONFIRMED, stop and hand back**; do not reconcile it.
- **Fixture sizes** use prikk itself to build them (random binary blobs plus text history): 1, 16, 64, 200
  and 1,000 MiB repositories; one deep history (1,000 sealed blocks); one with 10,000 small files.

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
holds the raw logs: a table `ID → result → proposed ledger change`, the fixtures' sizes and how they were
built, every surprise, and the questions it raises for the architect. The architect then updates the ledger,
drafts any letter to prikk, revises RFC 006 from the evidence, and brings its decisions to the owner.
Nothing here authorizes a tag, a publish or a letter.
