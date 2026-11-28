#!/bin/sh
# The project's gates, in three tiers (RFC 010 Amendment 1, D-138/D-140): one script, so that nobody keeps a copy.
#
#   tools/gates.sh --tier commit  [--fail-fast]
#   tools/gates.sh --tier review  [--commit REV] [--base REV] [--runs N] [--sheet FILE] [--no-ci] [--fail-fast]
#   tools/gates.sh --tier release --sheet FILE [--commit REV] [--runs N] [--no-ci] [--fail-fast]     (--sheet is REQUIRED in the release tier)
#   tools/gates.sh --tier T --plan          (print what the tier would run, and stop)
#
# **Every tier** runs inside `tools/run-capped.sh` (the development-machine budget: it limits what running the gates takes from a developer's
# computer, and says nothing about what the server uses when it serves), with the limits that script derives from the machine and prints; uses
# **the compiler the release uses**, read from the pin in `.github/workflows/ci.yml` that `planeter-pin-check` reads (the toolchain that is not
# the `rust-version` one), through `RUSTUP_TOOLCHAIN`: if it is not installed the script says so, names the `rustup toolchain install` command
# and stops, **never falling back to `stable`**; prints the compiler version and the step list first; and ends with a table of every step's
# exit status and wall time (and CPU-seconds, read from the scope's `cpu.stat` as a difference around each step, and the scope's `memory.peak`
# so far, when cgroup v2 allows). Its exit status is the first failing step's, after the cleanup.
#
#   commit   a developer before pushing, **where the developer is** (no worktree, incremental builds on): fmt, floor-check, pin-check, clippy,
#            and `cargo test -p …` for the crates the diff touches (the whole workspace when `Cargo.toml`, `Cargo.lock`, `tools/`, `.github/`
#            or the `Dockerfile` changed). The crates' dependents are not run here: CI runs the whole workspace on the pushed commit.
#   review   the architect, or a team member, on someone else's commit: **reads CI for that exact commit first** (`gh`) and refuses to go on
#            unless it is complete and green (`--no-ci` says so loudly), then runs what CI cannot: the workspace tests **with prikk** once
#            (`--runs N` for more), cargo-deny and cargo-audit on a **freshly fetched** advisory database (the tools pinned as the workflows
#            pin them, installed once per machine into `.git-exclude/tmp/gate-tools/`), the sheet step (**it needs no file**: it generates a Japanese
#            sheet from the worktree's live catalogs into the run's scratch and checks it; `--sheet FILE` ADDS a check of that file, reported as "the
#            supplied sheet does not match the catalogs" and never as a failing step of the commit), and what the change touches (the diff of
#            `--base`, default `REV^`, to `REV`): `doc` when documents, public items or a `Cargo.toml` changed; the 1.89.0 check and the publish
#            dry run when a `Cargo.toml`, `Cargo.lock` or `rust-version` changed; ENF-2 and pin-check when `.github/`, the `Dockerfile`,
#            `tools/` or the manifests changed. One shared `CARGO_TARGET_DIR` keyed by compiler and `Cargo.lock`, `CARGO_INCREMENTAL=0` (what CI
#            sets); **the shared target is for this script's own fresh worktrees only** (see below). **A review's green is not a release's evidence.**
#   release  a release candidate and the tag commit: **a clean worktree and an empty target, always** (a `CARGO_TARGET_DIR` in the
#            environment is refused), fmt, floor, pin, clippy, the test build, the workspace tests with prikk three times and once without it,
#            deny, audit (fresh database), ENF-2, doc, the 1.89.0 check, the publish dry run (its own target), **`package-keys`** (no file of any of the eleven packages is named like a
#            private key or holds one: `cargo package --list` of each), the check of **the sheet the owner
#            reads**, which `--sheet` must name (a release is not tagged on a sheet nobody named: without it the script says so and starts
#            nothing), and CI read as in the review tier; everything it made is removed at the end, the target included.
#
# **The shared target of the review tier** (`.git-exclude/tmp/gate-target-<compiler>-<lock>/`) is a cache that saves the third-party dependencies and
# the disk, **not** the workspace's own compile: cargo takes a workspace artifact as current by file times, so a worktree this run has just made
# (its files are newer than anything cached) rebuilds its own crates, while an OLDER checkout pointed at the same target is given the cached binary of
# another commit and another path. The script therefore uses the shared target only for a worktree it made in this run (every file newer than the run's
# start), says so in its header, and **refuses it for anything else** (a target of its own in the scratch). `GATES_EXISTING_CHECKOUT=DIR` runs the
# steps in an existing checkout instead of a fresh worktree (the script's own tests use it); the shared target is then never used.
#
# **The cleanup runs on every way out** (the end, a failing step, `INT`, `TERM`): `git worktree remove --force` and `git worktree prune`
# for the worktree this run made, and `rm -rf` of its scratch directory, each **by the one explicit path this run created**, never a glob
# and never a path it did not make. (Only the stale keys of the review tier's shared target are removed beyond that, by their exact names,
# and only those that carry this script's marker file.) The tools installed once per machine stay, in `.git-exclude/tmp/gate-tools/`.
#
# **A run is immune to edits of the scripts meanwhile.** A POSIX shell reads a script a piece at a time as it runs, so a script that runs for twenty
# minutes and is rewritten on disk (the working copy is shared) resumes in the middle of the new text. At its very start the script therefore copies
# itself and `tools/run-capped.sh` into a private directory (`mktemp -d`, mode 0700) and **re-executes from the copy** (`--snapshot-of ORIGINAL` marks the
# copy, so that it does not copy again), prints `running from a snapshot of tools/gates.sh (<sha256>)`, and removes that one directory when it ends, on
# every way out. A copy that cannot be made is a stop with a sentence, never a continuation from the original. Only the *runner* is snapshotted: the
# steps run the commit's own files in the worktree this run made.
#
# `GATES_SELF_TEST_STEPS=FILE` replaces the tier's step list with the lines `name<TAB>command` of FILE (and skips the compiler check and the tools):
# it is how this script's own tests (`tools/tests/gates.sh`) show the cleanup without building anything.
set -u

