# planeter — Internal Design Specification

| | |
|---|---|
| Document | planeter Internal Design (detailed design / white-box) — the architect's handoff to the dev team |
| Version | v0.2 (2026-09-24 — the revision section below records the architecture as built through planeter 0.2.0 and what measurement corrected; the v0.1 body of 2026-09-15 stands and is not rewritten) |
| Date | 2026-09-24 (v0.2); 2026-09-15 (v0.1) |
| Inputs | planeter Requirements v0.1 (PU/NG/CAP/STD/SEC/INT/OPS/BN/UD/OQ), External Design v0.1 (BD/AC/TX/WEB/API/AUTH/CI/REG/HOOK/PK/FL/CT/OP/GATED), Threat Model v0.1 (A/TB/T/C/INV/RR); **forge-commons**; prikk reality (2026-09-15 survey + write-path check, prikk `HEAD f6cbd057`); project rules (`.git-exclude/rules/`, §Feature Development: *Requirements → External → **Internal (Detailed)** → Program → Implementation*) |
| Scope | HOW planeter is built: the crate decomposition, the layering rules, the prikk-integration mechanism, the data model, the read and **write** paths, auth/CI/registry internals, concurrency, and — the load-bearing part — **how the security invariants are enforced structurally.** Detailed enough to hand off; it stops short of per-subsystem RFC detail and names where RFCs take over (§13). |
| Not | code, exact database or wire schemas, or the per-subsystem RFCs themselves. |
| ID scheme | `CR-` crate/component · `LAY-` layering & boundary rule · `PKI-` prikk-integration mechanism · `DM-` data model · `RD-` read path · `WR-` write path · `AZ-` auth internals · `CIO-` CI orchestration · `RG-` registry · `CON-` concurrency/consistency · `ENF-` invariant enforcement · `SEQ-` build sequence · `NR-` next-RFC subsystem · `IQ-` internal open question/dependency |

## Revision v0.2 (2026-09-24) — as built through 0.2.0

- **Crates and gates (CR/LAY):** the workspace is `planeter-prikk` (lower), `planeter-store` (lower),
  `planeter-core` (core: hosting, `authorize()`, read service, egress guard, outbound fetcher),
  `planeter-auth` (core: accounts, sessions, throttles, OIDC, tokens, keys, hashing), `planeter-web`
  (upper: UI + API), `planeter-transport` / `-ci` / `-registry` (upper skeletons), binaries `planeter`
  and `planeter-runner` (placeholder). LAY-1 is `tools/boundary-check` (a std-only test in the
  workspace). LAY-2 holds: every shipped surface calls `authorize()`. LAY-3 holds. LAY-4: the
  `forge-seal` cargo feature is reserved and off; **ENF-2** is `tools/enf2-symbol-check/check.sh` in
  CI. LAY-5: `planeter_core::egress::StdEgressGuard` plus `fetch::CurlFetcher` (a bubblewrap-confined
  `curl` pinned to the guard's resolved address); first caller: OIDC discovery/JWKS/token exchange.
- **prikk layer (PKI):** PKI-1 holds (a version pin, `MIN_PRIKK_VERSION` = 0.46.0, refuses older).
  PKI-3 is **bubblewrap** (`--unshare-all --clearenv`, repository-directory-only writes, wall-time
  bound, no operator keys). PKI-2's read map gained `tree`, `cat`, `diff`; the exchange verbs were
  measured: `bundle export/import`, `sync have/summary/accept` run keyless, **`sync build` does not**
  (PK-27). PKI-4 holds (unexpected `schema_version` is a refusal). PKI-5 holds (blame pending).
- **Data model (DM):** DM-1 as built — an opaque repository id maps to the on-disk path; rename and
  transfer keep both (store-level, no surface yet). DM-2 stands as design (M3). DM-3: **no derived
  cache exists yet** (IS-6); every read re-derives. DM-4 stands.
- **Read path (RD):** RD-1 shipped without the cache; RD-2 shipped (ammonia + pulldown-cmark, strict
  CSP, isolated content origin); RD-3 shipped (the `Assurance` distinction; approved-but-unsealed is
  never verified).
- **Write path (WR) — corrected by measurement and by prikk RFC 154:** WR-1 — the server **cannot**
  `sync build` keylessly; fetch/clone is prikk RFC 155's repository-complete artifact (whole-artifact
  first). WR-2/WR-3 stand as measured (keyless accept is idempotent and tamper-refusing; PK-28). WR-4
  stands (design). **WR-5a is superseded**: prikk declined the client-sealable prepared plan; the
  one-click merge is *the maintainer seals with their own prikk client-side and the forge adopts the
  trusted-maintainer-signed fast-forward* (RFC 154, "first fast-forward wins"). WR-5b stands as the
  fallback that works today. WR-5c remains the reserved feature. WR-6 → CON-1 → RFC 004 v2.
  **Transport layering (owner-ruled 2026-09-23, supersedes RFC 004 D-6):** loopback HTTP behind a
  trusted-proxy TLS terminator; SSH via host OpenSSH `ForceCommand` → `planeter ssh-shell`.
