# planeter — project status

| | |
|---|---|
| Document | The dated brief a new session reads **first**: where the project is, what is held and why, what is pending on whom, and the issue register. Updated at every release, RFC disposition and owner ruling. |
| As of | 2026-09-24 — planeter **0.2.0** released |
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
- **Runtime floor:** prikk ≥ 0.46.0 (`MIN_PRIKK_VERSION`), bubblewrap, curl for SSO. Local binary: 0.46.0.
- **Team:** migrating to a two-agent team (high-capability architect/reviewer + mid-capability
  implementer) under the owner's workflow (owner-directed 2026-09-24). **First assignment (owner ruling):
  the prikk 0.47.0 re-baseline** — [`../rfcs/handoffs/interim/prikk-0-47-0-rebaseline-handoff-v1.md`](../rfcs/handoffs/interim/prikk-0-47-0-rebaseline-handoff-v1.md),
  held until prikk 0.47.0 is released and installed.
- **Active handoff:** none assignable today. Handoffs 004–009 are v1 (pre-measurement) — see the
  quarantine rule in [`../rfcs/handoffs/README.md`](../rfcs/handoffs/README.md).

## Pending on the owner

- Make the `ghcr.io/prikk-vcs/planeter` package public (until then the image attestation cannot be
  verified anonymously; the tarball attestations verify).
- Adjust and send the draft letter to the prikk team (planning-assumption numbers for their fetch
  design): `.git-exclude/upstream/prikk/send/draft/2026-09-23-ack-sync-build-ruling-and-fetch-numbers.md` (private).
- Install prikk 0.47.0 when it ships (`cargo install prikk --version 0.47.0 --locked`) — that starts
  the re-baseline handoff.

## Pending upstream (prikk's schedule of 2026-09-23, by theme, no dates)

| prikk release | Contents that touch planeter | planeter action |
|---|---|---|
| **0.47.0** | PK-30 fix (explicit `--ref` on an unsealed repo), PK-31 (`sync accept` refuses over-size before reading; `PRIKK_EXCHANGE_MAX_BYTES`), RFC 136 inc. 2c | the re-baseline handoff (first assignment) |
| **0.48.0** | RFC 158 B/C: streaming, chunk manifests, **repository format 8** | re-baseline; check format-8 consequences for hosted repos (RELEASING §Format 7 rule generalizes); prikk invited planeter's input on the streaming-bound design |
| **0.49.0** | RFC 158 D (reclamation) | re-baseline |
| after 0.49.0 | **RFC 155** (repository-complete artifact) then **RFC 154** (trusted fast-forward adoption) | transport handoff v2 → M2; Track B1/B2 |

## Issue register

Small, known items that have no RFC of their own. `Disposition` says where each goes; nothing here is
assigned until a handoff names it.

| ID | Issue | Origin | Disposition |
|---|---|---|---|
| IS-1 | `planeter` has no `--version` flag; an install check that ran the binary started a server | 0.2.0 release verification | next interim handoff (small) |
| IS-2 | No admin path to link an OIDC `(issuer, subject)` to an account — store API only, so SSO needs code to become usable | 0.2.0 OIDC | design first (a `planeter admin link-oidc` subcommand or an equivalent), then a handoff |
| IS-3 | The driver's `bundle_import` passes the artifact positionally; prikk 0.46.0 takes `--input` | transport measurement 2026-09-23 | **re-baseline handoff T4** |
| IS-4 | Synchronous prikk subprocess calls run inside async handlers without `spawn_blocking` | RFC 003 review | next interim handoff |
| IS-5 | Blame/annotate view awaits a prikk verb (gap catalogue A3, not scheduled by prikk) | RFC 003 T7 | gated on prikk |
| IS-6 | Derived read cache (RFC 003 D-5) not built — nothing invalidates it until writes exist | RFC 003 | with M2 |
| IS-7 | No "list my repositories" home page — needs a `ReadService` listing method under `authorize()` | 0.1.1 UI | design first, then a handoff |
| IS-8 | Login throttles are in memory, per process (RR-11) | threat model v0.2 | accepted residual; rate-limit `/login` at the proxy for multi-replica deployments |
| IS-9 | No second factor (RR-10); an OIDC provider that enforces MFA is the interim answer | threat model | next auth increment |
| IS-10 | Handoffs 004–009 are v1 from 2026-09-15, written before any prikk measurement | migration review 2026-09-24 | re-issue as v2 before any assignment; 004 v2 waits for RFC 155/154 |
| IS-11 | prikk correspondence is private (`.git-exclude/upstream/`); only the ledger is the committed record | process | keep folding every reply into the ledger (UPSTREAM.md rule) |
| IS-12 | ~~The design set is v0.1 with no as-built delta~~ — closed 2026-09-24: `planeter-01`, `-02`, `-04` carry a *Revision v0.2* section; `-00` charter and `docs/SCHEDULE.md` added | migration review | closed |
| IS-13 | The repository record's `prikk_format_version` (RFC 009 / CT-04) is never populated; the format rule is only a deployment rule today | design-set revision 2026-09-24 | populate at create/open from prikk's reported format (needs a prikk verb or `status` field — check at the 0.47.0 re-baseline); RFC 009 |
| IS-14 | ENF-4's handler-enumeration test (every surface handler consults `authorize()`) is not written | design-set revision 2026-09-24 | next interim handoff; small |

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
