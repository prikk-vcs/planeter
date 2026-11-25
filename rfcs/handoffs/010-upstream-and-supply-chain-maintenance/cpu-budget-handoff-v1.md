# RFC 010 — The project's own CPU budget: gates and loaded repeats must not run the machine at 100% — Implementation Handoff (v1)

| | |
|---|---|
| Document | The owner asked on 2026-11-25 whether planeter's CPU use is optimized and said the machine is near 100% almost all the time, with a concern for overheating. The architect looked (`ps`): **part of it is ours.** `tools/repeat-under-load.sh` (IS-69) starts **one `yes` busy worker per core, 32 of them**, for the whole run (20 to 100 repeats), on top of cargo builds and a test suite that runs one thread per core; the architect's own gates called it. The rest at that moment was other projects' work (prikk probes, brygge, mimlys, cesauth), which is not ours to touch. The loaded repeats exist to expose tests that fail under load; they do not need every core burning for minutes. This handoff gives the project's scripts a **CPU budget** and the documents one paragraph that says how to run the gates without cooking the host. |
| Status | Assigned 2026-11-25 (SCHEDULE D-134). Small; after the provider handoff. |
| Basis | `tools/repeat-under-load.sh`; the dev team's own `scripts/gates.sh`/`run-gates.sh` pattern (a `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0` wrapper and `CARGO_BUILD_JOBS=8`); `docs/RELEASING.md` (the gates); the architect's measurement: a CPU quota on a user scope **works** on this host (four busy loops under `CPUQuota=200%` used 200.9% in total). |
| Audience | Dev team. Return a short review-request package. |

## Change scope
1. **`tools/repeat-under-load.sh`:** the default number of busy workers becomes **a quarter of the cores, at least 2 and at most 8** (`--workers W` still sets it, and the script prints the number it used and the core count); the workers run at **`nice -n 10`**; and when `systemd-run` is available the **whole run (workers and the command) is placed in a user scope with `-p CPUQuota=<workers+4>00%`** (say the formula you choose and why) so that it cannot take more than that, with a clear message and the plain behaviour when `systemd-run` is absent. The exit status and the log handling are unchanged. **Show that the script still does its job**: a test known to be sensitive to load (the old `a_light_read_never_waits_behind_a_waiting_heavy_one` assertion, or the deliberate-early-write helper from IS-69's evidence) fails under the new default load and passes without it, **or say plainly that it does not and what the number of workers must be to expose it** (the point of the script is to catch load flakes: the budget must not make it blind).
2. **A `scripts/run-capped.sh` (or the same logic in the existing gates wrapper)** that runs any command in a scope with `MemoryMax`, `MemorySwapMax=0` **and `CPUQuota` (default 800%, `--cpu` to change)** at `nice -n 10`, with `CARGO_BUILD_JOBS` defaulting to 8; the dev team's gate scripts and the architect's use it. Report how long the full gates take with and without the cap (the cap may lengthen them: say by how much).
3. **`docs/RELEASING.md`:** one short paragraph under the gates: how to run them capped (the wrapper, the numbers), that loaded repeats use the new defaults, that heavy jobs are not run at the same time as other heavy jobs, and why (the host's heat and the other projects on it).
4. **Do not change** what any test asserts, any product code, or the number of tests run.

## Non-change scope
No product code, no test assertion, no CI workflow (CI runners are not our host), no new dependency. Do not touch `docs/SCHEDULE.md`, `docs/STATUS.md`, the ledger, the threat model. No tag, release or upload.

## Acceptance criteria
The load script's default uses at most 8 workers at low priority inside a CPU cap and still exposes a known load-sensitive test (or says exactly what it needs to); the wrapper exists and caps CPU and memory; the paragraph is in `RELEASING.md`; gates green; CI green on all eight jobs.

## Required evidence
Commits (hash → what); the script's output showing the numbers it chose; the sensitivity check; the full-gates timings capped and uncapped; the wrapper's use; the gates in a clean worktree **run through the wrapper** (fmt, floor, pin, clippy, tests with prikk three times in a row and without, `cargo deny`, `cargo audit`, ENF-2, doc, msrv); deviations and open questions.