here=$(cd "$(dirname "$0")" && pwd)
usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; }

case "${1:-}" in
  -h | --help) usage; exit 0 ;;
esac

# Run from a snapshot, before anything else is read: see the header. The first start makes the copy and re-executes from it; the copy (and the
# process under the cap, which is the same copy again) knows it is one by `--snapshot-of`.
if [ "${1:-}" != --snapshot-of ]; then
  orig="$here/$(basename "$0")"
  nosnap() { echo "gates: cannot make a snapshot of $orig ($1): stopping rather than run from a file that others may edit meanwhile" >&2; exit 2; }
  snap=$(mktemp -d "${TMPDIR:-/tmp}/gates-snapshot.XXXXXX" 2> /dev/null) || nosnap "no private directory in ${TMPDIR:-/tmp}"
  chmod 700 "$snap" || { rm -rf "$snap"; nosnap "chmod"; }
  for f in gates.sh run-capped.sh; do
    cp "$here/$f" "$snap/$f" 2> /dev/null || { rm -rf "$snap"; nosnap "cannot copy $f"; }
  done
  cmp -s "$orig" "$snap/gates.sh" || { rm -rf "$snap"; nosnap "the file changed while it was copied"; }
  chmod 700 "$snap/gates.sh" "$snap/run-capped.sh" || { rm -rf "$snap"; nosnap "chmod"; }
  exec sh "$snap/gates.sh" --snapshot-of "$orig" "$@"
fi
snapshot_of=${2:?--snapshot-of needs the original file}
shift 2
SNAPDIR=$here
case "$SNAPDIR" in */gates-snapshot.*) trap 'rm -rf "$SNAPDIR"' EXIT ;; *) echo "gates: not running from a snapshot directory: $SNAPDIR" >&2; exit 2 ;; esac