- **Auth (AZ):** AZ-1 partial (Argon2id, OIDC, tokens, SSH keys, sessions + CSRF, throttles; no MFA,
  SAML, LDAP). AZ-2 shipped: one pure default-deny `authorize(principal, action, resource)` (RFC 002).
  AZ-3 shipped (`user-permissions ∩ scope`). AZ-4 holds.
- **CIO / RG:** skeleton crates only. **CON:** CON-1 is RFC 004 v2's; CON-3's idempotent accept is
  measured (PK-28). **ENF:** ENF-1 tested (an approved-but-unsealed change never reads verified);
  ENF-2 in CI; ENF-3 partial (sandbox, egress guard, accept-edge bound PK-31; hostile-artifact fixtures
  come with transport); ENF-4 partial — every shipped handler consults `authorize()`, but the
  **handler-enumeration test is not written** (IS-14); ENF-5 trivially true (no cache); ENF-6 holds.
- **Sequencing (SEQ):** SEQ-1 and SEQ-2 done (M0, M1, 0.1.1, 0.2.0). SEQ-3 and SEQ-4 are merged into
  **M2, held** until prikk RFC 155 then 154 (after prikk 0.49.0). SEQ-5 becomes adoption (RFC 154).
- **RFC map (NR) and open questions (IQ):** NR-1 → RFC 004 (held, v2 pending) · NR-2 → RFC 002 (done)
  · NR-3 → RFC 005 (accepted v1; re-issue before M3) · NR-4 → RFC 008 re-based on RFC 154 adoption ·
  NR-5 → RFC 006 · NR-6 → RFC 007 · NR-7 → RFC 009. IQ-1 resolved (independent `planeter-prikk`) ·
  IQ-2 superseded (adoption) · IQ-3 open (transport v2) · IQ-4 resolved (SQLite behind traits, one
  shared `SqliteDb`, PostgreSQL path kept) · IQ-5 done · IQ-6 settled (purely planeter's).

Design stance carried from the set: **planeter carries the hosting weight prikk refuses, over prikk's
stable CLI surface, holding authority prikk never had — without weakening prikk's verification.** The
internal design's job is to make that true *structurally*, so the security invariants (Threat Model §4)
are properties of the build graph and the module boundaries, not habits of careful coding. The single
most important such property, derived below (WR, ENF): **the default build contains no code path that
seals history with a forge-held key** — the construction-level form of OQ-1(a)/INV-2.

---

## 1. Architecture overview & component map (CR-…)

planeter is a Rust workspace (Rust 2024, MSRV-pinned, `forbid(unsafe_code)` outside any isolated FFI —
ecosystem convention), layered so the network-facing surfaces sit *above* a core that never depends on
them, and the prikk boundary is a subprocess boundary at the bottom.

```
                         ┌──────────────── binaries ────────────────┐
                         │  planeter (server)      planeter-runner   │  (runner on separate hosts)
                         └───────────┬───────────────────┬──────────┘
        ── upper (surfaces) ─────────┼───────────────────┼───────────────────────────
          CR-web (HTTP/API/UI/hooks)  CR-transport (TX)   CR-ci      CR-registry
        ── core ─────────────────────┴───────────────────┴───────────────────────────
          CR-core (domain: repos/namespace, orgs/teams, changes/reviews, issues; AUTHORIZATION logic)
          CR-auth (identity, sessions, tokens, SSH keys, authz enforcement primitives)
        ── lower ────────────────────────────────────────────────────────────────────
          CR-prikk (drives the prikk CLI as a subprocess)     CR-store (forge-metadata persistence)
        ─────────────────────────────────────────────────────┼─────────────────────────
                                                              ▼ subprocess, on-disk
                                              prikk CLI  →  one `.prikk` per hosted repo
```

- **CR-prikk** — the prikk-integration layer (§3): invokes the prikk CLI as a sandboxed subprocess,
  parses `--format json`, drives the `bundle`/`sync` artifact commands, and owns the per-repo working
  directory. The *only* component that talks to prikk. (Reuse-vs-build decision: IQ-1 / OQ-7.)
- **CR-store** — forge-metadata persistence (§4): accounts, orgs, teams, permissions, issues, reviews,
  CI config, webhook config — **forge-owned data keyed to prikk object ids**, never authoritative for
  history (INT-4/NG-6). DB-abstracted (SQLite → PostgreSQL, OPS-1).
- **CR-core** — the domain and, critically, the **authorization logic** (§7): repositories and their
  identity/namespace, orgs/teams, the change/review model, issues. Pure-ish logic that re-derives
  repository facts through CR-prikk and reads/writes forge metadata through CR-store. **Depends on no
  upper crate.**
- **CR-auth** — authentication (OAuth/OIDC/SAML/LDAP, sessions, tokens, SSH keys) and the authorization
  *enforcement primitives* CR-core's decisions are expressed in.
- **CR-transport** — the TX artifact-ferry (§6): the fetch/push endpoints that carry prikk artifacts,
  the accept/seal orchestration, per-repo write serialization.
- **CR-web** — the HTTP server: REST API (OpenAPI), the web UI (server-rendered, isolated content
  origin), outbound webhooks. The primary TB-1 surface.
- **CR-ci** — CI orchestration (§8): pipeline model, the runner protocol, job-token minting, secret
  handling. Runners execute elsewhere (CR-runner).
- **CR-registry** — OCI + language package registries (§9).
- **planeter** (binary) — wires the crates, loads config, serves. **planeter-runner** (binary) — the CI
  agent, deployed on separate hosts (OPS-4), a different trust domain.

## 2. Layering & boundary rules (LAY-…)

The layering is enforced, not merely drawn — the lesson prikk's own RFC 149 settled (a layer is a
gate rule, checked in CI). planeter adopts the same discipline from day one:

