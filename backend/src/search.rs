//! Stored-vector similarity — port of search.py's index (item_embeds blobs),
//! query_vec cosine ranking, _filter, and routes._fast_similar_entries.
//! No model needed: every vector comes from the DB. Items without a stored
//! vector fall back to the mean of their media embeds (also stored); if that
//! is missing too, python calls the CLIP text encoder — the Rust port returns
//! None there (no model), which surfaces as empty similar cards.

use std::sync::{Arc, Mutex, OnceLock, RwLock};

use rusqlite::Connection;
use rusqlite::OpenFlags;
use serde_json::{json, Map, Value};
use crate::lock::LockExt;

const EMBED_DIM: usize = 512;

pub struct EmbedIndex {
    pub ids: Vec<String>,
    pub mat: Vec<f32>, // n * EMBED_DIM, normalized rows
}

struct IndexCache {
    version: (i64, i64, i64, i64, i64),
    index: Arc<EmbedIndex>,
}

static CACHE: RwLock<Option<IndexCache>> = RwLock::new(None);

pub fn db_version(conn: &Connection) -> (i64, i64, i64, i64, i64) {
    let q = |sql: &str| -> (i64, i64) {
        conn.query_row(sql, [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .unwrap_or((0, 0))
    };
    let a = q("SELECT COUNT(*), COALESCE(MAX(rowid),0) FROM item_embeds");
    let b = q("SELECT COUNT(*), COALESCE(MAX(rowid),0) FROM items");
    let d: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM items WHERE deleted_at IS NOT NULL AND deleted_at != ''",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    (a.0, a.1, b.0, b.1, d)
}

fn blob_to_vec(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() != EMBED_DIM * 4 {
        return None;
    }
    let mut v = vec![0.0f32; EMBED_DIM];
    for (i, chunk) in blob.chunks_exact(4).enumerate() {
        v[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Some(v)
}

/// OpenBLAS-style sdot: 4 accumulators stepping by 4, then a scalar tail,
/// combined ((s0+s1)+(s2+s3))+tail — the pattern numpy's BLAS uses, so
/// near-tied cosine scores round the same way.
pub fn dot32(a: &[f32], b: &[f32]) -> f32 {
    let (mut s0, mut s1, mut s2, mut s3) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut i = 0;
    while i + 4 <= a.len() {
        s0 += a[i] * b[i];
        s1 += a[i + 1] * b[i + 1];
        s2 += a[i + 2] * b[i + 2];
        s3 += a[i + 3] * b[i + 3];
        i += 4;
    }
    let mut tail = 0.0f32;
    while i < a.len() {
        tail += a[i] * b[i];
        i += 1;
    }
    ((s0 + s1) + (s2 + s3)) + tail
}

fn normalize(v: &[f32]) -> Option<Vec<f32>> {
    let norm = dot32(v, v).sqrt();
    if norm <= 0.0 {
        return None;
    }
    Some(v.iter().map(|x| x / norm).collect())
}

fn load_index(conn: &Connection) -> EmbedIndex {
    let Ok(mut stmt) = conn.prepare(
        "SELECT e.item_id AS id, e.vec AS vec FROM item_embeds e \
         JOIN items i ON i.id=e.item_id AND (i.deleted_at IS NULL OR i.deleted_at='')",
    ) else {
        return EmbedIndex { ids: Vec::new(), mat: Vec::new() };
    };
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)));
    let mut ids: Vec<String> = Vec::new();
    let mut mat: Vec<f32> = Vec::new();
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (id, blob) = row;
            if let Some(mut v) = blob_to_vec(&blob) {
                let norm = dot32(&v, &v).sqrt();
                if norm > 0.0 {
                    for x in v.iter_mut() {
                        *x /= norm;
                    }
                    ids.push(id);
                    mat.extend(v);
                }
            }
        }
    }
    EmbedIndex { ids, mat }
}

/// Drop the cached vector index. Call after rewriting `item_embeds`, because
/// the cache key is (count, max rowid) and a bulk rewrite can keep both.
pub fn invalidate_index() {
    *CACHE.write().unwrap() = None;
}

fn ensure_index(conn: &Connection) -> Arc<EmbedIndex> {
    let version = db_version(conn);
    {
        let cache = CACHE.read().unwrap();
        if let Some(hit) = cache.as_ref() {
            if hit.version == version {
                return hit.index.clone();
            }
        }
    }
    let mut cache = CACHE.write().unwrap();
    if let Some(hit) = cache.as_ref() {
        if hit.version == version {
            return hit.index.clone();
        }
    }
    let index = Arc::new(load_index(conn));
    *cache = Some(IndexCache { version, index: index.clone() });
    index
}

