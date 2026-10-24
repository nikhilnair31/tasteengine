"""Backfill missing item_embeds so EVERY item is searchable + can return similar."""
import os, sqlite3, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from app import db, embed

DB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "data", "vault.db")
con = sqlite3.connect(DB)
con.row_factory = sqlite3.Row
c = con.cursor()

rows = c.execute(
    "SELECT i.id, i.title, i.note, i.text, i.url, i.kind FROM items i "
    "LEFT JOIN item_embeds e ON e.item_id=i.id WHERE e.vec IS NULL"
).fetchall()
print("embedless:", len(rows))
if not rows:
    sys.exit(0)

# batch text embeds for link/text rows
textual = [r for r in rows if r["kind"] in ("link", "text")]
imaged = [r for r in rows if r["kind"] == "image"]
print(f"  textual: {len(textual)}, imaged: {len(imaged)}")

if textual:
    texts = []
    for r in textual:
        t = " ".join(x for x in (r["title"], r["note"], r["text"], r["url"]) if x)
        texts.append(t or "link")
    vecs = embed.embed_text(texts)
    con.executemany(
        "INSERT INTO item_embeds (item_id, vec) VALUES (?,?)",
        [(r["id"], embed.to_blob(v)) for r, v in zip(textual, vecs)],
    )
    print(f"  embedded {len(textual)} texts")

if imaged:
    for r in imaged:
        # try local media file first
        m = c.execute(
            "SELECT file FROM media WHERE item_id=? ORDER BY rowid LIMIT 1", (r["id"],)
        ).fetchone()
        path = None
        if m and m["file"]:
            p = os.path.join(os.path.dirname(DB), "media", m["file"])
            if os.path.exists(p):
                path = p
        if path:
            vec = embed.embed_images([path])[0]
        else:
            vec = embed.embed_text(
                [" ".join(x for x in (r["title"], r["note"], r["text"], r["url"]) if x) or "image"]
            )[0]
        c.execute("INSERT INTO item_embeds (item_id, vec) VALUES (?,?)", (r["id"], embed.to_blob(vec)))
    print(f"  embedded {len(imaged)} images")

con.commit()
n = c.execute("SELECT COUNT(*) FROM item_embeds").fetchone()[0]
print("total item_embeds now:", n)
con.close()