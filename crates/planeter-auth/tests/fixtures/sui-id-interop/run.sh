#!/bin/sh
# RFC 014 D-12: the whole interoperability run, from a tagged sui-id to the evidence, and nothing left behind.
#
#   crates/planeter-auth/tests/fixtures/sui-id-interop/run.sh SUI_ID_REPO TAG EVIDENCE_DIR      (from the repository root)
#
# SUI_ID_REPO is a clone of sui-id (read only: the tag is exported with `git archive`, the clone is not touched); TAG is a release tag, never a
# commit. Everything the run makes lives in ONE work directory, `.git-exclude/tmp/sui-id-interop/` (the export, the build's target directory,
# the throw-away CA, the database and master key, the test users, the processes' PID files), and is removed at the end by its explicit path, and
# the two processes (sui-id and the TLS stand-in) are stopped by their recorded PIDs, on every way out (the end, a failing step, INT, TERM).
# The evidence goes to EVIDENCE_DIR (outside the work directory). The build and the tests run under `tools/run-capped.sh`.
# This script needs: git, tar, openssl, python3, curl, cargo (the toolchain the sui-id tag's `rust-version` names, 1.95, offline: its
# dependencies must be in the cargo cache), and no network access beyond 127.0.0.1.
set -u
REPO=$1 TAG=$2 OUT=$3
ROOT=$(pwd)
FIX=crates/planeter-auth/tests/fixtures/sui-id-interop
W=.git-exclude/tmp/sui-id-interop
TOOLCHAIN=${SUI_ID_TOOLCHAIN:-1.95}
[ -d "$W" ] && { echo "run.sh: $W exists; remove it first (it may belong to a run that is still going)" >&2; exit 2; }
mkdir -p "$OUT" "$W/run" "$W/tls" "$W/src" "$W/evidence" || exit 2

snapshot() { # snapshot FILE: what is running, the disk, the worktrees
  {
    echo "# $1 ($(date -Is))"
    echo "## processes whose command is sui-id (the server), the TLS stand-in, a planeter server, or a stand-in provider"
    ps -eo pid,ppid,etimes,args | awk '$4 ~ /(^|\/)sui-id$/ || $4 ~ /python3/ && /tlsproxy\.py/ || $4 ~ /\/target\/debug\/planeter$/ { print }'
    echo "(end)"
    echo "## df -h /home"; df -h /home | tail -2
    echo "## git worktree list"; git worktree list
    echo "## the work directory"; if [ -d "$W" ]; then du -sh "$W" 2>/dev/null; else echo "absent"; fi
  } > "$OUT/$1.txt" 2>&1
}

cleanup() {
  status=$?
  trap - EXIT INT TERM
  for f in "$W/run/pid.proxy" "$W/run/pid.sui-id"; do
    if [ -f "$f" ]; then pid=$(cat "$f"); kill -CONT "$pid" 2>/dev/null; kill "$pid" 2>/dev/null; fi
  done
  sleep 1
  for f in "$W/run/pid.proxy" "$W/run/pid.sui-id"; do
    if [ -f "$f" ]; then pid=$(cat "$f"); kill -0 "$pid" 2>/dev/null && kill -KILL "$pid" 2>/dev/null; fi
  done
  [ -d "$W/target" ] && du -sh "$W/target" > "$OUT/build-target-size-before-removal.txt" 2>&1
  rm -rf "$W"
  snapshot after
  echo "run.sh: cleanup done (exit $status): processes stopped by PID, $W removed" | tee -a "$OUT/run.log"
  exit "$status"
}
trap cleanup EXIT INT TERM