pub fn item_vec(conn: &Connection, item_id: &str) -> Option<Vec<f32>> {
    let blob: Option<Vec<u8>> = conn
        .query_row("SELECT vec FROM item_embeds WHERE item_id=?", [item_id], |r| r.get(0))
        .ok();
    if let Some(blob) = blob {
        if let Some(v) = blob_to_vec(&blob) {
            return Some(v);
        }
        // empty/short blob: python falls through to compute_item_vec
    }
    // compute_item_vec fallback: mean of the item's media embeddings
    let mut stmt = conn
        .prepare("SELECT embed FROM media WHERE item_id=? ORDER BY rowid")
        .ok()?;
    let rows: Vec<Option<Vec<u8>>> = stmt
        .query_map([item_id], |r| r.get(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    let mut vecs: Vec<Vec<f32>> = Vec::new();
    for raw in rows {
        if let Some(raw) = raw {
            if let Some(v) = blob_to_vec(&raw) {
                vecs.push(v);
            }
        }
    }
    if !vecs.is_empty() {
        let mut mean = vec![0.0f32; EMBED_DIM];
        for v in &vecs {
            for (m, x) in mean.iter_mut().zip(v.iter()) {
                *m += x;
            }
        }
        let n = vecs.len() as f32;
        for m in mean.iter_mut() {
            *m /= n;
        }
        return Some(mean);
    }
    // last fallback: CLIP text of title+body (needs the onnx model)
    let (title, text): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT title, text FROM items WHERE id=?",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let item_text = item_text_text(title.as_deref(), text.as_deref());
    if item_text.is_empty() {
        return None;
    }
    crate::embed::embed_text(&[item_text]).ok()?.into_iter().next()
}

/// search._item_text: title + "\n" + body, truncated to 2000 chars.
pub fn item_text_text(title: Option<&str>, text: Option<&str>) -> String {
    let title = title.unwrap_or("").trim();
    let body = text.unwrap_or("").trim();
    let joined = format!("{title}\n{body}");
    joined.chars().take(2000).collect::<String>().trim().to_string()
}

/// public normalize wrapper for the search handler
pub fn normalize_pub(v: &[f32]) -> Option<Vec<f32>> {
    normalize(v)
}

/// make filter_ranked reachable from the vault module
pub fn filter_ranked_pub(
    conn: &Connection,
    ranked: Vec<(String, f64)>,
    kind: &str,
    source: &str,
    hue: &str,
    tag: &str,
    top_k: i64,
) -> Vec<(String, f64)> {
    filter_ranked(conn, ranked, kind, source, hue, tag, top_k)
}

pub fn similar_to_item(
    conn: &Connection,
    item_id: &str,
    top_k: i64,
    kind: &str,
    source: &str,
) -> Vec<(String, f64)> {
    let Some(vec) = item_vec(conn, item_id) else { return Vec::new() };
    let Some(vec) = normalize(&vec) else { return Vec::new() };
    query_vec(conn, &vec, top_k, kind, source, "", "", exclude_of(item_id))
}

fn exclude_of(item_id: &str) -> Option<&str> {
    if item_id.is_empty() {
        None
    } else {
        Some(item_id)
    }
}

pub fn query_vec(
    conn: &Connection,
    vec: &[f32],
    top_k: i64,
    kind: &str,
    source: &str,
    hue: &str,
    tag: &str,
    exclude_id: Option<&str>,
) -> Vec<(String, f64)> {
    let index = ensure_index(conn);
    let n = index.ids.len();
    if n == 0 || index.mat.len() != n * EMBED_DIM {
        return Vec::new();
    }
    let mut scores: Vec<f32> = Vec::with_capacity(n);
    for row in 0..n {
        let base = row * EMBED_DIM;
        let score = dot32(vec, &index.mat[base..base + EMBED_DIM]); // numpy: BLAS sdot
        scores.push(score);
    }
    let mut order: Vec<usize> = (0..n).collect();
    // numpy argsort is unstable; on bit-equal scores its introsort tends to
    // emit the LATER index first for equal pairs (observed on this dataset).
    order.sort_by(|a, b| {
        scores[*b]
            .partial_cmp(&scores[*a])
            .unwrap()
            .then(b.cmp(a))
    });
    let mut out: Vec<(String, f64)> = Vec::new();
    for idx in order {
        let score = scores[idx] as f64;
        if score <= 0.0 {
            break;
        }
        let item_id = &index.ids[idx];
        if let Some(excl) = exclude_id {
            if item_id == excl {
                continue;
            }
        }
        out.push((item_id.clone(), score));
        if out.len() as i64 >= top_k {
            break;
        }
    }
    if !(kind.is_empty() && source.is_empty() && hue.is_empty() && tag.is_empty()) {
        out = filter_ranked(conn, out, kind, source, hue, tag, top_k);
    }
    out
}

fn filter_ranked(
    conn: &Connection,
    ranked: Vec<(String, f64)>,
    kind: &str,
    source: &str,
    hue: &str,
    tag: &str,
    top_k: i64,
) -> Vec<(String, f64)> {
    let ids: Vec<String> = ranked.iter().map(|(i, _)| i.clone()).collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let ph = vec!["?"; ids.len()].join(",");
    let params: Vec<rusqlite::types::Value> =
        ids.iter().map(|s| rusqlite::types::Value::Text(s.clone())).collect();
    let mut info: std::collections::HashMap<String, (String, String, String)> =
        std::collections::HashMap::new();
    if let Ok(mut stmt) = conn.prepare(&format!(
        "SELECT i.id, i.kind, i.source, m.hue FROM items i LEFT JOIN media m ON m.rowid=(SELECT MIN(rowid) FROM media WHERE item_id=i.id) WHERE i.id IN ({ph})"
    )) {
        if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        }) {
            for row in rows.flatten() {
                info.insert(row.0.clone(), (row.1, row.2, row.3.unwrap_or_default()));
            }
        }
    }
    let tagged: Option<std::collections::HashSet<String>> = if tag.is_empty() {
        None
    } else {
        let mut set = std::collections::HashSet::new();
        if let Ok(mut stmt) = conn.prepare("SELECT item_id FROM tags WHERE tag=?") {
            if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter([rusqlite::types::Value::Text(tag.to_lowercase())].iter()), |r| r.get::<_, String>(0)) {
                for id in rows.flatten() {
                    set.insert(id);
                }
            }
        }
        Some(set)
    };
    let mut out: Vec<(String, f64)> = Vec::new();
    for (item_id, score) in ranked {
        let Some(rec) = info.get(&item_id) else { continue };
        let (k, src, hueval) = rec;
        if !kind.is_empty() && k != kind {
            continue;
        }
        if !source.is_empty() && src != source {
            continue;
        }
        if !hue.is_empty() && hueval != hue {
            continue;
        }
        if let Some(allowed) = &tagged {
            if !allowed.contains(&item_id) {
                continue;
            }
        }
        out.push((item_id, score));
        if out.len() as i64 >= top_k {
            break;
        }
    }
    out
}

