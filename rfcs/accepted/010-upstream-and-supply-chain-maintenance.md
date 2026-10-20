# RFC 010 — Upstream tracking and supply-chain maintenance: staying correct while prikk and our dependencies move

**Status.** Accepted (2026-10-15) by the owner. The design is settled and the implementer may build against it
per the handoffs. Proposed by the architect on the owner's direction that work without an RFC of its own must
be given one; the `rfcs/handoffs/interim/` convention is retired by this RFC.
Handoffs: [`../handoffs/010-upstream-and-supply-chain-maintenance/`](../handoffs/010-upstream-and-supply-chain-maintenance/).
**Tracks.** ROADMAP §Release cycles ("planeter re-baselines against each prikk release", owner-confirmed
2026-09-23) and §Dependencies; SCHEDULE T1, C-4, C-11, C-15; requirements INT-1, INT-5, OPS-1, OPS-6,
SEC-3, SEC-4; threat model T-11, C-5d, C-11, INV-7; RFC 001 D-3 (version pin); RFC 009 (format rule);
issue register IS-1, IS-17, IS-21, IS-22.
**Touches.** The CI and release workflows, `tools/` (two new gates), the `planeter` binary's argument
handling (`--version`), `docs/UPSTREAM.md` and `docs/dependency-policy.md`. **Not here:** hosting,
authorization, the read path, transport and every user-visible capability; no change to a public API, a
persistent format or a security boundary.

## Summary

planeter's correctness depends on three things it does not control: **prikk** (pre-1.0, releasing every few
days, JSON schemas and exit codes not yet stable, PK-18), its **crate dependencies** (advisories, yanks,
licences) and its own **build chain** (actions, base image, toolchain). Until now the duties were real but
scattered: a sentence in ROADMAP, a procedure in `docs/UPSTREAM.md`, rules in `docs/dependency-policy.md`,
a control in the threat model, and handoffs housed outside any RFC. Three weeks of operation show where
procedure alone failed (F1–F4 below). This RFC makes the duties **decisions with structural enforcement**:
a fixed re-baseline procedure, a single checked floor, a supply-chain gate that also runs without a code
change, a pinned build chain, a running build that can identify itself, and a rule for how upstream
uncertainty is measured before a design leans on it.

## Evidence (verified facts)

- **F1.** CI runs only on push and pull request (`ci.yml`). Advisories arrive without a code change. On
  2026-10-15 `cargo audit` was red (two `ammonia` advisories, one yanked crate) and nobody knew until a
  re-baseline review ran the gates by hand.
- **F2.** The prikk floor is written in three places (`MIN_PRIKK_VERSION` in `cli.rs`, `PRIKK_VERSION` in
  `release.yml`, `ARG PRIKK_VERSION` in `Dockerfile`) and moved together only by procedure; nothing checks it.
- **F3.** `release.yml` and `ci.yml` pin third-party actions by mutable tag (`@v7`, `@stable`, …), contrary
  to threat model C-5d/C-11, on the workflow that produces the attested artifacts. The base image is
  `debian:bookworm-slim` by tag.
- **F4.** The `planeter` binary has no `--version`; an unrecognised argument starts the server (IS-1: an
  install check once did exactly that). A running build cannot identify itself.
- **F5.** Re-baselines have been done against 0.43.0, 0.44.0–0.46.0 and 0.52.0 within weeks, each under a
  handoff that had no governing RFC.
- **F6.** prikk 0.52.0 changed `trust maintainer check`'s **exit code** without changing its
  `schema_version`. A driver that keys on exit status is exposed to contract changes that a schema check
  cannot see (ledger PK-42, IS-21).
- **F7.** Our own review of 2026-10-15 tested the advisory shapes against the locked sanitizer and found them
  unreachable through the default configuration. That test was ad hoc; the evidence was not kept.

## Goals