# Run inside the cap, once.
if [ -z "${GATES_CAPPED:-}" ]; then
  exec "$here/run-capped.sh" -- env GATES_CAPPED=1 sh "$here/gates.sh" --snapshot-of "$snapshot_of" "$@"
fi

tier="" commit=HEAD base="" runs="" sheet="" no_ci=0 fail_fast=0 plan_only=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tier) tier="${2:?--tier needs commit, review or release}"; shift 2 ;;
    --commit) commit="${2:?--commit needs a revision}"; shift 2 ;;
    --base) base="${2:?--base needs a revision}"; shift 2 ;;
    --runs) runs="${2:?--runs needs a number}"; shift 2 ;;
    --sheet) sheet="${2:?--sheet needs a file}"; shift 2 ;;
    --no-ci) no_ci=1; shift ;;
    --fail-fast) fail_fast=1; shift ;;
    --plan) plan_only=1; shift ;;
    *) echo "gates: unknown argument $1" >&2; usage >&2; exit 2 ;;
  esac
done
# A relative --sheet is relative to where the script was run, and the steps run in another directory (the gate's worktree): make it absolute now.
case "$sheet" in "" | /*) ;; *) sheet="$PWD/$sheet" ;; esac
case "$tier" in commit | review | release) ;; *) echo "gates: --tier commit, review or release is required" >&2; exit 2 ;; esac
if [ "$tier" = release ] && [ -z "$sheet" ]; then
  echo "gates: the release tier needs --sheet FILE: the Japanese reading sheet the owner reads (a release is not tagged on a sheet nobody named). Nothing was started." >&2
  exit 2
fi

say() { echo "gates: $*"; }
die() { echo "gates: $*" >&2; exit 2; }
say "running from a snapshot of tools/gates.sh ($(sha256sum "$0" | cut -d' ' -f1))"

repo=$(git rev-parse --show-toplevel 2> /dev/null) || die "run it inside the repository"
cd "$repo" || exit 2
sha=$(git rev-parse --verify "$commit^{commit}" 2> /dev/null) || die "no such revision: $commit"
short=$(echo "$sha" | cut -c1-8)
selftest="${GATES_SELF_TEST_STEPS:-}"

# ---------------------------------------------------------------------------------------------------- the compiler, as the release has it
msrv=$(git show "$sha:Cargo.toml" 2> /dev/null | sed -n 's/^rust-version *= *"\([0-9.]*\)".*/\1/p' | head -1)
pinned=""
if [ -z "$selftest" ]; then
  # The pin planeter-pin-check reads: the `rustup toolchain install X.Y.Z` of `ci.yml` that is not the declared minimum's.
  pinned=$(git show "$sha:.github/workflows/ci.yml" 2> /dev/null | sed -n 's/.*rustup toolchain install \([0-9][0-9.]*\).*/\1/p' | while read -r v; do
    if [ "$v" != "$msrv" ]; then echo "$v"; break; fi
  done)
  [ -n "$pinned" ] || die "could not read the compiler pin from .github/workflows/ci.yml"
  if ! rustup toolchain list 2> /dev/null | grep -q "^$pinned-"; then
    die "the release's compiler $pinned is not installed. Install it with: rustup toolchain install $pinned --profile minimal --component rustfmt --component clippy   (the gates never fall back to stable)"
  fi
  RUSTUP_TOOLCHAIN=$pinned
  export RUSTUP_TOOLCHAIN
fi

