#!/bin/sh
# Run a command with the project's resource budget (D-134): memory, swap, CPU and priority are capped, so that the gates and other heavy
# jobs do not run the host at 100% or into swap.
#
#   tools/run-capped.sh [--cpu PERCENT] [--mem SIZE] [--jobs N] [--nice N] [--wait SECONDS] [--no-lock] -- COMMAND...
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
# derived value. The script prints the numbers it chose and why.
#
# **One heavy job at a time on the machine** (RFC 010 Amendment 2, the concurrency part of the budget): a job takes an exclusive `flock` on one file in
# the per-user runtime directory (`$XDG_RUNTIME_DIR`, else `$TMPDIR`, else `/tmp`; `RUN_CAPPED_LOCK_DIR` names another), and a second job **waits**,
# saying who holds the lock (the command and its age), and gives up after `--wait SECONDS` (`RUN_CAPPED_LOCK_WAIT`, default 5400) with exit status 3 and
# a sentence, having run nothing. The lock is held by the running process, not by the file's existence: it is released on every way out, a kill
# of the job included, and a file left behind means nothing (a killed *runner* leaves its command running, and the command still holds the lock). A job started inside a holder (the gates call the runner once, but a test may call it again) does not wait
# for its own parent (`RUN_CAPPED_LOCK_HELD`). `--no-lock` takes no lock and says so loudly: it is for the script's own tests. **Development machine, not the
# running service.**
#
# **The heat sampler** (RFC 010 Amendment 2): while the command runs, every `RUN_CAPPED_SAMPLE_INTERVAL` seconds (default 10; 0 turns it off) one line is
# written to `samples.tsv` in a directory of the run (beside the lock file; `RUN_CAPPED_SAMPLE_DIR` names another; its path is printed): the time, the
# 1-minute load average, the CPU temperature (`/sys/class/hwmon/*` found by sensor *name*: `k10temp`'s `Tctl`, else `coretemp`'s package, else
# `/sys/class/thermal`; none found: the file says "no temperature sensor" once and the run goes on), the hottest NVMe's temperature, and the three processes
# using the most CPU *outside* the run (by cgroup; command name only, no arguments, no paths). At the end the runner prints one block: peak and mean load and
# temperature, the seconds above **the device's own reported limit minus 15 degrees C** where the device reports one (an NVMe's `max`; `k10temp` reports none:
# then no threshold is applied and the block says so), and the names that were hot outside the run. **The block changes no default.** The exit status is the
# command's. `--sample-once FILE PID MODE` and `--report FILE` are the two halves, for the tests. With `systemd-run --user --scope` unavailable the command still runs, at that
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

# --- the heat sampler and its report (modes of this script; they run nothing of the project's)
SYSFS=${RUN_CAPPED_SYSFS:-/sys}
PROC=${RUN_CAPPED_PROC:-/proc}
PSCMD=${RUN_CAPPED_PS:-ps}

read_num() { # a file of one integer -> the integer, or failure
  v=$(cat "$1" 2> /dev/null) || return 1
  case "$v" in '' | *[!0-9-]*) return 1 ;; esac
  echo "$v"
}
milli_to_c() { awk -v m="$1" 'BEGIN { printf "%.1f", m / 1000 }'; }
milli_to_c2() { awk -v m="$1" 'BEGIN { printf "%.2f", m / 1000 }'; }

