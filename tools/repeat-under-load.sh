#!/bin/sh
# Run a test command N times under CPU load and report which runs failed (IS-69).
#
#   tools/repeat-under-load.sh N [--workers W] -- CARGO TEST ARGS...
#   e.g. tools/repeat-under-load.sh 100 -- cargo test -q -p planeter-web --test admission_gate a_light_read_never_waits_behind_a_waiting_heavy_one
#
# One `yes > /dev/null` worker per core (or W) keeps every core busy for the whole run, so a test that asserts a number of milliseconds as if it
# meant an ordering shows itself here and not in a release job. The command is run N times in a row; each run's output is kept in a
# scratch directory (printed), and the exit status is the number of failed runs (capped at 125). The workers are stopped on every exit path.
set -u
n="${1:?usage: repeat-under-load.sh N [--workers W] -- command...}"
shift
workers="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
if [ "${1:-}" = "--workers" ]; then workers="${2:?--workers needs a number}"; shift 2; fi
[ "${1:-}" = "--" ] || { echo "usage: repeat-under-load.sh N [--workers W] -- command..." >&2; exit 2; }
shift
out="$(mktemp -d "${TMPDIR:-/tmp}/repeat-under-load.XXXXXX")"
pids=""
stop() { for p in $pids; do kill "$p" 2>/dev/null; done; wait 2>/dev/null; }
trap stop EXIT INT TERM
i=0
while [ "$i" -lt "$workers" ]; do yes > /dev/null & pids="$pids $!"; i=$((i + 1)); done
echo "load: $workers workers; $n runs of: $*; logs in $out"
failed=0
run=1
while [ "$run" -le "$n" ]; do
  if "$@" > "$out/run-$run.log" 2>&1; then rm -f "$out/run-$run.log"; else failed=$((failed + 1)); echo "run $run FAILED (see $out/run-$run.log)"; fi
  run=$((run + 1))
done
echo "$failed of $n runs failed"
[ "$failed" -gt 125 ] && failed=125
exit "$failed"