# ---------------------------------------------------------------------------------------------------- what this run makes, and its cleanup
WT="" SCR="" CHILD="" CLEANED=0 PLAN="" GROUP=0
command -v setsid > /dev/null 2>&1 && GROUP=1
cleanup() {
  [ "$CLEANED" = 1 ] && return
  CLEANED=1
  if [ -n "$CHILD" ]; then
    # The step runs in a session of its own (`setsid`, when there is one): the whole group, whatever it has started, is ended; without it, the
    # step's children and the step.
    if [ "$GROUP" = 1 ]; then
      # (the external `kill`: the builtins of the shells differ in how they take a group and `--`)
      env kill -TERM -- "-$CHILD" 2> /dev/null || kill -TERM "$CHILD" 2> /dev/null
      pkill -TERM -P "$CHILD" 2> /dev/null
    else
      pkill -TERM -P "$CHILD" 2> /dev/null
      kill -TERM "$CHILD" 2> /dev/null
    fi
  fi
  cd "$repo" || return
  if [ -n "$WT" ] && [ -e "$WT" ]; then
    git worktree remove --force "$WT" 2> /dev/null
  fi
  [ -n "$WT" ] && git worktree prune 2> /dev/null
  [ -n "$SCR" ] && [ -e "$SCR" ] && rm -rf "$SCR"
  [ -n "$PLAN" ] && rm -f "$PLAN"
  # The snapshot of this runner, by its one path (it is the directory this script runs from; the check keeps a wrong path from being removed).
  case "$SNAPDIR" in */gates-snapshot.*) rm -rf "$SNAPDIR" ;; esac
  say "cleanup done: worktree ${WT:-(none)} and scratch ${SCR:-(none)} removed"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

# ---------------------------------------------------------------------------------------------------- the plan of each tier
runs_with=${runs:-1}
[ "$tier" = release ] && runs_with=${runs:-3}
changed=""
if [ "$tier" = review ]; then
  changed=$(git diff --name-only "${base:-$sha^}" "$sha" 2> /dev/null)
fi
touches() { printf '%s\n' "$changed" | grep -Eq "$1"; }
public_items_changed() { git diff -U0 "${base:-$sha^}" "$sha" -- '*.rs' 2> /dev/null | grep -Eq '^[+-][[:space:]]*(pub[[:space:](]|//[/!])'; }