- **G1.** Every prikk release the owner installs is assessed by one procedure with fixed outputs.
- **G2.** The floor has one source and a gate that fails on disagreement.
- **G3.** An advisory, a yank or a licence change is caught **without** a code change, triaged by one ladder,
  and its evidence is kept.
- **G4.** Our build chain is pinned by digest and a gate fails on a mutable reference.
- **G5.** A running or installed build identifies itself and never starts a server by accident.
- **G6.** A design is not accepted on upstream behaviour that is only assumed unless the owner accepts that
  risk in writing.

## Non-goals

Automatic dependency bots; vendoring; patches to prikk (NG-2); SBOM or provenance *generation* for our own
releases (a later RFC, aligned with forge-commons *Integrity*); runtime self-update; calendar-driven
cadences for planning (the CI schedule in D-3 is a mechanism, not a plan).

## Decisions

- **D-1 — Re-baselining is a standing procedure with five outputs.** For every prikk release the owner
  installs: (1) the release is read and the touched surfaces listed (verbs, JSON, **exit codes**, formats,
  security fixes, environment variables); (2) the driver and read-path suites run with the real binary on
  `PATH` **and** once with prikk genuinely off `PATH` (the shim uses the real toolchain binaries; a skip is
  printed, never silent); (3) a floor recommendation, the version being the owner's; (4) the dependency
  ledger is bumped with an outcome per affected row; (5) STATUS's pending-upstream table and any hold note
  are advanced. `docs/UPSTREAM.md` remains the procedure text; this RFC is its authority. An instance is a
  handoff under this RFC.
- **D-2 — One floor, checked.** `MIN_PRIKK_VERSION` is the source. A gate, `tools/floor-check`, asserts that
  `release.yml` and `Dockerfile` carry the same version and fails naming the mismatch; it runs in CI and in
  the release gates. (Generating the other two was considered and rejected: a Docker `ARG` and an Actions
  `env` are not reliably derivable from a Rust constant without a build script.)
