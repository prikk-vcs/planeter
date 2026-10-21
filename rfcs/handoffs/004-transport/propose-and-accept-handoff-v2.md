# RFC 004 — Transport — Implementation Handoff (v2, part 3b): propose — the accept path

| | |
|---|---|
| Document | Part 3b of the RFC 004 v2 series: the **write side of the transport**. One authenticated request puts an artifact on the forge as a **change revision**: authorize (`Propose`), bound at the edge, check quotas, take the **W lease**, write the **intent**, run `sync accept` in the sandbox **to completion** with its own deadline and memory admission, derive the revision from the intent (the pure model of part 3a), record it in one transaction, retain the artifact, bump the state fingerprint, release. A kill or timeout at any point leaves an open intent and `needs-recovery` (part 4 clears it). Built on part 1 (lease, artifact store, mounts), part 2 (routes, errors, fingerprint) and part 3a (records and rules). |
| Status | **Assigned** (2026-10-22), after part 3a and the sessions removal were reviewed Approved. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-3, D-5, D-6, D-7, D-11 and U-11…U-13; [`../../accepted/005-change-review.md`](../../accepted/005-change-review.md) D-1…D-3, D-9; [`../005-change-review/change-model-and-journal-handoff-v2.md`](../005-change-review/change-model-and-journal-handoff-v2.md) and `docs/changes.md`; ledger PK-59 (accept: time per patch, memory 4.2×, resumable), PK-58 (`queued`), PK-60; RFC 011 (admission, ceilings, child lifetime); part 2's `transport` module, `RevisionStore`/fingerprint, error codes and catalog texts. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T0 — Three corrections decided in part 3a's review (first, each its own commit)

(a) **`current_patch_set` (RFC 005 D-6) is conservative:** the current patch set is the latest *declared* revision's declared set
**plus the `added` sets of every revision after it**; if no revision is declared, the union of all `added` sets. A declared set never
silently drops out because a later push was undeclared. Update the model, the tests (including the sequence declared → undeclared → landed)
and `docs/changes.md`. (b) **The capacity ceiling counts retained artifacts:** `repo_pending_bytes` becomes the bytes of every
revision whose artifact is still retained (change open **or** within its `retain_until` grace), because that is what occupies the disk;
rename the usage field accordingly and keep `accepted_bytes` per principal cumulative. (c) `ChangeView::from_show`'s
`queued: false → Sealed` stays valid **only for the patches of a sealed block**: document that on the function and put the
"pending patches are classified by `patch_assurance`, never by `from_show`" rule in a test that feeds a pending patch through the new
path of T5.

## Change scope

- **T1 — The route and its framing.** `POST /api/v1/repos/{owner}/{name}/transport/propose`, registered through the route registry
  as `Authorized(Propose)` (the IS-14 probe must show a stranger and a read-only principal get the same existence-neutral 404/403 as
  elsewhere). **Request framing (no new dependency, no multipart):** the body is
  `PLANETER-PROPOSE-1\n` + an 8-byte big-endian length `M` + `M` bytes of JSON metadata + the artifact bytes to the end, with
  `Content-Length` required; `M` is capped (4 MiB) and the artifact by `PRIKK_EXCHANGE_MAX_BYTES` **at the edge, before buffering**
  (`413`); the artifact is streamed to the artifact store's `incoming/` while hashed, never held in memory. Metadata: `change` (an
  existing change id, or absent to open one), `title` (≤ 200 characters), `description` (≤ 64 KiB), `target_ref` (advisory),
  `base` (a block id), `declared_patches` (list of 64-hex ids, or absent), `client` (name/version string). Unknown fields are refused.
  Document the framing in the OpenAPI document (`application/octet-stream`, described) and in `docs/`.
