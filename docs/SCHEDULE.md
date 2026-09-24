# planeter — Execution schedule and prospects

| | |
|---|---|
| Document | The architect's execution schedule beneath the roadmap: the sequence, its dependencies and critical path, expected windows, an indicative calendar **scenario** (assumptions, not commitments), the prospects for 1.0, the risk register and the decision points ahead. The owner establishes the overall schedule; this is the proposal to confirm or correct. |
| As of | 2026-09-24 (0.2.0 released) |
| Basis | `ROADMAP.md` (milestones, holds, release cycles — deliberately undated); prikk's schedule by theme (letter 2026-09-23; dependency ledger §prikk schedule); the observed pace of the first nine days; `docs/STATUS.md`. |

## 1. Observed pace (the only calibration available)

| Span | Work | Elapsed |
|---|---|---|
| 2026-09-15 | Requirements, external design, threat model, internal design, nine RFCs with handoffs | 1 day |
| 2026-09-16 → 09-23 | A0 foundations (driver, store, core + `authorize`, auth seams, layering gate) → RFC 002 + 003 → auth crypto, SQLite, release workflow → **0.1.0** | 8 days |
| 2026-09-23 | Browse UI, sessions + CSRF, egress guard, login throttle → **0.1.1** | same day |
| 2026-09-23 → 09-24 | Trusted proxies, per-IP throttle, OIDC (two supply-chain forks ruled) → **0.2.0** + documentation audit | 2 days |

This pace was one agent doing design, implementation and review with the owner ruling in near real
time. **The two-agent workflow adds a handoff, a review request and a review report per task; its pace
is unknown until the first assignment (the 0.47.0 re-baseline) has run.** The windows below assume the
implementation pace roughly halves and use that as the planning figure until measured.

## 2. Sequence, dependencies and the critical path

```
now ── M2 hold ─────────────────────────────────────────────────────────────────────────►
      │
      ├─ prikk 0.47.0 ─► re-baseline (handoff written) ── first assignment of the new team
      ├─ prikk 0.48.0 ─► re-baseline; format 8 consequences; input on the streaming-bound design
      ├─ prikk 0.49.0 ─► re-baseline
      ├─ (meanwhile) small read-side increments from the issue register, one handoff each
      │
      └─ prikk RFC 155 shipped ─► prikk RFC 154 shipped ─► transport handoff v2 ─► M2 (clone + push)
                                                                 │
                                                                 ├─► M3 review + issues + merge (RFC 005 v2)
                                                                 │      └─► B1 canonical branch + one-click merge (RFC 008)
                                                                 ├─► M4 CI + runners (RFC 006 v2)
                                                                 ├─► M5 packages (RFC 007 v2)
                                                                 └─► B2 hosted-format durability (RFC 009) ─► 1.0 candidate ─► owner gate
```

**The critical path is prikk's**: RFC 155 (the repository-complete artifact, keyless fetch) then RFC 154
(trusted fast-forward adoption, the canonical branch) — both accepted, neither implemented, both after
prikk 0.49.0. Nothing planeter does shortens it; what planeter *can* do during the hold is keep the read
side shipping, re-baseline promptly, give prikk the design input it asked for, and have the v2 transport
handoff ready the week the binary lands.

**What does not wait on prikk**: RFC 005's review model can be designed (v2) against measured 0.46.0
behaviour before M2 ships, because an open change *is* prikk's accepted-but-unsealed claim set and
`sync accept` is keyless and measured (PK-28). RFC 006 (CI) and RFC 007 (packages) have no prikk
dependency at all beyond a hosted repository to trigger from; their order after M2 is a roadmap choice,
not a technical one.

## 3. Expected windows (relative to prikk, effort-based)

| Item | Precondition | Expected effort under the new team | Release |
|---|---|---|---|
| 0.47.0 re-baseline | prikk 0.47.0 installed | ≤ 1 week (five small tasks) | patch or minor if the floor moves (owner) |
| Read-side increments (IS-1, IS-2, IS-4, IS-7) | none | 1–2 days each | folded into the next minor |
| 0.48.0 / 0.49.0 re-baselines | each prikk release | ≤ 1 week each; 0.48.0 longer if format 8 needs a hosting rule | as above |
| RFC 005 v2 (design only) | 0.46.0 measurements (done) | 1 week of architect time | — |
| Transport handoff v2 + M2 | RFC 155 and 154 shipped | 2–3 weeks | **next minor after 0.2.0** |
| M3 review + issues + merge | M2 | 3–4 weeks | minor |
| B1 canonical branch + one-click merge | RFC 154 + M3 | 1–2 weeks (mostly adoption wiring) | within M3 or the minor after |
| M4 CI + runners | M2 (a repository to trigger from) | 3–4 weeks; `planeter-runner` ships | minor |
| M5 packages (OCI first) | M2 | 3 weeks for OCI; language registries by demand | minor |
| B2 durability policy; hardening; backup rehearsal; 1.0 readiness | RFC 155 `import --adopt`; all of the above | 2–3 weeks | **1.0 candidate → owner gate** |