# Finds the sensors by name: sets CPU_IN, CPU_MAX, CPU_DESC, NVME_INS, NVME_LIMIT (milli-degrees, the lowest `max` an NVMe reports, or empty).
hw_scan() {
  CPU_IN='' CPU_MAX='' CPU_DESC='' NVME_INS='' NVME_LIMIT=''
  best=9
  for h in "$SYSFS"/class/hwmon/hwmon*; do
    [ -d "$h" ] || continue
    name=$(cat "$h/name" 2> /dev/null) || continue
    case "$name" in
      k10temp) rank=1 want=Tctl ;;
      coretemp) rank=2 want='Package id' ;;
      nvme)
        inp=$h/temp1_input
        for l in "$h"/temp*_label; do
          [ -r "$l" ] || continue
          [ "$(cat "$l" 2> /dev/null)" = Composite ] && inp=${l%_label}_input
        done
        read_num "$inp" > /dev/null || continue
        NVME_INS="$NVME_INS $inp"
        if m=$(read_num "${inp%_input}_max"); then
          if [ -z "$NVME_LIMIT" ] || [ "$m" -lt "$NVME_LIMIT" ]; then NVME_LIMIT=$m; fi
        fi
        continue
        ;;
      *) continue ;;
    esac
    [ "$rank" -lt "$best" ] || continue
    for l in "$h"/temp*_label; do
      [ -r "$l" ] || continue
      lab=$(cat "$l" 2> /dev/null) || continue
      case "$lab" in
        "$want"*)
          inp=${l%_label}_input
          read_num "$inp" > /dev/null || continue
          CPU_IN=$inp CPU_MAX=${l%_label}_max CPU_DESC="$name $lab" best=$rank
          break
          ;;
      esac
    done
  done
  if [ -z "$CPU_IN" ]; then
    for z in "$SYSFS"/class/thermal/thermal_zone*; do
      [ -d "$z" ] || continue
      ty=$(cat "$z/type" 2> /dev/null) || continue
      read_num "$z/temp" > /dev/null || continue
      if [ -z "$CPU_IN" ] || [ "$ty" = x86_pkg_temp ]; then CPU_IN=$z/temp CPU_MAX='' CPU_DESC="thermal $ty"; fi
    done
  fi
}