- **T2 — The order of checks, all before any prikk call that writes.** rate limit (caller-keyed, existence-neutral) → the read gate
  and `authorize(Propose)` → `needs_recovery` flag (writes refused with the named reason `needs_recovery`, `503`, hint saying what the
  operator does; reads are unaffected) → framing and size → quota (`check_quota`, the policy of the repository's `proposals`
  setting; a refusal names the quota, the limit, the use and the remedy through the catalog texts of part 3a) → metadata validation →
  **adding a revision to an existing change**: the change must be `open` and the submitter must be its opener **or** hold `write`+ (a
  third party cannot append to someone's change; a refusal is a named reason) → the declared base is checked with
  `check_base` against the blocks listed by `log` of the held refs (never `show <base>`, PK-57) → idempotence: an artifact whose digest
  has a revision already returns that revision (`200`, `already_recorded`), recording nothing.
- **T3 — Accept under the W lease, with an intent.** Take the **W lease** (bounded wait, `503 busy` + `Retry-After`); read
  `sync pending` (the pending-before list); `open_or_resume_intent` (a resume keeps the first attempt's list); copy/link the artifact
  into a **per-accept working directory** mounted read-only as `artifact_in` and a second working directory for the accept's
  outputs (`--claims-out`) mounted as `spool_out` (the same narrowing as part 3a's T0b: no whole-directory mounts); run `prikk sync
  accept` in the sandbox with `PRIKK_EXCHANGE_MAX_BYTES`/`MAX_OBJECTS` (operator variables; defaults 256 MiB and 20,000), the accept
  **deadline** (default 600 s, operator variable) and the RFC 011 admission for a write (**cost 10 MiB + 4.2 × the artifact's
  bytes**, data ceiling × 1.5, from the same budget as reads, never in the light pool). Capture exit status, claim lines (text, parsed
  with the stable shape of U-12; an unparsable line is a named failure) and the claims file.
- **T4 — Complete or incomplete.** **Exit 0 ⇒ complete**: read `sync pending` again, call `derive_revision` (declaration checks with the
  `held` predicate from pending ∪ the patch ids of blocks in `log`), then in **one transaction** `complete_intent_with_revision`
  (creating the change if this push opened one); then `write_completed(&lease)` (fingerprint bump), `adopt` the artifact into the
  artifact store (retained), release the lease, return `201` (or `200` for an added revision) with `{change_id, revision, seq, claims,
  outcomes, added_count, declared: true|false, assurance: "forge-held-unsealed"}` — the result states plainly that the work is
  **held for review and no branch has changed** (catalog text). **Any other end** (non-zero exit, timeout, ceiling hit, a killed child,
  the server shutting down): set `needs_recovery` with the reason, keep the intent `open`, keep the uploaded artifact in the store so
  the retry needs no re-upload, release the lease, answer `503 incomplete` with a stable code, the intent id and the hint
  "send the same artifact again once the repository has recovered". A **refused** accept (prikk exit 1 with a refusal text: over bound,
  damaged artifact, base unknown to prikk) leaves nothing (U-12): abandon the intent, **do not** set `needs_recovery`, answer
  `422` with prikk's text kept verbatim behind a plain sentence (the wording rules of the catalog). Never derive a revision from an
  incomplete run; never infer "sealed" from `queued`.
- **T5 — Pending patches are classified by facts, not by `show`.** For the response and the stored record use
  `patch_assurance` with facts from `sync pending` (pending) and `log` (sealed); a pending patch is never rendered as sealed. Add the
  view-model needed by RFC 005's later review UI only as far as the response requires; no UI in this part.
- **T6 — Shutdown and kill behaviour.** A graceful shutdown during an accept waits for it within the drain, else treats it as an
  incomplete run (T4). A real-binary test **SIGKILLs the server in the middle of a large accept** (a 1,000-patch artifact as in the
  measurements): afterwards the store has an open intent with the first attempt's list, the flag is set (set by the next start if the
  process could not set it: reconcile at start from open intents and the durable lease records of part 1), reads still work, a new
  proposal is refused with `needs_recovery`, and `verify`/`doctor` of the repository are clean (U-13). **Recovery itself is part 4**;
  this part provides a test hook that clears the flag after unlocking, then shows that re-sending the same artifact **resumes the intent,
  completes, and records exactly one revision** derived from the first attempt's list.
- **T7 — Instrumentation and messages.** Counters (accepted, incomplete, refused by reason, bytes, wall time, lease wait, attempts per
  intent) and one structured log line per proposal (opaque repository id); every new user-visible text is a catalog message (English and
  the Japanese draft rows, added to a third addendum of the reviewers' sheet); the new API error codes are in OpenAPI and its drift guard.
- **T8 — Documentation.** The framing, the order of checks, the state diagram of an intent (`open → complete | abandoned`; the flag), the
  operator variables (bounds, accept deadline, memory), the failure table of T4 (what the client sees, what the operator sees), CHANGELOG,
  STATUS. The architect updates the threat model.

## Non-change scope

No client helper, no SSH, no recovery procedure beyond the test hook (part 4), no review UI, comments or decisions, no change view, no
repository creation from an archive, no publication, no `extract`, no new external dependency, no `unsafe`, no change to existing
responses, no version or release change.

## Acceptance criteria

T0's three items done; the end-to-end test with real prikk: a keyed client builds an artifact with `sync build` against the forge's
`sync have`, the proposal returns `201` with a change and revision 1, the advertisement's ETag changes, `sync pending` shows the added
patches, a second revision to the same change and a re-send of an identical artifact behave as specified, a declaration that misses an added patch or
names a patch that is not held is refused, an over-bound artifact is refused at the edge (`413`) and by prikk (exit 1, nothing accepted),
a read-only principal and a stranger are refused existence-neutrally, quotas refuse with named reasons; the kill test of T6 passes; the IS-14
table, the OpenAPI drift guard, ENF-2 and all earlier checks pass; every gate green including both supply-chain gates (fresh advisory
database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No derivation from an incomplete run; no recording a revision outside the transaction; no accept without an intent; no accept without the
W lease; no artifact held in memory; no whole-directory sandbox mount; no setting `needs_recovery` for a refusal that leaves nothing; no
prikk text shown as the only message; no hard-coded user-visible text outside the catalog; no hidden/absent/denied difference that a
stranger can see.

## Known risks

The accept is the one operation here that can be killed in the middle: every path out of it must end in a state the next start can
read (open intent, flag). A large `pending_before` list (up to 20,000 ids) is read twice per proposal: measure it. The SQLite connection
is shared by the whole server: keep every transaction short.

## Required evidence and review-request format

Commits (hash → what, by task); the check-order table with a test per refusal; the end-to-end transcript (request bodies elided,
responses shown); the kill test's log and the state afterwards; the failure table; timing of a 1,000-patch accept and the size of its
intent; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
