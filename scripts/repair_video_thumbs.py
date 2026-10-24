#!/usr/bin/env python3
"""Replace old video posters with later frames from their source URLs."""
import argparse
import os
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)
os.environ.setdefault("TV_DATA_DIR", os.path.join(ROOT, "data"))

from app import db, ingest, media, search  # noqa: E402


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--id", action="append", default=[])
    parser.add_argument("--all", action="store_true")
    args = parser.parse_args()
    if args.id:
        marks = ",".join("?" for _ in args.id)
        rows = db.fetchall(
            f"SELECT i.id, i.url, m.id AS media_id FROM items i "
            f"JOIN media m ON m.item_id=i.id WHERE i.kind='video' AND i.id IN ({marks})",
            args.id,
        )
    else:
        rows = db.fetchall(
            "SELECT i.id, i.url, m.id AS media_id FROM items i "
            "JOIN media m ON m.item_id=i.id WHERE i.kind='video' AND i.url LIKE 'http%'"
        )
    for row in rows:
        url = ingest._linkthumb_image(row["url"])
        if not url:
            print(f"FAIL {row['id']} no thumbnail URL")
            continue
        try:
            with urllib.request.urlopen(url, timeout=90) as response:
                data = response.read()
            media.replace_preview(row["media_id"], data)
            search.refresh_item_embed(row["id"])
            print(f"OK {row['id']} {url}")
        except Exception as exc:
            print(f"FAIL {row['id']} {type(exc).__name__}: {exc}")


if __name__ == "__main__":
    main()