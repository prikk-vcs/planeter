#!/bin/sh
# Run a command with the project's resource budget (D-134): memory, swap, CPU and priority are capped, so that the gates and other heavy
# jobs do not run the host at 100% or into swap.
#
#   tools/run-capped.sh [--cpu PERCENT] [--mem SIZE] [--jobs N] [--nice N] -- COMMAND...
#   e.g. tools/run-capped.sh -- sh tools/gates.sh
#        tools/run-capped.sh --cpu 400 -- cargo test --workspace --locked
#
# This is the **development-machine budget**: it limits what *running the project's tests and gates* may take from a developer's computer, and
# says nothing about what the server uses when it serves (RFC 011's cost model and the server's own admission settings, such as
# `PLANETER_PRIKK_MEMORY_BUDGET_MB`, govern that).
#
# Defaults: `--cpu 800` (`CPUQuota=800%`: eight cores' worth of time in all, however many the host has), `--mem 16G` (`MemoryMax`, with
# `MemorySwapMax=0`), `--nice 10`, and `CARGO_BUILD_JOBS` = 8 unless it is already set (or `--jobs`). With `systemd-run --user --scope`
# unavailable the command still runs, at that priority and with that `CARGO_BUILD_JOBS`, and the script says it is not capped.
# The exit status is the command's. Do not start a heavy job while another one of yours is running, and look at `uptime` first.
set -u
cpu=800 mem=16G nice=10 jobs="${CARGO_BUILD_JOBS:-8}"
while [ "${1:-}" != "--" ]; do
  case "${1:-}" in
    --cpu) cpu="${2:?--cpu needs a percentage}"; shift 2 ;;
    --mem) mem="${2:?--mem needs a size}"; shift 2 ;;
    --jobs) jobs="${2:?--jobs needs a number}"; shift 2 ;;
    --nice) nice="${2:?--nice needs a number}"; shift 2 ;;
    *) echo "usage: run-capped.sh [--cpu PERCENT] [--mem SIZE] [--jobs N] [--nice N] -- command..." >&2; exit 2 ;;
  esac
done
shift
[ "$#" -gt 0 ] || { echo "run-capped.sh: no command" >&2; exit 2; }
export CARGO_BUILD_JOBS="$jobs"
if command -v systemd-run > /dev/null 2>&1 && systemd-run --user --scope -q -p CPUQuota=100% true > /dev/null 2>&1; then
  echo "run-capped: CPUQuota=${cpu}% MemoryMax=${mem} MemorySwapMax=0 nice=${nice} CARGO_BUILD_JOBS=${jobs}" >&2
  exec systemd-run --user --scope -q -p MemoryMax="$mem" -p MemorySwapMax=0 -p "CPUQuota=${cpu}%" -- nice -n "$nice" "$@"
fi
echo "run-capped: NOT capped (systemd-run --user --scope is not available); nice=${nice} CARGO_BUILD_JOBS=${jobs}" >&2
exec nice -n "$nice" "$@"