// ---------- routes._canonical_item_url + _similar_keys ----------

fn looks_http(s: &str) -> bool {
    (s.len() >= 7 && s[..7].eq_ignore_ascii_case("http://"))
        || (s.len() >= 8 && s[..8].eq_ignore_ascii_case("https://"))
}

pub fn canonical_item_url(url: &str) -> String {
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
        return raw.to_lowercase().trim_end_matches('/').to_string();
    };
    let host_full = parsed.host_str().unwrap_or("").to_lowercase();
    let host = match host_full.split(':').next().unwrap_or("").strip_prefix("www.") {
        Some(h) => h.to_string(),
        None => host_full.clone(),
    };
    let path = parsed.path().trim_end_matches('/').to_string();
    if host == "x.com" || host == "twitter.com" {
        let re = regex_once();
        if let Some(m) = re.captures(&path) {
            return format!("x-status:{}", m.get(1).map(|g| g.as_str()).unwrap_or(""));
        }
    }
    format!("{}{}", host, path.to_lowercase())
}

fn regex_once() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)/(?:i/)?status/(\d+)").unwrap())
}

pub fn similar_keys(conn: &Connection, item_id: &str) -> std::collections::HashSet<String> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT url, source, title, text FROM items WHERE id=?",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();
    let (url, source, title, text) = match row {
        Some(v) => v,
        None => return std::collections::HashSet::new(),
    };
    let mut keys = std::collections::HashSet::new();
    let url_key = canonical_item_url(&url);
    if !url_key.is_empty() {
        keys.insert(format!("url:{url_key}"));
    }
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)(?:x|twitter)\.com/(?:i/)?status/(\d+)").unwrap());
    for m in re.captures_iter(&text) {
        keys.insert(format!("url:x-status:{}", m.get(1).map(|g| g.as_str()).unwrap_or("")));
    }
    // python: one normalized text — text wins when non-empty, else title
    let text_base = if text.trim().is_empty() { title.as_str() } else { text.as_str() };
    let norm: String = text_base
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !norm.is_empty() {
        keys.insert(format!("text:{}:{}", source, norm));
        keys.insert(format!("text-any:{norm}"));
    }
    let digest = media_digest(conn, item_id);
    if !digest.is_empty() {
        keys.insert(format!("media:{digest}"));
    }
    keys
}