- **D-3 — The supply-chain gate runs at every project event, not on a clock.** *(Amended 2026-10-15 at the
  owner's direction: the project has no calendar cadence, so no cron.)* `cargo deny check` and `cargo audit`
  must exit 0 at each of these **events**: (a) every push and pull request (CI, exists); (b) every release
  (the release gates, exists); (c) every **prikk re-baseline** (a D-1 output); (d) every **RFC disposition
  checkpoint** (an RFC accepted, done, superseded, withdrawn or deferred: the workflow's recurring control
  point, §6.4), where the architect runs both against a freshly fetched advisory database; (e) whenever the
  owner asks. The result is recorded as one line in STATUS ("supply chain: green|red as of <event>"); red
  becomes an issue-register entry. A manually dispatched workflow (`workflow_dispatch` only, no schedule) lets
  any checkpoint leave a durable CI record. **What this does not do, stated plainly:** it detects an advisory
  at the next event, not during a quiet period; "prompt patch release" means *prompt once found*.
- **D-4 — One triage ladder, with evidence.** In order: (1) a lock-only update within the allowed range;
  (2) a manifest upgrade, measured under dependency-policy rule 1; (3) replace or remove the crate; (4) a
  scoped, justified ignore in both `deny.toml` and `.cargo/audit.toml` (rule 3) **only** when no fix exists
  and unreachability is demonstrated. Each triage records the crate path, reachability and how it was
  shown, the decision, and the evidence, in the dependency-policy ledger. For a crate that parses or renders
  untrusted input (today: `ammonia`, `pulldown-cmark`, `jsonwebtoken`, `askama`) the advisory shape is pinned
  by a regression test under `tests/`. A *reachable* advisory triggers an out-of-band patch release (the
  existing ROADMAP rule); an unreachable one is still fixed at once, because the gates are release gates.
- **D-5 — The build chain is pinned by digest, and checked.** Every `uses:` references a full commit SHA
  with its human tag in a comment; the base image is referenced by digest; the Rust toolchain is an explicit
  version, not `@stable`. A gate, `tools/pin-check`, fails on a mutable reference in `.github/workflows` or
  the `Dockerfile`. *(Amended 2026-10-15 on the dev team's finding, IS-26: CI and release use the **same**
  pinned toolchain, and runner images are named by a fixed label such as `ubuntu-24.04`, never `-latest`;
  `pin-check` also fails on a second toolchain version or a `-latest` runner.)* Updating a pin is a reviewed change that names the upstream diff.
- **D-6 — Build identity.** `planeter --version` and `-V` print `planeter X.Y.Z`, the commit the build
  was made from (`unknown` when built without it, as from crates.io), and the prikk floor, then exit 0
  **without reading configuration or starting the server**. `--help` prints usage. Any other argument is
  refused with usage and exit 2. The commit is injected through an environment variable at build time
  (`option_env!`), **not** a build script (no new code runs at build, no git dependency for
  `cargo install`). The release's `build-info.txt` carries the same fields.
- **D-7 — Upstream contracts are tested, including exit statuses.** For every prikk surface the driver reads,
  tests assert the `schema_version`, the deserialized field set and the **exit-status contract**, on the
  real binary; a recorded stand-in is allowed only for a version that is no longer installable and is
  labelled as such. Each re-baseline adds or updates them (closes F6).
- **D-8 — Measure before designing to it.** A design (RFC or revision) that depends on upstream behaviour
  not yet **CONFIRMED** in the ledger says so; such an RFC is not accepted until the rows are confirmed or
  the owner accepts the risk in writing. Measurements are requested as handoffs **under the RFC being
  revised** (workflow Phases 1–2: limited investigation and small proofs of concept when requested), run in
  a scratch directory with no repository change, and their output is ledger rows.

### Proposed amendment (2026-10-20): a release chain that is rehearsed and trusts few outside parties (IS-35)

*Status: **accepted by the owner 2026-10-20** (O-4 and O-5). Proposed by the architect on the owner's direction to
take "a stable and secure approach" with a view to long-term investment. It amends D-5 and adds D-9 to D-11.
Handoffs: `release-rehearsal-handoff-v1.md` (D-10, on the current chain, first), then the D-11 simplification.*

**Evidence.** Five third-party actions (`docker/setup-qemu`, `setup-buildx`, `login`, `build-push`; GitHub's own
`attest-build-provenance`) lag upstream by one or two majors, and `dtolnay/rust-toolchain` is a sixth outside
dependency maintained by one person. None has a published advisory. The release steps they serve (registry
login and push, attestations, `gh release create`) are the ones a dry run cannot exercise; the first time they run
is a real tag. We know the failure mode is survivable (0.3.0's first tag failed before publishing anything) but
a failure after the registry push would leave a half-published release. Measured fact: the arm64 image is built
under QEMU emulation, although a native arm runner already builds the arm64 binary.

- **D-9 — A pin is updated by a procedure with outputs.** Triggers are the project events of D-3 (a release
  candidate, an advisory or deprecation notice that names a pin, the owner's request). For each pin changed the
  review records: old and new tag and **full SHA**; that the SHA is the tagged commit **in the upstream
  repository** (not a fork or an unreachable commit) and whether the tag's commit is marked verified; the
  upstream release notes or diff read and summarized (renamed inputs, runtime changes, new permissions); and one
  action changed per change. `pin-check` stays green, and D-10's rehearsal is green before the change lands.
- **D-10 — A release rehearsal exercises what a dry run cannot.** A `workflow_dispatch` mode `rehearsal` of
  `release.yml` builds both tarballs and the image, **logs in, pushes and attests**, but only to a throwaway
  package `ghcr.io/prikk-vcs/planeter-rehearsal` (one tag, overwritten each time), and never creates a GitHub
  release or tag, never touches `ghcr.io/prikk-vcs/planeter` or `latest`, and never touches crates.io. It is
  refused off `main`. `pin-check` fails if any push target in the rehearsal path is not the rehearsal package. It
  is run (and its run id recorded) before any change to the actions or the Dockerfile of the release chain and
  before each release candidate whose chain changed. The dry run (publishes nothing at all) stays.
  *Footprint, stated:* one additional private package; attestation records for the rehearsal artifacts, which
  for a public repository live in the public transparency log permanently and are labelled `rehearsal` in their
  subject name.
- **D-11 — The release chain trusts as few outside parties as possible.** Target state: the toolchain is
  installed by the runner's own `rustup` at the pinned version (`pin-check` checks the version in that command as
  it checks `toolchain:` today); each image architecture is built **natively** (the existing arm runner builds
  arm64) and a multi-arch manifest is pushed with the runner's `docker`/`buildx` commands; login is `docker login`
  with the job token on standard input; QEMU is gone. The only actions left are GitHub's own (`checkout`,
  `upload-artifact`, `download-artifact`, and the attestation action, moved to its current form). `pin-check`
  fails on any `uses:` outside the `actions/` owner. Result: the five lagging pins and the toolchain action
  disappear instead of being maintained, the build is faster and no longer emulated, and the remaining pins are
  GitHub's. The change is made behind D-10: rehearsal first on the current chain (a baseline), then each change.
  *Unknown, to be measured in the first handoff:* that the arm runner has `docker`/`buildx`, that the job token may
  create the rehearsal package, and the exact current form of the attestation action.

**Alternatives.** (A) Update the five pins one by one and keep the structure: smaller change now, five outside
parties maintained forever, QEMU kept. (B) Do nothing: the pins age and the first test of a newer action is a real
tag. (C) A scheduled bot (Dependabot) opening update PRs: rejected, the project has no calendar cadence (D-3).
(D) Rehearse in a personal fork: a public fork gives public attestation records under another owner's name; a
private fork may not support attestations; rejected in favour of the same repository with a separate package.

**Risks.** Shell in a workflow is new code to review (small, in the repository, exercised by D-10); a rehearsal
leaves permanent attestation records (labelled, accepted by the owner or the rehearsal does not attest); the
native build depends on the arm runner staying available (the binary build already does).

**Owner decisions.** **O-4:** accept D-9 to D-11 (the architect recommends this over alternative A, as the stable
long-term position). **O-5:** accept the rehearsal's footprint (one private package, labelled attestation
records). Handoffs follow acceptance: first the rehearsal mode on the current chain (a baseline), then the chain
simplification, each rehearsed.

