# RFC 005 — Change proposal and review (third edition)

**Status.** **Accepted (2026-10-21) by the owner, with the contribution model (SCHEDULE D-9: the `propose` action, the
`proposals` setting and its default `members`, the quota defaults of Q-2)** — third edition, drafted 2026-10-21. D-8 (publication)
remains the owner's open question and does not block this RFC. Handoffs: [`../handoffs/005-change-review/`](../handoffs/005-change-review/),
sequenced in §Sequencing and handoff. Previously: Proposed — third edition, drafted 2026-10-21 for the owner's reading; the implementer must
not start. It replaces the body of the second edition (2026-10-15; earlier editions are in this path's git
history). **Issues (CAP-4), bundled in v1, are split out** (see §Scope). The upstream rows the second edition
waited for were measured (transport measurement A1–A12, ledger v1.9 PK-57 to PK-61, 2026-10-21); this edition
changes the design where they disagreed: the accept is resumable, not atomic, so the change record is made
from an **intent journal after a complete accept** (D-1); a revision's patch set cannot be recovered from
prikk's output when patches overlap, so it is **declared by the client helper and checked by the forge**
(D-1); a change is shown **file by file from `prikk extract` of the claim and of its parent** (D-4; `show <block-id>` is a prikk defect and `show <patch-id>` cannot name the file of an edit, PK-57, PK-64); and `queued` never
means "unsealed" (D-10, PK-58). It depends on RFC 004 third edition.
Handoff: v1 ([`../handoffs/005-change-review/change-review-handoff-v1.md`](../handoffs/005-change-review/change-review-handoff-v1.md))
is superseded; **handoff v2 is written on acceptance.**
**Tracks.** ROADMAP Phase A3 → M3. Requirements `CAP-5`, `SEC-1`, `NG-3`, `NG-6`; external design
`WEB-02`, `WEB-06`, `FL-04`, `FL-08`; internal design `WR-4`, `DM-2`, `DM-4`, `RD-3`; threat model `T-1`,
`T-3`, `T-9`, `T-10`, `INV-1`, `INV-2`, `INV-6`; RFC 002 (`authorize()`), RFC 003 (re-derivation), RFC 004 v2
(accept and retained artifacts), RFC 006 v2 (checks, D-C6 accepted), RFC 008 and the owner's decision D-8
(how a branch is published); ledger PK-2, PK-28, PK-34, PK-35, PK-46, PK-48, PK-49, PK-50, PK-56–PK-60.
**Touches.** `planeter-core` (the change and review model), `planeter-store` (change, revision and review
records), `planeter-web` (review UI and API), `planeter-transport` (the retained-artifact relay). **Not
here:** how a published branch comes to contain a change (publication/merge: RFC 008 and decision D-8), CI
execution (RFC 006), issues, notifications delivery, forks.

