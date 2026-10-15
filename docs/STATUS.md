# planeter — project status

| | |
|---|---|
| Document | The dated brief a new session reads **first**: where the project is, what is held and why, what is pending on whom, and the issue register. Updated at every release, RFC disposition and owner ruling. |
| As of | 2026-10-15 — planeter **0.2.0** released (2026-09-24); prikk is at **0.52.0**; upstream facts folded into ledger v1.5 |
| Read next | [`src/planeter-00-project-charter.md`](src/planeter-00-project-charter.md) (background, goals, governance) → [`../ROADMAP.md`](../ROADMAP.md) → [`SCHEDULE.md`](SCHEDULE.md) (themes, plans, concerns) → [`../rfcs/README.md`](../rfcs/README.md) → [`../rfcs/handoffs/README.md`](../rfcs/handoffs/README.md) and the active handoff → [`src/planeter-prikk-dependency-ledger.md`](src/planeter-prikk-dependency-ledger.md) → [`src/planeter-03-threat-model-v0.1.md`](src/planeter-03-threat-model-v0.1.md) (v0.3) → [`UPSTREAM.md`](UPSTREAM.md) → [`dependency-policy.md`](dependency-policy.md) |

## Where we are

- **Shipped: 0.2.0 (2026-09-24)** — host + browse + auth: bubblewrap-sandboxed prikk hosting, browse UI
  and read API, local passwords (Argon2id), OpenID Connect SSO (accounts linked administratively),
  scoped tokens, ed25519 SSH keys, sessions + CSRF, per-account and per-IP login throttles, trusted
  reverse proxies, SQLite. Attested Linux tarballs, `ghcr.io/prikk-vcs/planeter:0.2.0`, nine crates on
  crates.io. Full notes: [`../CHANGELOG.md`](../CHANGELOG.md); procedure and log: [`RELEASING.md`](RELEASING.md).
- **Milestones:** M0 and M1 done; 0.1.1 and 0.2.0 were read-side increments during the M2 hold.
  **M2 (keyless clone + push) is HELD** — measured on prikk 0.46.0, a keyless forge cannot build
  incremental-fetch artifacts (`sync build` needs a maintainer key, PK-27) and a planeter-created
  repository has no servable branch until prikk RFC 154 adoption. It resumes when prikk ships
  **RFC 155 then RFC 154, after prikk 0.49.0**, with a v2 transport handoff.
  **2026-10-15 — the hold's premise was overtaken (ledger v1.5, section D):** RFC 155 shipped in 0.51.0
  (`prikk archive`, whole-repository; closure fix in 0.52.0); RFC 154 is **unscheduled**; a keyless delta
  form is **not planned**. The owner's ruling on what M2 becomes is pending; the scoping notes are
  private working material (`.git-exclude/specs/`), not decisions.
- **Supply chain:** `cargo deny` / `cargo audit` **green as of 2026-10-15** (IS-22 closed: `ammonia` 4.2.3, `yoke-derive` 0.8.4, lock-only). Checked at every push, release, re-baseline and RFC disposition (RFC 010 D-3).
- **Runtime floor:** prikk ≥ 0.52.0 (`MIN_PRIKK_VERSION`, raised at the 0.52.0 re-baseline, 2026-10-15 — implemented and **conditionally approved by the architect**; to be pushed after the advisory lock update), bubblewrap, curl for SSO. Local binary: 0.52.0.
- **Team:** migrating to a two-agent team (high-capability architect/reviewer + mid-capability
  implementer) under the owner's workflow (owner-directed 2026-09-24). **First assignment (owner ruling):
  the prikk 0.47.0 re-baseline** — [`../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/prikk-0-47-0-rebaseline-handoff-v1.md`](../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/prikk-0-47-0-rebaseline-handoff-v1.md),
  held until prikk 0.47.0 is released and installed. **Overtaken 2026-10-15:** prikk is at 0.52.0, it is
  re-issued as [`../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/prikk-0-52-0-rebaseline-handoff-v2.md`](../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/prikk-0-52-0-rebaseline-handoff-v2.md)
  (T1 schema check, `trust_check` exit code, floor, ledger v1.7); **assignable: 0.52.0 is installed (2026-10-15)**.
