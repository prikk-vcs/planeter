#!/bin/sh
# Run a command with the project's resource budget (D-134): memory, swap, CPU and priority are capped, so that the gates and other heavy
# jobs do not run the host at 100% or into swap.
#
#   tools/run-capped.sh [--cpu PERCENT] [--mem SIZE] [--jobs N] [--nice N] -- COMMAND...
#   tools/run-capped.sh --derive CORES MEMTOTAL_KIB     (prints "CPU MEM JOBS" for a machine of that size, and runs nothing)
#   e.g. tools/gates.sh --tier review            (which runs itself inside this script)
#        tools/run-capped.sh --cpu 200 -- cargo test --workspace --locked
#
# This is the **development-machine budget**: it limits what *running the project's tests and gates* may take from a developer's computer, and
# says nothing about what the server uses when it serves (RFC 011's cost model and the server's own admission settings, such as
# `PLANETER_PRIKK_MEMORY_BUDGET_MB`, govern that).
#
# **The limits are derived from the machine** (RFC 010 Amendment 1; the measurement of the gates showed that a 400 % quota lengthens the compile
# steps of a 32-core host by under a minute, against a gate run whose wall is mostly waiting tests, and halves their peak heat):
#   * CPU quota  = an eighth of the cores' worth, at least 200 % and at most 800 %   (32 cores -> 400 %, 16 -> 200 %, 8 -> 200 %, 64 -> 800 %)
#   * memory     = the smaller of 16 GiB and 40 % of MemTotal, with MemorySwapMax=0     (59 GiB -> 16G, 16 GiB -> 6553M, 8 GiB -> 3276M)
#   * build jobs = cores / 4 between 2 and 8                                           (32 cores -> 8, 16 -> 4, 8 -> 2)
#   * nice 10
# Each can be set by a flag (`--cpu`, `--mem`, `--jobs`, `--nice`) or by an environment variable (`RUN_CAPPED_CPU`, `RUN_CAPPED_MEM`,
# `RUN_CAPPED_JOBS`, `RUN_CAPPED_NICE`; `CARGO_BUILD_JOBS`, if set, is kept as the jobs); a flag beats the variable, the variable beats the
# derived value. The script prints the numbers it chose and why. With `systemd-run --user --scope` unavailable the command still runs, at that
# priority and with that job count, and the script says it is **not** capped. The exit status is the command's. Do not start a heavy job while
# another one of yours is running, and look at `uptime` first.
set -u

# derive CORES MEMTOTAL_KIB -> "CPU MEM JOBS" (a pure function of its two arguments: the tests call it with machines that are not this one).
derive() {
  cores=$1 memkib=$2
  cpu=$((cores * 100 / 8))
  [ "$cpu" -lt 200 ] && cpu=200
  [ "$cpu" -gt 800 ] && cpu=800
  memmib=$((memkib * 40 / 100 / 1024))
  if [ "$memmib" -ge 16384 ]; then mem=16G; else mem="${memmib}M"; fi
  jobs=$((cores / 4))
  [ "$jobs" -lt 2 ] && jobs=2
  [ "$jobs" -gt 8 ] && jobs=8
  echo "$cpu $mem $jobs"
}

if [ "${1:-}" = "--derive" ]; then
  [ "$#" -eq 3 ] || { echo "usage: run-capped.sh --derive CORES MEMTOTAL_KIB" >&2; exit 2; }
  derive "$2" "$3"
  exit 0
fi

cores=$(getconf _NPROCESSORS_ONLN 2> /dev/null || nproc 2> /dev/null || echo 4)
memkib=$(sed -n 's/^MemTotal: *\([0-9]*\) kB.*/\1/p' /proc/meminfo 2> /dev/null | head -1)
memkib=${memkib:-16777216}
set -- $(derive "$cores" "$memkib") "$@"
cpu=$1 mem=$2 jobs=$3
shift 3
nice=10
cpu_why="an eighth of $cores cores' worth, between 200 and 800"
mem_why="the smaller of 16 GiB and 40% of $((memkib / 1024 / 1024)) GiB"
jobs_why="cores / 4 ($cores), between 2 and 8"
if [ -n "${RUN_CAPPED_CPU:-}" ]; then cpu=$RUN_CAPPED_CPU cpu_why="RUN_CAPPED_CPU"; fi
if [ -n "${RUN_CAPPED_MEM:-}" ]; then mem=$RUN_CAPPED_MEM mem_why="RUN_CAPPED_MEM"; fi
if [ -n "${CARGO_BUILD_JOBS:-}" ]; then jobs=$CARGO_BUILD_JOBS jobs_why="CARGO_BUILD_JOBS"; fi
if [ -n "${RUN_CAPPED_JOBS:-}" ]; then jobs=$RUN_CAPPED_JOBS jobs_why="RUN_CAPPED_JOBS"; fi
if [ -n "${RUN_CAPPED_NICE:-}" ]; then nice=$RUN_CAPPED_NICE; fi
while [ "${1:-}" != "--" ]; do
  case "${1:-}" in
    --cpu) cpu="${2:?--cpu needs a percentage}"; cpu_why="--cpu"; shift 2 ;;
    --mem) mem="${2:?--mem needs a size}"; mem_why="--mem"; shift 2 ;;
    --jobs) jobs="${2:?--jobs needs a number}"; jobs_why="--jobs"; shift 2 ;;
    --nice) nice="${2:?--nice needs a number}"; shift 2 ;;
    *) echo "usage: run-capped.sh [--cpu PERCENT] [--mem SIZE] [--jobs N] [--nice N] -- command..." >&2; exit 2 ;;
  esac
done
shift
[ "$#" -gt 0 ] || { echo "run-capped.sh: no command" >&2; exit 2; }
export CARGO_BUILD_JOBS="$jobs"
if command -v systemd-run > /dev/null 2>&1 && systemd-run --user --scope -q -p CPUQuota=100% true > /dev/null 2>&1; then
  echo "run-capped: CPUQuota=${cpu}% ($cpu_why) MemoryMax=${mem} ($mem_why) MemorySwapMax=0 nice=${nice} CARGO_BUILD_JOBS=${jobs} ($jobs_why)" >&2
  exec systemd-run --user --scope -q -p MemoryMax="$mem" -p MemorySwapMax=0 -p "CPUQuota=${cpu}%" -- nice -n "$nice" "$@"
fi
echo "run-capped: NOT capped (systemd-run --user --scope is not available): nothing limits CPU or memory; nice=${nice} CARGO_BUILD_JOBS=${jobs} ($jobs_why)" >&2
exec nice -n "$nice" "$@"