> **Terminology (amended 2026-10-22, D-38).** In everything a person or a program reads, what this RFC calls a *change* is a **proposal** (its versions are *revisions*): the owner-approved glossary already gives the word *change* to a recorded unit of history (prikk's block), and one word for two things is exactly the confusion the project forbids. The RFC's text keeps *change* as its internal name; the product, the API, the catalog and the docs say *proposal*. Nothing was released under the old word.

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

U-1…U-5 measured by the architect on prikk 0.52.0, keyless receiver, 2026-10-15; U-9…U-13 measured by the
transport measurement (dev team, prikk 0.53.0 with 0.52.0 side by side, 2026-10-21; rows A6, A7, A12):

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

| U-9 | **`prikk show <block-id>` fails for a block that edits a file for the third time** (integrity error from a lifecycle replay) while `verify` is clean; prikk confirmed it as its own defect (fix proposed for 0.54.0). `show <patch-id>` works but **names the file by an internal node id for a text edit** (renames and deletes carry their paths). So neither form can show which file a pending text edit changes. | M, R (prikk) | PK-57, PK-64 |
| U-10 | **`show <patch-id>` reports `queued: false` for an accepted-but-unsealed patch.** `queued` means "in this repository's own active queue"; pending versus sealed comes from `sync pending` and `log`, never from `queued`. | M | PK-58 |
| U-11 | **An interrupted `sync accept` is not atomic** (SIGKILL at 15/40/70/90 % leaves 147/397/706/902 of 1,000 patches pending, locks in place, `verify` and `doctor` clean); a retry resumes and converges. | M | PK-59 |
| U-12 | **The accept event's output is stable text and complete for the artifact, not for what is new:** a second push of 5 patches, 3 new, gives pending 5, new objects 3, claims file with all claim lines (new first), order stable; a re-accept is idempotent (claims file byte-identical). **Claim lines cannot be mapped to patch ids** (`show <claim-id>` is refused), so the set of patches *inside* an artifact is not recoverable from prikk's output when some are already pending. | M | PK-48, PK-59 |
| U-13 | **The relay works end to end (A12c):** an artifact accepted into a second, keyed repository can be sealed with `sync seal --claims <file>`: with the pusher's key adopted the claims are `Sound`, each claim becomes its own block, `verify` is clean; without the adoption the seal still succeeds but creates a `Root` block, `verify` exits 1 (`publication-trust`) and the received history stays pending. | M | PK-48 |

**Confirmed since the second edition:** the accept output's claim lines are parseable and deterministic (U-12);
the relay into a second repository works (U-13). **Refuted:** that the accept event is a race-free unit
(U-11): derivation from pending-before/after is sound only after a **complete** accept (D-1). **Open, to prikk
(letter drafted):** JSON for `sync accept`/`sync pending` (claim → patch ids), a documented meaning of
`queued`, and the `show <block-id>` defect.

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

- **D-1 — A change is built from complete accept events, recorded from an intent journal.** RFC 004 D-3
  writes an **intent record** (`artifact digest`, `pending-before`) *before* `sync accept` and closes it after a
  **complete** run (exit 0). Only then does planeter record a **change revision** — in one store transaction that
  also closes the intent:
  `{ revision_id, change_id, repo, artifact_digest, claim_ids (from --claims-out), added_patch_ids
  (pending-after minus the intent's pending-before), declared_patch_ids (below), claim outcomes (as reported),
  submitter principal, author key ids, base (D-3), created_at }`. Nothing about history is copied (NG-6); ids
  only. Re-accepting an identical artifact (same digest) finds the existing revision and creates nothing; an
  accept that adds nothing new records nothing and tells the submitter so. **A killed or timed-out accept
  records nothing** (U-11): its intent stays open, the repository is `needs-recovery` (RFC 004 D-6), and the
  retry of the same artifact completes it; derivation always uses the *first* attempt's pending-before.
  - **The patch set of a revision.** `added_patch_ids` is what this accept newly stored. It is **not** the
    artifact's patch set when some of its patches were already pending (U-12): a revision that amends or
    extends a series carries its earlier patches again. The client helper therefore **declares** the full
    list of patch ids its artifact contains (it computes them from its own `log` of the blocks since the
    declared base, ids only), and the forge **checks the declaration**: every declared id must be held (pending
    or reachable) after the accept, and `declared ⊇ added`. A declaration that fails either check is refused with
    a named reason. The revision's patch set is `declared_patch_ids`; the UI labels it "as declared by the
    submitter and found on the forge". A push by plain prikk (no helper) has no declaration: the revision is
    recorded with `added_patch_ids` only and the UI says "patches added by this push; earlier patches of the
    same series, if any, are not listed" — it never claims completeness it does not have.
  - **Why not derive it?** The only other sources are claims (unreadable, U-12) and a scratch accept (an
    incremental artifact omits objects the forge already holds, so it is refused outside the forge's store).
    When prikk exposes the accept's patch ids (the drafted ask), the declaration becomes a cross-check.
- **D-2 — A change is a series of immutable revisions.** A push names an existing `change_id` to add a
  revision, or none to open a new change. A change has `target` (the ref name the submitter intends, advisory
  only), a title and a description (Markdown, rendered through the RFC 003 sanitizer), and a state:
  `open`, `landed` (D-6) or `closed` (withdrawn or rejected, with a reason). Nothing is deleted: a closed
  change keeps its record, and **its bytes remain in the store** (no reclamation exists, ledger PK-41/E11);
  the UI says so.
- **D-3 — Base: declared by the client, verified as held.** The client helper states the block id it built
  against (it knows it; claims' parents are unreadable by the forge). The forge accepts a revision only if
  the base is **listed by `log` of a ref it holds** (published or received; **not** by `show <base>`, which fails for
  some blocks, U-9); otherwise it refuses with a named
  reason ("base unknown: push the base first"). A revision with no declared base records `base: none` and is
  reviewable but its results never count (D-7). A *declaration* is not a fact about the artifact; the UI
  labels it "declared base".
- **D-4 — The review view: what the change does to the files, from `extract`; the patch list from ids.**
  *(Amended 2026-10-21 on prikk's answer, PK-64.)* Two facts decide it: `prikk show <block-id>` is a prikk defect
  for blocks that edit a file a third time (fix proposed for 0.54.0), and `show <patch-id>`, which works for every
  patch including pending ones, **names the file by an internal node id for a *text edit*** (a rename and a delete carry
  their paths; prikk's correction of 2026-10-21, PK-65), so it cannot say which file an edit changes. Therefore:
  1. **The primary review view is file-level.** For a revision the forge materializes two points with
     `prikk extract` (RFC 006 / ledger PK-56): the **claim** (`extract --claim`, i.e. its parent's files plus the
     claim's patches) and its **parent** (`extract --ref <parent block>`); prikk measured exactly this route
     (`diff -r before/ after/` shows the edit and the rename, both extracts `provisional: false`; PK-65). **prikk
     will report the claim's parent in `extract-report-v1` (`claim_parent_block_id`, 0.54.0, PK-66)**, so the "before" tree is
     the claim's **real** parent, and a revision whose declared base (D-3) disagrees with it is refused with a named
     reason. (If the field were absent, the declared base would stand in and the page would say "compared with the
     declared base"; that fallback is not expected to ship.) planeter diffs the two trees
     itself (sanitized, size-bounded by `--max-bytes`/`--max-files`, rendered through RFC 003). This also shows
     exactly "what the change does to the files", the thing a reviewer needs, and uses only prikk's own
     materialization. It requires prikk's `extract` (0.54.0) and the checkpoint refresh of RFC 006 v2; **until
     then the review view does not exist** and the change page says so plainly.
  2. **The patch list is secondary**: ids and, where `show <patch-id>` gives them, operation kinds and the paths it
     resolves (creates, renames and deletes carry their paths; a text edit names only its node). A rename, which a tree diff shows as a removal and an addition, is labelled from the patch's two paths. Patch ids are content-addressed
     and immutable, so caching by patch id is correct forever and does not conflict with INV-6. Nothing is ever read
     through a block id for a pending change; sealed history is read through RFC 003 (and benefits from the prikk fix
     when it ships).
  3. What can be built **before** 0.54.0 is everything except the file-level view: the intent journal, revisions,
     quotas, comments and decisions, landing observation, the artifact relay. The implementation handoffs are
     sequenced that way.
- **D-5 — Review state is forge-owned and bound to a revision.** Comments (anchored to a patch id and, where
  prikk's view carries one, a path and span), threads, and a decision per reviewer per revision
  (`approve` | `request-changes`), authorized by RFC 002's existing `comment` and `review` actions. A new
  revision does **not** carry approvals over: they are shown on the revision they were given for, and a
  later revision shows "approved on revision N". This keeps "approved" from silently covering code the
  reviewer never saw.
- **D-6 — Landing is observed, never asserted.** *(Amended 2026-10-27, D-40: the first wording, "left the pending
  set", was wrong on a keyless forge — a block that is *received* does not clear `sync pending`, ledger PK-71 — and a
  received ref is replaceable and unverified, so it cannot make a final state.)* A change is `landed` when every
  patch id of its **current patch set** is present in a block that `log` lists for a **published** ref of the
  repository (`heads/*`); `sync pending` is not consulted, and received refs are shown as sightings, never as
  landing. Until a key-holding action publishes history here (decision D-8) a proposal normally stays open, and
  the page says so. The current patch set is the latest *declared* revision's declared set plus the `added` sets of
  every later revision (the union of all `added` sets when none is declared), and the UI says that a
  superseded patch that no one seals keeps such a change open. The forge never marks a change landed by its own say-so, and
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
- **D-10 — Honesty is a type, not a style.** *(Amended 2026-10-21 by IS-44/IS-47.)* The project's `Assurance`
  has four values (`prikk-verified`, `sealed`, `forge-approved-unsealed`, `unverified`). **`sealed` is never
  derived from `queued: false`** (U-10: a pending patch reports `queued: false`); a patch is shown as signed
  into the history only when a block that contains it is read from `log` or a published tip, and as pending when
  `sync pending` lists it. The change view model carries an `Assurance` for every item:
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

- **Model tests (pure):** revision derivation from an intent's pending-before and the pending-after of a complete run; **a killed accept records no revision and the resumed accept records exactly one, from the first attempt's pending-before**; idempotent re-accept; an accept that adds nothing; declared sets (accepted, `declared ⊉ added` refused, an id not held refused); approvals not carried; staleness; landing observation from a recorded pending sequence; a pending patch (`queued: false`) never rendered as `sealed`.
- **Authorization tests:** `propose` default-deny; setting `any-authenticated`; quotas; reviewers cannot
  approve their own revision's artifact retention beyond quota.
- **Honesty tests:** an open change never renders `PrikkVerified`; an approval never renders as verification.
- **Real-binary tests (prikk on `PATH`, skipped visibly otherwise):** accept → revision → `show` per **patch id** →
  relay → second repository accepts the relayed artifact; SIGKILL an accept at several points, recover per RFC 004 D-6 and obtain exactly one revision; a block that edits a file three times is reviewed through its patches (U-9); the stand-in is allowed only for text formats.
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

Reviewed 2026-10-21 against the owner's philosophy ("clean, safe and secure, robust and sophisticated; users must not
be confused"; "less is more"). Decided by the architect unless marked *owner*.

- **Q-1 (OQ-4, the feature edge) — decided: the core set only for the first release**, i.e. exactly D-1…D-11. Suggested
  changes, draft changes, thread resolution, required-reviewer lists, boards and issues are each their own RFC, added
  only when wanted; each would sit behind this model without changing D-1…D-7. Fewer features that are true beat more
  that are half-built.
- **Q-2 (D-9, the contribution model) — decided as a proposal for the owner's approval with the RFC:** `proposals:
  members` by default (never anonymous). **Default quotas, operator-configurable per repository, tuned by RFC 004
  D-12's data** (an accepted patch cannot be removed from a store, so quotas bound disk, not behaviour):

  | | `members` (default) | `any-authenticated` |
  |---|---|---|
  | open changes per principal per repository | 10 | 3 |
  | cumulative accepted bytes per principal per repository | 512 MiB | 64 MiB |
  | revisions per change | 20 | 10 |
  | per push | RFC 004 D-5 edge bounds (256 MiB, 20,000 objects) | the same |
  | repository-wide ceiling on pending bytes | 4 GiB, operator-set; past it proposals are refused with the named reason "repository capacity" | the same |
  | retention of an accepted artifact after `landed` or `closed` | 30 days, operator-set (while `open`: kept) | the same |

  Every refusal names the quota and what the person can do (close a change, ask a maintainer to raise it).
- **Q-3 (architect → prikk) — answered 2026-10-21** (ledger PK-64, PK-65): `show <block-id>` is prikk's defect (fix
  proposed for 0.54.0); `queued` means "in this repository's own queue"; a killed accept is resumable by design; the
  review route is `extract --claim` against `extract` of the parent (measured by prikk). `sync pending --format
  json` (claim → patches, signer, parent) is recorded as our first choice, not scheduled (prikk, PK-66); it would turn D-1's
  declaration into a cross-check.
- **Q-4 (D-8, publication) — the one owner question left; it does not block accepting this RFC.** Nothing in
  D-1…D-11 depends on its answer: "landed" is observed. Under model A a change can be proposed, reviewed and
  downloaded but never lands on this forge's `main`; under the recommended model B (RFC 004, "D-8 in plain words") it
  lands when the maintainers' sealed history is received and selected by rule. The change page says which is in
  force, in plain words, so nobody waits for something that cannot happen.

## Sequencing and handoff

Needs RFC 004 v2 (the lease, the accept edge, the artifact store) and the owner's D-9; independent of D-8.
On acceptance the architect writes handoff v2 (data model and migrations, the accept-event derivation, the
review API, the relay, the honesty tests). Issues follow as their own RFC and handoff. All v0.x; 1.0 is the
owner's gate.
