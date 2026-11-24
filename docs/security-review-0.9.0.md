# Security review of the 0.9.0 work

| | |
|---|---|
| Document | The security pass before a release, for what changed since 0.8.0 (the earlier reviews stay as the records of those releases). A **review by the architect of the design, the code paths and the evidence**, not an external audit. |
| Date | 2026-11-24. Basis: threat model v0.9 (T-21, C-21a…h, RR-21; RR-20 corrected), RFC 013 (D-1, D-2, D-4, D-5, D-6), the reviews of each part (SCHEDULE D-110…D-120), my own gates on the candidate `00d128d` (all green; 1,520 tests with and without prikk 0.58.0), the dev team's run of the release binaries (`release-0-9-0-candidate-logs/t7/`: an upgrade from a real 0.8.0 data directory, the operator's command with kills, a 1,000-repository pass), and the rehearsal `45375363159` of the chain. |
| Verdict | **No open finding blocks 0.9.0.** One defect of the new feature was found by reading before release and fixed (a key trusted since creation could have been recorded as "added by the operator": false provenance), one hole in the architect's own handoff was found by the dev team and closed (a kill between the add and the record could have let the start-up pass label the new key "creation"), and one product defect found while fixing a test was fixed (the server said "listening" before it bound the socket). The largest residual is unchanged: **a stolen maintainer key cannot be withdrawn** (RR-20); what changed is that a lost key no longer leaves a repository without a way forward. |

## How to read the evidence column
*Test* = a test that fails if the control is removed (with a **mutation** the dev team showed failing once); *gate* = a project gate; *measured* = a number from a real run; *seen* = read in the saved pages, JSON and logs of the end-to-end run; *checked* = looked up by the architect at the source (registry, Debian tracker).

## Surfaces

| Surface | What it defends against | Controls and evidence |
|---|---|---|
| **Adding a key** (T-21, C-21a) | a key added by someone who should not | **no network path**: `planeter operator key add` needs the server's data and the server stopped (the server's own process lock; beside a live server: exit 3, nothing changed: *test*, *seen*); a delivery cannot carry a key (the stage trusts the live keys only: *test*, *seen*: the delivery refused before the add, accepted after); a stolen token or session reaches nothing here |
| **Never removing a key** (C-21b) | breaking a repository's own `verify` | a source scan over every crate and tool and a driver-method test, each with a shown mutation (*test*); a word test over catalogs and rendered pages (*test*); the reason is measured on prikk 0.58.0 (PK-90) and confirmed by prikk (PK-91) |
| **The key id** (C-21c) | a wrong id listing the same key twice; a hostile archive carrying one | derived and compared in the command and in creation (*test*, mutation); recorded as prikk's gap (PK-93), to be raised with prikk |
| **What a page says about a key** (C-21d, C-21e) | a false origin; markup or disguised text in a reason | the table says only how a key came, prikk's list is the truth; "Source not recorded" for what was not recorded; the operator's completion only where rows exist (*test*, found by review, shown failing first); a hostile reason (`<script>`, U+202E) is text and visible escapes in both languages (*test*, *seen*) |
| **Interrupted acts** (C-21f) | a half-done add; a mislabelled key after a kill | write lease whose record is the intent; the older keys' record before the add; verify after the add; five failpoints killed with the real server's recovery and the re-run (*test*); a release-binary kill (*seen*); mutations on lock, lease, verify, record, bound and the older-keys record; failpoints absent from the release binary (*gate*: ENF-2; *checked*: `strings` finds none) |
| **The background pass** | a start that waits; a pass that changes a repository | runs after the server listens, one repository at a time through the prikk gate, never changes a repository (a test hashes the tree), skips a flagged one, logs counts only (*test*, *seen*): 1,000 tiny repositories in 8.5 s, no non-200 meanwhile; killed at 303 of 1,000 and restarted: the rest recorded, a third start does nothing (*measured*); the upgrade from a real 0.8.0 data directory (*seen*) |
| **Cost** (C-21g) | many keys making the repository slow | one bound of 64 by every path, from measurement (PK-92); the creation page tells how many slots remain |
| **The listening line** (IS-70) | a supervisor believing the server is up; a false "listening" after a failed bind | bind first, print after; a taken port: one honest sentence, exit 1, the lock gone (*test*, *seen*, mutation: 20 of 20 loaded runs fail with the old order) |
| **The release chain** | an advisory in a package of the image; a race failing a release | the base image moved to `trixie-20261123-slim` for zlib CVE-2026-85091 and CVE-2026-27171 and glibc 2.41-12+deb13u5 (*checked*: the three tags resolve to the digest in the Dockerfile; the Debian tracker lists both zlib CVEs fixed for trixie in the new version); the glibc step no longer pipes `docker` into `head` and a pin-check rule holds it (*test*); rehearsal `45375363159` read by me, green on both architectures; the chain unchanged since (`git diff 7e23768..00d128d -- .github Dockerfile` empty) |
| **Dependencies** | an advisory on the lock | `url` 2.5.9 with `form_urlencoded` 1.2.3 (RUSTSEC-2026-0592, not reachable: no `origin()` call anywhere; so no 0.8.1); `cargo deny` and `cargo audit` 0 with no ignore against a database of 1,566 advisories (*gate*) |
| **The tests themselves** | a release failing, or a real bug hiding, behind a flake | sixteen wall-clock assertions now state orderings, guards raised, the last two load-sensitive tests fixed, a repeat-under-load script; the architect's own repeated runs under load were clean (*gate*) |

