#!/usr/bin/env python3
"""Backfill thumbnails for items that have no media row.

Primary finder: the link-thumb service (localhost:8771) — platform CDN paths
(X/IG/TikTok/Pinterest/YouTube), OG tags, clean hi-res only, no screenshots.
Stores via media.save_media (thumb + palette + hue), then refreshes the
search embed so vibe-search + similar use the image vector.

Usage:
  python3 scripts/backfill_thumbs.py --dry-run
  python3 scripts/backfill_thumbs.py --workers 8
  python3 scripts/backfill_thumbs.py --limit 60
"""
import argparse
import concurrent.futures as cf
import os
import sqlite3
import sys
import time
import urllib.parse
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)
os.environ.setdefault("TV_DATA_DIR", os.path.join(ROOT, "data"))

from app import media, search  # noqa: E402

DB = os.path.join(ROOT, "data", "vault.db")
SVC = os.environ.get("THUMB_SVC", "http://localhost:8771")
WIDTH = 900  # detail modal shows up to ~900px; save_media makes the 720 thumb


def targets(limit=0):
    con = sqlite3.connect(DB)
    con.row_factory = sqlite3.Row
    sql = (
        "SELECT i.id, i.url, i.kind FROM items i "
        "LEFT JOIN media m ON m.item_id=i.id "
        "WHERE m.id IS NULL AND i.url LIKE 'http%'"
    )
    if limit:
        sql += " LIMIT ?"
        rows = con.execute(sql, (limit,)).fetchall()
    else:
        rows = con.execute(sql).fetchall()
    con.close()
    return rows


def fetch_image(url):
    q = urllib.parse.quote(url, safe="")
    try:
        with urllib.request.urlopen(SVC + f"/img?url={q}&w={WIDTH}", timeout=95) as r:
            data = r.read()
        if data and media.sniff_image(data):
            return data, r.headers.get("Content-Type", "image/jpeg")
    except Exception:
        pass
    return None, None


def try_item(id_, url, kind):
    data, ctype = fetch_image(url)
    if not data:
        time.sleep(3)  # transient IG/Reddit rate limits — one retry
        data, ctype = fetch_image(url)
    if not data:
        return (kind, url, False, "no image found")
    try:
        row = media.save_media(id_, data, url=url, mime=ctype)
        search.refresh_item_embed(id_)
        return (kind, url, True, row["thumb"] or row["file"])
    except Exception as e:
        return (kind, url, False, f"{type(e).__name__}: {e}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()

    rows = targets(args.limit)
    print(f"targets: {len(rows)} (any kind, no media, has url)")
    if args.dry_run:
        for r in rows[:30]:
            print(f"  {r['kind']:6s} {r['url'][:90]}")
        return

    ok = fail = 0
    t0 = time.time()
    with cf.ThreadPoolExecutor(max_workers=args.workers) as ex:
        futs = {ex.submit(try_item, r["id"], r["url"], r["kind"]): r for r in rows}
        done = 0
        for fut in cf.as_completed(futs):
            kind, url, success, note = fut.result()
            done += 1
            ok += 1 if success else 0
            fail += 0 if success else 1
            if done % 20 == 0 or done == len(rows):
                print(f"  {done}/{len(rows)} ok={ok} fail={fail} "
                      f"({time.time() - t0:.0f}s)", flush=True)

    print(f"\ndone: ok={ok} fail={fail} ({time.time() - t0:.0f}s)")


if __name__ == "__main__":
    main()