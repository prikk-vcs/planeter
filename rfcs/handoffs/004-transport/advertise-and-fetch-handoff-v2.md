# RFC 004 — Transport — Implementation Handoff (v2, part 2): advertise and fetch

| | |
|---|---|
| Document | Part 2 of the RFC 004 v2 series (after [`transport-foundations-handoff-v2.md`](transport-foundations-handoff-v2.md), done and approved): the **read side of the transport** over HTTPS — advertise what a repository holds, and fetch it as a **whole artifact that is spooled, verified, cached and conditional** (RFC 004 D-2), with the lease classes of D-7, the HTTP envelope of D-11 and the instrumentation of D-12. Push, accept and the intent journal are part 3; creation from an archive (D-4) is a later part. |
| Status | **Done, reviewed Approved (2026-10-21; commits `a53d955`…`258bc9a`).** Next: `../005-change-review/change-model-and-journal-handoff-v2.md` (part 3a). |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-2, D-7, D-11, D-12 and the measured facts U-15, U-16 (archive export: flat memory, byte-identical for the same state, readers unaffected; export and accept exclude each other, whoever starts second fails); ledger PK-32, PK-37, PK-38, PK-60; part 1 (`planeter-core::lease`, `planeter-transport::artifacts`, `ExtraMounts`, the route registry and the enumeration test); RFC 011 (`CallLimits`, the gate and admission); RFC 003 (error shape, `Authorized(Read)`, existence-neutral 404); the API-messages-by-language machinery of the catalog. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T0 — Two small corrections from part 1's review (first, each its own commit).** (a) `Action::Propose` is **never**
  granted to a `CiJob` principal, with any scope and under either `proposals` setting (a job does not propose changes);
  pin it in the matrix test, with a mutation. (b) The server shuts down **gracefully on SIGTERM and SIGINT**: stop accepting,
  let in-flight requests finish within a bounded drain (operator variable, default 30 s), release the process lock and
  the lease records, exit 0; a second signal exits at once. This needs tokio's `signal` feature (which pulls
  `signal-hook-registry`, a small, widely used crate): **report the dependency review the policy requires** (maintainer,
  size, licence, `cargo deny`/`audit` result, alternatives such as a `libc`-free approach) **before** you add it and
  stop if anything is off. Tests: a real server receives SIGTERM during a slow request (the request completes, the lock
  file is gone), and during idle (immediate).
