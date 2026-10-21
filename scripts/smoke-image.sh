#!/usr/bin/env bash
# Contract test against a running Taste Engine server.
#
#   TV_USER=a TV_PASS=b ./scripts/smoke-image.sh                       # http://127.0.0.1:8770
#   TV_URL=http://host:8770 TV_USER=a TV_PASS=b ./scripts/smoke-image.sh
#   TV_CONTAINER=te-ci ./scripts/smoke-image.sh     # also reads the container logs
#   TV_STRICT_MODELS=1 ./scripts/smoke-image.sh     # search must return a hit (needs weights)
#
# Exits non-zero if any check fails. Changes nothing outside the throwaway data
# dir it is pointed at: it saves a note and an image, uses and revokes an API
# token, then trashes what it wrote.
set -uo pipefail

URL="${TV_URL:-http://127.0.0.1:8770}"
USER_="${TV_USER:-ci}"
PASS_="${TV_PASS:-ci}"
CONTAINER="${TV_CONTAINER:-}"
STRICT_MODELS="${TV_STRICT_MODELS:-0}"

pass=0
fail=0
failed=()
TMPBODY=""

cleanup() { [ -n "$TMPBODY" ] && rm -f "$TMPBODY" "$TMPBODY.2" 2>/dev/null; }
trap cleanup EXIT

ok()  { pass=$((pass + 1)); printf '  ok    %s\n' "$1"; }
bad() { fail=$((fail + 1)); failed+=("$1"); printf '  FAIL  %s — %s\n' "$1" "$2"; }

probe() { # probe <method> <path> [curl args...]
  local method=$1 path=$2
  shift 2
  TMPBODY=$(mktemp)
  local out
  out=$(curl -sS --max-time 60 -X "$method" -o "$TMPBODY" -w '%{http_code}\t%{content_type}' "$@" "$URL$path" 2>/dev/null) \
    || out=$'000\tcurl failed'
  STATUS=${out%%$'\t'*}
  CTYPE=${out#*$'\t'}
}

want_status() { # want_status <name> <expected>
  if [ "$STATUS" = "$2" ]; then ok "$1"; else bad "$1" "status $STATUS, want $2 (body: $(head -c 120 "$TMPBODY" 2>/dev/null | tr -d '\n'))"; fi
}

want_json() { # want_json <name>
  case "$CTYPE" in
    application/json*) ok "$1 is json" ;;
    *) bad "$1" "content-type ${CTYPE:-none} — the SPA fallback answering again?" ;;
  esac
}

want_expr() { # want_expr <name> <python expression over d>
  local name=$1 expr=$2
  if ! command -v python3 >/dev/null 2>&1; then
    printf '  skip  %s (no python3)\n' "$name"
    return
  fi
  # The expressions are string literals in this file, never data from the server
  # or the environment, so eval is bounded by what a reviewer can read here.
  if python3 - "$TMPBODY" "$expr" <<'PY'
import builtins, json, sys
d = json.load(open(sys.argv[1]))
sys.exit(0 if eval(sys.argv[2], {"__builtins__": builtins}, {"d": d}) else 1)
PY
  then
    ok "$name"
  else
    bad "$name" "assertion failed: $expr"
  fi
}

json_field() { # json_field <key> -> value or empty
  python3 -c "import json;d=json.load(open('$TMPBODY'));v=d.get('$1');print(v if v is not None else '')" 2>/dev/null
}

AUTH=(-u "$USER_:$PASS_")
say() { printf '%s\n' "$*"; }

say "smoke: $URL"
say ""

for i in $(seq 1 60); do
  probe GET /healthz
  [ "$STATUS" = "200" ] && break
  sleep 1
done
want_status "healthz" 200

# ---------- the SPA ships in the image ----------
probe GET /
want_status "GET /" 200
case "$CTYPE" in text/html*) ok "/ is html" ;; *) bad "/ is html" "content-type $CTYPE" ;; esac
if grep -qi '<title>' "$TMPBODY"; then ok "/ has a title"; else bad "/ has a title" "no <title> in $(stat -c%s "$TMPBODY" 2>/dev/null) bytes"; fi
probe GET /app.js
want_status "app.js" 200
size=$(stat -c%s "$TMPBODY" 2>/dev/null || echo 0)
[ "$size" -gt 100000 ] && ok "app.js is $size bytes" || bad "app.js is $size bytes" "expected > 100KB"
probe GET /style.css
want_status "style.css" 200

