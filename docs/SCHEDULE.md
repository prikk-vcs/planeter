# planeter — Themes, plans, and concerns

| | |
|---|---|
| Document | The plan beneath the roadmap, without dates: the development themes, the plans in order with what each waits on and what it unblocks, and the issues, risks and concerns with the relations between them. The roadmap (`ROADMAP.md`) holds the milestones and release cycles; `docs/STATUS.md` holds the current state and the issue register. |
| As of | 2026-09-24 (0.2.0 released; M2 held) |
| Basis | `ROADMAP.md`; prikk's schedule by theme (dependency ledger §prikk schedule); the design-set revisions of 2026-09-24; the owner's rulings (no calendar dates — work ships when correct, tested, secure and honest). |

## 1. Themes

| Theme | Serves | What lands | Waits on | Governing record |
|---|---|---|---|---|
| **T1 prikk re-baselines** | every milestone | per prikk release: schema check, pin/floor, ledger, records; 0.47.0 (PK-30/31), 0.48.0 (format 8, streaming), 0.49.0 | each prikk release | `docs/UPSTREAM.md`; `rfcs/handoffs/interim/prikk-0-47-0-rebaseline-handoff-v1.md` |
| **T2 read-side increments** | M1 surface, usability | `--version` (IS-1); OIDC account linking from the binary (IS-2); `spawn_blocking` (IS-4); "my repositories" home (IS-7); ENF-4 enumeration test (IS-14); `prikk_format_version` populated (IS-13) | nothing | issue register; one interim handoff each |
| **T3 transport — clone + push, keyless** | M2 | RFC 155 artifact fetch/clone; keyless push (`sync accept`); client helper; HTTP behind the trusted proxy; SSH via host OpenSSH `ForceCommand` → `planeter ssh-shell`; per-repository write lease; accept-edge bounds | prikk RFC 155 then RFC 154 shipped | RFC 004 → handoff v2 |
| **T4 change review + issues + merge** | M3 | an open change = prikk's accepted-but-unsealed claims + review metadata; inline discussion, approvals, required checks; issues with cross-references; merge = maintainer seals client-side, forge stores and re-verifies (WR-5b) | T3 for the write side; the review *design* (RFC 005 v2) waits on nothing | RFC 005 v1 → v2 |
| **T5 canonical branch + one-click merge** | B1 | the forge adopts a trusted-maintainer-signed fast-forward (prikk RFC 154); "first fast-forward wins"; the client helper's one-click merge | prikk RFC 154 + T4 | RFC 008 (re-based on adoption) |
| **T6 admin surfaces** | CAP-1/CAP-3 remainder | orgs, teams, per-repository and per-ref permissions, repository settings, rename/transfer/archive/delete, imports via brygge | T3 (writes exist) | RFC 002/003 follow-on handoffs; WEB-04 |
| **T7 auth completion** | STD-2, RR-10 | TOTP/WebAuthn second factor; audit log shipped off-box (AUTH-05); SAML/LDAP as opt-ins only on demand | nothing for MFA; audit log with T3 | RFC 002 follow-on |
| **T8 CI + runners** | M4 | pipelines on push/change/schedule; separate-host ephemeral runners; job tokens; secrets withheld from untrusted changes; `planeter-runner` ships | T3 (a repository to trigger from); T4 for change-triggered runs | RFC 006 v2 |
| **T9 packages** | M5 | OCI Distribution first; scoped publish, immutable versions, object storage; language registries by demand | T3 | RFC 007 v2 |
| **T10 hosted-format durability** | B2 | carry-forward across prikk formats via RFC 155 `import --adopt`; the format-pin/refuse rule as code (needs IS-13) | prikk RFC 155; prikk 0.48.0's format 8 as the first real case | RFC 009 |
| **T11 hardening and operations** | OPS-2/3/5, 1.0 readiness | backup as one coherent set with a rehearsed restore; observability (metrics, structured logs, health); multi-replica story (throttles at the proxy, PostgreSQL path) | T3–T10 substantially done | new RFC (operations) before 1.0 |
| **T12 API write side + webhooks** | CAP-9 | the write endpoints for T4/T6, `Link` pagination and enforced rate-limit headers, HMAC-signed webhooks through the egress guard | T3/T4 | RFC 003 follow-on; STD-4 |
| **Deferred** | post-1.0 | federation, portable identity, built-in AI (forge-commons verdicts) | the owner's call after 1.0 | ROADMAP §Later |

## 2. Plans, in order

Each plan names what it waits on and what it unblocks. Order within a tier is the architect's; order
across tiers is fixed by the dependencies.

1. **Now, during the M2 hold** — T1 (0.47.0 first, as the new team's first assignment), then T2 items
   one handoff each (IS-1, IS-14, IS-4, IS-13, IS-2, IS-7 — smallest first), and the *design* of T4
   (RFC 005 v2 against measured 0.46.0 behaviour) so M3 does not start from a pre-measurement RFC.
   Unblocks: a measured two-agent cadence; a re-issuable RFC 005; the hosting rule for format 8.
2. **When prikk 0.48.0 ships** — T1 again; T10's first real case (format 8: what the serving rule and
   the record mean for repositories already hosted in format 7); planeter's input on the streaming-bound
   design that prikk invited. Unblocks: the durability rule before RFC 155 needs it.
