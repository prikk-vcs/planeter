# planeter — Threat Model

| | |
|---|---|
| Document | planeter Threat Model (security) |
| Version | v0.2 (2026-09-23 — implementation status through 0.1.x recorded under T-2/T-6/T-8 and RR-6/RR-7, per the release rule that a release touching a sensitive surface updates this document; v0.1 2026-09-15 was the review draft) |
| Date | 2026-09-23 (v0.2); 2026-09-15 (v0.1) |
| Basis | planeter Requirements v0.1 (PU/NG/CAP/STD/SEC/INT/OPS/BN/UD/OQ) and External Design v0.1 (BD/AC/TX/WEB/API/AUTH/CI/REG/HOOK/PK/FL/CT/OP/GATED); **forge-commons** (the standards frame); prikk reality (2026-09-15 survey, prikk `HEAD f6cbd057`); project rules (a threat model is a first-class release deliverable) |
| ID scheme | `A-` asset · `TB-` trust boundary · `T-` threat · `C-` control · `INV-` security invariant · `RR-` residual risk · `ASSUME-` assumption |
| Not | code, an API, or a dependency-audit report. It states what planeter must defend, against whom, and how — so the design and tests can be checked against it. |

**The essence, in one paragraph.** planeter is the opposite of stikk on the axis that matters most for
security: **it holds authority, faces the network, and serves many mutually-distrusting users.** It
accepts untrusted input from anonymous clients (web requests, pushed prikk artifacts, uploaded files),
it *renders* attacker-controlled content in a privileged web origin, it *runs* attacker-controlled code
in CI, and it *decides* who may read, push and merge. Its two signature dangers follow directly:
(1) **manufactured prikk-verified history** — the forge, or an attacker who owns the forge, producing
repository history that prikk's own `verify` would accept though no legitimate maintainer sealed it; and
(2) **the forge as the internet's highest-value target** — a single network-facing service that, if
breached, exposes every private repository, every credential, and (if it holds one) the key that signs
history. Everything below defends those two, plus the familiar web-service surface (auth, authz, XSS,
SSRF, CI RCE, DoS, supply chain) that a forge inherits by being a forge. The modern landscape adds one
genuinely **new** class — AI *in the loop* (prompt injection through attacker-controlled repository
text, AI-scaled contribution abuse, dependency hallucination), atop an *acceleration* of the classic
attacks — answered by keeping AI outside the trust boundary and treating all content as data, never
instructions (T-13). **The single most important design consequence — that planeter should hold no
history-signing key by default — is derived here (T-1, INV-2), and is this document's answer to the
requirements' OQ-1.**

---

## 1. Assets (A-…)

- **A-HISTORY — the integrity and prikk-verifiability of hosted repository history.** The crown jewel.
  Its value is that a client, offline, can `verify` what planeter served and get the truth. planeter
  must never be *able* to make false history that prikk accepts (SEC-1).
- **A-SIGNING-KEYS — any key that can seal prikk history.** The highest-value secret in the system,
  because it forges A-HISTORY directly. Under this model's recommendation planeter holds **none**
  (INV-2); that absence is itself a primary control.
- **A-AUTHZ — the authorization state** (who may read/push/merge, org/team membership). Corruption
  grants unauthorized access or writes.
- **A-CREDS — user credentials, sessions, tokens, and registered SSH public keys.** A forge is a
  credential honeypot; account takeover is the common path to everything else.
- **A-CONTENT — private repository content and private forge metadata.** Source (which may hold
  committed secrets), private issues, private reviews. Confidentiality.
- **A-HOST — the planeter server(s), database, object storage, and the CI runners.**
- **A-AVAILABILITY — the service itself.** A forge is infrastructure teams depend on; downtime is a real
  cost, and a familiar target for extortion.
- **A-SUPPLY — planeter's own dependency supply chain and the third-party CI-action ecosystem.**

## 2. Trust boundaries (TB-…)

