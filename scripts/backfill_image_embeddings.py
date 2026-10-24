"""Backfill per-image CLIP vectors, then refresh fused item vectors."""
from __future__ import annotations

import os
import sqlite3
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from app import config, db, embed, search

BATCH_SIZE = int(os.environ.get("TV_EMBED_BATCH", "16"))
VECTOR_BYTES = config.EMBED_DIM * 4


def missing_rows():
    return db.fetchall(
        "SELECT id, file, thumb FROM media "
        "WHERE kind='image' AND (embed IS NULL OR length(embed)<>?) "
        "ORDER BY rowid",
        (VECTOR_BYTES,),
    )


def image_path(row):
    for name in (row["file"], row["thumb"]):
        if name:
            path = os.path.join(config.MEDIA_DIR, name)
            if os.path.isfile(path):
                return path
    return ""


def valid_vector(vec):
    return (
        isinstance(vec, np.ndarray)
        and vec.ndim == 1
        and vec.shape[0] == config.EMBED_DIM
        and np.isfinite(vec).all()
        and float(np.linalg.norm(vec)) > 0
    )


def backfill_images():
    rows = missing_rows()
    total = len(rows)
    updated = 0
    failed = []
    print(f"image rows needing vectors: {total}", flush=True)
    for start in range(0, total, BATCH_SIZE):
        batch = rows[start:start + BATCH_SIZE]
        prepared = [(row, image_path(row)) for row in batch]
        usable = [(row, path) for row, path in prepared if path]
        if usable:
            vectors = embed.embed_images([path for _, path in usable])
            for (row, _), vec in zip(usable, vectors):
                if not valid_vector(vec):
                    failed.append((row["id"], "invalid vector"))
                    continue
                db.execute(
                    "UPDATE media SET embed=? WHERE id=?",
                    (embed.to_blob(vec), row["id"]),
                )
                updated += 1
        failed.extend((row["id"], "missing local file") for row, path in prepared if not path)
        done = min(start + len(batch), total)
        if done % (BATCH_SIZE * 8) == 0 or done == total:
            print(f"images: {done}/{total} updated={updated} failed={len(failed)}", flush=True)
    return total, updated, failed


def main():
    started = time.time()
    total, updated, failed = backfill_images()
    print("refreshing fused item vectors", flush=True)
    fused = search.refresh_item_embeds_batch(batch_size=64)
    remaining = len(missing_rows())
    print(
        f"complete images_total={total} images_updated={updated} "
        f"images_failed={len(failed)} remaining={remaining} "
        f"fused={fused} seconds={time.time() - started:.1f}",
        flush=True,
    )
    if failed:
        for item_id, reason in failed[:100]:
            print(f"failed {item_id}: {reason}", flush=True)
    return 1 if remaining else 0


if __name__ == "__main__":
    sys.exit(main())


# Keep sqlite imported in this standalone script for clear process ownership.
_ = sqlite3
