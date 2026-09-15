# planeter — prikk Dependency Ledger

| | |
|---|---|
| Document | A living record of every prikk behaviour planeter's design depends on, and how sure we are of it. |
| Version | v1 (2026-09-16) |
| Why | We were burned once by an unverified assumption (the "seal locally, forge's ref advances" fallback — measured false). This ledger surfaces every other such dependency so the next wrong one is caught **before** implementation, and so it is obvious what is real, what is ruled, what is not yet shipped, and what is still only assumed. |
| How to use | Before implementing any RFC, check its prikk dependencies here. When prikk ships or measures something, move the row's status and cite the evidence. An **ASSUMED** row implemented without being promoted to **CONFIRMED** is a risk taken knowingly. |

## Status legend

- **CONFIRMED** — measured on a prikk binary (survey of prikk `HEAD f6cbd057`, 2026-09-15; and the prikk
  team's replies measured on `main @ 26e8c528`, 0.42.0+, 2026-09-15/16).
- **RULED** — a prikk owner/architect ruling or an accepted prikk RFC.
- **GATED** — accepted or proposed in *direction* but **not yet shipped**; planeter designs to it and
  **gates implementation on the prikk binary**.
- **ASSUMED** — planeter's design relies on it but it is **not yet verified**; the live risk list.
- **PARTIAL** — partly confirmed; a named gap remains (a `UD-`).

## The ledger

### A. What prikk gives a forge today (CONFIRMED / RULED)

| ID | planeter relies on | Status | Evidence / note |
|---|---|---|---|
| PK-1 | A machine-readable read surface — `--format json` on `log`, `show`, `status`, `branch`, `tag`, `verify`, `worktree-status`, `checkout --patch-plan --content-path`, `bundle preview`, `trust`/`key status` | **CONFIRMED** | survey 2026-09-15. Substrate for RFC 003 (read path). |
| PK-2 | `sync accept` ingests **author-signed** patches **keyless** → accepted-but-unsealed claims (prints claim ids) | **CONFIRMED** | prikk replies 2026-09-16 (measured). The keyless push path (RFC 004). |
| PK-3 | Moving a repo's own ref requires a **maintainer signature made in that repo** (`sync seal`); a keyless repo is refused (*"maintainer signing is required"*) | **CONFIRMED** | prikk UD-6 reply, measured. This is *why* the forge can't self-advance a branch. |
| PK-4 | A received ref lands under `remotes/` as an **untrusted pointer**; it cannot be adopted as a local branch, checked out, or **re-exported** today | **CONFIRMED** | prikk replies, measured. The reason relay needs RFC 155. |
| PK-5 | Block/RefState ids are **payload-identified** — the same history sealed by a second repo yields **identical** ids; only signatures differ | **CONFIRMED** | prikk replies, measured. Enables dedup/identity-stability (RFC 155 R6). |
| PK-6 | **One signed envelope per id** — a repo cannot hold the same history under two keys (import refuses, repo stays clean) | **CONFIRMED** | prikk replies, measured. Constrains mirroring; resolved for canonical branch by RFC 154's single-chain. |
| PK-7 | `bundle verify` checks **structure only, not signatures** | **CONFIRMED** | prikk reply 2026-09-16. Offline *signature* verify depends on RFC 155, not `bundle verify`. |
| PK-8 | `setup` names every maintainer key `maintainer` and ignores `PRIKK_MAINTAINER_KEY_ID`; cross-repo accept/seal works only with **distinct key ids** | **CONFIRMED** | prikk replies, measured 2026-09-15. planeter's identity model assumes distinct ids (RFC 002). |
| PK-9 | Object identity + signatures are **frozen forever** across format changes | **RULED** | prikk RFC 114 §3. De-risks migration (RFC 009). |
| PK-10 | A **tested migration must exist before any format change ships** (CI-enforced) | **RULED** | prikk RFC 114 §5.2 (2026-08-19). planeter's adoption gate aligns with this. |
| PK-11 | `sync` is **negotiation-as-artifacts, off the network** | **RULED** | prikk RFC 116. planeter ferries artifacts; invents no wire protocol (RFC 004). |
| PK-12 | Serving-for-reading is **Shape D** — the CLI JSON surface is the substrate; prikk ships no server | **RULED** | prikk RFC 145. planeter drives the CLI, never links the crate. |
| PK-13 | At **1.0**, stability arrives in layers: object format + exchange artifacts first, CLI JSON second, library API last | **RULED** | prikk RFC 152 §5. planeter builds on the layers that stabilize first. |

### B. Accepted/proposed in direction, not yet shipped (GATED — implementation waits on the binary)

| ID | planeter relies on | Status | Evidence / note |
|---|---|---|---|
| PK-14 | **Trusted fast-forward ref adoption** — a keyless repo publishes a received, trusted-maintainer-signed fast-forward as its own canonical branch, no re-sign | **GATED** (accepted, not shipped) | prikk **RFC 154**, accepted by prikk owner 2026-09-16. The canonical-branch/merge primitive (RFC 004/005/008). |
| PK-15 | The **repository-complete artifact** — whole-repo, offline-verifiable, verbatim, read-only export + all-or-nothing import, `import --adopt` | **GATED** (proposed) | prikk **RFC 155**, proposed 2026-09-16 (planeter's requirements folded in). Clone/serve/migrate substrate (RFC 004/009). **Not yet accepted** — highest-leverage open dependency. |
| PK-16 | The **key-id collision fix** in `setup` | **GATED** (in prikk's ship order) | First in prikk's post-0.43.0 order. Workaround: distinct key ids (PK-8). |

*Ship order prikk stated (post-0.43.0): key-id fix → RFC 155 → RFC 154.*

### C. Still only assumed — the live risk list (ASSUMED / PARTIAL)

| ID | planeter assumes | Status | Risk / action |
|---|---|---|---|
| PK-17 | The `--format json` read surface is **complete enough** for a full forge browse (raw blob access is via `show`/`checkout` only; **no blame/annotate**) | **PARTIAL** (`UD-1`) | Views needing missing data render *"pending a prikk increment"*, never faked (RFC 003 D-7). Catalogue the gaps during RFC 003 build; raise as prikk asks. |
| PK-18 | The CLI **JSON schemas are stable enough** to drive across prikk versions | **ASSUMED** | prikk RFC 152 §5 says CLI JSON stabilizes **second**, at 1.0 — so **pre-1.0 the JSON may change**. planeter pins supported prikk versions (RFC 001 D-3) and must treat JSON-schema drift as a version-gated risk. **Verify per prikk release.** |
| PK-19 | prikk's **local locking is sufficient** beneath planeter's per-repo write serialization | **ASSUMED** (`UD-4`/`IQ-3`) | Confirm during the RFC 004 transport build (its handoff T6). If insufficient, raise before proceeding. |
| PK-20 | The accepted-but-unsealed queue (`sync pending`) exposes what planeter needs to render an **open change** | **CONFIRMED-ish** | prikk reply: the claims are stored objects that travel in RFC 155; review state is planeter metadata. Treat the *content* as confirmed, the *artifact carriage* as GATED on PK-15. |
| PK-21 | Driving prikk as a **subprocess** with a stable, gateable outcome is viable | **CONFIRMED** for reads (PK-1); **GATED** for the exchange/adopt verbs (RFC 155 promises JSON outcomes) | Reads work today; the adopt/artifact verbs' outcomes arrive with PK-14/PK-15. |

## Reading the ledger

- **Section A is the floor** — real today, safe to build M1 (host/browse/auth + keyless `accept`) on.
- **Section B is the gate** — the keyless canonical-branch, merge, relay and migration paths are *designed* but wait on prikk shipping PK-14/PK-15/PK-16. **PK-15 (RFC 155) is only proposed** and is the single most important open dependency: it is the clone/serve/migrate substrate and it is not yet accepted.
- **Section C is the watch list** — especially **PK-18** (pre-1.0 JSON drift) and **PK-19** (locking), which no correspondence has settled and which implementation must verify rather than assume.

Update this file whenever prikk ships, measures, or rules something a row depends on.
