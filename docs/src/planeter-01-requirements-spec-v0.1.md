# planeter — Requirements Specification

| | |
|---|---|
| Document | planeter Requirements (what the forge must do, must never do, and must decide) |
| Version | v0.1 (draft for review) |
| Date | 2026-09-15 |
| Basis | **forge-commons** (the standards / proposals / guidelines commons, `kos-commons/forge-commons`) as the conformance frame; **prikk reality** per the 2026-09-15 survey (prikk `HEAD f6cbd057`); **RFC 145** (serving a repository for reading — Shape D over the CLI); **RFC 116** (sync is negotiation-as-artifacts, off the network); prikk `ROADMAP.md` owner hosting direction (2026-09-06); project rules in `.git-exclude/rules/` |
| Not | a design, an API, a schema, or code. Where a decision belongs to a human, it is named in §9/§10 and left there. |
| ID scheme | `PU-` purpose · `NG-` non-goal · `CAP-` forge capability · `STD-` standards conformance · `SEC-` security & trust · `INT-` prikk integration · `OPS-` operational · `DEF-` deferred frontier · `UD-` prikk-side dependency · `OQ-` open question |

**planeter** (Norwegian: *planets* — the bodies that orbit a common star) is the **forge**: the web
service that hosts [prikk](https://github.com/prikk-vcs/prikk) repositories and the collaboration
around them — many repositories, many people, over the network, with accounts, review, automation and
packages. It is to prikk what Forgejo is to Git: prikk is the local, offline version-control system;
planeter is the hosting layer above it.

The governing sentence, and the one that resolves the apparent contradiction between "prikk refuses
hosting" and "build a familiar forge": **planeter carries the hosting weight prikk refuses, so prikk
stays a lean, offline, five-dependency CLI — and it builds that hosting *on prikk's stable read
surface*, holding all the authority prikk never had.** This is the exact parallel of brygge, which
carries the *dependency* weight prikk refuses; planeter carries the *hosting* weight. Neither makes
prikk larger.

---

## 0. What a reader must hold before the requirements make sense

**(a) prikk gives per-repository primitives, not a forge.** The 2026-09-15 survey is unambiguous: prikk
is a single-repository, offline, file-based CLI, and everything a forge adds is **absent by settled
design, not by omission.** planeter must build all of it. The split:

| prikk provides (planeter consumes) | prikk does NOT provide (planeter builds) |
|---|---|
| A machine-readable read surface — `--format json` on `log`, `show`, `status`, `branch`, `tag`, `verify`, `worktree-status`, `checkout --patch-plan`, `bundle preview`, `trust`/`key status` | **Network transport** — zero network code; no `clone`/`fetch`/`push`/`serve` (RFC 115/116: "prikk stays off the network") |
| Cryptographic **attribution & integrity** — Ed25519 author + maintainer signatures, TOFU maintainer trust, offline `verify` | **Multi-repository hosting** — one `.prikk` dir per directory; repositories are **anonymous by settled design** (no name, owner, or URL — RFC 145 §7) |
| **Artifact exchange** — `bundle` and `sync` (summary → have → build → accept), bytes in / bytes out | **Accounts, ACLs, ref authorization** — trust is *object* trust; "adopting a key never lets it move a ref"; no accounts (`ROADMAP.md` hosting) |
| The `merge`, `seal`, `rollback`, `checkout` operations over one repo | **Multi-user concurrency** — undesigned (RFC 108 §D5); only local lock files |

**(b) planeter is the owner's intended home for hosting — now scoped to a *familiar* forge.** prikk's
`ROADMAP.md` (2026-09-06) rules that *full forge hosting is not prikk's to build*, on five structural
grounds (the four absences above plus the explicitly-unstable pre-1.0 format), and names the wanted
shape as *"either a prikk subcommand like git's `git instaweb` or another project of a small hosting
server."* RFC 145 resolved that to **Shape D — ship plumbing, not a server; the ecosystem builds views
over the CLI.** planeter *is* that separate project. The owner has, in scoping this document, **expanded
its ambition from "a small hosting server" to a real, familiar forge** — and that expansion does not
contradict prikk's lean position: it *depends* on it. prikk stays small precisely because planeter, not
prikk, carries accounts, network and multi-repo. A read-only browse view would serve almost no one; a
forge people can push to, review in, and automate is the product (PU-1).

**(c) The integration boundary is the prikk CLI, not the prikk crate.** RFC 145 §8d is explicit that
prikk's *crate* API is "an explicitly unstable compatibility surface" and **not** the sanctioned
substrate — the CLI's `--format json` read surface is. The existing precedent is **stikk**, a front-end
that "owns no repository authority and no secrets," re-derives every fact from prikk, and drives the
CLI. planeter follows the same integration pattern (INT-1) and diverges from stikk on exactly one axis:
**planeter holds authority** (accounts, authorization), because a forge must. Where stikk explains,
planeter *hosts*.

---

## 1. Purpose (PU-…)

- **PU-1 — Be a real, familiar forge for prikk.** Host many prikk repositories over the network and
  provide the recognizable collaboration loop a developer expects from GitHub/GitLab/Forgejo: clone and
  push, propose and review changes, track issues, run CI, publish packages, and manage teams and
  permissions, through a web UI and an API. "Familiar" is a requirement: a person who has used a forge
  should recognize planeter as one.
- **PU-2 — Carry the hosting weight so prikk stays lean.** planeter exists so that prikk need not grow
  a network stack, an account system, a ref-authorization model, or multi-writer concurrency. Every
  such capability lives in planeter and **nothing planeter needs may push weight back down into prikk's
  five-dependency, offline core** (the boundary is BN-, §8; the parallel is brygge's PU-5).
- **PU-3 — Build on prikk's stable read surface, not its unstable internals.** planeter consumes prikk
  through the CLI's machine-readable surface (RFC 145 Shape D) and prikk's artifact exchange, so that
  prikk's internal churn does not break the forge and the forge never depends on an unstable crate API
  (INT-1). prikk remains the single source of truth for history and verification (INT-4).
- **PU-4 — Hold authority safely, and map it onto prikk's cryptography without weakening it.** A forge
  must decide who may read, push, and merge — authority prikk deliberately lacks. planeter owns that
  decision, but every write it authorizes must still land as prikk objects that prikk's own offline
  `verify` accepts on their own merits. planeter adds access control **above** prikk's integrity model;
  it never forges, bypasses, or weakens a prikk signature (SEC-1).
- **PU-5 — Be clean, safe, secure and robust before it is rich.** The forge-commons stance binds
  planeter's *own* design: lean hard on standards (STD-), keep each feature's trust surface small
  (SEC-), defer the genuine frontier (DEF-), and prefer refusing a capability to shipping it half-safe.
  A broad scope is not licence for a sprawling one.

## 2. Non-goals (NG-…) — stated as firmly as the goals

- **NG-1 — Not a Git server.** planeter serves *prikk* repositories. It offers Git only by **import and
  mirror** (via brygge for history-in, and read-only mirror-out where scoped), never by pretending to
  be a Git host. (forge-commons, *Git Transport* — the VCS-agnostic principle.)
- **NG-2 — Not a change to prikk.** planeter does not add commands, block kinds, or a network protocol
  *to prikk*. Where the forge needs something prikk does not expose, that is a prikk-side dependency to
  name (UD-), not a prikk patch planeter writes. Inventing prikk's wire protocol is specifically
  refused (INT-2; RFC 116 deferred it deliberately).
- **NG-3 — Not a holder of authority it cannot map to prikk.** planeter must never present history as
  verified, authored, or merged in a way prikk's own `verify` would not confirm. Forge-level "approved"
  is not prikk-level "verified"; the two must be distinguishable to any reader (SEC-1, and the parallel
  of brygge's NG-3).
- **NG-4 — Not the frontier, by default.** Federation, portable/nomadic identity, and built-in AI are
  **not** v1 (DEF-). A familiar forge does not require them, and forge-commons rates each *pilot* or
  *defer*. Their absence in v1 is a scoping decision, not a gap.
- **NG-5 — Not a reinvention of standards.** Every capability planeter can get from an established
  standard (transport security, OAuth/OIDC, OpenAPI, OCI, HMAC-signed webhooks) it takes from that
  standard and implements no more of it than interoperability requires (STD-, forge-commons Part I).
- **NG-6 — Not a second source of truth.** planeter does not become the authoritative store of
  repository history that prikk cannot re-derive. Forge metadata (issues, reviews, permissions) is
  planeter's; repository *history and its verification* are prikk's, re-derivable from the hosted repo
  (INT-4, NG-3).
- **NG-7 — Not a lock-in.** Following forge-commons, planeter must offer credible export of the forge
  data it owns and unconditional access to the underlying prikk repositories; a project can always
  leave with its history and its issues (OPS-, and forge-commons *F3*).

## 3. The forge capabilities v1 must deliver (CAP-…)

These are the familiar-forge core. Each is a requirement of v1, not a deferral; sequencing is the
roadmap's job, and the prikk-side gaps some of them lean on are named in §9 (UD-).

- **CAP-1 — Repository hosting and identity.** Host many prikk repositories, each addressed by a
  forge-level identity planeter invents *above* prikk (owner/organization + repository name → a URL),
  since prikk repositories are anonymous by design (§0a). Create, fork, mirror, rename, transfer,
  archive and delete repositories; per-repository settings and visibility (public / internal /
  private).
- **CAP-2 — Network transport (clone / fetch / push).** Give developers the familiar clone-and-push
  experience over **HTTPS (and SSH)**, implemented by **ferrying prikk's artifact exchange** over the
  network and running its `sync` negotiation server-side (INT-2). A fetch is the server producing the
  artifacts a client imports; a push is the client producing artifacts the server validates and applies
  (`sync build` client-side → transport → `sync accept`/`seal` server-side). No new prikk wire protocol
  is invented.
- **CAP-3 — Identity, organizations, teams, and authorization.** Accounts; organizations and teams;
  per-repository and **per-ref** authorization (who may read, who may push, who may merge/seal);
  role-based membership. This is the multi-user authority layer prikk lacks entirely (§0a), and its
  mapping to prikk's signature model is SEC-1 / OQ-1 / OQ-2.
- **CAP-4 — Issues and project tracking.** An issue tracker with labels, milestones, assignees,
  comments, and automatic cross-referencing between issues, changes and repository objects; basic
  boards. Forge-owned metadata (NG-6).
- **CAP-5 — Change proposal and review (the "pull request", in prikk's terms).** A proposed change — a
  set of prikk patches/blocks on a branch, or an incoming pushed artifact — that can be **discussed,
  reviewed line-by-line, and merged**. A merge is executed with prikk's own `merge`/`seal` and lands as
  history a maintainer signature makes verifiable (SEC-1). planeter provides the review *around* the
  change; prikk performs the merge *of* it.
- **CAP-6 — Continuous integration and automation.** Pipelines triggered on push, on a proposed change,
  and on a schedule, executed by **isolated, ephemeral runners on separate hosts** (forge-commons
  *CI runners*), with least-privilege job tokens and secrets withheld from untrusted (fork) changes.
- **CAP-7 — Package and artifact registry.** Host build outputs against the protocols their own tools
  speak — **OCI distribution** first, language registries by demonstrated demand — with authenticated,
  scoped publishing and immutable published versions (forge-commons *Registries*).
- **CAP-8 — Web UI.** Browse repositories, history, changes and their contents, issues and reviews; run
  administration. The read views **re-derive every repository fact from prikk** (INT-4), in the spirit
  of stikk's "explain prikk to humans" — served, multi-user, over the web.
- **CAP-9 — REST API and webhooks.** A programmable REST surface described in **OpenAPI**, and
  **HMAC-signed** webhooks with SSRF-safe delivery, covering the capabilities above (STD-3, STD-4).
- **CAP-10 — Search and notifications.** At least repository, code, and issue search, and user
  notifications for the events a collaborator must not miss (review requested, mentioned, CI failed).

## 4. Standards conformance (STD-…) — lean on the commons floor

planeter must speak the forge-commons **Common Standards** and implement no more of each than
interoperability requires (NG-5). Each STD- cites the forge-commons chapter it conforms to.

- **STD-1 — Transport security.** All transport (CAP-2) is encrypted and authenticated for writes;
  HTTPS with modern TLS and HSTS, SSH with public keys. prikk's artifacts are unencrypted by design
  ("move only over a channel you trust") — planeter *is* that trusted channel. (*Git Transport*, *Web
  Security*.)
- **STD-2 — Identity and authentication.** SSH public keys and scoped, hashed access tokens for
  machines; **OAuth 2.0 (authorization-code + PKCE) and OIDC** for federated human sign-in; **TOTP and
  WebAuthn** second factors; **SAML and LDAP/AD** as opt-in enterprise integrations. No home-grown auth.
  (*Identity and Authentication*.)
- **STD-3 — HTTP API.** REST over HTTPS, **described in OpenAPI**, with consistent pagination, rate-limit
  headers, `ETag` conditional requests, a single error shape, and per-endpoint scope enforcement.
  (*The HTTP API Surface*.)
- **STD-4 — Webhooks.** A documented event taxonomy delivered with **HMAC-SHA256 signatures**,
  **SSRF egress controls** on every target, at-least-once delivery with retries and a visible delivery
  log. (*Webhooks and Events*.)
- **STD-5 — Registries.** **OCI Distribution** for containers (and OCI artifacts as the substrate for
  new types); language registries as faithful implementations added by demand; authenticated scoped
  publishing; **immutable published versions**. (*Package and Artifact Registries*.)
- **STD-6 — Web security.** HTTPS-only + HSTS; a **strict CSP designed in from the start**; sanitize all
  rendered Markdown/HTML; **serve raw repository content from an isolated origin**; `Secure`+`HttpOnly`+
  `SameSite` cookies with CSRF protection for cookie-authenticated writes; rate-limited authentication;
  SSRF controls on every user-supplied URL. (*Transport Security and the Web*.)

## 5. Security and trust (SEC-…) — the authority planeter owns

planeter introduces something prikk never had — authority over other people's actions — so its trust
model is a first-class requirement, not a setting.

- **SEC-1 — Forge authorization sits *above* prikk integrity and never weakens it.** planeter decides
  who may push or merge; but every authorized write still results in prikk objects that prikk's offline
  `verify` accepts on their own cryptographic merits. "planeter approved this change" and "prikk
  verifies this history" are **distinct claims**, both true, never conflated (NG-3). A forge compromise
  must not be able to manufacture prikk-verified history.
- **SEC-2 — Be deliberate about where signing keys live. — RULED (OQ-1, 2026-09-15): option (a).**
  stikk's founding property is that prikk, not the front-end, holds signing key material; planeter
  inherits it. The maintainer signature on a merge/seal comes from the **pushing/merging user's own
  prikk (client-side)**; **planeter holds no history-signing key by default.** A server-side forge
  signing identity exists only as an explicit per-repository opt-in (the labeled exception, threat model
  RR-2). This keeps the highest-value secret out of the most-attacked component (threat model T-1).
- **SEC-3 — Small trust surface per feature, secure by default.** Every capability ships with the
  secure default (private-by-default where appropriate, MFA available, least-privilege tokens, branch
  protection); optional power (SAML, broad tokens, permissive CORS) is off until turned on. The forge
  is a high-value target; each added surface must justify itself.
- **SEC-4 — Verifier and custodian for supply-chain integrity.** planeter verifies and displays prikk
  signatures and, per forge-commons *Integrity*, stores and verifies artifact signatures and provenance
  — **without holding users' signing keys**. Generation is client/CI-side; the forge checks and keeps
  the record.
- **SEC-5 — Authorization is least-privilege and legible.** Per-repo/per-ref permissions, scoped and
  expiring tokens, protected refs (review + checks required, no force-replace), and an audit log of
  security-relevant events, shipped off-box. (forge-commons *Security Hardening*.)

## 6. prikk integration (INT-…) — the boundary that keeps prikk clean

- **INT-1 — Drive the prikk CLI `--format json` surface as a subprocess; never link the prikk crate.**
  RFC 145 §8d: the crate API is explicitly unstable and is not the substrate; the CLI is. planeter (or
  a shared layer, INT-6) invokes prikk as a subprocess over on-disk repositories and parses its
  machine-readable output. This also means planeter links none of prikk's internals and cannot be
  broken by prikk's crate churn (PU-3).
- **INT-2 — Transport by ferrying prikk's artifact exchange; do not invent a prikk wire protocol.**
  prikk's `sync` is negotiation-as-artifacts *by design* (RFC 116) so a transport layer can carry it.
  planeter's clone/fetch/push (CAP-2) move `bundle`/`sync` artifacts over HTTPS/SSH and run
  `sync summary`/`have`/`build`/`accept`/`seal` server-side. The network protocol is planeter's; the
  repository protocol stays prikk's (NG-2).
- **INT-3 — Each hosted repository is one local prikk repository; identity lives above it.** planeter
  stores each repo as a prikk `.prikk` working store and provides the owner/name/URL namespace prikk
  omits (CAP-1). A view must not push forge identity *into* prikk (RFC 145 §7); planeter's identity is a
  layer above, not a mutation of prikk.
- **INT-4 — Re-derive repository facts from prikk; prikk is the source of truth.** History, contents,
  refs, verification status are read from prikk on demand (cached for performance, never as the
  authority). planeter must be able to reconstruct any repository fact it shows by re-invoking prikk
  (the stikk property), so a corrupted forge cache can never become false history (NG-6).
- **INT-5 — Where prikk's read surface is insufficient, name it — do not fake it.** If a forge view
  needs data prikk's CLI does not yet expose in machine-readable form (raw blob access, a
  blame/annotate equivalent, `--format json` on a command that lacks it), that is a prikk-side
  dependency (§9 UD-), satisfied by an ordinary prikk increment — not by planeter reaching around the
  CLI into prikk's internals.
- **INT-6 — Prefer reusing the ecosystem's prikk-CLI layer over reimplementing it.** stikk already has
  a crate that drives the prikk CLI and parses its JSON (`stikk-prikk`). planeter should evaluate
  building on that shared operation layer rather than reparsing prikk's output independently — a
  reuse/decoupling decision recorded as **OQ-7**.

## 7. Operational requirements (OPS-…) — the guidelines, as obligations

The forge-commons **Useful Guidelines** bind planeter's operation. Stated here as requirements so they
are designed in, not bolted on.

- **OPS-1 — Deployable simply, scalable deliberately.** A Rust service (per ecosystem convention:
  Rust 2024, MSRV-pinned, `forbid(unsafe)` outside any FFI) that starts as a single node with a
  relational database and local storage, and scales — PostgreSQL, object storage for artifacts, then
  horizontal nodes — **only against measured need** (*Deployment*, *Observability*).
- **OPS-2 — Hardened by default.** Forced HTTPS, minimal exposed ports, admin behind a private network,
  MFA for privileged accounts, the forge process unprivileged and patched (*Security Hardening*).
- **OPS-3 — Durable and recoverable.** Back up **the database + the hosted prikk repositories + config
  and secrets as one coherent, time-aligned set**, encrypted, offsite, with a **rehearsed restore**.
  The hosted prikk repos are irreplaceable content and are backed up as such (*Backup*).
- **OPS-4 — CI runners isolated.** Runners are separate-host, containerized, ephemeral, with secrets
  withheld from untrusted changes and least-privilege job tokens (*CI Runners*, and CAP-6).
- **OPS-5 — Observable.** Metrics, structured off-box logs, and health endpoints from day one; alert on
  low disk, error spikes, failed backups, and certificate expiry (*Observability*).
- **OPS-6 — Open and exitable.** Apache-2.0 (the ecosystem's license for its tooling); a public,
  contributable process; and credible **export** of forge data plus unconditional access to the
  underlying prikk repositories, so a project can leave (NG-7, *Governance*, *F3*).

## 8. Boundaries — planeter's responsibility vs. prikk's (BN-…)

- **BN-1 — planeter owns: hosting, and everything above the single repository.** Network transport,
  multi-repo namespace and identity, accounts/orgs/teams, authorization, issues, review, CI, packages,
  web, API, notifications — and the honest mapping of its authority onto prikk's signatures (SEC-1).
- **BN-2 — prikk owns: the repository.** The object model, history, refs, signing, verification, merge
  and seal semantics, and the artifact formats. planeter drives these; it does not redefine them (NG-2).
- **BN-3 — The CLI JSON surface + artifact exchange is the interface.** planeter's dependency on prikk
  is the machine-readable read surface (INT-1) and the `bundle`/`sync` artifacts (INT-2); prikk's
  obligation to planeter is that these are complete and stable enough to build on (UD-1, UD-3).
- **BN-4 — planeter never pushes weight into prikk.** Nothing planeter needs may require prikk to grow a
  network stack, an account model, a ref-authorization model, or a dependency (PU-2). If a capability
  cannot be built without changing prikk's core posture, it is refused or deferred, and the reason is
  recorded (the parallel of brygge's BN-5).
- **BN-5 — planeter holds no history-signing key by default (OQ-1 ruled, option a).** The forge holds
  **none**; client-side signing is the design. A server-side forge signing identity is an explicit
  per-repository opt-in only (SEC-2), never ambient.

## 9. prikk-side dependencies planeter is waiting on (UD-…)

These are prikk's (or the owner's) to settle; naming them keeps no requirement above silently assuming
them. They gate specific capabilities, not the whole forge.

| ID | Dependency | prikk state today | What it gates |
|---|---|---|---|
| **UD-1** | Completeness of the CLI machine-readable **read surface** for browse (raw blob/content access; a blame/annotate equivalent; `--format json` on any command a view needs that lacks it) | Rich JSON surface exists (RFC 146 done; 147 next); blob access is `pub(crate)`, reached only via `show`/`checkout` content reports; some commands lack `--format json` | CAP-8 web views and CAP-5 review depth (INT-5) |
| **UD-2** | A **multi-writer push/accept model**: who signs the sealed ref when a push is authorized by the forge for a user | prikk has **no ref-authorization model**; a ref move needs *the operator's own* maintainer signature; threshold fixed at one | CAP-2 push, CAP-5 merge, SEC-1 (see OQ-1/OQ-2) |
| **UD-3** | **Format stability** for hosted repositories | prikk on-disk format is explicitly **unstable pre-1.0**; "internal to the CLI, may change without notice" | Hosting durability — the risk of holding "the largest repositories in the project's life against an unstable format" (parallel to brygge UD-5); planeter must track prikk format versions and migrate (OQ-6) |
| **UD-4** | **Concurrency guarantees** under planeter's serialization | prikk multi-user concurrency undesigned (RFC 108 §D5); only local lock files | CAP-3 correctness — planeter must serialize writes per repo and confirm prikk's local locking is sufficient beneath that serialization |
| **UD-5** | (noted, not blocking) A compile-time **read-only facet** of `prikk-store`, *if* planeter ever links the crate | Does not exist; `RefStore` carries `publish` write authority (RFC 145 §8c) | Nothing, while INT-1 holds (drive the CLI, do not link) — recorded so the temptation to link is a decision, not a slip |
| **UD-6** | A **client-sealable prepared-merge** affordance: a prikk path to hand a *server-prepared* merge plan to a client whose own prikk **seals it in one action** | prikk separates `merge-plan`/`merge-evidence` (read-only, no key) from `merge`/`seal` (needs the maintainer key); whether a prepared plan can be handed off and sealed as one client action is to be confirmed/extended | The **one-click UX** of the OQ-1(a) signing model (CAP-5): without it, client-side signing is a manual pull-merge-push rather than a merge button |

## 10. Open questions — the ones that are not planeter's to answer (OQ-…)

Per project rules, these belong to the **owner**. planeter names each, states what it changes
downstream, and stops.

- **OQ-1 — Where does the maintainer signature on a forge merge/seal come from? — RULED 2026-09-15:
  option (a), client-side.** The seal is produced by the pushing/merging user's **own prikk**; **planeter
  holds no history-signing key by default.** The usability cost is dissolved by preparing the merge
  server-side and having the maintainer's prikk seal the prepared plan in one action (UD-6). A
  **server-side forge maintainer identity (option b)** is permitted only as an **explicit, per-repository,
  key-isolated, forge-attributed, audited opt-in** — never the default. **Downstream (now settled):**
  planeter is not a keyholder by default (SEC-2, BN-5); a forge compromise cannot manufacture
  prikk-verified history (threat model T-1/INV-2); the write path (CAP-2 push, CAP-5 merge) is now
  designable, pending the UD-6 affordance and the ref-authority mapping (OQ-2).
- **OQ-2 — How does forge authorization map onto prikk's ref model?** prikk has no ref-authorization;
  planeter's per-ref permissions are *above* prikk. Does planeter's authorization fully *replace* a
  ref-authority concept prikk will never have, or does the owner want a prikk-side notion to anchor to
  (UD-2)? **Downstream:** whether "who may move this ref" is answerable by prikk at all, or is purely
  planeter's.
- **OQ-3 — Confirm repository identity is a *forge-level* concept.** prikk repos are anonymous by design
  (RFC 145 §7 forbids a *view* inventing prikk-level identity). planeter's owner/name/URL identity lives
  above prikk (INT-3), which appears consistent — confirm this is the intended locus, not a prikk change.
- **OQ-4 — The v1 feature ceiling.** CAP-1…CAP-10 are the familiar core. Are wikis, discussions,
  richer project boards, and repository insights **in** v1 or the first deferral tier? "Familiar" is
  agreed; its exact edge is the owner's.
- **OQ-5 — Git interoperability scope.** Beyond history *import* (brygge), does v1 offer read-only Git
  *mirror-out* of hosted prikk repos, and/or live Git import? **Downstream:** how much of the Git
  ecosystem planeter meets, versus staying prikk-native (NG-1 bounds the answer: never a Git *server*).
- **OQ-6 — Hosted-format-stability policy.** Given UD-3, what does planeter promise about hosted repos
  across prikk format changes — automatic migration, version pinning, a stability gate before hosting a
  given prikk version? A product-durability decision.
- **OQ-7 — Reuse stikk's prikk-CLI layer, or not?** Build CAP-8/INT-1 on stikk's `stikk-prikk`
  operation layer (shared maintenance, coupled release) or an independent driver (decoupled, duplicated)
  — INT-6. A dependency-direction decision with real coupling consequences across two projects.

---

## Traceability (coverage)

| Requirement area | Where |
|---|---|
| Purpose; hosting weight; build-on-CLI; authority mapping; clean-before-rich | §1 PU-1…PU-5 |
| Non-goals as firm as goals (not-a-Git-server, not-a-prikk-change, frontier-deferred, no-lock-in) | §2 NG-1…NG-7 |
| Familiar-forge capabilities (repos, transport, identity, issues, review, CI, packages, web, API, search) | §3 CAP-1…CAP-10 |
| Standards conformance to forge-commons Part I | §4 STD-1…STD-6 |
| The authority layer planeter owns; mapping onto prikk signatures without weakening them | §5 SEC-1…SEC-5 |
| prikk integration: CLI-JSON not crate; ferry artifacts; re-derive; name gaps; reuse stikk layer | §6 INT-1…INT-6 |
| Operational guidelines as obligations | §7 OPS-1…OPS-6 |
| Boundaries planeter vs prikk; never push weight down | §8 BN-1…BN-5 |
| Deferred frontier (federation, portable identity, AI) per forge-commons verdicts | §2 NG-4, and DEF via forge-commons Part II |
| prikk-side dependencies named, not assumed | §9 UD-1…UD-5 |
| Owner-only questions, downstream effect, then stop | §10 OQ-1…OQ-7 |
| Uncertainty stated as uncertainty (prikk absences, key-location, format instability) | §0, §9 UD, OQ-1/OQ-6 |

*End of planeter Requirements v0.1. This document is the contract a design must satisfy; it contains no
architecture, API, or schema. What is buildable on prikk **today** without a prikk change: the entire
hosting layer over the CLI-JSON read surface and the `bundle`/`sync` artifact exchange (CAP-1, CAP-2,
CAP-8, and the read half of CAP-5). The **write/merge trust model is now ruled** (OQ-1 →
option a: no forge-held signing key; client-side seal), so the write path is designable; what still
waits is the **client-sealable-plan affordance** (UD-6) for one-click UX, the **ref-authority mapping**
(OQ-2), and the **hosted-format durability policy** (OQ-6, UD-3). The external design
(`planeter-02-external-design-v0.1.md`) designs up to the remaining gates and stops at each, and the
threat model (`planeter-03-threat-model-v0.1.md`) both substantiates the OQ-1 ruling and covers the
modern/AI threat surface, in the house manner.*
