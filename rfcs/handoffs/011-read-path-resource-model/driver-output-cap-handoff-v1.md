# RFC 011 follow-on — A cap on what the driver collects from a prikk call — Implementation Handoff (v1)

| | |
|---|---|
| Document | RFC 011 bounds a prikk call's **memory** (`prlimit --data`), **time** (deadline) and **lifetime** (dies with the server). It does not bound what planeter **collects**: `Sandbox::run` reads the child's stdout and stderr into `Vec<u8>` without limit. The memory ceiling protects the host from the child, not from our own buffer; a hostile archive with a million tiny branches makes `archive import --format json` print a report as large as the archive (≤ the 100 MiB edge bound), `log --limit 100000` on a long history is large by nature, and a damaged archive may print a long `damaged` list. Found by the dev team in part 7 (question 8); recorded as IS-55 and threat-model RR-16. This handoff closes it for **every** prikk call. |
| Status | **Assigned** (2026-10-28). Independent of the supply-chain ruling and of the other open work. |
| Basis | [`../../accepted/011-read-path-resource-model.md`](../../accepted/011-read-path-resource-model.md) D-2 (ceilings), D-5 (child lifetime); `crates/planeter-prikk/src/sandbox.rs` (`Sandbox::run`, the spawn-and-collect function, `Output`), `crates/planeter-prikk/src/cli.rs` (the typed verbs and `PrikkError`), `crates/planeter-core/src/admission.rs`; `repository-creation-review-request.md` §7 question 8; threat model v0.5 RR-16. |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`). |

## T1 — Measure what real calls print (scratch only)

With real prikk 0.54.1, report the size of stdout (and stderr) for every verb the drivers use, on the shapes that make it large: `log --format json` (default 20 blocks, and `--limit 100000` on repositories of 200, 5,000 and 20,000 blocks); `branch --all`; `tree` of 1,000 and 10,000 files; `cat`/`file` is content (already capped by `CONTENT_MAX_BYTES`: confirm and say where); `show` of a block with 1,000 patches; `sync pending` with 20,000 pending; `verify --format json` with many failures and with 2,000 unreached objects; `doctor`; `unlock`; `sync accept` text with 1,000 claims; `archive verify` and `archive import --format json` of archives with 1, 100, 2,000 and 20,000 refs/branches (as many as prikk allows) and a damaged one with many `damaged` entries; `extract --format json`. Give the table: verb, shape, bytes. State the largest legitimate output and the largest attacker-reachable one, per verb.

## T2 — The cap

(a) `Sandbox::run` (and its spawn-and-collect function) takes an **output limit**: collecting stops at the limit, the child is killed (the existing kill path), and the call returns a named error `PrikkError::OutputTooLarge { limit, stream }` (stdout and stderr each capped; stderr's cap is small, e.g. 1 MiB, with its tail kept for messages). Reading must not allocate beyond the limit plus one chunk. (b) Defaults chosen **from T1's table with a margin of at least 2× over the largest legitimate output of each verb** and expressed once, per verb class, in one table with provenance (like `cost_model` and `CostRule`); verbs whose legitimate output is bounded by construction get a tight cap; `log` with a large `--limit`, `archive import` and `verify` get the larger caps T1 justifies. A caller that knows better may pass a different limit (the observation's `log --limit 100000` is the example: its cap must admit what T1 says a 100,000-block log is, and the observation must treat `OutputTooLarge` as "unknown", never as "none"). (c) Every existing caller maps the new error to its existing "unavailable / incomplete" path with a log line and a counter (`prikk_output_too_large_total` by verb); **a creation or accept that hits the cap is an ordinary refusal or failure of that operation** (for an accept: incomplete, as any unreadable output; for a creation: the repository is removed whole). (d) Tests: a stand-in prikk that prints 1 GiB (the child is killed, memory stays flat — measure the process's resident set during the test and assert a bound), one that prints exactly the limit and one byte more, stderr flooding, a child that ignores the pipe closing; the real archive with very many refs against the real prikk under the creation cap; mutations: cap not applied to stderr, cap off by one, the error mapped to "empty output".

## Non-change scope

No change to any verb's arguments, to the cost or ceiling model, to authorization or the HTTP surface; no new dependency; no `unsafe`; no version or release change.

## Acceptance criteria

T1's table; T2's cap in one table with provenance; every existing caller handles the new error (a grep for the call sites of the sandbox's run shows each handled, listed in the report); T2(d)'s tests and mutations; all earlier checks pass; every gate green **except** the two supply-chain gates while IS-54 is open (report their exits as they are, on a fresh database), both test runs named, `cargo doc`.

## Prohibited shortcuts

No cap without the measurement behind it; no silent truncation (a truncated report must never be parsed as a whole one); no unbounded read left in any path (grep for `read_to_end`/`read_to_string` on child pipes and list each); no change that makes a legitimate output fail on the measured shapes.

## Required evidence and review-request format

Commits (hash → what, by task); T1's table and where the scripts are; the cap table with provenance; the list of call sites and how each handles `OutputTooLarge`; the memory-flat test transcript; every gate with command and exit code and the advisory-database HEAD; deviations and open questions.
