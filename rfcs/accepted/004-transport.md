# RFC 004 — Transport: clone and push as a ferry over prikk's artifact exchange

**Status.** Accepted (2026-09-15) — opens M2 (clone/push). The design is settled and the implementer may
build the transport surface and the client helper against it per the handoff. Defines *how bytes move
between a developer and planeter* — by carrying prikk's own `bundle`/`sync` artifacts over an
authenticated, encrypted envelope, so **planeter invents no prikk wire protocol and holds no signing
key**.
Handoff: [`../handoffs/004-transport/transport-handoff-v1.md`](../handoffs/004-transport/transport-handoff-v1.md).
**Tracks.** ROADMAP Phase A2 → M2 (Track A). Requirements `CAP-2`, `INT-2`, `STD-1`, `NG-2`, `BN-4`;
external design `TX-01…06`, `FL-02/03`, `CT-01/02`; internal design `WR-1/2/3/6`, `CON-1/3`, `PKI-2/3`;
threat model `T-4/T-9`, `INV-2/3`, `C-4`. Consumes RFC 002 `authorize()` and RFC 001's driver.
**Touches.** `planeter-transport` (the fetch/push endpoints, the accept orchestration, per-repo
serialization) and a **client helper** (a `planeter` client / prikk remote-helper). Over `planeter-prikk`
(the `bundle`/`sync` verbs) and `authorize()`. **Not here:** the **seal/merge** (advancing a ref by a
maintainer signature) — that is the change/review model (RFC 005) and the one-click seal (RFC 008/UD-6);
this RFC stops at *accepted + verified + pending*. No CI triggers (RFC 006).

## Revision 2026-09-16 — corrected to the prikk RFC 154 / 155 model

prikk resolved the trust-model question this RFC skirted. The corrections below supersede the affected
decisions; **implementation is gated on the prikk binary shipping**, order after 0.43.0: key-id fix →
RFC 155 (repository-complete artifact) → RFC 154 (adoption).

- **Canonical branch by *adoption*, not by the forge sealing (prikk RFC 154, accepted).** The forge holds
  its canonical branch by **adopting a trusted-maintainer-signed fast-forward advance** — signed by a key
  the forge has adopted, a fast-forward of the local tip, every block replaying+verifying, local CAS
  holding — **keyless and multi-maintainer**. This replaces any notion of the forge advancing a ref by
  sealing (supersedes the seal-side of D-3/D-4). *Whose `main`*: first fast-forward wins; a losing
  maintainer adopts the winner, merges on top, re-sends → one signed chain, no forge key.
- **Fetch / clone / serve via the RFC 155 repository-complete artifact + `import --adopt`** (supersedes
  D-2's `bundle export` of a received ref, which prikk measured impossible — a received ref stays under
  `remotes/`, unservable). Default import lands refs under `remotes/`; `--adopt` lands maintainer-signed
  refs as local branches under RFC 154's rule, inside one all-or-nothing import.
- **Push is unchanged and correct:** keyless `sync accept` → accepted + verified + pending (the accept
  half of D-3 stands).
