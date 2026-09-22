# planeter — prikk Dependency Ledger

| | |
|---|---|
| Document | A living record of every prikk behaviour planeter's design depends on, and how sure we are of it. |
| Version | v1.2 (2026-09-22 — folds prikk 0.44.0 (GHSA-px5q-233r-6hq5), 0.45.0 (format 7 signature union + key-id fix), and 0.46.0 (`tree`/`cat`/`diff` read verbs)) |
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
| PK-6 | ~~**One signed envelope per id** — a repo cannot hold the same history under two keys (import refuses)~~ → **superseded: repository format 7 stores a verified signature union** (one object, up to 4 counted signatures, each verified before counting) | **CONFIRMED** (prikk **0.45.0**, format **7**) | prikk letter 2026-09-22. The old limitation is lifted by an explicit `prikk format upgrade`; this is what **answers RFC 155 R6** (a re-import meeting a held id adds its signature to the union, within the counted bound, instead of refusing as damage). Format 7 is **breaking once, no downgrade**; 0.44.0-and-older refuse an upgraded repo ⇒ the serving binary must be ≥ the format the hosted repos carry. |
| PK-7 | `bundle verify` checks **structure only, not signatures** | **CONFIRMED** | prikk reply 2026-09-16. Offline *signature* verify depends on RFC 155, not `bundle verify`. |
| PK-8 | ~~`setup` names every maintainer key `maintainer`~~ → **fixed: default key ids are now `ed25519-<first 16 hex of the public key>`**, so two default installations can exchange history | **CONFIRMED** (prikk **0.45.0**) | prikk letter 2026-09-22 (was the collision behind PK-16). A prerequisite for a forge that relays between people; planeter's identity model (RFC 002) no longer needs the distinct-id workaround. |
| PK-9 | Object identity + signatures are **frozen forever** across format changes | **RULED** | prikk RFC 114 §3. De-risks migration (RFC 009). |
| PK-10 | A **tested migration must exist before any format change ships** (CI-enforced) | **RULED** | prikk RFC 114 §5.2 (2026-08-19). planeter's adoption gate aligns with this. |
| PK-11 | `sync` is **negotiation-as-artifacts, off the network** | **RULED** | prikk RFC 116. planeter ferries artifacts; invents no wire protocol (RFC 004). |
| PK-12 | Serving-for-reading is **Shape D** — the CLI JSON surface is the substrate; prikk ships no server | **RULED** | prikk RFC 145. planeter drives the CLI, never links the crate. |
| PK-13 | At **1.0**, stability arrives in layers: object format + exchange artifacts first, CLI JSON second, library API last | **RULED** | prikk RFC 152 §5. planeter builds on the layers that stabilize first. |
| PK-22 | `bundle export` / `sync build` handle **ordinary histories that delete a previously-edited file** (the exporter replays history to derive the deleted content) | **CONFIRMED** (prikk **0.43.0**) | Fixed in 0.43.0; every release 0.28.0–0.42.0 refused these with *"integrity error: missing blob object"* while `verify` passed. A 0.43.0 bundle still imports in a 0.42.0 binary (mixed-version relay property, measured). |
| PK-23 | **`prikk tree [--ref <ref\|block-id>] [--prefix <p>] --format json`** → schema **`tree-listing-v1`** — leaf paths only (`{path, kind: file\|symlink, encoding: text\|binary, mode (full), size (exact), content_id (binary only)}`); planeter builds directories from segments | **CONFIRMED** (prikk **0.46.0**) | prikk letter 2026-09-22, measured on the released asset. Closes the A1 directory-listing browse blocker (was `UD-1`). Takes a ref **or bare block id** (shared resolver); `remotes/…` read, not adopted. |
| PK-24 | **`prikk cat --path <p> [--ref <ref\|block-id>] [--output <file> [--force]] [--max-bytes N] [--format json]`** → schema **`path-content-v1`** — writes reconstructed bytes (text **and** binary) all-or-nothing; `--format json` writes metadata + size only | **CONFIRMED** (prikk **0.46.0**) | prikk letter 2026-09-22. Closes the A2 raw-bytes browse blocker. **Caveat: `--max-bytes` bounds what is *written*, NOT memory** — replay holds the whole content regardless (a memory bound via streaming/chunk manifests is scheduled). ⇒ planeter must bound hostile input **by what it accepts**, not by this flag. Binary refuses a terminal (requires `--output`); `--output` refuses an existing file / a `.prikk/`-internal path. |
| PK-25 | **`prikk diff --from <point> --to <point> --format json`** (or a point vs the worktree) → schema **`diff-report-v1`** — per-path status, per-side blob ids/modes, unified hunks `patch(1)` applies, **declared renames only**, binary by ids/sizes, per-entry `minimal` flag | **CONFIRMED** (prikk **0.46.0**) | prikk letter 2026-09-22 (this is RFC 153). Satisfies the A4 compare ask with no new request; planeter's compare view (M2) consumes it. |
| PK-26 | **Security/format floor.** 0.44.0 fixed **GHSA-px5q-233r-6hq5** — a `bundle import` refused over a conflicting author key corrupted the receiving repo (all releases 0.23.0–0.43.0); `sync accept` had a narrower version under concurrent writes. **A forge relaying between people is exactly the exposure.** | **CONFIRMED** (prikk **0.44.0**) | prikk letter 2026-09-22. **⇒ planeter's transport floor rises to prikk ≥ 0.45.0** (GHSA fix + format 7 signature union + key-id fix). To use the read verbs PK-23/24/25 the read/hosting floor is **≥ 0.46.0**. Since 0.46.0 ⊇ these, planeter pins **≥ 0.46.0** once the local binary is upgraded (bump the pin only with the binary present, so the integration tests validate against it rather than refusing it). |