## What was not done (said plainly)
* The earlier lists still hold: **no external penetration test and no fuzzing campaign**; no second factor (RR-10); in-memory rate limits (RR-11, RR-13).
* **A key cannot be removed, retired or revoked** (RR-20): a stolen key stays trusted for the repository's life, and each added key uses one of 64 slots for good (RR-21). prikk's trust design round (0.59.0, a build no earlier than 0.60.0) is where this may change; RFC 013 D-3 (adding a key through a delivery) is gated on it and not built.
* **Accounts, tokens and role grants still have no management surface** (IS-66; RFC 014, planned). The operator's command covers keys only.
* **No person using assistive technology has tried the new key list**; 320 px was not measured in a browser; the structural rules pass over every state including 64 keys and a hostile reason.
* **The 1,000-repository figure is for tiny repositories** (3.5 KB each) on a host near load 30; a real repository costs a prikk call that scales with its trust list, not measured at size.
* **The operator's command was killed in the release binary at a time chosen, not at every point**: the five points are the debug build's failpoints.
* **`gh attestation verify` of the rehearsal images and the image run on a host were not done** (no docker here; the workflow's own run is the evidence).
* **The client's `1 changes`** (IS-64) and the missing singular of its counted messages remain; a macOS runner job was cancelled on two commits (a runner-pool matter, to be watched at the tag run).
* **The Japanese** of the 37 new messages is `<owner's reading>`.

## Findings made on the way, all closed before this review
A key trusted since creation recorded as "added by the operator" in a repository with no record (the architect, by reading; fixed test-first); the start-up pass could have labelled a new key "creation" after a kill (the dev team); the creation page's "Adopting all 1 leaves 63" (the architect); `url` 2.5.8's advisory arriving after the 0.8.0 check (the architect's gates); a glibc step failing by a broken pipe (the 0.8.0 tag run); the server saying "listening" before it bound (the dev team); a brief of the architect's that said the stop line now prints counters also without a spool, which the dev team measured and corrected (what changed is the key-record counters line).

## Recorded for later
IS-61, IS-63, IS-64, IS-65 (its second part), IS-66, the trust design of prikk 0.59.0 (D-3's gate), the 0.59.0 re-baseline and `sync seal` re-measure (PK-88).

## Residual risks accepted
RR-10…RR-21 as in the threat model; the not-done list above.