- **LAY-1 — Upper may depend on core may depend on lower; never the reverse.** CR-core and CR-auth do
  **not** depend on CR-web/transport/ci/registry. A production edge from lower/core into an upper crate
  fails a `boundary-check`-style gate, naming the edge. (Directly echoes RFC 149's `LAYER` rule.)
- **LAY-2 — Authorization lives in CR-core, called by every surface** (INV-4). No authorization decision
  is made in the web/transport tier alone; a surface asks CR-core "may this principal do this?" and gets
  a yes/no. This makes "enforced on every access path" a structural property, not a per-handler habit.
- **LAY-3 — Only CR-prikk touches prikk** (BD-03). No other crate invokes the prikk CLI or parses its
  output; the process boundary is also the module boundary.
- **LAY-4 — The forge-seal capability is an isolated, off-by-default module** (ENF-2, and the heart of
  §11). Any code that would seal history with a *forge-held* key (OQ-1 option b) lives behind a
  non-default cargo feature in a single module, so the **default build cannot seal** — the
  construction-level guarantee for INV-2, exactly the `#[cfg]`-gate mechanism prikk uses for its own
  write/test surfaces.
- **LAY-5 — One egress guard for every outbound fetch** (INV-3, T-8): webhooks, mirroring, import, and
  OIDC discovery all route through a single SSRF-filtering egress component; there is no second way to
  make the server fetch a URL.

## 3. The prikk-integration layer (CR-prikk) (PKI-…)

- **PKI-1 — Subprocess, not linkage** (INT-1). CR-prikk shells out to the prikk binary against a repo's
  on-disk `.prikk`, capturing `--format json` on stdout. It links no prikk crate; the prikk version is a
  runtime dependency, and the supported prikk **format versions** are pinned (OP-04/UD-3).
- **PKI-2 — The command map.** CR-prikk exposes typed operations over prikk's real verbs:
  - *Reads* (→ RD): `log`, `show`, `status`, `branch`, `tag`, `verify`, `worktree-status`,
    `checkout --patch-plan --content-path`, `bundle preview`, `trust`/`key status` — all `--format json`.
  - *Exchange* (→ WR): `bundle export/import`; `sync summary` (PSYNCSU1), `sync have <ref>` (PSYNCHV1),
    `sync build <ref> --have` (→ PEXCH002), `sync accept <file> --claims-out`, `sync pending`,
    `sync seal <ref> --claim` (**the only key-needing verb** — WR-5).
- **PKI-3 — Sandbox the parser** (C-4c/INV-3). prikk parses untrusted pushed bytes, so CR-prikk runs
  each prikk invocation under confinement: a restricted user, a filesystem view limited to that repo's
  directory and the artifact temp, no ambient credentials, resource/time bounds (C-4e). A prikk parser
  fault is contained in that subprocess, not forge-wide.
- **PKI-4 — Typed JSON, versioned.** CR-prikk parses each command's JSON into typed values and records
  which prikk CLI/format version produced them; an unrecognized or unsupported version is a refusal, not
  a guess (OP-04). *This is the crate that most benefits from reusing stikk's `stikk-prikk` layer* —
  IQ-1/OQ-7.
- **PKI-5 — Never fabricate a gap** (INT-5/UD-1). A read prikk does not expose in machine-readable form
  (raw blob, blame/annotate) is surfaced by CR-core as *pending a prikk increment*, never synthesized by
  reaching around the CLI.

## 4. Data model (DM-…)

- **DM-1 — Repository identity is planeter's, above prikk.** A repository record = `{owner (user|org),
  name, visibility, → on-disk .prikk path}`. planeter assigns the owner/name/URL that prikk deliberately
  omits (INT-3, RFC 145 §7); prikk sees only an anonymous `.prikk`. Hosted repos are laid out as one
  `.prikk` per repository under a planeter-managed root.
- **DM-2 — A change/PR is prikk's "accepted-but-unsealed" state, plus review metadata.** This is the
  key mapping (WR-4): pushed author-signed patches that CR-transport `sync accept`ed but has **not**
  sealed onto the target ref are exactly an open pull request. The forge stores review metadata
  (comments, approvals, required checks) keyed to the prikk **claim id** returned by `sync accept
  --claims-out`; the patches themselves live in prikk, not the forge DB.
- **DM-3 — Forge-owned vs re-derived.** CR-store holds **forge-owned** data only: accounts, orgs, teams,
  permissions, issues, reviews, CI config/results, webhook config, and *pointers* (prikk object/claim
  ids, ref names). Repository **history, contents, and verification status are re-derived** from prikk
  and cached; the cache is invalidated on write and is never the authority (INV-6). A DB compromise can
  mislead a rendered page but cannot produce history a client's prikk accepts (T-10).
- **DM-4 — Cross-references by prikk id.** Issues/reviews/changes link to repository objects by prikk's
  own content-addressed ids, so a link survives independently of the forge cache.

## 5. The read path (RD-…)

- **RD-1 — Re-derive, then cache.** A page or API read resolves the repo → its `.prikk` (DM-1) → the
  relevant prikk read command via CR-prikk (PKI-2) → typed JSON → rendered/serialized. Results are
  cached in CR-store keyed by the prikk object id they derive from; a write to the repo invalidates the
  affected cache entries.
- **RD-2 — Render safely** (STD-6/T-6). Markdown/HTML is sanitized; raw repository content, uploads and
  avatars are served from an **isolated content origin** (CR-web serves the app origin; a distinct
  origin serves raw bytes). The CSP is authored with the app, not retrofitted.
- **RD-3 — Show prikk's honesty** (WEB-06/SEC-1). Every history view carries prikk's `verify` status and
  distinguishes **prikk-verified** from **forge-approved**; an unsigned/unverifiable object reads as
  such. The verify status is re-derived, never trusted from cache alone for a security display.

## 6. The write path (WR-…) — the centerpiece

This realizes CAP-2/CAP-5 and the OQ-1(a) trust model. It follows prikk's two-phase **accept then seal**
mechanic, which is what lets the forge hold author-signed content through the whole flow while signing
nothing.

- **WR-1 — Fetch/clone = server builds, client accepts.** For an initial clone, CR-transport serves a
  `bundle export`; the client `bundle import`s. For incremental fetch: the client sends its `have`
  (PSYNCHV1), the server `sync build <ref> --have <client-have>` produces a PEXCH002, the client
  `sync accept`s it locally. Authorization (AZ) gates private repos; anonymous is allowed for public.
- **WR-2 — Push = client builds, server accepts — with no key.** The client `sync build`s a PEXCH002
  against the server's advertised `have`/summary and uploads it (over TLS, TX-06). CR-transport
  **authorizes the target ref** (AZ, per-ref), then CR-prikk runs `sync accept`, ingesting the
  **author-signed** patches as **accepted-but-unsealed** claims. *No maintainer key is involved in a
  push* — accept ingests, it does not seal. planeter now holds author-signed patches it has not blessed.
- **WR-3 — `verify` on accept.** Immediately after accept, CR-prikk runs `verify`; a push whose patches
  fail prikk's own verification is rejected with a named reason (never silently stored). "Accepted" means
  "ingested and internally verified", not "sealed onto the ref".
- **WR-4 — Propose & review = the accepted-unsealed state as a PR** (DM-2). The pending claims
  (`sync pending`) are presented as a proposed change: CR-core renders their effect via `show`, attaches
  review metadata (CR-store), and gates them on CI (CIO). Nothing here needs a key; a reviewer reads
  prikk's own view of the change.
- **WR-5 — Merge = seal, and the seal is the client's** (OQ-1 a / INV-2). Sealing an accepted claim onto
  a protected ref (`sync seal <ref> --claim <id>`) is the **one** operation that needs a maintainer key.
  Per OQ-1(a), that key is the maintainer's own and lives client-side. Two mechanisms:
  - **WR-5a — the default, one-click via UD-6.** CR-transport hands the maintainer's client a *sealable
    package* for the pending claim; the maintainer's own prikk seals it (key never leaves the client) and
    returns the sealed block, which CR-transport stores and re-verifies. This is the "merge button" that
    holds no forge key. **It depends on a prikk affordance to seal a claim the client did not itself
    accept** — the precise shape of UD-6 (IQ-2); until it lands, WR-5b is the fallback.
  - **WR-5b — the fallback that works today.** The maintainer fetches the pending claim to their local
    repo (WR-1), `sync seal`s it locally, and pushes the sealed block (WR-2 storing a sealed result).
    Correct and keyless-for-the-forge, but a manual pull-seal-push rather than one click.
  - **WR-5c — the opt-in exception (option b).** A per-repository, key-isolated, forge-attributed
    server-side signer (LAY-4, behind the non-default feature) may perform the seal for teams that choose
    it — never in the default build (ENF-2, RR-2).
- **WR-6 — Writes per repository are serialized** (CON-1). accept and seal for one repo are serialized by
  CR-transport so two pushes never race the prikk subprocess; prikk's local locks sit beneath (UD-4, to
  confirm).

## 7. Authentication & authorization internals (AZ-…)

- **AZ-1 — Authentication** (CR-auth, STD-2): OAuth2/OIDC (code+PKCE) and optional SAML/LDAP for humans;
  Argon2id for any local password; TOTP/WebAuthn second factors; SSH public keys and scoped, hashed
  tokens for machines. Sessions are `Secure`/`HttpOnly`/`SameSite` with CSRF tokens (STD-6).
- **AZ-2 — Authorization is a CR-core service** (LAY-2/INV-4): a single `authorize(principal, action,
  resource)` decision point, default-deny, consulted by web, API, and transport alike. Per-repo and
  **per-ref** roles (read/push/merge-seal/admin); protected-ref rules (required review/checks, no
  force-replace) are authorization outcomes, not UI conveniences.
- **AZ-3 — Scope enforced per endpoint** (STD-3): a token's scope is checked at each API endpoint and at
  each transport verb; a read token cannot reach a write path.
- **AZ-4 — The mapping to prikk** (SEC-1): authorization decides *whether* accept/seal may proceed; prikk
  decides *whether the result verifies*. The two never merge. planeter authorizes a push; the patches are
  still author-signed and prikk-verified on their own (WR-3); planeter authorizes a merge; the seal is
  still the maintainer's own signature (WR-5).

## 8. CI orchestration internals (CIO-…)

- **CIO-1 — Pipelines** defined in-repository, triggered on push (WR-2), on a proposed change (WR-4), and
  on schedule. CR-ci resolves and enqueues jobs.
- **CIO-2 — The runner protocol.** planeter-runner (separate host, OPS-4) registers to CR-ci over HTTPS
  with a scoped token, pulls a job, executes it in a fresh container, and streams status/logs back.
  Runners are ephemeral and in a distinct trust domain (INV-5).
- **CIO-3 — Least privilege & fork safety** (C-5): secrets are withheld from fork-originated changes and
  require maintainer approval to run; a job token is short-lived, scoped to that job, and is **never a
  signing key** (WR-5 is not a CI capability).
- **CIO-4 — Integrity custodian** (SEC-4): CR-ci stores and verifies artifact signatures/provenance;
  generation is a job's own doing, planeter keeps the record.

## 9. Registry internals (RG-…)

- **RG-1 — OCI Distribution** first (containers + OCI artifacts as the substrate), language registries by
  demand (STD-5). Each is a faithful protocol implementation behind CR-registry.
- **RG-2 — Rules enforced structurally**: authenticated, scoped publishing (AZ); **immutable published
  versions** (a published digest is write-once); digest-addressed **object storage** for bytes (OPS-1).

## 10. Concurrency & consistency (CON-…)

- **CON-1 — Per-repo write serialization** (OP-01/UD-4): a per-repository write lease in CR-transport
  serializes accept/seal; reads run concurrently against the cache + prikk. prikk's local locks are the
  layer beneath, whose sufficiency under this serialization is confirmed in the transport RFC (IQ-3).
- **CON-2 — Cache coherence** (DM-3): a write invalidates the derived-cache entries for the affected
  refs/objects; a stale read is labelled stale rather than served as fresh (OP-06). prikk remains the
  re-derivable source of truth.
- **CON-3 — Idempotent exchange**: accept is keyed by claim id so a retried upload does not double-apply;
  a seal is keyed by claim id so a retried merge is a no-op, not a second block.

## 11. Security-invariant enforcement (ENF-…) — invariants as structure

Each Threat-Model invariant is given a *structural* home so it is testable, not aspirational.

- **ENF-1 — INV-1 (no manufactured verification):** authorization (CR-core) and verification (prikk) are
  different subsystems with no code path from the first to a prikk signature; RD-3/WEB-06 keep the two
  claims distinct in every view. Tested by asserting a forge-approved-but-unsealed change never reads as
  verified.
- **ENF-2 — INV-2 (no forge-held signing key by default):** the seal-with-forge-key path is a single
  module behind a non-default feature (LAY-4). **The default build links no such path** — a CI check
  greps the default build's symbols for the forge-signer and fails if present, the construction-level
  guarantee (the boundary-rule discipline of prikk RFC 149, pointed at the signing capability). WR-2's
  push and WR-5a/b's client-seal need no forge key at all.
- **ENF-3 — INV-3 (untrusted input; confined prikk; SSRF):** PKI-3 sandboxes prikk; LAY-5 routes every
  outbound fetch through the one egress guard; pushed bytes are bounded (C-4e). Tested with a hostile
  artifact fixture and an SSRF fixture per fetch site.
- **ENF-4 — INV-4 (default-deny authz everywhere):** LAY-2's single decision point; a test enumerates
  every surface handler and asserts it consults `authorize` before acting.
- **ENF-5 — INV-6 (prikk is source of truth):** DM-3 keeps history out of the authoritative store; a test
  reconstructs a repository view from prikk alone with the forge cache cleared.
- **ENF-6 — INV-9 (AI outside the trust boundary):** no CR-* exposes an AI actor in the authority path;
  content is passed to rendering as data (RD-2), never interpreted as instructions. Enforced by the
  absence of an AI-authority dependency and reviewed as a boundary rule.

## 12. Build sequencing (SEQ-…)

The order maximizes what ships without a prikk change or an open owner gate (External Design §8).

1. **SEQ-1 — Foundation:** CR-prikk (subprocess driver + typed JSON + sandbox), CR-store (DB
   abstraction), CR-core (domain + `authorize`), CR-auth. No network surface yet.
2. **SEQ-2 — Read/host spine:** multi-repo hosting/identity (DM-1), the read path (RD, CR-web browse),
   authentication and authorization (AZ). Buildable now; delivers a real hosted, browsable, access-
   controlled forge.
3. **SEQ-3 — Transport, read side:** fetch/clone (WR-1).
4. **SEQ-4 — Transport, write side:** push = accept + verify (WR-2/WR-3) — keyless; then the
   propose/review model (WR-4) and issues (CR-core).
5. **SEQ-5 — Merge:** WR-5b (works today) first; WR-5a one-click once UD-6 lands (IQ-2).
6. **SEQ-6 — CI, then registry** (CIO, RG).
7. **SEQ-7 — Frontier deferred** (federation/AI/portable identity) per forge-commons verdicts.

## 13. Subsystems that become RFCs (NR-…)

Per project rules, detailed per-subsystem design and the schedule live in `rfcs/` + `ROADMAP.md`. This
internal design is the architecture; each of these is a first RFC + handoff (and **planeter needs its
own `rfcs/` directory and `ROADMAP.md` set up first** — IQ-5):

- **NR-1 — The transport envelope** (TX): the HTTP/SSH framing that carries prikk artifacts, negotiation,
  and per-repo serialization; and the **client helper** (`planeter` client / prikk remote-helper, TX-05).
- **NR-2 — The authorization model** (AZ-2): the role/permission schema and the `authorize` contract.
- **NR-3 — The change/review model** (WR-4/DM-2): accepted-unsealed-as-PR, review metadata, merge.
- **NR-4 — The seal/UD-6 mechanism** (WR-5a): the client-sealable-claim affordance — jointly with a prikk
  RFC, since it needs a prikk-side capability.
- **NR-5 — The CI runner protocol** (CIO-2) and **NR-6 — the registry** (RG).
- **NR-7 — The hosted-format durability policy** (OQ-6/GATED-3): version pinning and migration.

## 14. Internal open questions & dependencies (IQ-…)

- **IQ-1 (→ OQ-7) — Reuse stikk's `stikk-prikk` for CR-prikk, or build independently?** stikk already
  drives the prikk CLI and parses its JSON. Reuse shares maintenance and couples releases; independent
  decouples at the cost of duplication. **Recommendation:** factor CR-prikk behind a trait and evaluate
  `stikk-prikk` as the first implementation — decide with the dev team.
- **IQ-2 (→ UD-6) — The client-sealable-claim affordance.** WR-5a needs prikk to let a client seal a
  claim it did not itself accept (a "sealable package" out, a sealed block back). Confirm against prikk's
  `sync accept`/`pending`/`seal` internals; if absent, it is a joint prikk RFC (NR-4). WR-5b covers the
  gap meanwhile.
- **IQ-3 (→ UD-4) — prikk local-locking sufficiency** beneath CR-transport's per-repo serialization
  (CON-1) — confirm in the transport RFC.
- **IQ-4 — Database choice and the store abstraction** (OPS-1): SQLite-first with a PostgreSQL path;
  fix the abstraction boundary so the choice is deferrable.
- **IQ-5 — Set up planeter's `rfcs/` (five-folder lifecycle) and `ROADMAP.md`** before NR-* begin, per
  project rules.
- **IQ-6 (→ OQ-2) — The ref-authority mapping**: whether planeter's per-ref authorization is purely
  planeter's or anchors to a prikk-side notion — still the owner's, and it shapes AZ-2/WR-5.

## 15. Traceability (internal design → prior docs)

| Internal design | Realizes / grounds in |
|---|---|
| CR-1…9, LAY-1…5 | BD-01…06, INT-1/4, SEC-1; RFC 149 layering discipline |
| PKI-1…5 | INT-1/2/5, PK-01…06, C-4c, UD-1 |
| DM-1…4 | INT-3/4, NG-6, CT-03, RFC 145 §7 |
| RD-1…3 | CAP-8, STD-6, WEB-01/05/06, INV-6 |
| WR-1…6 | CAP-2/5, TX-01/02, FL-02/03/04, OQ-1(a), UD-6, prikk `sync accept`/`seal` |
| AZ-1…4 | CAP-3, STD-2/3, SEC-1/5, AUTH-01…05 |
| CIO-1…4 | CAP-6, CI-01…04, INV-5, SEC-4 |
| RG-1…2 | CAP-7, STD-5, REG-01/02 |
| CON-1…3 | OP-01, UD-4, INV-6 |
| ENF-1…6 | Threat Model INV-1/2/3/4/6/9 |
| SEQ-1…7, NR-1…7 | External Design §8 sequencing; project-rules workflow |
| IQ-1…6 | OQ-2/6/7, UD-4/6, OPS-1 |

*End of Internal Design v0.1. This is the architect's detailed handoff: the crate decomposition, the
layering gate, the prikk-subprocess mechanism, the accept-then-seal write path, and the structural
enforcement of the security invariants. The load-bearing results: **the forge signs nothing in the
default build** (ENF-2), realized because prikk's `sync accept` ingests author-signed patches without a
key and only `sync seal` needs one (WR); **authorization is a single core service** consulted by every
surface (LAY-2); and **prikk stays the re-derivable source of truth** (DM-3). Next: stand up planeter's
`rfcs/` + `ROADMAP.md` (IQ-5), then the NR-* subsystem RFCs in SEQ order, with NR-4 (seal/UD-6) taken up
jointly with prikk. Program design and implementation follow per the project-rules workflow.*
