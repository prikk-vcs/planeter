#!/bin/sh
# Tests of tools/gates.sh without building anything (fake step commands, a throw-away repository): the cleanup on a failing step, on INT and on
# TERM, the exit status, `--fail-fast`, the release tier's refusal of a shared target, the review tier's refusal when CI is not green or cannot be
# read, the refusal when the release's compiler is not installed, and the plan of each tier for a diff. `sh tools/tests/gates.sh`
# (run from `cargo test` by tools/pin-check/tests/shell_tools.rs).
set -u
# The tests run inside other gates (the review tier runs `cargo test`, which runs them): nothing of that run's environment may reach them.
unset CARGO_TARGET_DIR CARGO_INCREMENTAL GATES_CAPPED GATES_DRYRUN_TARGET GATES_ADVDB GATES_NOPRIKK_PATH GATES_SELF_TEST_STEPS GATES_ADVISORY_DB_URL
repo=$(cd "$(dirname "$0")/../.." && pwd)
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected [$2] got [$3]"; fails=$((fails + 1)); fi; }
contains() { case "$3" in *"$2"*) echo "ok   $1" ;; *) echo "FAIL $1: [$2] not in: $3" | head -c 600; echo; fails=$((fails + 1)) ;; esac; }
absent() { case "$3" in *"$2"*) echo "FAIL $1: [$2] is in: $3" | head -c 600; echo; fails=$((fails + 1)) ;; *) echo "ok   $1" ;; esac; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gates-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
# A throw-away repository holding the two scripts, a manifest and a ci.yml with a compiler pin.
r="$tmp/repo"
mkdir -p "$r/tools" "$r/.github/workflows" "$r/docs"
cp "$repo/tools/gates.sh" "$repo/tools/run-capped.sh" "$r/tools/"
printf '[workspace]\nrust-version = "1.89.0"\n' > "$r/Cargo.toml"
echo "lock 1" > "$r/Cargo.lock"
echo "doc" > "$r/docs/a.md"
cat > "$r/.github/workflows/ci.yml" <<'YML'
      - run: rustup toolchain install 1.99.1 --profile minimal --no-self-update
      - run: rustup toolchain install 1.89.0 --profile minimal --no-self-update
YML
(cd "$r" && git init -q -b main . && git add -A && git commit -q -m one)
first=$(git -C "$r" rev-parse HEAD)
# A second commit that changes only a document.
echo "doc 2" > "$r/docs/a.md"
(cd "$r" && git commit -qam two)
docs_only=$(git -C "$r" rev-parse HEAD)
# A third that changes Cargo.lock and a workflow.
echo "lock 2" > "$r/Cargo.lock"
echo "# x" >> "$r/.github/workflows/ci.yml"
(cd "$r" && git commit -qam three)
lock_change=$(git -C "$r" rev-parse HEAD)

# The output goes to a file, not a pipe: a step that is left running (a failure of what is tested here) would keep a pipe open and hang the test; and
# nothing waits for longer than five minutes.
# A shell started as a background job (or by `nohup`) has INT *ignored*, and a signal ignored on entry cannot be trapped by anything below it: the
# whole chain (cargo test -> this script -> gates.sh) may have been started that way. `env --default-signal` gives the script the default
# disposition back, so that the INT test means something wherever it runs; where `env` cannot, the INT test is skipped and says so.
if env --default-signal=INT,TERM true 2> /dev/null; then DEFSIG="env --default-signal=INT,TERM"; else DEFSIG=""; fi
gates() {
  (cd "$r" && GATES_CAPPED=1 $DEFSIG timeout 300 sh tools/gates.sh "$@" > "$tmp/gates.out" 2>&1)
  rc_gates=$?
  cat "$tmp/gates.out"
  return $rc_gates
}
state() { echo "worktrees=$(git -C "$r" worktree list | wc -l) leftovers=$(ls "$r/.git-exclude/tmp/arch" 2> /dev/null | wc -l)"; }
start=$(state)
steps="$tmp/steps.tsv"
osheet="$tmp/owner-sheet.md" # the sheet the owner reads (the release tier requires one to be named; the fake steps never open it)