- **Active handoffs (RFC 010, accepted 2026-10-15), in order:** [`../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/supply-chain-advisories-lock-update-handoff-v1.md`](../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/supply-chain-advisories-lock-update-handoff-v1.md) (it lands before the push), `maintenance-gates-handoff-v1.md`, `build-identity-handoff-v1.md`; then the read-path follow-on [`../rfcs/handoffs/003-read-path/read-concurrency-handoff-v1.md`](../rfcs/handoffs/003-read-path/read-concurrency-handoff-v1.md) (IS-4). The 0.52.0 re-baseline is implemented and conditionally approved (push waits on the lock update). The T13 measurement handoffs under RFC 004 and RFC 006 are authorized and follow the push. Handoffs 004–009 are v1 (pre-measurement) — see the quarantine rule in [`../rfcs/handoffs/README.md`](../rfcs/handoffs/README.md).

## Pending on the owner


- Make the `ghcr.io/prikk-vcs/planeter` package public (until then the image attestation cannot be
  verified anonymously; the tarball attestations verify).
- **Convey** the architect's requirements note for prikk's 0.54.0 read-only command design round, once the spike reports (SCHEDULE plan 1, step 4). prikk's plan letter of 2026-10-15 is folded into ledger v1.6 (PK-46, PK-47); our reply to it needs nothing urgent.
- Decide the floor at the re-baseline's review request (SCHEDULE D-1).
- The measurement spike (SCHEDULE D-7) is **authorized (owner, 2026-10-15)**; it is **startable now** (prikk 0.52.0 is installed, the lock update is pushed). Handoff: [`../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/transport-measurement-spike-handoff-v1.md`](../rfcs/handoffs/010-upstream-and-supply-chain-maintenance/transport-measurement-spike-handoff-v1.md) (measurement only, scratch directory, no repository change).
- Rule on what M2 becomes (SCHEDULE D-8) once the spike reports.

## Pending upstream (prikk's actual releases and plan, from its letters and ROADMAP of 2026-10-13/15)

