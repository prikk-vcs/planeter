# Security review of the 0.8.0 work

| | |
|---|---|
| Document | The security pass before a release, for what changed since 0.7.0 (`docs/security-review-0.7.0.md` and the earlier ones stay as the records of those releases). It is a **review by the architect of the design, the code paths and the evidence**, not an external audit. |
| Date | 2026-11-21. Basis: threat model v0.8 (T-20, C-20a…i, RR-20), RFC 008 (D-1…D-11), the reviews of each part (D-91…D-104), the gates, the real-binary end-to-end look at the candidate (`release-0-8-0-candidate-logs/t7/`, including a real SIGKILL and restart) and the architect's own gates on the candidate `32494f6` (all green; 1,467 tests with and without prikk 0.58.0). |
| Verdict | **No open finding blocks 0.8.0.** One advisory against the release builder and one against the base image's package manager were found and fixed in the release chain before the tag (IS-60; apt 3.0.4); two flaky tests were found and fixed (test code only). The largest residual is unchanged and now sharper: **a repository has one set of maintainer keys and no way to change it** (RR-20). |

## How to read the evidence column
*Test* = a test that fails if the control is removed (with a **mutation** the dev team showed failing once); *gate* = a project gate; *measured* = a number from a real run; *seen* = read in the saved pages, JSON and logs of the end-to-end run.

## Surfaces