# --- the plan of each tier, for diffs of different kinds (compiler checks are real here: the pin is read from the fixture's ci.yml; the
# self-test hook is not used, so the toolchain must be installed for these; they are skipped, said, when it is not)
pin=1.99.1
if rustup toolchain list 2> /dev/null | grep -q "^$pin-"; then
  out=$(gates --tier review --commit "$docs_only" --plan)
  contains "review plan: a documents-only diff runs doc" "doc " "$out"
  absent "review plan: a documents-only diff has no msrv check" "msrv-check" "$out"
  absent "review plan: a documents-only diff has no publish dry run" "publish-dry-run" "$out"
  absent "review plan: a documents-only diff has no ENF-2" "enf2" "$out"
  contains "review plan: always the tests CI cannot run, deny, audit, the sheet" "tests-ci-cannot-1" "$out"
  absent "review plan: no longer the whole workspace with prikk" "tests-with-prikk" "$out"
  contains "review plan: the compiler is the pin" "RUSTUP_TOOLCHAIN=$pin" "$out"
  out=$(gates --tier review --commit "$lock_change" --plan)
  contains "review plan: a Cargo.lock change runs the 1.89.0 check" "msrv-check" "$out"
  contains "review plan: a Cargo.lock change runs the publish dry run" "publish-dry-run" "$out"
  contains "review plan: a workflow change runs ENF-2 and pin-check" "enf2" "$out"
  # --sheet: a relative path is the caller's, made absolute before the steps change directory (a run was lost to it); an absolute one is kept
  out=$(gates --tier review --commit "$docs_only" --sheet some/dir/sheet.md --plan)
  contains "review plan: a relative --sheet is made absolute (the caller's directory)" "--check \"$r/some/dir/sheet.md\"" "$out"
  out=$(gates --tier review --commit "$docs_only" --sheet /elsewhere/sheet.md --plan)
  contains "review plan: an absolute --sheet is kept" "--check \"/elsewhere/sheet.md\"" "$out"
  absent "review plan: an absolute --sheet is not prefixed" "$r/elsewhere" "$out"
  out=$(gates --tier review --commit "$lock_change" --base "$first" --runs 2 --plan)
  contains "review plan: --runs 2 is two runs" "tests-ci-cannot-2" "$out"
  out=$(gates --tier release --commit "$docs_only" --sheet "$osheet" --plan)
  for s in fmt floor-check pin-check clippy test-build tests-with-prikk-3 tests-without-prikk deny audit enf2 doc msrv-check publish-dry-run package-keys sheet-check; do contains "release plan has $s" "$s" "$out"; done
  contains "release plan: package-keys comes after the publish dry run" "publish-dry-run" "$(printf '%s' "$out" | sed -n '/publish-dry-run/,/package-keys/p')"
  out=$(gates --tier review --commit "$docs_only" --plan)
  absent "review plan has no package-keys" "package-keys" "$out"
  out=$(gates --tier commit --plan)
  absent "commit plan has no package-keys" "package-keys" "$out"
  out=$(gates --tier release --commit "$docs_only" --sheet "$osheet" --plan)
  out=$(gates --tier commit --plan)
  contains "commit plan: fmt and clippy" "clippy" "$out"
  # --- the docs tier (RFC 010 Amendment 2, principle 7): only for a diff of documents, on the evidence of a commit; a fourth tier that is not a release
  (cd "$r" && mkdir -p rfcs && echo r > rfcs/x.md && echo c > CHANGELOG.md && echo m > README.md && echo o > ROADMAP.md && git add -A && git commit -qm docs-more)
  docs_more=$(git -C "$r" rev-parse HEAD)
  out=$(gates --tier docs --commit "$docs_more" --plan); rc=$?
  check "docs tier without --base: refused, exit 2" "2" "$rc"
  contains "...saying what the base is" "needs --base REV" "$out"
  out=$(gates --tier docs --base "$lock_change" --commit "$docs_more" --plan); rc=$?
  check "docs tier on a diff of docs/, rfcs/, CHANGELOG, README and ROADMAP: planned" "0" "$rc"
  for s in fmt floor-check pin-check deny audit doc package-keys sheet-check tests-that-read-documents; do contains "docs plan has $s" "$s" "$out"; done
  contains "docs plan says it is not a release tier" "NOT a release tier" "$out"
  absent "docs plan has no clippy" "clippy" "$out"
  absent "docs plan has no whole-workspace tests" "cargo test --workspace" "$out"
  contains "docs plan: the tests are the ones that read documents" 'GATES_TESTSET" docs' "$out"
  out=$(gates --tier docs --base "$first" --commit "$lock_change" --plan); rc=$?
  check "docs tier on a diff that touches Cargo.lock and a workflow: refused, exit 2" "2" "$rc"
  contains "...naming the files" "Cargo.lock" "$out"
  contains "...and sending the reader to the release tier" "run the release tier" "$out"
  absent "...and nothing was started (no steps)" "steps:" "$out"
  check "...and it left nothing behind" "$start" "$(state)"
  out=$(gates --tier docs --base "$docs_more" --commit "$first" --plan); rc=$?
  check "docs tier whose evidence is not an ancestor: refused, exit 2" "2" "$rc"
  contains "...with a sentence" "not an ancestor" "$out"
  out=$(gates --tier docs --base "no-such-revision" --commit "$docs_more" --plan); rc=$?
  check "docs tier with an unknown base: refused, exit 2" "2" "$rc"
  # the compiler that is not installed
  sed -i 's/1\.99\.1/1.2.3/' "$r/.github/workflows/ci.yml"
  (cd "$r" && git commit -qam four)
  out=$(gates --tier review --commit HEAD --plan); rc=$?
  contains "an uninstalled compiler is refused with the install command" "rustup toolchain install 1.2.3" "$out"
  contains "...and never falls back to stable" "never fall back to stable" "$out"
  check "$(state)" "$start" "$(state)"