### B. Accepted/proposed in direction, not yet shipped (GATED — implementation waits on the binary)

| ID | planeter relies on | Status | Evidence / note |
|---|---|---|---|
| PK-14 | **Trusted fast-forward ref adoption** — a keyless repo publishes a received, trusted-maintainer-signed fast-forward as its own canonical branch, no re-sign | **GATED** (accepted, not shipped) | prikk **RFC 154**, accepted by prikk owner 2026-09-16. The canonical-branch/merge primitive (RFC 004/005/008). |
| PK-15 | The **repository-complete artifact** — whole-repo, offline-verifiable, verbatim, read-only export + all-or-nothing import, `import --adopt` | **GATED** (accepted, not shipped; scheduled **0.48.0 onward**) | prikk **RFC 155**, accepted 2026-09-16. Clone/serve/migrate substrate (RFC 004/009). Highest-leverage **unshipped** dependency. Its all-or-nothing import gets its own design round (0.48.0+); **R6 is already answered** by format 7's signature union (PK-6), and **0.47.0 carries a size bound for import/verify** against untrusted input — the nearest thing to **R4** shippable before RFC 155 opens (prikk letter 2026-09-22). |
| PK-16 | ~~The **key-id collision fix** in `setup`~~ → **shipped** | **CONFIRMED** (prikk **0.45.0**) | Now folded into PK-8 (`ed25519-<16hex>` default ids). No longer gated. |

*Ship order (updated 2026-09-22): key-id fix + signature union (**0.45.0, done**) → `diff`/read verbs (**0.46.0, done**) → import/verify size bound (**0.47.0**) → RFC 155 then RFC 154's adoption act (**0.48.0 onward**).*

### C. Still only assumed — the live risk list (ASSUMED / PARTIAL)

| ID | planeter assumes | Status | Risk / action |
|---|---|---|---|
| PK-17 | The `--format json` read surface is **complete enough** for a full forge browse | **CONFIRMED except blame** (residual `UD-1`) | The RFC 003 gap catalogue's asks landed in **0.46.0**: directory listing (`prikk tree`, PK-23), raw file bytes (`prikk cat`, PK-24), and compare (`prikk diff`, PK-25). **Only blame/annotate remains** (a candidate, not scheduled — M2+); that one view still renders *"pending a prikk increment"*, never faked (RFC 003 D-7). |
| PK-18 | The CLI **JSON schemas are stable enough** to drive across prikk versions | **ASSUMED** | prikk RFC 152 §5 says CLI JSON stabilizes **second**, at 1.0 — so **pre-1.0 the JSON may change**. **0.43.0 changed no `schema_version` and no format planeter reads (clean this release).** planeter pins supported prikk versions (RFC 001 D-3) and treats JSON-schema drift as a version-gated risk. **Verify per prikk release.** |
| PK-19 | prikk's **local locking is sufficient** beneath planeter's per-repo write serialization | **ASSUMED** (`UD-4`/`IQ-3`) | Confirm during the RFC 004 transport build (its handoff T6). If insufficient, raise before proceeding. |
| PK-20 | The accepted-but-unsealed queue (`sync pending`) exposes what planeter needs to render an **open change** | **CONFIRMED-ish** | prikk reply: the claims are stored objects that travel in RFC 155; review state is planeter metadata. Treat the *content* as confirmed, the *artifact carriage* as GATED on PK-15. |
| PK-21 | Driving prikk as a **subprocess** with a stable, gateable outcome is viable | **CONFIRMED** for reads (PK-1); **GATED** for the exchange/adopt verbs (RFC 155 promises JSON outcomes) | Reads work today; the adopt/artifact verbs' outcomes arrive with PK-14/PK-15. |

## Reading the ledger

- **Section A is the floor** — real today, safe to build M1 (host/browse/auth + keyless `accept`) on.
- **Section B is the gate** — the keyless canonical-branch, merge, relay and migration paths are *designed* but wait on prikk **shipping** PK-14 (RFC 154) and PK-15 (RFC 155), now scheduled **0.48.0 onward** (0.47.0 lands an import/verify size bound first). PK-16 (key-id) shipped in 0.45.0. PK-15 (RFC 155) remains the highest-leverage unshipped dependency; its all-or-nothing import (R4) gets its own design round, though **R6 is already answered** by format 7's signature union.
- **The version floor moved (2026-09-22, PK-26):** the read/hosting floor is **prikk ≥ 0.46.0** (to use `tree`/`cat`/`diff`, PK-23/24/25), which also carries the GHSA-px5q-233r-6hq5 fix (0.44.0), the format-7 signature union and the key-id fix (0.45.0). planeter bumps its version pin to 0.46.0 **once the local binary is upgraded** (the pin refuses out-of-range, so the binary must be present to keep the integration tests green). **Format 7 is a breaking, explicit-only `prikk format upgrade`** with no downgrade — a hosting decision: the serving binary must stay ≥ the format the hosted repos carry.
- **Section C is the watch list** — especially **PK-18** (pre-1.0 JSON drift) and **PK-19** (locking), which no correspondence has settled and which implementation must verify rather than assume.

Update this file whenever prikk ships, measures, or rules something a row depends on.
