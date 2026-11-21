# Handoff: 0.8.0 — one flaky test, one sentence, then the tag commit (SCHEDULE D-103)

Your candidate `17e7aae` is approved except for one thing found by my own gates: `delivery::tests::a_chain_that_cannot_be_bounded_is_none_not_a_prefix` failed once with `Spawn(Text file busy)` (ETXTBSY) in the run with prikk on PATH (1,466 passed, 1 failed); the run without prikk passed 1,467. Your run did not show it; mine did under the load of the whole workspace. Details: `.git-exclude/reviewed/release-0-8-0-candidate-review-v1.md`.

## F1 — the flaky helper (required)
`standin()` in `crates/planeter-transport/src/delivery/mod.rs` (≈ line 2031) writes a script and the test runs it immediately. Give it the wait the project already uses (`crates/planeter-transport/tests/proposals_unit.rs:80`: run `--version` up to 200 times, sleep 10 ms on `raw_os_error() == Some(26)`, panic on any other error). Then **audit every write-then-exec helper that this release added or changed** (`git diff 0.7.0..HEAD` for `set_permissions(… 0o755)` / `from_mode(0o7`): `delivery/mod.rs`, `planeter-transport/tests/delivery_real.rs`, `planeter-web/tests/deliveries_http.rs`, `planeter-web/tests/changes_landed.rs`, `crates/planeter/tests/*`, `planeter-client/tests/deliver.rs` and `src/commands/deliver.rs`. For each: does it wait (or is it written once and only then executed by every test, or never executed while another test thread forks)? Fix those that do not, with the same pattern, in one small helper if there are several. Report the table (file, helper, before, after). Test code only.

## F2 — one CHANGELOG sentence (Q1)
In *Upgrading from 0.7.0*, "Why the floor moved", replace "`sync seal`, the maintainer's side, reads about 23 KB a claim instead of growing with the history" with the measured comparison: "`sync seal`, the maintainer's side, read 5.5 GB for 1,000 claims on 0.57.0 and reads 23 MB on 0.58.0". Nothing else about speed or memory.

## Rules
No catalog text, no `locales/ja.signed`, no baseline, no `SignOff` change (the owner reads the Japanese from your reading order now; a catalog edit would make the sheet stale). Do not tag, publish or upload. Do not touch SCHEDULE, STATUS, ledger, threat model.

## Evidence I will check
A new candidate commit (F1 and F2 only; `git diff 17e7aae..<new> --stat` shows test files and CHANGELOG.md), **CI green on all eight jobs**, and for the flake: with prikk on PATH, the workspace test run **3 times in a row** under the memory scope with no failure, plus the audit table. I run my own gates again.

## After I approve and the owner authorizes (do not start before)
One docs-only tag commit (date the CHANGELOG heading, fill the RELEASING 0.8.0 row), as for 0.6.0 and 0.7.0.
