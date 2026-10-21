"""Backfill titles for untitled items from their text/author/url."""
import re, sqlite3, sys, os
DB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "data", "vault.db")
con = sqlite3.connect(DB)
con.row_factory = sqlite3.Row
c = con.cursor()

rows = c.execute(
    "SELECT id, url, title, kind, source, platform, author, text FROM items "
    "WHERE title IS NULL OR title=''"
).fetchall()
print("untitled:", len(rows))

def clean_title(raw):
    if not raw:
        return ""
    t = raw.replace("\r", " ").replace("\n", " ").strip()
    t = re.sub(r"\s+", " ", t)
    t = t.strip('"\' ')
    if len(t) > 140:
        t = t[:137].rstrip() + "..."
    return t

def title_for(r):
    t = clean_title(r["text"])
    if t:
        return t
    if r["author"]:
        return clean_title(r["author"])
    if r["url"]:
        host = re.sub(r"^(https?://)?(www\.)?", "", r["url"]).split("/")[0]
        return host or r["url"]
    return "untitled"

fixed = 0
for r in rows:
    t = title_for(r)
    if t and t != "untitled":
        c.execute("UPDATE items SET title=? WHERE id=?", (t, r["id"]))
        fixed += 1

con.commit()
left = c.execute("SELECT COUNT(*) FROM items WHERE title IS NULL OR title=''").fetchone()[0]
print(f"fixed: {fixed}, still untitled: {left}")
for r in c.execute("SELECT id, title, kind, url FROM items WHERE title IS NULL OR title='' LIMIT 8"):
    print("  left:", dict(r))
con.close()