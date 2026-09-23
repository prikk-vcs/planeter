# RFC 004 — Transport — Implementation Handoff (v1)

| | |
|---|---|
| Document | Companion execution doc for RFC 004 (Transport). Task/PR plan + QA checklist. |
| Status | Inherited from RFC 004 — **Accepted**. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) (decisions D-1…D-8); RFC 001 (driver, confinement, ENF-2), RFC 002 (`authorize` read/push); `STD-1`, `INT-2`, `NG-2`; threat model `T-4/T-9`, `INV-2/3`, `C-4`. |
| Audience | Dev team. Return a review-request package when green. |
| Scope | **Phase A2 → M2.** The clone/push transport and the client helper — prikk's **keyless** phase only. **Out of scope:** seal/merge (RFC 005/008), CI triggers (RFC 006). Build on A0 + A1. |

> **Held 2026-09-23 (owner ruling) — do not start.** Measured on prikk 0.46.0 (dependency-ledger
> PK-27/28/29): the push path is fully keyless and robust, but **`sync build` requires a maintainer
> key** and a planeter-created repository has no servable branch until RFC 154 adoption — so clone/fetch
> cannot land keylessly. A2 waits for prikk to ship RFC 155 then RFC 154 — **after 0.49.0** on prikk's
> schedule of 2026-09-23 (`sync build` stays keyed, ruled; keyless fetch is RFC 155's verbatim path) —
> and a **v2 handoff**. Two 0.47.0 facts for that v2: `sync accept` refuses over-size input before
> reading (exit code 1, new wording — key on the code) and the bound is `PRIKK_EXCHANGE_MAX_BYTES`,
> which the clearing sandbox must set explicitly. Two decisions already taken for that v2: **TLS is proxy-terminated
> (loopback HTTP + trusted-proxy rule) and SSH is host OpenSSH `ForceCommand` → `planeter ssh-shell`** —
> no in-process rustls/russh (measured: +165 crates, deny + audit failing). T1/T2 below are superseded
> accordingly; T3–T8 stand in substance.
>
> **Revised 2026-09-16 — gated on the prikk binary.** The prikk trust-model resolution (RFC 154 trusted
> fast-forward adoption, *accepted*; RFC 155 repository-complete artifact, *proposed*) reshapes the
> **fetch/serve** and **canonical-branch** work: fetch/serve go via the RFC 155 artifact + `import
> --adopt`, and the forge's canonical branch advances by **adopting** a trusted-maintainer-signed
> fast-forward (RFC 154), not by the forge sealing. The **keyless push = `sync accept`** task stands. New
> tasks: **freshness** and **split-view detection** (a keyless forge can deny, not forge). Implementation
> of the gated parts waits on prikk shipping (post-0.43.0: key-id fix → RFC 155 → RFC 154); a **v2 handoff
> follows** when the binary lands. See RFC 004 §Revision 2026-09-16.

## Task breakdown (PR plan — build in this order)

- **T1 — HTTPS transport endpoints (D-1/D-3/D-6).** In `planeter-transport`: the advertise/`have`,
  upload, and build/fetch endpoints, over **TLS only** (plaintext refused). Authenticate via RFC 002
  principals (token / session / SSH-mapped). Address repos by `owner/name` → `.prikk` (RFC 001).
- **T2 — SSH transport (D-6).** The same negotiation over SSH, authenticated by a **registered public
  key → user** (RFC 002 D-7); a deploy key scoped to one repo. Restrict the SSH command surface to the
  transport verbs.
- **T3 — Fetch / clone (D-2).** Clone: stream `bundle export`, client `bundle import`. Incremental: client
  `have` (`PSYNCHV1`) → server `sync build <ref> --have` (`PEXCH002`) → client `sync accept`. Gate with
  `authorize(_, read, repo)`; `public` anonymous, `private` grant-required. All via `planeter-prikk`.
- **T4 — Push = authorize → keyless accept → verify (D-3/D-4).** Client uploads a `PEXCH002`. planeter:
  (1) `authorize(_, push, ref)` (protected-ref + scope, RFC 002 D-6/D-7); (2) `sync accept` in the
  confined subprocess — **no maintainer key**; (3) `verify`; reject with a **named reason** on failed
  authz or verify. Result: claims in `sync pending` (accepted + verified). **Do not seal, do not advance
  a ref with a forge key** — that is RFC 005/008.
- **T5 — Untrusted-artifact bounds + confined parse (D-7, T-4/INV-3).** Enforce ceilings — artifact
  size, object count, path depth, decompression ratio — before/around the parse; run every prikk
  invocation under RFC 001 D-3 confinement (fs-scoped, no network, resource/time bounds). Malformed /
  over-ceiling / bomb → **refusal with reason**, never OOM, hang, or out-of-repo write. Execute no
  content-provided code.
- **T6 — Per-repo write lease + idempotency (D-8, UD-4/IQ-3).** Serialize `accept`s per repository with a
  write lease (reads stay concurrent); dedupe a retried upload by claim id. **Confirm prikk local-locking
  sufficiency** beneath the lease and record the finding (this closes `IQ-3`); if insufficient, raise it
  before proceeding.
- **T7 — The client helper (D-5).** A small `planeter` client (or prikk remote-helper) turning
  "clone/push `<url>`" into the negotiation (advertise → `have` → `build` → upload/`accept`). Choose the
  form (standalone CLI vs. remote-helper) and record it; the contract (the sequence + envelope) is fixed.
- **T8 — Threat-model update.** Push is a **new inbound data flow**; update `planeter-03` (confirm `T-4`
  controls present, `INV-2` untouched, `INV-3` holds) per the project release rule.

## QA checklist (from RFC 004 acceptance)

- [ ] Clone + fetch work over **HTTPS and SSH**; read-authorized (`public` anon, `private` gated); TLS
      enforced, plaintext refused.
- [ ] Push: build → upload → `authorize(push,ref)` → `sync accept` (keyless) → `verify` → claims in
      `sync pending`; failed authz/verify **rejected with a reason**; **ENF-2 check still passes** (no
      seal/forge-key path).
- [ ] Untrusted-input (tested): oversized / malformed / decompression-bomb artifact refused; parse runs
      confined (no out-of-repo write, no socket).
- [ ] Concurrency (tested): two concurrent pushes to one repo serialize and both resolve; retried upload
      idempotent by claim id; prikk local-locking sufficiency confirmed (`IQ-3`).
- [ ] The client helper clones and pushes against a running planeter with a familiar `<url>` UX.
- [ ] Gates green; `planeter-03` updated for the push data flow.

## Definition of done & handback

Done when the checklist is green — **A0 + A1 + this = M2 (0.2.0)**: a live host you clone from and push
to, keylessly. Return the review-request package naming the PRs, the client-helper form (T7), and the
`IQ-3` finding (T6). The architect proceeds to **RFC 005 (change/review)** — turning accepted-pending
claims into review and merge for M3. All v0.x; 1.0 is the owner gate.