- **T1 — Advertise.** `GET /api/v1/repos/{owner}/{name}/transport/advertise`, `Authorized(Read)` through the route
  registry (so the IS-14 table and probe cover it), existence-neutral 404 for hidden and absent repositories. The JSON
  names, per ref the forge holds: its name, whether it is **published** (`heads/…`, `tags/…`) or **received**
  (`remotes/…`), the tip block id and `ref_state_id` where `branch --format json`/`log` give them, the repository's
  **state fingerprint** (T3), the available fetch **kinds** for it (`bundle` for a published ref, `archive` for the
  whole repository) with the **last known size** of each (absent until the first export) and the `ETag` a fetch would
  return. No filesystem path appears. Described in the OpenAPI document and covered by its drift guard; errors use the
  one shape `{ code, message, hint }`, texts in the catalog (both languages' machinery).
- **T2 — Fetch.** `GET /api/v1/repos/{owner}/{name}/transport/fetch?kind=archive|bundle[&ref=<name>]`,
  `Authorized(Read)`. Responses: `200` with the artifact (`Content-Type: application/octet-stream`,
  `Content-Disposition: attachment`, `ETag`, `Accept-Ranges: bytes`), `304` for a matching `If-None-Match`, `206` for a valid
  `Range` **within the one current artifact**, `416` for an unsatisfiable range; a `bundle` of a received ref is refused with a
  named reason that points to `archive` (PK-60); `422`-class named refusals for an unknown ref or kind; `503` + `Retry-After`
  (`busy`) when the lease wait or the export capacity is exhausted; `429` on the rate limits (T6). **The artifact is never
  streamed from `prikk` to the client**: it is served from the spool file (T3), so a slow client cannot extend any lock.
- **T3 — Export under the X lease, spool, verify, cache.** On a cache miss: take the **X lease** (bounded wait, RFC 004
  D-7), run `prikk archive export` (or `bundle export --ref`) **into the spool-out mount** (part 1's `ExtraMounts`),
  release X, then **`archive verify`** the spooled file (an archive that does not verify is **never served**: the
  serve gate; if a per-ref bundle can serve the request, offer that, else `503` with a named reason and a counter), rename
  into place, and serve. **Cache key** `(repo, kind, ref?, state fingerprint)`; the fingerprint is a per-repository revision
  counter that planeter bumps under the **W lease** whenever a write completes (nothing writes yet in this part, so it is
  stable: build the counter and its bump API now, with tests), cross-checked on every request against the lengths and mtimes
  of the repository's `.prikk` container files so that an out-of-band write invalidates the cache rather than serving stale
  bytes. Two exports of the same state are byte-identical (U-15), so the fingerprint is a sound `ETag`. **Single flight per key**
  (concurrent requests for the same key wait for one export and share its result, including its failure), a bounded number
  of concurrent exports per host (default 2, operator variable), the spool budget and LRU eviction of part 1, and the
  spool on its own volume when so configured. Export and verify run through the existing sandbox and gate with their own
  deadline (default 600 s, operator variable) and a data ceiling; a ceiling hit or a timeout is a named refusal and never
  a served artifact, and the cache never keeps a partial result.
- **T4 — Reads stay unblocked.** Readers never wait for an export (R is allowed with X, D-7); an export that conflicts with a
  future writer is part 3's concern. In this part prove, with real prikk, that reads (`tree`, `log`, `verify`) of the
  repository succeed while an export runs (the A10-style loop), and that a **client throttled to 1 KiB/s mid-download does
  not hold the X lease** (the lease is free while the bytes drain from the spool).
- **T5 — Instrumentation (D-12).** Per fetch: changed vs unchanged (200 vs 304), kind, bytes, wall time, lease wait, cache
  hit or miss; per repository: export count and spool hit rate; refusals by reason. Counters readable by tests and by a
  future metrics endpoint (none is added); one structured log line per fetch without repository names beyond what the
  access log already carries.
- **T6 — Edge limits.** Per-principal and per-IP rate limits on advertise and fetch (default conditional fetches 120 per minute,
  429 with `Retry-After`; operator variables), reusing the existing throttle machinery where it fits, with the same
  existence-neutral behaviour for hidden repositories (a stranger cannot learn a repository exists from a rate-limit answer).
- **T7 — Documentation and records.** `docs/RELEASING.md` (configuration: spool and export capacity, deadlines, rate limits; the
  cost model sentence of D-2: "the cost of a changed-state fetch is the artifact size; these are the mitigations"), the OpenAPI
  document, CHANGELOG `[Unreleased]`, STATUS. The architect updates the threat model and the design set.

## Non-change scope

No push, no `sync accept`, no `propose` route, no intent journal, no recovery behaviour, no SSH, no client helper, no
repository creation from an archive, no change to existing routes or the read path's responses, no new external dependency except the
one T0(b) names after its review, no `unsafe`, no version or release change.

## Required documentation updates

As T7. The enumeration table (IS-14) gains the two routes; the `args::USAGE` drift test covers every new variable.

## Acceptance criteria

T0's matrix and signal tests pass; advertise and fetch work for a public and a private repository with the right
authorization and neutral 404s; a repeated fetch of the same state is `304`; after a (simulated) write the next fetch exports
again and serves a different `ETag`; a corrupted spool file or a failing `archive verify` is never served; single flight is
proved (N concurrent requests, one export); a slow client holds no lock; reads work during an export; ranges work within one
artifact; limits and refusals are named; ENF-2 passes (no signing code); every gate green including both supply-chain gates
(fresh advisory database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No streaming of an export to a client; no serving an artifact that has not passed `archive verify`; no cache key without the
fingerprint cross-check; no absolute path in any response; no fetch that bypasses the route registry; no hard-coded
user-visible text outside the catalog; no hidden/absent difference in any response.

## Known risks

The fingerprint's cross-check by mtimes can be fooled by a same-size same-second out-of-band write; the counter under the W
lease is the primary signal and the document says so. An export of a very large store holds a process slot for seconds
(0.9 s/GiB measured); the export capacity bound protects the host. Rate limits keyed by IP behind a proxy rely on the
trusted-proxy rule.

## Required evidence and review-request format

Commits (hash → what, by task); the T0(b) dependency review; the route table additions; the fetch matrix (200, 304, 206,
416, refusals, 503, 429) with real prikk; the single-flight, slow-client and reads-during-export tests; the fingerprint tests;
the serve-gate test (a failing archive); instrumentation output; every gate with command and exit code and the advisory-database
HEAD; deviations and open questions.
