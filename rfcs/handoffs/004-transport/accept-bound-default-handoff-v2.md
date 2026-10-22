# RFC 004 — Transport — Implementation Handoff (v2, part 4c): the default size bound of an accept becomes 100 MiB

| | |
|---|---|
| Document | One decision of the part 4b review (D-38), small by design: with the measured cost model (10 MiB + 7.0 × the artifact + 2 KiB × the object bound) the default 256 MiB bound needs 1.8 GiB of the memory budget, which only a host with about 8.1 GiB of usable memory admits at the default budget; a 4 GiB host (a common small server) would refuse every large proposal and warn at start-up. The default `PRIKK_EXCHANGE_MAX_BYTES` becomes **100 MiB** (a 4 GiB host admits up to 125 MiB). RFC 004 D-5.3, D-5.4, R-3 and Q-3 are amended in the same step by the architect (done 2026-10-22). |
| Status | **Assigned** (2026-10-22), after part 4b was reviewed Approved. |
| Basis | [`../../accepted/004-transport.md`](../../accepted/004-transport.md) D-5.3 (amended); `accept-cost-model-review-request.md` §5 and §6 question 1; `crates/planeter-transport/src/propose.rs` (`cost_model`, `ProposeConfig`), `crates/planeter/src/args.rs` (`USAGE`), `docs/propose.md`, the OpenAPI text of the bounds. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## Change scope

- **T1 — The default.** `PRIKK_EXCHANGE_MAX_BYTES` defaults to `104857600` (100 MiB) wherever the default lives (the config, `args::USAGE`, `docs/propose.md`'s settings table and its cost paragraph, the OpenAPI text, the start-up line, `docs/RELEASING.md` if it names it, the CHANGELOG). The edge refusal (`413`) and prikk's own bound use the same value. The hint of `error.payload_too_large` still tells the operator-facing truth ("ask the person who runs this server to raise the limit"); add to `docs/propose.md` one honest sentence for people: **prikk keeps a full copy of a file for every edit it records, so a 4 MiB file edited fifty times makes a 200 MiB artifact** — the usual reason a proposal exceeds the bound — and the remedy (propose in smaller steps, or ask the operator).
- **T2 — Tests follow the default.** The end-to-end test of the largest allowed blob uses the **default** bound (a blob within a few KiB of 100 MiB) and still passes under the default ceiling; keep one test with the bound raised to 256 MiB **by configuration** (it proves the operator can, and that the start-up check agrees), provided the two together stay under 30 s; the admission test (`accept_admission.rs`) is updated to the new default (a 4 GiB host admits the default bound; a 2 GiB host warns and says what to change; 8 GiB and more admit a raised 256 MiB). The unit test that lists measured shapes is unchanged (it is about the model, not the default).
- **T3 — The budget message.** The start-up line names the largest artifact the *configured* budget admits and the configured bound, in one sentence each, so an operator sees at once whether the two agree (it exists; check it reads well with the new default and say what you changed).

## Non-change scope

No change to the cost model's constants, the object bound (5,000), the deadline (600 s), the framing, the check order, quotas, recovery, the lease classes or the HTTP surface; no new dependency; no `unsafe`; no version or release change.

## Acceptance criteria

The default is 100 MiB in every place that states it (a grep of the repository for the old number shows only history, the RFC's amendment note and the measurement report); T2's tests pass; both supply-chain gates and every earlier gate are green on a fresh advisory database, both test runs named, `cargo doc`; the OpenAPI drift guard and the `USAGE` drift test pass.

## Prohibited shortcuts

No change of any constant of the model; no silent change of another default; no test deleted to make the new default pass.

## Required evidence and review-request format

Commits (hash → what, by task); the grep of the old number with each remaining hit explained; the two test transcripts with times; the start-up line for a 4 GiB and a 2 GiB budget; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
