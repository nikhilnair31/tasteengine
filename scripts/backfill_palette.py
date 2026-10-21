"""Recompute palette / dominant / hue for stored media.

The Rust service owns the color pipeline now, so this script no longer touches
images or the database. It drives POST /api/admin/recompute-colors in pages
until the server reports done. Safe to re-run; it only rewrites color columns.

    python3 scripts/backfill_palette.py [--base URL] [--limit N]

Credentials come from TV_USER / TV_PASS in the environment (the service .env):
direnv or `set -a; . .env; set +a` before running.
"""
import argparse
import base64
import json
import os
import sys
import time
import urllib.request

DEFAULT_BASE = "http://127.0.0.1:8770"


def _post(base, path, payload, auth):
    req = urllib.request.Request(
        base.rstrip("/") + path,
        data=json.dumps(payload).encode(),
        headers={
            "Content-Type": "application/json",
            "Authorization": "Basic " + base64.b64encode(auth.encode()).decode(),
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=600) as resp:
        return json.loads(resp.read())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("TE_BASE", DEFAULT_BASE))
    ap.add_argument("--limit", type=int, default=200, help="rows per request")
    args = ap.parse_args()

    user = os.environ.get("TV_USER", "")
    password = os.environ.get("TV_PASS", "")
    if not user or not password:
        sys.exit("TV_USER and TV_PASS must be set (source the service .env)")

    auth = f"{user}:{password}"
    after, scanned, updated, failed = "", 0, 0, 0
    t0 = time.time()
    while True:
        page = _post(args.base, "/api/admin/recompute-colors",
                     {"limit": args.limit, "after": after}, auth)
        if "error" in page:
            sys.exit(f"server error: {page['error']}")
        scanned += page.get("scanned", 0)
        updated += page.get("updated", 0)
        failed += page.get("failed", 0)
        after = page.get("next_after") or after
        print(f"scanned={scanned} updated={updated} failed={failed} "
              f"cursor={after} {time.time() - t0:.0f}s", flush=True)
        if page.get("done") or not page.get("scanned"):
            break

    print(f"DONE scanned={scanned} updated={updated} failed={failed} "
          f"elapsed={time.time() - t0:.0f}s")


if __name__ == "__main__":
    main()