3. **When prikk RFC 155 then RFC 154 ship** — RFC 004 handoff v2, then T3. **This is the gate every
   later plan waits behind.** Unblocks: T4's write side, T6, T8, T9, T12.
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
| **C-1** | prikk RFC 155 and 154 are accepted but unimplemented and unscheduled beyond "after 0.49.0"; everything from T3 onward waits behind them | risk (schedule, external) | T3–T6, T8, T9, T10, T12 | C-2, C-4, C-6 | design ahead (RFC 005 v2, RFC 004 v2 draft), keep the read side shipping, deliver prikk the input it asked for; never invent a prikk wire protocol to shortcut it |
| **C-2** | Keyless fetch is whole-artifact until RFC 155 gains a delta form; large repositories clone expensively | concern (performance, external) | T3 | C-1, C-8 | planeter's planning numbers are in the draft letter to prikk; serve whole artifacts first, bounded and streamed (RFC 158 B/C) |
| **C-3** | prikk format 8 (0.48.0) arrives before RFC 155's `import --adopt` exists — hosted format-7 repositories and a format-8 serving binary | risk (durability) | T1, T10 | C-9, IS-13 | the serving-binary ≥ hosted-format rule; refuse to host an unsupported format; populate the record (IS-13) so the rule can be enforced in code; RFC 009 |
| **C-4** | JSON schema drift in a prikk release (PK-18) changes a view-model | risk (correctness) | T1, every read surface | C-1 | the driver refuses unknown `schema_version`; the re-baseline handoff stops at T1 on drift and the architect decides |
| **C-5** | The two-agent workflow's cadence is unmeasured; the hand-over could lose context | risk (delivery) | all | C-1 (the hold gives room) | small first assignment; the hand-over records (STATUS, handoffs README, UPSTREAM, dependency policy, charter); v1 handoffs quarantined until re-issued |
| **C-6** | Handoffs 004–009 predate every measurement (IS-10) | issue (records) | T3–T5, T8–T10 | C-1, C-5 | re-issue as v2 before assignment; RFC 005 v2 first (it waits on nothing) |
| **C-7** | No second factor on local accounts (RR-10); an OIDC provider that enforces MFA is the only interim answer | concern (security) | T7 | — | schedule T7's MFA with the next auth-touching release; document the interim in SECURITY.md (done) |
| **C-8** | Multi-replica deployments: in-memory throttles (RR-11), SQLite, no observability | concern (operations) | T11 | C-2 | rate-limit `/login` at the proxy (documented); store traits keep a PostgreSQL path; operations RFC before 1.0 |
| **C-9** | `prikk_format_version` never populated (IS-13); the durability rule is a deployment rule only | issue (code) | T2, T10 | C-3 | populate at create/open once prikk reports the format machine-readably (check at 0.47.0 re-baseline; else a prikk ask) |
| **C-10** | ENF-4's handler-enumeration test is missing (IS-14): "every surface consults `authorize()`" is reviewed by hand | issue (assurance) | T2, every new surface | T3, T6, T12 add handlers | write the test before T3 adds the first write handlers |
| **C-11** | A dependency advisory with no upstream fix (as `rsa` today) turns the audit gate red | risk (supply chain) | any theme adding a crate | T3 (TLS/SSH kept out of the tree), T9 (registry stacks) | scoped, justified ignores only; the subprocess pattern for protocol stacks; the rejected-crate ledger |
| **C-12** | The open owner questions OQ-4 (feature ceiling) and OQ-5 (Git mirror-out) shape T4/T6 and T9 | concern (scope) | T4, T6, T9 | D-2, D-3 | raise as decision requests before the affected handoffs, not during |
| **C-13** | Blame/annotate is unscheduled by prikk (gap catalogue A3) | concern (feature) | T2, T4 | — | the view stays "pending a prikk increment"; not faked |
| **C-14** | A single human authority; rulings, tags and letters wait on the owner's availability | concern (governance) | releases, upstream | C-1 | batch decision requests; nothing technical blocks on a ruling; the read side never waits |

**Relations, as a map** (an arrow reads "unblocks" or "is the first real case for"):

```
prikk 0.47.0 ─► T1 ─► (C-4 checked) ─► T2 items ─► measured cadence (C-5) ─► RFC 005 v2 (C-6)
prikk 0.48.0 ─► T1 ─► format 8 (C-3) ─► T10 rule ◄─ IS-13 (C-9)
prikk 0.49.0 ─► T1
prikk RFC 155 ─► keyless fetch (C-2) ─┐
                                      ├─► RFC 004 v2 ─► T3 (M2) ─► T4 (M3) ─► T5 (B1)
prikk RFC 154 ─► adoption ────────────┘                  │        └─► T12 write side + webhooks
                                                          ├─► T6 admin
                                                          ├─► T8 CI (M4) ─┐
                                                          └─► T9 packages (M5) ─┴─► T10 complete ─► T11 ─► 1.0 (owner gate)
C-10 (ENF-4 test) must land before T3 adds write handlers.   C-7 (MFA) rides any auth-touching release.
C-11 (supply chain) is checked at every crate.               C-14 (owner availability) bounds releases, not work.
```

## 4. Decision points (ordered by what triggers them, not by date)

- **D-1** the version of the 0.47.0 re-baseline (patch, or a minor if the floor moves) — at its review request.
- **D-2** OQ-4, the v1 feature ceiling (wikis, discussions, boards, insights) — before RFC 005 v2 is handed off.
- **D-3** OQ-5, Git mirror-out scope — before T9's first registry handoff or the first import request.
- **D-4** T8 before T9 or the reverse — after M2; recommended T8.
- **D-5** whether T7's MFA lands during the hold or with M3 — recommended during the hold if a release touching auth opens.
- **D-6** the 1.0 promotion — the owner's alone, after T10/T11 and the readiness report.