- **New planeter responsibilities (a keyless forge can *deny*, not *forge*):** planeter owns **freshness**
  (detecting a withheld/stale tip — prikk has no clock or network) and **split-view detection**
  (different clients shown different signed tips; prikk reports two trusted successors of one state as a
  refusal naming both, but cross-client comparison is transport's). Add both to the transport surface.
- **Distinct maintainer key-ids** are required (prikk fixes the default-`maintainer` collision first).
- **Minimum prikk version ≥ 0.43.0 for transport.** `bundle export` / `sync build` only handle ordinary
  *delete-after-edit* histories from prikk 0.43.0 (refused 0.28.0–0.42.0); a forge serving real repos
  needs this, so 0.43.0 is planeter's transport floor (dependency-ledger PK-22). Note also that
  **RFC 154 and RFC 155 are accepted but *not shipped* in 0.43.0** — implementation still waits.

## Summary

prikk deliberately does not move bytes over a network (RFC 115/116: "prikk stays off the network"); its
`sync` is **negotiation-as-artifacts** precisely so a transport layer can carry it. planeter is that
layer. This RFC gives developers the familiar **clone** and **push** by ferrying prikk's artifacts over
**HTTPS/SSH**, adding only what prikk lacks — TLS, authentication, per-ref authorization, and repository
addressing. The load-bearing property, from `INV-2`: a push runs **`sync accept`, which ingests
author-signed patches with no maintainer key**, so the transport surface — the most exposed, most-pushed
part of the forge — **holds and needs no signing key**. Turning accepted patches into a sealed ref (a
merge) is a separate, key-bearing step owned by RFC 005/008; this RFC delivers everything up to it.

## The constraints that scope this design

- **No prikk wire protocol exists, and inventing one is refused** (`NG-2`, RFC 116 deferred it): planeter
  carries prikk's artifacts, it does not define prikk's network semantics.
- **prikk's write path is two-phase** (survey): `sync build`(client) → `sync accept`(server) ingests
  author-signed patches as **accepted-but-unsealed** claims (keyless, prints claim ids); only
  `sync seal <ref> --claim` needs a maintainer key. This RFC lives entirely in the **keyless** phase.
- **prikk artifacts are unencrypted** ("move only over a channel you trust"): planeter *is* the trusted
  channel — **TLS is mandatory**, no plaintext transport (`STD-1`).
- **A pushed artifact is untrusted input** (`T-4`): it is parsed by the confined prikk subprocess
  (RFC 001 D-3), bounded, and refused-not-crashed on malformed input.
- **prikk multi-user concurrency is undesigned** (`UD-4`): planeter must serialize writes per repo above
  prikk's local locks — and this RFC is where that sufficiency is confirmed (`IQ-3`).

## Decisions

- **D-1 — Transport is a ferry; the envelope is planeter's, the payload is prikk's.** planeter defines
  the **HTTP(S) and SSH envelope**: TLS, authentication (RFC 002 principals), per-ref authorization, and
  repository addressing (`owner/name` → the `.prikk`, RFC 001 D-4/D-5). Inside it travel prikk's opaque
  artifacts (`PSYNCSU1` summary, `PSYNCHV1` have-list, `PEXCH002` exchange, `bundle`). planeter never
  interprets or rewrites the payload (`INT-2`/`NG-2`).
- **D-2 — Fetch / clone = server builds, client accepts (WR-1).** An initial clone streams a
  `bundle export` the client `bundle import`s. Incremental fetch: the client sends its `have`
  (`PSYNCHV1`); the server runs `sync build <ref> --have <client-have>` → `PEXCH002`; the client
  `sync accept`s it locally. Reads are gated by `authorize(principal, read, repo)` (RFC 002 D-5):
  `public` anonymous, `private` requires a grant. Fetch needs no key on either side.
- **D-3 — Push = authorize, then keyless `accept`, then verify (WR-2/WR-3).** The client builds a
  `PEXCH002` against the server's advertised `have`/`summary` and uploads it over TLS. planeter:
  (1) **authorizes** the target ref — `authorize(principal, push, ref)` (RFC 002 D-6/D-7), protected-ref
  and scope rules applied; (2) runs **`sync accept`** in the confined prikk subprocess — ingesting the
  **author-signed** patches as accepted-but-unsealed claims, **with no maintainer key**; (3) runs
  **`verify`**, and **rejects with a named reason** any push whose patches fail prikk's own verification
  (a bad push is never silently stored). The forge signs nothing (`INV-2`/`ENF-2`).
- **D-4 — A push produces *accepted, verified, pending* — and stops there.** The result of D-3 is
  author-signed patches ingested and verified, listed by `sync pending` as claim ids, **not yet sealed
  onto a ref**. Whether those claims then advance a ref (a maintainer *merge*, sealed by the maintainer's
  **own** key — OQ-1 a) or surface as an open change is the **change/review model (RFC 005)**; the
  one-click client-side seal is **RFC 008 (UD-6)**. This RFC deliberately does not seal, auto-advance a
  ref with a forge key, or presume the seal's location — that would breach `INV-2` or pre-empt UD-6.
- **D-5 — The client helper (TX-05).** Because prikk has no `clone`/`push` verb, a thin **client helper**
  — a small `planeter` client or a prikk remote-helper — turns "clone/push `<url>`" into the artifact
  round trip (advertise → `have` → `build` → upload/`accept`). Its *contract* is fixed here (the
  negotiation sequence and the envelope); its *form* (standalone CLI vs. remote-helper integration) is a
  handoff choice. The developer experience is familiar; the artifact exchange is invisible.
- **D-6 — Two transports, both encrypted (STD-1).** **HTTPS** (the artifact round trip over TLS) and
  **SSH** (the same, authenticated by a registered public key mapping to a user, RFC 002 D-7). No
  plaintext, no `git://`-style unauthenticated path. Write endpoints require authentication; public
  fetch may be anonymous.
- **D-7 — Pushed bytes are untrusted and bounded (T-4/INV-3).** The uploaded artifact is parsed only by
  the **confined** prikk subprocess (RFC 001 D-3): filesystem-scoped, no network, resource-bounded.
  planeter enforces **size, object-count, path-depth, and decompression-ratio ceilings** before/around
  the parse; a ceiling hit or malformed artifact is a **refusal with a reason**, never an OOM, a hang, or
  an out-of-repo write. planeter never executes content-provided code.
- **D-8 — Per-repo write serialization, idempotent by claim (CON-1/CON-3, UD-4).** `accept`s for one
  repository are **serialized** by a per-repo write lease in `planeter-transport`, so two pushes never
  race the prikk subprocess; reads run concurrently. prikk's local locks sit beneath — **this RFC
  confirms their sufficiency under the serialization (`IQ-3`/`UD-4`)**. A retried upload is deduplicated
  by claim id (idempotent), so a network retry does not double-apply.

## What "done" means (acceptance criteria)

- **Clone + fetch** work over **HTTPS and SSH** against a running planeter: initial clone (bundle) and
  incremental fetch (have→build→accept); read-authorized (`public` anonymous, `private` gated); TLS
  enforced, plaintext refused.
- **Push** works: client build → upload → `authorize(push, ref)` → `sync accept` (keyless) → `verify` →
  the claims appear in `sync pending`; a push failing authorization or verification is **rejected with a
  named reason**; **the ENF-2 symbol-absence check still passes** (no seal/forge-key path added).
- **Untrusted-input handling (tested):** an oversized / malformed / decompression-bomb artifact is
  refused (bounded, reason given); the prikk parse runs confined (no out-of-repo write, no socket).
- **Concurrency (tested):** two concurrent pushes to one repo are serialized and both resolve correctly;
  a retried upload is idempotent by claim id; prikk local-locking sufficiency confirmed (`IQ-3`).
- **The client helper** clones and pushes against a running planeter with a familiar `<url>` UX.
- Threat model re-verified: `T-4` controls present (confined parse, bounds), `INV-2` untouched (no
  signing path), `INV-3` holds; `planeter-03` updated (this RFC adds a new inbound data flow — the push
  endpoint).

## Alternatives considered

- **Implement a prikk smart wire protocol.** Rejected (`NG-2`, RFC 116): the network semantics are
  prikk's to define, not planeter's; ferrying the existing artifacts needs no prikk change and keeps
  prikk lean (`BN-4`).
- **Auto-seal on push with a forge key (advance the ref server-side).** Rejected (`INV-2`): that puts a
  history-signing key in the most-attacked component. The seal is client-side (RFC 005/008); a push stops
  at accepted+pending.
- **Accept first, verify later (lazily).** Rejected (`WR-3`): verify on accept so a bad push is rejected
  immediately, not discovered later; the pending state is always verified.

## Open questions & dependencies

- **UD-4 / IQ-3 (prikk)** — prikk local-locking sufficiency beneath planeter's per-repo serialization;
  **confirmed by D-8's tests** in this RFC's implementation.
- **RFC 005 / RFC 008 (UD-6)** — how accepted-pending claims become a sealed ref (merge), and the
  one-click client-side seal. This RFC hands off at *accepted + verified + pending*.
- **Client helper form** (standalone CLI vs. prikk remote-helper) — handoff decision; the contract (D-5)
  is fixed regardless.

## Sequencing & handoff

Builds on RFC 001 (driver, hosting, confinement, ENF-2) and RFC 002 (`authorize` for read/push). On
acceptance, the architect writes `handoffs/004-transport/` (the HTTP/SSH endpoint shapes, the
negotiation sequence, the client-helper form, the bounds/confinement specifics, the per-repo lease and
idempotency, and the tests above — plus the `planeter-03` update for the new push data flow). Delivering
this reaches **M2 (0.2.0)** — a live host you clone from and push to, keylessly. Then RFC 005 turns
accepted-pending claims into review and merge for M3. All v0.x; 1.0 is the owner gate.
