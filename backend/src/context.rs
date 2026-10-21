//! routes.vault_context + vault_image_sources — MCP context tools.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::maintain;
use crate::mediaio;
use crate::vault;
use crate::write;

fn superscript_number(number: usize) -> String {
    let digits = ["\u{2070}", "\u{00b9}", "\u{00b2}", "\u{00b3}", "\u{2074}", "\u{2075}", "\u{2076}", "\u{2077}", "\u{2078}", "\u{2079}"];
    number
        .to_string()
        .chars()
        .map(|c| digits[c.to_digit(10).unwrap_or(0) as usize].to_string())
        .collect()
}

fn inline_citation(url: &str, number: usize) -> String {
    let url = url.trim();
    let is_http = url.to_lowercase().starts_with("http://") || url.to_lowercase().starts_with("https://");
    let netloc = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or("").to_string())
        .unwrap_or_default();
    if !is_http || netloc.is_empty() {
        return String::new();
    }
    format!("[{}]({})", superscript_number(number), url)
}

/// routes._context_excerpt — flatten preferred explainer values, dedupe, cap
fn context_excerpt(value: &Value, max_chars: usize) -> String {
    const PREFERRED: &[&str] = &[
        "summary", "visuals", "mood", "style_dna", "why_it_fits",
        "color_words", "video_note", "sections",
    ];
    fn flatten(entry: &Value, parts: &mut Vec<String>) {
        match entry {
            Value::String(s) => {
                let text = s.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            Value::Number(n) => parts.push(n.to_string()),
            Value::Array(a) => {
                for child in a {
                    flatten(child, parts);
                }
            }
            Value::Object(o) => {
                let mut keys: Vec<&String> = PREFERRED
                    .iter()
                    .filter_map(|p| o.keys().find(|k| k.as_str() == *p))
                    .collect();
                keys.extend(o.keys().filter(|k| !PREFERRED.contains(&k.as_str())));
                for key in keys {
                    if let Some(v) = o.get(key) {
                        flatten(v, parts);
                    }
                }
            }
            _ => {}
        }
    }
    let mut parts: Vec<String> = Vec::new();
    flatten(value, &mut parts);
    // dict.fromkeys order-preserving dedupe
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        if seen.insert(p.clone()) {
            out.push(p);
        }
    }
    out.join(" ").chars().take(max_chars).collect::<String>().trim_end().to_string()
}

fn context_media_record(row: &Map<String, Value>) -> Value {
    let parsed = crate::vault::media_to_dict_pub(row);
    json!({
        "id": parsed.get("id").cloned().unwrap_or(json!("")),
        "kind": parsed.get("kind").cloned().unwrap_or(json!("")),
        "mime": parsed.get("mime").cloned().unwrap_or(json!("")),
        "thumb": parsed.get("thumb").cloned().unwrap_or(json!("")),
        "width": parsed.get("width").cloned().unwrap_or(json!(0)),
        "height": parsed.get("height").cloned().unwrap_or(json!(0)),
        "duration": parsed.get("duration").cloned().unwrap_or(json!(0)),
        "palette": parsed.get("palette").cloned().unwrap_or(json!([])),
        "dominant": parsed.get("dominant").cloned().unwrap_or(json!("")),
        "hue": parsed.get("hue").cloned().unwrap_or(json!("")),
    })
}

