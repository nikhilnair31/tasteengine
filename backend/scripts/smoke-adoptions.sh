#!/usr/bin/env bash
# Contract smoke test for the four adopted behaviours:
#   1. declarative source registry
#   2. per-source gate that persists cooldowns across a restart
#   3. API tokens (bearer auth, revoke)
#   4. lossless originals (stored bytes are byte-identical to the API hash)
#
# Runs a SECOND server against a COPY of the database, so a live instance is
# never touched. Never kills by process name: the live service runs the same
# binary name.
#
#   TE_DATA_DIR=~/taste-engine/data PORT=8931 ./scripts/smoke-adoptions.sh
#
# Requires: curl, python3, sha256sum.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN="${TE_BIN:-$ROOT/target/release/taste-engine-rs}"
TE_CLI="${TE_CLI:-$ROOT/target/release/te}"
SRC_DATA="${TE_DATA_DIR:-$ROOT/../data}"
PORT="${PORT:-8931}"
BASE="http://127.0.0.1:$PORT"
B="${TV_USER:-admin}:${TV_PASS:-change-me}"
SM="${TE_SMOKE_DIR:-$(mktemp -d)}"
FAIL=0

say() { printf '%s\n' "$*"; }
check() { if [ "$1" = "$2" ]; then say "  ok   $3 ($1)"; else say "  FAIL $3 (got $1, want $2)"; FAIL=1; fi; }

[ -x "$BIN" ] || { say "no binary at $BIN — run: cargo build --release --bins"; exit 2; }
[ -f "$SRC_DATA/vault.db" ] || { say "no vault.db in $SRC_DATA — set TE_DATA_DIR"; exit 2; }

mkdir -p "$SM/data"
cp "$SRC_DATA/vault.db" "$SM/data/"
[ -d "$SRC_DATA/media" ] && ln -sfn "$SRC_DATA/media" "$SM/data/media"
say "smoke dir: $SM"

start() {
  env TV_BASE="$ROOT" TV_DATA_DIR="$SM/data" TV_FRONTEND_DIR="$ROOT/../web" \
      TV_USER="${TV_USER:-admin}" TV_PASS="${TV_PASS:-change-me}" PORT="$PORT" \
      nohup "$BIN" >> "$SM/server.log" 2>&1 &
  PID=$!
  for _ in $(seq 1 60); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/healthz")" = "200" ] && return 0
    sleep 0.25
  done
  say "server did not come up; log:"; tail -5 "$SM/server.log"; exit 1
}

start
say "server pid=$PID"

say "1. registry"
N=$(curl -s -u "$B" "$BASE/api/sources" | python3 -c 'import json,sys;print(json.load(sys.stdin)["count"])' 2>/dev/null)
[ "${N:-0}" -ge 10 ] && say "  ok   13 sources listed ($N)" || { say "  FAIL registry ($N)"; FAIL=1; }

say "2. gate starts closed"
S=$(curl -s -u "$B" "$BASE/api/sources/gates" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sources"][0]["state"])' 2>/dev/null)
check "$S" "closed" "fresh gate state"

say "3. a platform 429 becomes a persisted cooldown"
X=$(curl -s -u "$B" -X POST "$BASE/api/sources/x/report" -H 'content-type: application/json' -d '{"status":429,"retry_after":120}' \
  | python3 -c 'import json,sys;x=[s for s in json.load(sys.stdin)["sources"] if s["source"]=="x"][0];print(x["state"],x["reason"],x["retry_in_ms"]>0)' 2>/dev/null)
case "$X" in "open rate_limited True") say "  ok   $X" ;; *) say "  FAIL gate after 429: $X"; FAIL=1 ;; esac

say "4. a save against a cooling-down source is refused"
C=$(curl -s -o "$SM/save.json" -w '%{http_code}' -u "$B" -X POST "$BASE/api/items" -H 'content-type: application/json' -d '{"url":"https://x.com/someone/status/1234567890"}')
check "$C" "429" "save on gated source"

say "5. api token create"
curl -s -u "$B" -X POST "$BASE/api/tokens" -H 'content-type: application/json' -d '{"name":"smoke"}' > "$SM/tok.json"
TOK=$(python3 -c 'import json;print(json.load(open("'"$SM"'/tok.json"))["token"])' 2>/dev/null)
TID=$(python3 -c 'import json;print(json.load(open("'"$SM"'/tok.json"))["id"])' 2>/dev/null)
case "$TOK" in te_*) say "  ok   token created (${TOK:0:8}…)" ;; *) say "  FAIL token create"; FAIL=1 ;; esac

say "6. bearer auth"
C=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $TOK" "$BASE/api/items?limit=1")
check "$C" "200" "bearer accepted"
C=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/items?limit=1")
check "$C" "401" "no credentials"

say "7. revoke"
C=$(curl -s -o /dev/null -w '%{http_code}' -u "$B" -X DELETE "$BASE/api/tokens/$TID")
check "$C" "200" "revoke"
C=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $TOK" "$BASE/api/items?limit=1")
check "$C" "401" "bearer after revoke"

say "8. lossless original"
MID=$(python3 - "$SM/data/vault.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute("SELECT id FROM media WHERE kind='image' AND file<>'' LIMIT 1").fetchone()
print(r[0] if r else "")
PY
)
if [ -n "$MID" ]; then
  curl -s -u "$B" "$BASE/api/media/$MID/origin" > "$SM/origin.json"
  API=$(python3 -c 'import json;print(json.load(open("'"$SM"'/origin.json"))["sha256"])' 2>/dev/null)
  FILE=$(python3 -c 'import json;print(json.load(open("'"$SM"'/origin.json"))["file"])' 2>/dev/null)
  DISK=$(sha256sum "$SRC_DATA/media/$FILE" 2>/dev/null | cut -d' ' -f1)
  check "$DISK" "$API" "stored bytes match the reported hash"
else
  say "  skip no image media in this vault"
fi

say "9. cooldown survives a restart"
kill "$PID"; sleep 1
start
X=$(curl -s -u "$B" "$BASE/api/sources/gates" | python3 -c 'import json,sys;x=[s for s in json.load(sys.stdin)["sources"] if s["source"]=="x"][0];print(x["state"],x["reason"])' 2>/dev/null)
check "$X" "open rate_limited" "gate state after restart"

say "10. manual resume"
S=$(curl -s -u "$B" -X POST "$BASE/api/sources/x/resume" | python3 -c 'import json,sys;x=[s for s in json.load(sys.stdin)["sources"] if s["source"]=="x"][0];print(x["state"])' 2>/dev/null)
check "$S" "closed" "resume"

say "11. cli"
if [ -x "$TE_CLI" ]; then
  TE_URL="$BASE" TE_BASIC="$B" "$TE_CLI" sources > "$SM/cli.txt" 2>&1
  L=$(wc -l < "$SM/cli.txt")
  [ "$L" -ge 10 ] && say "  ok   te sources printed $L lines" || { say "  FAIL cli ($L lines)"; FAIL=1; }
  TE_URL="$BASE" TE_BASIC="$B" "$TE_CLI" ls --limit 2 >> "$SM/cli.txt" 2>&1
else
  say "  skip no te binary"
fi

kill "$PID"; sleep 1
say ""
[ "$FAIL" = "0" ] && say "ALL CHECKS PASSED" || say "SOME CHECKS FAILED"
exit "$FAIL"
