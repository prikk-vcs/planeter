# RFC 004 — Transport — Implementation Handoff (v2, part 8): the forge's have-list over HTTP

| | |
|---|---|
| Document | **A gap found while planning the client helper.** RFC 004 D-10 says a push is "advertise (have for the target) → `prikk sync build <ref> --have` → upload". The advertisement built in part 2 carries refs, the state fingerprint and the fetches on offer — **not the have-list** (`prikk sync have <ref> --output`, the `PSYNCHV1` file that tells a sender what the receiver already holds). Without it a client cannot build an incremental artifact over HTTP; its only choices would be a full build (wasteful, and past the object bound for any real history) or a hidden out-of-band step. This part adds one read route that serves the have-list. |
| Status | **Assigned** (2026-10-29). Must be done before the client helper (`client-helper-handoff-v2.md`). |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-10, D-11; ledger PK-27 (`sync have` runs keyless), PK-59, PK-72/PK-74 (output sizes); part 2's advertise/fetch routes, spool, single flight, rate limits, ETag and fingerprint (`export.rs`, `transport.rs`); `CliPrikkRepo::sync_have`; the output cap table (`output_cap.rs`: `sync have` is in the 1 MiB class — **check that is right for a 20,000-patch ref**); the route registry (IS-14); `docs/propose.md` check order. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T0 — Measure first

With real prikk on a repository with 1, 1,000, 5,000 and 20,000 sealed patches on `heads/main`, and the same plus 5,000 *pending* patches: the **size of the have-list file**, `sync have` wall time and peak `VmData`, and whether the have-list covers **pending** (accepted, unsealed) patches or only the ref's sealed history (it matters: a proposer who re-sends a pending patch is harmless, but one who cannot learn that a patch is already held may send it again — report the facts). Fix the cap-table row for `sync have` from the numbers (it is a file written with `--output`, not stdout: say which stream/limit the driver actually bounds) and the response size bound of the route.

## Change scope

- **T1 — The route.** `GET /api/v1/repos/{owner}/{name}/transport/have?ref=NAME` → the `PSYNCHV1` file as `application/octet-stream`, `Content-Length`, `ETag` (the ref's state: the fingerprint plus the ref name), `Cache-Control: private, no-cache` and conditional `If-None-Match` → `304`. Registered through the route registry as `Authorized(Read)` (the same existence-neutral 404 for hidden/absent repositories and unknown refs as the advertisement; a ref that exists only as a received ref is a named refusal, not a have-list: only a **published** ref has one). Read-lease class **R**; computed with the sandbox under the admission rule of the read verbs (measure in T0 and use the table's row; heavy pool unless the numbers say light); produced into a per-request working directory outside the repository (the export pattern), served from a spool file keyed by (repository, ref, fingerprint) with **single flight and the same spool budget and LRU** as artifacts; never held under a lease while the client downloads. Rate limits as the other transport reads. The advertisement gains, per published ref, a `have` offer (`kind=have`, size, ETag) beside the existing offers, so a client discovers it; the OpenAPI document and drift guard follow; one error shape; catalog sentences for any new refusal (English and a Japanese draft row in the next addendum).
- **T2 — The cap and the cost.** `sync have`'s output limit and admission cost come from T0, in the existing tables with provenance (`output_cap.rs`, `CostRule`), with the table tests; the route is covered by the "every verb has a row" tests.
- **T3 — Tests.** Real-prikk end to end: a keyed client builds an artifact against the **served** have-list (not a locally made one) and `propose` accepts it; the conditional request; a received-only ref is refused; a stranger/reader/anonymous matrix as for the advertisement; the kill/abort of the export leaves nothing half-served (the artifact-serving rule: never serve what did not verify — the have-list is small, but the file is written then renamed); a mutation per rule (published-only filter; ETag ignoring the ref; served under a lease).
- **T4 — Documentation.** `docs/` (the route, its place in the push flow, its cost), CHANGELOG, STATUS, OpenAPI text.

## Non-change scope

No change to `propose`, fetch, recovery or review; no new dependency; no `unsafe`; no version or release change.

## Acceptance criteria

T0's table; the route works end to end with real prikk as T3 says; IS-14 table, OpenAPI drift guard, ENF-2 and all earlier checks pass; every gate green (fmt, floor, pin, clippy, both test runs named, deny/audit as they are while IS-54 is open, ENF-2, doc), CI.

## Prohibited shortcuts

No have-list for a received ref; no guessed bound; no export under a lease; no unbounded read; no hidden/absent/denied difference a stranger can see.

## Required evidence and review-request format

Commits (hash → what, by task); T0's table and scripts' location; the end-to-end transcript (client builds against the served have-list, propose accepts); the matrix; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