else
  echo "skip the plan tests: the toolchain $pin is not installed here"
fi

# --- the sheet step (RFC 010 Amendment 1): the review tier generates its own; a supplied sheet is an extra check, named as such; the release tier needs one
if rustup toolchain list 2> /dev/null | grep -q "^$pin-"; then
  out=$(gates --tier review --commit "$docs_only" --plan)
  contains "review plan: without --sheet the sheet step generates a sheet from the live catalogs" '--out "$GATES_SHEET_OUT"' "$out"
  contains "review plan: ...and checks what it generated" '--check "$GATES_SHEET_OUT"' "$out"
  absent "review plan: ...and has no step for a supplied sheet" "supplied-sheet" "$out"
  out=$(gates --tier review --commit "$docs_only" --sheet "$osheet" --plan)
  contains "review plan: with --sheet there is a separate supplied-sheet step" "supplied-sheet" "$out"
  contains "review plan: ...that checks that file" "--check \"$osheet\"" "$out"
  contains "review plan: ...next to the generated one" "GATES_SHEET_OUT" "$out"
fi
out=$(gates --tier release --commit "$first" --plan); rc=$?
check "the release tier without --sheet is refused: exit 2" "2" "$rc"
contains "...saying what to name" "needs --sheet FILE" "$out"
absent "...and nothing was started (no plan, no worktree)" "steps:" "$out"
check "...and it left nothing behind" "$start" "$(state)"
out=$(gates --tier release --commit "$first" --sheet "$osheet" --plan); rc=$?
check "the release tier with --sheet plans as before" "0" "$rc"
contains "...checking the named sheet" "--check \"$osheet\"" "$out"
absent "...and generating none" "GATES_SHEET_OUT" "$out"
# --- the cleanup, with fake steps
use_steps() { printf '%b' "$1" > "$steps"; }
export GATES_SELF_TEST_STEPS="$steps"
use_steps 'one\techo first step ran\n'
out=$(gates --tier release --commit "$first" --sheet "$osheet" --no-ci); rc=$?
check "a passing run exits 0" "0" "$rc"
contains "its table says the step passed" "one " "$out"
check "a passing run leaves the worktrees and the scratch as they were" "$start" "$(state)"

use_steps 'a\ttrue\nb\texit 7\nc\ttrue\n'
out=$(gates --tier release --commit "$first" --sheet "$osheet" --no-ci); rc=$?
check "a failing step: the exit status is the first failing step's" "7" "$rc"
contains "...the later step ran (without --fail-fast)" "c " "$out"
contains "...and the report names the first failure" "first failing step is b (exit 7)" "$out"
check "a failing step: worktrees and scratch as they were" "$start" "$(state)"
check "a failing step: the worktree directory is gone" "no" "$([ -e "$r/.git-exclude/tmp/arch/gates-$(echo "$first" | cut -c1-8)-"* ] 2> /dev/null && echo yes || echo no)"

out=$(gates --tier release --commit "$first" --sheet "$osheet" --no-ci --fail-fast); rc=$?
check "--fail-fast: still 7" "7" "$rc"
absent "--fail-fast: step c did not run" "=== step: c" "$out"
check "--fail-fast: cleaned up" "$start" "$(state)"

mark=$((900000 + $$ % 90000)) # a duration no other process on the machine is sleeping
# an interrupted run: the step sends INT (then TERM) to the script that runs it, which is its parent
if [ -n "$DEFSIG" ]; then
use_steps "a\ttrue\nb\tkill -INT \$PPID; sleep $mark\nc\ttrue\n"
out=$(gates --tier release --commit "$first" --sheet "$osheet" --no-ci); rc=$?
check "INT: the exit status is 130" "130" "$rc"
contains "INT: the cleanup ran" "cleanup done" "$out"
absent "INT: the next step did not run" "=== step: c" "$out"
check "INT: worktrees and scratch as they were" "$start" "$(state)"
else
  echo "skip INT: this env cannot give the script the default disposition of INT"
