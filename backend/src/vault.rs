//! Taste Vault read layer — port of routes.list_items + items_to_list +
//! _dedupe_search_items + media.aggregate_palettes, against the SAME vault.db
//! the Python service uses. Outputs are diffed against FastAPI for parity.

use std::sync::{Mutex, OnceLock};

use regex::Regex;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use serde_json::{json, Map, Value};

use crate::mt::Mt19937;
use crate::lock::LockExt;

pub struct Vault {
    pub conn: Mutex<Connection>,
}

const SOCIAL_SOURCES: [&str; 11] = [
    "instagram", "x", "pinterest", "reddit", "youtube", "tiktok",
    "facebook", "linkedin", "bluesky", "threads", "mastodon",
];

pub fn sqlite_to_json(v: &rusqlite::types::Value) -> Value {
    use rusqlite::types::Value as SV;
    match v {
        SV::Null => Value::Null,
        SV::Integer(i) => json!(i),
        SV::Real(f) => json!(f),
        SV::Text(t) => json!(t),
        SV::Blob(b) => json!(String::from_utf8_lossy(b).to_string()),
    }
}

impl Vault {
    pub fn open(path: &str) -> rusqlite::Result<Vault> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        // python's db module sets PRAGMA foreign_keys=ON per connection —
        // cascade deletes (items -> media/tags/...) must behave the same.
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        // WAL matches the Python service; harmless when already WAL.
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        // Base schema for a VIRGIN database (ported from legacy/python/db.py).
        // Every statement is IF NOT EXISTS, so an existing vault is untouched.
        // Without this a fresh container serves the SPA but cannot save.
        if let Err(e) = conn.execute_batch(include_str!("../schema.sql")) {
            eprintln!("taste-engine-rs: schema bootstrap failed: {e}");
        }
        Ok(Vault { conn: Mutex::new(conn) })
    }

    fn fetch_all(&self, sql: &str, params: &[Value]) -> Vec<Map<String, Value>> {
        let rp = rusqlite_params(params);
        let conn = self.conn.lock_ok();
        let mut stmt = match conn.prepare_cached(sql) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("taste-engine-rs: SQL prepare failed: {e} :: {sql}");
                return Vec::new();
            }
        };
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let iter = stmt.query_map(rusqlite::params_from_iter(rp.iter()), |row| {
            let mut m = Map::new();
            for (i, c) in names.iter().enumerate() {
                let v: rusqlite::types::Value = row.get(i)?;
                m.insert(c.clone(), sqlite_to_json(&v));
            }
            Ok(m)
        });
        match iter {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                eprintln!("taste-engine-rs: SQL query failed: {e} :: {sql}");
                Vec::new()
            }
        }
    }

    fn fetch_scalar(&self, sql: &str, params: &[Value]) -> i64 {
        let rp = rusqlite_params(params);
        let conn = self.conn.lock_ok();
        conn.query_row(sql, rusqlite::params_from_iter(rp.iter()), |r| r.get::<_, i64>(0))
            .unwrap_or(0)
    }
}

pub fn fetch_rows(conn: &Connection, sql: &str, params: &[Value]) -> Vec<Map<String, Value>> {
    let rp = rusqlite_params(params);
    let mut stmt = match conn.prepare_cached(sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("taste-engine-rs: SQL prepare failed: {e} :: {sql}");
            return Vec::new();
        }
    };
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let iter = stmt.query_map(rusqlite::params_from_iter(rp.iter()), |row| {
        let mut m = Map::new();
        for (i, c) in names.iter().enumerate() {
            let v: rusqlite::types::Value = row.get(i)?;
            m.insert(c.clone(), sqlite_to_json(&v));
        }
        Ok(m)
    });
    match iter {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("taste-engine-rs: SQL query failed: {e} :: {sql}");
            Vec::new()
        }
    }
}

pub fn fetch_scalar_val(conn: &Connection, sql: &str, params: &[Value]) -> i64 {
    let rp = rusqlite_params(params);
    conn.query_row(sql, rusqlite::params_from_iter(rp.iter()), |r| r.get::<_, i64>(0))
        .unwrap_or(0)
}

pub fn rusqlite_params(params: &[Value]) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value as SV;
    params
        .iter()
        .map(|v| to_rusqlite_value(v))
        .collect()
}

pub fn to_rusqlite_value(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as SV;
    match v {
        Value::Null => SV::Null,
        Value::Bool(b) => SV::Integer(*b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SV::Integer(i)
            } else {
                SV::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => SV::Text(s.clone()),
        other => SV::Text(other.to_string()),
    }
}

fn pyround(x: f64, nd: usize) -> f64 {
    format!("{:.*}", nd, x).parse().unwrap_or(x)
}

fn pyround_i(x: f64) -> i64 {
    format!("{:.0}", x).parse::<f64>().unwrap_or(x) as i64
}

// ---------- palette aggregation (media.aggregate_palettes) ----------

fn row_palette(palette_json: &str) -> Vec<(String, f64)> {
    let Ok(v) = serde_json::from_str::<Value>(palette_json) else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(list) = v.as_array() {
        for entry in list {
            if let Some(hexv) = entry.as_str() {
                out.push((hexv.to_lowercase(), 1.0));
            } else if let Some(obj) = entry.as_object() {
                let hexv = obj.get("hex").and_then(|h| h.as_str()).unwrap_or("").to_lowercase();
                let pct = obj.get("pct").and_then(|p| p.as_f64()).unwrap_or(0.0);
                out.push((hexv, pct));
            }
        }
    }
    out
}

fn aggregate_palettes(rows: &[Map<String, Value>]) -> Vec<Value> {
    let mut merged: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new(); // first-seen order (python dict semantics)
    for row in rows {
        let palette = row.get("palette").and_then(|p| p.as_str()).unwrap_or("[]");
        for (hexv, pct) in row_palette(palette) {
            if !hexv.starts_with('#') || hexv.len() != 7 || pct <= 0.0 {
                continue;
            }
            if !merged.contains_key(&hexv) {
                order.push(hexv.clone());
            }
            *merged.entry(hexv).or_insert(0.0) += pct;
        }
    }
    if merged.is_empty() {
        return Vec::new();
    }
    // python sums dict values in insertion order — float sum order matters
    let total: f64 = order.iter().filter_map(|h| merged.get(h).copied()).sum::<f64>().max(1e-9);
    let mut ranked: Vec<(String, f64)> = merged.into_iter().collect();
    // python sorted() is stable: ties keep first-seen (dict insertion) order
    let pos = |hex: &String| order.iter().position(|h| h == hex).unwrap_or(usize::MAX);
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap()
            .then(pos(&a.0).cmp(&pos(&b.0)))
    });
    ranked.truncate(5);
    ranked
        .into_iter()
        .map(|(hexv, value)| json!({"hex": hexv, "pct": pyround(100.0 * value / total, 1)}))
        .collect()
}

// ---------- dedupe keys (dedupe.canonical_source_key) ----------

