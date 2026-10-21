# RFC 004 — Transport — Measurement Handoff (v1)

| | |
|---|---|
| Document | An **investigation** handoff under RFC 004 (workflow Phase 1–2: limited technical investigation and small proofs of concept when requested; RFC 010 D-8). It measures; it implements nothing. It turns assumptions under RFC 004's second edition into ledger evidence. SCHEDULE theme T13. |
| Status | **Done, reviewed Approved 2026-10-21 (A1–A12 measured on 0.53.0 and 0.52.0; ledger v1.9 PK-57–PK-60).** (Authorized by the owner 2026-10-15, SCHEDULE D-7.) It inherits RFC 004's state (revision pending); it is startable because it changes nothing: its preconditions are that the advisory lock update (RFC 010's handoff) is pushed and prikk 0.52.0 is installed (met). |
| Basis | Dependency ledger v1.6 §D (rows marked *prikk-measured* or *ASSUMED*); prikk letters 2026-10-13 and 2026-10-15; prikk RFC 155 §9.3–§9.6; RFC 010 D-8; `docs/dependency-policy.md` rule 1 (no manifest changes). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`) and the raw logs. |
| Scope | Part A: the transport measurements A1–A11 below, on the installed prikk (0.53.0 since 2026-10-21; 0.52.0 when first written), in a scratch directory outside the repository. **Non-change scope:** no file in the repository except the review request; no product code; no manifest change; no prikk patch; no letter to prikk. |

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
| A12 | **Change derivation (RFC 005 v2).** On a keyless forge: `sync pending` before and after one `sync accept` (the difference is the change's patches); an idempotent re-accept; an interrupted accept (`SIGKILL`) then retry; `show <patch-id> --format json` on each pending patch; the format of the claim lines in `accept`'s output and `--claims-out` across two runs; the relayed artifact accepted into a second keyed repository and `sync seal --claims`-ed under its own key; whether `sync pending` ever disagrees with `verify` | Whether the accept event is a race-free unit of a change, and whether the relay works end to end | PK-48 |
| A11 | `archive import` into an **existing** repository (no `--adopt`, then with): what lands where; time and peak RSS | The fetch path of the client helper | PK-34 |

## Definition of done & handback

Done when every row has a result (or an explained inability, with the blocking fact) and the review request
holds the raw logs: a table `ID → result → proposed ledger change`, the fixtures' sizes and how they were
built, every surprise, and the questions it raises for the architect. The architect then updates the ledger,
drafts any letter to prikk, revises RFC 004 from the evidence, and brings its decisions to the owner.
Nothing here authorizes a tag, a publish or a letter.
