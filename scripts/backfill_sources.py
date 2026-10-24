"""Backfill source = real platform derived from each item's URL.

Fixes the "source=mymind" wart: every item gets source=instagram / x / reddit /
pinterest / youtube / ... derived from the shared link. Items with no URL fall
back to a mapping from their mymind card type.
"""
import os, sqlite3, sys
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)
from app import ingest

DB = os.path.join(ROOT, "data", "vault.db")
con = sqlite3.connect(DB)
con.row_factory = sqlite3.Row
c = con.cursor()

# mymind card type -> platform when there is no URL at all
TYPE_FALLBACK = {
    "XPost": "x",
    "TwitterPost": "x",
    "InstagramReel": "instagram",
    "InstagramPost": "instagram",
    "RedditPost": "reddit",
    "YouTubeVideo": "youtube",
    "PinterestPost": "pinterest",
    "PinterestPin": "pinterest",
    "TikTokVideo": "tiktok",
    "TikTokPost": "tiktok",
    "Screenshot": "image",
    "Image": "image",
    "Video": "video",
    "WebPage": "web",
    "Article": "web",
    "Repository": "github",
    "Embed": "web",
    "Content": "web",
    "Post": "web",
}

rows = c.execute("SELECT id, url, platform FROM items").fetchall()
upd = {}
n_url = n_fb = 0
for r in rows:
    if r["url"]:
        src = ingest.platform_of(r["url"])
        n_url += 1
    else:
        src = TYPE_FALLBACK.get((r["platform"] or "").strip(), "web")
        n_fb += 1
    upd[r["id"]] = src

con.executemany(
    "UPDATE items SET source=? WHERE id=?", [(s, i) for i, s in upd.items()])
con.commit()

print(f"updated {len(upd)} items: {n_url} from URL, {n_fb} fallback (no URL)")
from collections import Counter
cnt = Counter(upd.values())
print("sources:", dict(cnt.most_common(15)))
con.close()