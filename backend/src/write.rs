//! Write + CRUD layer — port of routes.ensure_item / save_item_upload /
//! create_note / tags / interact / note / delete / trash / duplicates /
//! cleanup / settings PUT / followups POST / share / import / admin backfill.
//! Pipeline enqueue writes the jobs row and queues in-memory; the real
//! worker lands with the pipeline port (jobs stay 'queued' until then).

use std::sync::{Mutex, OnceLock};

use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::vault;

pub fn now_sql() -> String {
    // mirror of sqlite datetime('now'): "YYYY-MM-DD HH:MM:SS" UTC
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    // civil-from-days (Howard Hinnant's algorithm)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

pub fn token_hex8() -> String {
    // python secrets.token_hex(8): 16 hex chars from 8 random bytes
    let mut bytes = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn env_num(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

pub fn trash_days() -> i64 {
    env_num("TV_TRASH_DAYS", 7)
}

// python html.unescape for the entities that appear in real titles/bodies;
// numeric refs handled exactly, named refs cover the common set.
fn unescape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = value[i..].find(';') {
                let ent = &value[i + 1..i + semi];
                let decoded = decode_entity(ent);
                if let Some(d) = decoded {
                    out.push_str(&d);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = value[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_entity(ent: &str) -> Option<String> {
    if let Some(rest) = ent.strip_prefix('#') {
        let code = if let Some(hexpart) = rest.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hexpart, 10).ok().or(None);
            u32::from_str_radix(hexpart, 16).ok()
        } else {
            rest.parse::<u32>().ok()
        };
        return code.and_then(char::from_u32).map(|c| c.to_string());
    }
    let named: &[(&str, &str)] = &[
        ("amp", "&"), ("lt", "<"), ("gt", ">"), ("quot", "\""), ("apos", "'"),
        ("nbsp", "\u{a0}"), ("mdash", "\u{2014}"), ("ndash", "\u{2013}"),
        ("hellip", "\u{2026}"), ("rsquo", "\u{2019}"), ("lsquo", "\u{2018}"),
        ("rdquo", "\u{201d}"), ("ldquo", "\u{201c}"), ("copy", "\u{a9}"),
        ("reg", "\u{ae}"), ("trade", "\u{2122}"), ("deg", "\u{b0}"),
        ("eacute", "\u{e9}"), ("egrave", "\u{e8}"), ("agrave", "\u{e0}"),
        ("ccedil", "\u{e7}"), ("uuml", "\u{fc}"), ("ouml", "\u{f6}"),
        ("auml", "\u{e4}"), ("szlig", "\u{df}"), ("ntilde", "\u{f1}"),
        ("times", "\u{d7}"), ("middot", "\u{b7}"), ("bull", "\u{2022}"),
        ("prime", "\u{2032}"), ("dagger", "\u{2020}"), ("sect", "\u{a7}"),
        ("para", "\u{b6}"), ("laquo", "\u{ab}"), ("raquo", "\u{bb}"),
        ("pound", "\u{a3}"), ("euro", "\u{20ac}"), ("yen", "\u{a5}"),
        ("cent", "\u{a2}"),
    ];
    for (name, val) in named {
        if *name == ent {
            return Some((*val).to_string());
        }
    }
    None
}

// ---------- ingest.platform_of ----------

pub fn platform_of(url: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"https?://(?:www\.|m\.|mobile\.)?([^/]+)").unwrap());
    let host = re
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_lowercase())
        .unwrap_or_default();
    let hosts: &[(&str, &[&str])] = &[
        ("youtube", &["youtube.com", "youtu.be"]),
        ("instagram", &["instagram.com"]),
        ("pinterest", &["pinterest.com", "pin.it"]),
        ("tiktok", &["tiktok.com"]),
        ("reddit", &["reddit.com", "redd.it"]),
        ("x", &["x.com", "twitter.com", "fxtwitter.com", "fixupx.com", "vxtwitter.com"]),
        ("vimeo", &["vimeo.com"]),
        ("imgur", &["imgur.com", "i.imgur.com"]),
        ("giphy", &["giphy.com", "media.giphy.com"]),
        ("yarn", &["yarn.co"]),
        ("github", &["github.com"]),
        ("steam", &["store.steampowered.com", "steamcommunity.com"]),
        ("spotify", &["spotify.com", "open.spotify.com"]),
        ("etsy", &["etsy.com"]),
        ("amazon", &["amazon.com", "amazon.co.uk", "amazon.ca", "amazon.de", "amazon.in",
            "amazon.co.jp", "amazon.com.au", "amazon.fr", "amazon.it", "amazon.es",
            "amazon.nl", "amazon.sg", "amazon.ae", "a.co", "amzn.to"]),
        ("alibaba", &["alibaba.com", "1688.com"]),
        ("aliexpress", &["aliexpress.com", "aliexpress.us"]),
        ("temu", &["temu.com"]),
        ("seeed", &["seeedstudio.com", "seeed.cc", "seedstudio.com"]),
    ];
    for (name, list) in hosts {
        for h in *list {
            if host == *h || host.ends_with(&format!(".{h}")) {
                return (*name).to_string();
            }
        }
    }
    if host.is_empty() {
        "web".to_string()
    } else {
        host
    }
}

// ---------- jobs queue (delegates to the pipeline workers) ----------

pub fn enqueue(conn: &Connection, item_id: &str) {
    crate::pipeline::enqueue(conn, item_id);
}

pub fn enqueue_staged(conn: &Connection, item_id: &str) {
    crate::pipeline::enqueue_staged(conn, item_id);
}

pub fn set_job(conn: &Connection, item_id: &str, state: &str) {
    let _ = conn.execute(
        "INSERT INTO jobs(item_id, state, error, updated_at) VALUES(?,?,NULL,?) \
         ON CONFLICT(item_id) DO UPDATE SET state=excluded.state, error=NULL, updated_at=excluded.updated_at",
        rusqlite::params![item_id, state, now_sql()],
    );
}

pub fn create_jobs_table(conn: &Connection) {
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS jobs(item_id TEXT PRIMARY KEY, state TEXT, error TEXT, updated_at TEXT)",
        [],
    );
}