**Improvement on finding (owner, 2026-10-15).** The gates and their timing are improved whenever an issue
exposes a gap in them: the finding, the gap and the change are recorded in the dependency-policy ledger and
this RFC is amended, as D-3 was on the day it was accepted.

## Alternatives considered

- **A dependency bot** (Dependabot, Renovate). Rejected for now: it adds a write-capable actor and a stream of
  pull requests to a project whose rule 1 wants a human-measured delta for every manifest change. Revisit
  when the lock churn justifies it.
- **cargo-vet or vendoring.** Heavier than the problem; deferred.
- **Keep procedure only.** It is the status quo and it failed F1–F4.
- **A build script to derive the floor and the commit.** Rejected (D-2, D-6): it runs code at build time and
  does not reach the Docker or Actions side.
- **One mega "operations" RFC.** Rejected: backup, observability and multi-replica operation (SCHEDULE T11) are
  product operations with their own design; this RFC is about staying correct against upstream.

## Compatibility and migration

No persistent format, API or protocol changes. `planeter` gains argument refusal: a script that passed an
unknown argument and relied on the server starting will now fail with exit 2 (acceptable pre-1.0; named in
the CHANGELOG). Existing `deny.toml` and `.cargo/audit.toml` are unchanged. Pins introduce a maintenance
duty (D-5): updating them is a deliberate, reviewed act.

## Security considerations

