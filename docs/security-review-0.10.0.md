# Security review of the 0.10.0 work

| | |
|---|---|
| Document | The security pass before a release, for what changed since 0.9.0 (the earlier reviews stay as the records of those releases). A **review by the architect of the design, the code paths and the evidence**, not an external audit. |
| Date | 2026-11-28. Basis: threat model v0.10 (T-22, C-22a…k, INV-12, RR-22, RR-23), RFC 014 with Amendment 1, the reviews of each part (SCHEDULE D-121…D-151), the interoperability ledger (ID-1…ID-13), my own gates on the candidate `d33ac8d` (see "My gates" below) and the dev team's release tier on it (all 17 steps, 1,697 tests in each of four runs), the candidate's end-to-end run (T7, `release-0-10-0-candidate-review-request.md`). |
| Verdict | **No open finding blocks 0.10.0.** Defects found before release by reading or by running, all fixed with a failing test first or by a stated change, are listed below; the limits are listed in "What was not done" and in RR-22 and RR-23, and are said in the release notes. |

## How to read the evidence column
*Test* = a test that fails if the control is removed (with a **mutation** the dev team showed failing once); *gate* = a project gate; *measured* = a number from a real run; *seen* = read in the saved pages, JSON and logs of the end-to-end run; *checked* = looked at by the architect.

## Surfaces

| Surface | What it defends against | Controls and evidence |
|---|---|---|
| **The operator's door** (T-22, C-22a) | an account or a token made, changed or stopped by someone who should not | no network path: the commands are for a shell on the server and open no route; they run beside the live server and are felt at the next request (*test*, *seen*: accounts beside the running server and with it stopped, a token made while it was stopped accepted at its first request) |
| **Passwords** (C-22b) | a guessable or leaked password; a password on a command line | never an argument or an environment variable; no echo or a descriptor; 12–128 after NFKC, not the handle, not a common one; Argon2id; no email reset, no email used to find anyone (*test*, mutation of the list) |
| **Handles** (C-22c) | a handle that looks like another; a reuse | one rule, case-insensitive across accounts, organizations and owners, never changed or reused; old collisions kept and reported (*test*; *seen*: a real 0.9.0 data directory with two colliding handles) |
| **Tokens** (C-22d) | a token read twice or in clear; one that never ends; one with more reach than meant | shown once, one stdout line or a new 0600 file; hash only; id not presentable; 1–365 days; scope and repository list enforced (*test*, *seen*: the 0600 mode, the second `--out` refused, a `write` token refused a delivery) |
| **Disable** (C-22e, INV-12) | access surviving a stop; an oracle for which accounts exist or are stopped | six principal paths refused at the next request (four through the real server, two in process: stated), the right password of a stopped account answers as a wrong one byte for byte, `enable` does not revive ended sign-ins (*test*, *seen*) |
| **The account's log** (C-22f) | a forged or secret-bearing record; an act without its event | typed facts, one renderer, no secret ever stored, text never markup, the event in the act's transaction (*test*; *measured*: 300 random SIGKILLs, 244 in flight, every database consistent and the server started on it) |
| **The provider path** (C-22g, C-22h) | the provider reaching an internal host; a forged identity; an email trusted unverified | the private allowance is one authority, https, pinned address, no redirects; the ID token fully verified; an email returned only when `email_verified` is exactly `true` and nothing stores it; no userinfo, no `prompt`, no `max_age`; linking only by the operator (*test* with the real `curl` and EdDSA; mutation 3 of 5; *seen*: through the running server against a stand-in) |
| **Session life and passwords off** (C-22i) | a session that outlives its welcome; a provider-only server that answers differently | `PLANETER_SESSION_HOURS` (default = the old seven days), `PLANETER_LOCAL_PASSWORDS=off` (three identical answers, hashes untouched) (*test*, *seen*) |
| **The upgrade** (C-22j) | a migration that loses or rewrites | idempotent, old columns byte-identical, a second start changes nothing (*seen* on a real 0.9.0 data directory) |
| **Availability of reads** (T-9; IS-74, IS-75) | a read, an export or a changes page refused because prikk removed a file while the store was walked | a vanished entry counts as nothing, every other error still refuses, a missing root still refuses; six and nine deterministic tests with a hook, mutations (3 of 6; 4 of 9), the digest of an unchanged store byte-identical to 0.9.0's, and through the router 380–394 of 2,000 and 231–242 of 300 refused before, **0** after. They were in 0.9.0 (by reading its code): availability, not security, in the safe direction (*test*, *measured*) |
| **What a package holds** (T-11) | a private key inside a crate that can never be taken back | the throw-away keys of `planeter-auth` and three `planeter-web` tests are kept out of the packages; a release-tier step fails if any file of any of the eleven packages is named like a key or holds one; two older literals in earlier packages are named, not hidden (*gate*, mutation: removing the exclude fails it and names the files) |
| **The gates** | a release resting on a flaky or edited check | a run is immune to an edited script (a snapshot), the review tier reads CI and generates its own sheet, a shared target is refused for any checkout that is not the run's own worktree, CI runs the tests that need prikk, the release tier runs the whole set three times with prikk and once without (*gate*, mutation of the snapshot: 14 checks fail) |
| **Dependencies and the chain** | an advisory on the lock or the image | no third-party crate changed since 0.9.0 (one new edge, `icu_normalizer`, already in the lock); `cargo deny` and `cargo audit` 0 with no ignore against a database of 1,571 advisories; no pin moved; the chain unchanged since 0.9.0's rehearsals (*gate*, *checked*) |
| **prikk 0.59.0** | an upgrade of prikk breaking planeter silently | the floor stays 0.58.0; the whole workspace's tests passed against 0.59.0 (1,697, my run); the `tag create` old-tip case does not touch planeter (it creates no tag); the `lock conflict` substring it reads is unchanged; a new `doctor` flag is outside the four-flag allow-list (*measured*, *checked*) |