// ---------- explain queue (worker stub until the explain port) ----------

static EXPLAIN_PENDING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn explain_pending() -> &'static Mutex<std::collections::HashSet<String>> {
    EXPLAIN_PENDING.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

pub fn queue_explain(conn: &Connection, item_id: &str, force: bool) {
    crate::explain::queue_explain(conn, item_id, force);
}

pub fn backfill_explainers(conn: &Connection, limit: i64, force: bool) -> i64 {
    crate::explain::backfill_explainers(conn, limit, force)
}

// ---------- helpers ----------

pub fn norm_ws_pub(value: &str) -> String {
    norm_ws(value)
}

fn norm_ws(value: &str) -> String {
    value.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_placeholder_title(value: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:x post(?:\s+\d+)?|x\.com|twitter\.com|instagram\.com|reddit\.com|reddit - the heart of the internet|reddit link|saved image|shared file|[a-z0-9.-]+\.[a-z]{2,})$").unwrap()
    });
    re.is_match(&norm_ws(value))
}

fn first_note_url(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?i)https?://[^\s<>"']+"#).unwrap());
    match re.find(text) {
        Some(m) => {
            let mut u = m.as_str().to_string();
            while u.ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '}']) {
                u.pop();
            }
            u.chars().take(2000).collect()
        }
        None => String::new(),
    }
}

fn summary_of(conn: &Connection, item_id: &str, created: bool) -> Value {
    let rows = vault::fetch_rows(conn, "SELECT * FROM items WHERE id=?", &[json!(item_id)]);
    match rows.into_iter().next() {
        None => json!({"id": item_id, "created": created}),
        Some(row) => {
            let cards = vault::rows_to_cards(conn, std::slice::from_ref(&row));
            let mut d = match cards.into_iter().next() {
                Some(v) => v.as_object().cloned().unwrap_or_default(),
                None => Map::new(),
            };
            d.insert("created".into(), json!(created));
            Value::Object(d)
        }
    }
}

// ---------- ensure_item ----------