# One sample: sample_once FILE CHILDPID MODE (MODE: cgroup = the run is the child's cgroup; tree = the run is the child's process tree)
sample_once() {
  file=$1 child=$2 mode=$3
  now=${RUN_CAPPED_NOW:-$(date +%s)}
  load=$(cut -d' ' -f1 "$PROC/loadavg" 2> /dev/null) || load=''
  [ -n "$load" ] || load=-
  ct=-
  if [ -n "${CPU_IN:-}" ] && m=$(read_num "$CPU_IN"); then ct=$(milli_to_c "$m"); fi
  nt=-
  hot=''
  for inp in ${NVME_INS:-}; do
    if m=$(read_num "$inp") && { [ -z "$hot" ] || [ "$m" -gt "$hot" ]; }; then hot=$m; fi
  done
  [ -z "$hot" ] || nt=$(milli_to_c "$hot")
  runcg=''
  if [ "$mode" = cgroup ]; then runcg=$(sed -n 's/^0:://p' "$PROC/$child/cgroup" 2> /dev/null | tail -1); fi
  $PSCMD -eo pid=,ppid=,cputimes=,comm= > "$file.ps" 2> /dev/null || : > "$file.ps"
  # candidates: pid, percent of one core since the previous sample, whether the pid is in the child's tree, name; best first
  awk -v now="$now" -v child="$child" -v prev="$file.prev" '
    BEGIN { while ((getline l < prev) > 0) { n = split(l, a, " "); if (a[1] == "#") pt = a[2]; else pc[a[1]] = a[2] } }
    { pid = $1; par[pid] = $2; cpu[pid] = $3; nm = $4; for (i = 5; i <= NF; i++) nm = nm "_" $i; name[pid] = nm }
    END {
      dt = now - pt
      for (p in cpu) {
        t = 0; q = p; guard = 0
        while (q != "" && q != 0 && guard < 64) { if (q == child) { t = 1; break } q = par[q]; guard++ }
        if (dt > 0 && (p in pc)) { pct = int((cpu[p] - pc[p]) * 100 / dt); if (pct > 0) printf "%d\t%d\t%d\t%s\n", pct, p, t, name[p] }
      }
      out = prev ".new"; print "# " now > out
      for (p in cpu) print p " " cpu[p] > out
    }' "$file.ps" 2> /dev/null | sort -rn > "$file.cand"
  [ -f "$file.prev.new" ] && mv "$file.prev.new" "$file.prev"
  top=''
  k=0
  while IFS='	' read -r pct pid intree name; do
    [ "$k" -lt 3 ] || break
    if [ "$mode" = cgroup ]; then
      cg=$(sed -n 's/^0:://p' "$PROC/$pid/cgroup" 2> /dev/null | tail -1)
      case "$cg" in "$runcg" | "$runcg"/*) [ -n "$runcg" ] && continue ;; esac
    else
      [ "$intree" = 1 ] && continue
    fi
    top="$top${top:+,}$name:$pct"
    k=$((k + 1))
  done < "$file.cand"
  printf '%s\t%s\t%s\t%s\t%s\n' "$now" "$load" "$ct" "$nt" "${top:--}" >> "$file"
}

if [ "${1:-}" = "--sample-once" ]; then # FILE CHILDPID MODE (the tests; a header is written if the file is new)
  hw_scan
  sample_once "$2" "$3" "${4:-tree}"
  exit 0
fi

if [ "${1:-}" = "--sampler" ]; then # FILE CHILDPIDFILE MODE INTERVAL: runs until the runner is gone or kills it
  file=$2 pidfile=$3 mode=$4 interval=$5
  trap 'kill "$sp" 2> /dev/null; exit 0' TERM INT
  hw_scan
  {
    echo "# run-capped heat samples: time, 1-min load, CPU temp C, hottest NVMe temp C, top 3 outside the run (name:percent of one core); every $interval s"
    if [ -z "$CPU_IN" ] && [ -z "$NVME_INS" ]; then echo "# no temperature sensor"; fi
    [ -z "$CPU_IN" ] || { if [ -n "$CPU_MAX" ] && m=$(read_num "$CPU_MAX"); then echo "# cpu sensor: $CPU_DESC; limit $(milli_to_c2 "$m")"; else echo "# cpu sensor: $CPU_DESC; limit none reported"; fi; }
    [ -z "$CPU_IN" ] && [ -n "$NVME_INS" ] && echo "# no CPU temperature sensor"
    [ -z "$NVME_INS" ] || { if [ -n "$NVME_LIMIT" ]; then echo "# nvme: limit $(milli_to_c2 "$NVME_LIMIT")"; else echo "# nvme: limit none reported"; fi; }
    echo "# mode: $mode"
  } > "$file"
  while :; do
    child=$(cat "$pidfile" 2> /dev/null)
    [ -n "$child" ] && kill -0 "$child" 2> /dev/null || break
    sample_once "$file" "$child" "$mode"
    sleep "$interval" &
    sp=$!
    wait "$sp"
  done
  exit 0
fi

if [ "${1:-}" = "--report" ]; then # FILE: the block
  awk -F'\t' -v file="$2" -v hot="${RUN_CAPPED_HOT_PCT:-100}" '
    /^# every|^# run-capped heat/ { if (match($0, /every [0-9]+ s/)) iv = substr($0, RSTART + 6, RLENGTH - 8) + 0 }
    /^# cpu sensor:/ { cpu = $0; sub(/^# cpu sensor: /, "", cpu); if (match(cpu, /limit [0-9.]+/)) climit = substr(cpu, RSTART + 6, RLENGTH - 6) + 0 }
    /^# nvme: limit/ { if (match($0, /[0-9.]+$/)) nlimit = substr($0, RSTART) + 0 }
    /^# no temperature sensor/ { nosensor = 1 }
    /^#/ { next }
    NF >= 5 {
      n++; if (!t0) t0 = $1; t1 = $1
      if ($2 != "-") { ln++; ls += $2; if ($2 + 0 > lp) lp = $2 + 0 }
      if ($3 != "-") { cn++; cs += $3; if ($3 + 0 > cp) cp = $3 + 0; if (climit && $3 + 0 > climit - 15) cabove++ }
      if ($4 != "-") { nn++; ns += $4; if ($4 + 0 > np) np = $4 + 0; if (nlimit && $4 + 0 > nlimit - 15) nabove++ }
      if ($5 != "-") { m = split($5, parts, ","); for (i = 1; i <= m; i++) { c = split(parts[i], kv, ":"); pct = kv[c] + 0; nm = parts[i]; sub(/:[0-9]+$/, "", nm); if (pct >= hot) { hc[nm]++; if (pct > hp[nm]) hp[nm] = pct } } }
    }
    END {
      printf "run-capped: heat report (development machine): %d sample(s)", n
      if (n > 1) printf " over %d s", t1 - t0
      printf ", file %s\n", file
      if (n == 0) { print "run-capped:   no sample was taken (the run was shorter than one interval, or the sampler is off)"; exit }
      printf "run-capped:   load average (1 min): peak %.2f, mean %.2f\n", lp, ls / ln
      if (cn) printf "run-capped:   CPU temperature: peak %.1f C, mean %.1f C; %s\n", cp, cs / cn, (climit ? sprintf("%d s above the device limit %.2f minus 15 C", cabove * iv, climit) : "no threshold applied: the sensor reports no limit")
      else if (nosensor) print "run-capped:   no temperature sensor"
      else print "run-capped:   no CPU temperature sensor"
      if (nn) printf "run-capped:   NVMe temperature (hottest): peak %.1f C, mean %.1f C; %s\n", np, ns / nn, (nlimit ? sprintf("%d s above the device limit %.2f minus 15 C", nabove * iv, nlimit) : "no threshold applied: the device reports no limit")
      printf "run-capped:   hot outside the run (%d%% of a core or more in a sample):", hot
      c = 0; for (k in hc) { printf " %s (%d sample(s), peak %d%%)", k, hc[k], hp[k]; c++ }
      if (!c) printf " none"
      printf "\n"
    }' "$2"
  exit 0
fi

cores=$(getconf _NPROCESSORS_ONLN 2> /dev/null || nproc 2> /dev/null || echo 4)
memkib=$(sed -n 's/^MemTotal: *\([0-9]*\) kB.*/\1/p' /proc/meminfo 2> /dev/null | head -1)
memkib=${memkib:-16777216}
set -- $(derive "$cores" "$memkib") "$@"
cpu=$1 mem=$2 jobs=$3
shift 3
nice=10
lockwait=${RUN_CAPPED_LOCK_WAIT:-5400}
nolock=0
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
    --wait) lockwait="${2:?--wait needs a number of seconds}"; shift 2 ;;
    --no-lock) nolock=1; shift ;;
    *) echo "usage: run-capped.sh [--cpu PERCENT] [--mem SIZE] [--jobs N] [--nice N] [--wait SECONDS] [--no-lock] -- command..." >&2; exit 2 ;;
  esac