# ---------- auth ----------
probe GET /api/items
want_status "no credentials -> 401" 401
probe GET /api/items -u "wrong:wrong"
want_status "wrong credentials -> 401" 401

# ---------- read routes: status, then content type, then shape ----------
probe GET "/api/items?limit=5" "${AUTH[@]}"
want_status "/api/items" 200
want_json "/api/items"
want_expr "/api/items has items[] and total" "isinstance(d.get('items'), list) and 'total' in d"

probe GET /api/sources "${AUTH[@]}"
want_status "/api/sources" 200
want_json "/api/sources"
want_expr "/api/sources lists 13 platforms" "d.get('count') == 13 and len(d.get('sources', [])) == 13"
want_expr "/api/sources has the named platforms" "{'x','instagram','pinterest','reddit','youtube','tiktok','threads','bluesky','tumblr','arena','cosmos','steam','spotify'} <= {s['id'] for s in d['sources']}"

probe GET /api/sources/gates "${AUTH[@]}"
want_status "/api/sources/gates" 200
want_json "/api/sources/gates"
want_expr "/api/sources/gates covers every source" "len(d.get('sources', [])) == 13 and all('state' in s for s in d['sources'])"

probe GET /api/meta "${AUTH[@]}"
want_status "/api/meta" 200
want_json "/api/meta"
want_expr "/api/meta has the facet keys" "{'hues','kinds','platforms','sources','tags','total'} <= set(d)"

probe GET /api/settings "${AUTH[@]}"
want_status "/api/settings" 200
want_json "/api/settings"

probe GET /api/followups "${AUTH[@]}"
want_status "/api/followups" 200
want_json "/api/followups"
want_expr "/api/followups has items[]" "isinstance(d.get('items'), list)"

probe GET /api/trash "${AUTH[@]}"
want_status "/api/trash" 200
want_json "/api/trash"
want_expr "/api/trash has items[] and a retention" "isinstance(d.get('items'), list) and 'trash_days' in d"

probe GET /api/duplicates "${AUTH[@]}"
want_status "/api/duplicates" 200
want_json "/api/duplicates"

probe GET /api/inspire/meta "${AUTH[@]}"
want_status "/api/inspire/meta" 200
want_json "/api/inspire/meta"

probe GET /api/tokens "${AUTH[@]}"
want_status "/api/tokens" 200
want_json "/api/tokens"

# ---------- write paths ----------
probe POST /api/notes "${AUTH[@]}" -F 'text=ci smoke note about a stone lantern at dusk'
want_status "save a note" 200
want_json "save a note"
NOTE_ID=$(json_field id)
if [ -n "$NOTE_ID" ]; then ok "note has an id ($NOTE_ID)"; else bad "note has an id" "no id in response"; fi

probe GET "/api/items?limit=20" "${AUTH[@]}"
want_expr "the note is in the grid" "any(i.get('id') == '$NOTE_ID' for i in d.get('items', []))"

# a real picture: a diagonal gradient with blocks, so the blank-frame detector
# in mediaio sees content and not a placeholder
IMG=$(mktemp /tmp/te-smoke-XXXXXX.png)
python3 - "$IMG" <<'PY' 2>/dev/null || true
import struct, sys, zlib

w = h = 160
rows = b""
for y in range(h):
    row = bytearray(b"\x00")
    for x in range(w):
        r = (x * 255) // w
        g = (y * 255) // h
        b = 130 if (40 < x < 120 and 30 < y < 110) else 40
        row += bytes((r, g, b))
    rows += bytes(row)


def chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(rows, 6))
    + chunk(b"IEND", b"")
)
open(sys.argv[1], "wb").write(png)
PY
probe POST /api/items/upload "${AUTH[@]}" \
  -F 'payload={"title":"ci smoke image","source":"note","kind":"image"}' \
  -F "file=@$IMG;type=image/png"
want_status "upload an image" 200
want_json "upload an image"
IMG_ID=$(json_field id)
rm -f "$IMG"
[ -n "$IMG_ID" ] && ok "upload returned an id ($IMG_ID)" || bad "upload returned an id" "none"

# the cover is written by the pipeline, so give it a moment
cover=""
for i in $(seq 1 20); do
  probe GET "/api/items/$IMG_ID" "${AUTH[@]}"
  cover=$(json_field cover)
  [ -n "$cover" ] && break
  sleep 1