pub fn ensure_item(conn: &Connection, item: &Value) -> Result<Value, (u16, String)> {
    let obj = item.as_object().cloned().unwrap_or_default();
    let get_str = |k: &str| obj.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let item_id = token_hex8();
    let mut title = get_str("title").trim().to_string();
    title = title.chars().take(400).collect();
    let mut text = get_str("text").trim().to_string();
    let thread = get_str("thread").trim().to_string();
    if !thread.is_empty() && !text.contains(&thread) {
        text = format!("{text}\n\n{thread}").trim().to_string();
    }
    text = text.chars().take(4000).collect();
    let url = get_str("url").trim().to_string();
    let url = url.chars().take(2000).collect::<String>();
    let note = get_str("note").trim().to_string();
    let note = note.chars().take(2000).collect::<String>();
    let mut kind = get_str("kind").trim().to_string();
    kind = kind.chars().take(16).collect();
    if !["image", "video", "text", "link"].contains(&kind.as_str()) {
        kind = "image".into();
    }

    let platform = if url.is_empty() { "manual".to_string() } else { platform_of(&url) };
    let mut source = match obj.get("source") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => platform.clone(),
    };
    let _ = &mut source;
    // Reddit page titles arrive as "Post title : r/subreddit"; drop the suffix.
    if platform == "reddit" && !title.is_empty() {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"\s*:\s*r/[A-Za-z0-9_]+$").unwrap());
        title = re.replace(&title, "").trim().to_string();
    }
    let dedupe_key = vault::canonical_source_key(&url);

    if !dedupe_key.is_empty() {
        let existing = vault::fetch_rows(
            conn,
            "SELECT * FROM items WHERE dedupe_key=? OR (dedupe_key='' AND url=?) \
             ORDER BY saved_at, rowid LIMIT 1",
            &[json!(dedupe_key), json!(url)],
        )
        .into_iter()
        .next();
        if let Some(existing) = existing {
            let deleted = existing.get("deleted_at").and_then(|v| v.as_str()).unwrap_or("");
            if !deleted.is_empty() {
                let id = existing.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let _ = conn.execute("UPDATE items SET deleted_at='' WHERE id=?", [&id]);
                return Ok(summary_of(conn, &id, false));
        }
        let id = existing.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let xextra: Value = existing
                .get("extra")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(json!({}));
            let fresh = format!("{title} {text}").trim().to_string();
            let has_mymind = xextra.get("mymind_id").map(|v| !v.is_null()).unwrap_or(false);
            if has_mymind && fresh.len() > 10 {
                let mut ups: Vec<String> = Vec::new();
                let mut pars: Vec<Value> = Vec::new();
                if !title.is_empty() {
                    ups.push("title=?".into());
                    pars.push(json!(title.chars().take(400).collect::<String>()));
                }
                if !text.is_empty() {
                    ups.push("text=?".into());
                    pars.push(json!(text.chars().take(4000).collect::<String>()));
                }
                let author = get_str("author");
                if !author.is_empty() {
                    ups.push("author=?".into());
                    pars.push(json!(author.chars().take(200).collect::<String>()));
                }
                if !ups.is_empty() {
                    pars.push(json!(id));
                    let _ = conn.execute(
                        &format!("UPDATE items SET {} WHERE id=?", ups.join(", ")),
                        rusqlite::params_from_iter(pars.iter().map(vault::to_rusqlite_value)),
                    );
                }
                // python spawns pipeline.refresh_item in a thread; the worker
                // lands with the pipeline port — the response is unchanged.
            }
            return Ok(summary_of(conn, &id, false));
        }
    }

    let image_urls: Vec<Value> = obj
        .get("image_urls")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|u| u.as_str().map(|s| !s.is_empty()).unwrap_or(false)).take(8).cloned().collect())
        .unwrap_or_default();
    let mut video_url = get_str("video_url");
    if !url.is_empty() && video_url.is_empty() && kind == "video" {
        video_url = url.clone();
    }
    if title.is_empty() && text.is_empty() && url.is_empty() && image_urls.is_empty() && video_url.is_empty() {
        return Err((422, "nothing to save".into()));
    }

    let author = get_str("author");
    let extra = json!({"image_urls": image_urls, "video_url": video_url});
    let _ = conn.execute(
        "INSERT INTO items (id, url, dedupe_key, title, kind, source, platform, author, text, note, extra) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            item_id, url, dedupe_key, title, kind, source, platform,
            author.chars().take(200).collect::<String>(), text, note,
            serde_json::to_string(&extra).unwrap_or_default(),
        ],
    );
    for tag in obj
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().take(12).cloned().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let tag = match tag.as_str() {
            Some(s) => s.to_string(),
            None => tag.to_string(),
        };
        let tag = tag.trim().to_lowercase();
        let tag: String = tag.chars().take(40).collect();
        if !tag.is_empty() {
            let _ = conn.execute("INSERT INTO tags(item_id, tag) VALUES(?,?)", rusqlite::params![item_id, tag]);
        }
    }
    enqueue(conn, &item_id);
    Ok(summary_of(conn, &item_id, true))
}