fi
use_steps "a\ttrue\nb\tkill -TERM \$PPID; sleep $mark\nc\ttrue\n"
out=$(gates --tier review --commit "$first" --no-ci); rc=$?
check "TERM: the exit status is 143" "143" "$rc"
check "TERM: worktrees and scratch as they were" "$start" "$(state)"
left=1
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  left=$(ps -eo args | grep -c "[s]leep $mark\$")
  [ "$left" = 0 ] && break
  sleep 0.2
done
check "no step is left running after an interrupt" "0" "$left"

# --- the sheet step, continued: a stale supplied sheet (fake steps)
# a stale supplied sheet fails only its own check, and says whose it is (fake steps: the names are the plan's)
use_steps 'tests\ttrue\nsheet-check\ttrue\nsupplied-sheet\texit 1\ndoc\ttrue\n'
out=$(gates --tier review --commit "$first" --no-ci --sheet "$osheet"); rc=$?
check "a stale supplied sheet fails the run (exit 1)" "1" "$rc"
contains "...saying the supplied sheet does not match the catalogs, naming the file" "the supplied sheet does not match the catalogs: $osheet" "$out"
contains "...that the commit's own steps all passed" "Every step of the commit passed" "$out"
absent "...and it is not reported as a failing step of the commit" "first failing step is supplied-sheet" "$out"
contains "...the later step still ran" "doc " "$out"
check "...and it cleaned up" "$start" "$(state)"
use_steps 'tests\texit 3\nsupplied-sheet\texit 1\n'
out=$(gates --tier review --commit "$first" --no-ci --sheet "$osheet"); rc=$?
check "a failing step of the commit and a stale sheet: the exit is the step's" "3" "$rc"
contains "...the first failing step is the commit's" "first failing step is tests (exit 3)" "$out"
contains "...and the stale sheet is named too" "and the supplied sheet does not match the catalogs" "$out"

# --- the shared target says who may use it, and is refused to anything but this run's own fresh worktree
use_steps 'show\tprintf "TARGET=%s\\n" "$CARGO_TARGET_DIR"\n'
out=$(gates --tier review --commit "$first" --no-ci); rc=$?
check "a fresh worktree of this run uses the shared target" "0" "$rc"
contains "...saying so in its header line" "(the worktree is fresh: made by this run" "$out"
contains "...the step sees the shared target" "TARGET=$r/.git-exclude/tmp/gate-target-" "$out"
marker=$(ls "$r"/.git-exclude/tmp/gate-target-*/.planeter-gates-target 2> /dev/null | head -1)
contains "the marker file explains the cache" "Do not point another checkout at it" "$(cat "$marker" 2> /dev/null)"
contains "...and how to use it by hand" "make a fresh worktree" "$(cat "$marker" 2> /dev/null)"
rm -rf "$r"/.git-exclude/tmp/gate-target-*
# a fake existing checkout (older files than anything cached) is refused the shared target
mkdir -p "$tmp/old-checkout"
echo "old file" > "$tmp/old-checkout/a.txt"
touch -d '2020-01-01' "$tmp/old-checkout/a.txt"
out=$(GATES_EXISTING_CHECKOUT="$tmp/old-checkout" gates --tier review --commit "$first" --no-ci); rc=$?
check "an existing checkout still runs the steps" "0" "$rc"
contains "...says it is not a fresh worktree and is refused the shared target" "NOT using the shared target" "$out"
absent "...does not use the shared target" "shared target: $r" "$out"
absent "...the step's target is not a gate-target directory" "TARGET=$r/.git-exclude/tmp/gate-target-" "$out"
contains "...it is a target of its own in the scratch" "TARGET=$r/.git-exclude/tmp/arch/gates-scratch-" "$out"
check "...no shared target directory was made" "0" "$(ls -d "$r"/.git-exclude/tmp/gate-target-* 2> /dev/null | wc -l)"
check "...and the existing checkout was not removed" "yes" "$([ -e "$tmp/old-checkout/a.txt" ] && echo yes || echo no)"
check "...and it cleaned up" "$start" "$(state)"