| ID | Boundary | Direction & trust |
|---|---|---|
| **TB-1** | **anonymous internet → planeter** | **Untrusted in.** The public web/API surface; every byte attacker-controllable. |
| **TB-2** | **authenticated user → planeter** | Semi-trusted, *scoped by authorization*. An account may be compromised or its owner malicious; authority is never ambient (default-deny). |
| **TB-3** | **pushed content / uploaded prikk artifacts → planeter** | **Untrusted repository bytes over the network** — brygge's defining boundary, now remote. Parsed by the prikk subprocess and later *rendered* in the web UI. |
| **TB-4** | **planeter → prikk subprocess** | planeter drives prikk; **prikk is the verifier and the source of truth** (BN-2/INT-4). The process boundary is also a *containment* boundary for a prikk parser fault (C-4c). |
| **TB-5** | **CI workflow → runner** | **Untrusted code execution.** The largest RCE surface; a workflow, including one from a fork, is arbitrary code planeter agreed to run. |
| **TB-6** | **planeter → external services** | Outbound: webhooks, OAuth/OIDC providers, repository mirrors, package pulls, import sources. Every user-influenced URL is an SSRF vector (C-8). |
| **TB-7** | **planeter → viewer's browser** | Rendered content crosses into a privileged origin; the XSS boundary (C-6). |
| **TB-8** | **operator ↔ planeter** | Trusted. The operator deploys, configures, and backs up (AC-05); planeter trusts the host and its configuration (ASSUME-1). |
| **TB-9** | **the seal-signing boundary** | Where the maintainer signature on a merge is produced. Under the recommendation it is **client-side, outside planeter** (T-1); its location is the requirements' OQ-1. |

## 3. Threats and controls (T-…, C-…)

### T-1 (Spoofing / Tampering) — manufactured prikk-verified history
The forge, or an attacker who has compromised it, produces repository history that prikk's `verify`
accepts though no legitimate maintainer sealed it. This is the ecosystem-defining failure: a *prikk*
forge exists to offer stronger verifiability than a Git forge, and this threat is that promise broken
(A-HISTORY, A-SIGNING-KEYS).

- **C-1a — authority sits *above* verification and never substitutes for it** (SEC-1). planeter decides
  *who may* push/merge; prikk decides *whether the result verifies*. "Approved on planeter" and
  "verified by prikk" are distinct, both shown (WEB-06). A forge decision never becomes a prikk
  signature.
- **C-1b — planeter holds no history-signing key (the recommended posture, OQ-1 answer).** The seal on a
  merge is produced **client-side by the merging maintainer's own prikk** (TB-9 outside planeter); the
  forge stores and re-verifies it. A total forge compromise then **cannot** manufacture a seal that any
  client's prikk will accept — the worst case is serving unsigned or invalid data, which offline `verify`
  rejects. *This control is the reason to prefer OQ-1 option (a): it removes the catastrophic asset
  (A-SIGNING-KEYS) from the most-attacked component entirely.*
- **C-1c — verification is always re-runnable by the client** (INT-4). A client re-verifies on fetch
  against its own trusted maintainer keys; planeter is never the sole or final verifier. Trust does not
  terminate at the server.
- **C-1d — if server-side signing (OQ-1 option b) is ever enabled, it is contained, not casual.** It is
  **per-repository opt-in**, the key lives in an isolated signer (not the web tier), each use is scoped
  to an authenticated action and audited, and the resulting seal is **attributed to the forge identity,
  not a human** so a reader can tell a forge-sealed merge from a maintainer-sealed one (WEB-06). This
  bounds — but cannot eliminate — the blast radius (RR-2), which is exactly why (b) is never the silent
  default.