/// routes._context_records
fn context_records(
    conn: &Connection,
    ranked: &[(String, f64, Vec<String>)],
) -> Vec<Value> {
    let mut ids: Vec<String> = Vec::new();
    for (item_id, _, _) in ranked {
        if !ids.contains(item_id) {
            ids.push(item_id.clone());
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<Value> = ids.iter().map(|s| json!(s)).collect();
    let rows = vault::fetch_rows(
        conn,
        &format!("SELECT * FROM items WHERE id IN ({placeholders}) AND (deleted_at IS NULL OR deleted_at='')"),
        &params,
    );
    let by_id: HashMap<String, Map<String, Value>> = rows
        .into_iter()
        .filter_map(|r| {
            let id = r.get("id").and_then(|v| v.as_str())?.to_string();
            Some((id, r))
        })
        .collect();
    let ordered_rows: Vec<Map<String, Value>> = ids.iter().filter_map(|i| by_id.get(i).cloned()).collect();
    let cards: HashMap<String, Value> = vault::items_to_list(conn, &ordered_rows)
        .into_iter()
        .filter_map(|c| {
            let id = c.get("id").and_then(|v| v.as_str())?.to_string();
            Some((id, c))
        })
        .collect();
    let mut media_by: HashMap<String, Vec<Map<String, Value>>> = HashMap::new();
    for row in vault::fetch_rows(
        conn,
        &format!("SELECT * FROM media WHERE item_id IN ({placeholders}) ORDER BY item_id, rowid"),
        &params,
    ) {
        if let Some(k) = row.get("item_id").and_then(|v| v.as_str()) {
            media_by.entry(k.to_string()).or_default().push(row);
        }
    }
    let mut explanations: HashMap<String, Value> = HashMap::new();
    for row in vault::fetch_rows(
        conn,
        &format!("SELECT item_id, status, data FROM explanations WHERE item_id IN ({placeholders})"),
        &params,
    ) {
        if let Some(k) = row.get("item_id").and_then(|v| v.as_str()) {
            explanations.insert(k.to_string(), Value::Object(row));
        }
    }
    let mut records: Vec<Value> = Vec::new();
    for (item_id, score, matched_by) in ranked {
        let Some(card) = cards.get(item_id) else { continue };
        let raw = match by_id.get(item_id) {
            Some(r) => r,
            None => continue,
        };
        let media_records: Vec<Value> = media_by
            .get(item_id)
            .map(|v| v.iter().filter(|m| m.get("placeholder").and_then(|p| p.as_i64()).unwrap_or(0) == 0).map(|m| context_media_record(m)).collect())
            .unwrap_or_default();
        let cover = card.get("cover").cloned().unwrap_or(json!({}));
        let palette = cover
            .get("palette")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().take(6).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let explanation = explanations.get(item_id);
        let mut expl_data = json!({});
        if let Some(e) = explanation {
            if e.get("status").and_then(|v| v.as_str()) == Some("done") {
                expl_data = serde_json::from_str(e.get("data").and_then(|v| v.as_str()).unwrap_or("{}")).unwrap_or(json!({}));
            }
        }
        let extra: Map<String, Value> = raw
            .get("extra")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut outlinks: Vec<Value> = Vec::new();
        let mut citation_urls: Vec<String> = Vec::new();
        let card_url = card.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if !card_url.is_empty() {
            citation_urls.push(card_url.chars().take(2000).collect());
        }
        for link in extra.get("outlinks").and_then(|v| v.as_array()).into_iter().flatten() {
            let link_url = link.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if link_url.is_empty() {
                continue;
            }
            let link_url = link_url.chars().take(2000).collect::<String>();
            let t = link.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let t2 = link.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let link_title = (if t.is_empty() { t2 } else { t }).chars().take(240).collect::<String>();
            outlinks.push(json!({
                "url": link_url,
                "title": link_title,
            }));
            if !citation_urls.contains(&link_url) {
                citation_urls.push(link_url);
            }
            if outlinks.len() >= 12 {
                break;
            }
        }
        let mut citations: Vec<Value> = Vec::new();
        for url in &citation_urls {
            let number = citations.len() + 1;
            let markdown = inline_citation(url, number);
            if markdown.is_empty() {
                continue;
            }
            citations.push(json!({
                "number": number,
                "marker": superscript_number(number),
                "url": url,
                "markdown": markdown,
            }));
        }
        let info = json!({
            "id": item_id,
            "title": card.get("title").cloned().unwrap_or(json!("")),
            "url": card.get("url").cloned().unwrap_or(json!("")),
            "source": card.get("source").cloned().unwrap_or(json!("")),
            "platform": card.get("platform").cloned().unwrap_or(json!("")),
            "author": card.get("author").cloned().unwrap_or(json!("")),
            "kind": card.get("kind").cloned().unwrap_or(json!("")),
            "saved_text": card.get("text").and_then(|v| v.as_str()).unwrap_or("").chars().take(2400).collect::<String>(),
            "note": card.get("note").and_then(|v| v.as_str()).unwrap_or("").chars().take(1200).collect::<String>(),
            "tags": card.get("tags").and_then(|v| v.as_array()).map(|a| a.iter().take(40).cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "palette": palette,
            "media": media_records,
            "media_count": media_records.len(),
            "explanation_status": explanation.and_then(|e| e.get("status")).cloned().unwrap_or(Value::Null),
            "explainer_excerpt": context_excerpt(&expl_data, 1200),
            "outlinks": outlinks,
            "citations": citations,
            "citation_markdown": citations.iter().filter_map(|c| c.get("markdown").and_then(|m| m.as_str())).collect::<Vec<_>>().join(" "),
            "similarity": vault::pyround_pub(*score, 4),
            "matched_by": matched_by,
        });
        records.push(info);
    }
    records
}

fn context_item_keys(item_row: &Map<String, Value>, media_rows: &[Map<String, Value>]) -> (String, HashSet<String>) {
    let source_key = vault::canonical_source_key(item_row.get("url").and_then(|v| v.as_str()).unwrap_or(""));
    let media_keys: HashSet<String> = media_rows
        .iter()
        .filter(|r| r.get("placeholder").and_then(|p| p.as_i64()).unwrap_or(0) == 0)
        .filter_map(|r| r.get("sha256").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    (source_key, media_keys)
}

/// routes.vault_context
pub fn vault_context(conn: &Connection, query: &str, seed_ids: &[String], limit: i64) -> Value {
    let query = query.trim().chars().take(1200).collect::<String>();
    let limit = limit.clamp(1, 24);
    let mut requested_seed_ids: Vec<String> = Vec::new();
    for raw_id in seed_ids.iter().take(12) {
        let item_id = raw_id.trim().chars().take(64).collect::<String>();
        if !item_id.is_empty() && !requested_seed_ids.contains(&item_id) {
            requested_seed_ids.push(item_id);
        }
    }
    let mut live_seeds: Vec<String> = Vec::new();
    let mut missing_seed_ids: Vec<String> = Vec::new();
    for item_id in &requested_seed_ids {
        let live = vault::fetch_rows(
            conn,
            "SELECT id FROM items WHERE id=? AND (deleted_at IS NULL OR deleted_at='')",
            &[json!(item_id)],
        )
        .into_iter()
        .next()
        .is_some();
        if live {
            live_seeds.push(item_id.clone());
        } else {
            missing_seed_ids.push(item_id.clone());
        }
    }

    let mut candidates: HashMap<String, (f64, Vec<String>)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut add_candidate = |item_id: &str, score: f64, matched_by: &str,
                             candidates: &mut HashMap<String, (f64, Vec<String>)>,
                             order: &mut Vec<String>| {
        if item_id.is_empty() {
            return;
        }
        let entry = candidates.entry(item_id.to_string()).or_insert_with(|| {
            order.push(item_id.to_string());
            (0.0f64, Vec::new())
        });
        entry.0 = entry.0.max(score);
        if !entry.1.iter().any(|m| m == matched_by) {
            entry.1.push(matched_by.to_string());
        }
        if matched_by == "seed" {
            entry.0 = 1.0;
        }
    };

    for item_id in &live_seeds {
        add_candidate(item_id, 1.0, "seed", &mut candidates, &mut order);
    }
    if !query.is_empty() {
        let search_result = vault::vault_search_pub(conn, &query, "auto", (limit * 2).min(48), "", "", "", "", 0);
        for card in search_result.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let score = card.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cid = card.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            add_candidate(&cid, score, "query", &mut candidates, &mut order);
            if score > 0.0 {
                add_candidate(&cid, score, "visual", &mut candidates, &mut order);
            }
        }
    }
    let similar_limit = limit.clamp(8, 24);
    for item_id in &live_seeds {
        let similar = match maintain::all_similar_ranked(conn, item_id) {
            Ok(r) => crate::search::filter_ranked_pub(conn, r, "", "", "", "", similar_limit),
            Err(_) => Vec::new(),
        };
        for (cid, score) in similar {
            add_candidate(&cid, score, "visual", &mut candidates, &mut order);
        }
    }

    if order.is_empty() {
        return json!({
            "query": query,
            "seed_ids": requested_seed_ids,
            "seed_ids_used": [],
            "missing_seed_ids": missing_seed_ids,
            "items": [],
            "total": 0,
        });
    }

    let candidate_ids = &order;
    let placeholders = candidate_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<Value> = candidate_ids.iter().map(|s| json!(s)).collect();
    let rows_by_id: HashMap<String, Map<String, Value>> = vault::fetch_rows(
        conn,
        &format!("SELECT * FROM items WHERE id IN ({placeholders}) AND (deleted_at IS NULL OR deleted_at='')"),
        &params,
    )
    .into_iter()
    .filter_map(|r| {
            let id = r.get("id").and_then(|v| v.as_str())?.to_string();
            Some((id, r))
        })
    .collect();
    let mut media_by_id: HashMap<String, Vec<Map<String, Value>>> = HashMap::new();
    for row in vault::fetch_rows(
        conn,
        &format!("SELECT item_id, sha256, placeholder FROM media WHERE item_id IN ({placeholders})"),
        &params,
    ) {
        if let Some(k) = row.get("item_id").and_then(|v| v.as_str()) {
            media_by_id.entry(k.to_string()).or_default().push(row);
        }
    }
    let mut seen_sources: HashSet<String> = HashSet::new();
    let mut seen_media: HashSet<String> = HashSet::new();
    let mut filtered_ranked: Vec<(String, f64, Vec<String>)> = Vec::new();
    let mut omitted_duplicate_ids: Vec<String> = Vec::new();
    for item_id in candidate_ids {
        let Some(row) = rows_by_id.get(item_id) else { continue };
        let (source_key, media_keys) = context_item_keys(row, media_by_id.get(item_id).map(|v| v.as_slice()).unwrap_or(&[]));
        let duplicate_source = !source_key.is_empty() && seen_sources.contains(&source_key);
        let duplicate_media = !media_keys.is_empty() && media_keys.is_subset(&seen_media);
        if duplicate_source || duplicate_media {
            omitted_duplicate_ids.push(item_id.clone());
            continue;
        }
        if !source_key.is_empty() {
            seen_sources.insert(source_key);
        }
        seen_media.extend(media_keys);
        if let Some((score, matched_by)) = candidates.get(item_id) {
            filtered_ranked.push((item_id.clone(), *score, matched_by.clone()));
        }
        if filtered_ranked.len() >= limit as usize {
            break;
        }
    }

    let records = context_records(conn, &filtered_ranked);
    let seed_ids_used: Vec<String> = records
        .iter()
        .filter(|r| r.get("matched_by").and_then(|v| v.as_array()).map(|a| a.iter().any(|m| m.as_str() == Some("seed"))).unwrap_or(false))
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .collect();
    let mut references: Vec<Value> = Vec::new();
    let mut reference_numbers: HashMap<String, usize> = HashMap::new();
    let records_mut: Vec<Value> = records
        .into_iter()
        .map(|mut record| {
            let mut numbered_citations: Vec<Value> = Vec::new();
            let citations = record.get("citations").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            for citation in &citations {
                let url = citation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if url.is_empty() {
                    continue;
                }
                let number = match reference_numbers.get(url) {
                    Some(n) => *n,
                    None => {
                        let number = references.len() + 1;
                        reference_numbers.insert(url.to_string(), number);
                        references.push(json!({
                            "number": number,
                            "marker": superscript_number(number),
                            "item_id": record.get("id").cloned().unwrap_or(json!("")),
                            "title": record.get("title").cloned().unwrap_or(json!("")),
                            "url": url,
                            "markdown": inline_citation(url, number),
                        }));
                        number
                    }
                };
                numbered_citations.push(json!({
                    "number": number,
                    "marker": superscript_number(number),
                    "url": url,
                    "markdown": inline_citation(url, number),
                }));
            }
            if let Some(obj) = record.as_object_mut() {
                obj.insert("citations".into(), json!(numbered_citations));
                let markdown = obj
                    .get("citations")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|c| c.get("markdown").and_then(|m| m.as_str())).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                obj.insert("citation_markdown".into(), json!(markdown));
            }
            record
        })
        .collect();
    let reference_markdown = references
        .iter()
        .filter_map(|r| r.get("markdown").and_then(|m| m.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    json!({
        "query": query,
        "seed_ids": requested_seed_ids,
        "seed_ids_used": seed_ids_used,
        "missing_seed_ids": missing_seed_ids,
        "omitted_duplicate_ids": omitted_duplicate_ids,
        "items": records_mut,
        "references": references,
        "reference_markdown": reference_markdown,
        "total": records_mut.len(),
    })
}

/// routes.vault_image_sources
pub fn vault_image_sources(conn: &Connection, item_ids: &[String], limit: i64) -> Value {
    let mut requested: Vec<String> = Vec::new();
    for raw_id in item_ids.iter().take(24) {
        let item_id = raw_id.trim().chars().take(64).collect::<String>();
        if !item_id.is_empty() && !requested.contains(&item_id) {
            requested.push(item_id.clone());
        }
    }
    let limit = limit.clamp(1, 24);
    if requested.is_empty() {
        return json!({"item_ids": [], "missing_item_ids": [], "images": []});
    }
    let placeholders = requested.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<Value> = requested.iter().map(|s| json!(s)).collect();
    let item_rows = vault::fetch_rows(
        conn,
        &format!("SELECT id, title, url FROM items WHERE id IN ({placeholders}) AND (deleted_at IS NULL OR deleted_at='')"),
        &params,
    );
    let items: HashMap<String, Map<String, Value>> = item_rows
        .into_iter()
        .filter_map(|r| {
            let id = r.get("id").and_then(|v| v.as_str())?.to_string();
            Some((id, r))
        })
        .collect();
    let missing: Vec<String> = requested
        .iter()
        .filter(|i| !items.contains_key(*i))
        .cloned()
        .collect();
    let media_rows = vault::fetch_rows(
        conn,
        &format!(
            "SELECT * FROM media WHERE item_id IN ({placeholders}) AND placeholder=0 AND kind IN ('image','video') ORDER BY item_id, rowid"
        ),
        &params,
    );
    let media_root = mediaio::media_dir();
    let mut images: Vec<Value> = Vec::new();
    let mut seen_media: HashSet<String> = HashSet::new();
    for row in &media_rows {
        if images.len() >= limit as usize {
            break;
        }
        let thumb = row.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
        let file = row.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let name = {
            let n = if thumb.is_empty() { file } else { thumb };
            std::path::Path::new(n)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        if name.is_empty() {
            continue;
        }
        let path = media_root.join(&name);
        if !path.starts_with(&media_root) || !path.exists() {
            continue;
        }
        let digest = {
            let s = row.get("sha256").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
            if s.is_empty() { path.to_string_lossy().to_string() } else { s }
        };
        if seen_media.contains(&digest) {
            continue;
        }
        seen_media.insert(digest);
        let Some(item) = row.get("item_id").and_then(|v| v.as_str()).and_then(|k| items.get(k)) else { continue };
        let parsed = crate::vault::media_to_dict_pub(row);
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        images.push(json!({
            "item_id": row.get("item_id").cloned().unwrap_or(json!("")),
            "media_id": row.get("id").cloned().unwrap_or(json!("")),
            "title": item.get("title").cloned().unwrap_or(json!("")),
            "source_url": item.get("url").cloned().unwrap_or(json!("")),
            "kind": row.get("kind").and_then(|v| v.as_str()).unwrap_or("image"),
            "mime": row.get("mime").and_then(|v| v.as_str()).unwrap_or(""),
            "thumb": if thumb.is_empty() { file } else { thumb },
            "width": row.get("width").cloned().unwrap_or(json!(0)),
            "height": row.get("height").cloned().unwrap_or(json!(0)),
            "palette": parsed.get("palette").and_then(|v| v.as_array()).map(|a| a.iter().take(6).cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "dominant": row.get("dominant").cloned().unwrap_or(json!("")),
            "hue": row.get("hue").cloned().unwrap_or(json!("")),
            "path": path.to_string_lossy(),
            "bytes": bytes,
        }));
    }
    json!({
        "item_ids": requested,
        "missing_item_ids": missing,
        "images": images,
    })
}