# --- a run is immune to edits of the script meanwhile (a shell reads a script a piece at a time, so a rewritten file resumed in the middle of new text)
snaptmp="$tmp/snaptmp"
mkdir -p "$snaptmp"
snapshots() { ls "$snaptmp" 2> /dev/null | grep -c '^gates-snapshot\.'; }
cp "$r/tools/gates.sh" "$tmp/gates.sh.orig"
orig_sum=$(sha256sum "$r/tools/gates.sh" | cut -d' ' -f1)
# $1 = a label, $2 = what happens to the script on disk (in place, as an editor or a script writes a file) while a slow step is running
edit_while_running() {
  rm -f "$tmp/started" "$tmp/go" "$tmp/edit.rc"
  use_steps "slow\ttouch $tmp/started; while [ ! -e $tmp/go ]; do sleep 0.1; done; echo slow-step-done\nafter\techo after-the-edit\n"
  (cd "$r" && TMPDIR="$snaptmp" GATES_CAPPED=1 $DEFSIG timeout 120 sh tools/gates.sh --tier release --commit "$first" --no-ci --sheet "$osheet" > "$tmp/edit.out" 2>&1; echo $? > "$tmp/edit.rc") &
  n=0
  while [ ! -e "$tmp/started" ] && [ "$n" -lt 300 ]; do sleep 0.1; n=$((n + 1)); done
  check "$1: the slow step started" "yes" "$([ -e "$tmp/started" ] && echo yes || echo no)"
  check "$1: the run is from a snapshot (one private directory exists while it runs)" "1" "$(snapshots)"
  eval "$2"
  touch "$tmp/go"
  n=0
  while [ ! -e "$tmp/edit.rc" ] && [ "$n" -lt 600 ]; do sleep 0.1; n=$((n + 1)); done
  out=$(cat "$tmp/edit.out"); rc=$(cat "$tmp/edit.rc" 2> /dev/null)
  cp "$tmp/gates.sh.orig" "$r/tools/gates.sh"
  check "$1: the run finishes with its own exit status" "0" "$rc"
  contains "$1: ...and its own table" "result: tier release" "$out"
  contains "$1: ...with the step that ran after the edit" "after " "$out"
  contains "$1: ...and its own verdict" "all steps passed" "$out"
  absent "$1: ...no syntax error from a file read in the middle" "syntax error" "$out"
  absent "$1: ...nor the replacement's text" "REPLACED BY ANOTHER SCRIPT" "$out"
  contains "$1: ...the log names the version that ran (the file's hash at the start)" "running from a snapshot of tools/gates.sh ($orig_sum)" "$out"
  contains "$1: ...and it cleaned up" "cleanup done" "$out"
  check "$1: ...the snapshot is gone" "0" "$(snapshots)"
  check "$1: ...worktrees and scratch as they were" "$start" "$(state)"
}
# the edit that breaks the syntax: a line inserted near the top shifts everything the shell has not read yet (appended at the end it would never be reached)
edit_insert_junk() { { sed -n 1,3p "$tmp/gates.sh.orig"; echo 'fi )'; sed -n '4,$p' "$tmp/gates.sh.orig"; } > "$tmp/edited.sh" && cat "$tmp/edited.sh" > "$r/tools/gates.sh"; }
# the edit that replaces it with a different, valid script
edit_replace() { printf '#!/bin/sh\necho REPLACED BY ANOTHER SCRIPT\nexit 99\n' > "$r/tools/gates.sh"; }
edit_while_running "an inserted syntax error" edit_insert_junk
edit_while_running "a replaced script" edit_replace
# the snapshot is removed on every way out: a failing step, TERM, INT
use_steps 'a\texit 7\n'
out=$(TMPDIR="$snaptmp" gates --tier release --commit "$first" --no-ci --sheet "$osheet"); rc=$?
check "a failing step: exit 7" "7" "$rc"
check "...and the snapshot is gone" "0" "$(snapshots)"
use_steps "a\tkill -TERM \$PPID; sleep $mark\nb\ttrue\n"
out=$(TMPDIR="$snaptmp" gates --tier release --commit "$first" --no-ci --sheet "$osheet"); rc=$?
check "TERM: exit 143" "143" "$rc"
check "...and the snapshot is gone" "0" "$(snapshots)"
if [ -n "$DEFSIG" ]; then
  use_steps "a\tkill -INT \$PPID; sleep $mark\nb\ttrue\n"
  out=$(TMPDIR="$snaptmp" gates --tier release --commit "$first" --no-ci --sheet "$osheet"); rc=$?
  check "INT: exit 130" "130" "$rc"
  check "...and the snapshot is gone" "0" "$(snapshots)"