done
shift
[ "$#" -gt 0 ] || { echo "run-capped.sh: no command" >&2; exit 2; }
case "$lockwait" in '' | *[!0-9]*) echo "run-capped.sh: --wait wants a whole number of seconds, not '$lockwait'" >&2; exit 2 ;; esac

# --- the machine-wide lock for heavy jobs
lockdir=${RUN_CAPPED_LOCK_DIR:-${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}}
lock="$lockdir/planeter-heavy-job-$(id -u).lock"
holder_text() { # who holds it, from the note the holder wrote when it took the lock
  IFS='	' read -r hpid hat hcmd < "$lock.info" 2> /dev/null || hpid=''
  if [ -z "$hpid" ] || [ -z "$hat" ]; then echo "another heavy job (it left no description)"; return; fi
  age=$(($(date +%s) - hat))
  echo "\`$hcmd\` (pid $hpid, running $((age / 60)) min $((age % 60)) s)"
}
if [ "$nolock" = 1 ]; then
  echo "run-capped: --no-lock: NO machine-wide lock is taken; two heavy jobs may run at once and heat the machine. This is for the script's own tests, not for a heavy job." >&2
elif [ -n "${RUN_CAPPED_LOCK_HELD:-}" ] && kill -0 "$RUN_CAPPED_LOCK_HELD" 2> /dev/null; then
  :