// ---------- notes ----------

#[derive(Clone)]
pub struct StagedFile {
    pub path: String,
    pub name: String,
    pub mime: String,
}

pub fn stage_bytes_public(name: &str, mime: &str, data: &[u8]) -> StagedFile {
    stage_bytes(name, mime, data)
}

pub const MAX_IDS_PUBLIC: usize = 100;

fn stage_bytes(name: &str, mime: &str, data: &[u8]) -> StagedFile {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let path = format!("{dir}/taste-note-{}.upload", token_hex8());
    let _ = std::fs::write(&path, data);
    StagedFile {
        path,
        name: name.chars().take(255).collect(),
        mime: mime.to_string(),
    }
}

pub fn create_note(
    conn: &Connection,
    text_in: &str,
    title_in: &str,
    files: &[StagedFile],
) -> Result<Value, (u16, String)> {
    let note_max_text = env_num("TV_NOTE_MAX_TEXT", 100 * 1024);
    let media_keep = env_num("TV_MEDIA_KEEP_BYTES", 64 * 1024 * 1024);
    let note_max_upload = env_num("TV_NOTE_MAX_UPLOAD_BYTES", 256 * 1024 * 1024);
    let text = text_in.to_string();
    if text.len() as i64 > note_max_text {
        return Err((413, format!("note text too large ({note_max_text} character cap)")));
    }
    let mut staged: Vec<StagedFile> = Vec::new();
    let mut total: i64 = 0;
    for f in files {
        let len = std::fs::metadata(&f.path).map(|m| m.len() as i64).unwrap_or(0);
        if len > media_keep {
            return Err((413, format!("attachment too large (64MB cap): {}", f.name)));
        }
        if len == 0 {
            continue;
        }
        total += len;
        if total > note_max_upload {
            return Err((413, "note attachments too large (256MB cap)".into()));
        }
        staged.push(StagedFile {
            path: f.path.clone(),
            name: f.name.clone(),
            mime: f.mime.clone(),
        });
    }

    if text.trim().is_empty() && staged.is_empty() {
        return Err((422, "write a note or attach a file".into()));
    }

    let url = first_note_url(&text);
    // A bare-URL note IS a link save: run it through the normal pipeline.
    if !url.is_empty() && staged.is_empty() {
        let mut remainder = text.replacen(&url, "", 1).trim().to_string();
        if remainder.is_empty() {
            remainder = title_in.trim().to_string();
        }
        remainder = remainder.trim().to_string();
        if remainder == url {
            remainder = String::new();
        }
        return ensure_item(
            conn,
            &json!({
                "url": url,
                "title": remainder.chars().take(400).collect::<String>(),
                "text": remainder.chars().take(4000).collect::<String>(),
                "kind": "link",
            }),
        );
    }

    let item_id = token_hex8();
    let first_line = text
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    let title = if !title_in.trim().is_empty() {
        title_in.trim().to_string()
    } else if !first_line.is_empty() {
        first_line
    } else if !staged.is_empty() {
        "note with attachments".to_string()
    } else {
        "note".to_string()
    };
    let title: String = title.trim().chars().take(400).collect();
    let url2 = first_note_url(&text);
    let staged_json: Vec<Value> = staged
        .iter()
        .map(|s| json!({"path": s.path, "name": s.name, "mime": s.mime}))
        .collect();
    let extra = json!({"note": true, "_staged_files": staged_json});
    let res = conn.execute(
        "INSERT INTO items (id, url, dedupe_key, title, kind, source, platform, author, text, note, extra) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            item_id, url2, "", title, "text", "note", "note", "", text, "",
            serde_json::to_string(&extra).unwrap_or_default(),
        ],
    );
    if res.is_err() {
        for s in &staged {
            let _ = std::fs::remove_file(&s.path);
        }
        return Err((500, "insert failed".into()));
    }
    enqueue(conn, &item_id);
    Ok(item_detail_value(conn, &item_id))
}