fi
use_steps 'one\techo ran\n'
out=$(TMPDIR="$snaptmp" gates --tier release --commit "$first" --no-ci --sheet "$osheet" --plan); rc=$?
check "--plan: exit 0" "0" "$rc"
check "...and the snapshot is gone" "0" "$(snapshots)"
out=$(TMPDIR="$snaptmp" gates --tier release --commit "$first" --no-ci --bogus); rc=$?
check "an unknown argument: exit 2" "2" "$rc"
check "...and the snapshot is gone" "0" "$(snapshots)"
# a snapshot that cannot be made is a stop with a sentence, not a continuation from the original
out=$(TMPDIR="$tmp/no/such/dir" gates --tier release --commit "$first" --no-ci --sheet "$osheet"); rc=$?
check "no snapshot, no run: exit 2" "2" "$rc"
contains "...with a sentence" "cannot make a snapshot of" "$out"
absent "...and nothing ran" "=== step" "$out"
check "...and it left nothing behind" "$start" "$(state)"

# --- package-keys (release tier): no file of a package is named like a private key or holds one. A fake `cargo package --list` and a fake crates tree.
pk="$tmp/pk"
mkdir -p "$pk/bin" "$pk/root/crates" "$pk/lists"
for c in planeter-i18n planeter-prikk planeter-store planeter-core planeter-auth planeter-ci planeter-registry planeter-transport planeter-web planeter planeter-client; do
  mkdir -p "$pk/root/crates/$c/src" "$pk/root/crates/$c/tests"
  echo "fn main() {}" > "$pk/root/crates/$c/src/lib.rs"
  printf 'src/lib.rs\n' > "$pk/lists/$c"
done
cat > "$pk/bin/cargo" <<'FAKE'
#!/bin/sh
# cargo package --list -p CRATE --locked
cat "$PKLISTS/$4"
FAKE
chmod +x "$pk/bin/cargo"
pk_run() {
  use_steps "pk\tcd $pk/root && PKLISTS=$pk/lists PATH=$pk/bin:\$PATH sh \"\$GATES_PACKAGE_KEYS\"\n"
  gates --tier release --commit "$first" --no-ci --sheet "$osheet"
}
out=$(pk_run); rc=$?
check "package-keys on clean packages: exit 0" "0" "$rc"
contains "...says how many files and which crates it looked at" "looked at 11 files of: planeter-i18n" "$out"
contains "...and that there is none" "no private key in any package" "$out"
printf 'src/lib.rs\ntests/fixtures/leaf.pem\n' > "$pk/lists/planeter-auth"
out=$(pk_run); rc=$?
check "a file named *.pem in a package: the step fails" "1" "$rc"
contains "...and names the crate and the file" "planeter-auth: tests/fixtures/leaf.pem is named like a key" "$out"
for ext in key seed p12; do
  printf 'src/lib.rs\nx.%s\n' "$ext" > "$pk/lists/planeter-auth"
  out=$(pk_run); rc=$?
  check "a file named *.$ext: the step fails" "1" "$rc"
done
printf 'src/lib.rs\ntests/keys.json\n' > "$pk/lists/planeter-auth"
printf '{"pem": "-----BEGIN PRIVATE KEY-----\\nAAAA\\n-----END PRIVATE KEY-----"}\n' > "$pk/root/crates/planeter-auth/tests/keys.json"
out=$(pk_run); rc=$?
check "a file that holds a private key (not named like one): the step fails" "1" "$rc"
contains "...and names it" "planeter-auth: tests/keys.json holds a private key" "$out"
printf 'src/lib.rs\ntests/keys.json\n' > "$pk/lists/planeter-auth"
rm "$pk/root/crates/planeter-auth/tests/keys.json"
printf 'src/lib.rs\nsrc/oidc.rs\n' > "$pk/lists/planeter-auth"
printf 'const K: &str = "-----BEGIN PRIVATE KEY-----";\n' > "$pk/root/crates/planeter-auth/src/oidc.rs"
out=$(pk_run); rc=$?
check "the file that held a test key in earlier releases (IS-76): the list of known files is empty, so it fails like any other" "1" "$rc"
contains "...and names it" "planeter-auth: src/oidc.rs holds a private key" "$out"
absent "...and no \"known\" line is printed" "(known" "$out"
printf 'src/lib.rs\nsrc/other.rs\n' > "$pk/lists/planeter-auth"
printf 'const K: &str = "-----BEGIN PRIVATE KEY-----";\n' > "$pk/root/crates/planeter-auth/src/other.rs"
out=$(pk_run); rc=$?
check "the same text in any other file: the step fails" "1" "$rc"
check "...and the run cleaned up" "$start" "$(state)"

