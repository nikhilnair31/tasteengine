#!/usr/bin/env bash
# Smoke test: API reachable, auth works, CLIP search answers, MCP lists tools.
set -euo pipefail
cd "$(dirname "$0")/.."

BASE="${BASE:-http://127.0.0.1:8770}"
USER_NAME="$(grep -m1 '^TV_USER=' .env | cut -d= -f2-)"
USER_PASS="$(grep -m1 '^TV_PASS=' .env | cut -d= -f2-)"

code() { curl -s -o /dev/null -w '%{http_code}' -u "$USER_NAME:$USER_PASS" "$1"; }

echo "spa     $(code "$BASE/")"
echo "noauth  $(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/meta")   (expect 401)"
echo "meta    $(code "$BASE/api/meta")"
echo "search  $(code "$BASE/api/search?q=dark&limit=1")"
hermes mcp test taste-engine 2>&1 | grep -E 'Connected|Tools discovered' || true