> **Usability note (answering OQ-1's real tension).** "Strict verification (a) vs. easy one-click (b)"
> is a *false* dilemma the design can dissolve: prikk separates **planning** a merge (`merge-plan`,
> `merge-evidence` — read-only, no key) from **sealing** it (`merge`/`seal` — needs the key). planeter
> can therefore do all the heavy preparation **server-side** and hand the maintainer's client a
> ready-to-seal plan that their prikk signs in **one action** — approaching (b)'s convenience while
> keeping (a)'s integrity and holding no key. This likely needs a small prikk-side affordance ("seal
> this prepared plan"), tracked as a prikk dependency; the internal design confirms it. The point for
> this threat model: **(a) is not doomed to be unusable, so its integrity advantage is not paid for in
> friction.**

### T-2 (Spoofing / Elevation) — authentication and session attacks
Credential stuffing, password reuse, session hijacking, token theft, OAuth/OIDC misconfiguration, SSH
key confusion — the common road to account takeover (A-CREDS).

- **C-2a — standard, audited auth only** (STD-2): OAuth 2.0 (code + PKCE) / OIDC; Argon2id for any local
  passwords; **TOTP and WebAuthn** second factors, **required for privileged accounts** (SEC-3).
- **C-2b — resist guessing**: rate-limited and lockout-protected authentication (SEC-5).
- **C-2c — session safety** (STD-6): `Secure` + `HttpOnly` + `SameSite` cookies; CSRF tokens for
  cookie-authenticated writes; short-lived, **scoped** tokens for machines, revocable and rotable.

**Status (0.1.x, 2026-09-23).** *New inbound flow:* browser sign-in — `POST /login` (TB-1 → TB-2)
creates an opaque random session id stored server-side (SQLite) and returned in an `HttpOnly` +
`SameSite=Strict` + `Secure` cookie; every cookie-authenticated `POST` carries a CSRF double-submit
token compared in constant time; the API accepts the same session cookie or a bearer token through one
principal extractor. *Implemented:* C-2a **partially** — Argon2id for local passwords, SHA-256 +
constant-time compare for scoped tokens, ed25519 SSH keys; **OAuth 2.0 / OIDC implemented 2026-09-24**
(authorization-code flow with PKCE, state one-shot and server-side, nonce-bound ID tokens verified by
`kid` against the provider's JWKS with `jsonwebtoken`'s RustCrypto backend; accounts linked to
`(issuer, subject)` administratively, never auto-provisioned; provider traffic only through C-8 + a
confined `curl`) and **no second factor yet** (RR-6).
C-2b **implemented as a per-account throttle** (5 consecutive failures → 15-minute lock, a correct
password refused while locked) **and a per-client-IP throttle** (20 failures across any accounts; client
IP via the trusted-proxy rules; RR-7). C-2c **implemented** as above; token revocation is by deleting the
stored hash.

### T-3 (Elevation) — authorization bypass / confused deputy
A user reaches a repository, ref, or action they are not entitled to: insecure direct object references,
missing checks on a code path, a low-scope token performing a high-scope action, privilege escalation
through org/team membership (A-AUTHZ, A-CONTENT).

- **C-3a — default-deny, least-privilege, everywhere** (SEC-5, AUTH-03): per-repository and **per-ref**
  authorization enforced on **every** access path — web, API, and the TX push/fetch endpoints alike —
  not just in the UI.
- **C-3b — scope enforced per endpoint** (STD-3): a token's scope is checked at each API endpoint; a
  read token cannot write, a one-repo token cannot reach another.
- **C-3c — protected refs** (AUTH-03): required review and checks, no force-replace or deletion on
  protected refs, so authorization cannot be sidestepped by rewriting history.

### T-4 (Tampering / DoS / Elevation) — hostile pushed content
A crafted prikk artifact or repository content exploits the prikk subprocess, escapes the server's repo
store via path traversal, or exhausts the host with decompression/delta bombs, pathological object
counts, or deep trees (A-HOST, A-HISTORY). This is brygge's TB-1, arriving over the network from anyone
who can push.

- **C-4a — all pushed bytes are untrusted** (TB-3). No server path assumes well-formed input; malformed
  input is a refusal with a named reason, never undefined behaviour.
- **C-4b — planeter never executes content-provided code.** It does not run repository hooks, filters,
  or scripts carried in pushed content; such content is data, stored, never invoked. **(INV-3.)**
- **C-4c — the prikk subprocess is the parser, and it is confined** (TB-4). planeter drives prikk over
  on-disk repos; a prikk parser fault is contained in a **separate, sandboxed process** (restricted
  user, filesystem and syscall confinement, no ambient credentials), so it cannot become forge-wide
  compromise — the network analogue of brygge's subprocess isolation (RR-3).
- **C-4d — server-declared paths are never write targets.** Repository content names data *inside*
  prikk's model; planeter never uses pushed path strings as server filesystem destinations, and confines
  every repo's storage to its own location.
- **C-4e — bounded, refuse rather than exhaust** (→ T-9): declared ceilings on artifact size, object
  count, path depth, and decompression ratio, per push; a ceiling hit is a clean refusal.

### T-5 (Elevation / RCE) — CI arbitrary code execution
A workflow runs attacker code (especially from a fork), exfiltrates secrets, escapes the runner, or
poisons build outputs; a malicious third-party action does the same through the supply chain
(A-HOST, A-SUPPLY, A-CREDS).

- **C-5a — runners are isolated and disposable** (CI-02, OPS-4): separate hosts from the forge,
  containerized, **ephemeral** (fresh per job), in a different trust domain, reaching planeter only over
  HTTPS with a scoped token.
- **C-5b — untrusted changes get no secrets** (CI-03): secrets are **withheld from fork-originated
  workflows** by default and require maintainer approval to run at all — closing the "pwn request".
- **C-5c — least-privilege job identity**: the job token is short-lived, scoped to that job, and is
  **never a prikk signing key** — CI does not seal history on a maintainer's behalf.
- **C-5d — pin the action supply chain** (T-11): third-party actions pinned by commit digest, not a
  mutable tag; reviewed before adoption.
- **C-5e — bound every job**: time, resource, and log-size limits (→ T-9).

### T-6 (Information disclosure) — XSS via rendered untrusted content
A forge continually renders attacker-controllable input (Markdown, repository files, issue comments,
uploaded assets). Rendering it into the app origin, or serving raw files from it, is account takeover
(A-CREDS, A-CONTENT, TB-7).

- **C-6a — strict CSP, designed in from the start** (STD-6): a bug becomes a non-event, not a takeover.
- **C-6b — sanitize on render**: Markdown/HTML stripped of scripts, handlers, and dangerous URL schemes
  before it reaches the page.
- **C-6c — isolate raw content on a separate origin** (STD-6): raw file views, uploaded attachments,
  avatars, and hosted pages come from a different domain than the application, so a malicious file
  executes in a throwaway origin with no session and no access to the forge DOM.

**Status (0.1.x, 2026-09-23).** *Implemented:* C-6a — a strict CSP on every response (`script-src 'self'`,
`style-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`, `base-uri 'none'`; no inline script
or style anywhere in the UI, whose one stylesheet is served from the app origin), plus `nosniff`,
`X-Frame-Options`, `Referrer-Policy`; C-6b — `ammonia` sanitization of all Markdown/HTML before it reaches
a page (scripts, handlers, `javascript:`/`data:` URLs stripped; tested); C-6c — raw repository bytes
served only as inert downloads (`Content-Disposition: attachment`, `nosniff`, a `sandbox` CSP) from the
configured isolated content origin, which the UI links to and never inlines.

### T-7 (Information disclosure) — confidentiality of private content
Private repository content or metadata leaks: an authorization gap (T-3), cross-tenant bleed, secrets
committed *inside* repository history, or content in logs (A-CONTENT).

- **C-7a — access control on every read path** (C-3a); private-by-default where appropriate (SEC-3).
- **C-7b — planeter does not worsen secrets-in-history exposure, and states the risk.** prikk carries
  repository content faithfully, so history may contain committed secrets; planeter's access control is
  the boundary, but it cannot un-leak a secret already committed. planeter states this plainly (as
  brygge does) so operators rotate/scrub **in the repository**; planeter does not silently redact
  (which would corrupt history) (RR-1).
- **C-7c — no sensitive content in logs**: diagnostics name repos, actors, and counts, not private
  content; audit logs (C-12) hold events, not payloads.

### T-8 (Elevation) — SSRF via user-supplied URLs
Webhooks, repository mirroring, avatar-by-URL, OIDC discovery, and import sources all make the *server*
fetch a URL a user chose; without controls, a user turns planeter into a proxy into its own
infrastructure (A-HOST).

- **C-8 — SSRF egress controls on every user-supplied URL** (STD-4/STD-6): block loopback, link-local,
  and private ranges by default, applied uniformly to webhooks, mirrors, imports, and OIDC — widened
  only by deliberate operator allowlist. This is the item most often missed and most damaging.

**Status (0.1.x, 2026-09-23).** *Implemented* as `planeter_core::egress::StdEgressGuard`: HTTPS only
(plain HTTP by explicit opt-in), userinfo refused, the host resolved via the OS resolver and **every**
answer required to be public unicast (loopback, private, CGNAT, link-local incl. `169.254.169.254`,
unspecified/multicast/reserved/documentation, and the IPv6 equivalents including IPv4-mapped and NAT64
forms all refused), returning the **pinned** addresses the caller must connect to and re-check on every
redirect; a private-target opt-in exists for internal webhook receivers. **First caller (2026-09-24): OIDC discovery,
JWKS and token exchange**, through `planeter_core::fetch::CurlFetcher` — the guard's pinned address is
passed to a bubblewrap-confined `curl` (`--resolve`, `--proto =https`, no redirects, byte and time
bounds). Webhooks and mirrors must take the same path — that is the review check for every future
outbound feature.

### T-9 (Denial of service) — resource exhaustion and availability
Enormous repositories and pushes, expensive prikk operations, CI abuse, API hammering, and disk fill —
a normal operating condition as much as an attack (A-AVAILABILITY, A-HOST).

- **C-9a — rate limits and quotas** (STD-3): API rate limits with `429`/`Retry-After`; per-account and
  per-repo quotas; bounded page sizes.
- **C-9b — bounded operations** (C-4e, C-5e): pushes, prikk invocations, and CI jobs are size/time/
  resource-bounded; a ceiling hit is a refusal, not an OOM.
- **C-9c — meter and alert** (OPS-5): disk especially (repos, artifacts and packages only grow); alert
  before exhaustion, not after an outage.

### T-10 (Tampering) — the forge metadata store as a false source of truth
An attacker who corrupts planeter's database or cache makes the *web view* show false history, or tries
to make the forge itself the authority for what history *is* (A-HISTORY).

- **C-10 — prikk is the source of truth; the store is cache + forge-owned data** (INT-4/NG-6). History
  and verification are re-derived from prikk over the hosted `.prikk`; a client re-verifies against its
  own trusted keys (C-1c). A DB compromise can mislead a *rendered page* but **cannot** produce history a
  client's prikk accepts — the integrity of A-HISTORY does not rest on the forge database.

### T-11 (Supply chain) — planeter's own dependencies and the CI-action ecosystem
planeter's crates and the third-party CI actions it runs are attack and audit surface (A-SUPPLY).

- **C-11 — pin, lock, minimize, audit**: exact versions with a committed lockfile; `cargo-deny` /
  `cargo-audit` in CI failing on a new advisory; a minimal, justified dependency set; planeter's own
  crates `forbid(unsafe_code)` outside any isolated FFI; new dependencies get architect review — the
  ecosystem's standing discipline. CI actions pinned by digest (C-5d).

### T-12 (Repudiation) — who did what
Disputes over who pushed, merged, or changed permissions (A-AUTHZ, A-HISTORY).

- **C-12a — audit log, off-box** (SEC-5, OPS-5): security-relevant events (sign-in, push, merge,
  permission and admin changes) logged durably off the server.
- **C-12b — cryptographic attribution of history is prikk's** — and is *better under OQ-1 (a)*: a
  client-side seal attributes a merge to the **human maintainer's** key; a forge-side seal (b) attributes
  it only to the forge. Non-repudiation of merges is therefore another reason the (a) posture is
  stronger (C-1b/C-1d).

### T-13 (Spoofing / Tampering / Elevation) — the AI-era threat surface
Modern attacks add one genuinely **new** class — an AI *in the loop* — on top of an *acceleration* of
the classic ones. The new class matters because a forge is saturated with attacker-controllable natural
language (repository files, commit messages, issues, review comments, and context files such as
`AGENTS.md`), and AI systems act on natural language (A-CONTENT, A-CREDS, A-HISTORY, A-AVAILABILITY).
Three concrete vectors, and one honest non-vector:

- **Prompt injection / AI-in-the-loop hijack.** An assistant, review bot, or autonomous agent that
  ingests repository content, an issue, or an `AGENTS.md` can be steered by instructions an attacker
  planted in that text — to leak private code, approve a malicious change, or take an authorized action.
- **AI-scaled abuse of the contribution surface.** Machine-generated pull requests and issues at volume
  — plausible-looking "slop", subtly back-doored code, automated vulnerability probing — aimed at
  overwhelming review or slipping a change past a tired maintainer.
- **Dependency hallucination ("slopsquatting").** An AI suggests a package or action name that does not
  exist; an attacker pre-registers it; it enters a build through the registry (REG) or CI (T-5/T-11).
- **The non-vector, stated to avoid hype:** most "AI attacks" on credentials, social engineering, and
  exploit generation are the *classic* threats (T-2, T-5, T-11) made cheaper and faster, not new ones
  (RR-8). They are answered by making the existing controls stronger, not by new machinery.

Controls follow forge-commons' AI verdict (*defer built-in AI; passive, privacy-preserving conventions
only*) — the forge's safety comes from **never being in the instruction-following position**:

- **C-13a — AI stays outside the trust boundary.** planeter builds no AI with **autonomous authority**:
  no AI can push, merge, seal, or change permissions on its own. Any AI assistance is advisory, and the
  authority path (AUTH, TX, the seal) has no AI actor in it. **(INV-9.)**
- **C-13b — content is data, never instructions.** Repository files, issues, comments, and context files
  like `AGENTS.md` are treated as untrusted input to *display or pass through*, never as commands the
  forge (or a forge-hosted integration) obeys. This is the structural defense against prompt injection:
  a system that never takes instructions from content cannot be injected through content. It is the same
  instruction-source discipline this project itself is built on.
- **C-13c — no default egress of private content to AI, no vendor baked in** (forge-commons AI). private
  repository content is never sent to an external AI service without explicit, per-use opt-in; the
  privacy-respecting integration point is the [API](#), where a *user* builds an integration that runs on
  infrastructure they control. The forge does not become the leak.
- **C-13d — AI-scaled abuse is bounded like any abuse.** The untrusted-change controls already required —
  review-required and CI-gated changes with **secrets withheld from forks** (C-3c/C-5b), rate limits and
  new-contributor trust gates (C-9a) — blunt volume; a maintainer is never obligated to review unbounded
  machine output. (The moderation problem federation raises, arriving early via AI.)
- **C-13e — hallucinated dependencies meet the same supply-chain controls.** Registry **immutability +
  scoped publish** (REG-02) and **digest-pinned** dependencies and actions (C-5d/C-11) deny a
  pre-registered phantom package an easy path in; a modern reason those controls are not optional.

### T-14 (Information disclosure) — secrets committed to repositories, at scale
Independent of AI, the most common real-world forge incident is a **secret committed to a repository**
(an API key, a token, a credential). prikk carries repository content faithfully (C-7b), so a committed
secret is exposed to everyone who could read that repository, for as long as it lives in history
(A-CONTENT).

- **C-14a — access control is the boundary; the risk is stated, not silently scrubbed** (C-7b). planeter
  does not redact history (which would corrupt it); it states plainly that history may contain secrets so
  the owner rotates them **in the repository**.
- **C-14b — optional secret scanning, as detection.** planeter may offer to scan pushes and existing
  history for likely secrets and **alert** the author/operator so they rotate promptly — running the scan
  **in-process / self-hosted with no third-party egress** (consistent with C-13c). This is *detection,
  not prevention*: a committed secret is already exposed (RR-9); scanning shortens time-to-rotate, it does
  not un-leak.

---

## 4. Security invariants (INV-…) — the non-negotiables

A change that breaks one of these is a security bug, not a preference. Several must be enforced by test.

- **INV-1 — No manufactured prikk-verified history.** Forge authority never produces or substitutes for
  a prikk signature; "approved" ≠ "verified" is enforced and visible; verification is always
  client-re-runnable. (T-1/T-10)
- **INV-2 — planeter holds no history-signing key by default.** The merge/seal signature is produced
  client-side (OQ-1 (a)). If server-side signing (b) is ever enabled it is **per-repo opt-in,
  key-isolated, forge-attributed, and audited** — never a hidden default. (T-1/T-12)
- **INV-3 — All network input is untrusted; planeter executes no content-provided code; the prikk parser
  runs confined; every user-supplied URL is SSRF-filtered.** (T-4/T-6/T-8)
- **INV-4 — Authorization is default-deny, least-privilege, enforced on every access path** (web, API,
  transport); admin and any signing capability are the most restricted. (T-2/T-3)
- **INV-5 — CI runs untrusted code isolated off the forge host; secrets are withheld from untrusted
  changes; a job token is never a signing key.** (T-5)
- **INV-6 — prikk is the source of truth; the forge store is never authoritative for history.** (T-10)
- **INV-7 — The dependency and CI-action supply chain is pinned, minimized, and audited;** planeter's
  crates `forbid(unsafe_code)` outside any isolated FFI. (T-11)
- **INV-8 — Private content is access-controlled on every path; planeter does not worsen secrets-in-
  history exposure and states the risk.** (T-7/T-14)
- **INV-9 — AI stays outside the trust boundary.** No AI holds autonomous authority (cannot push, merge,
  seal, or change permissions); repository content and context files are data, never instructions; no
  private content egresses to an external AI without explicit opt-in. (T-13)

## 5. Residual risks & assumptions (RR-…, ASSUME-…)

- **RR-1 — Secrets/PII committed in repository history are carried faithfully by prikk.** planeter's
  access control protects them in place but cannot un-commit them; redaction would corrupt history.
  planeter *states* the risk (C-7b); rotation/scrubbing is the operator's act in the repository.
- **RR-2 — If server-side signing (OQ-1 b) is enabled, a forge compromise can forge prikk-verified
  history for that repository until detected and the key revoked.** This is the accepted, **opt-in**
  cost of that convenience, bounded by key isolation, forge-attribution, and audit (C-1d) — and the
  reason (a) is the default (C-1b). The owner's OQ-1 ruling sets how widely this risk is ever taken on.
- **RR-3 — The prikk subprocess parses untrusted pushed bytes.** A prikk parser vulnerability is a real
  threat, confined by the sandboxed subprocess boundary (C-4c) but not eliminated; mitigated further by
  prikk's own `forbid(unsafe)` posture and by running prikk with least privilege.
- **RR-4 — prikk's on-disk format is unstable pre-1.0 (UD-3).** Hosting durability across format changes
  is a managed risk: planeter pins supported prikk format versions and refuses the unsupported (OP-04)
  until the durability policy (OQ-6) is ruled.
- **RR-5 — Multi-writer safety rests on prikk's local locking beneath planeter's per-repo serialization
  (UD-4)** — to be confirmed, not assumed, in the internal design.
- **RR-6 — No second factor on accounts yet (C-2a).** 0.1.x sign-in is password + per-account throttle
  only; TOTP/WebAuthn (required for privileged accounts by SEC-3) are not implemented. Until they are,
  privileged accounts should use long random passwords and scoped tokens, and deployments needing MFA
  should front planeter with an SSO/identity-aware proxy. Tracked for the 0.2.x auth increment with OIDC.
- **RR-7 — The login throttles are in memory, per process.** *(Reduced 2026-09-23.)* Sign-in is now
  throttled **per account (5 failures) and per client IP (20 failures across any accounts)**, the client
  IP derived from `X-Forwarded-For` only behind an operator-declared trusted proxy (`TB-8`;
  `PLANETER_TRUSTED_PROXIES`) and never from the header alone; a non-loopback bind is refused without
  that configuration. What remains: both throttles reset on restart and are not shared across
  replicas, so a multi-replica deployment should also rate-limit `/login` at the proxy.
- **RR-6 — A compromised CI runner can poison the build outputs it produces.** Mitigated by isolation
  (C-5) and by storing/verifying provenance (SEC-4), but a determined runner compromise is an
  industry-wide residual.
- **RR-7 — Third-party CI actions and planeter's own dependencies may carry vulnerabilities** (T-11);
  mitigated by pinning/audit, not eliminated.
- **RR-8 — AI lowers the cost of the classic attacks** (credential stuffing, phishing and
  account-recovery social engineering, exploit generation, mass low-quality contributions). This is
  *acceleration*, not a new vector: it raises the value of the existing controls (WebAuthn/MFA, rate
  limits, review and CI gates) rather than adding new ones. Named so the model is not lulled by treating
  "AI" as only the novel vectors of T-13.
- **RR-9 — Secret scanning (C-14b) is detection, not prevention.** A committed secret is already exposed
  to everyone who could read the repository; scanning shortens time-to-rotate but cannot un-leak, and has
  false negatives. Rotation in the repository remains the operator's act (RR-1).
- **ASSUME-1 — The operator and host are trusted; planeter defends against network actors and malicious
  users, not a hostile operator** (TB-8).
- **ASSUME-2 — prikk's offline `verify` is correct and is the ultimate arbiter of history integrity.**
  planeter's central integrity property (C-1/C-10) rests on prikk verification being sound and
  client-re-runnable.
- **ASSUME-3 — Users protect their own signing keys.** Under (a), a maintainer's key is client-side; a
  compromised maintainer key forges that maintainer's own seals — outside planeter's control, and
  correctly so.

## 6. Controls × threats

| Control ↓ / Threat → | T-1 | T-2 | T-3 | T-4 | T-5 | T-6 | T-7 | T-8 | T-9 | T-10 | T-11 | T-12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| C-1 authority≠verification / no forge key | ● | | | | | | | | | ○ | | ○ |
| C-2 standard auth / MFA / sessions | | ● | ○ | | | | | | | | | |
| C-3 default-deny authz / scope / protected refs | | ○ | ● | | | | ○ | | | | | |
| C-4 untrusted-input / confined prikk / bounds | | | | ● | | | | | ○ | | | |
| C-5 CI isolation / no-secrets-to-forks | | | | | ● | | | | ○ | | ○ | |
| C-6 CSP / sanitize / isolated content origin | | ○ | | | | ● | ○ | | | | | |
| C-7 confidentiality / secrets-stated | | | ○ | | | | ● | | | | | |
| C-8 SSRF egress controls | | | | | | | | ● | | | | |
| C-9 rate limits / bounds / metering | | | | ○ | ○ | | | | ● | | | |
| C-10 prikk is source of truth | ○ | | | | | | | | | ● | | |
| C-11 supply-chain pin/audit | | | | | ○ | | | | | | ● | |
| C-12 audit log / prikk attribution | | | | | | | | | | | | ● |

(● primary control, ○ contributing.)

**Modern additions (kept out of the grid above for width):** **C-13**a…e is the primary control for
**T-13** (AI-era), with C-3 (review gates), C-5 (CI/supply-chain isolation) and C-11 (pinning)
contributing; **C-14**a/b is the primary control for **T-14** (secrets at scale), with C-7 contributing.

## 7. Traceability

| Threat-model element | Requirements / design basis |
|---|---|
| T-1 / C-1* / INV-1/2 | SEC-1/SEC-2, NG-3, WEB-06, OQ-1, GATED-1; the ecosystem's key-locality (stikk) |
| T-2 / C-2* / INV-4 | STD-2, SEC-3/5, STD-6 |
| T-3 / C-3* / INV-4 | AUTH-03, STD-3, SEC-5 |
| T-4 / C-4* / INV-3 | TX-02, PK-01, INT-2, and the prikk-subprocess boundary (BD-03) |
| T-5 / C-5* / INV-5 | CAP-6, CI-01…04, OPS-4 |
| T-6 / C-6* / INV-3 | STD-6, WEB-05 |
| T-7 / C-7* / INV-8 | SEC-3, CT-03, and the secrets-in-history reality |
| T-8 / C-8 / INV-3 | STD-4/STD-6, HOOK-01 |
| T-9 / C-9* | STD-3, OP-03, OPS-5 |
| T-10 / C-10 / INV-6 | INT-4, NG-6, CT-03 |
| T-11 / C-11 / INV-7 | OPS-1, project dependency discipline |
| T-12 / C-12* | SEC-5, OPS-5, and OQ-1's attribution consequence |
| T-13 / C-13* / INV-9 | forge-commons *AI-Assisted Development* verdict (DEF); the instruction-source discipline |
| T-14 / C-14* / INV-8 | C-7b, and modern secret-scanning practice |

*End of Threat Model v0.1. Per project rules, this document is revisited every release: changes that
touch authentication/authorization, the transport or push path, the prikk-subprocess boundary, CI
execution, rendered content, or the signing model **update** this model; other releases **re-verify**
its controls. The controls most in need of a test from day one: INV-1/INV-2 (no manufactured
verification; no forge-held signing key by default), INV-3 (untrusted-input handling, confined prikk,
SSRF filtering), INV-4 (default-deny authorization on every path), and INV-5 (CI isolation; no secrets
to untrusted changes). **This model's headline finding, offered as the answer to OQ-1: hold no
history-signing key in the forge (option a) as the default, dissolve the usability cost with
server-prepared / client-sealed merges, and permit server-side signing (option b) only as an explicit,
isolated, forge-attributed, per-repository opt-in.** **OQ-1 was ruled by the owner (2026-09-15) as
option (a)** — so INV-1/INV-2 are settled policy, not a recommendation; the AI-era threats (T-13, T-14)
were added on the owner's 2026-09-15 review and are revisited whenever an AI-adjacent or
content-ingesting feature is proposed.