Raises INV-7 (supply chain pinned, minimized, audited) from policy to enforced gates; closes the F3
contradiction with C-5d. The dispatch-only workflow needs no secrets and `contents: read` only. Pinning by SHA
protects against a moved tag but not against a malicious commit at the pinned SHA: reviewing the diff at pin
time is the control (D-5). The `--version` output reveals nothing an operator's `build-info.txt` does not.

## Operational considerations

The checkpoint audit is a step in work that already happens. A red result needs a human; the RFC makes who
reads it explicit (the architect, at each checkpoint). Pin updates are periodic work and a release cannot proceed on a stale
pin only if the gate is red, never silently.

## Testing and verification

- `tools/floor-check` and `tools/pin-check` each have a test that plants a violation and expects failure
  (a mismatched floor; a mutable `uses:`), as the layering gate and ENF-2 do.
- The dispatch-only workflow exists and one manual run on `main` is green and recorded; STATUS carries the
  supply-chain line.
- `planeter --version`, `-V`, `--help` and an unknown argument are tested without starting a server.
- The driver tests of D-7 cover the exit-status contracts named by the ledger (PK-42 today).

## Acceptance criteria

Both gates exist, run in CI and in `release.yml`, and fail on a planted violation. The checkpoint list of D-3 is
written into `docs/dependency-policy.md` and the dispatch-only audit has run green. Every `uses:` and the base image are pinned and `pin-check` passes. `--version`
works as D-6. `docs/UPSTREAM.md` and `docs/dependency-policy.md` cite this RFC as their authority. The
re-baseline instances already done (0.47.0 v1, 0.52.0 v2) and the advisory lock update are housed under
this RFC's handoff directory.

## Implementation boundaries

`tools/floor-check`, `tools/pin-check`, `.github/workflows/`, `Dockerfile`, `crates/planeter/src/main.rs`
(argument handling only) and documentation. **Not** `planeter-core`, `-auth`, `-web`, `-store`; **not** any
manifest except where D-4 requires and then only after measuring.

## Release implications

None of it is a capability. The candidate release 0.3.0 (SCHEDULE D-11) carries the 0.52.0 floor, the
advisory update and these gates; D-6 and D-5 change the release workflow, so a release dry run (the workflow
on a non-release branch with publishing disabled, or the next real tag) is the verification.

## Risks and settled questions

- **R-1 — Pins are immutable, so they do not refresh themselves.** This is a *consequence of D-5*, not an
  observed incident: a pinned action never receives an upstream fix until someone updates it. **Settled
  (architect, minor concretization, reviewable):** immutable pins are accepted without a separate staleness
  gate; instead the release checklist (`docs/RELEASING.md`) gains a step, *"review the pins against upstream
  releases and advisories before tagging"*, so the check is tied to an event (a release), not to a clock.
  `pin-check` therefore fails only on mutable references.
- **R-2 — Detection during a quiet period.** With event triggers an advisory published while the project is
  idle is found at the next event. **Settled by the owner (2026-10-15):** acceptable; the project has no
  calendar and "prompt" is read as "prompt once found". Operators who need earlier warning audit their own
  deployment (SECURITY.md says so). Revisit when planeter has deployments with availability commitments.
- **R-3.** `--version`'s commit comes from an environment variable that a build can omit, so a build can
  claim `unknown`. This is honest and deliberate; the attested release artifacts always carry it.
- **R-4.** D-8 can slow a design when an upstream fact is slow to measure. The owner's written acceptance of
  risk is the release valve.

## Sequencing and handoffs

On acceptance the architect derives these handoffs (their state is inherited from this RFC):
1. **Maintenance gates** — D-2, D-3 (workflow), D-5, and the document updates.
2. **Build identity** — D-6.
3. **Advisory lock update** — an instance of D-4 (exists, awaiting acceptance).
4. **Re-baseline instances** — D-1 (0.47.0 v1 superseded, 0.52.0 v2 implemented and reviewed): housed here
   for the record; future instances follow the same template, including D-7.
