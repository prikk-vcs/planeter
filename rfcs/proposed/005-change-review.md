# RFC 005 — Change proposal and review (second edition)

**Status.** Proposed — **second edition, drafted 2026-10-15** for the owner's reading; the implementer must
not start. It replaces the body of the 2026-09-15 acceptance (v1 is in this path's git history; its
2026-09-16 revision preface is folded in below). **Issues (CAP-4), bundled in v1, are split out** (see
§Scope). This RFC is not accepted until the unconfirmed upstream rows named in §Upstream facts are confirmed
or the owner accepts that risk in writing (RFC 010 D-8).
Handoff: v1 ([`../handoffs/005-change-review/change-review-handoff-v1.md`](../handoffs/005-change-review/change-review-handoff-v1.md))
is superseded; **handoff v2 is written on acceptance.**
**Tracks.** ROADMAP Phase A3 → M3. Requirements `CAP-5`, `SEC-1`, `NG-3`, `NG-6`; external design
`WEB-02`, `WEB-06`, `FL-04`, `FL-08`; internal design `WR-4`, `DM-2`, `DM-4`, `RD-3`; threat model `T-1`,
`T-3`, `T-9`, `T-10`, `INV-1`, `INV-2`, `INV-6`; RFC 002 (`authorize()`), RFC 003 (re-derivation), RFC 004 v2
(accept and retained artifacts), RFC 006 v2 (checks, D-C6 accepted), RFC 008 and the owner's decision D-8
(how a branch is published); ledger PK-2, PK-28, PK-34, PK-35, PK-46, PK-48, PK-49, PK-50.
**Touches.** `planeter-core` (the change and review model), `planeter-store` (change, revision and review
records), `planeter-web` (review UI and API), `planeter-transport` (the retained-artifact relay). **Not
here:** how a published branch comes to contain a change (publication/merge: RFC 008 and decision D-8), CI
execution (RFC 006), issues, notifications delivery, forks.

## Summary

An **open change is exactly what prikk already holds**: patches a keyless `sync accept` has stored and not
yet reached by any block. The forge adds *only* review state around it, keyed by prikk ids, and never copies
history (NG-6). What v1 did not know, and this edition builds on, is how little of that structure prikk
exposes: a **claim cannot be read** by any verb, **`sync pending` lists patch ids only (text)**, and the
only reliable grouping of patches into "one proposal" is **the accept event itself**, which planeter
serializes under its own lease. So v2 makes the *accept event* the unit of a change, keeps every accepted
artifact verbatim so that a maintainer can take it into their own repository, and refuses to say anything
about a change that prikk cannot be asked to confirm. Honesty is unchanged and now structural: a change is
"held by the forge, unsealed, not prikk-verified" until a published tip reaches it, and a result or approval
belongs to the exact revision and base it was given for.

## Upstream facts (what this design stands on)

Measured by the architect on prikk 0.52.0, scratch directory, keyless receiver, 2026-10-15
(`.git-exclude/reviewed/` note; the transport spike repeats and extends them):

