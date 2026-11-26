#!/bin/sh
# Tests of tools/run-capped.sh: the derivation of the limits for machines that are not this one, the overrides, and the path without
# systemd-run. No build, no cap, nothing heavy: `sh tools/tests/run-capped.sh` (run from `cargo test` by tools/pin-check/tests/shell_tools.rs).
set -u
repo=$(cd "$(dirname "$0")/../.." && pwd)
rc="$repo/tools/run-capped.sh"
fails=0
check() { # name expected got
  if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected [$2] got [$3]"; fails=$((fails + 1)); fi
}
gib() { echo $(($1 * 1024 * 1024)); }

# --- the derivation: CORES, MEMORY -> "CPU MEM JOBS" (cores' worth/8 in [200, 800]; min(16 GiB, 40%); cores/4 in [2, 8])
for row in \
  "4 8 200 3276M 2" "4 16 200 6553M 2" "4 64 200 16G 2" \
  "8 8 200 3276M 2" "8 16 200 6553M 2" "8 64 200 16G 2" \
  "16 8 200 3276M 4" "16 16 200 6553M 4" "16 64 200 16G 4" \
  "32 8 400 3276M 8" "32 16 400 6553M 8" "32 64 400 16G 8" \
  "64 8 800 3276M 8" "64 16 800 6553M 8" "64 64 800 16G 8" \
  "2 4 200 1638M 2" "128 256 800 16G 8"; do
  set -- $row
  check "derive $1 cores, $2 GiB" "$3 $4 $5" "$(sh "$rc" --derive "$1" "$(gib "$2")")"
done
check "derive needs two numbers" "2" "$(sh "$rc" --derive 4 > /dev/null 2>&1; echo $?)"

# --- the overrides, with a stand-in systemd-run that records its arguments and then runs the command it was given
tmp=$(mktemp -d "${TMPDIR:-/tmp}/run-capped-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin"
cat > "$tmp/bin/systemd-run" <<'STUB'
#!/bin/sh
# probe: `--user --scope -q -p CPUQuota=100% true`
for a; do [ "$a" = "CPUQuota=100%" ] && exit 0; done
echo "$@" > "$STUB_LOG"
while [ "$1" != "--" ]; do shift; done
shift
exec "$@"
STUB
chmod +x "$tmp/bin/systemd-run"
run() { # env assignments are in the caller; prints the recorded systemd-run line
  : > "$tmp/log"
  STUB_LOG="$tmp/log" PATH="$tmp/bin:$PATH" "$@" > /dev/null 2> "$tmp/err"
  cat "$tmp/log"
}
cores_here=$(getconf _NPROCESSORS_ONLN)
line=$(run env -u RUN_CAPPED_CPU -u RUN_CAPPED_MEM -u RUN_CAPPED_JOBS -u RUN_CAPPED_NICE -u CARGO_BUILD_JOBS sh "$rc" -- true)
want=$(sh "$rc" --derive "$cores_here" "$(sed -n 's/^MemTotal: *\([0-9]*\) kB.*/\1/p' /proc/meminfo)" | cut -d' ' -f1)
check "the default on this host ($cores_here cores) is the derived quota" "yes" "$(case "$line" in *"CPUQuota=${want}%"*) echo yes ;; *) echo no ;; esac)"
check "the numbers and the reasons are printed" "yes" "$(grep -q 'CPUQuota=.*% (an eighth of' "$tmp/err" && grep -q 'MemoryMax=.* (the smaller of 16 GiB and 40%' "$tmp/err" && grep -q 'CARGO_BUILD_JOBS=.* (cores / 4' "$tmp/err" && echo yes || echo no)"
line=$(run env RUN_CAPPED_CPU=123 RUN_CAPPED_MEM=3G RUN_CAPPED_JOBS=5 RUN_CAPPED_NICE=7 sh "$rc" -- true)
case "$line" in *"CPUQuota=123%"*"MemoryMax=3G"*) r=yes ;; *"MemoryMax=3G"*"CPUQuota=123%"*) r=yes ;; *) r=no ;; esac
check "the RUN_CAPPED_* variables override" "yes" "$r"
check "the nice override" "yes" "$(case "$line" in *"nice -n 7"*) echo yes ;; *) echo no ;; esac)"
check "the jobs override reaches the command" "5" "$(STUB_LOG="$tmp/log" PATH="$tmp/bin:$PATH" RUN_CAPPED_JOBS=5 sh "$rc" -- sh -c 'echo $CARGO_BUILD_JOBS' 2> /dev/null)"
line=$(run env RUN_CAPPED_CPU=123 sh "$rc" --cpu 456 -- true)
check "a flag beats the variable" "yes" "$(case "$line" in *"CPUQuota=456%"*) echo yes ;; *) echo no ;; esac)"
check "CARGO_BUILD_JOBS, if set, is kept" "3" "$(STUB_LOG="$tmp/log" PATH="$tmp/bin:$PATH" CARGO_BUILD_JOBS=3 sh "$rc" -- sh -c 'echo $CARGO_BUILD_JOBS' 2> /dev/null)"
check "MemorySwapMax=0 is always set" "yes" "$(case "$(run sh "$rc" -- true)" in *"MemorySwapMax=0"*) echo yes ;; *) echo no ;; esac)"
check "the exit status is the command's" "9" "$(STUB_LOG="$tmp/log" PATH="$tmp/bin:$PATH" sh "$rc" -- sh -c 'exit 9' > /dev/null 2>&1; echo $?)"

# --- without systemd-run: a PATH with only what the script uses
mkdir "$tmp/nobin"
for t in sh nice getconf sed head echo env cat; do p=$(command -v "$t") && [ -x "$p" ] && ln -sf "$p" "$tmp/nobin/$t"; done
out=$(PATH="$tmp/nobin" RUN_CAPPED_JOBS=6 /bin/sh "$rc" -- sh -c 'echo jobs=$CARGO_BUILD_JOBS' 2> "$tmp/err2")
check "without systemd-run the command still runs, with the jobs" "jobs=6" "$out"
check "without systemd-run it says it is not capped" "yes" "$(grep -q 'NOT capped' "$tmp/err2" && echo yes || echo no)"

[ "$fails" -eq 0 ] && echo "all run-capped tests passed" && exit 0
echo "$fails failed"
exit 1
