#!/usr/bin/env python3
"""Queue missing explainers and drive them with EXPLAIN_WORKERS parallel workers.

Usage:
  python3 backfill_explainers.py [--limit N] [--force] [--workers N]

Default: queue up to 500 missing explainers, then block until the queue drains.
Set EXPLAIN_WORKERS higher (e.g. 4-6) to clear bulk-import backlogs fast.

Safe against a live server? No — run as a STANDALONE process with the same DB;
it spawns its own workers. Stop the uvicorn service first, or run in parallel
only if you accept queue contention with the server workers.
"""
import argparse
import importlib
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

os.environ.setdefault("TV_DATA_DIR", os.path.join(ROOT, "data"))

from app import config, explain  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--limit", type=int, default=500)
ap.add_argument("--force", action="store_true")
ap.add_argument("--workers", type=int, default=4)
args = ap.parse_args()

os.environ["EXPLAIN_WORKERS"] = str(args.workers)

# Re-read config with the env we just set, then re-import explain so its
# module-level worker spawn picks up the new count.
importlib.reload(config)
importlib.reload(explain)

queued = explain.backfill_explainers(limit=args.limit, force=args.force)
print(f"queued {queued} explainers with {args.workers} workers", flush=True)

while not explain._queue.empty() or len(explain._pending) > 0:
    time.sleep(5)
    print(f"  pending: {len(explain._pending)}", flush=True)

print("done", flush=True)