| prikk | What touches planeter | planeter action |
|---|---|---|
| **0.47.0–0.50.0** (released) | PK-30 fix, `PRIKK_EXCHANGE_MAX_BYTES` (0.47.0); recovery and cost at depth (0.48.0); crash-state recovery, import refusals, dependency-ordered writes, `ref complete` (0.49.0); corrections (0.50.0) | one re-baseline handoff v2 against **0.52.0** (the 0.47.0 handoff is overtaken); floor decision (PK-42) |
| **0.51.0** (released 2026-10-13) | RFC 155 as `prikk archive`; closure defect for repositories that accepted a push | do not serve archives from 0.51.0 |
| **0.52.0** (released) | archive closure fix; export refuses over-256 MiB objects; `verify` names a failed received ref; `trust maintainer check` exit code | re-baseline target; spike M1–M11 (RFC 004 scoping); `trust_check` exit-status handling |
| **0.53.0 / 0.54.0 / 0.55.0** (prikk owner's plan, 2026-10-15; PK-46) | 0.53.0 commit cost; 0.54.0 the read-only "files at a point" command (design round first, shown to us) and removal of unreachable received content; 0.55.0 `bundle import` memory | requirements note before the 0.54.0 design round; CI snapshots use copy/overlay until the command exists |
| **unscheduled** | RFC 154 adoption act; a keyless delta archive and the append-only promise (**not scheduled; decided with our first-deployment numbers, not before 0.55.0**, PK-47); RFC 158 Stages B–D incl. format 8 | plan as unavailable; instrument fetches from the first deployment; revisit on prikk's word (PK-35, PK-47) |

## Issue register

Small, known items that have no RFC of their own. `Disposition` says where each goes; nothing here is
assigned until a handoff names it.

| ID | Issue | Origin | Disposition |
|---|---|---|---|
| IS-1 | ~~`planeter` has no `--version` flag; an install check that ran the binary started a server~~ — fixed 2026-10-15 (RFC 010 D-6, build-identity handoff): `--version`/`-V`, `--help`/`-h`, unknown argument exit 2; **architect review: approved** (release-built `--version` is verified at the next real tag) | 0.2.0 release verification | closed on review |
| IS-2 | No admin path to link an OIDC `(issuer, subject)` to an account — store API only, so SSO needs code to become usable | 0.2.0 OIDC | design first (a `planeter admin link-oidc` subcommand or an equivalent), then a handoff |
| IS-3 | The driver's `bundle_import` passes the artifact positionally; prikk 0.46.0 takes `--input` | transport measurement 2026-09-23 | **re-baseline v2 T4 — implemented (`--input`, round-trip test); architect review: approved** |
| IS-4 | ~~Synchronous prikk subprocess calls run inside async handlers without `spawn_blocking`, and nothing caps concurrent prikk subprocesses. **Re-rated 2026-10-15: an unauthenticated denial of service on any public repository** (each in-flight call pins a tokio worker for up to the 120 s bound)~~ — fixed 2026-10-15 (read-concurrency handoff): every read-path prikk call runs through one gate (`spawn_blocking` + semaphore, bounded wait, 503 + `Retry-After`); awaiting review | RFC 003 review; architect review 2026-10-13 | closed on review |
| IS-5 | Blame/annotate view awaits a prikk verb (gap catalogue A3, not scheduled by prikk) | RFC 003 T7 | gated on prikk |
| IS-6 | Derived read cache (RFC 003 D-1/D-3) not built — nothing invalidates it until writes exist | RFC 003 | with M2 |
| IS-7 | No "list my repositories" home page — needs a `ReadService` listing method under `authorize()` | 0.1.1 UI | design first, then a handoff |
| IS-8 | Login throttles are in memory, per process (RR-11) | threat model v0.2 | accepted residual; rate-limit `/login` at the proxy for multi-replica deployments |
| IS-9 | No second factor (RR-10); an OIDC provider that enforces MFA is the interim answer | threat model | next auth increment |
| IS-10 | Handoffs 004–009 are v1 from 2026-09-15, written before any prikk measurement | migration review 2026-09-24 | re-issue as v2 before any assignment; 004 v2 waits for RFC 155/154 |
| IS-11 | prikk correspondence is private (`.git-exclude/upstream/`); only the ledger is the committed record | process | keep folding every reply into the ledger (UPSTREAM.md rule) |
| IS-12 | ~~The design set is v0.1 with no as-built delta~~ — closed 2026-09-24: `planeter-01`, `-02`, `-04` carry a *Revision v0.2* section; `-00` charter and `docs/SCHEDULE.md` added | migration review | closed |
| IS-13 | The repository record's `prikk_format_version` (RFC 009 / CT-04) is never populated; the format rule is only a deployment rule today | design-set revision 2026-09-24 | populate at create/open from prikk's reported format (needs a prikk verb or `status` field — check at the 0.47.0 re-baseline); RFC 009 |
| IS-14 | ENF-4's handler-enumeration test (every surface handler consults `authorize()`) is not written | design-set revision 2026-09-24 | next interim handoff; small |
| IS-15 | The prikk sandbox bounds wall-time only: no memory, CPU, file-size or process limits, while prikk's own figures are ≈3× (`verify`) and ≈6× (`bundle import`) the artifact (ledger PK-40) | architect review 2026-10-13 | RFC 004 v2 (S4, S5); an interim handoff before any write path |
| IS-16 | Raw-content isolation is operator configuration: `PLANETER_CONTENT_ORIGIN` defaults to the app origin and the raw route also answers on the app host (no `Host` gating). The attachment, `nosniff` and sandbox-CSP headers contain it; SECURITY.md and threat model C-6c overstate "served only from an isolated origin" | architect review 2026-10-13 | small interim handoff: gate the route by host, refuse or warn on a non-loopback bind with same-origin content, correct the wording |
| IS-17 | `release.yml` and `ci.yml` pin third-party actions by mutable tag (`@v7`, `@stable`, `@v2`, …), contrary to threat model C-5d/C-11, on the workflow that builds the attested artifacts | architect review 2026-10-13 | small interim handoff: pin by digest |
| IS-18 | The container run recipe needs `seccomp=unconfined` (and often `SYS_ADMIN`) for bubblewrap, weakening the outer boundary; the image bakes prikk 0.46.0, without the 0.49.0 robustness and security fixes | architect review 2026-10-13 | state the trade-off in the threat model; the floor moves at the re-baseline |
| IS-19 | Requirements with no theme, RFC or milestone: CAP-10 (search, notifications); export (NG-7, OPS-6, CT-05); fork and mirror (CAP-1); the contribution model for non-writers (no `propose` action); the binding of a prikk author key to a forge account | architect review 2026-10-13 | decision requests (OQ-4) and RFCs before M3; RFC 005 v2 and an RFC 002 follow-on |
| IS-20 | The RFC folders do not tell the truth (RFC 000): 004–009 sit in `accepted/` while quarantined; 001–003 are `done/` with deferred items recorded only in review requests and here | architect review 2026-10-13 | **closed 2026-10-15**: RFCs 004–009 re-opened to `proposed/`; deferred and follow-on notes added to the Status of RFCs 001–003 |
| IS-21 | prikk 0.52.0's `trust maintainer check` exits 1 for "not trusted" (it exited 0); `planeter-prikk::trust_check` keys on exit status | prikk letter 2026-10-15 | re-baseline v2 T5 — implemented (document read on exit 1, typed failure otherwise), awaiting review |
| IS-22 | ~~The supply-chain gates are red: `ammonia 4.1.4` RUSTSEC-2021-0157 and -0158 (not reachable through the default `clean`, architect-tested) and `yoke-derive 0.8.3` yanked; `release.yml` would fail its own gates~~ — closed 2026-10-15: lock-only update (`ammonia` 4.2.3, `yoke-derive` 0.8.4), deny and audit exit 0, sanitizer behaviour pinned by `planeter-web/tests/sanitize_advisories.rs` | re-baseline review request 2026-10-15 | closed |
| IS-23 | 25 source files carry inline `#[cfg(test)]` modules, against `project-instructions-rust.md` | re-baseline review request 2026-10-15 | one separate mechanical migration handoff among the hardening handoffs; new tests never inline |
| IS-24 | Sign-in, session and token checks, Argon2id verification and the SQLite mutex run synchronously on the async workers: the IS-4 class, reachable unauthenticated (found by the IS-4 review) | dev team 2026-10-15 | `rfcs/handoffs/002-authorization/auth-blocking-handoff-v1.md` (assignable; T0 measures first) |
| IS-25 | No per-repository fairness: a slow repository can hold every prikk slot for the 120 s wall-time bound; options (a per-repository sub-cap, a shorter read bound) | dev team 2026-10-15 | decide after measurement (RFC 004 v2 D-5 and the spike); not assignable |
| IS-26 | The CI compiler is the runner image's and `runs-on` is `ubuntu-latest`; release pins `1.99.0` | dev team 2026-10-15 | `rfcs/handoffs/010-upstream-and-supply-chain-maintenance/maintenance-gates-handoff-v2.md` (assignable) |

## Process lessons (keep — each one cost a bad push or a wrong claim)

- **CI has no prikk.** Integration tests self-skip without it; run the suite locally **with** prikk and
  **once more with prikk shimmed off PATH** before pushing. A review request names both runs.
- **Gate chains use real exit codes.** Edit script → gates → commit chained with `&&`; never pipe a gate
  through `tail`/`echo` inside the chain; one heredoc per statement; edit scripts assert their anchors.
- **Dependency growth is reported before the commit** with `cargo deny` and `cargo audit` results and the
  crate delta ([`dependency-policy.md`](dependency-policy.md)). Manifest edits for measurement are done as
  a visible, separate step.
- **Status lines drift.** The README claimed "no forge code exists yet" through two releases; audit
  README, ROADMAP and this file at every release, not only the CHANGELOG.
- **Tag, publish, release and upstream letters are owner-authorized**, per release, per letter.
