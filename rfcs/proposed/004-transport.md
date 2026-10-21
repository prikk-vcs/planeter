# RFC 004 — Transport: clone and push as a ferry over prikk's artifact exchange (third edition)

**Status.** Proposed — **third edition, drafted 2026-10-21** for the owner's reading; the implementer must not
start. It replaces the body of the second edition (2026-10-15; earlier editions are in this path's git history).
**The measurement the second edition waited for is done and reviewed** (the transport measurement handoff
[`../handoffs/004-transport/transport-measurement-handoff-v1.md`](../handoffs/004-transport/transport-measurement-handoff-v1.md),
rows A1–A12, run on prikk 0.53.0 and side by side on 0.52.0; ledger v1.9 PK-57 to PK-60), so the rows that
blocked acceptance (RFC 010 D-8) are now **confirmed**; this edition changes the design where the numbers
disagreed with the second edition (see §What the measurements changed). It is ready for the owner's decision to
accept. Handoff v1 is superseded; **handoff v2 is written on acceptance.**
**Tracks.** ROADMAP Phase A2 → M2 (reduced; see §Scope); requirements `CAP-2`, `INT-2`, `STD-1`, `NG-2`,
`OPS-3`; external design `TX-01…06`, `FL-02/03`, `OP-01/03`; internal design `WR-1/2/3/6`, `CON-1/3`,
`PKI-2/3`; threat model `T-3`, `T-4`, `T-9`, `INV-2/3`, `C-4`; RFC 002 (`authorize()`, new action `propose`),
RFC 005 v2 (the change record, artifact retention), RFC 006 v2 (snapshots), RFC 010 (bounds, floor, gates);
ledger PK-28, PK-32–PK-40, PK-46–PK-50, PK-56–PK-60; SCHEDULE decisions D-8, D-9, D-23.
**Touches.** `planeter-transport` (new surface), `planeter-prikk` (sandbox profile, exchange verbs),
`planeter-core` (lease, artifact store), `planeter` (config), and a **client helper** (`planeter` CLI).
**Not here:** publication of a branch on the forge (RFC 008, owner decision D-8), the change and review
model (RFC 005 v2), CI (RFC 006 v2), the `archive`-range delta (not planned by prikk, ledger PK-47).

## Summary

planeter moves bytes and prikk makes them: the transport ferries prikk's own artifacts over an
authenticated, encrypted envelope and **invents no prikk wire protocol and holds no signing key**. The
2026-09-15 design assumed a forge that builds incremental fetch artifacts and advances refs. Measurement and
prikk's own decisions overturned both: `sync build` is keyed (a keyless forge builds none); fetch is a
**whole artifact** for the foreseeable future, and prikk will decide a delta form on *our* first-deployment
numbers, not before 0.55.0; **export holds every container lock** and would let a slow client block every
push; a killed writer leaves locks that prikk never steals. This edition therefore specifies a transport
that is correct on today's prikk: **whole-artifact fetch that is spooled, verified, cached and conditional;
push as a keyless, leased, bounded accept; a three-class repository lease; explicit recovery after a kill;
and a client whose first clone asks the user to adopt keys.** It states plainly what it does not deliver
(incremental fetch, forge-side publication).

## Upstream facts (what this design stands on)

| ID | Fact | Label | Ledger |
|---|---|---|---|
| U-1 | `sync build` refuses without a maintainer key and stays keyed (no `--unsigned`). Every fetch after the first is a full bundle or a full archive. | V, M | PK-27, PK-36 |
| U-2 | Keyless `sync accept` works, is idempotent and tamper-refusing; over-size is refused before reading, exit 1 (`PRIKK_EXCHANGE_MAX_BYTES`). | M | PK-28, PK-31 |
| U-3 | `archive export` is whole-repository, verbatim, takes no have-list; since 0.52.0 an archive of a repository that passes `verify` verifies and imports; every export is `archive verify`'d before serving (≈4 s, ≈388 MiB per GiB of blobs, prikk-measured). | V | PK-32 |
| U-4 | Keyless `archive import` works end to end on 0.51.0/0.52.0 (prikk-measured); `--adopt` lands a ref only if absent locally, signed by an adopted key, verified. | V (prikk-measured) | PK-33, PK-34 |
| U-5 | **Export holds all five container locks for its whole run; writers fail fast**; a killed export leaves its locks. | V | PK-37 |
| U-6 | Locks are exclusive-create files, fail fast, never stolen; `unlock` never decides from the recorded PID. Reads create no locks but read mutable state. | V | PK-39 |
| U-7 | Memory: `bundle import` ≈ 6.0×, `verify` ≈ 3.0× the artifact; bounds are env vars and `prikk config incoming.max-object-bytes`; `bundle import` memory is a 0.55.0 candidate. | V | PK-40, PK-46 |
| U-8 | `sync have heads/main` on an unpublished ref is an empty have-list; `sync have remotes/…` is refused. Pushers re-send history; accept is idempotent. | V (architect-measured) | PK-49 |
| U-9 | `sync pending` is text-only, patch ids only; claims unreadable; `verify` fails until signers' keys are adopted. | V (architect-measured) | PK-48, PK-50 |
| U-10 | The delta archive and the append-only promise are not scheduled; decided with our deployment numbers, not before 0.55.0. | V | PK-47 |
| U-11 | planeter's sandbox: `timeout --kill-after=5 120 bwrap --unshare-all --clearenv --die-with-parent`; binds only the repository root read-write; wall-time bound only; incoming-bound seam exists (none set). | V (code) | IS-4, IS-15 |

| U-12 | **Measured on 0.53.0 and 0.52.0 (identical), 2026-10-21 (planeter, A2/A3):** accept peak memory ≈ **4.0–4.6 × the artifact** (+ ≈ 10 MiB floor); `sync build` ≈ 2.0 ×. Accept time ≈ **7–17 ms per patch** (173 s at 10,000 patches, 227 s at 16,835; load roughly doubles it); `archive import` ≈ 3–4.5 ms per patch. A refused over-size artifact: exit 1, names size, bound and variable, accepts nothing; damaged artifacts: exit 1, repository stays `verify`-clean; idempotent re-accept: `new objects: 0`, claims file byte-identical. | M | PK-59 |
| U-13 | **A killed `sync accept` is not atomic but is resumable (A7, A12):** SIGKILL leaves a **prefix** of the patches pending (147/397/706/902 of 1,000 at 15/40/70/90 % of the run), `active.lock` (and `objects.lock`) in place, and the repository `verify`- and `doctor`-clean; after the locks are cleared a retry resumes and converges to all patches. | M | PK-59 |
| U-14 | **`sync have`/`summary` (A3):** empty (60 B) for pending-only history; for a published ref 60 B + 32 B per patch (252 B for 6 blocks, 32 KB at 1,000 patches, 320 KB at 10,000); `sync summary` a constant 68 B. Incremental pushes exist only against a published ref; a received ref cannot be queried (`remotes/…` refused). | M | PK-49, PK-59 |
| U-15 | **`archive export` (A8, A9):** flat memory (≈ 10 MiB at 63/199/1,000 MiB), 0.15/0.25/0.87 s warm, spool = store size; byte-identical for the same state (so an ETag over the state is sound); after an accept sections 0, 5 and 6 grow by appending but the whole-file common prefix is 24 bytes (no range-resume from a file offset). | M | PK-60 |
| U-16 | **Locks and readers (A7, A10):** `archive export` and `sync accept` exclude each other and **whoever starts second fails** (exit 1, 20–53 ms, `lock conflict: … <absolute path>`; an accept can lose to an export); `bundle export` is lock-free; **readers (`tree`, `log`, `verify`, `branch`, `status`, `sync pending`, `sync have`) never failed** during 30 accepts or a heavy accept. Inside our `bwrap --unshare-all` vector `prikk unlock` reports a false "appears running" (recorded pid 2); outside it says "does not appear to be running"; clearing needs `--yes` or `--force`. | M | PK-37, PK-39, PK-60 |
| U-17 | **Keyless archive path (A1, A4, A11):** `init` → `archive verify` → `trust maintainer add` → `archive import --adopt` works end to end and creates no key directory; without the adoption the ref lands as `remotes/heads/main` and `verify` exits 1 (`publication-trust`); an archive of a repository that accepted a sync artifact verifies and imports; `--adopt` lands a ref locally only if it is absent locally, signed by an adopted key and verified; `archive import` streams (11 MiB peak for 200 MiB). | M | PK-33, PK-34, PK-32 |
| U-18 | **`prikk extract`** (built, unreleased, 0.54.0; prikk's figures, not ours): a point = ref, block or one pending claim, no history, all or nothing, memory ≈ 2 × the largest file. Relevant to RFC 006 and the change preview, not to this RFC's transport. | R (prikk) | PK-56 |
| U-19 | **`prikk show <block-id>` fails for a block that edits a file for the third time (0.52.0 and 0.53.0)** while `verify` is clean; `show <patch-id>` works. Do not read a change through its block id. | M | PK-57 |
| U-20 | **`show <patch-id>` reports `queued: false` for a pending patch:** `queued` means "in this repository's own active queue", not "not sealed". | M | PK-58 |

**Confirmed since the second edition:** keyless `archive import` under our sandbox and from a fresh process
(A1, U-17); the accept memory multiplier *k* (A2, U-12); `sync have` states (A3, U-14); lock and kill behaviour
under `bwrap --unshare-all` (A7, U-13, U-16); export time and spool size (A8, U-15); reader/writer interference
(A10, U-16); a change's derivation (A12 → RFC 005 third edition). **Still not measured:** a 100,000-patch history
(could not be built; the linear law is extrapolated and stated as such); `prikk extract` (0.54.0).

## What the measurements changed (from the second edition)

1. **Accept is resumable, not atomic (U-13).** The second edition said an interrupted accept "leaves an
   unreferenced remnant, not a hole" and "records nothing and is retried". The remnant is a **prefix of pending
   patches**: harmless to prikk, but a change derived from "pending after minus pending before" at that moment
   would be wrong. D-3 and D-6 now record **intent first** and derive only after the accept ran to completion.
2. **Accept time scales with patches, not bytes (U-12).** 10,000 patches take 173 s against a 120 s sandbox
   deadline, and a SIGKILL at the deadline produces exactly the prefix state. D-5 gives accept its own deadline
   and bounds the work by object count at the edge; a very large first landing goes through archive creation
   (D-4), which is 3–4.5 ms per patch.
3. **Readers need no wait on writers (U-16).** The second edition made the read class exclude writers and added
   a "retry once" rule. Nothing failed in thousands of reads during accepts, so D-7 lets the measured read
   verbs run alongside a write and drops the retry rule; only `archive export` conflicts with a write.
4. **`unlock`'s PID check is wrong inside our sandbox (U-16).** The lease is the only liveness authority
   (D-6), as the second edition already said; now with the measured reason.
5. **A write has a memory cost (U-12).** RFC 011's admission covers reads. D-5 extends it to the accept:
   cost ≈ 10 MiB + 4.2 × the artifact, ceiling ×1.5, so that a bound of 256 MiB means ≈ 1.1 GiB of
   budget, not "the artifact".
6. **Do not read a change through `show <block-id>` (U-19)** and do not infer "sealed" from `queued` (U-20):
   consequences for RFC 005.

## Goals

- **G1.** A developer can clone, fetch and push over HTTPS and SSH with a familiar URL.
- **G2.** Nothing a client can do makes the forge hold a signing key, run content-provided code, or exceed
  its stated resource bounds.
- **G3.** A slow or malicious client can never hold a repository's locks or block another client's push.
- **G4.** A crash or kill leaves a repository that is either untouched or recoverable by a written, tested
  procedure, never silently inconsistent.
- **G5.** The deployment can report what fetching costs (RFC 010 / ledger PK-47), so that prikk's decision
  on a delta form rests on numbers.

## Scope (the reduced M2) and non-goals

**Delivers:** advertise, whole-artifact fetch (per-ref bundle and whole-repository archive), keyless push as
accept, the retained-artifact relay RFC 005 needs, repository creation from a maintainer's archive with
explicit key adoption, the client helper, SSH and HTTPS surfaces, leases, bounds, recovery, instrumentation.
**Does not deliver:** incremental fetch (U-1, U-10); forge-side advance of a published branch (RFC 154
unscheduled; owner decision D-8 / RFC 008); a range-delta over the archive (PK-38: not a contract); any
change to prikk. **Not** a Git transport (NG-1).

## Decisions

- **D-1 — The ferry stands.** The envelope (TLS via the trusted proxy, authentication, authorization,
  addressing) is planeter's; the payload (`PEXCH002`, bundle, `PREPO001`) is prikk's and is never
  interpreted or rewritten. TLS is proxy-terminated (loopback HTTP plus the trusted-proxy rule); SSH is the
  host's OpenSSH with `ForceCommand` into `planeter ssh-shell` (owner rulings 2026-09-23).
- **D-2 — Fetch is a whole artifact: spooled, verified, cached, conditional.**
  - **Kinds:** `bundle` (one *published* ref's closure) and `archive` (the whole repository). A received
    ref is not exportable by bundle (PK-49 context; re-check A5), so it is served inside the archive.
  - **Spool, then serve.** Export runs under the X lease to `<spool>/<repo-id>/<key>.partial`, is
    `archive verify`'d, renamed, and only then served with `ETag`, `If-None-Match` → 304 and `Range`.
    **An export is never streamed to a client** (U-5): lock-hold time must not depend on a client's speed.
  - **Cache key and ETag:** `(repo, kind, ref?, state fingerprint)`; two exports of the same state are byte-identical (U-15), so the fingerprint is a sound `ETag`. `Range` works within one file only: after a change the common prefix of two exports is ≈ 24 bytes, so a client can never resume across versions; the fingerprint is a revision counter bumped
    under the W lease, cross-checked by the lengths and mtimes of the carried files so that an out-of-band
    write invalidates rather than serves stale.
  - **Serve gate:** an export that fails `archive verify` is never served; per-ref bundles are the fallback,
    else 503 with a named reason and a metric.
  - **Controls:** single flight per key; a bounded number of concurrent exports per host; a disk budget
    with LRU eviction; the spool on a volume separate from the repositories.
  - **Cost model published in the docs:** cost of a changed-state fetch = artifact size; the mitigations
    are listed so that nobody mistakes this for incremental fetch.
- **D-3 — Push = authorize, journal, accept to completion, derive, verify.**
  1. `authorize(principal, propose, repo)` (RFC 005 D-9) → size, object-count and rate checks at the edge
     (D-5) → the W lease.
  2. **Intent first.** Before `sync accept` starts, planeter durably writes an **intent record** in its own
     store: `{ intent_id, repo, artifact digest, the pending set before (patch ids, from `sync pending`),
     principal, started_at, attempts }`. The record is the only thing that survives a kill.
  3. **Accept with the bounds of D-5 and its own deadline.** A run that ends with exit 0 is *complete*
     (accept is idempotent: `new objects: 0` on a repeat). A run that is killed, times out or crashes is
     **incomplete**: the repository goes to `needs-recovery` (D-6) and the intent record stays open.
  4. **Complete → derive.** Only after a complete run, planeter reads `sync pending` again and records the
     change revision (RFC 005 D-1) from the intent's *pending-before* and the *pending-after*, in one store
     transaction that also closes the intent. **A diff taken after an incomplete run is never used.**
  5. **Recovery converges (U-13):** after `needs-recovery` is cleared (D-6), the *same* artifact is accepted
     again (an open intent for this digest is resumed, `attempts` incremented) until a run is complete.
  Refusals carry a named reason (unauthorized, over bound, quota, base unknown, prikk's refusal text kept
  verbatim with its exit status). The result body names accepted claim ids and the revision. A push is
  idempotent by artifact digest. **Push never publishes** (INV-2).
- **D-4 — Repository creation and the first landing.** A repository can be created from a maintainer's
  archive by: `init` (keyless) → `archive verify` → **the creating principal's explicit adoption of the
  listed maintainer public keys** (`trust maintainer add`, `admin` only, shown with fingerprints, never
  inferred from the archive) → `archive import --adopt`. A ref that is not adoptable lands under `remotes/`
  with prikk's reason, displayed. Advancing a published ref after creation is outside this RFC. **Measured (U-17, U-12):** the path is streaming and creates no key directory; import costs 3–4.5 ms per patch (≈ 33 s at 10,000 patches), so **a large first landing is an archive creation under the W lease with its own deadline, not a push** (a push of that history would cost 173 s of accept).
- **D-5 — Bounds with mechanisms.**
  1. **Edge:** body size cap before buffering (413); claim and object counts where the header allows;
     per-principal and per-IP rate limits (429, `Retry-After`).
  2. **prikk's own, set in the sandbox:** `PRIKK_EXCHANGE_MAX_BYTES`, `PRIKK_EXCHANGE_MAX_OBJECTS`,
     `PRIKK_BUNDLE_MAX_BYTES`, `PRIKK_BUNDLE_MAX_OBJECTS` through the existing seam, and per repository
     `incoming.max-object-bytes` via `prikk config` (never from content).
  3. **Memory budget (extends RFC 011 to writes).** Measured *k* (U-12): an accept's peak is ≈ 10 MiB +
     4.2 × the artifact; `sync build` (client) ≈ 2 ×. The admission cost of an accept is therefore
     **10 MiB + 4.2 × the artifact's bytes**, its data ceiling × 1.5 (RFC 011 D-2), drawn from the same byte
     budget as reads, in its own pool class (a write is never "light"). `PRIKK_EXCHANGE_MAX_BYTES` stays the
     hard edge bound (prikk's default 256 MiB ⇒ ≈ 1.1 GiB of budget per accept), so the operator's budget
     must be at least that to accept anything at the default bound; the start-up message says so. Values come
     from this measurement, re-measured at each prikk re-baseline (RFC 011 D-1).
  4. **Accept deadline and work bound.** Accept time follows the number of patches (≈ 7–17 ms each; U-12),
     not the bytes. At the edge planeter bounds the **object count** (`PRIKK_EXCHANGE_MAX_OBJECTS`, the seam
     exists) and gives the accept its own wall-time bound (default **600 s**, an operator variable, never
     the 120 s read bound). The default object bound is chosen so that bound × 20 ms stays under the deadline
     (≈ 20,000 objects at the default); a larger history is a repository creation (D-4), not a push.
  5. **Per-invocation limits** (memory, CPU, file size, processes) in the sandbox: the `prlimit` data
     ceiling of RFC 011 D-2 (a safe prefix, ≈ +0.15 ms) is the mechanism; a hit is a named refusal
     (`unavailable`-class), never an accept result.
- **D-6 — Containment and recovery.**
  - **Sandbox profile gains two mounts outside the repository:** an artifact-in directory (read-only) and a
    spool-out directory (read-write). Today only the repository root is bound (U-11; the measurement had to
    copy artifacts into the repository directory), so staging would write into a hosted repository's
    directory. With RFC 011 D-5 the child is `bwrap`-direct and dies with the server.
  - **Kill policy:** wall-time → SIGKILL at the deadline (RFC 011 D-5; there is no TERM grace for the
    read path, and for a *writer* the kill is survivable by design, U-13). A killed or timed-out writer
    marks the repository **`needs-recovery`**: writes are refused with a named reason, reads continue
    (the repository is `verify`- and `doctor`-clean after a kill, U-13).
  - **Recovery, in prikk's own order, with our lease as the authority:** `doctor` (read) → planeter confirms
    its own lease holder is gone (the sandbox child was killed by us or died with the server; **`unlock`'s
    PID advisory is ignored because it reports a false "appears running" inside our PID namespace, U-16**) →
    `unlock --lock <path> --force` for each lock `unlock` lists → **accept the same artifact again** (D-3.5)
    until a run is complete → `needs-recovery` cleared. `ref complete` where `doctor` names an interrupted
    publication. Every `doctor` repair's run id is recorded (`--recovery-restore` can undo it). Automatic
    recovery runs only when the open intent record, the lease table and the process table agree; otherwise
    the repository stays in `needs-recovery` for an operator, with the exact commands in the message.
  - Accepts are idempotent, so retry after recovery is safe and the prefix state is harmless to prikk
    (dependency-ordered writes, 0.49.0); what the prefix would corrupt is only a *derived* record, which D-3
    never derives from an incomplete run.
- **D-7 — Three lease classes** (per repository store; one writer process per store):

  | Class | Operations | Excludes | Allowed with |
  |---|---|---|---|
  | **R** | `tree`, `log`, `show`, `cat`, `diff`, `verify`, `branch`, `status`, `sync have`, `sync pending`, `bundle export` (all measured, U-16) | nothing | R, X, W |
  | **X** | `archive export` | W, X | R |
  | **W** | `sync accept`, `bundle import`, `archive import`, `doctor` repairs, `unlock`, `ref complete`, `compact`, `format upgrade` | X, W | R |

  **Changed from the second edition:** readers no longer wait for or exclude writers: nothing failed in
  thousands of reads during accepts (U-16), so the writer-preference wait and the "retry once" rule are gone.
  A read made during a write may see a prefix of the accepting patches as pending; no record is derived from
  such a read (D-3), and the page never says "verification failed" for a state we know is mid-write (the
  repository shows "updating"). prikk fails the second of {export, accept} to start fast (20–53 ms): planeter
  **never lets prikk be the arbiter**: it takes the lease first, waits a bounded time, and answers 503 with
  `Retry-After` past the bound, so a conflict is a queue, not an error. A multi-replica deployment needs
  repository affinity (one writer node per store), not a shared filesystem: prikk documents nothing about
  shared or network filesystems.
- **D-8 — Freshness and split-view evidence.** prikk has no clock; RefStates carry `update_seq` and
  `previous_ref_state_id`. The client helper **pins** the highest `update_seq` and RefState id it has seen
  per ref and refuses a served tip that is lower or not a descendant. The forge exposes a ref's recent
  RefState chain (small) so that independent monitors can compare. Gossip and transparency logs are out of
  scope; withholding the newest tip from all clients of one source is undetectable without a second source,
  and the documentation says so.
- **D-9 — SSH.** `AuthorizedKeysCommand` looks up a key by fingerprint and returns
  `restrict,command="planeter ssh-shell --key <fingerprint>" <key>`; no pty, no forwarding. A closed
  grammar (`advertise <repo>`, `fetch <repo> <kind>`, `push <repo>`) carries the **same artifacts and
  framing as HTTP**; every verb goes through `authorize()`; the ENF-4 enumeration test exists first (IS-14).
- **D-10 — The client helper.** A standalone `planeter` CLI (prikk has no remote-helper mechanism). It
  drives the user's own prikk and keys and holds none. **First clone:** download → local `archive verify` →
  `init` → show the carried maintainer key ids and fingerprints → **explicit** `trust maintainer add` by the
  user, taking public keys *through the user's own channel*, not only from the archive's inert record →
  `archive import --adopt`. **Fetch:** conditional GET → verify → `archive import` (lands under `remotes/`) →
  the user merges (`merge --from`) until RFC 154's act exists. **Push:** advertise (have for the target) →
  `prikk sync build <ref> --have` → upload; the result names claim ids and the revision. A pusher needs a
  maintainer key adopted in their *own* repository (`prikk setup` provides it).
  **Verbs say what happens (owner philosophy: users must not be confused).** The helper's commands are
  `planeter clone`, `planeter fetch` and **`planeter propose`**, not `push`: on this forge a submission
  becomes a *change* for review and does **not** move a branch, and a word that promises otherwise would
  mislead. Its output says exactly what happened, in plain words and with the next step (for example:
  "Proposed as change #7 (revision 1). Your work is held for review; no branch on the forge has changed.
  Reviewers can see it at <url>."). Refusals state the reason in one sentence and what the user can do
  about it; prikk's verbatim text is available behind `--verbose`, never the only message.
- **D-11 — The HTTP envelope** (versioned under `/api/v1`, described in OpenAPI like the read API):
  advertise (refs, fingerprint, sizes), fetch (`kind`, `Range`, `ETag`), push (size-capped upload, result
  body), change artifact relay (RFC 005 D-8), ref chain. Errors: 401/403/404 existence-neutral; 413 over
  bound; 429 rate; 409/503 with `Retry-After` for lease waits; 422 with prikk's named refusal. **One error
  shape** everywhere: `{ code, message, hint }` where `code` is stable and machine-readable, `message` is a
  plain-language sentence and `hint` says what to do next (STD-3: a single error shape).
- **D-12 — Instrumentation from the first deployment.** Per fetch: changed vs unchanged, kind, bytes, wall
  time, lease wait; per push: bytes, refusal reason; per repository: export count and spool hit rate. These
  are the numbers prikk asked for (PK-47) and the evidence for the owner's decision on the fallback (D-13).
- **D-13 — Floors and the fallback.** The accept path requires prikk ≥ 0.52.0 (the re-baseline floor);
  archive serving requires the 0.52.0 closure fix. If measurement shows whole-artifact cost unacceptable for
  developer clones after D-2 and RFC 006's snapshots, the options are, in the owner's hands: wait for a delta
  form (decided by prikk on our numbers), or an opt-in forge key (which reopens OQ-1(a)). **This RFC builds
  neither.**

## Alternatives considered

- **Stream the export to the client.** Rejected (U-5): a slow client holds all five locks.
- **Relay pushers' artifacts as a fetch delta log.** Rejected for fetch: it covers only the unsealed stream,
  and receivers would re-seal under their own keys, losing the maintainers' signatures that make this forge
  worth having (kept only as the *change artifact relay*, RFC 005 D-8).
- **`sync summary`-based change detection.** Rejected: it lists `heads/*` only; our own fingerprint does it.
- **A forge-held transport key for `sync build`.** Not recommended; owner decision (D-13).
- **Shared filesystem for horizontal scale.** Not supported by prikk's documents; affinity instead (D-7).
- **Range-fetch over the archive.** Gated on a contract prikk has declined to make yet (PK-38).

## Security considerations

New inbound flow with stored consequences (TB-3, T-3, T-4, T-9): bounded at every edge (D-5); accept in the
confined subprocess (INV-3); no content-provided code runs; authorization on every verb (INV-4) including
SSH; spool and retained artifacts are untrusted bytes served as attachments from the isolated origin; export
lock-hold DoS is removed by spooling (D-2) and by the X class (D-7); lock and kill recovery is written down
(D-6); first-clone trust is explicit (D-10). INV-2 and ENF-2 are unchanged: no signer exists in the default
build. The threat model is updated in the same release (v0.4).

## Compatibility and migration

Additive. `MIN_PRIKK_VERSION` is already 0.52.0. v1 handoff superseded, no v1 code exists. The HTTP
envelope is versioned (`/api/v1`) from the first release.

## Operational considerations

Spool and artifact-store disk budgets and eviction; lock-conflict, kill and recovery counters; the
instrumentation of D-12; the container recipe needs a spool volume. Backup (OPS-3) includes the retained
artifacts and excludes the spool cache.

## Testing and verification

- Real-binary tests (prikk on `PATH`, visible skips otherwise; both runs named): push → revision;
  tampered artifact refused; over-bound refused at the edge and by prikk (exit 1); idempotent re-push.
- **Concurrency:** a client throttled to 1 KiB/s mid-download does not delay a push (spool); two pushes
  serialize; export and push never both hold locks.
- **Kill:** `SIGKILL` an accept at several points; the repository is `needs-recovery`; the recovery order
  restores service; the retry is idempotent.
- **Serve gate:** the 0.51.0 defect fixture (an archive that fails `verify`) is never served.
- **Freshness:** a served tip with a lower `update_seq` is refused by the helper.
- **Authorization:** ENF-4 enumeration; `propose` default-deny; SSH grammar refuses everything else.
- **Bounds:** the memory budget holds at the measured *k* for sizes 1/16/64/200 MiB.

## Acceptance criteria

Clone, fetch and push work over HTTPS and SSH on the reduced scope; the first clone requires explicit key
adoption and lands nothing as a branch without it; no export is ever streamed under a lock; a kill leaves a
recoverable repository and the procedure is exercised in a test; every bound of D-5 has a refusal test; the
instrumentation of D-12 is present; ENF-2 and ENF-4 pass; `planeter-03` is updated.

## Implementation boundaries

`planeter-transport`; the sandbox profile and exchange verbs in `planeter-prikk`; the lease and artifact
store in `planeter-core`; configuration in `planeter`; the client helper as a new binary crate (publication
follows the crate policy). **Not** `planeter-auth` beyond the `propose` action, **not** any signing code.

## Release implications

A new inbound flow, a new action and a new binary: threat model v0.4, a security audit pass and a documented
deployment shape (spool volume, trusted proxy, host OpenSSH) before the release (project rule).

## Risks and open questions

- **R-1.** ~~The measured *k* or the lock behaviour differs from the documents~~ Measured (U-12, U-16, U-17): *k* ≈ 4.2, locks as drafted, no key directory. Residual: the numbers change per prikk release (re-measured at each re-baseline, RFC 011 D-1); the 100,000-patch point is extrapolated, not measured.
- **R-2.** Whole-artifact cost proves unacceptable for developer clones: D-13, the owner's call, on D-12's data. Nothing is decided now.
- **R-3.** Accept time per patch varies about 2× between runs on the same host and doubles under load (U-12): the 600 s default deadline and the 20,000-object bound leave that margin; an operator on slow storage lowers the object bound.

### Open questions, reviewed 2026-10-21 against the owner's philosophy ("clean, safe and secure, robust and sophisticated; users must not be confused")

- **Q-2 — decided (architect): the first clone never skips key adoption.** A clone made without adopting a key
  cannot be verified, and a quiet "trust me" for public repositories is exactly the confusion the philosophy
  forbids. Two explicit, honest options replace the proposed skip: `planeter clone --trust-key <fingerprint>`
  (non-interactive: the user supplies the fingerprint through their own channel; the helper checks it against
  the key carried by the archive and adopts only on a match) and `planeter clone --no-adopt` (inspect without
  trust: everything lands under `remotes/`, and the output says in one sentence that **nothing is verified or
  trusted**). The default is the interactive adoption of D-10.
- **Q-3 — decided (architect): initial defaults from the measurements, all operator-configurable and tuned by D-12's
  data:** the accept edge bound `PRIKK_EXCHANGE_MAX_BYTES` 256 MiB (prikk's own default) and objects 20,000 (D-5);
  accept deadline 600 s; concurrent exports per host 2; spool budget 25 % of the spool volume with LRU eviction;
  rate limits per principal per repository: pushes 20 per hour, conditional fetches 120 per minute, 429 with
  `Retry-After`. The per-principal quotas are RFC 005 D-9's.
- **Q-1 — the one owner question left (decision D-8, "what a branch is while publication is unavailable"); it does
  not block accepting this RFC**, which is correct under every answer. See the summary below and RFC 005 Q-4.

#### D-8 in plain words, with the architect's recommendation
prikk cannot advance a branch that already exists on a keyless receiver (`archive import --adopt` lands only refs that
are absent; RFC 154, which would, is unscheduled; the forge holds no key and must not). So today a change can be
**proposed, reviewed, downloaded and sealed by a maintainer elsewhere**, but the forge's own `main` does not move.
Three honest models: **A (strict)** `main` stays as created; a project that wants a new `main` re-creates the
repository from a fresh archive. Simple and true, and it makes the forge a review tool, not a home for a project.
**B (recommended)** the forge also *receives* the maintainers' sealed history as a received ref and **selects it as the
branch tip by a written rule** (a key adopted by an admin, a fast-forward of the previous tip, verified): no key on the
forge, the selection is re-derivable and labelled on every page ("received and accepted by this forge by policy;
verified by prikk"). It keeps OQ-1(a) and makes changes able to *land*. **C** a forge-held signing key: rejected, it
breaks the keyless forge the whole design stands on. **B needs one measurement first** (does a newer bundle or archive
update an existing received ref on a keyless forge, and what do `log`, `tree` and `extract --ref remotes/…` then read?)
and a wording ruling on INV-6/NG-6; that is a short investigation under RFC 008's revision.

## Sequencing and handoff

Needs RFC 010's gates (floor, pins), RFC 011 (admission, ceilings, child lifetime: done) and the measurement results (A1–A12: done). On acceptance the architect
writes handoff v2 in build order: lease and artifact store → sandbox profile mounts and limits → advertise
and fetch (spool, verify, cache) → accept and change derivation (with RFC 005) → recovery → HTTPS surface →
SSH → client helper → threat-model update. Everything stays v0.x; 1.0 is the owner's gate.
