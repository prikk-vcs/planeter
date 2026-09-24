# planeter — the prikk upstream: correspondence and re-baseline

| | |
|---|---|
| Document | How planeter depends on prikk operationally: the correspondence protocol with the prikk team, the rule that every reply is folded into the records, and the per-prikk-release re-baseline procedure. |
| As of | 2026-09-24 |
| Basis | ROADMAP §Dependencies and §Release cycles ("re-baseline per prikk release", owner-confirmed 2026-09-23); the dependency ledger (`src/planeter-prikk-dependency-ledger.md`); RFC 001 D-3 (version pin), D-2 (exit-status keyed driver). |

## The relationship

planeter builds on prikk's **stable CLI surface** (`--format json` read verbs; `bundle`/`sync`
artifact exchange), never its crate. prikk is the source of truth for every repository fact; planeter
re-derives and never caches a security display. prikk moves fast pre-1.0 (0.43.0 → 0.46.0 in six days)
and its JSON is explicitly *not yet* stable (RFC 152 §5), so planeter pins a floor and re-baselines at
every prikk release.

## Correspondence protocol

- Letters are **private** files under `.git-exclude/upstream/prikk/`: `send/draft/` (architect drafts),
  `send/` (the owner adjusted and sent it — the file moves up), `receive/` (replies and notices as
  received). Names are `YYYY-MM-DD-<subject>.md`.
- **The architect drafts; the owner conveys.** No letter leaves without the owner. A draft states what
  planeter measured, what it asks, and the planning assumptions behind the ask, with numbers.
- **Every received reply is folded into the committed records before any code changes:** new or updated
  rows in the dependency ledger (a `PK-nn` id, status CONFIRMED / RULED / ASSUMED, the evidence), the
  ROADMAP §Dependencies and the milestone table when a schedule or ruling moved, and the affected
  handoff's hold note. The ledger's version line names the letter. The private letters are the source;
  the ledger is the record a new reader can rely on.
- Open threads with prikk today: the fetch planning numbers (draft awaiting the owner); prikk's
  invitation for planeter's input on the RFC 158 Stage B streaming-bound design (0.48.0); the RFC 155
  all-or-nothing import round (R4), where planeter's input is invited.

## Re-baseline procedure (each prikk release)

1. **Read the release** (notes, letter, changelog): list what touches planeter — verbs, JSON, exit
   codes, formats, security fixes, environment variables.
2. **Install the binary** (`cargo install prikk --version X.Y.Z --locked`; the owner does this on the
   development host) and record `prikk --version`.
3. **Run the version-pinned suites with it on PATH** (`planeter-prikk`, `planeter-core`, the web
   tests): the driver refuses an unexpected `schema_version`, so drift fails loudly (PK-18). Then run
   the suite once more with prikk shimmed off PATH — that is what CI sees.
4. **Decide the floor.** Bump `MIN_PRIKK_VERSION` only when the release carries a verb planeter needs
   or a security/correctness fix that matters to a forge (0.44.0's import corruption fix and 0.46.0's
   read verbs did; a release that changes nothing planeter reads does not). The three floors move
   together: `cli.rs`, `release.yml`, `Dockerfile`; every "≥ x.y.z" mention follows (README, RELEASING,
   ROADMAP, STATUS). A floor bump is release-relevant: say so in the review request; the owner decides
   the version.
5. **Record**: ledger version bump with the outcome per affected row; STATUS "pending upstream" table
   advanced; a hold note on any handoff whose assumptions the release changed.
6. **Format changes are a deployment rule, not code**: a serving binary stays at or above the on-disk
   format its hosted repositories carry (format 7 since 0.45.0; format 8 announced for 0.48.0);
   planeter never upgrades a hosted repository's format on its own.

The first instance under the new team is 0.47.0:
[`../rfcs/handoffs/interim/prikk-0-47-0-rebaseline-handoff-v1.md`](../rfcs/handoffs/interim/prikk-0-47-0-rebaseline-handoff-v1.md).
