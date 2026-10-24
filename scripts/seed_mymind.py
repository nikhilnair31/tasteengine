#!/usr/bin/env python3
"""Seed the vault from the local mymind library.

Reads ~/.hermes/data/mymind_lib.jsonl (cards), reuses the already-CLIP-embedded
vectors (~/.hermes/data/mymind_embeddings.db) and the cached preview files
(~/.hermes/data/mymind_previews/) — no re-downloading, no re-embedding.

  python3 scripts/seed_mymind.py            # full import (idempotent)
  python3 scripts/seed_mymind.py --dry-run  # report what would happen
"""
import argparse
import json
import os
import sys
import time

import numpy as np

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

os.environ.setdefault("TV_DATA_DIR", os.path.join(ROOT, "data"))

from app import config, db, dedupe, embed, ingest, media, search  # noqa: E402

LIB = os.path.expanduser("~/.hermes/data/mymind_lib.jsonl")
EMB_DB = os.path.expanduser("~/.hermes/data/mymind_embeddings.db")
PREVIEWS = os.path.expanduser("~/.hermes/data/mymind_previews")

PREVIEW_EXTS = (".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif", ".mp4", ".webm", ".mov")


def load_vectors():
    vecs = {}
    if not os.path.isfile(EMB_DB):
        return vecs
    import sqlite3

    conn = sqlite3.connect(EMB_DB)
    rows = conn.execute("SELECT card_id, vec FROM embeddings").fetchall()
    conn.close()
    for cid, blob in rows:
        vecs[cid] = blob
    return vecs


def index_previews():
    idx = {}
    if not os.path.isdir(PREVIEWS):
        return idx
    for fn in os.listdir(PREVIEWS):
        card_id, _, ext = fn.rpartition(".")
        if ext and card_id:
            idx[card_id] = os.path.join(PREVIEWS, fn)
    return idx


def kind_of(card):
    media_url = (card.get("media") or "").strip()
    low = media_url.lower()
    if any(low.endswith(e) for e in (".mp4", ".webm", ".mov")):
        return "video", media_url
    if any(low.endswith(e) for e in (".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif", ".svg")):
        return "image", media_url
    if card.get("description"):
        return "text", media_url
    return "link", media_url


MIND_TYPE_SOURCE = {
    "XPost": "x", "TwitterPost": "x",
    "InstagramReel": "instagram", "InstagramPost": "instagram",
    "RedditPost": "reddit", "YouTubeVideo": "youtube",
    "PinterestPost": "pinterest", "PinterestPin": "pinterest",
    "TikTokVideo": "tiktok", "TikTokPost": "tiktok",
    "Screenshot": "image", "Image": "image", "Video": "video",
    "WebPage": "web", "Article": "web", "Repository": "github",
    "Embed": "web", "Content": "web", "Post": "web",
}

def source_of(card):
    u = (card.get("source") or "").strip()
    if u:
        return ingest.platform_of(u)
    t = (card.get("type") or "WebPage").strip()
    return MIND_TYPE_SOURCE.get(t, "web")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--limit", type=int, default=0, help="max cards to import (0 = all)")
    args = ap.parse_args()

    if not os.path.isfile(LIB):
        print(f"missing library: {LIB}")
        sys.exit(1)
    db.init()
    vectors = load_vectors()
    previews = index_previews()

    # Backfill: items imported by a previous run that have media but no
    # embedding (e.g. interrupted run) get their vector reused now.
    backfilled = 0
    rows = db.fetchall(
        "SELECT i.id, json_extract(i.extra, '$.mymind_id') AS mid FROM items i "
        "WHERE i.extra LIKE '%mymind_id%' AND EXISTS (SELECT 1 FROM media m "
        "WHERE m.item_id = i.id AND m.embed IS NULL)")
    for r in rows:
        blob = vectors.get(r["mid"])
        if not blob or len(blob) != config.EMBED_DIM * 4:
            continue
        mids = db.fetchall("SELECT id FROM media WHERE item_id=? AND embed IS NULL", (r["id"],))
        for (mid,) in mids:
            vec = embed.norm(np.frombuffer(blob, dtype="<f4").copy())
            db.execute("UPDATE media SET embed=? WHERE id=?", (embed.to_blob(vec), mid))
        search.refresh_item_embed(r["id"])
        backfilled += 1
    if backfilled:
        print(f"backfilled embeddings for {backfilled} items", flush=True)

    existing = {
        r for (r,) in db.fetchall("SELECT json_extract(extra, '$.mymind_id') FROM items "
                                  "WHERE extra LIKE '%mymind_id%'")
    }
    existing_keys = {
        r["dedupe_key"] for r in db.fetchall("SELECT dedupe_key FROM items WHERE dedupe_key<>''")
    }

    added = 0
    skipped = 0
    no_preview = 0
    embedded = 0
    t0 = time.time()

    for i, line in enumerate(open(LIB, encoding="utf-8")):
        line = line.strip()
        if not line:
            continue
        try:
            card = json.loads(line)
        except Exception:
            continue
        mid = card.get("id")
        if not mid:
            continue
        if mid in existing:
            skipped += 1
            continue
        if args.limit and added >= args.limit:
            break

        kind, media_url = kind_of(card)
        title = (card.get("title") or "").strip()[:400]
        text = (card.get("description") or "").strip()[:4000]
        url = (card.get("source") or "").strip()[:2000]
        created = (card.get("created") or "")[:40]
        platform = (card.get("type") or "WebPage").strip()[:16]
        dedupe_key = dedupe.canonical_source_key(url)

        if args.dry_run:
            added += 1
            continue

        item_id = os.urandom(8).hex()
        extra = {"mymind_id": mid, "no_explain": True}
        db.execute(
            "INSERT INTO items (id, url, dedupe_key, title, kind, source, platform, text, note, extra, created_at) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?)",
            (item_id, url, dedupe_key, title, kind, source_of(card), platform, text, "", json.dumps(extra), created),
        )
        for tag in list(card.get("tags") or [])[:12]:
            tag = str(tag).lower().strip()[:40]
            if tag:
                db.execute("INSERT INTO tags(item_id, tag) VALUES(?,?)", (item_id, tag))
        added += 1

        # Media from the cached preview (no re-download).
        pv = previews.get(mid)
        if pv and os.path.isfile(pv):
            with open(pv, "rb") as f:
                data = f.read()
            if len(data) > 200:
                try:
                    row = media.save_media(item_id, data)
                    vec_blob = vectors.get(mid)
                    if vec_blob and len(vec_blob) == config.EMBED_DIM * 4:
                        vec = embed.norm(np.frombuffer(vec_blob, dtype="<f4").copy())
                        db.execute("UPDATE media SET embed=? WHERE id=?", (embed.to_blob(vec), row["id"]))
                        embedded += 1
                    search.refresh_item_embed(item_id)
                except Exception as e:
                    print(f"  media failed {mid}: {type(e).__name__}: {e}")
        else:
            no_preview += 1
            if title or text:
                search.refresh_item_embed(item_id)

        if added % 200 == 0:
            print(f"  {added} imported, {embedded} embedded, {no_preview} no-preview "
                  f"({time.time() - t0:.0f}s)", flush=True)

    print(f"\ndone: +{added} items, {embedded} reused embeddings, "
          f"{no_preview} cards without cached preview, {skipped} already present "
          f"({time.time() - t0:.0f}s)")


if __name__ == "__main__":
    main()