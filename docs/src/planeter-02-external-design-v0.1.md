# planeter — External Design Specification

| | |
|---|---|
| Document | planeter External Design (black-box view) |
| Version | v0.1 (draft for review) |
| Date | 2026-09-15 |
| Inputs | planeter Requirements v0.1 (`planeter-01-requirements-spec-v0.1.md`) — cited as PU/NG/CAP/STD/SEC/INT/OPS/BN/UD/OQ; **forge-commons** (the standards frame); **prikk reality** (2026-09-15 survey, prikk `HEAD f6cbd057`); RFC 145 (Shape D), RFC 116 (sync as artifacts); the stikk integration precedent; project rules |
| Scope | WHAT planeter exposes at its boundaries — its transport, web, API, authentication, CI, registry and webhook surfaces, and its prikk-integration surface — **for the parts designable before the owner's open questions (OQ-1…OQ-7) are settled.** The design goes up to the write/merge **trust gate** and **stops** there (§8), rather than presuming where signing keys live. |
| Not | internal architecture, database schema, wire/artifact byte formats (prikk's), the OpenAPI document itself, or code. |
| ID scheme | `BD-` boundary · `AC-` actor · `TX-` transport surface · `WEB-` web surface · `API-` REST surface · `AUTH-` identity/authorization surface · `CI-` CI/runner surface · `REG-` registry surface · `HOOK-` webhook surface · `PK-` prikk-integration surface · `FL-` interaction flow · `CT-` external data contract · `OP-` operational behaviour · `GATED-` a surface blocked on an owner question or a prikk-side dependency |

Design stance carried from the requirements: **planeter carries the hosting weight prikk refuses, over
prikk's stable CLI read surface, holding authority prikk never had — and never weakening prikk's own
verification in the process.** Every user-visible surface therefore keeps two claims distinct:
*planeter approved this* and *prikk verifies this* (SEC-1, NG-3). Where a familiar forge would just
"merge", planeter merges **and** shows whose signature made the result prikk-verifiable — or, until the
owner rules where that signature comes from (OQ-1), stops at the gate and says so.

---

## 1. System boundary & actors

### 1.1 Boundary (BD-…)

```
   [browser]   [prikk client + keys]   [CI runner]   [package manager]        [forge operator]
       │              │  (AC-02)          (AC-06)          │                      (AC-05)
       │ HTTPS        │ HTTPS/SSH         │ HTTPS          │ OCI/registry          │ deploys, backs up,
       ▼              ▼  (ferried prikk   ▼                ▼   protocols           ▼  runs — OUTSIDE the app
  ┌────────────────── planeter process(es) ─── artifacts) ───────────────────────────────────────────┐
  │  AUTH (accounts, OAuth/OIDC, tokens, per-repo/per-ref authorization)   ── authority prikk lacks    │
  │  TX (clone/fetch/push = ferry prikk bundle/sync over TLS)   WEB (browse/review/issues/admin)       │
  │  API (REST + OpenAPI)   HOOK (signed webhooks)   REG (OCI + language)   CI (orchestrate runners)   │
  │  ───────────────────────────────────────────────────────────────────────────────────────────────│
  │  FORGE METADATA store: accounts, orgs, teams, permissions, issues, reviews, CI, webhook config     │
  │       (forge-owned; keyed to prikk object ids; NOT authoritative for history — INT-4/NG-6)         │
  │  ───────────────────────────────────────────────────────────────────────────────────────────────│
  │  PK: drives the prikk CLI as a SUBPROCESS (`--format json` reads + bundle/sync exchange)           │
  └───────────────────────────────────────┬───────────────────────────────────────────────────────────┘
                                           ▼  (subprocess, on-disk)
                        prikk CLI  ──▶  one `.prikk` repository per hosted repo
                        (source of truth for history, refs, signing, verify, merge/seal — BN-2)
```

- **BD-01 — Inside planeter:** the network transport, the multi-repo namespace and identity, accounts
  and authorization, issues, review, CI orchestration, packages, web, API, webhooks — and the forge
  metadata store that holds forge-owned data keyed to prikk object ids (BN-1). planeter holds authority;
  prikk does not.
- **BD-02 — Outside planeter:** the humans and their browsers; the developer's own **prikk client and
  signing keys** (AC-02; the default under BN-5/OQ-1 is that keys never enter planeter); CI runners
  (separate hosts, OPS-4); package managers; the operating system; the **forge operator** who deploys
  and backs up planeter (AC-05); and **prikk itself**, which owns the repository (BN-2).
- **BD-03 — The prikk boundary is a process boundary, not a link boundary** (INT-1, BN-3): planeter
  invokes the prikk CLI as a subprocess and consumes its `--format json` output and its `bundle`/`sync`
  artifacts. planeter links no prikk crate; a consumer can confirm planeter runs against a prikk binary,
  not a prikk library. This is what keeps prikk's unstable internals out of the forge (PU-3).
- **BD-04 — prikk is the source of truth; the metadata store is not** (INT-4, NG-6): every repository
  fact planeter shows is re-derivable by re-invoking prikk over the hosted `.prikk`. The metadata store
  caches for speed and holds *forge-owned* data (issues, permissions), never authoritative history.
- **BD-05 — planeter moves bytes; prikk makes them** (INT-2): the network transport ferries prikk's
  artifact exchange over TLS. planeter defines the HTTP/SSH envelope, the authorization, and the
  repository addressing; the repository protocol inside the envelope stays prikk's (NG-2).
- **BD-06 — The authority/verification line is externally observable** (SEC-1): a forge compromise
  cannot manufacture prikk-verified history, because verification is prikk's and re-runnable by anyone
  holding the repo. "Approved on planeter" and "verified by prikk" are separately checkable.

### 1.2 Actors (AC-…)

| ID | Actor | Does | Touches |
|---|---|---|---|
| AC-01 | **Anonymous reader** | browses public repositories, reads issues | WEB (read), API (public) |
| AC-02 | **Developer** | clones, pushes, opens changes, files issues, comments — holds their own prikk + keys | TX, WEB, API, AUTH |
| AC-03 | **Reviewer / maintainer** | reviews changes, approves, merges/seals | WEB (review), TX, and the merge gate (GATED-1) |
| AC-04 | **Org / repo admin** | manages orgs, teams, permissions, repo settings, imports | WEB (admin), API, AUTH |
| AC-05 | **Forge operator** | deploys, configures, backs up, scales planeter — **outside the app** | OPS surfaces; not a login role |
| AC-06 | **CI runner** | executes pipeline jobs on a separate host | CI, TX (pulls the change) |
| AC-07 | **Integration / bot** | automates via API + token; consumes webhooks | API, HOOK, AUTH (scoped tokens) |

AC-05 is listed to mark the boundary (OPS-, forge-commons *Deployment*): the operator runs planeter but
is not an in-app authority; AC-02's own prikk holds the keys (BN-5).

---

## 2. External interfaces

### 2.1 Transport surface (TX-…) — CAP-2 / INT-2

The familiar clone-and-push experience, implemented as **prikk artifact exchange ferried over the
network**. No prikk wire protocol is invented (NG-2).

- **TX-01 — Fetch / clone over HTTPS.** For the refs a client requests, planeter (authorizing per
  AUTH-03) runs prikk to produce the `bundle`/`sync` artifacts and streams them over TLS; the client
  imports them into its local prikk. Public repositories may be fetched anonymously; private require
  authentication.
- **TX-02 — Push over HTTPS.** The client produces artifacts (`sync build` against the server's
  advertised `have`) and uploads them; planeter authorizes the target ref (AUTH-03), validates with
  prikk (`sync accept` + `verify`), and applies — or rejects with a named reason. **The signature that
  makes the applied ref prikk-verifiable is the subject of the trust gate (GATED-1 / OQ-1);** the
  default designed here is that it is the *pushing user's own* prikk signature, and planeter stores and
  verifies but does not sign.
- **TX-03 — SSH transport.** The same negotiation over SSH, authenticated by the user's registered
  public key (AUTH-02) — the transport developers expect for automated pushes.
- **TX-04 — The negotiation is prikk's, ferried.** planeter's contribution is the envelope: TLS
  (STD-1), authentication and per-ref authorization (AUTH), repository addressing (owner/name → the
  `.prikk` on disk), and the `have`/`build`/`accept` round trip carried over the wire. prikk's
  artifacts (`PSYNCSU1`, `PSYNCHV1`, `PEXCH002`, bundle) travel opaque inside it.
- **TX-05 — Client integration is explicit, not magic.** Because prikk has no `clone`/`push` verbs, the
  client side needs a thin helper (a `planeter` client or a prikk remote-helper) that turns "push to
  URL" into the artifact round trip. Its exact shape is a design detail flagged for the internal design;
  the *contract* is TX-01…TX-04.
- **TX-06 — Everything is encrypted in transit.** prikk artifacts are unencrypted by design ("move only
  over a channel you trust"); planeter is that channel (STD-1). No plaintext transport, ever.

### 2.2 Web surface (WEB-…) — CAP-8 / STD-6

Read views **re-derive from prikk** (INT-4), rendered safely (STD-6), and keep prikk's honesty visible.

- **WEB-01 — Repository browser.** History (from `log --format json`), a change's contents and its
  effect (`show`, `checkout --patch-plan --content-path` content reports), files at a ref, branches and
  tags, and the repository's `verify` status — all re-derived from prikk on demand.
- **WEB-02 — Change review.** A proposed change rendered as prikk's own view of what it does (`show` /
  patch reports as the "diff"), with inline comments, approvals, required checks, and the merge control
  (WEB gates on GATED-1 for the actual seal).
- **WEB-03 — Issues and boards.** Forge-owned issue tracking (CAP-4), cross-referencing changes and
  prikk objects by prikk id.
- **WEB-04 — Administration.** Orgs, teams, per-repo/per-ref permissions, repository settings, imports.
- **WEB-05 — Safe rendering.** All Markdown/HTML sanitized; **raw repository content served from an
  isolated origin**; strict CSP; secure cookies + CSRF (STD-6).
- **WEB-06 — Honesty is on the page.** A merged change shows *both* "approved by … on planeter" and
  "sealed & prikk-verified" as **distinct** markers; an unsigned or unverifiable object is shown as
  such and never dressed up as verified (SEC-1, NG-3) — the forge-side analogue of stikk's "where prikk
  refuses, stikk explains."

### 2.3 REST API surface (API-…) — CAP-9 / STD-3

- **API-01 — REST over HTTPS, described in OpenAPI**, covering repositories, refs, changes/reviews,
  issues, CI, packages, orgs/teams and webhooks.
- **API-02 — Web conventions**: `Link`-header pagination with bounded defaults, rate-limit headers with
  `429`/`Retry-After`, `ETag` conditional requests, one consistent error shape, and **per-endpoint scope
  enforcement** on tokens (STD-3).
- **API-03 — Not a second source of truth.** Read endpoints re-derive repository facts from prikk
  (INT-4); the API exposes forge-owned metadata and prikk-derived reads, never an authoritative history
  planeter alone holds.
- **API-04 — Compatibility decided, not drifted.** Whether the API mirrors a GitHub-shaped surface for
  tooling reuse is a deliberate design choice (forge-commons *HTTP API*), recorded for the internal
  design, not stumbled into half-way.

### 2.4 Identity & authorization surface (AUTH-…) — CAP-3 / STD-2 / SEC

- **AUTH-01 — Human sign-in** via **OAuth 2.0 (code + PKCE) / OIDC**, with optional SAML and LDAP/AD;
  any local accounts use Argon2id hashing and offer **TOTP and WebAuthn** second factors (STD-2).
- **AUTH-02 — Machine credentials**: registered **SSH public keys** and **scoped, hashed access
  tokens** (and OAuth tokens) — the credentials TX and API authenticate against.
- **AUTH-03 — Authorization**: organizations, teams, and **per-repository and per-ref** roles (read /
  push / merge-seal / admin), least-privilege, with **protected refs** (review + checks required, no
  force-replace). This is the authority prikk lacks (§0a).
- **AUTH-04 — The mapping onto prikk** (SEC-1): an authorized push/merge produces prikk objects that
  prikk's `verify` accepts on their own merits. **Where the maintainer signature comes from is GATED-1
  (OQ-1);** the interim design holds *no* signing keys in planeter (client-side signing), the safer
  default (BN-5).
- **AUTH-05 — Audit log** of security-relevant events (sign-in, push, permission and admin changes),
  shipped off-box (SEC-5, OPS-5).

### 2.5 CI / runner surface (CI-…) — CAP-6

- **CI-01 — Pipelines** defined in-repository, triggered on push, on a proposed change, and on schedule.
- **CI-02 — Runners** are **separate-host, containerized, ephemeral**, registering to planeter over HTTPS
  with a scoped token (OPS-4).
- **CI-03 — Least privilege**: **secrets are withheld from untrusted (fork) changes**, and a job's token
  is short-lived and scoped to that job — and is **never a prikk signing key** (a CI job does not seal
  history on a maintainer's behalf).
- **CI-04 — Integrity**: CI may produce artifacts and (incrementally) provenance; planeter **stores and
  verifies** signatures/provenance as custodian (SEC-4), not as key-holder.

### 2.6 Registry surface (REG-…) — CAP-7 / STD-5

- **REG-01 — OCI Distribution** for containers (and OCI artifacts as the substrate for further types);
  language registries (npm, Maven, PyPI, …) added by demonstrated demand, each a faithful implementation.
- **REG-02 — Rules**: authenticated, **scoped publishing**; **immutable published versions** (yank,
  never mutate); digest-addressed **object storage** for the bytes (STD-5).

### 2.7 Webhook surface (HOOK-…) — CAP-9 / STD-4

- **HOOK-01 — A documented event taxonomy** delivered with per-delivery **event-type**, **delivery-id**,
  and **HMAC-SHA256 signature** headers; **SSRF egress controls** blocking private/loopback targets by
  default; **at-least-once** delivery with bounded retries and a **visible delivery log** (STD-4).

### 2.8 prikk-integration surface (PK-…) — INT / BN

- **PK-01 — planeter drives the prikk CLI per hosted repo.** Reads via `--format json`
  (`log`, `show`, `status`, `branch`, `tag`, `verify`, `worktree-status`, `checkout --patch-plan`,
  `bundle preview`, `trust`/`key status`); exchange via `bundle` and `sync`
  (`summary`/`have`/`build`/`accept`/`seal`). One `.prikk` per hosted repository (INT-3).
- **PK-02 — Process boundary, not link boundary** (INT-1, BD-03): planeter links no prikk crate; if a
  view needs data only in an unstable crate API, that is a prikk read-surface gap (UD-1), surfaced
  honestly (PK-06), not reached for.
- **PK-03 — prikk is the source of truth** (INT-4): the forge metadata store keys its records to prikk
  object ids and re-derives history/verify from prikk; it is never the authority for what the history
  *is*.
- **PK-04 — The seal/sign step is gated** (GATED-1, OQ-1): PK-01's `sync seal` / merge step is designed
  only up to the point where a signature is required; who signs and where the key lives is the owner's
  ruling.
- **PK-05 — Reuse of stikk's CLI layer is a marked choice** (INT-6, OQ-7): the PK surface is specified
  abstractly so it can be satisfied either by an independent prikk-CLI driver or by building on stikk's
  `stikk-prikk` operation layer — a dependency-direction decision (GATED-6).
- **PK-06 — Read-surface gaps are shown, not faked** (INT-5, UD-1): a view needing data prikk does not
  yet expose in machine-readable form (raw blob/blame) is marked *pending a prikk increment*, never
  fabricated from a workaround into prikk's internals.

---

## 3. User interaction flows (FL-…)

Numbered user-action → system-response, each citing the requirement it realizes.

- **FL-01 — Browse a public repository (AC-01).** 1. `GET` the repo's page. 2. planeter re-derives
  history and contents from prikk (`log`/`show` JSON) over the on-disk `.prikk` (PK-01/INT-4). 3. It
  renders them sanitized, from the isolated content origin (WEB-05), showing prikk's `verify` status
  honestly (WEB-06). No authority is exercised.
- **FL-02 — Clone (AC-02).** 1. Client requests the repo over HTTPS/SSH. 2. planeter authorizes
  (public → anonymous; private → AUTH-02). 3. planeter runs prikk to produce the fetch artifacts and
  streams them over TLS (TX-01/TX-06). 4. The client imports them into a local prikk. The developer
  typed a familiar clone command; the artifact exchange was invisible.
- **FL-03 — Push (AC-02).** 1. The client `sync build`s a delta against planeter's advertised `have` and
  uploads it (TX-02). 2. planeter authorizes the **target ref** (AUTH-03; protected refs may refuse). 3.
  planeter validates with prikk (`sync accept` + `verify`) and applies. 4. The applied ref is
  prikk-verifiable by the **pushing user's own signature** (interim default, BN-5); planeter stores and
  verifies, holding no key. 5. **If the owner rules server-side signing (OQ-1), the alternative is
  GATED-1** — not designed here. 6. On refusal, the reason is named (unauthorized ref, failed verify),
  never a silent drop.
- **FL-04 — Propose, review, and merge a change (AC-02 → AC-03).** 1. A developer pushes a branch or
  opens a change. 2. planeter opens a review (WEB-02), rendering prikk's `show`/patch as the diff. 3.
  Reviewers comment and approve; required CI checks (FL-06) must pass. 4. A maintainer merges: planeter
  runs prikk's `merge`/`seal`; the result lands as prikk-verifiable history. 5. The UI shows **"approved
  by … on planeter"** and **"sealed & prikk-verified"** as distinct facts (WEB-06, SEC-1). 6. **The
  signature on the seal is GATED-1** (OQ-1); until ruled, the merge control stops at the gate and states
  that the seal is performed by a maintainer's own prikk, not by the forge.
- **FL-05 — File and cross-reference an issue (AC-02).** 1. Open an issue (forge-owned, CAP-4). 2.
  Mentioning a change or a prikk object links it by prikk id (PK-03). 3. Nothing here touches prikk's
  authority; it is forge metadata (NG-6).
- **FL-06 — Run CI on a change (AC-06).** 1. A push/change triggers a pipeline. 2. A **separate-host,
  ephemeral runner** pulls the change via TX and runs it (CI-02). 3. **Secrets are withheld if the
  change is from an untrusted fork** (CI-03); the job token is scoped and is not a signing key. 4.
  Status reports back and gates the merge (FL-04).
- **FL-07 — Publish and pull a package (AC-02 / package manager).** 1. `docker push` / `npm publish` /
  … against planeter's registry (REG-01) with a scoped token. 2. The published version is **immutable**
  and object-storage-backed (REG-02). 3. Pulls follow the ecosystem's own protocol unchanged.
- **FL-08 — Read honesty in any history view (AC-01/AC-03).** 1. Any change or ref view distinguishes
  **prikk-verified** from **forge-approved** (WEB-06). 2. An unsigned, unverifiable, or forge-approved-
  but-not-yet-sealed object reads as exactly that — the two claims are never conflated (SEC-1, NG-3).
- **FL-09 — Import a Git repository (AC-04).** 1. Admin points an import at a Git (or hg/SVN/CVS) source.
  2. **brygge** decodes it to prikk objects (its fidelity report travels with the import). 3. planeter
  hosts the resulting prikk repository. 4. planeter never becomes a Git *server* (NG-1); Git mirror-out,
  if any, is GATED-5 (OQ-5).
- **FL-10 — Operator backs up and restores (AC-05).** 1. A coherent, time-aligned snapshot of the
  **metadata store + the hosted prikk repositories + config/secrets** is taken (OPS-3). 2. A restore is
  **rehearsed** into a scratch environment and verified — repos clone, history verifies, issues return.
  3. The prikk repos are treated as irreplaceable content, not a cache.

---

## 4. Data contracts (external) (CT-…)

- **CT-01 — Inputs planeter accepts:** authenticated HTTPS/SSH requests; **uploaded prikk artifacts**
  (`bundle`/`sync`) on push; OAuth/OIDC and token credentials; API calls; webhook configurations;
  package publishes; import sources (for brygge). No user-supplied URL is fetched without SSRF controls
  (STD-4/STD-6).
- **CT-02 — Outputs planeter produces:** rendered web pages (from the isolated content origin);
  OpenAPI-described **API JSON**; **prikk artifacts** streamed for fetch/clone; **HMAC-signed** webhook
  deliveries; package pulls in each ecosystem's protocol; and **export archives** (CT-05).
- **CT-03 — The forge metadata store** (BD-04): forge-owned data — accounts, orgs, teams, per-repo/ref
  permissions, issues, reviews, CI config/results, webhook config — keyed to prikk object ids. **Not**
  authoritative for repository history (INT-4/NG-6); re-derivable facts are cache, not truth.
- **CT-04 — The prikk-integration contract** (BN-3): planeter consumes prikk's `--format json` read
  surface (versioned by prikk's CLI) and its artifact formats. planeter treats these as the interface
  and pins the prikk **format versions** it supports (OP-04, UD-3). It links no prikk crate (BD-03).
- **CT-05 — Export, so a project can leave** (NG-7): the forge-owned data in a portable form (F3-shaped
  where applicable — forge-commons *F3*) **plus** unconditional access to the underlying prikk
  repositories. Leaving planeter costs a project neither its history nor its issues.

## 5. Operational behaviours (OP-…)

- **OP-01 — Writes per repository are serialized by planeter** (UD-4): because prikk multi-user
  concurrency is undesigned, planeter owns the concurrency control above prikk's local locks, so two
  pushes to one repo never race prikk. The sufficiency of prikk's local locking beneath that
  serialization is confirmed, not assumed (UD-4).
- **OP-02 — The authority/verification distinction is observable everywhere** (SEC-1): API and UI both
  expose "approved" and "verified" as separate fields; no surface collapses them.
- **OP-03 — Large transfers stream and are bounded**: fetch/push artifacts stream over TLS with bounded
  buffering; a slow client cannot exhaust the server; a long transfer reports progress.
- **OP-04 — Hosted-format tracking** (UD-3, OQ-6): planeter records each repo's prikk **format version**
  and, on a prikk format change, **refuses to host an unsupported version rather than risk it**, pending
  the owner's durability policy (GATED-3). It never silently hosts a repo it cannot re-verify.
- **OP-05 — Gaps are shown, not faked** (INT-5/PK-06): a view depending on a prikk read-surface gap
  (UD-1) is marked *pending a prikk increment*; planeter does not fabricate the missing datum.
- **OP-06 — Degrade safely**: if the prikk subprocess is unavailable for a repo, reads serve last-known
  cache **labelled stale** and writes refuse; planeter never invents history to stay "up".

## 6. Traceability (design → requirements)

| Design area | Realizes |
|---|---|
| BD-01…06 · AC-01…07 | BN-1…5, INT-1/4, SEC-1, PU-2 |
| TX-01…06 | CAP-2, INT-2, STD-1, NG-2 |
| WEB-01…06 | CAP-8, STD-6, INT-4, SEC-1/NG-3 |
| API-01…04 | CAP-9, STD-3, INT-4 |
| AUTH-01…05 | CAP-3, STD-2, SEC-1/3/5 |
| CI-01…04 | CAP-6, SEC-4, OPS-4 |
| REG-01…02 | CAP-7, STD-5 |
| HOOK-01 | CAP-9, STD-4 |
| PK-01…06 | INT-1…6, BN-2/3, UD-1 |
| FL-01…10 | CAP-*, SEC-1, NG-1/3, OPS-3 |
| CT-01…05 | STD-*, INT-4, NG-6/7, UD-3 |
| OP-01…06 | UD-3/4, SEC-1, INT-5 |

## 7. Design-set entry point (for the owner)

Two planeter documents exist, in the house manner:

1. `planeter-01-requirements-spec-v0.1.md` — what it must do / never do / decide (the contract).
2. `planeter-02-external-design-v0.1.md` — this document: the black-box surface for what can be designed
   now.

The load-bearing decisions this document commits: the **prikk boundary is the CLI-JSON read surface plus
artifact exchange, driven as a subprocess** (BD-03/PK-*), so prikk's internals never enter the forge and
prikk stays lean (PU-2/PU-3); **transport is ferried prikk artifact exchange** (TX-*), so no prikk wire
protocol is invented (NG-2); **planeter holds authority but not prikk's verification** (SEC-1/BD-06/
WEB-06), so a forge compromise cannot forge prikk-verified history; and **the write/merge trust model is
left at a gate** (§8), because where signing keys live is the owner's to rule, not the design's to
presume.

## 8. What cannot be externally designed yet — GATED surfaces (GATED-…)

The requirements name owner questions (OQ-1…OQ-7) and prikk-side dependencies (UD-1…UD-4). This document
designs up to each and **stops**, rather than presuming an answer. Each gate names what it blocks and
what planeter does in the meantime.

| ID | Gated surface | Blocked on | planeter's interim behaviour |
|---|---|---|---|
| **GATED-1** | The **signing model** for a push/merge/seal — who signs, and whether planeter holds a key | **OQ-1** / SEC-2 / UD-2 | Design assumes **client-side signing only**: the user's own prikk signs; planeter stores and verifies, holds **no** key (BN-5). The server-side "merge-button" signing surface is not designed until ruled |
| **GATED-2** | Anchoring **ref authorization** to a prikk-side concept | **OQ-2** / UD-2 | Authorization is **purely planeter's**, above prikk (AUTH-03); no prikk-side ref-authority is assumed to exist |
| **GATED-3** | The **hosted-format durability policy** across prikk format changes | **OQ-6** / UD-3 | planeter **pins supported prikk format versions** and refuses to host an unsupported one (OP-04); the migration/version-pinning promise awaits the owner |
| **GATED-4** | The **v1 feature ceiling** beyond the CAP-1…10 core (wikis, discussions, richer boards, insights) | **OQ-4** | The core is designed; the edges are left as clearly-marked extensions, not built into v1 by assumption |
| **GATED-5** | **Git mirror-out** and/or live Git import scope | **OQ-5** | History **import via brygge** is designed (FL-09); Git *mirror-out* is left unbuilt; planeter is never a Git server (NG-1) |
| **GATED-6** | Whether PK builds on **stikk's `stikk-prikk`** layer or an independent driver | **OQ-7** / INT-6 | The PK surface is specified abstractly so either satisfies it; the dependency-direction choice is deferred |
| **GATED-7** | Depth of **browse/review views** that need prikk read-surface data not yet exposed (raw blob, blame/annotate) | **UD-1** | Views are built on what prikk exposes today; deeper views are marked *pending a prikk increment* (PK-06/OP-05), not faked |

**Sequencing consequence.** What can be built **now**, with no prikk change and no owner ruling, is the
whole read-and-host spine: multi-repo hosting and identity (CAP-1), fetch/clone transport (TX-01),
the web browse surface (WEB-01), accounts and authorization (AUTH), issues (WEB-03), the API (API), and
the read half of review (WEB-02) — all over prikk's CLI-JSON surface and artifact exchange. What waits
on **GATED-1** (the owner's OQ-1) is the **write completion**: turning an authorized push/merge into a
*signed, sealed, prikk-verified* result. This document deliberately stops at that gate rather than guess
where the signing key lives — a confident wrong trust model would be worse than a stated gate (the
requirements' own rule, and brygge's before it).

*End of External Design v0.1. Next design work should start where no gate blocks it: the multi-repo
hosting model and identity layer (CAP-1) and the CLI-JSON read/browse spine (WEB-01, PK-01), then the
transport ferry (TX-01/TX-02 fetch and the push *validation* path), leaving the **seal signature**
(GATED-1) and the **durability policy** (GATED-3) for the owner's rulings on OQ-1 and OQ-6. The internal
design and a threat model (`planeter-03-threat-model`, expected next per the house set) follow — a forge
that holds authority and accepts untrusted input over a network makes the threat model a first-class
deliverable, as it is for brygge.*