## What was not done (said plainly)
* **No external penetration test and no fuzzing campaign**; no second factor (RR-10); rate limits in memory (RR-11, RR-13).
* **A sign-in with the real sui-id through a running planeter server was not done**: the running-server run used a stand-in of ours; the sui-id run was in process (RR-23; ID-11). The page says so.
* **Disabling a person at the provider does not end their planeter sign-in** (RR-22; the owner's open question 7).
* **No page for a person's own log, tokens or password** (step 2): the log is read by the operator only.
* **Two of the six principal paths (an SSH key, a deploy key) have no entry in the running server**; they are shown in process only. The account commands have no failpoints (each is one transaction); 300 random kills stand in for them.
* **The upgrade was shown on a seeded 0.9.0 database**, not one made by use (0.9.0 had no command to make accounts or tokens).
* **No person using assistive technology has tried the new sign-in-page state** (the one with passwords off); it uses the same elements as every notice, and the structural rules pass.
* **The 1,000-repository figure is for tiny repositories** (3.5 KB each).
* **The Japanese** of the 160 new messages was read by the architect, an AI assistant that is **not a native speaker**, on the owner's delegation; 35 messages and three signed ones were corrected; the owner's six earlier readings stand. A native reader's correction is a one-line fix.
* **`gh attestation verify` of the rehearsal images and the image run on a host were not done** (no docker here; the workflow's own run is the evidence).

## Findings made on the way, all closed before this review
*Found by the architect, by reading:* the first handoffs left a person with no way to get a token (step 1 gained the token commands); the server-stopped rule was wrongly copied to the account commands; a cap of 16 contradicted creation's 64; a token event stored English prose (now typed facts); the allowance for a private issuer had ignored the port. *Found by running:* five tests that read an order off a clock, two that were load-sensitive, a gate script edited under a running gate; **the store-size race in the running service** (IS-74) and its two siblings (IS-75), found because CI began to run the tests that need prikk; the private keys in a published package (found in the candidate's review). *Found in the Japanese:* ten messages that said *linked* in a different word from the signed page, five that said a role was *taken away* in a punishing one, idioms and literal English (38 messages, `corrections.tsv`).

## Recorded for later
Step 2 (self-service pages) and step 3 (provisioning, collaborators) of RFC 014; RFC 015 (organizations' lifecycle); a private certificate authority for the provider (RFC 014 open question 4); the periodic check of a person at the provider (open question 7); the run of T7(d) with the real sui-id through a running server, and a re-run of the nine checks on sui-id 0.95.0 (ID-13); RFC 013 D-3 by prikk's statements (Amendment 2; three questions to prikk); the floor move to the prikk release that ships them; the `sync seal` re-measure on 0.59.0 (PK-88, PK-94); IS-61, IS-63, IS-64, IS-65 (its second part); IS-71 (the test suite's cost); the two older key literals in earlier packages (the `known` list of the `package-keys` step).

## Residual risks accepted
RR-10…RR-23 as in the threat model; the not-done list above.
