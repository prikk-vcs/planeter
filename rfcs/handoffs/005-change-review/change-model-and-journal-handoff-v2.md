# RFC 005 / RFC 004 — Implementation Handoff (v2, part 3a): the change model, the intent journal and the quotas

| | |
|---|---|
| Document | Part 3a of the transport series and the first part of RFC 005's v2 handoffs: **the data and the pure rules** behind a proposed change — the change and revision records (RFC 005 D-1…D-3, D-6), the **intent journal** and the `needs-recovery` flag (RFC 004 D-3, D-6), and the quota rules (RFC 005 D-9) — as store tables and pure model functions with exhaustive tests. **No route, no `sync accept`, no prikk call.** Part 3b wires these to the network; part 4 is recovery. Keeping this part pure is deliberate: the hard correctness questions (a killed accept, a declared patch set, an idempotent re-push) are all decidable here without a process or a socket. |
| Status | **Done, reviewed Approved (2026-10-22; commits `c8e67bc`…`5e12202`).** Next: `../004-transport/propose-and-accept-handoff-v2.md` (part 3b). |
| Basis | [`../../accepted/005-change-review.md`](../../accepted/005-change-review.md) D-1, D-2, D-3, D-6, D-9, D-10 and §Testing ("Model tests (pure)"); [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-3, D-6 and U-11, U-13 (a killed accept leaves a prefix pending; a retry converges); ledger PK-58, PK-59, PK-65; part 1's `planeter-core::lease` (holder records) and `planeter-store` (`apply_schema`, `ensure_column`, the SQLite and in-memory pairs); `planeter-core::authorize` (`Propose`, `Close`, `Comment`, `Review`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T0 — First, two things that have waited or were decided in part 2's review

(a) **Finish `rfcs/handoffs/003-read-path/verify-sessions-removal-handoff-v1.md`** (IS-49): it has waited through three parts
and is small; deliver it as its own commits and its own review request before T1. (b) **Narrow the export sandbox mount:** the
export currently mounts the whole spool directory read-write. Give each export its own working subdirectory
(`<spool>/work/<export-id>`, created before and removed after, orphans cleaned at start) as the `spool_out` mount, move the finished
file into the spool by the atomic rename of part 1, and re-hash it before serving as today. Tests: a stand-in prikk inside the real
sandbox can write its own working directory and **cannot read or write any other spool object**; the existing fetch matrix stays green.

## Change scope

- **T1 — The records (store).** In `planeter-store`, with the usual SQLite and in-memory implementations behind traits and
  idempotent migrations (`apply_schema`):
  - **`changes`**: `change_id`, `repo_id`, `title`, `description` (stored as text; rendered only through the sanitizer, later),
    `target_ref` (advisory), `state` ∈ {`open`, `landed`, `closed`} (a CHECK constraint), `closed_reason` (nullable), `opened_by`,
    `opened_at`, `updated_at`, `retain_until` (nullable; set when the change leaves `open`).
  - **`revisions`**: `revision_id`, `change_id`, `seq` (1, 2, …; `UNIQUE(change_id, seq)`), `artifact_digest` and
    `artifact_bytes`, **`UNIQUE(repo_id, artifact_digest)`** (an identical artifact is one revision), `declared_base` (nullable block
    id), `claim_ids`, `added_patch_ids`, `declared_patch_ids` (nullable: absent for a push by plain prikk), `claim_outcomes`,
    `submitter`, `author_key_ids`, `created_at`. Lists are stored as JSON text or a child table of your choice (state which and why);
    ids only, **no history is copied** (NG-6).
  - **`intents`** (RFC 004 D-3): `intent_id`, `repo_id`, `artifact_digest`, `pending_before` (the patch ids, from the **first**
    attempt), `principal`, `started_at`, `attempts`, `state` ∈ {`open`, `complete`, `abandoned`}, `completed_at`, `revision_id`
    (nullable).
  - **`repo_flags`**: per repository `needs_recovery` (bool), `reason`, `since`; the `proposals` column from part 1 stays where it is.
  - **One transaction** `complete_intent_with_revision(intent_id, revision)` that inserts the revision (and the change if the push
    opened one) and closes the intent **atomically**; any failure leaves the intent `open` and no revision. Also
    `open_intent`/`resume_intent` (same digest → the existing open intent, `attempts += 1`, `pending_before` **not** overwritten),
    `abandon_intent`, `set_needs_recovery`/`clear_needs_recovery`, the reads the model needs, and **usage queries** for the quotas
    computed from `revisions`/`changes` (no separate counters, so they cannot drift).
- **T2 — The pure model (core).** A `change` module in `planeter-core`, no I/O:
  - `derive_revision(intent, pending_after, held, declaration) -> Result<NewRevision, DeriveError>`: `added = pending_after −
    intent.pending_before`; a **declaration** (the list of patch ids the client says its artifact contains) is accepted only if
    **`declared ⊇ added`** and every declared id is **held** (`held` is a predicate the caller supplies: pending or reachable);
    errors are named (`DeclarationMissesAdded{…}`, `DeclaredNotHeld{id…}`, `NoNewContent`). Without a declaration the revision
    records `added` only and is marked `declared: none`. **A diff from an incomplete run is unrepresentable:** the function takes the
    intent record, never two bare sets, and refuses an intent that is not `open`.
  - **Idempotence:** the same digest finds the existing revision (`AlreadyRecorded(revision_id)`); an accept that adds nothing new
    returns `NoNewContent` and records nothing.
  - **Base (D-3):** `check_base(declared_base, listed_blocks)` — held iff listed by `log` of a held ref (the caller supplies the
    list); no declared base records `base: none` ("its results never count").
  - **States (D-2):** the transition function: `open → landed` only by **observation**, `open → closed` with a reason (by a
    principal the caller has already authorized), `landed`/`closed` never reopen; `retain_until` computed from a configurable grace
    (default 30 days) when leaving `open`.
  - **Landing (D-6):** `observe_landing(patch_set, pending_now) -> Landed | NotYet{remaining}`, where the **current patch set** is the
    latest revision's declared set, or for undeclared revisions the union of their `added` sets (RFC 005 D-6), and "landed" means
    every id has left `pending_now`; it never marks landed on its own say-so (it only reports an observation the caller feeds it).
  - **Quotas (D-9):** `check_quota(usage, policy, request) -> Result<(), QuotaRefusal>` with the policy table of RFC 005 Q-2
    (`members` and `any-authenticated` columns: open changes, cumulative accepted bytes, revisions per change; the per-push bounds
    belong to part 3b's edge; the repository-wide pending-bytes ceiling), every refusal **naming the quota, the limit, the current
    use and what the person can do** (catalog keys for the texts, English and the Japanese draft rows added to the sheet addendum).
  - **`Assurance` rule (D-10, IS-47):** a helper that classifies a patch as `Sealed` only from membership in a block read from `log`,
    as `pending` only from `sync pending`, and **never from `queued`**; tested with a pending patch whose `queued` is false.
- **T3 — Tests (pure and store).** The RFC 005 "Model tests" list, each as its own test, plus: **a killed accept records nothing
  and the resumed accept records exactly one revision, derived from the first attempt's `pending_before`** (simulate the sequence:
  open intent, prefix pending, no completion, restart, resume with more pending, complete); an intent for a different digest while
  one is open is a separate intent (a repository may have two open intents only if part 3b allows it: expose the query and test
  that W-lease serialization is a precondition, documented); the **atomicity** of `complete_intent_with_revision` (inject a failure
  after the revision insert: intent still `open`, no revision, change not created); a 0.4.0-shaped database migrates idempotently
  over three reopens; uniqueness constraints refuse duplicates with typed errors; quota boundaries (exactly at the limit, one over,
  each of the three limits, both policies); landing from a recorded pending sequence; state transitions (illegal ones refused).
  Use property-style tests where a rule is universal (never derive from a non-open intent; `declared ⊉ added` is always refused;
  no function returns `Sealed` for a pending patch).
- **T4 — Documentation.** A short design note `docs/changes.md` (the records, the journal states and the invariants in plain
  words; what is stored and what never is), CHANGELOG `[Unreleased]` only for operator-visible facts (new tables, the migration), STATUS.

## Non-change scope

No route, no UI, no API, no `sync accept`/`sync pending`/`log` calls (the model takes their results as arguments), no comments or
review decisions (RFC 005 D-5, a later part), no retention deletion job, no change to authorization other than reading
`Close`/`Propose`, no new dependency, no `unsafe`, no version or release change.

## Acceptance criteria

T0's two items done and reviewed; every table has its migration test; `complete_intent_with_revision` is atomic under injected
failure; the full model test list passes and each rule has a mutation (a diff accepted from a non-open intent; the declaration
check weakened; idempotence broken; `queued` read as sealed; quota off by one) that fails it; both store implementations pass the
same shared test suite; every gate green including both supply-chain gates (fresh advisory database), both test runs named, ENF-2,
`cargo doc`.

## Prohibited shortcuts

No derivation from two bare sets; no revision insert outside the transaction; no counters that can drift; no storage of patch
content or history; no `queued`-based sealed logic; no hard-coded user-visible text outside the catalog.

## Known risks

`pending_before` can be large (tens of thousands of ids): store it compactly (a blob or a child table), and say how large an intent row
gets at the 20,000-object bound. The SQLite connection is shared by the whole server: keep the transaction short and test it under
the shared mutex.

## Required evidence and review-request format

Commits (hash → what, by task); the schema (tables, constraints, indexes) as created; the model test list with each mutation; the
atomicity and migration evidence; the size of an intent at the object bound; every gate with command and exit code and the
advisory-database HEAD; deviations and open questions.
