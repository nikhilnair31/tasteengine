//! Maintenance layer — /api/duplicates, /api/cleanup/*, /api/import,
//! full /api/items/{id}/similar. Ports of dedupe.duplicate_groups,
//! cleanup.py (source + media + empty channels, star groups), importer.py.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use regex::Regex;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::search;
use crate::vault;
use crate::write;
use crate::lock::LockExt;

// ---------- dedupe helpers (mirror of dedupe.py) ----------

fn record_content_key(record: &Map<String, Value>) -> String {
    let text = norm(record_get(record, "text"));
    if !text.is_empty() {
        return text;
    }
    norm(record_get(record, "title"))
}

fn record_get<'a>(record: &'a Map<String, Value>, key: &str) -> &'a str {
    record.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn norm(value: &str) -> String {
    let v = html_unescape(value).to_lowercase();
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_unescape(value: &str) -> String {
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

fn decode_entity(ent: &str) -> Option<String> {
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
    ];
    for (name, val) in named {
        if *name == ent {
            return Some((*val).to_string());
        }
    }
    None
}

type Fingerprint = (String, Option<u64>);

fn media_fingerprints(record: &Map<String, Value>) -> Vec<Fingerprint> {
    let media = match record.get("media").and_then(|m| m.as_array()) {
        Some(m) => m,
        None => return Vec::new(),
    };
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
        let path = std::path::Path::new(&write::media_dir()).join(&base);
        if let Some(fp) = crate::vault::file_fingerprint(&path) {
            if !fp.0.is_empty() {
                out.push(fp);
            }
        }
    }
    out
}

fn phash_close(left: Option<u64>, right: Option<u64>, threshold: u64) -> bool {
    match (left, right) {
        (Some(l), Some(r)) => (l ^ r).count_ones() as u64 <= threshold,
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
    let mut used: HashSet<usize> = HashSet::new();
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

fn records_duplicate(left: &Map<String, Value>, right: &Map<String, Value>) -> bool {
    let left_source = vault::canonical_source_key(record_get(left, "url"));
    let right_source = vault::canonical_source_key(record_get(right, "url"));
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

fn keeper_score(record: &Map<String, Value>) -> Vec<(i64, String)> {
    let title = norm(record_get(record, "title"));
    let real_title = (!title.is_empty() && !is_placeholder_title(&title)) as i64;
    let text_length = norm(record_get(record, "text")).chars().count() as i64;
    let media_count = media_fingerprints(record).len() as i64;
    vec![
        (real_title, String::new()),
        ((!(record_get(record, "url").is_empty())) as i64, String::new()),
        ((record.get("explanation_done").and_then(|v| v.as_bool()).unwrap_or(false)) as i64, String::new()),
        (media_count, String::new()),
        (((!norm(record_get(record, "note")).is_empty())) as i64, String::new()),
        ((record.get("tag_count").and_then(|v| v.as_i64()).unwrap_or(0)), String::new()),
        (text_length.min(4000), String::new()),
        (0, record_get(record, "saved_at").to_string()),
    ]
}

fn keeper_cmp(a: &[(i64, String)], b: &[(i64, String)]) -> std::cmp::Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        match x.0.cmp(&y.0) {
            std::cmp::Ordering::Equal => match x.1.cmp(&y.1) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            },
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn is_placeholder_title(value: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:x post(?:\s+\d+)?|x\.com|twitter\.com|instagram\.com|reddit\.com|reddit - the heart of the internet|reddit link|saved image|shared file|[a-z0-9.-]+\.[a-z]{2,})$").unwrap()
    });
    re.is_match(&norm(value))
}

pub fn is_placeholder_title_pub(value: &str) -> bool {
    is_placeholder_title(value)
}