/// routes._media_digest: sha256 of the FIRST media row's file, cached by name
/// forever (python caches without mtime — mirror that).
fn media_digest(conn: &Connection, item_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT thumb, file FROM media WHERE item_id=? ORDER BY rowid LIMIT 1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let (thumb, file) = match row {
        Some(v) => v,
        None => return String::new(),
    };
    let name = if thumb.is_empty() { file } else { thumb };
    if name.is_empty() {
        return String::new();
    }
    struct DigestCache;
    static DIGEST_CACHE: OnceLock<Mutex<std::collections::HashMap<String, String>>> = OnceLock::new();
    let cache = DIGEST_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock_ok();
    if let Some(hit) = map.get(&name) {
        return hit.clone();
    }
    let base = std::path::Path::new(&name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let path = std::path::Path::new(&media_dir()).join(&base);
    let digest = std::fs::read(&path)
        .map(|bytes| {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()
        })
        .unwrap_or_default();
    map.insert(name, digest.clone());
    digest
}

fn media_dir() -> String {
    format!(
        "{}/Projects/taste-vault/data/media",
        std::env::var("HOME").unwrap_or_default()
    )
}

/// routes._fast_similar_entries: first similar cards, deduped by keys.
pub fn fast_similar_entries(conn: &Connection, item_id: &str, limit: i64) -> Vec<Value> {
    let limit = limit.clamp(1, 48);
    let top_k = 48.max(limit * 4);
    let ranked = similar_to_item(conn, item_id, top_k, "", "");
    let mut seen = similar_keys(conn, item_id);
    let mut picked: Vec<(String, f64)> = Vec::new();
    for (candidate, score) in ranked {
        let keys = similar_keys(conn, &candidate);
        if !keys.is_empty() && seen.intersection(&keys).next().is_some() {
            continue;
        }
        seen.extend(keys);
        picked.push((candidate, score));
        if picked.len() as i64 >= limit {
            break;
        }
    }
    entries_from_ranked(conn, picked)
}

/// routes._entries_from_ranked (2-tuple variant): hydrate cards, add similarity.
pub fn entries_from_ranked(conn: &Connection, ranked: Vec<(String, f64)>) -> Vec<Value> {
    let ids: Vec<String> = ranked.iter().map(|(i, _)| i.clone()).collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let ph = vec!["?"; ids.len()].join(",");
    let params: Vec<Value> = ids.iter().map(|s| json!(s)).collect();
    let rows = crate::vault::fetch_rows(
        conn,
        &format!("SELECT * FROM items WHERE id IN ({ph}) AND (deleted_at IS NULL OR deleted_at='')"),
        &params,
    );
    let by_id: std::collections::HashMap<String, Map<String, Value>> = rows
        .into_iter()
        .filter_map(|r| {
            let id = r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())?;
            Some((id, r))
        })
        .collect();
    let ordered: Vec<Map<String, Value>> = ids.iter().filter_map(|i| by_id.get(i).cloned()).collect();
    let score_by: std::collections::HashMap<String, f64> = ranked.into_iter().collect();
    let mut items = crate::vault::rows_to_cards(conn, &ordered);
    for item in items.iter_mut() {
        if let Some(obj) = item.as_object_mut() {
            let id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let score = score_by.get(&id).copied().unwrap_or(0.0);
            obj.insert("similarity".into(), json!(pyround3(score)));
        }
    }
    items
}

fn pyround3(x: f64) -> f64 {
    format!("{:.3}", x).parse().unwrap_or(x)
}

/// Open a read-only clone of vault.db for index work.
pub fn open_readonly(path: &str) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// Silence unused-import risk if Mutex is removed by future refactors.
#[allow(dead_code)]
fn _keep() {
    let _ = Mutex::new(0u8);
}
