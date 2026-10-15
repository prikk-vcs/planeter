# planeter — Themes, plans, and concerns

| | |
|---|---|
| Document | The plan beneath the roadmap, without dates: the development themes, the plans in order with what each waits on and what it unblocks, and the issues, risks and concerns with the relations between them. The roadmap (`ROADMAP.md`) holds the milestones and release cycles; `docs/STATUS.md` holds the current state and the issue register. |
| As of | 2026-10-15 (0.2.0 released; M2 held; prikk at 0.52.0; upstream facts per ledger v1.5) |
| Basis | `ROADMAP.md`; prikk's schedule by theme (dependency ledger §prikk schedule); the design-set revisions of 2026-09-24; the owner's rulings (no calendar dates — work ships when correct, tested, secure and honest). |

## 1. Themes

| Theme | Serves | What lands | Waits on | Governing record |
|---|---|---|---|---|
| **T1 prikk re-baselines** | every milestone | per prikk release: schema check, pin/floor, ledger, records; one re-baseline against 0.52.0 (PK-30/31 fixed; PK-42 items; `trust_check` exit code), then each later release | each prikk release | `docs/UPSTREAM.md`; `rfcs/handoffs/interim/prikk-0-52-0-rebaseline-handoff-v2.md` (v1, for 0.47.0, is overtaken) |
| **T2 read-side increments** | M1 surface, usability | `--version` (IS-1); OIDC account linking from the binary (IS-2); `spawn_blocking` (IS-4); "my repositories" home (IS-7); ENF-4 enumeration test (IS-14); `prikk_format_version` populated (IS-13) | nothing | issue register; one interim handoff each |
| **T3 transport — clone + push, keyless** | M2 | RFC 155 artifact fetch/clone; keyless push (`sync accept`); client helper; HTTP behind the trusted proxy; SSH via host OpenSSH `ForceCommand` → `planeter ssh-shell`; per-repository write lease; accept-edge bounds | RFC 155 has shipped (whole-repository); the forge-side advance and incremental fetch wait on RFC 154 / a delta form (unscheduled / not planned) and the owner's ruling on M2's shape | RFC 004 → handoff v2 |
| **T4 change review + issues + merge** | M3 | an open change = prikk's accepted-but-unsealed claims + review metadata; inline discussion, approvals, required checks; issues with cross-references; merge = maintainer seals client-side, forge stores and re-verifies (WR-5b) | T3 for the write side; the review *design* (RFC 005 v2) waits on nothing | RFC 005 v1 → v2 |
| **T5 canonical branch + one-click merge** | B1 | the forge adopts a trusted-maintainer-signed fast-forward (prikk RFC 154); "first fast-forward wins"; the client helper's one-click merge | prikk RFC 154 + T4 | RFC 008 (re-based on adoption) |
| **T6 admin surfaces** | CAP-1/CAP-3 remainder | orgs, teams, per-repository and per-ref permissions, repository settings, rename/transfer/archive/delete, imports via brygge | T3 (writes exist) | RFC 002/003 follow-on handoffs; WEB-04 |
| **T7 auth completion** | STD-2, RR-10 | TOTP/WebAuthn second factor; audit log shipped off-box (AUTH-05); SAML/LDAP as opt-ins only on demand | nothing for MFA; audit log with T3 | RFC 002 follow-on |
| **T8 CI + runners** | M4 | pipelines on push/change/schedule; separate-host ephemeral runners; job tokens; secrets withheld from untrusted changes; `planeter-runner` ships | T3 (a repository to trigger from); T4 for change-triggered runs | RFC 006 v2 |
| **T9 packages** | M5 | OCI Distribution first; scoped publish, immutable versions, object storage; language registries by demand | T3 | RFC 007 v2 |
| **T10 hosted-format durability** | B2 | carry-forward across prikk formats via RFC 155 `import --adopt`; the format-pin/refuse rule as code (needs IS-13) | prikk RFC 155 (shipped in 0.51.0 as `archive`; closure fix 0.52.0); a real format change does not exist yet (format 8 is unscheduled) | RFC 009 |
| **T11 hardening and operations** | OPS-2/3/5, 1.0 readiness | backup as one coherent set with a rehearsed restore; observability (metrics, structured logs, health); multi-replica story (throttles at the proxy, PostgreSQL path) | T3–T10 substantially done | new RFC (operations) before 1.0 |
| **T12 API write side + webhooks** | CAP-9 | the write endpoints for T4/T6, `Link` pagination and enforced rate-limit headers, HMAC-signed webhooks through the egress guard | T3/T4 | RFC 003 follow-on; STD-4 |
| **T13 measurement spike (transport and CI read side)** | M2's shape, M4 | measurements A1–A11 and B1–B7 on prikk 0.52.0 in a scratch directory (keyless receive, push memory and bounds, `sync have` on a keyless forge, locks under kill, export timing, snapshot mechanisms, runner isolation, workflow census); ledger rows promoted from ASSUMED; **no product code** | the owner's authorization (D-7); prikk 0.52.0 installed; T1 done | `rfcs/handoffs/interim/transport-measurement-spike-handoff-v1.md` |
| **Deferred** | post-1.0 | federation, portable identity, built-in AI (forge-commons verdicts) | the owner's call after 1.0 | ROADMAP §Later |

