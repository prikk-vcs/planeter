# planeter — Roadmap

How planeter gets from a design set to a real, familiar forge for prikk. This is a **direction with
milestones**, not a dated schedule: work ships when it is correct, tested, secure, and honest.
Requirement and design ids (e.g. `CAP-2`, `WR-5`, `INV-2`, `LAY-2`) refer to the design set in
[`docs/src/`](docs/src/) — `planeter-01` requirements, `-02` external design, `-03` threat model,
`-04` internal design. The commons frame is [forge-commons](https://github.com/kos-commons/forge-commons);
the upstream is [prikk](https://github.com/prikk-vcs/prikk).

## The wide perspective (read this first)

**planeter is a familiar forge — and that is a large build.** Host repositories people clone and push,
propose and review changes, track issues, run CI, publish packages, manage teams and permissions, over
a web UI and an API. The ambition is Forgejo-shaped, even if leaner; the roadmap is honest that reaching
1.0 is a multi-milestone journey, not a single release.

**One structural fact shapes the whole plan:** planeter builds on **prikk's stable CLI surface**, not
its internals (RFC 145 Shape D; the stikk precedent). So **almost all of planeter can be built now,
without changing prikk** — prikk is already a working single-repo VCS with a machine-readable read
surface and artifact-based exchange. Only a small, well-named set of things need *joint* prikk work, and
they are decoupled so they never block the bulk.

**Two tracks, deliberately decoupled** (the brygge pattern):

- **Track A — the forge over prikk-as-is.** The bulk of planeter. Depends only on prikk's existing CLI
  (`log`/`show`/`verify`/… JSON) and artifact exchange (`bundle`/`sync`). Buildable today, milestone by
  milestone, to a usable increment at each step.
- **Track B — the joint prikk work.** A short list of prikk-side capabilities planeter needs
  (one-click client-side merge, hosted-format durability). Each is a joint planeter+prikk RFC, owner-
  gated, advancing in parallel; Track A ships with a keyless fallback until each lands.

**The arc, at a glance** — each milestone is usable; 1.0 is *familiar-complete*:

```
 A0 foundations → A1 host+browse+auth → A2 clone+push (keyless) → A3 review+issues+merge
      → A4 CI → A5 packages ─────────────────────────────────────────────► 1.0 familiar forge
                                                                              │
 Track B (parallel, gated on prikk RFC 154/155): B1 canonical branch + merge · B2 format durability ─┘
 Later (deferred, per forge-commons verdicts): federation · AI (passive) · portable identity
```

## Guiding rules (constant across the roadmap)

- **Design before implementation** (project rules). Requirements → external → threat model → internal
  design → **RFC + handoff** → implementation → tests. Never inverted.
- **Carry the hosting weight; keep prikk lean** (PU-2/BN-4). Nothing planeter needs pushes weight back
  into prikk's five-dependency, offline core. If a capability cannot be built without changing prikk's
  posture, it is a Track-B joint RFC or it is deferred.
- **The forge signs nothing, by construction** (INV-2/ENF-2). The default build contains no
  forge-held-key seal path; a push `accept`s author-signed patches with no key, and the merge seal is
  the maintainer's own (OQ-1 a). No milestone ships a default build that can forge prikk-verified
  history.
- **Lean on the standards; defer the frontier** (STD-*/DEF). Federation, portable identity, and
  built-in AI are post-1.0, per forge-commons' *pilot/defer* verdicts. A familiar forge does not need
  them.
- **Authorization is a single core service** (LAY-2/INV-4), consulted by every surface, from the first
  milestone that has a surface.
- **Security is release-gating** (threat model). A release touching auth, transport, the prikk boundary,
  CI, rendered content, or the signing model **updates** `planeter-03`; others **re-verify** it.

---

## Track A — the forge over prikk-as-is

### Phase A0 — Foundations
The substrate every surface sits on; no network yet. **CR-prikk** (drive the prikk CLI as a sandboxed
subprocess, typed JSON — INT-1/PKI-*), **CR-store** (forge-metadata persistence, DB-abstracted),
**CR-core** with the `authorize()` service (LAY-2), **CR-auth** primitives, and the **layering gate**
(LAY-1, `boundary-check`-style). Delivers nothing user-facing; makes everything above it safe.

### Phase A1 — Host + browse + auth (the read/host spine)
Multi-repo hosting and forge-level identity (owner/name/URL above anonymous prikk repos — DM-1);
the **read path** (browse history/changes/contents/verify, re-derived from prikk — RD-*, safe rendering
STD-6); **authentication** (OAuth/OIDC, tokens, SSH keys) and **per-repo/per-ref authorization** (AZ).
**Outcome: a real, hosted, browsable, access-controlled forge.** Buildable with no prikk change.

### Phase A2 — Clone + push (keyless)
The **transport ferry** (TX): fetch/clone (server builds, client accepts — WR-1) and **push** (client
builds, server `accept`s author-signed patches — WR-2/WR-3), plus the **client helper** (TX-05).
**No maintainer key touches the forge.** Outcome: a live host you clone from and push to.
**Measured 2026-09-23 on 0.46.0 (ledger PK-27/28/29), which reshaped this phase:** push is fully
keyless (`sync accept` idempotent, tamper-refusing, verify-clean); `bundle export` / `sync have` /
`sync summary` need no key; but **`sync build` refuses without a maintainer key**, so a keyless forge
cannot build incremental-fetch artifacts, and a planeter-created repository has **no sealed branch to
serve** until RFC 154 adoption exists. **Owner ruling: A2 is held until prikk 0.48.0+** (RFC 155/154,
plus the `sync build` ask), so clone and push land together rather than shipping a push-only 0.2.0.
**Transport layering (owner-ruled 2026-09-23, supersedes RFC 004 D-6's in-process reading):** planeter
serves plain HTTP on loopback behind a **TLS-terminating reverse proxy** (TLS stays mandatory as a
deployment rule the binary enforces — no non-loopback bind without a trusted-proxy configuration), and
SSH is the **host's OpenSSH with `ForceCommand` → `planeter ssh-shell`** (fingerprint → user is RFC 002's).
Measured alternative rejected: in-process rustls + russh = +165 crates, cargo-deny advisories and
licenses both failing, russh's own advisory (RUSTSEC-2026-0154) and the unfixable `rsa` one.

### Phase A3 — Review + issues + merge
The **change/review model** (WR-4): an open change *is* prikk's accepted-but-unsealed claim set, with
review metadata, inline discussion, approvals; **issues** and cross-references (CAP-4); and **merge via
the keyless fallback** (WR-5b: maintainer's own prikk seals, forge stores). Outcome: the full
collaboration loop.

### Phase A4 — CI
Pipelines and **isolated, ephemeral, separate-host runners** (CIO-*, INV-5): triggers on push/change/
schedule, secrets withheld from untrusted changes, least-privilege job tokens.

### Phase A5 — Packages
The **registry** (RG): OCI Distribution first, language formats by demand; scoped publish, immutable
versions, object-storage-backed.

## Track B — joint prikk work (gated, parallel, never blocks Track A)

> **Trust-model resolved 2026-09-16.** The keyless multi-maintainer question is answered by two prikk
> RFCs: **RFC 154 — trusted fast-forward ref adoption (accepted by the prikk owner)** lets a keyless
> forge hold a canonical, multi-maintainer branch by *adopting* trusted-maintainer-signed advances; and
> **RFC 155 — the repository-complete artifact (proposed)** is the clone/serve/migrate substrate. Track B
> is now **gated on prikk shipping these**. Ship order (updated 2026-09-22): key-id fix + format-7
> signature union (**0.45.0, done**) → `diff`/read verbs (**0.46.0, done**) → import/verify size bound
> (**0.47.0**) → **RFC 155, then RFC 154's adoption act (0.48.0 onward)**. planeter designs to the
> direction now; implementation waits on the binary. **This gates Track B only — M2 (keyless clone +
> push) is buildable on prikk-as-is and proceeds now (owner-confirmed 2026-09-23).**

### Phase B1 — Canonical branch + one-click merge (prikk RFC 154 + 155)
A merge is a maintainer sealing (their own key, client-side) and the **forge adopting** the resulting
trusted-maintainer-signed fast-forward (RFC 154) — keyless, multi-maintainer, "first fast-forward wins".
Clone/serve of that canonical branch, and open-change claims, ride the RFC 155 artifact (`import
--adopt`). This supersedes the earlier "client-sealable claim" framing (withdrawn — prikk declined
blind-signing). Gated on the prikk binary.

### Phase B2 — Hosted-format durability (OQ-6 / UD-3)
Carry-forward = `init` → adopt maintainer keys → `import --adopt` of the RFC 155 artifact (read-only
export, all-or-nothing import). **De-risked:** prikk RFC 114 §5.2 already requires a tested migration
*before* any format change ships (CI-enforced), object identity/signatures are frozen forever, and
format 6 has held since 0.20.0 with none planned. planeter's adoption gate aligns with prikk's. Gated on
the prikk binary; required before 1.0.

## Later — deferred frontier (post-1.0)
Per forge-commons verdicts, and only when their triggers are met: **federation** (pilot ForgeFed
read/announce; defer cross-instance writes), **AI** (passive conventions only — no autonomous authority,
content-as-data, no default egress), **portable identity** (defer until a standard interoperates).

---

## Milestones & versions

| Milestone | Version | Contents | Track | Status |
|---|---|---|---|---|
| **M0** | 0.1.0-dev | Foundations: CR-prikk driver, store, core + `authorize`, auth, layering gate | A0 | **done** (2026-09-16) |
| **M1** | **0.1.0** | **Host + browse + auth**: multi-repo hosting/identity, read path, sign-in, per-ref authz. The first usable hosted forge | A1 | **released 2026-09-23** — signed tag `0.1.0` at `01bbb1c`; [GitHub release](https://github.com/prikk-vcs/planeter/releases/tag/0.1.0) (attested Linux x86_64 + aarch64 tarballs), container image `ghcr.io/prikk-vcs/planeter:0.1.0`, and all nine crates on crates.io (`cargo install planeter`) |
| **M2** | 0.2.0 | **Clone + push (keyless)**: transport ferry, fetch, push=accept+verify, client helper | A2 | **HELD until prikk 0.48.0+ (owner-ruled 2026-09-23)** — measured on 0.46.0: push is fully keyless, but `sync build` requires a maintainer key (PK-27) and a planeter-created repo has no servable branch until RFC 154 adoption, so clone/fetch cannot land keylessly yet; M2 ships clone + push together once RFC 155/154 (and a keyless `sync build`, asked) ship |
| **M3** | 0.3.0 | **Review + issues + merge**: change/review model, issues, merge (keyless fallback WR-5b) | A3 | planned |
| **M4** | 0.4.0 | **CI**: pipelines + isolated ephemeral runners | A4 | planned |
| **M5** | 0.5.0 | **Packages**: OCI + first language registries | A5 | planned |
| **B1** | ships within 0.x once prikk RFC 154 + 155 ship | **Canonical branch + one-click merge** (maintainer seals, forge adopts the fast-forward) | B1 | gated (prikk RFC 154 accepted, 155 proposed) |
| **B2** | before 1.0 | **Hosted-format durability policy** | B2 | gated (owner OQ-6) |
| **1.0.0** | 1.0 | **The familiar forge, complete**: host + transport + review + one-click merge + CI + packages + web + API, hardened, backup-tested, durability policy settled | A + B | pending A0–A5, B1, B2 **+ owner confirmation** |

> **v0 → v1 is an owner gate.** All development proceeds in **v0.x**. Promoting to **1.0 requires the
> owner's clear, explicit confirmation** — green gates and feature-completeness are necessary but **not
> sufficient**. Until then, the format, APIs, and surfaces are treated as pre-1.0 (unstable, additive
> where possible), and no milestone silently crosses into 1.0.

## Release cycles

**planeter releases an application; crates.io is an install path, not the release.** A release is
**a tagged bare version plus built artifacts to deploy** (the `planeter` server binaries and a container
image), published as a GitHub release with build attestations. **Amended 2026-09-23 (owner-ruled):** the
nine `planeter-*` crates are *also* published to crates.io at each release so operators can
`cargo install planeter`, matching prikk/stikk/brygge — but the deploy artifacts remain what a release
*is*; crates.io never substitutes for them (it cannot carry the prikk + bubblewrap prerequisites the
container does). `planeter-runner` stays unpublished until M4.

- **A0 is pre-release; the first release is M1 (0.1.0).** The workspace stays `0.0.0` through foundations
  (the "0.1.0-dev" milestone); **nothing is tagged until M1** ships the first usable product
  (host + browse + auth = A0 + RFC 002 + RFC 003). The version is set to `0.1.0` at the M1 tag.
- **0.1.0 means "genuinely usable", not "preview" (owner-confirmed 2026-09-23).** The read/host spine is
  built and runnable, but two increments were deliberately deferred behind seams during A1 and must land
  **before** the tag so the first release matches the M1 promise. The **M1 pre-tag checklist**:
  1. **Auth crypto** behind the RFC 002 seams — **done 2026-09-23** (`d3fa61b`): Argon2id password
     hashing, constant-time SHA-256 token hashing + minting, and OpenSSH ed25519 key parsing, through the
     supply-chain review (+15 crates; `ssh-key` rejected for locking the unfixable RUSTSEC-2023-0071
     `rsa` into the lockfile). **OIDC/SSO is deferred past 0.1.0 (owner-ruled 2026-09-23)**: its
     verifier needs a JWT stack (`ring`, a license exception) *and* an outbound HTTP client behind the
     egress guard — RFC 004's design — so it lands as a configured integration in 0.2.x; the
     `OidcVerifier` seam stays. 0.1.0 auth = local passwords + scoped tokens + ed25519 SSH keys.
  2. **A persistent store** — **done 2026-09-23** (`94e922d`): SQLite (`rusqlite`, bundled; +16 crates,
     deny/audit clean) behind all four traits — `RepositoryStore` / `MembershipStore` (planeter-store)
     and `AccountStore` / `CredentialStore` (planeter-auth) — over one shared `SqliteDb` (WAL, idempotent
     schema; each crate applies its own tables). Same invariant tests as the in-memory stores plus a
     reopen-persistence test. The `planeter` binary now runs on `PLANETER_DB` (default `./planeter.db`).
  3. **`release.yml`** — **done 2026-09-23**: on a bare-version tag → gates (tag == version, fmt,
     clippy, test, deny, audit, ENF-2, layering) → the `planeter` server for Linux x86_64 + aarch64
     (tarball + sha256 + build-info, **build-provenance attested**) → a `linux/amd64`+`arm64` container
     image on `ghcr.io/prikk-vcs/planeter` built from the prebuilt binaries with checksum-verified
     prikk 0.46.0 + bubblewrap baked in (attested) → the GitHub release with notes from the tag's
     `CHANGELOG.md` section. Linux-only by design (the sandbox is bubblewrap). Procedure:
     `docs/RELEASING.md`. The `planeter-runner` binary ships at M4 (CI), not here.
  4. The `planeter` binary drops its preview label once 1–2 are wired — **done 2026-09-23** (it now
     announces only its address and database).

  **All four done, and 0.1.0 shipped (2026-09-23).** The release run's `publish` job failed on an
  artifact-download defect (fixed for future releases in `fedf904`); the GitHub release was created by
  the documented fallback from the run's own checksum- and attestation-verified assets. The nine crates
  were published to crates.io in dependency order (the new-crate rate limit paced the last four).
  The tag itself remains owner-only.
- **Milestone-driven minors** (M1 `0.1.0` → M5 `0.5.0`). A minor ships only when its gates are green
  (fmt · clippy `-D warnings` · test · `cargo-deny`/`cargo-audit`), its security invariants hold
  (notably **ENF-2**: the default build links no forge-seal path — CI-checked), and the threat model is
  updated if the release touched a sensitive surface.
- **The read/host spine ships early and often.** M1 is a genuinely usable product; later minors add
  capability without a big-bang.
- **Track B ships when ready, within 0.x** (gated on prikk shipping RFC 154/155); not a blocker for its
  neighbours.
- **Security releases are out-of-band.** A dependency advisory or a threat-model control failure triggers
  a prompt patch release.
- **Runtime prerequisites are release notes, not code:** **prikk ≥ 0.46.0** (the version pin, PK-26 —
  it carries the `tree`/`cat`/`diff` read verbs, the 0.44.0 GHSA-px5q-233r-6hq5 import fix, the format-7
  signature union and the key-id fix) and **bubblewrap** on the host (the sandbox, T4) are documented
  deployment prerequisites of a release. **Format 7** is a breaking, explicit-only `prikk format
  upgrade` with no downgrade: a deployment keeps its serving binary at or above the format its hosted
  repositories carry, and planeter never upgrades a hosted repository without an intended act.
- **planeter re-baselines against each prikk release (owner-confirmed 2026-09-23).** prikk moves fast
  (0.43.0 → 0.46.0 in six days). For every prikk release: bump the pin when it ships a verb planeter
  needs or a security fix; **re-verify every JSON schema planeter reads** (`schema_version` drift, PK-18)
  against the released binary; record the outcome in the dependency ledger. A pin bump lands only with
  the binary present, so the version-pinned integration tests validate against it rather than refuse
  it. Mirrors stikk's per-release rebaseline practice.
- **The release workflow lands as M1 nears** — a `release.yml` (not needed during A0): on a bare-version
  tag → gates green → build binaries + container → attest/sign → GitHub release. **Tagging, publishing,
  and the v0→v1 promotion are owner-only.**

## Dependencies (on prikk and on the owner) — do not block Track A

Named so no plan silently assumes them:

- **prikk RFC 154 — trusted fast-forward ref adoption. ACCEPTED (prikk owner, 2026-09-16); scheduled
  0.48.0 onward, after RFC 155.** The keyless multi-maintainer canonical-branch primitive B1 depends on.
  Supersedes the old UD-6 "client-sealable-claim" ask.
- **prikk RFC 155 — the repository-complete artifact. ACCEPTED (prikk owner, 2026-09-16); scheduled
  0.48.0 onward.** planeter's R1–R6 are prikk's accepted direction; the clone/serve/migrate substrate for
  B1 and B2 (`import --adopt`). Its all-or-nothing import (R4) gets its own prikk design round, with
  planeter's input invited; **R6 is already answered** by format 7's signature union (0.45.0). 0.47.0
  lands an import/verify size bound first — the nearest thing to R4 shippable before RFC 155 opens.
- **prikk 0.43.0 → 0.46.0 (2026-09-16 → 2026-09-22).** 0.43.0 fixed `bundle export`/`sync build` on
  delete-after-edit histories (PK-22). **0.44.0** fixed **GHSA-px5q-233r-6hq5** — a refused `bundle
  import` corrupted the receiving repository (all 0.23.0–0.43.0; exactly a relaying forge's exposure).
  **0.45.0** shipped **repository format 7** (a verified signature union — up to four counted signatures
  per object; breaking-once, explicit-only, no downgrade) and the **key-id fix** (`ed25519-<16hex>`
  default ids). **0.46.0** shipped the read verbs co-designed with planeter — `prikk tree`
  (`tree-listing-v1`), `prikk cat` (`path-content-v1`), `prikk diff` (`diff-report-v1`, RFC 153) —
  closing the RFC 003 gap catalogue except blame. **⇒ planeter's version pin is prikk ≥ 0.46.0**
  (dependency-ledger PK-26); the ledger v1.2 records PK-23/24/25/26. One caveat honoured: `cat
  --max-bytes` bounds bytes *written*, not memory, so planeter bounds hostile input by what it accepts.
- **UD-3 / OQ-6** — hosted-format durability (B2): **de-risked** — prikk RFC 114 §5.2 requires a tested
  migration before any format change (CI-enforced); mechanism is RFC 155 `import --adopt`.
- **OQ-2** — whether per-ref authorization anchors to a prikk-side notion or is purely planeter's (shapes
  AZ-2/WR-5). Owner-ruled.
- **OQ-4** — the exact v1 feature ceiling beyond the CAP core (wikis, discussions, boards). Owner-ruled.
- **OQ-7 / IQ-1** — reuse stikk's `stikk-prikk` layer for CR-prikk, or build independently. Dev-team
  decision, architect recommendation to evaluate reuse behind a trait.
- **UD-4 / IQ-3** — prikk local-locking sufficiency beneath planeter's per-repo serialization. Confirmed
  in the transport RFC.

*OQ-1 is settled (2026-09-15, option a): the forge holds no history-signing key by default — and, as of
2026-09-16, this is viable for **multi-maintainer** forges too, via prikk RFC 154's trusted
fast-forward adoption (a keyless forge can deny, never forge).*

---

## The RFC plan (priority and order)

Detailed per-subsystem design lives in [`rfcs/`](rfcs/) (5-folder lifecycle, per project RFC 000);
this is their **priority and order**, each mapped to the phase it serves. The architect writes each RFC
+ handoff; the dev team implements against it; the owner authorizes. **RFC 001 first** — nothing else
starts until the foundations and the layering gate exist.

| # | RFC (planned) | Serves | Priority |
|---|---|---|---|
| **001** | Foundations: prikk-integration layer (CR-prikk), the layering gate (LAY-*), multi-repo hosting & identity (DM-1) | A0→A1 | **first** |
| **002** | Authorization model — the `authorize()` contract, roles, per-ref permissions (AZ-2) | A1 | high |
| **003** | Read path & web browse — re-derivation, caching, safe rendering (RD-*) | A1 | high |
| **004** | Transport envelope + client helper — fetch/push over ferried artifacts (WR-1/2, TX-05) | A2 | high |
| **005** | Change/review model — accepted-unsealed-as-PR, review, keyless merge (WR-4/5b) | A3 | medium |
| **006** | CI + runner protocol (CIO-*) | A4 | medium |
| **007** | Package registry (RG-*) | A5 | medium |
| **008** | *(joint prikk)* Seal / UD-6 — now via prikk RFC 154 adoption + RFC 155 artifact | B1 | parallel, gated (prikk 154 accepted) |
| **009** | Hosted-format durability policy (OQ-6/UD-3) | B2 | before 1.0 |

**Immediate next step:** stand up planeter's `rfcs/` (done alongside this roadmap — see
[`rfcs/README.md`](rfcs/README.md)), then draft **RFC 001** on the owner's go-ahead.

## Method & roles (project rules §Workflow)

The **architect** (High-Capability Model) writes the schedule, the RFCs and their handoffs, and manages
release cycles. The **dev team** (Mid-Capability Model) implements against the handoffs and returns
review packages. The **owner** authorizes releases, tags, and the owner-only decisions (the OQs above).
Both architect and dev team are authorized to commit and push. Entry points are surfaced to the owner as
file paths, so the owner manages at low cost: this roadmap, and [`rfcs/README.md`](rfcs/README.md).