fn item_detail_value(conn: &Connection, item_id: &str) -> Value {
    crate::vault::item_detail(conn, item_id).unwrap_or(json!({"id": item_id}))
}

// ---------- upload (extension image save) ----------

pub fn save_upload(
    conn: &Connection,
    payload: &str,
    file: Option<(&str, &str, &[u8])>, // (filename, content_type, bytes)
) -> Result<Value, (u16, String)> {
    let note_max_upload = env_num("TV_NOTE_MAX_UPLOAD_BYTES", 256 * 1024 * 1024);
    let item: Value = if payload.is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(payload)
            .map_err(|_| (422, "payload must be JSON".to_string()))?
    };
    if !item.is_object() {
        return Err((422, "payload must be a JSON object".into()));
    }
    let obj = item.as_object().unwrap();
    let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let url: String = url.chars().take(2000).collect();
    let mut title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    title = title.chars().take(400).collect();
    let tags: Vec<String> = obj
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .take(12)
                .filter_map(|t| t.as_str().map(|s| s.trim().to_lowercase()))
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(40).collect())
                .collect()
        })
        .unwrap_or_default();

    let mut staged: Vec<StagedFile> = Vec::new();
    if let Some((filename, content_type, data)) = file {
        if data.len() as i64 > note_max_upload {
            return Err((413, "image too large".into()));
        }
        if data.len() > 200 {
            let name = if filename.is_empty() { "saved-image" } else { filename };
            staged.push(stage_bytes(
                name,
                content_type.split(';').next().unwrap_or("").trim().to_lowercase().as_str(),
                data,
            ));
        }
    }

    let item_id = token_hex8();
    let dedupe_key = vault::canonical_source_key(&url);
    if !dedupe_key.is_empty() {
        let existing = vault::fetch_rows(
            conn,
            "SELECT id, deleted_at FROM items WHERE (deleted_at IS NULL OR deleted_at='') \
             AND (dedupe_key=? OR (dedupe_key='' AND url=?)) ORDER BY saved_at, rowid LIMIT 1",
            &[json!(dedupe_key), json!(url)],
        )
        .into_iter()
        .next();
        if let Some(existing) = existing {
            if !staged.is_empty() {
                for s in &staged {
                    let _ = std::fs::remove_file(&s.path);
                }
            }
            let id = existing.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            return Ok(summary_of(conn, &id, false));
        }
    }

    let mut extra = Map::new();
    extra.insert(
        "image_urls".into(),
        json!(if !url.is_empty() && staged.is_empty() { vec![url.clone()] } else { Vec::<String>::new() }),
    );
    extra.insert("video_url".into(), json!(""));
    if !staged.is_empty() {
        extra.insert(
            "_staged_files".into(),
            json!(staged.iter().map(|s| json!({"path": s.path, "name": s.name, "mime": s.mime})).collect::<Vec<_>>()),
        );
    }
    if url.is_empty() && staged.is_empty() && title.is_empty() {
        return Err((422, "nothing to save".into()));
    }

    let _ = conn.execute(
        "INSERT INTO items (id, url, dedupe_key, title, kind, source, platform, text, note, extra) \
         VALUES (?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            item_id, url, dedupe_key, if title.is_empty() { "saved image".to_string() } else { title },
            "image", "extension", "web", "", "",
            Value::Object(extra).to_string(),
        ],
    );
    for tag in tags {
        let _ = conn.execute("INSERT INTO tags(item_id, tag) VALUES(?,?)", rusqlite::params![item_id, tag]);
    }
    enqueue_staged(conn, &item_id);
    Ok(summary_of(conn, &item_id, true))
}

