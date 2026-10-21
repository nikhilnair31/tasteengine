-- Base schema, ported from legacy/python/db.py (SCHEMA).
-- Every statement is IF NOT EXISTS: applying it to an existing vault is a no-op.
CREATE TABLE IF NOT EXISTS items (
  id TEXT PRIMARY KEY,
  url TEXT DEFAULT '',
  dedupe_key TEXT DEFAULT '',
  title TEXT DEFAULT '',
  kind TEXT DEFAULT 'image',
  source TEXT DEFAULT '',
  platform TEXT DEFAULT '',
  author TEXT DEFAULT '',
  text TEXT DEFAULT '',
  note TEXT DEFAULT '',
  extra TEXT DEFAULT '{}',
  created_at TEXT DEFAULT '',
  saved_at TEXT DEFAULT (datetime('now')),
  deleted_at TEXT DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_items_saved ON items(saved_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_source ON items(source);
CREATE INDEX IF NOT EXISTS idx_items_kind ON items(kind);

CREATE TABLE IF NOT EXISTS media (
  id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  kind TEXT DEFAULT 'image',
  mime TEXT DEFAULT '',
  original_name TEXT DEFAULT '',
  transcript TEXT DEFAULT '',
  file TEXT DEFAULT '',
  thumb TEXT DEFAULT '',
  width INTEGER DEFAULT 0,
  height INTEGER DEFAULT 0,
  duration REAL DEFAULT 0,
  palette TEXT DEFAULT '[]',
  dominant TEXT DEFAULT '',
  hue TEXT DEFAULT '',
  embed BLOB,
  placeholder INTEGER DEFAULT 0,
  sha256 TEXT DEFAULT '',
  phash INTEGER DEFAULT 0,
  ocr_text TEXT DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_media_item ON media(item_id);

CREATE TABLE IF NOT EXISTS item_embeds (
  item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  vec BLOB
);

CREATE TABLE IF NOT EXISTS tags (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  tag TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tags ON tags(item_id, tag);

CREATE TABLE IF NOT EXISTS explanations (
  item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  status TEXT DEFAULT 'pending',
  data TEXT DEFAULT '{}',
  updated_at TEXT DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_expl_status ON explanations(status);

CREATE TABLE IF NOT EXISTS jobs (
  item_id TEXT PRIMARY KEY,
  state TEXT,
  error TEXT,
  updated_at TEXT
);

CREATE TABLE IF NOT EXISTS user_settings (
  username TEXT PRIMARY KEY,
  settings TEXT NOT NULL DEFAULT '{}',
  updated_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS item_interactions (
  username TEXT NOT NULL,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  clicks INTEGER NOT NULL DEFAULT 0,
  last_clicked TEXT DEFAULT '',
  PRIMARY KEY (username, item_id)
);
CREATE INDEX IF NOT EXISTS idx_item_interactions_item ON item_interactions(item_id);

CREATE TABLE IF NOT EXISTS inspire_jobs (
  job_id TEXT PRIMARY KEY,
  state TEXT DEFAULT 'queued',
  params TEXT DEFAULT '{}',
  result TEXT DEFAULT '',
  error TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS preview_jobs (
  job_id TEXT PRIMARY KEY,
  inspire_job_id TEXT DEFAULT '',
  state TEXT DEFAULT 'queued',
  params TEXT DEFAULT '{}',
  result TEXT DEFAULT '',
  error TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS followups (
  id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  kind TEXT NOT NULL DEFAULT 'todo',
  title TEXT DEFAULT '',
  url TEXT DEFAULT '',
  status TEXT NOT NULL DEFAULT 'open',
  confidence REAL DEFAULT 1.0,
  why TEXT DEFAULT '',
  body TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_followups_status ON followups(status);



CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(title, text, content='items', content_rowid='rowid');

CREATE VIRTUAL TABLE IF NOT EXISTS items_fts_stem USING fts5(title, text, content='items', content_rowid='rowid', tokenize='porter unicode61');

CREATE TRIGGER IF NOT EXISTS items_ai AFTER INSERT ON items BEGIN
  INSERT INTO items_fts(rowid, title, text) VALUES (new.rowid, new.title, new.text);
END;
CREATE TRIGGER IF NOT EXISTS items_ad AFTER DELETE ON items BEGIN
  INSERT INTO items_fts(items_fts, rowid, title, text) VALUES ('delete', old.rowid, old.title, old.text);
END;
CREATE TRIGGER IF NOT EXISTS items_au AFTER UPDATE ON items BEGIN
  INSERT INTO items_fts(items_fts, rowid, title, text) VALUES ('delete', old.rowid, old.title, old.text);
  INSERT INTO items_fts(rowid, title, text) VALUES (new.rowid, new.title, new.text);
END;

CREATE TRIGGER IF NOT EXISTS items_stem_ai AFTER INSERT ON items BEGIN
  INSERT INTO items_fts_stem(rowid, title, text) VALUES (new.rowid, new.title, new.text);
END;
CREATE TRIGGER IF NOT EXISTS items_stem_ad AFTER DELETE ON items BEGIN
  INSERT INTO items_fts_stem(items_fts_stem, rowid, title, text) VALUES ('delete', old.rowid, old.title, old.text);
END;
CREATE TRIGGER IF NOT EXISTS items_stem_au AFTER UPDATE ON items BEGIN
  INSERT INTO items_fts_stem(items_fts_stem, rowid, title, text) VALUES ('delete', old.rowid, old.title, old.text);
  INSERT INTO items_fts_stem(rowid, title, text) VALUES (new.rowid, new.title, new.text);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS expl_fts USING fts5(item_id UNINDEXED, body, tokenize='porter unicode61');