# --- the test sets of the review tier and of the docs tier are DERIVED from the sources (RFC 010 Amendment 2): a fake `cargo` and a fake tree
ts="$tmp/ts"
mkdir -p "$ts/bin" "$ts/root/crates/a/tests" "$ts/root/crates/d/tests/common" "$ts/root/crates/d/src" "$ts/root/crates/a/src" "$ts/root/crates/b/src" "$ts/root/crates/c/tests" "$ts/root/tools"
printf '[package]\nname = "a-pkg"\n' > "$ts/root/crates/a/Cargo.toml"; printf '[package]\nname = "b-pkg"\n' > "$ts/root/crates/b/Cargo.toml"; printf '[package]\nname = "c-pkg"\n' > "$ts/root/crates/c/Cargo.toml"; printf '[package]\nname = "d-pkg"\n' > "$ts/root/crates/d/Cargo.toml"; echo "pub fn h() {}" > "$ts/root/crates/d/src/lib.rs"
echo "pub fn f() {}" > "$ts/root/crates/a/src/lib.rs"; echo "pub fn g() {}" > "$ts/root/crates/b/src/lib.rs"
cat > "$ts/root/crates/a/tests/needs_bwrap.rs" <<'RS'
#[test] fn t() { if !ok() { eprintln!("skipping: needs prikk and bubblewrap (with user namespaces)"); return; } }
RS
cat > "$ts/root/crates/a/tests/needs_prikk_only.rs" <<'RS'
#[test] fn t() { if !ok() { eprintln!("skipping: needs prikk on PATH"); return; } }
RS
cat > "$ts/root/crates/a/tests/plain.rs" <<'RS'
#[test] fn t() { assert!(true); }
RS
cat > "$ts/root/crates/a/tests/why_form.rs" <<'RS'
fn sandbox_unusable() -> Option<String> { Some("bwrap cannot make a user namespace".into()) }
#[test] fn t() { if let Some(why) = sandbox_unusable() { eprintln!("skipping: {why}"); return; } }
RS
cat > "$ts/root/crates/a/tests/reads_docs.rs" <<'RS'
#[test] fn t() { let s = std::fs::read_to_string("../../docs/glossary/ja.md").unwrap(); assert!(!s.is_empty()); }
RS
cat > "$ts/root/crates/d/tests/common/mod.rs" <<'RS'
pub fn nothing() { println!("SKIPPED: no sandbox here"); }
RS
cat > "$ts/root/crates/b/src/lib.rs" <<'RS'
#[cfg(test)] mod tests { #[test] fn t() { eprintln!("SKIPPED: the bubblewrap sandbox cannot run
 here"); } }
RS
cat > "$ts/root/crates/c/tests/other_skip.rs" <<'RS'
#[test] fn t() { eprintln!("skipping: needs python3 (with ssl) and curl"); }
RS
cat > "$ts/bin/cargo" <<'FAKE'
#!/bin/sh
echo "cargo $*" >> "$TSLOG"
FAKE
chmod +x "$ts/bin/cargo"
ts_run() { # MODE
  : > "$tmp/tslog"
  use_steps "ts\tcd $ts/root && TSLOG=$tmp/tslog PATH=$ts/bin:\$PATH sh \"\$GATES_TESTSET\" $1\n"
  gates --tier review --commit "$first" --no-ci
}
out=$(ts_run bwrap); rc=$?
check "the review tier's test set (bwrap): exit 0" "0" "$rc"
contains "...a test that skips for want of bubblewrap is in it" "-p a-pkg" "$out"
contains "...(its binary)" "--test needs_bwrap" "$out"
contains "...a skip whose reason is a variable, in a file that probes bwrap, is in it" "--test why_form" "$out"
contains "...a unit test in a crate's source is its --lib" "-p b-pkg --lib" "$out"
contains "...a helper under tests/ sends the crate's whole tests/" "-p d-pkg --tests --lib" "$out"
absent "...a test that skips only for want of prikk is not (CI has prikk)" "needs_prikk_only" "$(cat "$tmp/tslog")"
absent "...a test with no skip is not" "plain" "$(cat "$tmp/tslog")"
absent "...a skip for python3 and curl is not" "other_skip" "$out"
contains "...the set is printed with its size before anything runs" "of 6 integration-test binaries in the workspace" "$out"
out=$(ts_run docs); rc=$?
check "the docs tier's test set: exit 0" "0" "$rc"
contains "...a test that reads docs/ is in it" "--test reads_docs" "$(cat "$tmp/tslog")"
absent "...a test that does not is not" "plain" "$(cat "$tmp/tslog")"
rm "$ts/root/crates/a/tests/needs_bwrap.rs" "$ts/root/crates/a/tests/why_form.rs" "$ts/root/crates/d/tests/common/mod.rs" "$ts/root/crates/b/src/lib.rs"; echo "pub fn g() {}" > "$ts/root/crates/b/src/lib.rs"
out=$(ts_run bwrap); rc=$?
check "an empty bwrap set is a failure (the derivation is broken, not the tests unneeded)" "1" "$rc"
contains "...saying so" "the derivation found no test that skips for want of bubblewrap" "$out"
absent "...and running nothing" "cargo test" "$(cat "$tmp/tslog")"