elif ! command -v flock > /dev/null 2>&1; then
  echo "run-capped: flock is not available: NO machine-wide lock is taken (install util-linux); do not start a heavy job while another is running." >&2
else
  mkdir -p "$lockdir" 2> /dev/null
  exec 9>> "$lock" || { echo "run-capped: cannot open the lock file $lock" >&2; exit 2; }
  if ! flock -n 9; then
    who=$(holder_text)
    echo "run-capped: waiting for the heavy-job lock ($lock): it is held by $who; giving up after $lockwait s" >&2
    if ! flock -w "$lockwait" 9; then
      echo "run-capped: gave up: the heavy-job lock was still held after $lockwait s by $who. Nothing was run. Wait for it, or look at what holds it; do not start a second heavy job beside it." >&2
      exit 3
    fi
    echo "run-capped: got the lock" >&2
  fi
  printf '%s\t%s\t%.300s\n' "$$" "$(date +%s)" "$*" > "$lock.info" 2> /dev/null
  RUN_CAPPED_LOCK_HELD=$$
  export RUN_CAPPED_LOCK_HELD
fi
export CARGO_BUILD_JOBS="$jobs"
interval=${RUN_CAPPED_SAMPLE_INTERVAL:-10}
case "$interval" in '' | *[!0-9]*) echo "run-capped.sh: RUN_CAPPED_SAMPLE_INTERVAL wants a whole number of seconds" >&2; exit 2 ;; esac
if command -v systemd-run > /dev/null 2>&1 && systemd-run --user --scope -q -p CPUQuota=100% true > /dev/null 2>&1; then
  echo "run-capped: CPUQuota=${cpu}% ($cpu_why) MemoryMax=${mem} ($mem_why) MemorySwapMax=0 nice=${nice} CARGO_BUILD_JOBS=${jobs} ($jobs_why)" >&2
  mode=cgroup
  set -- systemd-run --user --scope -q -p MemoryMax="$mem" -p MemorySwapMax=0 -p "CPUQuota=${cpu}%" -- nice -n "$nice" "$@"
else
  echo "run-capped: NOT capped (systemd-run --user --scope is not available): nothing limits CPU or memory; nice=${nice} CARGO_BUILD_JOBS=${jobs} ($jobs_why)" >&2
  mode=tree
  set -- nice -n "$nice" "$@"
fi
if [ "$interval" -eq 0 ]; then exec "$@"; fi
find "$lockdir" -maxdepth 1 -name 'planeter-run-capped.*' -mtime +7 -exec rm -rf {} + 2> /dev/null
sdir=${RUN_CAPPED_SAMPLE_DIR:-}
[ -n "$sdir" ] || sdir=$(mktemp -d "$lockdir/planeter-run-capped.XXXXXX" 2> /dev/null) || sdir=''
if [ -z "$sdir" ] || ! mkdir -p "$sdir" 2> /dev/null; then exec "$@"; fi
samples="$sdir/samples.tsv"
"$@" <&0 &
child=$!
echo "$child" > "$sdir/child.pid"
sh "$0" --sampler "$samples" "$sdir/child.pid" "$mode" "$interval" 9>&- < /dev/null > /dev/null 2>&1 &
sampler=$!
trap 'kill -TERM "$child" 2> /dev/null' INT TERM
wait "$child"
status=$?
while kill -0 "$child" 2> /dev/null; do
  wait "$child"
  status=$?
done
trap - INT TERM
kill "$sampler" 2> /dev/null
wait "$sampler" 2> /dev/null
sh "$0" --report "$samples" >&2
exit "$status"
