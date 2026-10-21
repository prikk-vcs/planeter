# RFC 008 — Publication (D-8, model B) — Measurement Handoff (v1): can a keyless forge receive newer history into a received ref?

| | |
|---|---|
| Document | An **investigation** handoff under RFC 008 (workflow Phase 1–2; RFC 010 D-8: measure before designing). It measures; it implements nothing. It answers the one question the owner's decision D-8 ("what a branch is while publication is unavailable") needs before model B can be chosen: *when a maintainer seals more history elsewhere and delivers it to a keyless forge, what does prikk do to a received ref the forge already holds, and does it give a program enough facts to apply a written selection rule?* Model B: the forge **receives** the maintainers' sealed history as a received ref and **selects it as the branch tip by a rule** (an adopted key, a fast-forward of the previous tip, verified), holding no key. |
| Status | **Done, reviewed Approved (2026-10-21)** (ledger PK-67). R8 (`extract` on a received ref) waits for prikk 0.54.0. Authorized by the owner 2026-10-21. |
| Basis | [`../../proposed/008-seal-one-click-merge.md`](../../proposed/008-seal-one-click-merge.md); RFC 004 third edition (accepted) §"D-8 in plain words"; ledger PK-32…PK-34, PK-49, PK-50, PK-59, PK-60 (what was measured on the transport spike); the transport measurement review request's rows A1, A5, A6, A11, A12 (reuse its scripts); RFC 154 as prikk describes it (the rule planeter would apply itself). |
| Audience | Dev team. Return a review-request package (`.git-exclude/review-requests/`) with raw logs. |
| Scope | Scratch directories under `.git-exclude/tmp/<name>/` (removed when finished), prikk **0.53.0** (and the 0.52.0 release asset side by side where a row's answer could differ; checksum-verified, extracted into its own empty directory). **Non-change scope:** nothing in the repository except the review request; no product code; no manifest change; no letter; no commit to `main`. |

## Ground rules

Same as the transport measurement: a new empty directory per fixture; scripts apart from data; treat every artifact as
untrusted; the forge-side repository is created with `env -i PATH=… prikk init` (no `HOME`, no key directory); the
maintainer repositories use `prikk setup` with their own isolated key directories; record `prikk --version`, the exact
command, exit status, the relevant output lines and, where memory is named, peak resident set; a negative result is a
result; if a measurement contradicts a ledger row marked CONFIRMED, stop and hand back; relative `cd` only.

## Rows

| ID | Measure | Record / learn |
|---|---|---|
| R1 | **A newer delivery of the same ref.** A keyless forge holds `remotes/heads/main` (delivered by `archive import` without `--adopt`, and separately by `bundle import`). The maintainer seals 3 more blocks and delivers them (a) as a per-ref bundle (`bundle export --ref`), (b) as a whole archive (`archive export`), each with and without `--adopt`, each with the maintainer's key adopted and not adopted on the forge. | For each of the 8 combinations: exit status, the exact lines, and the state afterwards: does `remotes/heads/main` move to the new tip (fast-forward), stay, or is a second ref created? `branch list`, `log --ref remotes/heads/main`, `verify` (exit, conditions), `tree --ref remotes/heads/main`. Time and peak RSS. |
| R2 | **Not a fast-forward.** Deliver (a) an **older** bundle/archive after the newer one (a rollback), (b) a **divergent** history (the maintainer rewrites from an earlier block, signing both), (c) a history signed by a **different adopted maintainer key**, (d) one signed by a key **not adopted**. | Is the received ref moved backwards, refused, or kept beside? What does prikk report (names, reasons) — is the rule "adopted key, fast-forward of the previous tip" decidable **by prikk's own output** or must planeter reimplement it? Any rollback protection by `update_seq` / `previous_ref_state_id`? |
| R3 | **What prikk tells a program about a received ref.** For each state above: the `branch --format json` fields for received refs (`received`, tip block, RefState id, `update_seq`, `previous_ref_state_id`, signer key id, any trust or publication flag), `log --format json` top block, `verify --format json` conditions (`publication-trust`), `merge-evidence` / `merge-plan` output for the received ref, `sync summary`. | A table: which facts a program can read **without parsing prose**, which only from text, and whether they suffice to apply the written rule (adopted signer; the new tip descends from the previous tip; verified). |
| R4 | **Can the forge advance its own `heads/main` without a key?** Try `merge`, `ref` verbs, `branch` verbs and `archive import --adopt` against an existing local `heads/main` (published at creation) with a newer received ref available. | The exact refusals (RFC 154 is the missing act: confirm the texts and that nothing moves). |
| R5 | **Landing observation.** A pending change (accepted with `sync accept`, unsealed) whose claims the maintainer seals elsewhere (`sync accept` + `sync seal --claims` in a keyed repository) and delivers back as a bundle/archive. | Do the patches leave the forge's `sync pending` once the delivery contains blocks that reach them? In which delivery forms, with which key states, and how soon (immediately after the import exits 0)? |
| R6 | **Two maintainers.** Two maintainer repositories (different keys, both adopted) each deliver a different `main`. Also two refs of different names. | How are they recorded (one `remotes/heads/main` or two, with what names), and can the forge tell which came from whom? What happens to the first when the second arrives? |
| R7 | **Cost and locks.** For R1(b) at 63 / 199 MiB and a 1,000-block history: time, peak RSS, which locks the import holds (W-class?), and whether reads (`tree`, `log`, `verify`) fail during it (as A10 did for accept). | Numbers for the write-admission cost of a delivery (RFC 011 extension). |
| R8 | **`extract` and `show` on the received ref** — **only if** a prikk build with `extract` is available (0.54.0 or `main`); otherwise write "not measured: needs 0.54.0". | `extract --ref remotes/heads/main` works and says `provisional`? (prikk wrote it should.) |

## Definition of done and what NOT to do

Done when every row has a result (or an explained inability with the blocking fact) and the review request holds a
table `ID → result → what it means for the rule "an adopted key, a fast-forward of the previous tip, verified"`, the raw
logs, the fixtures' sizes and how they were built, every surprise, and the questions for the architect. **Do not
recommend a design**: the architect revises RFC 008 and brings the model decision to the owner. Nothing here
authorizes a letter, a tag or a publish.
