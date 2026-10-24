"""Compute and store media fingerprints (sha256 + perceptual hash).

Idempotent: rows that already have a sha256 are skipped. Run once after the
db.py migration adds the columns; the cleanup scan then reads hashes from the
DB instead of hashing every file on every scan.

    ./venv/bin/python3 scripts/backfill_media_hashes.py

The fingerprint comes from dedupe._file_fingerprint (exact sha256 over the
whole file + 64-bit phash over a 32x32 grayscale). We hash both thumb and
file when both exist, so a "thumbnail vs full" pair (same picture, different
stored file) still matches on phash.
"""
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from app import config, db, dedupe


def main() -> None:
    db.init()
    rows = db.fetchall(
        "SELECT id, thumb, file FROM media WHERE sha256='' OR phash=0"
    )
    print(f"{len(rows)} media rows to hash", flush=True)
    start = time.time()
    updated = 0
    missing = 0
    for row in rows:
        names = []
        for field in ("thumb", "file"):
            name = (row[field] or "").strip()
            if name:
                names.append(name)
        if not names:
            continue
        entry = None
        for name in names:
            path = os.path.join(config.MEDIA_DIR, os.path.basename(name))
            try:
                stat = os.stat(path)
            except OSError:
                missing += 1
                continue
            fp = dedupe._file_fingerprint(path, stat.st_size, stat.st_mtime_ns)
            if fp[0]:
                entry = fp
                break
        if not entry:
            continue
        db.execute(
            "UPDATE media SET sha256=?, phash=? WHERE id=?",
            (entry[0], entry[1] if entry[1] is not None else 0, row["id"]),
        )
        updated += 1
        if updated % 500 == 0:
            print(f"  {updated}/{len(rows)} ({time.time()-start:.0f}s)", flush=True)
    print(
        f"done: {updated} hashed, {missing} files missing, "
        f"{time.time()-start:.0f}s",
        flush=True,
    )


if __name__ == "__main__":
    main()