PLAN=$(mktemp "${TMPDIR:-/tmp}/gates-plan.XXXXXX") || exit 2
plan() { printf '%s\t%s\n' "$1" "$2" >> "$PLAN"; }
tests_with="cargo test --workspace --locked --no-fail-fast"
tests_without='PATH="$GATES_NOPRIKK_PATH" cargo test --workspace --locked --no-fail-fast'
case "$tier" in
  commit)
    plan fmt "cargo fmt --all --check"
    plan floor-check "cargo run -q --locked -p planeter-floor-check"
    plan pin-check "cargo run -q --locked -p planeter-pin-check"
    plan clippy "cargo clippy --workspace --all-targets --locked -- -D warnings"
    # The crates the diff touches.
    base_c=$(git merge-base HEAD origin/main 2> /dev/null || echo HEAD)
    files=$({ git diff --name-only "$base_c"; git ls-files --others --exclude-standard; } 2> /dev/null | sort -u)
    whole=0 pkgs=""
    for f in $files; do
      case "$f" in
        Cargo.toml | Cargo.lock | tools/* | .github/* | Dockerfile | rust-toolchain*) whole=1 ;;
        crates/*/*)
          c=${f#crates/}; c=${c%%/*}
          n=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "crates/$c/Cargo.toml" 2> /dev/null | head -1)
          [ -n "$n" ] && case " $pkgs " in *" -p $n "*) ;; *) pkgs="$pkgs -p $n" ;; esac
          ;;
      esac
    done
    if [ "$whole" = 1 ]; then
      plan test "cargo test --workspace --locked --no-fail-fast"
    elif [ -n "$pkgs" ]; then
      plan test "cargo test --locked --no-fail-fast$pkgs"
    else
      plan test "echo 'no crate changed: no tests run (the diff touches none)'"
    fi
    ;;
  review)
    i=1
    while [ "$i" -le "$runs_with" ]; do plan "tests-with-prikk-$i" "$tests_with"; i=$((i + 1)); done
    plan deny "cargo deny check"
    plan audit "cargo audit --db \"\$GATES_ADVDB\" --no-fetch"
    # The sheet step needs no file: a sheet is generated from this worktree's live catalogs and checked (the catalogs are complete and the generator
    # accepts them). A supplied sheet is an extra check of that file, and its failure says whose it is.
    plan sheet-check "cargo run -q --locked -p planeter-i18n-sheet -- --lang ja --out \"\$GATES_SHEET_OUT\" && cargo run -q --locked -p planeter-i18n-sheet -- --lang ja --check \"\$GATES_SHEET_OUT\""
    [ -n "$sheet" ] && plan supplied-sheet "cargo run -q --locked -p planeter-i18n-sheet -- --lang ja --check \"$sheet\""
    if touches '(^docs/|\.md$|(^|/)Cargo\.toml$)' || public_items_changed; then plan doc "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked"; fi
    if touches '(^|/)Cargo\.(toml|lock)$'; then
      plan msrv-check "cargo +$msrv check --workspace --locked"
      plan publish-dry-run "CARGO_TARGET_DIR=\"\$GATES_DRYRUN_TARGET\" cargo publish --workspace --exclude planeter-runner --dry-run --locked"
    fi
    if touches '(^\.github/|^Dockerfile$|^tools/|(^|/)Cargo\.(toml|lock)$)'; then
      plan enf2 "tools/enf2-symbol-check/check.sh"
      plan pin-check "cargo run -q --locked -p planeter-pin-check"
    fi
    ;;
  release)
    plan fmt "cargo fmt --all --check"
    plan floor-check "cargo run -q --locked -p planeter-floor-check"
    plan pin-check "cargo run -q --locked -p planeter-pin-check"
    plan clippy "cargo clippy --workspace --all-targets --locked -- -D warnings"
    plan test-build "cargo test --workspace --locked --no-run"
    i=1
    while [ "$i" -le "$runs_with" ]; do plan "tests-with-prikk-$i" "$tests_with"; i=$((i + 1)); done
    plan tests-without-prikk "$tests_without"
    plan deny "cargo deny check"
    plan audit "cargo audit --db \"\$GATES_ADVDB\" --no-fetch"
    plan enf2 "tools/enf2-symbol-check/check.sh"
    plan doc "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked"
    plan msrv-check "cargo +$msrv check --workspace --locked"
    plan publish-dry-run "CARGO_TARGET_DIR=\"\$GATES_DRYRUN_TARGET\" cargo publish --workspace --exclude planeter-runner --dry-run --locked"
    # No private key in any package that would be published (a crate is never taken back from crates.io).
    plan package-keys 'sh "$GATES_PACKAGE_KEYS"'
    plan sheet-check "cargo run -q --locked -p planeter-i18n-sheet -- --lang ja --check \"$sheet\""
    ;;
esac
if [ -n "$selftest" ]; then : > "$PLAN"; cat "$selftest" > "$PLAN"; fi

print_header() {
  say "tier: $tier   commit: $short   compiler: ${pinned:-"(self test: none)"} ($(rustc --version 2> /dev/null))"
  [ -n "$pinned" ] && say "the release's compiler, from .github/workflows/ci.yml; RUSTUP_TOOLCHAIN=$pinned"
  case "$tier" in
    commit) say "runs where you are; incremental builds on; no worktree is made and none is cleaned up" ;;
    review) say "a review's green is NOT a release's evidence: it reads CI for the commit and runs what CI cannot, on a shared target (CARGO_INCREMENTAL=0)" ;;
    release) say "a clean worktree and an empty target; everything it makes is removed at the end" ;;
  esac
  say "steps:"
  awk -F'\t' '{printf "  %d. %-20s %s\n", NR, $1, $2}' "$PLAN"
}
print_header
if [ "$plan_only" = 1 ]; then rm -f "$PLAN"; exit 0; fi