fn normalize_text(value: &str) -> String {
    let v = unescape_html(value).to_lowercase();
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// python html.unescape: numeric refs exactly; named refs for the common set.
fn unescape_html(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = value[i..].find(';') {
                if semi <= 10 {
                    let ent = &value[i + 1..i + semi];
                    if let Some(d) = decode_entity(ent) {
                        out.push_str(&d);
                        i += semi + 1;
                        continue;
                    }
                }
            }
        }
        let ch = value[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn decode_entity(ent: &str) -> Option<String> {
    if let Some(rest) = ent.strip_prefix('#') {
        let code = if let Some(hexpart) = rest.strip_prefix(['x', 'X']) {
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

fn looks_http(s: &str) -> bool {
    (s.len() >= 7 && s[..7].eq_ignore_ascii_case("http://"))
        || (s.len() >= 8 && s[..8].eq_ignore_ascii_case("https://"))
}

fn re_once(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<std::collections::HashMap<&'static str, Regex>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        let specs: [(&'static str, &'static str); 7] = [
            ("status", r"(?i)/(?:i/)?status/(\d+)(?:/photo/(\d+))?"),
            ("ig", r"(?i)/(?:p|reel|tv)/([\w-]+)"),
            ("reddit", r"(?i)/r/[^/]+/comments/([\w-]+)"),
            ("ytv", r"(?:^|&)v=([\w-]{6,})"),
            ("shorts", r"(?i)/shorts/([\w-]{6,})"),
            ("pin", r"(?i)/pin/(\d+)"),
            ("placeholder", r"(?i)^(?:x post(?:\s+\d+)?|x\.com|twitter\.com|instagram\.com|reddit\.com|reddit - the heart of the internet|reddit link|saved image|shared file|[a-z0-9.-]+\.[a-z]{2,})$"),
        ];
        for (k, p) in specs {
            m.insert(k, Regex::new(p).unwrap());
        }
        m
    });
    map.get(pattern).unwrap()
}

pub fn canonical_source_key(url: &str) -> String {
    let raw = url.trim();
    if raw.is_empty() {
        return String::new();
    }
    let prefixed;
    let candidate = if looks_http(raw) {
        raw
    } else {
        prefixed = format!("https://{raw}");
        prefixed.as_str()
    };
    let Ok(parsed) = url::Url::parse(candidate) else {
        return format!("web|{}", normalize_text(raw).trim_end_matches('/'));
    };
    let host_full = parsed.host_str().unwrap_or("").to_lowercase();
    let host = match host_full.strip_prefix("www.") {
        Some(h) => h.to_string(),
        None => host_full,
    };
    let path = parsed.path().trim_end_matches('/').to_string();
    let path_lower = path.to_lowercase();

    if host == "x.com" || host == "twitter.com" || host == "fxtwitter.com" || host == "fixupx.com" || host == "vxtwitter.com" {
        if let Some(m) = re_once("status").captures(&path) {
            let id = m.get(1).map(|g| g.as_str()).unwrap_or("");
            let suffix = match m.get(2) {
                Some(p) => format!("|photo:{}", p.as_str()),
                None => "|post".to_string(),
            };
            return format!("tweet|{id}{suffix}");
        }
    }
    if host == "instagram.com" {
        if let Some(m) = re_once("ig").captures(&path) {
            if let Some(g) = m.get(1) {
                return format!("instagram|{}", g.as_str());
            }
        }
    }
    if host == "reddit.com" || host == "old.reddit.com" {
        if let Some(m) = re_once("reddit").captures(&path) {
            if let Some(g) = m.get(1) {
                return format!("reddit|{}", g.as_str());
            }
        }
    }
    if host == "redd.it" {
        let post_id = path.trim_start_matches('/').split('/').next().unwrap_or("");
        if !post_id.is_empty() {
            return format!("reddit|{post_id}");
        }
    }
    if (host == "youtube.com" || host == "m.youtube.com") && path_lower == "/watch" {
        let query = parsed.query().unwrap_or("");
        if let Some(m) = re_once("ytv").captures(query) {
            if let Some(g) = m.get(1) {
                return format!("youtube|{}", g.as_str());
            }
        }
    }
    if host == "youtu.be" {
        let video_id = path.trim_start_matches('/').split('/').next().unwrap_or("");
        if !video_id.is_empty() {
            return format!("youtube|{video_id}");
        }
    }
    if host.ends_with("youtube.com") {
        if let Some(m) = re_once("shorts").captures(&path) {
            if let Some(g) = m.get(1) {
                return format!("youtube|{}", g.as_str());
            }
        }
    }
    if host == "pinterest.com" {
        if let Some(m) = re_once("pin").captures(&path) {
            if let Some(g) = m.get(1) {
                return format!("pinterest|{}", g.as_str());
            }
        }
    }
    if host == "pin.it" {
        let pin_id = path.trim_start_matches('/').split('/').next().unwrap_or("");
        if !pin_id.is_empty() {
            return format!("pinterest|{pin_id}");
        }
    }
    format!("web|{}{}", host, if path_lower.is_empty() { "/".to_string() } else { path_lower })
}

// ---------- list route ----------

pub struct ListParams {
    pub q: String,
    pub kind: String,
    pub source: String,
    pub platform: String,
    pub hue: String,
    pub tag: String,
    pub limit: i64,
    pub offset: i64,
    pub sort: String,
    pub seed: i64,
}

fn sort_sql(sort: &str, username: &str) -> (String, Vec<Value>) {
    match sort {
        "alpha_asc" => (
            "COALESCE(NULLIF(title,''), NULLIF(text,''), url) COLLATE NOCASE ASC, saved_at DESC, rowid DESC".into(),
            vec![],
        ),
        "alpha_desc" => (
            "COALESCE(NULLIF(title,''), NULLIF(text,''), url) COLLATE NOCASE DESC, saved_at DESC, rowid DESC".into(),
            vec![],
        ),
        "oldest" => ("saved_at ASC, rowid ASC".into(), vec![]),
        "frequent" => (
            "COALESCE((SELECT clicks FROM item_interactions WHERE username=? AND item_id=items.id), 0) DESC, saved_at DESC, rowid DESC".into(),
            vec![json!(username)],
        ),
        _ => ("saved_at DESC, rowid DESC".into(), vec![]),
    }
}

pub fn list_items(vault: &Vault, username: &str, p: &ListParams) -> Value {
    let limit = p.limit.clamp(1, 200);
    let offset = p.offset.max(0);

    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if !p.kind.is_empty() {
        clauses.push("kind=?".into());
        params.push(json!(p.kind));
    }
    if !p.source.is_empty() {
        if p.source == "other" {
            let social = SOCIAL_SOURCES.iter().map(|_| "?".to_string()).collect::<Vec<_>>().join(",");
            clauses.push(format!("source NOT IN ({social}) AND source != ''"));
            for s in SOCIAL_SOURCES {
                params.push(json!(s));
            }
        } else {
            clauses.push("source=?".into());
            params.push(json!(p.source));
        }
    }
    if !p.platform.is_empty() {
        clauses.push("platform=?".into());
        params.push(json!(p.platform));
    }
    if !p.hue.is_empty() {
        clauses.push("id IN (SELECT item_id FROM media WHERE hue=?)".into());
        params.push(json!(p.hue));
    }
    if !p.tag.is_empty() {
        clauses.push("id IN (SELECT item_id FROM tags WHERE tag=?)".into());
        params.push(json!(p.tag.to_lowercase()));
    }
    let live = "(deleted_at IS NULL OR deleted_at='')";
    let where_sql = if clauses.is_empty() {
        format!("WHERE {live}")
    } else {
        format!("WHERE {live} AND {}", clauses.join(" AND "))
    };
    let q_where = if !p.q.is_empty() {
        " AND items.rowid IN (SELECT rowid FROM items_fts WHERE items_fts MATCH ?)".to_string()
    } else {
        String::new()
    };

    // random (default): stable per-seed order, paged by seed-shuffled ids
    if p.sort == "random" && p.q.is_empty() {
        let id_rows = vault.fetch_all(&format!("SELECT id FROM items {where_sql}"), &params);
        let mut ids: Vec<String> = id_rows
            .iter()
            .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();
        Mt19937::new_seeded(p.seed).shuffle(&mut ids);
        let total = ids.len() as i64;
        let page: Vec<String> = ids.iter().skip(offset as usize).take(limit as usize).cloned().collect();
        let mut rows: Vec<Map<String, Value>> = Vec::new();
        if !page.is_empty() {
            let ph = vec!["?"; page.len()].join(",");
            let pparams: Vec<Value> = page.iter().map(|s| json!(s)).collect();
            let mut all = vault.fetch_all(&format!("SELECT * FROM items WHERE id IN ({ph})"), &pparams);
            let mut order: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for (k, id) in page.iter().enumerate() {
                order.insert(id.clone(), k);
            }
            all.sort_by_key(|r| {
                r.get("id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| order.get(s).copied())
                    .unwrap_or(usize::MAX)
            });
            rows = all;
        }
        let items = items_to_list(&vault.conn.lock_ok(), &rows);
        let items = dedupe_search_items(items);
        return json!({
            "items": items,
            "total": total,
            "has_more": ((offset + items.len() as i64) < total),
        });
    }

    let (order_sql, order_params) = sort_sql(&p.sort, username);
    let mut lparams = params.clone();
    if !p.q.is_empty() {
        lparams.push(json!(format!("\"{}\"", p.q)));
    }
    lparams.extend(order_params.iter().cloned());
    lparams.push(json!(limit));
    lparams.push(json!(offset));
    let rows = vault.fetch_all(
        &format!("SELECT * FROM items {where_sql}{q_where} ORDER BY {order_sql} LIMIT ? OFFSET ?"),
        &lparams,
    );
    let total = if p.q.is_empty() {
        vault.fetch_scalar(&format!("SELECT COUNT(*) c FROM items {where_sql}"), &params)
    } else {
        rows.len() as i64
    };
    let items = items_to_list(&vault.conn.lock_ok(), &rows);
    let items = dedupe_search_items(items);
    json!({
        "items": items,
        "total": total,
        "has_more": ((offset + items.len() as i64) < total),
    })
}

// ---------- hydration (routes.items_to_list) ----------

fn parse_json_field(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        Value::Null => Value::Null,
        other @ (Value::Object(_) | Value::Array(_)) => other.clone(),
        _ => Value::Null,
    }
}

pub fn items_to_list(conn: &Connection, rows: &[Map<String, Value>]) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    for row in rows {
        let mut d = row.clone();
        let extra = d.remove("extra").map(|v| parse_json_field(&v)).unwrap_or(Value::Null);
        let mymind = extra
            .as_object()
            .map(|e| e.contains_key("mymind_id") && !e.get("mymind_id").map(|v| v.is_null()).unwrap_or(true))
            .unwrap_or(false)
            || d.get("source").and_then(|s| s.as_str()) == Some("mymind");
        let dedupe_key = d.get("dedupe_key").cloned().unwrap_or(Value::Null);
        let id = d.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        d.insert("is_mymind".into(), json!(mymind));
        d.insert("dedupe_key".into(), dedupe_key);
        d.insert("media_count".into(), json!(0));
        d.insert("cover".into(), Value::Null);
        d.insert("tags".into(), json!([]));
        d.insert("job".into(), Value::Null);
        d.insert("explanation_status".into(), Value::Null);
        items.push(Value::Object(d));
        ids.push(id);
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let ph = vec!["?"; ids.len()].join(",");
    let pp: Vec<Value> = ids.iter().map(|s| json!(s)).collect();

    let mut by_media: std::collections::HashMap<String, Vec<Map<String, Value>>> = std::collections::HashMap::new();
    for m in fetch_rows(
        conn,
        &format!(
            "SELECT item_id, id, kind, thumb, file, dominant, hue, palette, width, height, duration, placeholder FROM media WHERE item_id IN ({ph}) ORDER BY rowid"
        ),
        &pp,
    ) {
        let iid = m.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        by_media.entry(iid).or_default().push(m);
    }
    let mut jobs: std::collections::HashMap<String, Map<String, Value>> = std::collections::HashMap::new();
    for r in fetch_rows(conn, &format!("SELECT item_id, state, error FROM jobs WHERE item_id IN ({ph})"), &pp) {
        let iid = r.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        jobs.insert(iid, r);
    }
    let mut tags_by: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
    for t in fetch_rows(conn, &format!("SELECT item_id, tag FROM tags WHERE item_id IN ({ph})"), &pp) {
        let iid = t.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        tags_by.entry(iid).or_default().push(t.get("tag").cloned().unwrap_or(Value::Null));
    }
    let mut expl: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for r in fetch_rows(conn, &format!("SELECT item_id, status FROM explanations WHERE item_id IN ({ph})"), &pp) {
        let iid = r.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        expl.insert(iid, r.get("status").cloned().unwrap_or(Value::Null));
    }
    let mut fu_count: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in fetch_rows(
        conn,
        &format!("SELECT item_id, COUNT(*) n FROM followups WHERE status='open' AND item_id IN ({ph}) GROUP BY item_id"),
        &pp,
    ) {
        let iid = r.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        fu_count.insert(iid, r.get("n").and_then(|v| v.as_i64()).unwrap_or(0));
    }

    for item in items.iter_mut() {
        let obj = item.as_object_mut().unwrap();
        let iid = obj.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mrows = by_media.get(&iid).cloned().unwrap_or_default();
        let media_count = mrows.len();
        let cover = cover_from_media(&mrows);
        obj.insert("media_count".into(), json!(media_count));
        obj.insert("cover".into(), cover);
        let job = jobs.get(&iid).map(|j| {
            json!({
                "state": j.get("state").cloned().unwrap_or(Value::Null),
                "error": j.get("error").cloned().unwrap_or(Value::Null),
            })
        });
        obj.insert("job".into(), job.unwrap_or(Value::Null));
        let tags = tags_by.get(&iid).cloned().unwrap_or_default();
        obj.insert("tags".into(), Value::Array(tags));
        obj.insert("explanation_status".into(), expl.get(&iid).cloned().unwrap_or(Value::Null));
        obj.insert("open_followups".into(), json!(fu_count.get(&iid).copied().unwrap_or(0)));
    }
    items
}

fn cover_from_media(mrows: &[Map<String, Value>]) -> Value {
    let real: Vec<&Map<String, Value>> = mrows
        .iter()
        .filter(|m| {
            let placeholder = m.get("placeholder").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
            let kind = m.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let thumb = m.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
            !placeholder && (kind == "image" || kind == "video") && !thumb.is_empty()
        })
        .collect();
    if real.is_empty() {
        return Value::Null;
    }
    let first = real[0];
    let dicts: Vec<Map<String, Value>> = real.iter().map(|m| (*m).clone()).collect();
    let mut palette = aggregate_palettes(&dicts);
    if palette.is_empty() {
        let raw = first.get("palette").and_then(|p| p.as_str()).unwrap_or("[]");
        if let Ok(v) = serde_json::from_str::<Value>(raw) {
            if let Some(arr) = v.as_array() {
                palette = arr.clone();
            }
        }
    }
    let thumb = first.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
    let thumb_v = if thumb.is_empty() {
        first.get("file").cloned().unwrap_or(Value::Null)
    } else {
        json!(thumb)
    };
    json!({
        "id": first.get("id").cloned().unwrap_or(Value::Null),
        "kind": first.get("kind").cloned().unwrap_or(Value::Null),
        "thumb": thumb_v,
        "dominant": first.get("dominant").cloned().unwrap_or(Value::Null),
        "hue": first.get("hue").cloned().unwrap_or(Value::Null),
        "palette": Value::Array(palette),
        "width": first.get("width").cloned().unwrap_or(Value::Null),
        "height": first.get("height").cloned().unwrap_or(Value::Null),
        "duration": first.get("duration").cloned().unwrap_or(Value::Null),
    })
}

// ---------- grid dedupe (routes._dedupe_search_items + dedupe.py subset) ----------

fn get_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn record_content_key(record: &Value) -> String {
    let text = normalize_text(&get_str(record, "text"));
    if !text.is_empty() {
        return text;
    }
    normalize_text(&get_str(record, "title"))
}

fn dedupe_search_items(items: Vec<Value>) -> Vec<Value> {
    let mut kept: Vec<Value> = Vec::new();
    let mut seen: Vec<Value> = Vec::new();
    for item in items {
        let mut media_rows: Vec<Value> = Vec::new();
        if let Some(cover) = item.get("cover").filter(|c| !c.is_null()) {
            media_rows.push(cover.clone());
        }
        let mut record = item.as_object().cloned().unwrap_or_default();
        record.insert("media".into(), Value::Array(media_rows));
        let record = Value::Object(record);
        if seen.iter().any(|old| records_duplicate(&record, old)) {
            continue;
        }
        seen.push(record);
        kept.push(item);
    }
    kept
}

fn records_duplicate(left: &Value, right: &Value) -> bool {
    let left_source = canonical_source_key(&get_str(left, "url"));
    let right_source = canonical_source_key(&get_str(right, "url"));
    let left_media = media_fingerprints(left);
    let right_media = media_fingerprints(right);

    if !left_source.is_empty() && left_source == right_source {
        return true;
    }
    if !left_media.is_empty() && !right_media.is_empty() && media_equivalent(&left_media, &right_media) {
        let lc = record_content_key(left);
        let rc = record_content_key(right);
        return !lc.is_empty() && lc == rc;
    }
    false
}

pub type Fingerprint = (String, Option<u64>);

fn media_fingerprints(record: &Value) -> Vec<Fingerprint> {
    let media = record.get("media").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for m in media {
        let file = m.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let name = if file.is_empty() {
            m.get("thumb").and_then(|v| v.as_str()).unwrap_or("")
        } else {
            file
        };
        if name.is_empty() {
            continue;
        }
        let base = std::path::Path::new(name)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let path = std::path::Path::new(&media_dir()).join(&base);
        if let Some(fp) = file_fingerprint(&path) {
            out.push(fp);
        }
    }
    out
}

fn media_dir() -> String {
    format!(
        "{}/Projects/taste-vault/data/media",
        std::env::var("HOME").unwrap_or_default()
    )
}

pub fn normalize_text_pub(value: &str) -> String {
    normalize_text(value)
}

pub fn unescape_pub(value: &str) -> String {
    unescape_html(value)
}

/// urllib.parse.quote_plus (space -> '+', safe chars kept)
pub fn quote_plus_pub(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// urlsplit(url).netloc
pub fn host_of_pub(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    rest[..end].to_string()
}

pub fn pyround_pub(x: f64, nd: usize) -> f64 {
    pyround(x, nd)
}

pub fn platform_of_pub(url: &str) -> String {
    crate::write::platform_of(url)
}

pub fn decode_entity_pub(ent: &str) -> Option<String> {
    // reuse the vault-module entity table
    decode_entity(ent)
}

pub fn file_fingerprint(path: &std::path::Path) -> Option<Fingerprint> {
    use sha2::{Digest, Sha256};
    // python caches fingerprints on (path, size, mtime_ns) via lru_cache — same here
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let key = (path.to_path_buf(), md.len(), mtime);
    struct FpCache;
    static FP_CACHE: OnceLock<Mutex<std::collections::HashMap<(std::path::PathBuf, u64, u64), Fingerprint>>> = OnceLock::new();
    let cache = FP_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock_ok();
    if let Some(hit) = map.get(&key) {
        return Some(hit.clone());
    }
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let exact: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    let fp = (exact, compute_phash(&bytes));
    map.insert(key, fp.clone());
    Some(fp)
}

fn phash_close(left: Option<u64>, right: Option<u64>, threshold: u32) -> bool {
    match (left, right) {
        (Some(l), Some(r)) => (l ^ r).count_ones() <= threshold,
        _ => false,
    }
}

fn media_equivalent(first: &[Fingerprint], second: &[Fingerprint]) -> bool {
    if first.len() != second.len() {
        return false;
    }
    if first.is_empty() {
        return true;
    }
    let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (exact_first, phash_first) in first {
        let mut match_index: Option<usize> = None;
        for (index, (exact_second, phash_second)) in second.iter().enumerate() {
            if used.contains(&index) {
                continue;
            }
            if exact_first == exact_second || phash_close(*phash_first, *phash_second, 4) {
                match_index = Some(index);
                break;
            }
        }
        match match_index {
            None => return false,
            Some(i) => {
                used.insert(i);
            }
        }
    }
    true
}

/// The same dCTF pHash the Python save path computes (64x64 contain, 32x32
/// LANCZOS resize, 8x8 DCT via precomputed basis, median-sign bits).
fn compute_phash(bytes: &[u8]) -> Option<u64> {
    let img = image::load_from_memory(bytes).ok()?.to_luma8();
    let (w, h) = (img.width() as f64, img.height() as f64);
    let scale = (64.0 / w).min(64.0 / h);
    let nw = ((w * scale) as usize).max(1);
    let nh = ((h * scale) as usize).max(1);
    let contained = image::imageops::resize(&img, nw as u32, nh as u32, image::imageops::FilterType::Lanczos3);
    let mut canvas = image::GrayImage::from_pixel(64, 64, image::Luma([128u8]));
    let ox = ((64 - nw) / 2) as i64;
    let oy = ((64 - nh) / 2) as i64;
    image::imageops::overlay(&mut canvas, &contained, ox, oy);

    let mut pixels = vec![0.0f64; 32 * 32];
    pil_lanczos_resize(&canvas, 32, 32, &mut pixels);

    let n = 32usize;
    let mut basis = vec![0.0f64; n * 8];
    for (row, j) in (0..n).enumerate() {
        for (col, k) in (0..8).enumerate() {
            let mut v = ((std::f64::consts::PI / n as f64) * (j as f64 + 0.5) * k as f64).cos();
            if k == 0 {
                v /= 2.0f64.sqrt();
            }
            v *= (2.0 / n as f64).sqrt();
            basis[row * 8 + col] = v;
        }
    }
    let mut low = vec![0.0f64; 64];
    for i in 0..8 {
        for j in 0..8 {
            let mut acc = 0.0f64;
            for a in 0..32 {
                for b in 0..32 {
                    acc += basis[a * 8 + i] * pixels[a * 32 + b] * basis[b * 8 + j];
                }
            }
            low[i * 8 + j] = acc;
        }
    }
    let values = &low[1..];
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    };
    let mut phash: u64 = 0;
    for v in values {
        phash <<= 1;
        if *v > median {
            phash |= 1;
        }
    }
    Some(phash)
}

/// PIL-style Lanczos (a=3.0, clamped edges) resize to f64 pixel values.
fn pil_lanczos_resize(src: &image::GrayImage, dw: usize, dh: usize, out: &mut [f64]) {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let center_at = |dst: usize, dstn: usize, srcn: usize| -> f64 {
        (dst as f64 + 0.5) * srcn as f64 / dstn as f64 - 0.5
    };
    let lanczos = |x: f64| -> f64 {
        let ax = x.abs();
        if ax < 1e-8 {
            return 1.0;
        }
        if ax >= 3.0 {
            return 0.0;
        }
        let px = std::f64::consts::PI;
        (3.0 * (px * ax).sin() * (px * ax / 3.0).sin()) / ((px * ax) * (px * ax / 3.0))
    };
    let mut tmp = vec![0.0f64; sh * dw];
    for y in 0..sh {
        for x in 0..dw {
            let center = center_at(x, dw, sw);
            let lo = (center.floor() as i64 - 2).max(0) as usize;
            let hi = ((center.ceil() as i64 + 3).min(sw as i64)).max(lo as i64 + 1) as usize;
            let mut acc = 0.0f64;
            let mut wsum = 0.0f64;
            for sx in lo..hi {
                let wgt = lanczos(sx as f64 - center);
                acc += src.get_pixel(sx as u32, y as u32).0[0] as f64 * wgt;
                wsum += wgt;
            }
            tmp[y * dw + x] = if wsum != 0.0 { acc / wsum } else { 0.0 };
        }
    }
    for y in 0..dh {
        for x in 0..dw {
            let center = center_at(y, dh, sh);
            let lo = (center.floor() as i64 - 2).max(0) as usize;
            let hi = ((center.ceil() as i64 + 3).min(sh as i64)).max(lo as i64 + 1) as usize;
            let mut acc = 0.0f64;
            let mut wsum = 0.0f64;
            for sy in lo..hi {
                let wgt = lanczos(sy as f64 - center);
                acc += tmp[sy * dw + x] * wgt;
                wsum += wgt;
            }
            out[y * dw + x] = if wsum != 0.0 { acc / wsum } else { 0.0 };
        }
    }
}

// ---------- detail / meta / exists / followups / settings / hue ----------

pub fn rows_to_cards(conn: &Connection, rows: &[Map<String, Value>]) -> Vec<Value> {
    items_to_list(conn, rows)
}

pub fn media_to_dict_pub(row: &Map<String, Value>) -> Value {
    media_to_dict(row)
}

fn media_to_dict(row: &Map<String, Value>) -> Value {
    let mut d = row.clone();
    d.remove("embed");
    let palette_raw = d.get("palette").and_then(|p| p.as_str()).unwrap_or("[]");
    let parsed = serde_json::from_str::<Value>(palette_raw).unwrap_or(json!([]));
    d.insert("palette".into(), if parsed.is_array() { parsed } else { json!([]) });
    Value::Object(d)
}

pub fn item_detail(conn: &Connection, item_id: &str) -> Option<Value> {
    let rows = fetch_rows(conn, "SELECT * FROM items WHERE id=?", &[json!(item_id)]);
    let row = rows.into_iter().next()?;
    let cards = items_to_list(conn, std::slice::from_ref(&row));
    let mut d = match cards.into_iter().next() {
        Some(v) => v.as_object().cloned().unwrap_or_default(),
        None => return None,
    };
    let all_media = fetch_rows(conn, "SELECT * FROM media WHERE item_id=? ORDER BY rowid", &[json!(item_id)]);
    let real: Vec<&Map<String, Value>> = all_media
        .iter()
        .filter(|m| m.get("placeholder").and_then(|v| v.as_i64()).unwrap_or(0) == 0)
        .collect();
    // Detail still shows placeholder rows when they are all the item has.
    let chosen: Vec<Map<String, Value>> = if real.is_empty() {
        all_media.clone()
    } else {
        real.iter().map(|m| (*m).clone()).collect()
    };
    d.insert("media".into(), Value::Array(chosen.iter().map(media_to_dict).collect()));
    let transcripts: Vec<Value> = all_media
        .iter()
        .filter(|m| {
            m.get("kind").and_then(|v| v.as_str()) == Some("audio")
                && m.get("transcript").and_then(|v| v.as_str()).map(|t| !t.is_empty()).unwrap_or(false)
        })
        .map(|m| {
            json!({
                "media_id": m.get("id").cloned().unwrap_or(Value::Null),
                "name": m.get("original_name").cloned().unwrap_or(Value::Null),
                "text": m.get("transcript").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let transcript_text = transcripts
        .iter()
        .filter_map(|t| t.get("text").and_then(|v| v.as_str()))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !transcript_text.is_empty() {
        let text_empty = match d.get("text") {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.is_empty(),
            _ => false,
        };
        if text_empty {
            d.insert("text".into(), json!(transcript_text));
        }
    }
    d.insert("transcripts".into(), Value::Array(transcripts));
    let expl = fetch_rows(
        conn,
        "SELECT status, data, updated_at FROM explanations WHERE item_id=?",
        &[json!(item_id)],
    )
    .into_iter()
    .next();
    let explanation = match expl {
        Some(e) if e.get("status").and_then(|v| v.as_str()) == Some("done") => e
            .get("data")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    d.insert("explanation".into(), explanation);
    let job = fetch_rows(
        conn,
        "SELECT state, error FROM jobs WHERE item_id=?",
        &[json!(item_id)],
    )
    .into_iter()
    .next();
    let job_v = job.map(|j| json!({"state": j.get("state").cloned().unwrap_or(Value::Null), "error": j.get("error").cloned().unwrap_or(Value::Null)}));
    d.insert("job".into(), job_v.unwrap_or(Value::Null));
    let similar = crate::search::fast_similar_entries(conn, item_id, 12);
    let n = similar.len();
    d.insert("similar".into(), Value::Array(similar));
    d.insert("similar_offset".into(), json!(n));
    d.insert("similar_total".into(), json!(n));
    d.insert("similar_has_more".into(), json!(true));
    let extra = row
        .get("extra")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| json!({}));
    let outlinks: Vec<Value> = extra
        .get("outlinks")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|o| o.is_object()).cloned().collect())
        .unwrap_or_default();
    d.insert("outlinks".into(), Value::Array(outlinks));
    let followups = fetch_rows(
        conn,
        "SELECT * FROM followups WHERE item_id=? ORDER BY \
         CASE status WHEN 'open' THEN 0 ELSE 1 END, created_at DESC, rowid DESC",
        &[json!(item_id)],
    );
    d.insert("followups".into(), Value::Array(followups.into_iter().map(Value::Object).collect()));
    Some(Value::Object(d))
}

pub fn item_exists_lookup(conn: &Connection, url: &str) -> Result<Value, String> {
    let url = url.trim();
    let url: String = url.chars().take(2000).collect();
    if url.is_empty() {
        return Err("url required".into());
    }
    let key = canonical_source_key(&url);
    let live = "(deleted_at IS NULL OR deleted_at='')";
    let mut found: Option<String> = None;
    if !key.is_empty() {
        found = fetch_rows(
            conn,
            &format!(
                "SELECT id FROM items WHERE {live} AND (dedupe_key=? OR (dedupe_key='' AND url=?)) ORDER BY saved_at, rowid LIMIT 1"
            ),
            &[json!(key), json!(url)],
        )
        .into_iter()
        .next()
        .and_then(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()));
    }
    if found.is_none() {
        found = fetch_rows(
            conn,
            &format!("SELECT id FROM items WHERE {live} AND url=? LIMIT 1"),
            &[json!(url)],
        )
        .into_iter()
        .next()
        .and_then(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()));
    }
    Ok(match found {
        Some(id) => json!({"exists": true, "id": id}),
        None => json!({"exists": false, "id": Value::Null}),
    })
}

/// Counting helper that keeps first-seen order (python dict semantics).
struct Counter {
    entries: Vec<(String, i64)>,
}

impl Counter {
    fn new() -> Counter {
        Counter { entries: Vec::new() }
    }
    fn bump(&mut self, key: &str) {
        if let Some(i) = self.entries.iter().position(|(k, _)| k == key) {
            self.entries[i].1 += 1;
        } else {
            self.entries.push((key.to_string(), 1));
        }
    }
    fn sorted_desc(self) -> Vec<(String, i64)> {
        let mut v = self.entries;
        v.sort_by(|a, b| b.1.cmp(&a.1)); // stable: ties keep insertion order
        v
    }
}

pub fn meta(conn: &Connection) -> Value {
    let rows = fetch_rows(
        conn,
        "SELECT i.kind AS kind, i.source AS source, i.platform AS platform, m.hue AS hue \
         FROM items i LEFT JOIN media m ON m.rowid=(SELECT MIN(rowid) FROM media WHERE item_id=i.id) \
         WHERE (i.deleted_at IS NULL OR i.deleted_at='')",
        &[],
    );
    let mut total = 0i64;
    let mut sources = Counter::new();
    let mut platforms = Counter::new();
    let mut hues = Counter::new();
    for r in &rows {
        total += 1;
        let src = r.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if !src.is_empty() {
            let mapped = if SOCIAL_SOURCES.contains(&src) { src } else { "other" };
            sources.bump(mapped);
        }
        let platform = r.get("platform").and_then(|v| v.as_str()).unwrap_or("");
        if !platform.is_empty() {
            platforms.bump(platform);
        }
        let hue = r.get("hue").and_then(|v| v.as_str()).unwrap_or("");
        if !hue.is_empty() {
            hues.bump(hue);
        }
    }
    let tag_rows = fetch_rows(
        conn,
        "SELECT tag, COUNT(*) c FROM tags GROUP BY tag ORDER BY c DESC LIMIT 60",
        &[],
    );
    let kind_rows = fetch_rows(conn, "SELECT kind, COUNT(*) c FROM items GROUP BY kind", &[]);
    let mut exclude: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (k, _) in kind_rows
        .iter()
        .filter_map(|r| r.get("kind").and_then(|v| v.as_str()).map(|s| (s, 0)))
    {
        exclude.insert(k.to_lowercase());
    }
    let source_entries = sources.entries.iter().map(|(k, _)| k.to_lowercase());
    for s in source_entries {
        exclude.insert(s);
    }
    for (h, _) in hues.entries.iter() {
        exclude.insert(h.to_lowercase());
    }
    for extra in [
        "twitter", "xpost", "x-posts", "instagrampost", "instagram post",
        "reddit post", "screenshot", "web", "manual", "selection", "note",
        "charcoal", "white", "black", "silver", "grey", "gray",
        "brown", "beige", "cream", "ivory", "navy", "gold",
    ] {
        exclude.insert(extra.to_string());
    }
    let filtered_tags: Vec<(String, i64)> = tag_rows
        .iter()
        .filter_map(|r| {
            let tag = r.get("tag").and_then(|v| v.as_str())?;
            let c = r.get("c").and_then(|v| v.as_i64())?;
            if exclude.contains(&tag.to_lowercase()) {
                None
            } else {
                Some((tag.to_string(), c))
            }
        })
        .collect();
    let src_sorted = sources.sorted_desc();
    let plat_sorted = platforms.sorted_desc();
    let hue_sorted = hues.sorted_desc();
    json!({
        "total": total,
        "sources": src_sorted.iter().map(|(k, c)| json!([k, c])).collect::<Vec<_>>(),
        "platforms": plat_sorted.iter().map(|(k, c)| json!([k, c])).collect::<Vec<_>>(),
        "hues": hue_sorted.iter().map(|(k, c)| json!([k, c])).collect::<Vec<_>>(),
        "tags": filtered_tags.iter().map(|(k, c)| json!([k, c])).collect::<Vec<_>>(),
        "kinds": kind_rows
            .iter()
            .filter_map(|r| {
                let kind = r.get("kind").and_then(|v| v.as_str())?;
                let c = r.get("c").and_then(|v| v.as_i64())?;
                Some(json!([kind, c]))
            })
            .collect::<Vec<_>>(),
    })
}

pub fn followups_query(conn: &Connection, status: &str, kind: &str) -> Vec<Value> {
    let status = if status.is_empty() { "open" } else { status };
    let mut clauses = vec!["f.status=?".to_string()];
    let mut params: Vec<Value> = vec![json!(status)];
    if !kind.is_empty() {
        clauses.push("f.kind=?".to_string());
        params.push(json!(kind));
    }
    fetch_rows(
        conn,
        &format!(
            "SELECT f.*, i.title AS item_title, i.url AS item_url, i.kind AS item_kind, \
             (SELECT m.thumb FROM media m WHERE m.item_id=i.id \
              AND m.placeholder=0 AND m.kind IN ('image','video') AND m.thumb<>'' \
              ORDER BY m.rowid LIMIT 1) AS item_thumb, \
             COALESCE((SELECT m.width FROM media m WHERE m.item_id=i.id \
              AND m.placeholder=0 AND m.kind IN ('image','video') AND m.thumb<>'' \
              ORDER BY m.rowid LIMIT 1),0) AS item_w, \
             COALESCE((SELECT m.height FROM media m WHERE m.item_id=i.id \
              AND m.placeholder=0 AND m.kind IN ('image','video') AND m.thumb<>'' \
              ORDER BY m.rowid LIMIT 1),0) AS item_h \
             FROM followups f JOIN items i ON i.id=f.item_id \
             WHERE {} AND (i.deleted_at IS NULL OR i.deleted_at='') \
             ORDER BY f.created_at DESC, f.rowid DESC LIMIT 300",
            clauses.join(" AND ")
        ),
        &params,
    )
    .into_iter()
    .map(Value::Object)
    .collect()
}

pub fn get_settings(conn: &Connection, username: &str) -> Value {
    let row = fetch_rows(
        conn,
        "SELECT settings, updated_at FROM user_settings WHERE username=?",
        &[json!(username)],
    )
    .into_iter()
    .next();
    match row {
        None => json!({"settings": Value::Null, "updated_at": Value::Null}),
        Some(r) => {
            let raw = r.get("settings").and_then(|v| v.as_str()).unwrap_or("{}");
            let parsed = serde_json::from_str::<Value>(raw).unwrap_or(json!({}));
            let settings = if parsed.is_object() { parsed } else { json!({}) };
            json!({
                "settings": settings,
                "updated_at": r.get("updated_at").cloned().unwrap_or(Value::Null),
            })
        }
    }
}

const HUE_FAMILIES: [(&str, f64, f64); 12] = [
    ("red", 350.0, 12.0),
    ("orange", 12.0, 42.0),
    ("amber", 42.0, 62.0),
    ("yellow", 62.0, 82.0),
    ("olive", 82.0, 120.0),
    ("green", 120.0, 165.0),
    ("teal", 165.0, 190.0),
    ("cyan", 190.0, 210.0),
    ("blue", 210.0, 265.0),
    ("purple", 265.0, 290.0),
    ("pink", 300.0, 336.0),
    ("magenta", 290.0, 300.0),
];

/// HSV hue family for a color.
///
/// Every degree of the circle maps to a family. Pink covers 300-336, so a crimson
/// rose (#c80044 sits at 339.6) stays "red"; 336-350 falls through to red as well.
/// The ported code skipped the pink slot AND wrapped negative hue to red, so every
/// pink and fuchsia in the vault was stored as "red".
pub fn classify_hue(r: f64, g: f64, b: f64) -> String {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let v = maxc;
    let delta = maxc - minc;
    let s = if maxc > 0.0 { delta / maxc } else { 0.0 };
    let h_raw = if delta == 0.0 {
        0.0
    } else if maxc == r {
        // Rust `%` keeps the sign: without the +6 wrap, red-magenta hues
        // (g < b) went negative and fell through to "red" instead of "pink".
        (((g - b) / delta) % 6.0 + 6.0) % 6.0
    } else if maxc == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let h_deg = (h_raw / 6.0) * 360.0;
    if v < 0.14 {
        return "neutral-dark".into();
    }
    if s < 0.10 && v > 0.90 {
        return "neutral-light".into();
    }
    if s < 0.22 {
        return if r > b { "warm-neutral".into() } else { "cool-neutral".into() };
    }
    for (name, lo, hi) in HUE_FAMILIES {
        if lo <= h_deg && h_deg < hi {
            return name.to_string();
        }
    }
    "red".into()
}

pub fn hue_from_hex(hex: &str) -> Result<Value, String> {
    let mut raw = hex.trim().trim_start_matches('#').to_string();
    if raw.len() == 3 {
        raw = raw.chars().map(|c| format!("{c}{c}")).collect();
    }
    let is_hex = raw.len() == 6 && raw.chars().all(|c| c.is_ascii_hexdigit());
    if !is_hex {
        return Err("bad hex".into());
    }
    let byte = |i: usize| u8::from_str_radix(&raw[i..i + 2], 16).unwrap_or(0);
    let hue = classify_hue(byte(0) as f64, byte(2) as f64, byte(4) as f64);
    Ok(json!({"ok": true, "hue": hue, "hex": format!("#{}", raw.to_lowercase())}))
}

// ---------- /api/search (routes.vault_search) ----------

const CONTENTLESS_TITLES: [&str; 12] = [
    "gfycat.com", "reddit.com", "imgur.com", "imgur", "pinterest.com", "instagram.com",
    "x.com", "twitter.com", "youtube.com", "github.com", "https://", "http://",
];

fn is_contentless_item(conn: &Connection, row: &Map<String, Value>) -> bool {
    let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let text = row.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let url = row.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let has_media = fetch_scalar_val(
        conn,
        "SELECT COUNT(*) FROM media WHERE item_id=? AND placeholder=0 AND embed IS NOT NULL LIMIT 1",
        &[row.get("id").cloned().unwrap_or(Value::Null)],
    ) > 0;
    if has_media {
        return false;
    }
    let title_l = title.to_lowercase();
    let text_l = text.to_lowercase();
    if !title.is_empty() && title.len() < 3 && (text.is_empty() || text.len() < 3) {
        return true;
    }
    if CONTENTLESS_TITLES.contains(&title_l.as_str()) {
        return true;
    }
    if text.is_empty() && (title_l.starts_with("https://") || title_l.starts_with("http://")) {
        return true;
    }
    if text.is_empty() && !url.is_empty() && title_l == url.to_lowercase() {
        return true;
    }
    if (text_l.starts_with("https://") || text_l.starts_with("http://")) && !text_l.contains(' ') {
        return true;
    }
    if text_l == "account private" || text_l == "archived link" || text_l.contains("account private") {
        return true;
    }
    !title_l.is_empty()
        && text_l == title_l
        && !has_media
        && (title_l.ends_with(".com") || title_l.ends_with(".net") || title_l.ends_with(".org"))
}

pub struct SearchParams {
    pub q: String,
    pub mode: String,
    pub limit: i64,
    pub kind: String,
    pub source: String,
    pub hue: String,
    pub tag: String,
    pub offset: i64,
}

pub fn vault_search(conn: &Connection, p: &SearchParams) -> Value {
    if p.q.trim().is_empty() {
        return json!({"items": [], "mode": p.mode});
    }
    let limit = p.limit.clamp(1, 200);
    let fetch_cap = (limit * 4 + 50).max(50);

    let mut text_results: Vec<(String, f64)> = Vec::new();
    if p.mode == "auto" || p.mode == "text" {
        let exact = fetch_rows(
            conn,
            &format!(
                "SELECT items.* FROM items JOIN items_fts ON items.rowid=items_fts.rowid \
                 WHERE (items.deleted_at IS NULL OR items.deleted_at='') AND items_fts MATCH ? \
                 ORDER BY bm25(items_fts) LIMIT {fetch_cap}"
            ),
            &[json!(format!("\"{}\"", p.q))],
        );
        let terms: Vec<String> = p.q.trim().split_whitespace().map(|s| s.to_string()).collect();
        let stem_phrase = if terms.is_empty() {
            format!("\"{}\"", p.q)
        } else {
            terms
                .iter()
                .map(|t| format!("\"{}\"", t.replace('"', "")))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let stem_rows = fetch_rows(
            conn,
            &format!(
                "SELECT items.* FROM items JOIN items_fts_stem ON items.rowid=items_fts_stem.rowid \
                 WHERE (items.deleted_at IS NULL OR items.deleted_at='') AND items_fts_stem MATCH ? \
                 ORDER BY bm25(items_fts_stem) LIMIT {fetch_cap}"
            ),
            &[json!(stem_phrase)],
        );
        let mut merged_rows: Vec<Map<String, Value>> = Vec::new();
        let mut seen_rows: std::collections::HashSet<String> = std::collections::HashSet::new();
        for row in exact.into_iter().chain(stem_rows.into_iter()) {
            let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if seen_rows.contains(&id) {
                continue;
            }
            seen_rows.insert(id);
            merged_rows.push(row);
        }
        let expl_rows = fetch_rows(
            conn,
            &format!(
                "SELECT items.* FROM items JOIN expl_fts ON expl_fts.item_id=items.id \
                 WHERE (items.deleted_at IS NULL OR items.deleted_at='') AND expl_fts MATCH ? \
                 ORDER BY bm25(expl_fts) LIMIT {fetch_cap}"
            ),
            &[json!(stem_phrase)],
        );
        for row in expl_rows {
            let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if seen_rows.contains(&id) {
                continue;
            }
            seen_rows.insert(id);
            merged_rows.push(row);
        }
        if !(p.kind.is_empty() && p.source.is_empty() && p.hue.is_empty() && p.tag.is_empty()) {
            let ranked: Vec<(String, f64)> = merged_rows
                .iter()
                .map(|r| (r.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(), 0.0))
                .collect();
            let allowed: std::collections::HashSet<String> =
                crate::search::filter_ranked_pub(conn, ranked, &p.kind, &p.source, &p.hue, &p.tag, fetch_cap)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
            merged_rows.retain(|r| {
                allowed.contains(r.get("id").and_then(|v| v.as_str()).unwrap_or(""))
            });
        }
        text_results = merged_rows
            .iter()
            .map(|r| (r.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(), 0.0))
            .collect();
    }

    let mut semantic_results: Vec<(String, f64)> = Vec::new();
    if p.mode == "auto" || p.mode == "semantic" {
        let vecs = crate::embed::embed_text(&[p.q.clone()]);
        let Ok(vecs) = vecs else { return json!({"items": [], "mode": p.mode}) };
        let Some(vec) = vecs.into_iter().next() else {
            return json!({"items": [], "mode": p.mode});
        };
        let Some(vec) = crate::search::normalize_pub(&vec) else {
            return json!({"items": [], "mode": p.mode});
        };
        let candidates = crate::search::query_vec(
            conn,
            &vec,
            limit * 4 + 20,
            &p.kind,
            &p.source,
            &p.hue,
            &p.tag,
            None,
        );
        let candidate_ids: Vec<String> = candidates.iter().map(|(id, _)| id.clone()).collect();
        let mut media_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        if !candidate_ids.is_empty() {
            let ph = vec!["?"; candidate_ids.len()].join(",");
            let params: Vec<Value> = candidate_ids.iter().map(|s| json!(s)).collect();
            for r in fetch_rows(
                conn,
                &format!(
                    "SELECT DISTINCT item_id FROM media WHERE item_id IN ({ph}) \
                     AND placeholder=0 AND embed IS NOT NULL"
                ),
                &params,
            ) {
                if let Some(id) = r.get("item_id").and_then(|v| v.as_str()) {
                    media_ids.insert(id.to_string());
                }
            }
        }
        for (item_id, score) in candidates {
            if !media_ids.contains(&item_id) {
                continue;
            }
            let rows = fetch_rows(conn, "SELECT * FROM items WHERE id=?", &[json!(item_id)]);
            match rows.into_iter().next() {
                Some(row) => {
                    if is_contentless_item(conn, &row) {
                        continue;
                    }
                    semantic_results.push((item_id, score));
                    if semantic_results.len() as i64 >= limit * 4 {
                        break;
                    }
                }
                None => continue,
            }
        }
    }

    let results: Vec<(String, f64)> = if p.mode == "auto" {
        let mut out: Vec<(String, f64)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (id, score) in text_results.into_iter().chain(semantic_results.into_iter()) {
            if seen.contains(&id) {
                continue;
            }
            seen.insert(id.clone());
            out.push((id, score));
        }
        out
    } else if p.mode == "text" {
        text_results
    } else {
        semantic_results
    };

    let offset = p.offset.max(0) as usize;
    let page: Vec<(String, f64)> = results
        .iter()
        .skip(offset)
        .take(limit as usize)
        .cloned()
        .collect();
    let has_more = (offset + page.len()) < results.len();
    let cards = crate::search::entries_from_ranked(conn, page);
    let items = dedupe_search_items(cards);
    json!({"items": items, "mode": p.mode, "has_more": has_more})
}


/// positional convenience wrapper used by the context tool
pub fn vault_search_pub(conn: &Connection, q: &str, mode: &str, limit: i64, kind: &str, source: &str, hue: &str, tag: &str, offset: i64) -> Value {
    vault_search(conn, &SearchParams {
        q: q.to_string(), mode: mode.to_string(), limit, kind: kind.to_string(),
        source: source.to_string(), hue: hue.to_string(), tag: tag.to_string(), offset,
    })
}

#[cfg(test)]
mod hue_tests {
    use super::classify_hue;

    fn hue_of(hex: &str) -> String {
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64;
        classify_hue(byte(0), byte(2), byte(4))
    }

    #[test]
    fn pink_and_fuchsia_reach_the_pink_family() {
        // 300-336 used to fall through to "red": the pink slot was skipped and
        // Rust's signed % sent g < b hues negative.
        assert_eq!(hue_of("ff00a8"), "pink");
        assert_eq!(hue_of("ff8fc7"), "pink");
        // 339.6 is a crimson rose, not a pink.
        assert_eq!(hue_of("ff2d78"), "red");
    }

    #[test]
    fn other_families_keep_their_bins() {
        assert_eq!(hue_of("c80044"), "red");
        assert_eq!(hue_of("ff7a00"), "orange");
        assert_eq!(hue_of("00a86b"), "green");
        assert_eq!(hue_of("3fa9f5"), "cyan");
        assert_eq!(hue_of("4b3fd9"), "blue");
        assert_eq!(hue_of("000000"), "neutral-dark");
    }
}

