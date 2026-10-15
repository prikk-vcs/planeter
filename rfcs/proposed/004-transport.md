# RFC 004 — Transport: clone and push as a ferry over prikk's artifact exchange (second edition)

**Status.** Proposed — **second edition, drafted 2026-10-15** for the owner's reading; the implementer must
not start. It replaces the body of the 2026-09-15 acceptance (v1 and its 2026-09-16 revision are in this
path's git history). It is **not accepted** while the rows marked *unconfirmed* in §Upstream facts remain
unconfirmed or the owner has not accepted that risk in writing (RFC 010 D-8): the transport measurement
handoff ([`../handoffs/004-transport/transport-measurement-handoff-v1.md`](../handoffs/004-transport/transport-measurement-handoff-v1.md))
produces that evidence. Handoff v1 is superseded; **handoff v2 is written on acceptance.**
**Tracks.** ROADMAP Phase A2 → M2 (reduced; see §Scope); requirements `CAP-2`, `INT-2`, `STD-1`, `NG-2`,
`OPS-3`; external design `TX-01…06`, `FL-02/03`, `OP-01/03`; internal design `WR-1/2/3/6`, `CON-1/3`,
`PKI-2/3`; threat model `T-3`, `T-4`, `T-9`, `INV-2/3`, `C-4`; RFC 002 (`authorize()`, new action `propose`),
RFC 005 v2 (the change record, artifact retention), RFC 006 v2 (snapshots), RFC 010 (bounds, floor, gates);
ledger PK-28, PK-32–PK-39, PK-46–PK-50; SCHEDULE decisions D-8, D-9.
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

**Unconfirmed on our configuration** (rows of the measurement handoff): keyless `archive import` under our
sandbox and from a fresh process (A1); the accept memory multiplier *k* (A2); `sync have` states (A3); lock
and kill behaviour under `bwrap --unshare-all` (A7); export time and spool size (A8); reader/writer
interference (A10); a change's derivation (A12, RFC 005).

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
  - **Cache key:** `(repo, kind, ref?, state fingerprint)`; the fingerprint is a revision counter bumped
    under the W lease, cross-checked by the lengths and mtimes of the carried files so that an out-of-band
    write invalidates rather than serves stale.
  - **Serve gate:** an export that fails `archive verify` is never served; per-ref bundles are the fallback,
    else 503 with a named reason and a metric.
  - **Controls:** single flight per key; a bounded number of concurrent exports per host; a disk budget
    with LRU eviction; the spool on a volume separate from the repositories.
  - **Cost model published in the docs:** cost of a changed-state fetch = artifact size; the mitigations
    are listed so that nobody mistakes this for incremental fetch.
- **D-3 — Push = authorize, accept, derive, verify.** `authorize(principal, propose, repo)` (RFC 005 D-9)
  → size and rate checks at the edge → W lease → `sync accept` with the bounds of D-5 → **derive the change
  revision** (RFC 005 D-1, pending-after minus pending-before) → record and retain the artifact → release.
  Refusals carry a named reason (unauthorized, over bound, quota, base unknown, prikk's refusal text kept
  verbatim with its exit status). The result body names accepted claim ids and the revision. A push is
  idempotent by artifact digest. **Push never publishes** (INV-2).
- **D-4 — Repository creation and the first landing.** A repository can be created from a maintainer's
  archive by: `init` (keyless) → `archive verify` → **the creating principal's explicit adoption of the
  listed maintainer public keys** (`trust maintainer add`, `admin` only, shown with fingerprints, never
  inferred from the archive) → `archive import --adopt`. A ref that is not adoptable lands under `remotes/`
  with prikk's reason, displayed. Advancing a published ref after creation is outside this RFC.
- **D-5 — Bounds with mechanisms.**
  1. **Edge:** body size cap before buffering (413); claim and object counts where the header allows;
     per-principal and per-IP rate limits (429, `Retry-After`).
  2. **prikk's own, set in the sandbox:** `PRIKK_EXCHANGE_MAX_BYTES`, `PRIKK_EXCHANGE_MAX_OBJECTS`,
     `PRIKK_BUNDLE_MAX_BYTES`, `PRIKK_BUNDLE_MAX_OBJECTS` through the existing seam, and per repository
     `incoming.max-object-bytes` via `prikk config` (never from content).
  3. **Memory budget:** per-invocation resident memory ≤ *k* × artifact, *k* measured (A2); concurrency cap
     per host = RAM budget / (k × cap). Values are set from measurement, not guessed here.
  4. **Per-invocation limits** (memory, CPU, file size, processes) in the sandbox; the mechanism (a
     `prlimit` wrapper, a cgroup scope, or both) is chosen on A2's evidence and must **kill at the ceiling,
     not degrade**.
- **D-6 — Containment and recovery.**
  - **Sandbox profile gains two mounts outside the repository:** an artifact-in directory (read-only) and a
    spool-out directory (read-write). Today only the repository root is bound (U-11), so staging would write
    into a hosted repository's directory.
  - **Kill policy:** wall-time → `SIGTERM`, grace, `SIGKILL`; a killed *writer* marks the repository
    `needs-recovery` and takes it out of write service.
  - **Recovery, in prikk's own order:** `doctor` (read) → confirm the writer's process is gone (our sandbox
    confirms, not merely signals) → `unlock --lock <path>` for each listed lock → run the same operation
    again, or `doctor --cancel-import <id>`; `ref complete` where `doctor` names an interrupted publication.
    **Our lease is the only gate; `unlock`'s PID advisory is ignored.** Every `doctor` repair's run id is
    recorded (`--recovery-restore` can undo it).
  - Accepts are idempotent, so retry after recovery is safe; dependency-ordered writes (0.49.0) mean an
    interrupted accept leaves an unreferenced remnant, not a hole.
- **D-7 — Three lease classes** (per repository store; one writer process per store):

  | Class | Operations | Excludes | Allowed with |
  |---|---|---|---|
  | **R** | `tree`, `log`, `show`, `cat`, `diff`, `verify`, `branch`, `sync have`, `sync pending`, `bundle export` (confirm, A7) | W | R, X |
  | **X** | `archive export` | W, X | R |
  | **W** | `sync accept`, `bundle import`, `archive import`, `doctor` repairs, `unlock`, `ref complete`, `compact`, `format upgrade` | all | nothing |

  Waits are bounded with writer preference; past the bound: 503 with `Retry-After`. A transient read failure
  during a write (A10) is retried once under R before it is reported, and the honesty display never shows
  "verification failed" for a state we know is mid-write. A multi-replica deployment needs repository
  affinity (one writer node per store), not a shared filesystem: prikk documents nothing about shared or
  network filesystems.
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
- **D-11 — The HTTP envelope** (versioned under `/api/v1`, described in OpenAPI like the read API):
  advertise (refs, fingerprint, sizes), fetch (`kind`, `Range`, `ETag`), push (size-capped upload, result
  body), change artifact relay (RFC 005 D-8), ref chain. Errors: 401/403/404 existence-neutral; 413 over
  bound; 429 rate; 409/503 with `Retry-After` for lease waits; 422 with prikk's named refusal.
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

Additive. `MIN_PRIKK_VERSION` is already 0.52.0. v1 handoff superseded, no v1 code exists. The C-5 envelope
is versioned (`/api/v1`) from the first release.

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

- **R-1.** The measured *k* or the lock behaviour under our sandbox differs from the documents (A2, A7): the
  design holds; the numbers change. If keyless `archive import` needs a key directory (A1), D-4 changes.
- **R-2.** Whole-artifact cost proves unacceptable for developer clones: D-13, the owner's call, on D-12's data.
- **Q-1 (owner, D-8/D-T2).** What a branch is meanwhile; this RFC is correct under any answer.
- **Q-2 (owner, OQ-4).** Whether the client helper's first clone should offer to skip key adoption for
  public read-only use (this draft says **no**: a clone without adoption lands under `remotes/` only).
- **Q-3 (owner).** Quota and bound *values*: set from the measurement handoff.

## Sequencing and handoff

Needs RFC 010's gates (floor, pins) and the measurement results (A1–A12). On acceptance the architect
writes handoff v2 in build order: lease and artifact store → sandbox profile mounts and limits → advertise
and fetch (spool, verify, cache) → accept and change derivation (with RFC 005) → recovery → HTTPS surface →
SSH → client helper → threat-model update. Everything stays v0.x; 1.0 is the owner's gate.