work="$repo"
FRESH=0
if [ "$tier" != commit ]; then
  arch="$repo/.git-exclude/tmp/arch"
  mkdir -p "$arch" || die "cannot make $arch"
  SCR="$arch/gates-scratch-$short-$$"
  mkdir -p "$SCR" || die "cannot make $SCR"
  : > "$SCR/.run-start"
  if [ -n "${GATES_EXISTING_CHECKOUT:-}" ] && [ "$tier" = review ]; then
    # An existing checkout (the script's own tests, or a deliberate run): never a worktree of this run's, never the shared target.
    work="$GATES_EXISTING_CHECKOUT"
    [ -d "$work" ] || die "GATES_EXISTING_CHECKOUT is not a directory: $work"
    say "existing checkout: $work   scratch: $SCR"
  else
    WT="$arch/gates-$short-$$"
    sleep 0.05
    git worktree add -q --detach "$WT" "$sha" || die "cannot make a worktree of $short"
    work="$WT"
    # Fresh: made by this run a moment ago, and every file of it newer than the run's start (nothing older that a cached artifact could outlast).
    if [ -z "$(find "$WT" -path "$WT/.git" -prune -o -type f ! -newer "$SCR/.run-start" -print -quit 2> /dev/null)" ]; then FRESH=1; fi
    say "worktree: $WT   scratch: $SCR"
  fi
fi

# ---------------------------------------------------------------------------------------------------- the environment of the steps
GATES_DRYRUN_TARGET="$SCR/dry-run-target"
GATES_ADVDB="$SCR/advisory-db"
GATES_SHEET_OUT="$SCR/sheet.md"
GATES_PACKAGE_KEYS="$SCR/package-keys.sh"
export GATES_DRYRUN_TARGET GATES_ADVDB GATES_SHEET_OUT GATES_PACKAGE_KEYS
if [ "$tier" = release ]; then
  # The `package-keys` step (release tier only): for each of the eleven published crates, `cargo package --list`; fail if a listed file is named like a key
  # (`*.pem`, `*.key`, `*.seed`, `*.p12`) or contains `PRIVATE KEY-----` or `-----BEGIN PRIVATE`. Run in the worktree. A test key written as a string in a
  # Rust file that was already in the packages of earlier releases is named below, printed every time, and is not a failure; any other file is.
  cat > "$GATES_PACKAGE_KEYS" <<'PKEYS'
crates="planeter-i18n planeter-prikk planeter-store planeter-core planeter-auth planeter-ci planeter-registry planeter-transport planeter-web planeter planeter-client"
known="planeter-auth:src/oidc.rs planeter-web:tests/oidc.rs"
bad=0 files=0
for c in $crates; do
  dir=crates/$c
  list=$(cargo package --list -p "$c" --locked 2> /dev/null) || { echo "package-keys: cannot list the package of $c"; exit 2; }
  for f in $list; do
    files=$((files + 1))
    case "$f" in
      *.pem | *.key | *.seed | *.p12) echo "package-keys: $c: $f is named like a key"; bad=$((bad + 1)); continue ;;
    esac
    [ -f "$dir/$f" ] || continue
    if grep -qE 'PRIVATE KEY-----|-----BEGIN PRIVATE' "$dir/$f" 2> /dev/null; then
      case " $known " in
        *" $c:$f "*) echo "package-keys: $c: $f holds a test key as a string (known, in earlier releases' packages)" ;;
        *) echo "package-keys: $c: $f holds a private key"; bad=$((bad + 1)) ;;
      esac
    fi
  done
done
echo "package-keys: looked at $files files of: $crates"
if [ "$bad" -ne 0 ]; then echo "package-keys: $bad file(s) must not be in a published package"; exit 1; fi
echo "package-keys: no private key in any package"
PKEYS
fi
case "$tier" in
  release)
    [ -z "${CARGO_TARGET_DIR:-}" ] || die "the release tier builds in an empty target of its own and refuses a shared one: CARGO_TARGET_DIR is set (to $CARGO_TARGET_DIR); unset it"
    CARGO_INCREMENTAL=0
    export CARGO_INCREMENTAL
    ;;
  review)
    CARGO_INCREMENTAL=0
    export CARGO_INCREMENTAL
    if [ "$FRESH" = 1 ]; then
      lock=$(git show "$sha:Cargo.lock" 2> /dev/null | sha256sum | cut -c1-12)
      key="gate-target-$(rustc --version | cut -d' ' -f2)-$lock"
      CARGO_TARGET_DIR="$repo/.git-exclude/tmp/$key"
      for d in "$repo"/.git-exclude/tmp/gate-target-*; do
        [ -d "$d" ] && [ -e "$d/.planeter-gates-target" ] && [ "$d" != "$CARGO_TARGET_DIR" ] && { say "removing the stale shared target $d"; rm -rf "$d"; }
      done
      mkdir -p "$CARGO_TARGET_DIR" && cat > "$CARGO_TARGET_DIR/.planeter-gates-target" <<'MARK'
