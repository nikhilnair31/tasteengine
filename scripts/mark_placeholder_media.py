#!/usr/bin/env python3
"""Mark placeholder media rows: solid/blank thumbs and platform default icons.

Placeholder rows are skipped by item cover and detail media lists. Rule:
- thumb file is visually blank (grayscale stddev < 5), or
- thumb is duplicated across >=3 media rows and icon-sized (<=256px) —
  the reddit default sub/gamepad icons imported with the seed.

Run: python scripts/mark_placeholder_media.py [--apply]
"""
import os
import re
import sqlite3
import sys
from collections import defaultdict

from PIL import Image

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DB = os.path.join(HERE, "data", "vault.db")
MEDIA_DIR = os.path.join(HERE, "data", "media")

APPLY = "--apply" in sys.argv


def main():
    con = sqlite3.connect(DB)
    con.row_factory = sqlite3.Row
    cols = {r["name"] for r in con.execute("PRAGMA table_info(media)")}
    if "placeholder" not in cols:
        con.execute("ALTER TABLE media ADD COLUMN placeholder INTEGER DEFAULT 0")

    rows = con.execute("SELECT id, item_id, thumb FROM media WHERE thumb IS NOT NULL AND thumb != ''").fetchall()

    digests = defaultdict(list)
    blank = {}
    cache = {}
    for r in rows:
        name = r["thumb"]
        path = os.path.join(MEDIA_DIR, name)
        if not os.path.exists(path) or name in cache:
            continue
        with open(path, "rb") as f:
            digest = __import__("hashlib").md5(f.read()).hexdigest()[:12]
        cache[name] = digest
        digests[digest].append((r["id"], r["item_id"], name))
        try:
            img = Image.open(path).convert("L").resize((64, 64))
            px = list(img.getdata())
            mean = sum(px) / len(px)
            sd = (sum((p - mean) ** 2 for p in px) / len(px)) ** 0.5
            blank[name] = sd
        except Exception:
            blank[name] = 255

    mark_ids = set()
    reasons = {}
    for name, sd in blank.items():
        if sd < 5:
            for rid, _, _ in (digests.get(cache.get(name)) or []):
                if rid:
                    mark_ids.add(rid)
                    reasons[rid] = "blank"

    for digest, group in digests.items():
        if len(group) >= 3:
            dims = set()
            for _, _, name in group:
                try:
                    dims.add(Image.open(os.path.join(MEDIA_DIR, name)).size)
                except Exception:
                    pass
            if dims and all(w <= 256 and h <= 256 for w, h in dims):
                for rid, _, name in group:
                    mark_ids.add(rid)
                    reasons[rid] = "shared icon x%d" % len(group)

    # Reddit tall default OG cards: portrait (ratio >= 1.8) with a pure-black
    # top band (the Snoo-bench empty-state image). A real scene never has a
    # perfectly uniform black top third.
    for r in rows:
        rid = r["id"]
        if rid in mark_ids or not r["thumb"]:
            continue
        path = os.path.join(MEDIA_DIR, r["thumb"])
        url = ""
        it = con.execute("SELECT url FROM items WHERE id=?", (r["item_id"],)).fetchone()
        if it:
            url = it["url"] or ""
        if not re.search(r"(?:^|\.)reddit\.com/", url):
            continue
        try:
            img = Image.open(path).convert("L")
            w, h = img.size
            if w <= 0 or h / w < 1.8:
                continue
            top = img.crop((0, 0, w, int(h * 0.30)))
            px = list(top.getdata())
            mean = sum(px) / len(px)
            sd = (sum((v - mean) ** 2 for v in px) / len(px)) ** 0.5
            if mean < 4 and sd < 2:
                mark_ids.add(rid)
                reasons[rid] = "reddit default card"
        except Exception:
            continue

    mark_ids -= {r["id"] for r in con.execute("SELECT id FROM media WHERE placeholder=1")}
    if not mark_ids:
        print("nothing to mark")
        return

    for rid in sorted(mark_ids):
        row = con.execute(
            "SELECT i.url, m.thumb, m.width, m.height FROM media m JOIN items i ON i.id=m.item_id WHERE m.id=?",
            (rid,)).fetchone()
        print("%-16s %-12s %sx%s %s" % (rid[:16], reasons[rid], row["width"], row["height"], (row["url"] or "")[:95]))

    if APPLY:
        con.executemany("UPDATE media SET placeholder=1 WHERE id=?", [(r,) for r in mark_ids])
        con.commit()
        print("marked %d media rows as placeholder" % len(mark_ids))
    else:
        print("dry run: %d rows would be marked (pass --apply to commit)" % len(mark_ids))


if __name__ == "__main__":
    main()