snapshot before
run_all() {
echo "== sui-id $TAG exported from $REPO (git archive; the clone is not touched)"
git -C "$REPO" rev-parse "$TAG^{commit}"
git -C "$REPO" archive "$TAG" | tar -x -C "$W/src" || return 2

echo "== the build: cargo $TOOLCHAIN, release, offline, locked, its own target directory"
T0=$(date +%s)
( cd "$W/src" && CARGO_TARGET_DIR="$ROOT/$W/target" RUSTUP_TOOLCHAIN="$TOOLCHAIN" "$ROOT/tools/run-capped.sh" -- cargo build --release --offline --locked -p sui-id ) || return 3
echo "build wall time: $(( $(date +%s) - T0 )) s"
BIN="$ROOT/$W/target/release/sui-id"
"$BIN" --version
RUSTUP_TOOLCHAIN="$TOOLCHAIN" rustc --version

echo "== the throw-away CA and the certificate for idp.test (3 days)"
( cd "$W/tls" &&
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout ca.key -out ca.pem -days 3 -subj "/CN=planeter-interop-throwaway-ca" -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null &&
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout idp.key -out idp.csr -subj "/CN=idp.test" 2>/dev/null &&
  printf "subjectAltName=DNS:idp.test,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature\nextendedKeyUsage=serverAuth\nauthorityKeyIdentifier=keyid:always\nsubjectKeyIdentifier=hash\n" > ext.cnf &&
  openssl x509 -req -in idp.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out idp.pem -days 3 -extfile ext.cnf 2>/dev/null &&
  openssl verify -CAfile ca.pem idp.pem && chmod 600 ./*.key ) || return 3

echo "== two free loopback ports; sui-id's configuration (https issuer on the stand-in's port, cookie_secure, access log)"
PORTS=$(python3 -c "
import socket
a=socket.socket();a.bind(('127.0.0.1',0));b=socket.socket();b.bind(('127.0.0.1',0))
print(a.getsockname()[1], b.getsockname()[1])")
UP=${PORTS% *}; PP=${PORTS#* }
echo "UP=$UP PP=$PP" > "$W/run/ports.env"
( cd "$W/run" && "$BIN" --print-sample-config > sui-id.toml.sample &&
  sed -e "s|^listen_addr = .*|listen_addr = \"127.0.0.1:$UP\"|" -e "s|^issuer = .*|issuer = \"https://idp.test:$PP\"|" -e "s|^cookie_secure = .*|cookie_secure = true|" -e "s|^access_log = .*|access_log = true|" sui-id.toml.sample > sui-id.toml &&
  chmod 600 sui-id.toml && grep -n "^listen_addr\|^issuer\|^cookie_secure\|^access_log" sui-id.toml &&
  "$BIN" setup --config ./sui-id.toml --admin-username admin > setup.out 2>&1 && sed -e 's/password: .*/password: <hidden>/' setup.out ) || return 3

echo "== start sui-id and the TLS stand-in (PIDs recorded)"
( cd "$W/run" || exit 1; nohup "$BIN" --config ./sui-id.toml > sui-id.log 2>&1 & echo $! > "$ROOT/$W/run/pid.sui-id" )
nohup python3 "$FIX/tlsproxy.py" "$PP" "$UP" "$W/tls/idp.pem" "$W/tls/idp.key" "$W/run/ctl" > "$W/run/proxy.log" 2>&1 &
echo $! > "$W/run/pid.proxy"
i=0; until curl -s -o /dev/null --cacert "$W/tls/ca.pem" --resolve "idp.test:$PP:127.0.0.1" "https://idp.test:$PP/.well-known/openid-configuration"; do i=$((i+1)); [ $i -gt 100 ] && { echo "the provider did not come up" >&2; return 3; }; sleep 0.2; done
echo "sui-id pid $(cat "$W/run/pid.sui-id"), stand-in pid $(cat "$W/run/pid.proxy")"

ADMIN="python3 -I $ROOT/$FIX/admin.py $ROOT/$W"
j() { python3 -c "import json,sys; print(json.loads(sys.stdin.read())['$1'])"; }

echo "== one ordinary user with an email, one without (the administrator's console creates them; the recovery link comes from the CLI because the administrator has no second factor)"
mkuser() { # mkuser NAME EMAIL PASSWORD -> prints the user id
  uid=$(EMAIL="$2" $ADMIN create-user "$1" | j user_id)
  link=$( cd "$W/run" && "$BIN" admin issue-recovery-link --username "$1" --reason "interop test user" --config ./sui-id.toml 2>&1 | awk '/dropped in transit/ { getline; print; exit }')
  $ADMIN set-password "$link" "$3" > /dev/null
  echo "$uid"
}
PW1="Interop-User-Pw-$(python3 -c 'import secrets;print(secrets.token_hex(6))')"
PW2="Interop-Test-Pw-$(python3 -c 'import secrets;print(secrets.token_hex(6))')"
UID1=$(mkuser interop interop@idp.test "$PW1")
UID2=$(mkuser testuser "" "$PW2")
echo "users created: interop ($UID1, with an email), testuser ($UID2, without)"

echo "== the OIDC clients: a public one (PKCE only), a confidential one (client_secret_basic); redirect URI https://forge.example/login/oidc/callback"
PUB=$($ADMIN create-client planeter-public-pkce public)
CONF=$($ADMIN create-client planeter-confidential confidential)
echo "public: $(echo "$PUB" | python3 -c 'import json,sys; d=json.load(sys.stdin); d["client_secret"]=None; print(d)')"
echo "confidential: $(echo "$CONF" | python3 -c 'import json,sys; d=json.load(sys.stdin); d["client_secret"]="<hidden>"; print(d)')"
PUBID=$(echo "$PUB" | j client_id); CONFID=$(echo "$CONF" | j client_id); CONFSECRET=$(echo "$CONF" | j client_secret)
$ADMIN client-scopes "$CONFID" "openid profile email offline_access" > /dev/null
echo "(the confidential client was also allowed offline_access, so that check 8 can measure the provider's claim about refresh tokens)"

echo "== a finding, reproduced: the console form's 'Confidential client' box UNCHECKED (a browser sends no field at all)"
UNCHECKED=$($ADMIN create-client planeter-unchecked-box public-unchecked-box)
echo "$UNCHECKED" | python3 -c 'import json,sys; d=json.load(sys.stdin); print("created with the box unchecked: listed as", d["listed_kind"], "and a client secret was shown:", d["client_secret"] is not None)' | tee "$W/evidence/finding-unchecked-box.txt"

cat > "$W/run/interop.env" <<ENV
export PLANETER_SUI_ID_URL=https://idp.test:$PP
export PLANETER_SUI_ID_CA=$ROOT/$W/tls/ca.pem
export PLANETER_SUI_ID_PUBLIC_CLIENT=$PUBID
export PLANETER_SUI_ID_CONFIDENTIAL_CLIENT=$CONFID
export PLANETER_SUI_ID_CONFIDENTIAL_SECRET=$CONFSECRET
export PLANETER_SUI_ID_REDIRECT_URI=https://forge.example/login/oidc/callback
export PLANETER_SUI_ID_USER=interop
export PLANETER_SUI_ID_PASSWORD=$PW1
export PLANETER_SUI_ID_USER_ID=$UID1
export PLANETER_SUI_ID_NOEMAIL_USER=testuser
export PLANETER_SUI_ID_NOEMAIL_PASSWORD=$PW2
export PLANETER_SUI_ID_CONTROL_DIR=$ROOT/$W/run/ctl
export PLANETER_SUI_ID_ADMIN="$ADMIN"
export PLANETER_SUI_ID_EVIDENCE=$ROOT/$W/evidence
ENV
chmod 600 "$W/run/interop.env"

echo "== the tests: cargo test -p planeter-auth --test interop_sui_id -- --ignored --test-threads=1 (under tools/run-capped.sh --cpu 200)"
. "$W/run/interop.env"
T0=$(date +%s)
tools/run-capped.sh --cpu 200 -- cargo test -p planeter-auth --locked --test interop_sui_id -- --ignored --test-threads=1 --nocapture > "$W/evidence/full-run.log" 2>&1
RC=$?
echo "tests exit $RC, wall $(( $(date +%s) - T0 )) s"
grep -E "^test result|\[CORRECTED\]|\[NOT SHOWN\]|panicked" "$W/evidence/full-run.log"
echo "CONFIRMED: $(grep -c '\[CONFIRMED\]' "$W/evidence/full-run.log")"

echo "== the evidence is copied out (the proxy's request log has paths, statuses and sizes; no body, no query, no secret)"
cp "$W/evidence/"*.txt "$W/evidence/full-run.log" "$OUT/" 2>/dev/null
cp "$W/run/ctl/requests.log" "$OUT/provider-request-log.jsonl"
sed -e 's/\x1b\[[0-9;]*m//g' "$W/run/sui-id.log" | cut -c1-300 > "$OUT/sui-id-server.log"
ls "$OUT"
return "$RC"
}
run_all >> "$OUT/run.log" 2>&1
exit $?