/// dedupe.duplicate_groups over _load_dedupe_records
pub fn duplicates_payload(conn: &Connection) -> Value {
    let rows = vault::fetch_rows(
        conn,
        "SELECT * FROM items WHERE deleted_at IS NULL OR deleted_at='' ORDER BY saved_at, rowid",
        &[],
    );
    let mut records: Vec<Map<String, Value>> = Vec::new();
    for row in rows {
        let mut rec = row.clone();
        let id = rec.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let media = vault::fetch_rows(
            conn,
            "SELECT * FROM media WHERE item_id=? ORDER BY rowid",
            &[json!(id)],
        );
        let tag_count = vault::fetch_scalar_val(conn, "SELECT COUNT(*) c FROM tags WHERE item_id=?", &[json!(id)]);
        let explanation = vault::fetch_rows(
            conn,
            "SELECT status FROM explanations WHERE item_id=?",
            &[json!(id)],
        )
        .into_iter()
        .next();
        rec.insert("media".into(), Value::Array(media.into_iter().map(Value::Object).collect()));
        rec.insert("tag_count".into(), json!(tag_count));
        rec.insert(
            "explanation_done".into(),
            json!(explanation
                .and_then(|e| e.get("status").and_then(|v| v.as_str()).map(|s| s == "done").or(Some(false)))
                .unwrap_or(false)),
        );
        records.push(rec);
    }

    // candidate grouping
    let mut candidates: HashMap<String, Vec<Map<String, Value>>> = HashMap::new();
    let mut key_order: Vec<String> = Vec::new();
    for record in &records {
        let source = vault::canonical_source_key(record_get(record, "url"));
        let key = if !source.is_empty() {
            format!("source:{source}")
        } else {
            let content = record_content_key(record);
            if content.len() >= 20 { format!("text:{content}") } else { String::new() }
        };
        if !key.is_empty() {
            if !candidates.contains_key(&key) {
                key_order.push(key.clone());
            }
            candidates.entry(key).or_default().push(record.clone());
        }
    }

    // union-find components within each candidate group
    let mut groups: Vec<Vec<Map<String, Value>>> = Vec::new();
    for key in &key_order {
        let cands = &candidates[key];
        if cands.len() <= 1 {
            continue;
        }
        let n = cands.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let find = |parent: &mut Vec<usize>, mut i: usize| -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        };
        for left in 0..n {
            for right in left + 1..n {
                if records_duplicate(&cands[left], &cands[right]) {
                    let (rl, rr) = (find(&mut parent, left), find(&mut parent, right));
                    if rl != rr {
                        parent[rr] = rl;
                    }
                }
            }
        }
        let mut by_root: HashMap<usize, Vec<Map<String, Value>>> = HashMap::new();
        for (i, record) in cands.iter().enumerate() {
            by_root.entry(find(&mut parent, i)).or_default().push(record.clone());
        }
        for group in by_root.into_values() {
            if group.len() > 1 {
                groups.push(group);
            }
        }
    }
    // unique by sorted id set
    let mut unique: HashMap<Vec<String>, Vec<Map<String, Value>>> = HashMap::new();
    let mut uniq_order: Vec<Vec<String>> = Vec::new();
    for group in groups {
        let mut ids: Vec<String> = group
            .iter()
            .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();
        ids.sort();
        if !unique.contains_key(&ids) {
            uniq_order.push(ids.clone());
        }
        unique.insert(ids, group);
    }
    let groups: Vec<Vec<Map<String, Value>>> = uniq_order.iter().map(|k| unique[k].clone()).collect();

    // summary
    let mut by_source: HashMap<String, i64> = HashMap::new();
    let mut source_order: Vec<String> = Vec::new();
    let mut duplicates = 0i64;
    for group in &groups {
        let key = vault::canonical_source_key(record_get(&group[0], "url"));
        let source = if key.is_empty() {
            "text".to_string()
        } else {
            key.split('|').next().unwrap_or("").to_string()
        };
        let source = if source.is_empty() { "text".to_string() } else { source };
        if !by_source.contains_key(&source) {
            source_order.push(source.clone());
        }
        *by_source.entry(source).or_insert(0) += group.len() as i64 - 1;
        duplicates += group.len() as i64 - 1;
    }
    let mut sorted_sources: Vec<(String, i64)> = source_order
        .into_iter()
        .map(|s| (s.clone(), by_source[&s]))
        .collect();
    sorted_sources.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); // (-count, name)
    let mut by_source_obj = Map::new();
    for (s, c) in &sorted_sources {
        by_source_obj.insert(s.clone(), json!(c));
    }

    json!({
        "summary": {
            "groups": groups.len(),
            "duplicates": duplicates,
            "by_source": Value::Object(by_source_obj),
        },
        "groups": groups
            .iter()
            .map(|group| {
                // keeper = max(group, key=keeper_score); python max keeps the FIRST max
                let mut best: Option<(Vec<(i64, String)>, String)> = None;
                for record in group {
                    let score = keeper_score(record);
                    let id = record.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    match &best {
                        None => best = Some((score, id)),
                        Some((bs, _)) => {
                            if keeper_cmp(&score, bs) == std::cmp::Ordering::Greater {
                                best = Some((score, id));
                            }
                        }
                    }
                }
                json!({
                    "keeper": best.map(|(_, id)| id).unwrap_or_default(),
                    "items": group.iter().map(|item| json!({
                        "id": item.get("id").cloned().unwrap_or(Value::Null),
                        "title": item.get("title").cloned().unwrap_or(Value::Null),
                        "url": item.get("url").cloned().unwrap_or(Value::Null),
                        "kind": item.get("kind").cloned().unwrap_or(Value::Null),
                        "source": item.get("source").cloned().unwrap_or(Value::Null),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

// ---------- cleanup (cleanup.py; text channel removed in python — dead code) ----------

const PHASH_BITS: u64 = 8;
const MAX_GROUP_ITEMS: usize = 30;
const MAX_IDS_PER_DELETE: usize = 100;

struct CleanupCache {
    version: (i64, i64, i64, i64, i64),
    tsim: f64,
    payload: Value,
}

static CLEANUP_CACHE: Mutex<Option<CleanupCache>> = Mutex::new(None);

fn cleanup_normalized(value: &str) -> String {
    value.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn corroborates(left_id: &str, right_id: &str, rows_by: &HashMap<String, Map<String, Value>>) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let token_re = RE.get_or_init(|| Regex::new(r"[a-z0-9_]{2,}").unwrap());
    let left = rows_by.get(left_id);
    let right = rows_by.get(right_id);
    let (Some(left), Some(right)) = (left, right) else { return false };
    let left_key = record_get(left, "dedupe_key").trim();
    let right_key = record_get(right, "dedupe_key").trim();
    if !left_key.is_empty() && left_key == right_key {
        return true;
    }
    let left_text = cleanup_normalized(&format!(
        "{} {}",
        record_get(left, "text"),
        record_get(left, "title")
    ));
    let right_text = cleanup_normalized(&format!(
        "{} {}",
        record_get(right, "text"),
        record_get(right, "title")
    ));
    if left_text.len() >= 20 && right_text.len() >= 20 {
        let lt: HashSet<&str> = token_re.find_iter(&left_text).map(|m| m.as_str()).collect();
        let rt: HashSet<&str> = token_re.find_iter(&right_text).map(|m| m.as_str()).collect();
        if lt.intersection(&rt).count() >= 4 {
            return true;
        }
    }
    false
}

fn media_pairs(
    media_rows: &[Map<String, Value>],
    rows_by: &HashMap<String, Map<String, Value>>,
) -> Vec<(String, String, f64, String)> {
    // exact bytes first
    let mut by_sha: HashMap<String, Vec<String>> = HashMap::new();
    for m in media_rows {
        let sha = record_get(m, "sha256").trim().to_string();
        if !sha.is_empty() {
            by_sha.entry(sha).or_default().push(record_get(m, "item_id").to_string());
        }
    }
    let mut pairs: Vec<(String, String, f64, String)> = Vec::new();
    for members in by_sha.into_values() {
        if members.len() < 2 {
            continue;
        }
        let first = members[0].clone();
        for other in &members[1..] {
            if first != *other {
                pairs.push((first.clone(), other.clone(), 1.0, "media".into()));
            }
        }
    }
    // phash with corroboration
    let hashed: Vec<(String, u64)> = media_rows
        .iter()
        .filter_map(|m| {
            let phash = m.get("phash").and_then(|v| v.as_i64()).unwrap_or(0);
            if phash != 0 {
                Some((record_get(m, "item_id").to_string(), phash as u64))
            } else {
                None
            }
        })
        .collect();
    if hashed.len() >= 2 {
        for i in 0..hashed.len() {
            for j in i + 1..hashed.len() {
                let hd = (hashed[i].1 ^ hashed[j].1).count_ones() as u64;
                if hd == 0 || hd > PHASH_BITS {
                    continue;
                }
                let (left, right) = (&hashed[i].0, &hashed[j].0);
                if left == right {
                    continue;
                }
                if !corroborates(left, right, rows_by) {
                    continue;
                }
                let score = (1.0 - hd as f64 / 64.0).max(0.0);
                pairs.push((left.clone(), right.clone(), score, "media".into()));
            }
        }
    }
    // dedupe pairs keeping the highest score
    let mut best: HashMap<Vec<String>, (f64, String)> = HashMap::new();
    for (left, right, score, basis) in pairs {
        let mut key = vec![left.clone(), right.clone()];
        key.sort();
        match best.get(&key) {
            Some((s, _)) if *s >= score => {}
            _ => {
                best.insert(key, (score, basis));
            }
        }
    }
    best.into_iter()
        .map(|(mut key, (score, basis))| {
            let b = key.pop().unwrap();
            let a = key.pop().unwrap();
            (a, b, score, basis)
        })
        .collect()
}

fn cleanup_keeper_score(item_id: &str, rows_by: &HashMap<String, Map<String, Value>>) -> Vec<(i64, String)> {
    let d = match rows_by.get(item_id) {
        Some(d) => d,
        None => return Vec::new(),
    };
    let title = record_get(d, "title").trim();
    let url = record_get(d, "url").trim();
    let source = record_get(d, "source").trim().to_lowercase();
    let text = record_get(d, "text").trim();
    let real_url = (!url.is_empty()) as i64;
    let mut x_username = 0i64;
    if !url.is_empty() && source == "x" {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"/[^/]+/status/").unwrap());
        if re.is_match(url) && !url.contains("/i/status/") {
            x_username = 1;
        }
    }
    let real_title = ((!title.is_empty()) && !is_placeholder_title(title)) as i64;
    vec![
        (x_username, String::new()),
        (real_url, String::new()),
        (real_title, String::new()),
        (text.chars().count() as i64, String::new()),
        (0, record_get(d, "saved_at").to_string()),
    ]
}

fn cleanup_cover_fields(cover: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    for field in ["thumb", "hue", "palette", "width", "height", "duration", "kind"] {
        let value = cover.get(field).cloned().unwrap_or(Value::Null);
        let value = if field == "palette" {
            // python uses media_to_dict: palette arrives as a JSON string
            match &value {
                Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(json!([])),
                other => other.clone(),
            }
        } else {
            value
        };
        out.insert(field.into(), value);
    }
    Value::Object(out)
}

pub fn cleanup_payload(conn: &Connection, tsim: f64, max_groups: i64) -> Value {
    let version = search::db_version(conn);
    {
        let cache = CLEANUP_CACHE.lock_ok();
        if let Some(hit) = cache.as_ref() {
            if hit.version == version && hit.tsim == tsim {
                return hit.payload.clone();
            }
        }
    }
    let rows = vault::fetch_rows(
        conn,
        "SELECT * FROM items WHERE deleted_at IS NULL OR deleted_at='' ORDER BY saved_at, rowid",
        &[],
    );
    let mut rows_by: HashMap<String, Map<String, Value>> = HashMap::new();
    for d in &rows {
        if let Some(id) = d.get("id").and_then(|v| v.as_str()) {
            rows_by.insert(id.to_string(), d.clone());
        }
    }
    // 1: same canonical source
    let mut edges: Vec<(String, String, f64, String)> = Vec::new();
    let mut by_key: HashMap<String, Vec<String>> = HashMap::new();
    for d in &rows {
        let key = record_get(d, "dedupe_key").trim().to_string();
        if !key.is_empty() {
            if let Some(id) = d.get("id").and_then(|v| v.as_str()) {
                by_key.entry(key).or_default().push(id.to_string());
            }
        }
    }
    for members in by_key.into_values() {
        if members.len() > 1 {
            let first = members[0].clone();
            for item_id in &members[1..] {
                edges.push((first.clone(), item_id.clone(), 1.0, "source".into()));
            }
        }
    }
    // 2: same picture
    let media_rows = vault::fetch_rows(
        conn,
        "SELECT item_id, sha256, phash FROM media WHERE placeholder=0 \
         AND item_id NOT IN (SELECT id FROM items WHERE deleted_at IS NOT NULL AND deleted_at != '')",
        &[],
    );
    edges.extend(media_pairs(&media_rows, &rows_by));

    // star groups
    let mut adjacency: HashMap<String, Vec<(String, f64, String)>> = HashMap::new();
    for (left, right, score, basis) in &edges {
        adjacency
            .entry(left.clone())
            .or_default()
            .push((right.clone(), *score, basis.clone()));
        adjacency
            .entry(right.clone())
            .or_default()
            .push((left.clone(), *score, basis.clone()));
    }
    let mut order: Vec<String> = rows
        .iter()
        .filter_map(|d| d.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    order.sort_by(|a, b| {
        let sa = cleanup_keeper_score(a, &rows_by);
        let sb = cleanup_keeper_score(b, &rows_by);
        keeper_cmp(&sb, &sa) // reverse=True, stable: ties keep rows order
    });
    let mut used: HashSet<String> = HashSet::new();
    let mut groups: Vec<Value> = Vec::new();
    for leader in &order {
        if used.contains(leader) {
            continue;
        }
        let related = adjacency.get(leader).cloned().unwrap_or_default();
        let mut members: Vec<(String, Option<f64>, Option<String>)> = vec![(leader.clone(), None, None)];
        let mut scores = 0.0f64;
        let mut basis: Vec<String> = Vec::new();
        for (other, score, b) in related {
            if used.contains(&other) {
                continue;
            }
            used.insert(other.clone());
            members.push((other, Some(score), Some(b.clone())));
            if score != 0.0 {
                scores = scores.max(score);
            }
            if !b.is_empty() && !basis.contains(&b) {
                basis.push(b);
            }
        }
        if members.len() < 2 {
            continue;
        }
        used.insert(leader.clone());
        members.sort_by(|a, b| {
            let ka = (if a.1.is_none() { 0.0 } else { -a.1.unwrap() }, a.0.clone());
            let kb = (if b.1.is_none() { 0.0 } else { -b.1.unwrap() }, b.0.clone());
            ka.partial_cmp(&kb).unwrap()
        });
        let full_size = members.len();
        let member_ids: Vec<String> = members.iter().map(|m| m.0.clone()).collect();
        basis.sort();
        groups.push(json!({
            "leader": leader,
            "basis": basis,
            "score": format!("{:.3}", scores).parse::<f64>().unwrap_or(scores),
            "members": member_ids,
            "full_size": full_size,
            "_members_typed": members,
        }));
    }
    groups.sort_by(|a, b| {
        let la = a.get("_members_typed").and_then(|m| m.as_array()).map(|m| m.len()).unwrap_or(0);
        let lb = b.get("_members_typed").and_then(|m| m.as_array()).map(|m| m.len()).unwrap_or(0);
        lb.cmp(&la).then(
            b.get("score")
                .and_then(|v| v.as_f64())
                .partial_cmp(&a.get("score").and_then(|v| v.as_f64()))
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let mut selected: Vec<Value> = if max_groups > 0 {
        groups.into_iter().take(max_groups as usize).collect()
    } else {
        groups
    };

    // 3: empty link saves
    let used_ids: HashSet<String> = selected
        .iter()
        .flat_map(|g| g.get("_members_typed").and_then(|m| m.as_array()).cloned().unwrap_or_default())
        .filter_map(|m| m.get(0).and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let empty_ids: Vec<String> = rows
        .iter()
        .filter(|d| {
            record_get(d, "kind") == "link" && record_get(d, "url").trim().is_empty()
        })
        .filter_map(|d| d.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    for chunk_start in (0..empty_ids.len()).step_by(MAX_GROUP_ITEMS) {
        let chunk: Vec<String> = empty_ids[chunk_start..(chunk_start + MAX_GROUP_ITEMS).min(empty_ids.len())]
            .iter()
            .filter(|i| !used_ids.contains(*i))
            .cloned()
            .collect();
        if chunk.is_empty() {
            continue;
        }
        selected.push(json!({
            "leader": Value::Null,
            "basis": ["empty"],
            "score": 0.0,
            "members": chunk,
            "full_size": chunk.len(),
            "_members_typed": chunk.iter().map(|i| json!([i, Value::Null, Value::Null])).collect::<Vec<_>>(),
        }));
    }

    let all_member_ids: Vec<String> = selected
        .iter()
        .flat_map(|g| g.get("members").and_then(|m| m.as_array()).cloned().unwrap_or_default())
        .filter_map(|m| m.as_str().map(|s| s.to_string()))
        .collect();
    let mut covers: HashMap<String, Map<String, Value>> = HashMap::new();
    if !all_member_ids.is_empty() {
        let ph = vec!["?"; all_member_ids.len()].join(",");
        let params: Vec<Value> = all_member_ids.iter().map(|s| json!(s)).collect();
        for row in vault::fetch_rows(
            conn,
            &format!(
                "SELECT * FROM media WHERE item_id IN ({ph}) AND placeholder=0 \
                 AND kind IN ('image','video') AND thumb<>'' ORDER BY rowid"
            ),
            &params,
        ) {
            let iid = row.get("item_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            covers.entry(iid).or_insert(row);
        }
    }

    let payload = json!({
        "scanned": rows.len(),
        "version": [version.0, version.1, version.2, version.3, version.4],
        "thresholds": {"text": tsim, "phash_bits": PHASH_BITS},
        "groups": selected
            .iter()
            .map(|g| {
                let members_typed: Vec<(String, Option<f64>, Option<String>)> = g
                    .get("_members_typed")
                    .and_then(|m| m.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|m| {
                                (
                                    m.get(0).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                    m.get(1).and_then(|v| v.as_f64()),
                                    m.get(2).and_then(|v| v.as_str()).map(|s| s.to_string()),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!({
                    "leader": g.get("leader").cloned().unwrap_or(Value::Null),
                    "full_size": g.get("full_size").cloned().unwrap_or(Value::Null),
                    "evidence": g.get("basis").cloned().unwrap_or(json!([])),
                    "score": g.get("score").cloned().unwrap_or(Value::Null),
                    "members": members_typed
                        .iter()
                        .map(|(item_id, _, _)| {
                            let d = rows_by.get(item_id);
                            let cover = covers.get(item_id);
                            json!({
                                "id": item_id,
                                "kind": d.map(|d| d.get("kind").cloned().unwrap_or(Value::Null)).unwrap_or(Value::Null),
                                "title": d.map(|d| json!(record_get(d, "title"))).unwrap_or(json!("")),
                                "url": d.map(|d| json!(record_get(d, "url"))).unwrap_or(json!("")),
                                "source": d.map(|d| json!(record_get(d, "source"))).unwrap_or(json!("")),
                                "saved_at": d.map(|d| json!(record_get(d, "saved_at"))).unwrap_or(json!("")),
                                "cover": cover.map(cleanup_cover_fields).unwrap_or(Value::Null),
                            })
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>(),
    });
    let mut cache = CLEANUP_CACHE.lock_ok();
    *cache = Some(CleanupCache { version, tsim, payload: payload.clone() });
    payload
}

pub fn delete_similar(conn: &Connection, item_ids: &[String]) -> Value {
    let cleaned: Vec<String> = item_ids
        .iter()
        .filter(|i| !i.trim().is_empty())
        .take(MAX_IDS_PER_DELETE)
        .cloned()
        .collect();
    let mut deleted_count = 0;
    let mut missing: Vec<String> = Vec::new();
    for item_id in &cleaned {
        let row = vault::fetch_rows(conn, "SELECT id FROM items WHERE id=?", &[json!(item_id)]);
        if row.is_empty() {
            missing.push(item_id.clone());
            continue;
        }
        let _ = conn.execute(
            "UPDATE items SET deleted_at=datetime('now') WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            [item_id.as_str()],
        );
        deleted_count += 1;
    }
    json!({"deleted": deleted_count, "missing": missing})
}

pub fn delete_type_channel(conn: &Connection, channel: &str) -> Result<Value, (u16, String)> {
    let payload = cleanup_payload(conn, 0.62, 0);
    let mut ids: Vec<String> = Vec::new();
    for g in payload.get("groups").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
        let members = g.get("members").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        if members.is_empty() {
            continue;
        }
        if channel != "all" {
            let evidence = g.get("evidence").and_then(|e| e.as_array()).cloned().unwrap_or_default();
            let has = evidence.iter().any(|e| e.as_str() == Some(channel));
            if !has {
                continue;
            }
        }
        let leader = g.get("leader").and_then(|v| v.as_str()).unwrap_or("");
        for m in members {
            if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                if id != leader {
                    ids.push(id.to_string());
                }
            }
        }
    }
    let mut deleted = 0;
    let mut missing: Vec<String> = Vec::new();
    for chunk in ids.chunks(MAX_IDS_PER_DELETE) {
        let result = delete_similar(conn, chunk);
        deleted += result.get("deleted").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(Value::Array(m)) = result.get("missing") {
            for x in m {
                if let Some(s) = x.as_str() {
                    missing.push(s.to_string());
                }
            }
        }
    }
    Ok(json!({"deleted": deleted, "missing": missing, "channel": channel}))
}

// ---------- import (importer.py) ----------

fn importer_canonical_key(url: &str) -> String {
    let mut u = url.trim().to_string();
    if u.is_empty() {
        return String::new();
    }
    if let Some(pos) = u.find('#') {
        u.truncate(pos);
    }
    u = u.replace("https://twitter.com", "https://x.com");
    let re = |pat: &'static str| -> Option<String> {
        static CACHE: Mutex<Option<HashMap<&'static str, Regex>>> = Mutex::new(None);
        let mut guard = CACHE.lock_ok();
        if guard.is_none() {
            let mut m = HashMap::new();
            for (k, p) in [
                ("tweet1", r"x\.com/\w+/status/(\d+)"),
                ("tweet2", r"x\.com/i/status/(\d+)"),
                ("ig", r"instagram\.com/(?:p|reel|tv)/([\w-]+)"),
                ("reddit1", r"reddit\.com/r/\w+/comments/(\w+)"),
                ("reddit2", r"redd\.it/(\w+)"),
                ("yt1", r"youtube\.com/watch\?.*v=([\w-]{6,})"),
                ("yt2", r"youtu\.be/([\w-]{6,})"),
                ("yt3", r"youtube\.com/shorts/([\w-]{6,})"),
                ("pin1", r"pinterest\.com/pin/(\d+)"),
                ("pin2", r"pin\.it/(\w+)"),
            ] {
                m.insert(k, Regex::new(p).unwrap());
            }
            *guard = Some(m);
        }
        guard.as_ref().unwrap().get(pat).and_then(|re| re.captures(&u)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
    };
    if let Some(id) = re("tweet1").or_else(|| re("tweet2")) {
        return format!("tweet|{id}");
    }
    if let Some(id) = re("ig") {
        return format!("ig|{id}");
    }
    if let Some(id) = re("reddit1").or_else(|| re("reddit2")) {
        return format!("reddit|{id}");
    }
    if let Some(id) = re("yt1").or_else(|| re("yt2")).or_else(|| re("yt3")) {
        return format!("youtube|{id}");
    }
    if let Some(id) = re("pin1").or_else(|| re("pin2")) {
        return format!("pin|{id}");
    }
    let parsed = url::Url::parse(&u).ok();
    if let Some(p) = parsed {
        let mut host = p.host_str().unwrap_or("").to_lowercase();
        if let Some(h) = host.strip_prefix("www.") {
            host = h.to_string();
        }
        let mut path = p.path().trim_end_matches('/').to_string();
        if path.is_empty() {
            path = "/".into();
        }
        return format!("web|{host}{path}");
    }
    String::new()
}

const URL_KEYS: [&str; 11] = [
    "permalink", "posturl", "url", "link", "address", "savedurl",
    "tweeturl", "bookmarkurl", "sourceurl", "href", "permalink",
];

fn url_from_value(value: &Value) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?i)https?://[^\s<>"']+"#).unwrap());
    let text = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match re.find(&text) {
        Some(m) => {
            let mut u = m.as_str().to_string();
            while u.ends_with(['.', ',', ';', ':', ')', ']', '}', '>']) {
                u.pop();
            }
            u
        }
        None => String::new(),
    }
}

fn header_key(value: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[^a-z0-9]").unwrap());
    re.replace_all(&value.to_lowercase(), "").to_string()
}

fn parse_urls(data: &[u8], ext: &str) -> Vec<String> {
    let text = String::from_utf8_lossy(data);
    let text = text.trim_start_matches('\u{feff}').to_string();
    if ext == "json" {
        let Ok(obj) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
        let items = if obj.is_array() {
            obj
        } else {
            obj.get("items").cloned().or_else(|| obj.get("data").cloned()).unwrap_or(json!([]))
        };
        let mut urls = Vec::new();
        if let Some(list) = items.as_array() {
            for it in list {
                let url = match it {
                    Value::String(s) => url_from_value(&json!(s)),
                    Value::Object(_) => {
                        let mut url = String::new();
                        let normalized: HashMap<String, Value> = it
                            .as_object()
                            .map(|o| o.iter().map(|(k, v)| (header_key(k), v.clone())).collect())
                            .unwrap_or_default();
                        for k in ["permalink", "posturl", "url", "link", "address", "savedurl", "tweeturl", "bookmarkurl", "sourceurl", "href"] {
                            url = url_from_value(normalized.get(k).unwrap_or(&Value::Null));
                            if !url.is_empty() {
                                break;
                            }
                        }
                        url
                    }
                    _ => String::new(),
                };
                if !url.is_empty() {
                    urls.push(url);
                }
            }
        }
        return urls;
    }
    if ext == "csv" {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(text.as_bytes());
        let headers: Vec<String> = rdr.headers().map(|h| h.iter().map(|s| header_key(s)).collect()).unwrap_or_default();
        let mut urls = Vec::new();
        for row in rdr.records().flatten() {
            let normalized: HashMap<String, String> = headers
                .iter()
                .zip(row.iter())
                .map(|(h, v)| (h.clone(), v.to_string()))
                .collect();
            let mut url = String::new();
            for k in URL_KEYS {
                url = url_from_value(&json!(normalized.get(k).cloned().unwrap_or_default()));
                if !url.is_empty() {
                    break;
                }
            }
            if url.is_empty() {
                for v in row.iter() {
                    url = url_from_value(&json!(v));
                    if !url.is_empty() {
                        break;
                    }
                }
            }
            if !url.is_empty() {
                urls.push(url);
            }
        }
        if urls.is_empty() {
            // headerless CSV
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(text.as_bytes());
            for row in rdr.records().flatten() {
                for cell in row.iter() {
                    let url = url_from_value(&json!(cell));
                    if !url.is_empty() {
                        urls.push(url);
                        break;
                    }
                }
            }
        }
        return urls;
    }
    text.lines().filter_map(|line| {
        let url = url_from_value(&json!(line));
        if url.is_empty() { None } else { Some(url) }
    }).collect()
}

pub fn run_import(conn: &Connection, data: &[u8], filename: &str) -> Value {
    let ext = if filename.contains('.') {
        filename.rsplit('.').next().unwrap_or("").to_lowercase()
    } else {
        "txt".into()
    };
    let ext = if ["csv", "json", "txt"].contains(&ext.as_str()) { ext } else { "csv".into() };
    let urls = parse_urls(data, &ext);
    if urls.is_empty() {
        return json!({"parsed": 0, "new": 0, "skipped": 0, "error": "no URLs found in file"});
    }
    let mut seen_keys: HashSet<String> = HashSet::new();
    for r in vault::fetch_rows(conn, "SELECT url FROM items", &[]) {
        let key = importer_canonical_key(record_get(&r, "url"));
        if !key.is_empty() {
            seen_keys.insert(key);
        }
    }
    let mut new_items = 0i64;
    let mut skipped = 0i64;
    let mut imported_ids: Vec<String> = Vec::new();
    for url in &urls {
        let key = vault::canonical_source_key(url);
        if key.is_empty() || seen_keys.contains(&key) {
            skipped += 1;
            continue;
        }
        seen_keys.insert(key);
        let item_id = write::token_hex8();
        let platform = write::platform_of(url);
        let kind = if ["youtube", "tiktok", "vimeo"].contains(&platform.as_str()) { "video" } else { "image" };
        let extra = json!({"image_urls": [], "video_url": "", "no_explain": true, "bulk": true});
        let _ = conn.execute(
            "INSERT INTO items (id, url, dedupe_key, title, kind, source, platform, text, note, extra) \
             VALUES (?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![
                item_id, url, vault::canonical_source_key(url), "", kind, platform, platform, "", "",
                serde_json::to_string(&extra).unwrap_or_default(),
            ],
        );
        let _ = conn.execute("INSERT INTO tags(item_id, tag) VALUES(?, 'imported')", rusqlite::params![item_id]);
        new_items += 1;
        imported_ids.push(item_id);
    }
    for item_id in &imported_ids {
        write::enqueue(conn, item_id);
    }
    json!({"parsed": urls.len(), "new": new_items, "skipped": skipped, "queued": new_items})
}

// ---------- full similar (routes._all_similar_ranked) ----------

struct SimilarCache {
    key: (String, (i64, i64, i64, i64, i64)),
    ranked: Vec<(String, f64)>,
}

static SIMILAR_CACHE: Mutex<Option<SimilarCache>> = Mutex::new(None);

pub fn all_similar_ranked(conn: &Connection, item_id: &str) -> Result<Vec<(String, f64)>, (u16, String)> {
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
    let version = search::db_version(conn);
    {
        let cache = SIMILAR_CACHE.lock_ok();
        if let Some(hit) = cache.as_ref() {
            if hit.key == (item_id.to_string(), version) {
                return Ok(hit.ranked.clone());
            }
        }
    }
    let ranked = search::similar_to_item(conn, item_id, i64::MAX - 1, "", "");
    // query_vec with top_k=None uses request_limit = len(ids)+1
    let mut seen = search::similar_keys(conn, item_id);
    let mut out: Vec<(String, f64)> = Vec::new();
    for (candidate, score) in ranked {
        let keys = search::similar_keys(conn, &candidate);
        if !keys.is_empty() && seen.intersection(&keys).next().is_some() {
            continue;
        }
        seen.extend(keys);
        out.push((candidate, score));
    }
    {
        let mut cache = SIMILAR_CACHE.lock_ok();
        *cache = Some(SimilarCache { key: (item_id.to_string(), version), ranked: out.clone() });
    }
    Ok(out)
}