Made by tools/gates.sh for its own fresh worktrees. Do not point another checkout at it: a cached workspace artifact can be taken as current for older
files and be a different commit's. To use it by hand, make a fresh worktree. (It is a cache for the third-party dependencies and the disk, not for the
workspace's own compile.)
MARK
      export CARGO_TARGET_DIR
      say "shared target: $CARGO_TARGET_DIR   (the worktree is fresh: made by this run, every file newer than its start)"
    else
      CARGO_TARGET_DIR="$SCR/target"
      mkdir -p "$CARGO_TARGET_DIR"
      export CARGO_TARGET_DIR
      say "NOT using the shared target: the checkout is not a fresh worktree of this run, and a cached workspace artifact could be taken as current for older files and be a different commit's. A target of its own: $CARGO_TARGET_DIR"
    fi
    ;;
esac

# ---------------------------------------------------------------------------------------------------- CI, read for the exact commit
read_ci() {
  command -v gh > /dev/null 2>&1 || { say "gh is not available: CI cannot be read"; return 1; }
  rows=$(gh run list --commit "$sha" --json databaseId,status,conclusion,name -q '.[] | "\(.databaseId)\t\(.status)\t\(.conclusion)\t\(.name)"' 2> /dev/null) || { say "gh could not list the runs of $short"; return 1; }
  [ -n "$rows" ] || { say "CI has no run for $short"; return 1; }
  ok=0
  while IFS="$(printf '\t')" read -r id st concl wf; do
    say "CI run $id ($wf): $st $concl"
    [ "$st" = completed ] || ok=1
    jobs=$(gh run view "$id" --json jobs -q '.jobs[] | "\(.name)\t\(.status)\t\(.conclusion)"' 2> /dev/null) || ok=1
    while IFS="$(printf '\t')" read -r jn js jc; do
      [ -n "$jn" ] || continue
      say "  job $jn: $js $jc"
      case "$jc" in success | skipped) ;; *) ok=1 ;; esac
    done <<EOF
$jobs
EOF
  done <<EOF
$rows
EOF
  return $ok
}
if [ "$tier" != commit ]; then
  if [ "$no_ci" = 1 ]; then
    say "!!! --no-ci: CI was NOT read; this run proves nothing about what CI said for $short !!!"
  elif ! read_ci; then
    die "CI for $short is not complete and green. Wait for it, or run with --no-ci (and say so in the report)."
  fi
fi