| Surface | What it defends against | Controls and evidence |
|---|---|---|
| **The archive a maintainer delivers** (T-20, C-20a, C-20b) | a branch nobody signed becoming the project's branch; a rollback or rewrite; a deleted branch; a key smuggled in by the archive | prikk imports the archive into a scratch repository that trusts **only the keys the live repository already trusts**; the written rule (one pure function) over the stage's own output; *test*: every archive of the measurement (older, divergent, unrelated, unadopted key, key merely carried, unadopted block under an adopted tip, truncated, flipped) against two forge states, the live tree **byte-identical after every refusal** (tree hash), with mutations (the stage trusting the archive's keys; the sealer check removed); a 6,000-block chain read whole, a chain that cannot be bounded refused |
| **Who may deliver** (C-20c) | a stolen token or a session moving the branch | `MergeSeal` (maintainer) and a **full-scope bearer token**; a `write` token gets 403, a cookie is not a credential on these routes, a stranger and a hidden repository get the one 404 (*test*, *seen*: `refused-write-token*.txt`); even a valid token can only deliver history an adopted key signed, as a successor |
| **The install** (C-20d) | a half-changed repository; a lost write; a rollback that loses signed history | write lease; the rule again over fresh readers; the **intent written durably before anything moves**; exchange by two plain renames (no `unsafe`; 62–84 µs, *measured*); revision counter; re-accept of every open proposal's artifacts; `verify`; *test*: a kill at every step of the engine and of the real server; *seen*: the real server killed (SIGKILL) while re-accepting, restarted on the same data, the delivery `done`, `prikk verify` exit 0, `last_delivery.new_tip` equal to the maintainer's tip; an install is finished, never rolled back |
| **Cost** (C-20f) | memory, disk, time or the write lease exhausted by one person or many | one in flight per repository, two per person, 1,024 MiB of staged archives, the archive bound (default 100 MiB, refused at the edge), the heavy-pool admission of the creation model (3.0 × + 6.0 × *measured*), a 600 s deadline, a stage that expires in a day; reads do not wait for the write lease (*test*) |
| **The forge signs nothing** (C-20g, INV-2) | the forge becoming a key holder by a code path | a source-scan test over the delivery module (no `seal`, `merge`, `branch close`, `--allow-non-fast-forward`, `--yes`, `unsafe`); the release binary has **no failpoint** (the ENF-2 script fails if the name is in it; *gate*, run by me); ENF-2 over both binaries |
| **The client's `deliver`** (C-20h) | a token or archive left behind; a surprise confirmation | private scratch (0700/0600) removed on every exit path, a lock tells the next run a killed run's directory is nobody's; https only, no redirect followed, the token never printed; no default yes; calls prikk only for `archive export` and one `log` read (source scan) |
| **What a reader sees** (C-20i) | a page claiming more than is true | "signed by a key this repository trusts and checked by prikk" (the owner's accepted sentence); the project's branch is one constant (`heads/main`); landing counts only it; a partly landed revision says so (`partly_landed`) |
| **The landed view** (T-19 extended) | a changes view that is wrong, stale or built from the wrong history | built from the sealed blocks of `heads/main` with the landing observation's own function (no second definition of landed); cache keyed by the two block ids; *test*: against `diff -r` with real prikk and real deliveries (the split revision, three patches sealed together, the root, the scattered set, the extra branch, the cache) |
| **prikk 0.58.0** (the floor) | reading a newer prikk wrongly | the floor is checked at start and by the client; the JSON the readers rely on is held by a test over 19 real documents (it fails on purpose when a shape changes); the cost models were measured again (all 77 read rows within 5 %); `unlock` is only ever `--force` |
| **The release chain** | an advisory in a build tool reaching the image | BuildKit v0.33.1 (two advisories) → v0.34.2, **digest and revision annotation checked by me at the registry**; the base image's apt 3.0.3 had **CVE-2026-109518 (DSA-6607-1)**: moved to `trixie-20261104-slim` (apt 3.0.4), **checked by me against the Debian tracker and the registry**; both moves rehearsed (runs `44872932579`, `44874110381`), the chain unchanged since |
| **The gate tools** | a verdict changed by a tool update | `cargo-deny` 0.20.2 and `cargo-audit` 0.23.0 stay pinned; the newer deny (0.22.0) gave the identical verdict on this lock |
| **Accessibility and plain words** | a maintainer who cannot read or operate the new pages and messages | the structural rules (17) over 28 saved pages in both languages after each delivery: 0 violations (the dev team's scratch run; the page test itself does not run them: IS-65); the refusals are one sentence each with what to do |

## What was not done (said plainly)
* The earlier lists still hold: **no external penetration test and no fuzzing campaign**; no second factor (RR-10); in-memory rate limits (RR-11, RR-13).
* **No person using assistive technology has tried the new pages**; no Firefox or Safari; no forced-colours check on a real system.
* **Maintainer keys cannot be added, rotated or revoked** (RR-20): the repository's trusted keys are its creator's. A lost key leaves the repository unable to take a delivery; a stolen key can deliver. This is the largest gap in the trust model and is the first item after 0.8.0.
* **Accounts, tokens and role grants have no management surface** (IS-66): they are seeded in the store; an operator has no command and a person has no page.
* **The suite may hold other load-dependent tests.** Two flakes were found in this cycle (a write-then-exec race in test stand-ins; a recovery flag asserted before the unattended recovery thread had cleared it); the evidence for the rest is clean runs at host load 12–30, not a proof.
* **The Japanese** of the 107 new messages is the owner's reading (2026-11-21), recorded as data.
* **`sync seal` cost** is prikk's: the memory grows with the claims and faster than linearly for a tree that gains a file per claim (PK-88, PK-89); prikk 0.59.0 will measure the cause. The forge never runs it.

## Findings made on the way, all closed before this review
The 413 words of the delivery route; IS-62 (a freshness record per server address told a second clone "nothing changed"); the client's own floor constant missed in the 0.58 floor move (a test now holds it); a CHANGELOG sentence that gave the wrong count for the server's start-up line; the ETXTBSY flake and the recovery-flag flake above; a first fix that put a process spawn into `src/delivery/` was stopped by the ENF-2 source test, which was not touched.

## Recorded for later
IS-61 (the client shows a refusal's reason only with `--verbose`), IS-63 (a stacked proposal loses its view), IS-64 (the client's counted messages have no singular), IS-65 (a structure test for the delivery line; four "smaller archive" hints), IS-66 (account and sign-in management).

## Residual risks accepted
RR-10…RR-20 as in the threat model; the not-done list above.