// ---------- tags / interact / note / delete ----------

pub fn add_tags(conn: &Connection, item_id: &str, body: &Value) -> Result<Value, (u16, String)> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Err((404, "not found".into()));
    }
    let raw = body.get("tags").cloned().unwrap_or(json!([]));
    if !raw.is_array() {
        return Err((422, "tags must be a list".into()));
    }
    let mut existing: std::collections::HashSet<String> = vault::fetch_rows(
        conn,
        "SELECT tag FROM tags WHERE item_id=?",
        &[json!(item_id)],
    )
    .into_iter()
    .filter_map(|r| r.get("tag").and_then(|v| v.as_str()).map(|s| s.to_string()))
    .collect();
    for t in raw.as_array().unwrap().iter().take(12) {
        let tag = match t.as_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let tag: String = tag.trim().to_lowercase().chars().take(40).collect();
        if !tag.is_empty() && !existing.contains(&tag) {
            let _ = conn.execute("INSERT INTO tags(item_id, tag) VALUES(?,?)", rusqlite::params![item_id, tag]);
            existing.insert(tag);
        }
    }
    let mut sorted: Vec<String> = existing.into_iter().collect();
    sorted.sort();
    Ok(json!({"ok": true, "tags": sorted}))
}

pub fn interact(conn: &Connection, username: &str, item_id: &str) -> Result<Value, (u16, String)> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Err((404, "not found".into()));
    }
    let _ = conn.execute(
        "INSERT INTO item_interactions(username, item_id, clicks, last_clicked) \
         VALUES(?,?,1,datetime('now')) \
         ON CONFLICT(username, item_id) DO UPDATE SET \
           clicks=item_interactions.clicks + 1, last_clicked=datetime('now')",
        rusqlite::params![username, item_id],
    );
    let (clicks, last): (i64, String) = conn
        .query_row(
            "SELECT clicks, last_clicked FROM item_interactions WHERE username=? AND item_id=?",
            rusqlite::params![username, item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, String::new()));
    Ok(json!({"ok": true, "clicks": clicks, "last_clicked": last}))
}

pub fn set_note(conn: &Connection, item_id: &str, body: &Value) -> Result<Value, (u16, String)> {
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let note: String = note.chars().take(2000).collect();
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Err((404, "not found".into()));
    }
    let _ = conn.execute("UPDATE items SET note=? WHERE id=?", rusqlite::params![note, item_id]);
    Ok(json!({"ok": true}))
}

pub fn delete_item(conn: &Connection, item_id: &str) -> Result<Value, (u16, String)> {
    let changed = conn
        .execute(
            "UPDATE items SET deleted_at=datetime('now') WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            [item_id],
        )
        .unwrap_or(0);
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if changed == 0 && exists == 0 {
        return Err((404, "not found".into()));
    }
    Ok(json!({"ok": true, "trashed": true}))
}

// ---------- trash ----------

fn trash_rows(conn: &Connection) -> Vec<Map<String, Value>> {
    vault::fetch_rows(
        conn,
        "SELECT * FROM items WHERE deleted_at IS NOT NULL AND deleted_at != '' ORDER BY deleted_at DESC",
        &[],
    )
}

