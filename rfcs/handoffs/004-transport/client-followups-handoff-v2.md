# RFC 004 — Transport — Implementation Handoff (v2, part 10): what the client helper's review found

| | |
|---|---|
| Document | Small follow-ups of parts 8 and 9 (the have-list route and `planeter-client`), all decided in the review of 2026-10-30 (D-51): the advertisement should say what a client needs to know before it sends (the artifact size bound) and what makes a freshness check a *comparison* (the tip's `update_seq`); the client's tests should run its three commands on macOS and Windows if prikk can run there; and one undocumented variable should be documented. |
| Status | **Assigned** (2026-10-30). Independent of the image handoff. |
| Basis | `client-helper-review-request.md` §9 (items 1, 2, 7, 8), `have-route-review-request.md` §7; RFC 004 D-8 (pin `update_seq` and RefState id), D-11; `docs/client.md`, `docs/propose.md`; `crates/planeter-transport/src/export.rs` (`Advertisement`, `RefInfo`), the OpenAPI document and drift guard; `.github/workflows/ci.yml` (job `client`). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The advertisement carries the bound.** Add `limits: { max_artifact_bytes, max_objects }` (the server's configured propose bounds, from `ProposeConfig`) to the advertisement when proposals are on (absent when the artifact store is off); OpenAPI + drift guard; `docs/propose.md`. The client uses it as it was written to (refuse locally with the size and the remedy) — remove "if the advertisement ever carries it" caution from `docs/client.md`; add the client test that a too-large work is refused before any upload, with the server's number in the sentence.
- **T2 — `update_seq` in the advertisement.** Per published branch, add `update_seq` (the tip's RefState sequence; `prikk log` already reports `update_seq` for each block, and the advertisement reads the tip's log for `tip_block`, so no extra prikk call) and keep `ref_state_id`. The client's freshness check becomes the one RFC 004 D-8 describes: **pin the highest `update_seq` and RefState id seen per ref and refuse a served tip with a lower `update_seq` or one that does not continue the recorded tip** — a comparison **before** the download when the advertisement shows a lower number (no trial import); keep the continuation check (and its trial import) only when the number is equal or higher. Tests: lower number refused before downloading (nothing fetched: the server stub saw no fetch request), equal/higher with continuation accepted, higher without continuation refused, advertisement without `update_seq` (an older server) falls back to today's behaviour and says so; the `fetch.no_sequence` message becomes shown only in that fallback.
- **T3 — Run the three commands on macOS and Windows if prikk can.** Check whether the prikk project publishes release assets for macOS and Windows (look at its release page and the ledger; do not guess). If it does: the `client` CI job installs the pinned prikk asset with its checksum (as the `Dockerfile` does for Linux, pinned by version and `.sha256`), and runs `flows.rs` (and `proxy.rs` where it is Unix-only: say which stay Linux-only and why) on those systems. If it does not: say so in the report and in `docs/client.md`'s "what CI covers" section — the 3-OS claim stays limited to the HTTP/TLS stack, the command line and the messages. Either way report which tests ran on which system.
- **T4 — Document `PLANETER_CLIENT_PRIKK`.** The variable (the path of the prikk program the client runs) is in `docs/client.md` with one sentence on why it exists (tests, unusual layouts) and one on what it is not (not a way to choose a different VCS; whatever it names runs with the user's own rights, like the `prikk` on `PATH` would) — a variable that changes which program runs must not be a secret. It is not in `--help`.
- **T5 — Messages.** Any new text in both languages (the sentence for the over-bound refusal exists; the new fallback sentence) with Japanese draft rows in the next addendum.

## Non-change scope

No new command; no change to the propose framing, quotas or recovery; no distribution work; no new dependency; no `unsafe`; no version or release change.

## Acceptance criteria

T1–T5 as written with the tests named; the server's IS-14 table, the OpenAPI drift guard, ENF-2 (both binaries) and all earlier checks pass; every gate green (fmt, floor, pin, clippy, both test runs named, deny and audit both 0, ENF-2, doc), CI green on all three systems.

## Prohibited shortcuts

No download before the freshness comparison when the advertisement already shows a lower number; no CI claim larger than what ran; no unpinned prikk asset.

## Required evidence and review-request format

Commits (hash → what); the advertisement before/after (JSON); the freshness test transcripts; the prikk-assets finding and the per-system test table; every gate with command and exit code; deviations and open questions.