## 4. Indicative calendar scenario (assumptions to confirm — not commitments)

Assumptions: prikk keeps its recent cadence (0.43.0 → 0.46.0 in six days, but RFC 158's four stages
are larger); RFC 155/154 need a design round each; the new team's pace is as in §1.

| When (scenario) | Milestone |
|---|---|
| Oct 2026 | prikk 0.47.0–0.49.0 land; three re-baselines; read-side increments; RFC 005 v2 designed; the new team's cadence measured |
| Nov–Dec 2026 | prikk RFC 155 then 154 ship; transport handoff v2; **M2 clone + push** (keyless) — the first minor under the new team that is a milestone |
| Q1 2027 | **M3** review + issues + merge with B1's adoption-based one-click merge |
| Q1–Q2 2027 | **M4** CI on isolated runners |
| Q2 2027 | **M5** packages (OCI) |
| H2 2027 | B2, hardening, backup rehearsal, **1.0 candidate**; promotion is the owner's call |

The single largest uncertainty is the RFC 155/154 date; every later row shifts with it one for one.
If prikk's design rounds take a quarter longer, 1.0 candidacy moves to 2028 and the read side keeps
shipping meanwhile — planeter remains useful at each step by design.

## 5. Prospects

**What 1.0 is**: a keyless, verifiable, familiar forge — the one property no Git forge can offer (a
compromised forge cannot forge history) inside the loop every developer already knows. The read side
already demonstrates the posture (honest verify status, sandboxed prikk, strict rendering, no signing
key); M2 and B1 make it a forge people push to and merge on.

**Upside**: prikk's RFC 154/155 land early and M2 follows within weeks; the review model (RFC 005) is
ready the same month, so M3 comes fast; planeter becomes the reference deployment prikk's own
documentation points at. **Downside**: prikk's schedule slips or RFC 155's delta form takes a second
round, and planeter spends a quarter as a read-only host — still useful, but not yet the product (PU-1).
The roadmap's hedge is that every 0.x release is usable on its own and nothing waits on the frontier.

## 6. Risk register

| ID | Risk | Likelihood | Impact | Mitigation | Owner of the decision |
|---|---|---|---|---|---|
| RK-1 | prikk RFC 155/154 slip past the scenario | medium | M2 and everything after shift | read side keeps shipping; RFC 005 v2 designed early; planeter's design input to prikk delivered promptly | owner (schedule) |
| RK-2 | prikk format 8 (0.48.0) needs a hosting/migration rule before RFC 155's `import --adopt` exists | medium | hosted repositories pinned to a serving binary | the "serving binary ≥ hosted format" rule; refuse to host an unsupported format (OP-04); RFC 009 | architect → owner |
| RK-3 | JSON schema drift in a prikk release (PK-18) | low per release, certain over time | view-models change | driver refuses unknown `schema_version`; re-baseline handoff stops at T1 on drift | architect |
| RK-4 | Keyless incremental fetch stays whole-artifact (no delta form) longer than expected | medium | clone cost on large repositories | RFC 155's design round has planeter's numbers (draft letter); serve whole artifacts first | prikk team |
| RK-5 | Team migration slows delivery or loses context | medium | pace, quality | hand-over records (STATUS, handoffs README, UPSTREAM, dependency policy); first assignment deliberately small | owner |
| RK-6 | A dependency advisory with no fix (as `rsa` today) | low | audit gate red | scoped ignores only with a justification; subprocess pattern keeps protocol stacks out of the tree | architect → owner |
| RK-7 | Single owner as the only human authority (availability) | low | releases and rulings wait | decision requests batched; nothing else blocks on a ruling | owner |
| RK-8 | Multi-replica deployments outgrow in-memory throttles and SQLite (OPS-1) | low before M4 | operations | throttle at the proxy (documented); store traits keep a PostgreSQL path | architect |

## 7. Decision points ahead (for the owner, when they come due)

- **The version of the 0.47.0 re-baseline** (patch vs. minor if the floor moves) — at its review request.
- **RFC 005 v2 timing** (design during the hold vs. after M2) — recommended: during the hold.
- **OQ-4 the v1 feature ceiling** (wikis, discussions, boards, insights) — before M3 handoffs.
- **OQ-5 Git mirror-out scope** — before M5 or the first import request, whichever is first.
- **M4 before M5 or the reverse** — after M2; recommended M4 (CI gates the review loop of M3).
- **The 1.0 promotion** — the owner's alone, after B2 and the readiness report.