| ID | Fact | Label | Ledger |
|---|---|---|---|
| U-1 | `sync accept` prints one claim id per claim and, with `--claims-out`, writes them to a file; it reports each claim *Unverifiable* when the signer's key is not adopted ("its order is unattributed"). | V (measured) | PK-48 |
| U-2 | `sync pending` prints the pending patch ids as text; `--format json` is refused. It lists no claim, parent, author or grouping. | V | PK-48 |
| U-3 | `show <patch-id> --format json` works on a pending patch (`show-report-v1`, key `patches`); `show <claim-id>` is refused ("requires a Block or Patch id"). | V | PK-48 |
| U-4 | On a keyless forge with only a received ref, `sync have heads/main` answers with an empty have-list; `sync have remotes/heads/main` is refused ("ref namespace is reserved"). A pusher therefore re-sends history the forge holds only as received or pending; accept is idempotent. | V | PK-49 |
| U-5 | `verify` exits 1 ("publication-trust issues") while the keys that signed received or pending history are not adopted; adopting a public key (`trust maintainer add`) is the receiver's own trust decision. | V | PK-50 |
| U-6 | A claim names the block it proposes and its parents; those blocks may be absent from the store. A patch is content-addressed and immutable. | D | PK-2 |
| U-7 | A keyed receiver can `sync accept` an artifact built elsewhere and `sync seal --claims <file>` it under its own key; nothing about this needs the forge. | D (sync guide) | PK-2 |
| U-8 | prikk is considering a read-only command whose point may be **one pending accepted claim** (files = its parent's files plus its patches); 0.54.0 behind a design round, not promised. | D | PK-46 |

**Not yet confirmed** (RFC 010 D-8): that the accept output's claim lines are stable enough to parse (text);
that pending-before/pending-after is a race-free derivation under the lease across idempotent re-accept and
interrupted accept; that U-7 works end to end with an artifact relayed from the forge. These are rows A12 of
the transport measurement handoff.

## Goals

- **G1.** A contributor can propose work and see exactly what the forge holds of it.
- **G2.** A reviewer can read the proposed change, comment, and record a decision, bound to the revision and
  base they actually reviewed.
- **G3.** A maintainer can take a change into their own repository, byte-identical to what was pushed.
- **G4.** At every step the forge tells the truth: unsealed is not verified; forge-approved is not
  prikk-verified; a result is never carried to a base it did not run on.
- **G5.** The model is correct on today's prikk and does not wait for RFC 154 or the delta archive.

## Non-goals and scope

- **Not here — publication/merge.** Whether `main` advances by a maintainer's sealed bundle ingested as a
  received ref and chosen by rule, by RFC 154's act, or otherwise is **decision D-8** and RFC 008. This RFC
  defines only the *observation* of landing (§D-6) and the artifact relay a maintainer needs.
- **Not here — issues (CAP-4).** v1 D-8 bundled them with review. They share a comment machinery but no
  prikk coupling and no dependency on publication; they can ship independently. **Issues are split out to a
  separate RFC, to be numbered when the owner places it on the roadmap.** v1 D-8 is kept as input, undecided.
- **Not here — forks, mirrors, notifications, search, suggested changes, draft changes, boards, wikis.** The
  feature edge (OQ-4) is the owner's; §Open questions states the default and what each extension would add.
- **Not a second merge.** The forge never computes "what main would look like with this change" (D-C6, and
  prikk's own stance in its 2026-10-15 letter).

## Decisions

- **D-1 — A change is one accept event, and its record is made under the write lease.** On a successful
  `sync accept` (RFC 004 v2 lease class W), planeter records a **change revision**:
  `{ revision_id, change_id, repo, artifact_digest, claim_ids (from --claims-out), patch_ids (the pending set
  after minus the pending set before), claim outcomes (as reported), submitter principal, author key ids,
  base (D-3), created_at }`. Nothing about history is copied (NG-6); ids only. Re-accepting an identical
  artifact (same digest) finds the existing revision and creates nothing. An interrupted accept records
  nothing and is retried (accept is idempotent, ledger PK-28).
- **D-2 — A change is a series of immutable revisions.** A push names an existing `change_id` to add a
  revision, or none to open a new change. A change has `target` (the ref name the submitter intends, advisory
  only), a title and a description (Markdown, rendered through the RFC 003 sanitizer), and a state:
  `open`, `landed` (D-6) or `closed` (withdrawn or rejected, with a reason). Nothing is deleted: a closed
  change keeps its record, and **its bytes remain in the store** (no reclamation exists, ledger PK-41/E11);
  the UI says so.
- **D-3 — Base: declared by the client, verified as held.** The client helper states the block id it built
  against (it knows it; claims' parents are unreadable by the forge). The forge accepts a revision only if
  `show <base>` succeeds, i.e. the base is held (published or received); otherwise it refuses with a named
  reason ("base unknown: push the base first"). A revision with no declared base records `base: none` and is
  reviewable but its results never count (D-7). A *declaration* is not a fact about the artifact; the UI
  labels it "declared base".
- **D-4 — The review view is patch-level and re-derived.** For each patch in `patch_ids`, the diff is
  prikk's own `show <patch-id> --format json` rendered through RFC 003 (sanitized, honest). Because patch
  ids are content-addressed and immutable, caching by patch id is correct forever and does not conflict with
  INV-6. A file-level tree view is added behind the same `ChangeView` interface **when** prikk ships its
  read-only command with a claim as a point (U-8); until then the UI shows per-patch effects and says what
  it cannot show.
- **D-5 — Review state is forge-owned and bound to a revision.** Comments (anchored to a patch id and, where
  prikk's view carries one, a path and span), threads, and a decision per reviewer per revision
  (`approve` | `request-changes`), authorized by RFC 002's existing `comment` and `review` actions. A new
  revision does **not** carry approvals over: they are shown on the revision they were given for, and a
  later revision shows "approved on revision N". This keeps "approved" from silently covering code the
  reviewer never saw.
- **D-6 — Landing is observed, never asserted.** A change is `landed` when every patch id of its latest
  revision has left the repository's pending set (they are now reachable from a block: `sync pending` no
  longer lists them), re-derived from prikk. The forge never marks a change landed by its own say-so, and
  records the first observation point (the ref or received ref that reaches them, where prikk reports it).
- **D-7 — Checks and approvals are bound to (revision, base) and stale never counts.** As the owner
  accepted for RFC 006 (D-C6): a result or approval belongs to the exact (revision, declared base) it was
  given for; when the base is no longer the target's tip it is shown stale and is never counted toward any
  gate; there is no merge simulation; one status per change, with the base label part of it.
- **D-8 — The accepted artifact is retained and relayed verbatim.** The forge keeps each accepted artifact
  (content-addressed by digest, immutable, inside a quota; retention while the change is `open`, plus a
  stated grace after `landed` or `closed`) and serves it, to principals holding `review` or `merge-seal`, as
  "download this change". A maintainer's own prikk then `sync accept`s it and, if they decide to merge,
  `sync seal --claims`s it under their own key (U-7): the forge neither interprets nor rewrites it (RFC 004
  D-1) and holds no key. **This is the one place a pushed artifact is served back out**, so it passes the same
  serve rules as other artifacts (RFC 004 v2 S1: spool, never streamed under a lock).
- **D-9 — Who may propose, and how much (owner decision D-9 in SCHEDULE).** A new repository-level action
  **`propose`** gates `sync accept`; it is granted by default to `write` and above, never to anonymous, and a
  repository setting `proposals: members | any-authenticated` (default `members`) can open it to any
  authenticated principal. **Quotas are mandatory:** per principal per repository, accepted bytes, claim
  count and open changes; a refusal names the quota. The quotas exist because an accepted patch cannot be
  removed from a store (PK-41/E11). A change from a principal without `write` is an *untrusted change* for
  CI (RFC 006 C8): no secrets, a maintainer's approval to run.
- **D-10 — Honesty is a type, not a style.** The change view model carries an `Assurance` for every item:
  `ForgeHeldUnsealed` (default), `SignatureOutcome { adopted | not-adopted }` per claim as accept reported it
  (stored, not re-asserted), and `PrikkVerified` only for objects that a published or adopted tip reaches and
  `verify` confirms. A forge approval is never rendered as, or convertible to, `PrikkVerified` (INV-1).
  Because `verify` fails while signer keys are not adopted (U-5), the page shows **both** the repository's
  verify state and which keys are adopted, and offers adoption only to `admin` (a trust decision).

- **D-11 — Presentation: less is more, advanced on demand** (owner philosophy). A change's default view shows
  only: the title, who proposed it, its state (*Open*, *Landed*, *Closed*), a one-line summary of what
  changed, the review state (*awaiting review*, *approved*, *changes requested*) and the checks status, each
  with its plain-language meaning on hover or in a line of text. Everything else is one click away under
  **Details**: revisions, the patches, the claim signature outcomes, key ids, the declared base, retained
  artifact digests. Wording uses the user's words ("built on", not "declared base", in the default view);
  the honest qualifiers of D-10 are never removed, only stated in plain language ("held for review: not yet
  part of the project's history"). No page offers an action whose consequence the page does not state.

## Alternatives considered

- **A change per claim.** Rejected: claims are unreadable (U-3), so there is no per-claim data to show.
- **A change as "a branch of pending patches" derived from parent relations.** Rejected: parents are not
  exposed (U-2, U-3); deriving them would need an unconfirmed text format.
- **Carry approvals across revisions when the diff is "small".** Rejected: needs a notion of equivalence
  prikk does not give us, and the failure mode is approving unseen code.
- **Compute the merged-onto-`main` tree for a change.** Rejected (D-7; prikk's letter): a second, silent merge.
- **Keep issues in this RFC.** Rejected: unrelated decisions in one unit; issues need none of the above.
- **Re-serve the change from the archive.** Rejected for D-8: the archive is the whole repository; the
  contributor's artifact is exactly what the maintainer's prikk expects.

## Security considerations

New inbound flow with stored consequences (T-3, T-9): quotas (D-9) and the lease (RFC 004) bound it; stored
Markdown is untrusted and rendered only through the sanitizer (T-6); the retained artifact is untrusted data
served as an attachment from the isolated origin; an approver's decision is bound to a revision (D-5) so a
force-update cannot inherit trust; the adoption of keys (D-10) is `admin`-only because it widens what the
forge treats as attributable. INV-1 and INV-2 hold: nothing here signs, seals or publishes.

## Compatibility and migration

Additive: new records in `planeter-store`, new routes. Behind RFC 004 v2; until transport ships there are
no changes to record. v1's handoff is superseded, not migrated (no v1 code exists). If prikk later exposes
claims (JSON for `sync pending`/`accept`, a claim as a point), D-1 and D-4 get better inputs without a model
change; that is the intent of keeping the provider interface.

## Operational considerations

Change records and retained artifacts grow monotonically; a retention and quota policy is required before
M3 (OPS-5 metering applies). Observability: counters for revisions, refusals by reason, bytes retained.

## Testing and verification

- **Model tests (pure):** revision derivation from (pending-before, pending-after); idempotent re-accept;
  approvals not carried; staleness; landing observation from a recorded pending sequence.
- **Authorization tests:** `propose` default-deny; setting `any-authenticated`; quotas; reviewers cannot
  approve their own revision's artifact retention beyond quota.
- **Honesty tests:** an open change never renders `PrikkVerified`; an approval never renders as verification.
- **Real-binary tests (prikk on `PATH`, skipped visibly otherwise):** accept → revision → `show` per patch →
  relay → second repository accepts the relayed artifact; the stand-in is allowed only for text formats.
- **Hostile input:** a Markdown title/description with script/handler/URL attacks; an artifact over bound;
  a base id that is not held.

## Acceptance criteria

A principal with `propose` pushes an artifact and sees a change with one revision, its patches rendered, its
claim outcomes and a "declared base"; a second push with the same `change_id` adds revision 2 and approvals
on revision 1 are shown as such; closing keeps the record; a maintainer downloads the artifact and a second
repository accepts it byte-identically; the change shows `landed` only after prikk reports the patches no
longer pending; nothing in the product can seal, merge or publish (ENF-2 passes); quotas refuse with named
reasons; the threat model is updated in the same release.

## Implementation boundaries

`planeter-core` (model, `ChangeView` provider interface), `planeter-store` (records, quotas), `planeter-web`
(UI/API), `planeter-transport` (the artifact relay) over RFC 004 v2. **Not** `planeter-prikk` beyond the
read verbs already present (`show`, `sync pending` text reader, `sync accept` output reader); **not** any
publication code.

## Release implications

M3's review half can ship without publication; "merge" is whatever D-8 settles. A release containing this
adds a new inbound flow and a new action: threat model v0.4 and a security audit pass (project rule).

## Open questions

- **Q-1 (owner, OQ-4).** The review feature edge. **Default in this draft:** the core set above only.
  Suggested changes, draft changes, review-thread resolution, required-reviewer lists and boards would each
  be additions behind this model; none changes D-1…D-7.
- **Q-2 (owner, D-9).** `proposals: members` as the default; quota values (set from measurement, not here).
- **Q-3 (architect → prikk).** Asks for a later letter: JSON for `sync pending` and `sync accept` (claim →
  patch ids, parents, signers) and a claim as a point for the read-only command. The transport measurement
  handoff row A12 supplies the evidence.
- **Q-4 (owner, D-8).** Publication. Nothing in D-1…D-10 depends on its answer; "landed" is observed.

## Sequencing and handoff

Needs RFC 004 v2 (the lease, the accept edge, the artifact store) and the owner's D-9; independent of D-8.
On acceptance the architect writes handoff v2 (data model and migrations, the accept-event derivation, the
review API, the relay, the honesty tests). Issues follow as their own RFC and handoff. All v0.x; 1.0 is the
owner's gate.
