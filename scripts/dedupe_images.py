#!/usr/bin/env python3
"""Find visually identical items via media CLIP embeddings and dedupe them.

Usage:
  python3 scripts/dedupe_images.py --scan          # report duplicate groups only
  python3 scripts/dedupe_images.py --apply --thr 0.99
"""
import argparse
import os
import sqlite3
import sys

import numpy as np

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)
from app import db, media  # noqa: E402

DB = os.path.join(ROOT, "data", "vault.db")


def load_embeds(con):
    rows = con.execute(
        "SELECT m.id AS mid, m.item_id, m.embed, i.title, i.url, i.created_at "
        "FROM media m JOIN items i ON i.id = m.item_id WHERE m.embed IS NOT NULL"
    ).fetchall()
    dim = len(np.frombuffer(rows[0][2], dtype="<f4"))
    M = np.zeros((len(rows), dim), dtype="<f4")
    for k, r in enumerate(rows):
        M[k] = np.frombuffer(r[2], dtype="<f4")
    return rows, M


def find_groups(rows, M, thr):
    N = M.shape[0]
    if N == 0:
        return []
    Mn = M / (np.linalg.norm(M, axis=1, keepdims=True) + 1e-9)
    S = Mn @ Mn.T
    used = set()
    groups = []
    for i in range(N):
        if i in used:
            continue
        j = np.nonzero(S[i] >= thr)[0]
        j = [x for x in j if x not in used]
        if len(j) > 1:
            used.update(j)
            groups.append([int(x) for x in j])
    return groups


def score_row(r):
    # prefer: title present > url present > earlier saved
    return (1 if (r["title"] or "").strip() else 0,
            1 if (r["url"] or "").strip() else 0,
            (r["created_at"] or ""))  # earlier string sorts first; max() picks latest


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scan", action="store_true", help="only report groups")
    ap.add_argument("--apply", action="store_true", help="delete duplicates")
    ap.add_argument("--thr", type=float, default=0.99)
    args = ap.parse_args()

    con = sqlite3.connect(DB)
    con.row_factory = sqlite3.Row
    rows, M = load_embeds(con)

    groups = find_groups(rows, M, args.thr)
    dupe_hits = sum(len(g) - 1 for g in groups)
    print(f"media rows: {len(rows)}, pairs above {args.thr}: "
          f"{dupe_hits} duplicates across {len(groups)} groups")

    if not groups:
        con.close()
        return

    total_del = 0
    for g in groups:
        members = [rows[i] for i in g]
        keeper = max(members, key=score_row)
        dupes = [m for m in members if m["item_id"] != keeper["item_id"]]
        if not dupes:
            continue
        total_del += len(dupes)
        if args.scan:
            print(f"\n  group: keep {keeper['item_id']} | {str(keeper['title'])[:45]!r}")
            for m in dupes:
                print(f"    -del {m['item_id']} | {str(m['title'])[:45]!r} | {str(m['url'])[:60]}")
        else:
            for m in dupes:
                media_rows = db.fetchall("SELECT * FROM media WHERE item_id=?", (m["item_id"],))
                for mr in media_rows:
                    media.delete_media_files(dict(mr))
                db.execute("DELETE FROM items WHERE id=?", (m["item_id"],))
            print(f"  deleted {len(dupes)} from group, kept {keeper['item_id']} | {str(keeper['title'])[:45]!r}")

    if not args.scan and total_del:
        db.execute("DELETE FROM items WHERE id IN ('__noop__')")  # keep connection warm
        print(f"\napply: deleted {total_del} duplicate item(s)")
    elif args.scan:
        print(f"\nscan: would delete {total_del} duplicate item(s)")
    con.close()


if __name__ == "__main__":
    main()