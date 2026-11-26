#!/bin/sh
# Run a test command N times under CPU load and report which runs failed (IS-69), **within a CPU budget** (D-134).
#
#   tools/repeat-under-load.sh N [--workers W] -- CARGO TEST ARGS...
#   e.g. tools/repeat-under-load.sh 20 -- cargo test -q -p planeter-web --test admission_gate a_light_read_never_waits_behind_a_waiting_heavy_one
#
# `yes > /dev/null` workers keep some cores busy for the whole run, so a test that asserts a number of milliseconds as if it meant an
# ordering shows itself here and not in a release job. The command is run N times in a row; each run's output is kept in a scratch
# directory (printed), and the exit status is the number of failed runs (capped at 125). The workers are stopped on every exit path.
#
# **The development-machine budget.** It limits what *running the project's tests and gates* may take from a developer's computer, and says nothing about what the server uses when it serves (that is RFC 011's cost model and the server's own admission settings, such as `PLANETER_PRIKK_MEMORY_BUDGET_MB`).
# The load must not run the host at 100% (it is a person's machine, shared with other work):
#   * the default number of workers is **a quarter of the cores, at least 2 and at most 8** (`--workers W` sets it);
#   * the workers run at `nice -n 10`;
#   * **`--confine N`** (optional) puts the workers **and the command** on the same N cpus (`taskset`, the first N the process may use) and
#     runs the workers at normal priority: on a host with many cores a few niced workers leave the command idle cores and expose nothing;
#     sharing N cpus makes the command wait for a cpu, as it does in a loaded run, while everything stays within N cores of heat
#     (the cap is then `N * 100 %`). Try `--confine 4 --workers 16`.
#   * when `systemd-run --user --scope` works, the whole run (workers and the command) is placed in a scope with
#     **`CPUQuota = (workers + 4) * 100 %`**: the workers can use at most `workers` cores, and the command gets four more, so the
#     command is under load but nothing in the run can take more than the cap. Without `systemd-run` the run is not capped (it says so).
# **When to use it** (RFC 010 Amendment 1, principle 3): when a **new or changed test uses timing, ports, processes or a terminal**, or **after an
# unexplained failure**: not as a routine on every file. Use the **confined** form (`--confine 4 --workers 16`: the command and the workers share
# four cpus, so the command waits for a cpu as it does on a busy host while the heat stays within four cores) and a small number of runs (20).
# The default (a few niced workers) leaves a many-core host's other cores idle for the command and exposed nothing in the measurement; it is kept as
# it is (what a confined default would cost and find is a question of its own). Do not start it while another heavy job is running.
set -u
usage="usage: repeat-under-load.sh N [--workers W] [--confine CPUS] -- command..."
n="${1:?$usage}"
shift
cores="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
workers=$((cores / 4))
[ "$workers" -lt 2 ] && workers=2
[ "$workers" -gt 8 ] && workers=8
how="${REPEAT_UNDER_LOAD_HOW:-a quarter of $cores cores, at least 2, at most 8}"
confine=""
while [ "${1:-}" = "--workers" ] || [ "${1:-}" = "--confine" ]; do
  case "$1" in
    --workers) workers="${2:?--workers needs a number}"; [ -n "${REPEAT_UNDER_LOAD_HOW:-}" ] || how="--workers" ;;
    --confine) confine="${2:?--confine needs a number of cpus}" ;;
  esac
  shift 2
done
[ "${1:-}" = "--" ] || { echo "$usage" >&2; exit 2; }
shift
quota=$(((workers + 4) * 100))
capnote="workers $workers + 4, x 100%"
nicelevel=10
if [ -n "$confine" ]; then
  command -v taskset > /dev/null 2>&1 || { echo "--confine needs taskset" >&2; exit 2; }
  quota=$((confine * 100)); nicelevel=0; capnote="$confine cpus x 100%"
fi
# Re-run this script inside a CPU-capped user scope (once).
if [ -z "${REPEAT_UNDER_LOAD_SCOPED:-}" ]; then
  if command -v systemd-run > /dev/null 2>&1 && systemd-run --user --scope -q -p CPUQuota=100% true > /dev/null 2>&1; then
    echo "cpu cap: CPUQuota=${quota}% ($capnote) in a user scope"
    REPEAT_UNDER_LOAD_SCOPED=1 REPEAT_UNDER_LOAD_HOW="$how" exec systemd-run --user --scope -q -p "CPUQuota=${quota}%" -- "$0" "$n" --workers "$workers" ${confine:+--confine "$confine"} -- "$@"
  fi
  echo "cpu cap: none (systemd-run --user --scope is not available); the workers are limited to $workers at nice 10 only"
fi
out="$(mktemp -d "${TMPDIR:-/tmp}/repeat-under-load.XXXXXX")"
pids=""
stop() { for p in $pids; do kill "$p" 2>/dev/null; done; wait 2>/dev/null; }
trap stop EXIT INT TERM
cpus=""
if [ -n "$confine" ]; then
  # the first N cpus this process may use
  cpus="$(taskset -cp $$ 2>/dev/null | sed 's/.*: //' | tr ',' '\n' | while IFS=- read -r a b; do b="${b:-$a}"; seq "$a" "$b"; done | head -n "$confine" | paste -sd, -)"
  [ -n "$cpus" ] || { echo "--confine: could not read the allowed cpus" >&2; exit 2; }
fi
i=0
while [ "$i" -lt "$workers" ]; do
  if [ -n "$cpus" ]; then taskset -c "$cpus" nice -n "$nicelevel" yes > /dev/null & else nice -n "$nicelevel" yes > /dev/null & fi
  pids="$pids $!"; i=$((i + 1))
done
echo "load: $workers workers ($how; $cores cores) at nice $nicelevel${cpus:+, workers and command confined to cpus $cpus}; $n runs of: $*; logs in $out"
failed=0
run=1
while [ "$run" -le "$n" ]; do
  if { if [ -n "$cpus" ]; then taskset -c "$cpus" "$@"; else "$@"; fi; } > "$out/run-$run.log" 2>&1; then rm -f "$out/run-$run.log"; else failed=$((failed + 1)); echo "run $run FAILED (see $out/run-$run.log)"; fi
  run=$((run + 1))
done
echo "$failed of $n runs failed"
[ "$failed" -gt 125 ] && failed=125
exit "$failed"
