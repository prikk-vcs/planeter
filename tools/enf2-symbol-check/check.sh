#!/usr/bin/env bash
#
# ENF-2 (RFC 001 §0a / LAY-4 / INV-2): the DEFAULT build must contain NO forge history-signing symbol.
#
# planeter is a keyless forge — it holds no history-signing key and never seals history itself. Any code
# that could sign history lives behind the `forge-seal` cargo feature, which is OFF by default. This
# check builds the default profile (no `forge-seal`) and greps the resulting artifacts' symbols; it fails
# if a forge-signer symbol is present. It passes trivially today (no seal code exists) and stays as the
# structural guard that keeps INV-2 true as write code arrives.
#
# The reserved marker: any forge-seal signer MUST name its entry point so its symbol contains the string
# below (e.g. `forge_seal_sign_history`), so this check can find it if it ever ships in a default build.
set -euo pipefail

PATTERN='forge_seal_sign'

# workspace root (this script lives at tools/enf2-symbol-check/)
cd "$(dirname "$0")/../.."

echo "ENF-2: building the default profile (no forge-seal feature)…"
cargo build --release --workspace --locked

if ! command -v nm >/dev/null 2>&1; then
  echo "ENF-2 ERROR: 'nm' (binutils) not found; cannot inspect symbols." >&2
  exit 2
fi

shopt -s nullglob
artifacts=(
  target/release/planeter
  target/release/deps/planeter_core-*.rlib
  target/release/deps/libplaneter_core-*.rlib
)

scanned=0
found=0
for a in "${artifacts[@]}"; do
  [ -e "$a" ] || continue
  scanned=$((scanned + 1))
  if nm "$a" 2>/dev/null | grep -qi "$PATTERN"; then
    echo "ENF-2 FAIL: forge-signer symbol (matching '$PATTERN') present in $a"
    found=1
  fi
done

if [ "$scanned" -eq 0 ]; then
  echo "ENF-2 ERROR: no artifacts found to scan (did the build produce target/release/*?)." >&2
  exit 2
fi

if [ "$found" -ne 0 ]; then
  echo "ENF-2: a keyless forge must not ship a history-signing symbol in the default build (INV-2)." >&2
  exit 1
fi

echo "ENF-2 OK: no forge-signer symbol in the default build ($scanned artifact(s) scanned)."
