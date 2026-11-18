# prikk's JSON: what planeter relies on (the answer to prikk's letter of 2026-11-18) — Implementation Handoff (v1)

| | |
|---|---|
| Document | prikk 0.59.0 opens a design round on what prikk promises about its `--format json` outputs and asks consumers eight questions (`.git-exclude/upstream/prikk/receive/2026-11-18-what-you-rely-on-in-our-json.md`: read it first). The answer must be **exact and checkable**: it becomes a letter from the owner to prikk and then a promise prikk may build on. This handoff is an **inventory from the code and the tests**, read-only; it changes no file in the repository. The architect writes the letter from your report. |
| Status | Assigned 2026-11-19 (SCHEDULE D-98). |
| Basis | the letter; `crates/planeter-prikk/src/` (every reader of prikk output: `model.rs`, `cli.rs`, `exchange.rs`, `extract.rs`, `delivery.rs`, `maintenance.rs`, the `Failure`/error mapping, the readers in `planeter-transport/src/delivery/`), `crates/planeter-client/src/prikk.rs`; the tests that pin shapes; ledger PK-1, PK-18 (schema drift), PK-77–PK-85. |
| Audience | Dev team. Return a short review-request package (the report). |

## Questions (answer each with file:line evidence; "none" is an answer)
1. **Which outputs do we read, and which fields of each?** One table: prikk command and flags (every `--format json` invocation the driver makes **and** every one the client makes) → schema name → the field paths read (a field "read" = deserialized into a struct field that a decision uses, as opposed to ignored) → where (file:line). Include the text outputs we read when no JSON exists (`sync pending` text form is not used any more? say), and the prikk verbs we call **without** `--format json` and parse text (`sync accept` report, `doctor`, `unlock`: list the parsers and what each depends on).
2. **Unknown fields.** For every deserializer of prikk output: does it ignore a field it does not know (serde default) or refuse it (`deny_unknown_fields`)? Grep and list any `deny_unknown_fields` on prikk output types (the ones in `planeter-web` are for our own request bodies: say so); add a test-backed statement: a document with an extra field at the top and in every nested object still parses (write the test if it is missing; it is the one file this handoff may add, a test, and only if it is missing — list it).
3. **Open values.** Every enumeration of a prikk value (`refusal-v1` `refusal` ids, `reason`, `point_kind`, signature `outcome`, `status` strings of `archive verify`, `object_type`, doctor codes, ref states, claim signature outcomes, `verify` condition ids): for each, what a value we have not seen does (`Other(String)` and log and count; a default branch; an error: **which ones are errors**). The closed lists are the interesting ones (the doctor `PRIKK-DOCTOR-UNREACHED-*` family reads **unknown as unreadable**: say that and why).
4. **Absent versus `null`.** Where do we require a field to be present even when `null` (the `required_nullable` helper of the extract report), where do we treat absent as `None`/default, and where are the two **different** (list each with file:line and the reason).
5. **Large integers.** Every number read from prikk JSON: its Rust type (`u64`, `u128`, `i64`, `f64`), and the largest value we handle; whether any passes through a language or library with doubles. (The web UI and OpenAPI show some of them to JavaScript clients: list those that reach our own JSON API unchanged, since a double-based consumer of **our** API would meet them.)
6. **Exit codes.** Per prikk command we call: the exit codes we key on and what each means to us (0, 1, 2, anything else; the timeout wrapper's 124; a signal), with the mapping function and its tests. Say where "exit 1 with a JSON `refusal-v1` on stdout" is read as a refusal and where "exit 1 with text" is.
7. **Refusals.** List every prikk command whose **refusal** output we parse: from `refusal-v1` documents (which commands, which ids we branch on), from stderr text (which sentences, matched how: substring, prefix, exact), from exit code alone. Answer: **would we rely on every refusal under `--format json` printing `refusal-v1`?** (Do we need it anywhere? Where would it replace a text match? List the text matches we would like to delete: `lock conflict`, `unsupported format version`, the stale-lock words, the truncated-artifact sentence, the `unlock` liveness words, `incoming.max-object-bytes` …).
8. **Our languages.** Every program that reads prikk output: Rust in the workspace, and **anything else** (the measurement and test scripts in Python under `.git-exclude/` are not product: say so; any shell script in `tools/` or the workflows that parses prikk output: list). Which language reads which output.
9. **Beyond their eight (for the architect):** (a) the outputs we **would** like prikk to add or promise (the claim ids and the parent block id for `claim-parent-not-sealed`/`claim-chain-shared-block`; the signer of a received tip; `log` limits; a `verify` finding list with ids; the in-flight publication finding in the JSON), each with the planeter feature that needs it; (b) the text output we parse that has **no** JSON form at all, in order of how much we would like one; (c) which fields we read that prikk has said are **not** a promise (any note in the ledger).

## Format of the report
The table of Q1 is the heart: one row per (command, schema, field path); the rest are short sections with the evidence. Mark every statement **verified by a test** (name it) or **read from the code only**. Do not guess; if you cannot tell from the code, say so.

## Non-change scope
No code, doc or version change; the only file you may add is one missing test (Q2), listed; no letter to prikk (the architect writes it for the owner).

## Acceptance criteria
Q1–Q9 answered with evidence; the unknown-field test exists and passes; a clean `git status` otherwise.

## Required evidence and review-request format
The report; the list of any test added; deviations and open questions.