## 2. Plans, in order

Each plan names what it waits on and what it unblocks. Order within a tier is the architect's; order
across tiers is fixed by the dependencies.

1. **Now, during the M2 hold** — in this order (the architect's; each step names its precondition):
   1. **T1**, the 0.52.0 re-baseline (handoff v2). *Waits on:* nothing: prikk 0.52.0 was installed 2026-10-15, so it is assignable.
   2. **T13**, the measurement spike. *Waits on:* step 1 (D-7 is granted). Unblocks: D-8 and the
      evidence for RFC 004 v2 and RFC 006 v2. Measurement only.
   3. **Defects and hardening, one small handoff each** (no prikk dependency; the architect may reorder
      against step 2 if the owner prefers): IS-4 (a denial of service on public repositories: first, and it
      blocks any public-deployment guidance), IS-15 (limits seam), IS-17 (pin actions by digest), IS-16
      (raw-content host gating), IS-14 (ENF-4 test, before any write handler), then IS-1, IS-13, IS-2,
      IS-7 smallest first.
   4. **Architect, in parallel, no dev time:** RFC 005 v2 (waits on nothing but D-2), then RFC 004 v2 and
      RFC 006 v2 after step 2, and the requirements note for prikk's 0.54.0 read-only command design round
      (the owner conveys it; prikk wants requirements before that round); each moves its RFC to `proposed/` while it is revised (IS-20).
   5. **Candidate release 0.2.1** (a patch: no new features): the 0.52.0 floor, step 3's hardening, IS-1.
      The architect recommends it at step 3's end; tag and publish are the owner's.
   Unblocks: a measured two-agent cadence; re-issuable RFCs 004–006; the owner's ruling on M2 (D-8).
2. **When prikk next changes the repository format** (format 8 is unscheduled) — T1 again; T10's first
   real case (what the serving rule and the record mean for repositories already hosted in format 7);
   planeter's input on RFC 158's streaming bound if prikk reopens it. Unblocks: the durability rule.
3. **The write path.** RFC 155 has shipped (whole-repository); **RFC 154 is unscheduled and a keyless delta
   form is not planned.** So the plan is the owner's ruling on M2's shape after the measurement spike
   (theme T13, `rfcs/handoffs/interim/transport-measurement-spike-handoff-v1.md`): RFC 004 handoff v2, then T3, on whole artifacts. **Forge-side branch
   advance (merge) and incremental fetch are what wait for prikk.** Unblocks: T4's review side, T6, T8, T9, T12.
4. **After M2** — T4 (M3) with T5 riding on it (adoption is small once RFC 154 exists), T12's write
   endpoints alongside; T6 as the administration those need. T7's MFA can land in any tier, since it
   waits on nothing — the architect schedules it when a release touching auth is open anyway.
5. **After M3** — T8 (M4) before T9 (M5), recommended: CI gates the review loop; packages have no
   dependents. The owner decides the order (decision point D-4 below).
6. **Before 1.0** — T10 complete, T11 (operations RFC, backup rehearsal, observability), the readiness
   report, then the owner's 1.0 decision.

## 3. Issues, risks and concerns — and how they relate

| ID | Concern | Kind | Touches | Related | Handling |
|---|---|---|---|---|---|
| **C-1** | prikk RFC 155 has shipped (0.51.0, whole-repository); RFC 154 is accepted, unbuilt and **unscheduled**, and a keyless delta form is **not planned** (as of 2026-10-15); forge-side branch advance and incremental fetch wait behind them | risk (schedule, external) | T3–T6, T8, T9, T10, T12 | C-2, C-4, C-6 | design ahead (RFC 005 v2, RFC 004 v2 draft), keep the read side shipping, deliver prikk the input it asked for; never invent a prikk wire protocol to shortcut it |
| **C-2** | Keyless fetch is whole-artifact and prikk does **not plan** a delta form (ledger PK-36); large repositories clone expensively | concern (performance, external) | T3 | C-1, C-8 | planeter's numbers were sent to prikk (2026-10-13); CI gets source snapshots, not history (RFC 006 v2); unchanged repositories answered by conditional GET; each artifact built once per state, spooled, verified and served (RFC 004 v2); measure the residual at the first deployment: **instrument fetches from day one (changed vs unchanged, artifact kind, bytes), because prikk decides a delta archive on those numbers, not before 0.55.0 (PK-47)** |
| **C-3** | a future prikk format (format 8 is unscheduled) arrives while repositories are hosted in format 7; `archive import --adopt` (0.51.0) is the carry-forward mechanism | risk (durability) | T1, T10 | C-9, IS-13 | the serving-binary ≥ hosted-format rule; refuse to host an unsupported format; populate the record (IS-13) so the rule can be enforced in code; RFC 009 |
| **C-4** | JSON schema drift in a prikk release (PK-18) changes a view-model | risk (correctness) | T1, every read surface | C-1 | the driver refuses unknown `schema_version`; the re-baseline handoff stops at T1 on drift and the architect decides |
| **C-5** | The two-agent workflow's cadence is unmeasured; the hand-over could lose context | risk (delivery) | all | C-1 (the hold gives room) | small first assignment; the hand-over records (STATUS, handoffs README, UPSTREAM, dependency policy, charter); v1 handoffs quarantined until re-issued |
| **C-6** | Handoffs 004–009 predate every measurement (IS-10) | issue (records) | T3–T5, T8–T10 | C-1, C-5 | re-issue as v2 before assignment; RFC 005 v2 first (it waits on nothing) |
| **C-7** | No second factor on local accounts (RR-10); an OIDC provider that enforces MFA is the only interim answer | concern (security) | T7 | — | schedule T7's MFA with the next auth-touching release; document the interim in SECURITY.md (done) |
| **C-8** | Multi-replica deployments: in-memory throttles (RR-11), SQLite, no observability | concern (operations) | T11 | C-2 | rate-limit `/login` at the proxy (documented); store traits keep a PostgreSQL path; operations RFC before 1.0 |
| **C-9** | `prikk_format_version` never populated (IS-13); the durability rule is a deployment rule only | issue (code) | T2, T10 | C-3 | populate at create/open once prikk reports the format machine-readably (check at the 0.52.0 re-baseline; else a prikk ask) |
| **C-10** | ENF-4's handler-enumeration test is missing (IS-14): "every surface consults `authorize()`" is reviewed by hand | issue (assurance) | T2, every new surface | T3, T6, T12 add handlers | write the test before T3 adds the first write handlers |
| **C-11** | A dependency advisory with no upstream fix (as `rsa` today) turns the audit gate red | risk (supply chain) | any theme adding a crate | T3 (TLS/SSH kept out of the tree), T9 (registry stacks) | scoped, justified ignores only; the subprocess pattern for protocol stacks; the rejected-crate ledger |
| **C-12** | The open owner questions OQ-4 (feature ceiling) and OQ-5 (Git mirror-out) shape T4/T6 and T9 | concern (scope) | T4, T6, T9 | D-2, D-3 | raise as decision requests before the affected handoffs, not during |
| **C-13** | Blame/annotate is unscheduled by prikk (gap catalogue A3) | concern (feature) | T2, T4 | — | the view stays "pending a prikk increment"; not faked |
| **C-15** | The design basis of RFC 004 v2 and RFC 006 v2 is documents, not measurements: keyless `archive import`, `sync have` on a keyless forge, lock behaviour under our sandbox, memory multipliers and the snapshot mechanisms are ASSUMED or prikk-measured only | risk (design) | T3, T8, M2's shape | C-1, C-2, C-5 | T13 first; no v2 RFC is accepted before its rows are CONFIRMED; a measurement that contradicts a ledger row stops the spike |
| **C-14** | A single human authority; rulings, tags and letters wait on the owner's availability | concern (governance) | releases, upstream | C-1 | batch decision requests; nothing technical blocks on a ruling; the read side never waits |

**Relations, as a map** (an arrow reads "unblocks" or "is the first real case for"):

```
prikk 0.52.0 ─► T1 ─► (C-4 checked) ─► T13 spike (D-7) ─► D-8 ruling on M2 ─► RFC 004 v2 / RFC 006 v2
                       └──────────► T2 items ─► measured cadence (C-5) ─► RFC 005 v2 (C-6)
a future format change ─► T1 ─► (C-3) ─► T10 rule ◄─ IS-13 (C-9)
prikk archive (0.51.0) ─► whole-artifact fetch (C-2) ─┐
                                      ├─► RFC 004 v2 ─► T3 (M2) ─► T4 (M3) ─► T5 (B1)
prikk RFC 154 (unscheduled) ─► adoption ┘                │        └─► T12 write side + webhooks
                                                          ├─► T6 admin
                                                          ├─► T8 CI (M4) ─┐
                                                          └─► T9 packages (M5) ─┴─► T10 complete ─► T11 ─► 1.0 (owner gate)
C-10 (ENF-4 test) must land before T3 adds write handlers.   C-7 (MFA) rides any auth-touching release.
C-11 (supply chain) is checked at every crate.               C-14 (owner availability) bounds releases, not work.
```

## 4. Decision points (ordered by what triggers them, not by date)

- **D-1** the version of the 0.52.0 re-baseline (patch, or a minor if the floor moves) — at its review request.
- **D-2** OQ-4, the v1 feature ceiling (wikis, discussions, boards, insights) — before RFC 005 v2 is handed off.
- **D-3** OQ-5, Git mirror-out scope — before T9's first registry handoff or the first import request.
- **D-4** T8 before T9 or the reverse — after M2; recommended T8.
- **D-5** whether T7's MFA lands during the hold or with M3 — recommended during the hold if a release touching auth opens.
- **D-6** the 1.0 promotion — the owner's alone, after T10/T11 and the readiness report.
- **D-7** authorize the T13 spike (measurement only; no repository change) — **AUTHORIZED by the owner 2026-10-15**; it becomes assignable when prikk 0.52.0 is installed and T1 is done.
- **D-8** the ruling on M2's shape and on what a branch is while publication is unavailable (private RFC 004
  scoping note D-T1/D-T2) — at T13's review request. The architect's lean is stated there; it is the owner's call.
- **D-9** the contribution model for non-writers (the `propose` action, the proposals setting, quotas; D-T3) — with RFC 005 v2.
- **D-10** the CI decisions: how a runner gets the tree of an open change, the workflow subset, runner
  isolation, the job network default (D-C2, D-C3, D-C4, D-C5; D-C6 is accepted) — with RFC 006 v2, after T13.
- **D-11** the version and timing of candidate release 0.2.1 and its content (floor 0.52.0 plus hardening) — at step 3's end.

*Owner, 2026-10-15: D-7 granted; the plan's order and the triggers of D-8 to D-11 accepted. D-8 to D-11 are **not** decided: each is taken at its trigger, on the evidence then available. A decision made earlier than its trigger would be made on documents, which is what C-15 forbids.*