done
[ -n "$cover" ] && ok "uploaded image got a cover" || bad "uploaded image got a cover" "cover stayed empty (pipeline) id=$IMG_ID"

probe GET "/api/items/$IMG_ID/similar" "${AUTH[@]}"
want_status "similar for the new item" 200
want_json "similar for the new item"

# ---------- api tokens ----------
probe POST /api/tokens "${AUTH[@]}" -H 'Content-Type: application/json' -d '{"name":"smoke"}'
want_status "create an api token" 200
TOKEN=$(json_field token)
TOKEN_ID=$(json_field id)
case "$TOKEN" in te_*) ok "token looks like a token" ;; *) bad "token looks like a token" "got ${TOKEN:0:8}…" ;; esac

probe GET "/api/items?limit=1" -H "Authorization: Bearer $TOKEN"
want_status "bearer token is accepted" 200

probe DELETE "/api/tokens/$TOKEN_ID" "${AUTH[@]}"
case "$STATUS" in 200|204) ok "revoke the token" ;; *) bad "revoke the token" "status $STATUS" ;; esac

probe GET "/api/items?limit=1" -H "Authorization: Bearer $TOKEN"
want_status "a revoked token is rejected" 401

# ---------- traps a client depends on ----------
probe GET /api/this-route-does-not-exist "${AUTH[@]}"
want_status "an unknown api route is 404" 404
case "$CTYPE" in
  application/json*) ok "an unknown api route answers json, not the SPA" ;;
  *) bad "an unknown api route answers json, not the SPA" "content-type ${CTYPE:-none}" ;;
esac

probe GET "/media/../../etc/passwd" "${AUTH[@]}" --path-as-is
if grep -q 'root:x:0' "$TMPBODY" 2>/dev/null; then
  bad "path traversal is refused" "served /etc/passwd"
else
  ok "path traversal is refused (no /etc/passwd)"
fi

# ---------- writes did not wedge the service ----------
probe GET /healthz
want_status "healthz still up after writes" 200

# ---------- optional: the real model path ----------
if [ "$STRICT_MODELS" = "1" ]; then
  found=""
  for i in $(seq 1 20); do
    probe GET "/api/search?q=stone%20lantern&limit=5" "${AUTH[@]}"
    if python3 -c "import json;d=json.load(open('$TMPBODY'));raise SystemExit(0 if d.get('items') else 1)" 2>/dev/null; then
      found=1
      break
    fi
    sleep 2
  done
  [ -n "$found" ] && ok "search returns the note (weights loaded)" \
    || bad "search returns the note (weights loaded)" "no hits after 40s — weights missing, or the embed job never finished"
fi

# ---------- cleanup: the note goes to the trash last, so the search check above
# still had something to find ----------
probe DELETE "/api/items/$NOTE_ID" "${AUTH[@]}"
want_status "trash the note" 200
probe GET /api/trash "${AUTH[@]}"
want_expr "the trashed note is in the trash view" "any(i.get('id') == '$NOTE_ID' for i in d.get('items', []))"

# ---------- container-level facts ----------
if [ -n "$CONTAINER" ] && command -v docker >/dev/null 2>&1; then
  logs=$(docker logs "$CONTAINER" 2>&1 || true)
  if printf '%s' "$logs" | grep -qE 'panicked|no such column|SQL prepare failed'; then
    bad "no panics or schema errors in the log" "$(printf '%s' "$logs" | grep -E 'panicked|no such column|SQL prepare failed' | head -3 | tr '\n' ' ')"
  else
    ok "no panics or schema errors in the log"
  fi
  uid=$(docker exec "$CONTAINER" id -u 2>/dev/null || echo "?")
  [ "$uid" = "1000" ] && ok "runs as uid 1000 (not root)" || bad "runs as uid 1000 (not root)" "uid=$uid"
  # the healthcheck has a 25s start-period, so a young container still says
  # "starting"; give it time rather than failing the build on a race
  health=""
  for i in $(seq 1 12); do
    health=$(docker inspect -f '{{.State.Health.Status}}' "$CONTAINER" 2>/dev/null)
    [ "$health" = "healthy" ] && break
    sleep 5
  done
  [ "$health" = "healthy" ] \
    && ok "docker healthcheck reports healthy" \
    || bad "docker healthcheck reports healthy" "status=$health after 60s"
fi

say ""
say "$pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
  printf 'failed: %s\n' "${failed[*]}"
  exit 1
fi
