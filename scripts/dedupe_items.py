#!/usr/bin/env python3
"""Scan or remove cards that show the same saved content."""
import argparse
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from app import db, dedupe, media, search  # noqa: E402


def load_records():
    records = []
    for row in db.fetchall("SELECT * FROM items ORDER BY saved_at, rowid"):
        media_rows = db.fetchall("SELECT * FROM media WHERE item_id=? ORDER BY rowid", (row["id"],))
        tag_count = db.fetchone("SELECT COUNT(*) c FROM tags WHERE item_id=?", (row["id"],))["c"]
        explanation = db.fetchone("SELECT status FROM explanations WHERE item_id=?", (row["id"],))
        records.append(dedupe.record_from_row(
            row,
            media_rows=media_rows,
            tag_count=tag_count,
            explanation_done=bool(explanation and explanation["status"] == "done"),
        ))
    return records


def delete_record(record):
    for row in db.fetchall("SELECT * FROM media WHERE item_id=?", (record["id"],)):
        media.delete_media_files(dict(row))
    db.execute("DELETE FROM items WHERE id=?", (record["id"],))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--scan", action="store_true", help="report duplicate groups")
    parser.add_argument("--apply", action="store_true", help="remove duplicate records")
    parser.add_argument("--json", action="store_true", help="write machine-readable output")
    args = parser.parse_args()
    if not args.scan and not args.apply:
        parser.error("use --scan or --apply")

    db.init()
    groups = dedupe.duplicate_groups(load_records())
    report = {
        "summary": dedupe.duplicate_summary(groups),
        "groups": [],
    }
    for group in groups:
        keeper = max(group, key=dedupe.keeper_score)
        report["groups"].append({
            "keeper": keeper["id"],
            "items": [
                {
                    "id": item["id"],
                    "title": item["title"],
                    "url": item["url"],
                    "kind": item["kind"],
                    "source": item["source"],
                }
                for item in group
            ],
        })

    if args.apply:
        deleted = 0
        for group in groups:
            keeper = max(group, key=dedupe.keeper_score)
            for item in group:
                if item["id"] != keeper["id"]:
                    delete_record(item)
                    deleted += 1
        search.invalidate()
        report["deleted"] = deleted

    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return
    print(json.dumps(report["summary"], ensure_ascii=False))
    for group in report["groups"]:
        print("KEEP", group["keeper"])
        for item in group["items"]:
            marker = "*" if item["id"] == group["keeper"] else "-"
            print(marker, item["id"], item["title"][:100], item["url"])
    if args.apply:
        print("deleted", report["deleted"])


if __name__ == "__main__":
    main()