# ---------------------------------------------------------------------------------------------------- tools and the fresh advisory database
tools_root="$repo/.git-exclude/tmp/gate-tools"
if [ -z "$selftest" ] && [ "$tier" != commit ]; then
  PATH="$tools_root/bin:$PATH"
  export PATH
  for spec in $(git show "$sha:.github/workflows/ci.yml" | sed -n 's/.*cargo install \(cargo-[a-z]*\) --version \([0-9.]*\) .*/\1=\2/p' | sort -u); do
    t=${spec%%=*} v=${spec#*=}
    if ! "$tools_root/bin/$t" --version 2> /dev/null | grep -q "$v"; then
      say "installing $t $v once into $tools_root (the pin of the workflows)"
      mkdir -p "$tools_root"
      cargo install "$t" --version "$v" --locked --root "$tools_root" --target-dir "$SCR/tool-target" || die "cannot install $t $v"
      rm -rf "$SCR/tool-target"
    fi
  done
  git clone -q --depth 1 "${GATES_ADVISORY_DB_URL:-https://github.com/RustSec/advisory-db.git}" "$GATES_ADVDB" || die "cannot fetch the advisory database"
  say "advisory database: $(git -C "$GATES_ADVDB" log -1 --format='%H %cs')"
fi
# `PATH` for the run without prikk: a directory with only what cargo needs.
if [ "$tier" = release ] && [ -z "$selftest" ]; then
  nop="$SCR/no-prikk-bin"
  mkdir -p "$nop"
  for t in cargo rustc rustup; do ln -s "$(command -v "$t")" "$nop/$t"; done
  GATES_NOPRIKK_PATH="$nop:/usr/bin:/bin"
  export GATES_NOPRIKK_PATH
  PATH="$tools_root/bin:$PATH"
fi

# ---------------------------------------------------------------------------------------------------- the steps
cg=/sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup 2> /dev/null)
cpu_usec() { awk '/^usage_usec/{print $2}' "$cg/cpu.stat" 2> /dev/null; }
RESULTS="$SCR/results.tsv"
[ -n "$SCR" ] || RESULTS=$(mktemp "${TMPDIR:-/tmp}/gates-results.XXXXXX")
: > "$RESULTS"
first_rc=0 first_name="" sup_rc=0
while IFS="$(printf '\t')" read -r name cmd; do
  [ -n "$name" ] || continue
  say "=== step: $name"
  c0=$(cpu_usec); t0=$(date +%s)
  if [ "$GROUP" = 1 ]; then
    (cd "$work" && exec setsid sh -c "$cmd") < /dev/null &
  else
    (cd "$work" && exec sh -c "$cmd") < /dev/null &
  fi
  CHILD=$!
  wait "$CHILD"
  rc=$?
  CHILD=""
  t1=$(date +%s); c1=$(cpu_usec)
  cpu="-"
  [ -n "$c0" ] && [ -n "$c1" ] && cpu=$(((c1 - c0) / 1000000))
  peak="-"
  [ -r "$cg/memory.peak" ] && peak=$(($(cat "$cg/memory.peak") / 1048576))
  printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$rc" "$((t1 - t0))" "$cpu" "$peak" >> "$RESULTS"
  if [ "$rc" -ne 0 ]; then
    if [ "$name" = supplied-sheet ]; then
      # A fact about the file somebody supplied, not about the commit under review.
      say "the supplied sheet does not match the catalogs: $sheet (this says nothing about the commit; regenerate that file)"
      [ "$sup_rc" -eq 0 ] && sup_rc=$rc
    else
      say "step $name FAILED with $rc"
      if [ "$first_rc" -eq 0 ]; then first_rc=$rc first_name=$name; fi
    fi
    [ "$fail_fast" = 1 ] && break
  fi
done < "$PLAN"

echo
say "result: tier $tier, commit $short, compiler ${pinned:-none}"
printf '%-24s %6s %8s %9s %12s\n' step exit "wall s" "CPU s" "peak MiB*"
awk -F'\t' '{printf "%-24s %6s %8s %9s %12s\n", $1, $2, $3, $4, $5; w+=$3; if ($4 != "-") c+=$4} END {printf "%-24s %6s %8d %9d\n", "total", "", w, c}' "$RESULTS"
say "* peak MiB is the scope's memory.peak so far (it includes page cache); CPU s is the scope's cpu.stat difference around the step"
if [ "$first_rc" -ne 0 ]; then
  say "FAILED: the first failing step is $first_name (exit $first_rc)"
  [ "$sup_rc" -ne 0 ] && say "and the supplied sheet does not match the catalogs: $sheet"
  exit "$first_rc"
fi
if [ "$sup_rc" -ne 0 ]; then
  say "FAILED: the supplied sheet does not match the catalogs: $sheet. Every step of the commit passed; the file is stale, not the commit."
  exit "$sup_rc"
fi
say "all steps passed"
exit 0