fn hard_delete_item(conn: &Connection, item_id: &str) {
    for r in vault::fetch_rows(conn, "SELECT * FROM media WHERE item_id=?", &[json!(item_id)]) {
        for field in ["file", "thumb"] {
            let name = r.get(field).and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() {
                let path = std::path::Path::new(&media_dir()).join(name);
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _ = conn.execute("DELETE FROM items WHERE id=?", [item_id]);
}

pub fn media_dir() -> String {
    format!(
        "{}/Projects/taste-vault/data/media",
        std::env::var("HOME").unwrap_or_default()
    )
}

pub fn purge_expired(conn: &Connection) -> i64 {
    let days = trash_days();
    let rows = vault::fetch_rows(
        conn,
        "SELECT id FROM items WHERE deleted_at IS NOT NULL AND deleted_at != '' \
         AND deleted_at <= datetime('now', ?)",
        &[json!(format!("-{days} days"))],
    );
    let mut n = 0;
    for r in rows {
        if let Some(id) = r.get("id").and_then(|v| v.as_str()) {
            hard_delete_item(conn, id);
            n += 1;
        }
    }
    n
}

pub fn list_trash(conn: &Connection) -> Value {
    purge_expired(conn);
    let rows = trash_rows(conn);
    let mut items = vault::rows_to_cards(conn, &rows);
    for (d, r) in items.iter_mut().zip(rows.iter()) {
        if let Some(obj) = d.as_object_mut() {
            obj.insert("deleted_at".into(), r.get("deleted_at").cloned().unwrap_or(Value::Null));
        }
    }
    json!({"items": items, "total": items.len(), "trash_days": trash_days()})
}

pub fn trash_restore(conn: &Connection, item_id: &str) -> Result<Value, (u16, String)> {
    let changed = conn
        .execute("UPDATE items SET deleted_at='' WHERE id=?", [item_id])
        .unwrap_or(0);
    if changed == 0 {
        return Err((404, "not found".into()));
    }
    Ok(json!({"ok": true}))
}

pub fn trash_destroy(conn: &Connection, item_id: &str) -> Result<Value, (u16, String)> {
    let row = vault::fetch_rows(
        conn,
        "SELECT id FROM items WHERE id=? AND deleted_at IS NOT NULL AND deleted_at != ''",
        &[json!(item_id)],
    )
    .into_iter()
    .next();
    if row.is_none() {
        return Err((404, "not found".into()));
    }
    hard_delete_item(conn, item_id);
    Ok(json!({"ok": true}))
}

pub fn trash_purge(conn: &Connection) -> Value {
    let mut n = 0;
    for r in trash_rows(conn) {
        if let Some(id) = r.get("id").and_then(|v| v.as_str()) {
            hard_delete_item(conn, id);
            n += 1;
        }
    }
    json!({"ok": true, "purged": n})
}

pub fn trash_restore_all(conn: &Connection) -> Value {
    let rows = trash_rows(conn);
    if rows.is_empty() {
        return json!({"ok": true, "restored": 0});
    }
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let n = ids.len();
    let ph = vec!["?"; n].join(",");
    let params: Vec<rusqlite::types::Value> = ids
        .iter()
        .map(|s| rusqlite::types::Value::Text(s.clone()))
        .collect();
    let _ = conn.execute(
        &format!("UPDATE items SET deleted_at='' WHERE id IN ({ph})"),
        rusqlite::params_from_iter(params.iter()),
    );
    json!({"ok": true, "restored": n})
}

// ---------- settings PUT ----------

pub fn put_settings(conn: &Connection, username: &str, body: &Value) -> Result<Value, (u16, String)> {
    let settings = body.get("settings");
    let valid = matches!(settings, Some(Value::Object(_)));
    if !valid {
        return Err((422, "settings must be an object".into()));
    }
    let settings = settings.unwrap();
    let encoded = serde_json::to_string(settings).unwrap_or_default();
    if encoded.len() > 20000 {
        return Err((413, "settings too large".into()));
    }
    let _ = conn.execute(
        "INSERT INTO user_settings(username, settings, updated_at) VALUES(?,?,datetime('now')) \
         ON CONFLICT(username) DO UPDATE SET settings=excluded.settings, updated_at=datetime('now')",
        rusqlite::params![username, encoded],
    );
    Ok(json!({"ok": true, "settings": settings}))
}

// ---------- followups POST ----------

pub fn update_followup(conn: &Connection, fid: &str, body: &Value) -> Result<Value, (u16, String)> {
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if !["done", "dismissed", "open"].contains(&status) {
        return Err((422, "status must be done, dismissed, or open".into()));
    }
    let changed = conn
        .execute("UPDATE followups SET status=? WHERE id=?", rusqlite::params![status, fid])
        .unwrap_or(0);
    if changed == 0 {
        return Err((404, "not found".into()));
    }
    Ok(json!({"ok": true, "id": fid, "status": status}))
}

pub fn followup_to_wiki(conn: &Connection, fid: &str) -> Result<Value, (u16, String)> {
    let row = vault::fetch_rows(conn, "SELECT * FROM followups WHERE id=?", &[json!(fid)])
        .into_iter()
        .next();
    let Some(row) = row else { return Err((404, "not found".into())) };
    let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let body_text = row.get("body").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if kind != "recipe" || body_text.is_empty() {
        return Err((422, "only recipe follow-ups with a body can go to the wiki".into()));
    }
    let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let title = if title.is_empty() { "recipe".to_string() } else { title };
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[^a-z0-9]+").unwrap());
    let mut slug: String = re.replace_all(&title.to_lowercase(), "-").trim_matches('-').to_string();
    slug = slug.chars().take(60).collect();
    if slug.is_empty() {
        slug = "recipe".into();
    }
    let food_dir = format!("{}/Wiki/Food", std::env::var("HOME").unwrap_or_default());
    let _ = std::fs::create_dir_all(&food_dir);
    let mut path = format!("{food_dir}/{slug}.md");
    if std::path::Path::new(&path).exists() {
        let created = row.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        let stamp: String = created.chars().filter(|c| c.is_ascii_digit()).collect();
        let stamp: String = stamp.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
        let stamp = if stamp.is_empty() { "0000".to_string() } else { stamp };
        path = format!("{food_dir}/{slug}-{stamp}.md");
    }
    let content = format!("# {title}\n\n{body_text}\n");
    std::fs::write(&path, content).map_err(|e| (500, format!("write failed: {e}")))?;
    let base = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(json!({"ok": true, "path": base, "note": format!("written to Food/{base}")}))
}

// ---------- share ----------

pub fn share_target(
    conn: &Connection,
    url: &str,
    title: &str,
    text: &str,
    _token: &str,
    files: &[StagedFile],
) -> Result<String, (u16, String)> {
    let payload = json!({"url": url, "title": title, "text": text, "note": ""});
    if !files.is_empty() {
        let saved: Vec<StagedFile> = files
            .iter()
            .filter(|f| std::fs::metadata(&f.path).map(|m| m.len() > 0).unwrap_or(false))
            .cloned()
            .collect();
        if !saved.is_empty() {
            let item_id = token_hex8();
            let dedupe_key = vault::canonical_source_key(url);
            if !dedupe_key.is_empty() {
                let existing = vault::fetch_rows(
                    conn,
                    "SELECT id FROM items WHERE dedupe_key=? LIMIT 1",
                    &[json!(dedupe_key)],
                )
                .into_iter()
                .next();
                if let Some(existing) = existing {
                    for f in &saved {
                        let _ = std::fs::remove_file(&f.path);
                    }
                    let id = existing.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    return Ok(format!("/?saved={id}"));
                }
            }
            let staged_json: Vec<Value> = saved
                .iter()
                .map(|s| json!({"path": s.path, "name": s.name, "mime": s.mime}))
                .collect();
            let _ = conn.execute(
                "INSERT INTO items (id, url, dedupe_key, title, kind, source, platform, text, note, extra) \
                 VALUES (?,?,?,?,?,?,?,?,?,?)",
                rusqlite::params![
                    item_id, url, dedupe_key,
                    if title.is_empty() { "shared file".to_string() } else { title.to_string() },
                    "image", "share-sheet", "share", text, "",
                    json!({"_staged_files": staged_json}).to_string(),
                ],
            );
            enqueue_staged(conn, &item_id);
            return Ok(format!("/?saved={item_id}"));
        }
    }
    let item = ensure_item(conn, &payload)?;
    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Ok(format!("/?saved={id}"))
}


/// local HH:MM for an epoch-ms timestamp (python time.localtime semantics)
pub fn time_hhmm_from_ms(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1000);
    // local offset
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m) = (rem / 3600, (rem % 3600) / 60);
    let _ = days;
    format!("{h:02}:{m:02}")
}