# --- the release tier refuses a shared target (and cleans up what it had made)
use_steps 'one\techo should not run\n'
out=$(CARGO_TARGET_DIR="$tmp/shared" gates --tier release --commit "$first" --sheet "$osheet" --no-ci); rc=$?
check "release refuses a shared target: exit 2" "2" "$rc"
contains "...with a sentence" "refuses a shared one" "$out"
absent "...and no step ran" "=== step: one" "$out"
check "...and it cleaned up" "$start" "$(state)"

# --- CI is read for the exact commit (a stand-in gh)
mkdir "$tmp/ghbin"
write_gh() { # $1 = the lines `gh run list` prints; $2 = the lines `gh run view` prints
  cat > "$tmp/ghbin/gh" <<GH
#!/bin/sh
case "\$1 \$2" in
  "run list") printf '%b' '$1' ;;
  "run view") printf '%b' '$2' ;;
esac
GH
  chmod +x "$tmp/ghbin/gh"
}
runner() {
  (cd "$r" && PATH="$tmp/ghbin:$PATH" GATES_CAPPED=1 timeout 300 sh tools/gates.sh "$@" > "$tmp/gates.out" 2>&1)
  rc_gates=$?
  cat "$tmp/gates.out"
  return $rc_gates
}
use_steps 'one\techo ran after CI\n'
write_gh '42\tcompleted\tsuccess\tCI\n' 'build\tcompleted\tsuccess\nmsrv\tcompleted\tsuccess\nclient (macos-15)\tcompleted\tskipped\n'
out=$(runner --tier review --commit "$first"); rc=$?
check "green CI: the review goes on" "0" "$rc"
contains "...and prints each job's conclusion" "job build: completed success" "$out"
write_gh '42\tcompleted\tfailure\tCI\n' 'build\tcompleted\tsuccess\nmsrv\tcompleted\tfailure\n'
out=$(runner --tier review --commit "$first"); rc=$?
check "red CI: the review refuses" "2" "$rc"
contains "...saying CI is not complete and green" "not complete and green" "$out"
absent "...and no step ran" "=== step: one" "$out"
check "...and it cleaned up" "$start" "$(state)"
write_gh '42\tin_progress\t\tCI\n' 'build\tin_progress\t\n'
out=$(runner --tier review --commit "$first"); rc=$?
check "CI still running: the review refuses" "2" "$rc"
write_gh '' ''
out=$(runner --tier review --commit "$first"); rc=$?
check "no CI run for the commit: the review refuses" "2" "$rc"
contains "...saying so" "CI has no run" "$out"
out=$(runner --tier review --commit "$first" --no-ci); rc=$?
check "--no-ci goes on" "0" "$rc"
contains "...and says loudly that CI was not read" "CI was NOT read" "$out"
# no gh at all
mkdir "$tmp/nogh"
for t in sh git sed head cut awk mktemp rm mkdir date cat grep pkill kill ps rustc rustup nice getconf env sort wc ls dirname basename chmod cp cmp sha256sum tr sleep printf true; do p=$(command -v "$t") && ln -sf "$p" "$tmp/nogh/$t"; done
(cd "$r" && PATH="$tmp/nogh" GATES_CAPPED=1 /bin/sh tools/gates.sh --tier review --commit "$first" > "$tmp/gates.out" 2>&1); rc=$?
out=$(cat "$tmp/gates.out")
check "without gh the review refuses (unless --no-ci)" "2" "$rc"
contains "...saying CI cannot be read" "CI cannot be read" "$out"
check "...and it cleaned up" "$start" "$(state)"

# --- the commit tier makes no worktree
use_steps 'one\techo in place\n'
out=$(gates --tier commit); rc=$?
check "the commit tier runs where it is" "0" "$rc"
absent "...and makes no worktree" "worktree:" "$out"

[ "$fails" -eq 0 ] && echo "all gates tests passed" && exit 0
echo "$fails failed"
exit 1
