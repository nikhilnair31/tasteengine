//! Background processing — port of pipeline.py: worker threads, staged file
//! import, enrich, media download, palette/phash/embed, transcript append,
//! refresh_item. OCR and the LLM explainer hook in via queue_explain.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::ingest;
use crate::mediaio;
use crate::vault;
use crate::write;
use crate::lock::LockExt;

pub struct Queue {
    items: Mutex<Vec<String>>,
    running: Mutex<HashSet<String>>,
}

impl Queue {
    fn new() -> Queue {
        Queue { items: Mutex::new(Vec::new()), running: Mutex::new(HashSet::new()) }
    }

    fn push(&self, item_id: &str) {
        self.items.lock_ok().push(item_id.to_string());
    }

    fn pop(&self) -> Option<String> {
        let mut items = self.items.lock_ok();
        if items.is_empty() {
            None
        } else {
            Some(items.remove(0))
        }
    }

    fn mark_running(&self, item_id: &str) -> bool {
        self.running.lock_ok().insert(item_id.to_string())
    }

    fn mark_done(&self, item_id: &str) {
        self.running.lock_ok().remove(item_id);
    }
}

static QUEUE: OnceLock<Queue> = std::sync::OnceLock::new();

fn queue() -> &'static Queue {
    QUEUE.get_or_init(Queue::new)
}

pub fn set_job(conn: &Connection, item_id: &str, state: &str, error: &str) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO jobs(item_id, state, error, updated_at) VALUES(?,?,?,datetime('now'))",
        rusqlite::params![item_id, state, error],
    );
}

/// pipeline.enqueue / enqueue_staged
pub fn enqueue(conn: &Connection, item_id: &str) {
    if !queue().mark_running(item_id) {
        return;
    }
    set_job(conn, item_id, "queued", "");
    queue().push(item_id);
}

pub fn enqueue_staged(conn: &Connection, item_id: &str) {
    enqueue(conn, item_id);
}

/// pipeline.start: recover stuck jobs, spawn two workers
pub fn start(db_path: String) {
    {
        let conn = Connection::open(&db_path).expect("open db for pipeline");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS jobs(item_id TEXT PRIMARY KEY, state TEXT, error TEXT, updated_at TEXT)",
            [],
        );
        let stuck = vault::fetch_rows(
            &conn,
            "SELECT item_id FROM jobs WHERE state IN ('queued','working')",
            &[],
        );
        for row in stuck {
            if let Some(id) = row.get("item_id").and_then(|v| v.as_str()) {
                if queue().mark_running(id) {
                    queue().push(id);
                }
            }
        }
    }
    for i in 0..2 {
        let db = db_path.clone();
        std::thread::Builder::new()
            .name(format!("pipeline-worker-{i}"))
            .spawn(move || worker_loop(db))
            .expect("spawn pipeline worker");
    }
}

fn worker_loop(db_path: String) {
    let conn = Connection::open(&db_path).expect("open db");
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    loop {
        let Some(item_id) = queue().pop() else {
            std::thread::sleep(std::time::Duration::from_millis(300));
            continue;
        };
        let _ = process_item(&conn, &item_id);
        queue().mark_done(&item_id);
    }
}

fn text_cap() -> usize {
    crate::write::env_num("TV_TEXT_CAP", 16000) as usize
}

fn max_images() -> i64 {
    8 // config.MAX_IMAGES_PER_ITEM
}

fn append_transcript(conn: &Connection, item_id: &str, transcript_text: &str) {
    if transcript_text.trim().is_empty() {
        return;
    }
    let row = vault::fetch_rows(conn, "SELECT text FROM items WHERE id=?", &[json!(item_id)])
        .into_iter()
        .next();
    let Some(row) = row else { return };
    let current = row.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let addition = transcript_text.trim().to_string();
    if current.contains(&addition) {
        return;
    }
    let combined = if current.is_empty() { addition } else { format!("{current}\n\n{addition}") };
    let combined: String = combined.chars().take(crate::write::env_num("TV_NOTE_MAX_TEXT", 100 * 1024) as usize).collect();
    let _ = conn.execute("UPDATE items SET text=? WHERE id=?", rusqlite::params![combined, item_id]);
}

/// parakeet transcription (transcribe.transcribe_file)
fn transcribe_file(path: &Path, mime: &str, name: &str) -> String {
    let base = std::env::var("PARAKEET_BASE_URL").unwrap_or_default();
    if base.is_empty() {
        return String::new();
    }
    let model = std::env::var("PARAKEET_MODEL").unwrap_or_else(|_| "parakeet-tdt-0.6b-v3".into());
    let timeout = crate::write::env_num("PARAKEET_TIMEOUT", 300) as u64;
    let content_type = if mime.is_empty() { "application/octet-stream" } else { mime };
    let file_bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    let boundary = crate::write::token_hex8();
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{model}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\njson\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nen\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",
            name.replace('"', "")
        )
        .as_bytes(),
    );
    body.extend_from_slice(&file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(timeout))
        .build();
    let resp = agent
        .post(&format!("{base}/audio/transcriptions"))
        .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send_bytes(&body);
    let Ok(resp) = resp else { return String::new() };
    let mut text = String::new();
    use std::io::Read;
    let _ = resp.into_reader().read_to_string(&mut text);
    let parsed: Result<Value, _> = serde_json::from_str(&text);
    let cap = crate::write::env_num("TV_TRANSCRIPT_MAX_CHARS", 100 * 1024) as usize;
    match parsed {
        Ok(v) => v
            .get("text")
            .or_else(|| v.get("transcript"))
            .and_then(|t| t.as_str())
            .unwrap_or(&text)
            .trim()
            .chars()
            .take(cap)
            .collect(),
        Err(_) => text.trim().chars().take(cap).collect(),
    }
}

fn transcribe_existing_audio(conn: &Connection, item_id: &str) {
    let rows = vault::fetch_rows(
        conn,
        "SELECT id, file, mime, original_name, transcript FROM media WHERE item_id=? AND kind='audio'",
        &[json!(item_id)],
    );
    for row in rows {
        let transcript = row.get("transcript").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let original_name = row.get("original_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !transcript.is_empty() {
            append_transcript(conn, item_id, &transcript);
            continue;
        }
        let file = row.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let path = mediaio::media_dir().join(file);
        if !path.exists() {
            continue;
        }
        let mime = row.get("mime").and_then(|v| v.as_str()).unwrap_or("");
        let text = transcribe_file(&path, mime, &original_name);
        if !text.is_empty() {
            let _ = conn.execute(
                "UPDATE media SET transcript=? WHERE id=?",
                rusqlite::params![text, row.get("id").and_then(|v| v.as_str()).unwrap_or("")],
            );
            append_transcript(conn, item_id, &text);
        }
    }
}

/// process one item end-to-end (pipeline._process)
pub fn process_item(conn: &Connection, item_id: &str) -> Result<(), String> {
    set_job(conn, item_id, "working", "");
    let result = process_inner(conn, item_id);
    match &result {
        Ok(()) => set_job(conn, item_id, "done", ""),
        Err(e) => set_job(conn, item_id, "error", &e[..e.len().min(200)]),
    }
    result
}

fn fetch_item(conn: &Connection, item_id: &str) -> Option<Value> {
    vault::fetch_rows(conn, "SELECT * FROM items WHERE id=?", &[json!(item_id)])
        .into_iter()
        .next()
        .map(Value::Object)
}

fn process_inner(conn: &Connection, item_id: &str) -> Result<(), String> {
    let mut item = fetch_item(conn, item_id).ok_or_else(|| "not found".to_string())?;

    let mut extra: Map<String, Value> = item
        .get("extra")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut image_urls: Vec<String> = extra
        .get("image_urls")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|u| u.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let staged: Vec<Value> = extra.get("_staged_files").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    // 0. staged files from the share sheet / note composer
    for entry in &staged {
        let (path, mime, original_name) = match entry {
            Value::String(p) => (p.clone(), String::new(), String::new()),
            Value::Object(_) => (
                entry.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                entry.get("mime").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                entry.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ),
            _ => (String::new(), String::new(), String::new()),
        };
        if path.is_empty() {
            continue;
        }
        if let Ok(data) = std::fs::read(&path) {
            if data.len() > 40 {
                let hint = mediaio::kind_for_upload(&mime, &original_name);
                if let Ok(saved) = mediaio::save_media(conn, item_id, &data, "", &mime, &hint, &original_name) {
                    if saved.get("kind").and_then(|v| v.as_str()) == Some("audio") {
                        let transcript = transcribe_file(
                            Path::new(&path),
                            saved.get("mime").and_then(|v| v.as_str()).unwrap_or(&mime),
                            &original_name,
                        );
                        if !transcript.is_empty() {
                            let _ = conn.execute(
                                "UPDATE media SET transcript=? WHERE id=?",
                                rusqlite::params![transcript, saved.get("id").and_then(|v| v.as_str()).unwrap_or("")],
                            );
                            append_transcript(conn, item_id, &transcript);
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    if !staged.is_empty() {
        extra.remove("_staged_files");
        let _ = conn.execute(
            "UPDATE items SET extra=? WHERE id=?",
            rusqlite::params![Value::Object(extra.clone()).to_string(), item_id],
        );
        item = fetch_item(conn, item_id).ok_or("not found")?;
    }
    transcribe_existing_audio(conn, item_id);
    item = fetch_item(conn, item_id).ok_or("not found")?;

    let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let is_note = extra.get("note").and_then(|v| v.as_bool()).unwrap_or(false);

    // 1. enrich a bare URL when no media candidates exist
    if !url.is_empty() && image_urls.is_empty() && !is_note {
        let lower = url.to_lowercase();
        let path_part = url::Url::parse(&url).map(|p| p.path().to_lowercase()).unwrap_or_default();
        if [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"].iter().any(|e| path_part.ends_with(e)) {
            image_urls = vec![url.clone()];
        } else if [".mp4", ".webm", ".mov"].iter().any(|e| path_part.ends_with(e)) || lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mov") {
            extra.insert("video_url".into(), json!(url));
        } else {
            let meta = ingest::enrich(&url);
            let mut updates: Vec<&str> = Vec::new();
            let mut params: Vec<Value> = Vec::new();
            let plat = meta.get("platform").and_then(|v| v.as_str()).unwrap_or("");
            let fresh_title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let fresh_text = meta.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let cur_text = item.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let cur_title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let take_text = !fresh_text.is_empty()
                && (cur_text.is_empty()
                    || (plat == "reddit" && !cur_text.chars().take(400).collect::<String>().is_empty()
                        && cur_text.chars().take(400).collect::<String>() == fresh_text.chars().take(400).collect::<String>()));
            let placeholder_title = !cur_title.is_empty()
                && vault::normalize_text_pub(cur_title).len() > 0
                && crate::maintain::is_placeholder_title_pub(cur_title);
            if !fresh_title.is_empty() && (cur_title.is_empty() || (plat == "reddit" && placeholder_title)) {
                updates.push("title=?");
                params.push(json!(fresh_title.chars().take(400).collect::<String>()));
            }
            if take_text {
                updates.push("text=?");
                params.push(json!(fresh_text.chars().take(text_cap()).collect::<String>()));
            }
            let fresh_author = meta.get("author").and_then(|v| v.as_str()).unwrap_or("");
            if !fresh_author.is_empty() && item.get("author").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                updates.push("author=?");
                params.push(json!(fresh_author.chars().take(200).collect::<String>()));
            }
            if !plat.is_empty() {
                updates.push("platform=?");
                params.push(json!(plat));
            }
            let fresh_created = meta.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
            if !fresh_created.is_empty() {
                updates.push("created_at=?");
                params.push(json!(fresh_created.chars().take(40).collect::<String>()));
            }
            if meta.get("private").and_then(|v| v.as_bool()).unwrap_or(false) {
                updates = vec!["title=?", "text=?", "kind=?"];
                params = vec![json!("Instagram post"), json!("Account private"), json!("text")];
            }
            if !updates.is_empty() {
                params.push(json!(item_id));
                let sql = format!("UPDATE items SET {} WHERE id=?", updates.join(", "));
                let _ = conn.execute(&sql, rusqlite::params_from_iter(params.iter().map(vault::to_rusqlite_value)));
                item = fetch_item(conn, item_id).ok_or("not found")?;
            }
            let resolved = meta.get("resolved_url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if !resolved.is_empty() && resolved != url.trim() {
                let key = vault::canonical_source_key(&resolved);
                let _ = conn.execute(
                    "UPDATE items SET url=?, dedupe_key=? WHERE id=?",
                    rusqlite::params![resolved.chars().take(2000).collect::<String>(), key, item_id],
                );
                item = fetch_item(conn, item_id).ok_or("not found")?;
            }
            if let Some(imgs) = meta.get("images").and_then(|v| v.as_array()) {
                image_urls = imgs.iter().filter_map(|u| u.as_str().map(|s| s.to_string())).collect();
            } else {
                // extension-less CDN URL: download and sniff the bytes
                if let Ok((probe, _ctype)) = mediaio::download_bytes(&url) {
                    if probe.len() > 200 {
                        if mediaio::sniff_image(&probe) {
                            image_urls = vec![url.clone()];
                        } else if probe.len() > 8
                            && (&probe[4..8] == b"ftyp" || &probe[4..8] == b"moov" || &probe[4..8] == b"mdat")
                        {
                            extra.insert("video_url".into(), json!(url));
                        }
                    }
                }
            }
            if let Some(ol) = meta.get("outlinks").and_then(|v| v.as_array()) {
                if !ol.is_empty() {
                    extra.insert("outlinks".into(), Value::Array(ol.clone()));
                    let _ = conn.execute(
                        "UPDATE items SET extra=? WHERE id=?",
                        rusqlite::params![Value::Object(extra.clone()).to_string(), item_id],
                    );
                    item = fetch_item(conn, item_id).ok_or("not found")?;
                }
            }
            let text_links = ingest::outlinks_from_text(
                item.get("text").and_then(|v| v.as_str()).unwrap_or(""),
                item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                &url,
            );
            if !text_links.is_empty() {
                let mut cur: Vec<Value> = extra.get("outlinks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let seen: Vec<String> = cur
                    .iter()
                    .filter_map(|o| o.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()))
                    .collect();
                for o in text_links {
                    let o_url = o.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    if !seen.contains(&o_url.to_string()) {
                        cur.push(o);
                    }
                }
                extra.insert("outlinks".into(), Value::Array(cur));
                let _ = conn.execute(
                    "UPDATE items SET extra=? WHERE id=?",
                    rusqlite::params![Value::Object(extra.clone()).to_string(), item_id],
                );
            }
            if let Some(music) = meta.get("ig_music") {
                if music.is_object() && !music.as_object().unwrap().is_empty() {
                    extra.insert("ig_music".into(), music.clone());
                    let _ = conn.execute(
                        "UPDATE items SET extra=? WHERE id=?",
                        rusqlite::params![Value::Object(extra.clone()).to_string(), item_id],
                    );
                    item = fetch_item(conn, item_id).ok_or("not found")?;
                }
            }
        }
    }

    // 2. download media
    let mut total_media = vault::fetch_scalar_val(
        conn,
        "SELECT COUNT(*) c FROM media WHERE item_id=?",
        &[json!(item_id)],
    );
    let mut seen_urls: Vec<String> = Vec::new();
    for u in &image_urls {
        if !seen_urls.contains(u) {
            seen_urls.push(u.clone());
        }
    }
    image_urls = seen_urls;
    let url_lower = url.to_lowercase();
    if !url.is_empty()
        && item.get("kind").and_then(|v| v.as_str()) == Some("video")
        && url_lower.contains("x.com/")
    {
        let generated = ingest::linkthumb_image(&url);
        if !generated.is_empty() {
            if let Ok((data, _)) = mediaio::download_bytes(&generated) {
                if data.len() > 200 && total_media > 0 {
                    let first = vault::fetch_rows(
                        conn,
                        "SELECT id FROM media WHERE item_id=? ORDER BY rowid LIMIT 1",
                        &[json!(item_id)],
                    )
                    .into_iter()
                    .next();
                    if let Some(first) = first {
                        let _ = mediaio::replace_preview(
                            conn,
                            first.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            &data,
                        );
                        image_urls.clear();
                    }
                } else if data.len() > 200 {
                    image_urls = vec![generated.clone()];
                }
            }
        }
    }
    let mut deduped: Vec<String> = Vec::new();
    for u in &image_urls {
        if !u.is_empty() && !deduped.contains(u) {
            deduped.push(u.clone());
        }
    }
    image_urls = deduped.into_iter().take(max_images() as usize).collect();
    for u in &image_urls {
        if total_media >= max_images() {
            break;
        }
        match mediaio::download_bytes(u) {
            Ok((data, _ctype)) => {
                if data.len() < 200 {
                    continue;
                }
                if mediaio::save_media(conn, item_id, &data, u, "", "", "").is_ok() {
                    total_media += 1;
                }
            }
            Err(e) => eprintln!("[pipeline-rs] download failed: {u} -> {e}"),
        }
    }
    let video_url = extra.get("video_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if !video_url.is_empty() && total_media == 0 {
        if let Ok((data, _)) = mediaio::download_bytes(&video_url) {
            if data.len() > 200 {
                let _ = mediaio::save_media(conn, item_id, &data, &video_url, "", "", "");
            }
        }
    }

    // 2b. instagram: attach the real MP4 + carousel slides
    if url_lower.contains("instagram.com/") {
        total_media = attach_ig_media(conn, item_id, &url, total_media);
    }

    // 3. embed media
    let pending = vault::fetch_rows(
        conn,
        "SELECT * FROM media WHERE item_id=? AND embed IS NULL ORDER BY rowid",
        &[json!(item_id)],
    );
    for r in pending {
        let file = r.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let path = mediaio::media_dir().join(file);
        if !path.exists() {
            eprintln!("[pipeline-rs] media file missing: {}", path.display());
            continue;
        }
        let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let mid = r.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let vec = match kind {
            "video" => {
                let frames = mediaio::video_extract_frames(&path, 4);
                if frames.is_empty() {
                    None
                } else {
                    let vecs: Vec<Option<Vec<f32>>> = frames.iter().map(|f| crate::embed::embed_image_bytes(f)).collect();
                    let good: Vec<Vec<f32>> = vecs.into_iter().flatten().collect();
                    match good.len() {
                        0 => None,
                        1 => crate::search::normalize_pub(&good[0]),
                        _ => {
                            let mut sum = good[0].clone();
                            for g in &good[1..] {
                                for (a, b) in sum.iter_mut().zip(g.iter()) {
                                    *a += b;
                                }
                            }
                            crate::search::normalize_pub(&sum)
                        }
                    }
                }
            }
            "image" => {
                let bytes = std::fs::read(&path).unwrap_or_default();
                crate::embed::embed_image_bytes(&bytes).and_then(|v| crate::search::normalize_pub(&v))
            }
            _ => None,
        };
        if let Some(v) = vec {
            let blob: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
            let _ = conn.execute("UPDATE media SET embed=? WHERE id=?", rusqlite::params![blob, mid]);
        }
    }
    refresh_item_embed(conn, item_id);

    // 3b. OCR video rows missing ocr_text (tesseract port of ocr.video_text)
    let videos = vault::fetch_rows(
        conn,
        "SELECT id, file FROM media WHERE item_id=? AND kind='video' AND (ocr_text IS NULL OR ocr_text='')",
        &[json!(item_id)],
    );
    for r in videos {
        let file = r.get("file").and_then(|v| v.as_str()).unwrap_or("");
        if file.is_empty() {
            continue;
        }
        let path = mediaio::media_dir().join(file);
        let text = mediaio::video_ocr(&path, 6);
        if !text.is_empty() {
            let _ = conn.execute(
                "UPDATE media SET ocr_text=? WHERE id=?",
                rusqlite::params![text, r.get("id").and_then(|v| v.as_str()).unwrap_or("")],
            );
        }
    }

    // 4. explain when the item is worth a model call (notes always)
    let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let text_len = (text.len() + title.len()) as i64;
    let no_explain = extra.get("no_explain").and_then(|v| v.as_bool()).unwrap_or(false) && !is_note;
    let has_media = vault::fetch_scalar_val(conn, "SELECT COUNT(*) FROM media WHERE item_id=? LIMIT 1", &[json!(item_id)]) > 0;
    if !no_explain && (is_note || has_media || text_len > 40) {
        write::queue_explain(conn, item_id, false);
    }
    Ok(())
}

/// search.refresh_item_embed: recompute the item vector and persist it.
/// The vector mixes pictures and words (`cluster::fused_vec`), so an item's
/// caption joins the same space as its image.
pub fn refresh_item_embed(conn: &Connection, item_id: &str) {
    if let Some(vec) = crate::cluster::fused_vec(conn, item_id) {
        let blob: Vec<u8> = vec.iter().flat_map(|f| f.to_le_bytes()).collect();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO item_embeds(item_id, vec) VALUES(?,?)",
            rusqlite::params![item_id, blob],
        );
    }
}

/// pipeline._attach_ig_media
fn attach_ig_media(conn: &Connection, item_id: &str, url: &str, mut total_media: i64) -> i64 {
    if url.is_empty() || !url.to_lowercase().contains("instagram.com/") {
        return total_media;
    }
    let has_video = vault::fetch_scalar_val(
        conn,
        "SELECT COUNT(*) FROM media WHERE item_id=? AND kind='video' LIMIT 1",
        &[json!(item_id)],
    ) > 0;
    if !has_video {
        let vid = ingest::ig_video_download(url);
        if !vid.is_empty() {
            if mediaio::save_media(conn, item_id, &vid, url, "", "", "").is_ok() {
                total_media += 1;
            }
        }
    }
    let fingerprint = |mid: &str| -> (String, Option<u64>) {
        let row = vault::fetch_rows(conn, "SELECT file, thumb FROM media WHERE id=?", &[json!(mid)])
            .into_iter()
            .next();
        let Some(row) = row else { return (String::new(), None) };
        let thumb = row.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
        let file = row.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let path = mediaio::media_dir().join(if thumb.is_empty() { file } else { thumb });
        match vault::file_fingerprint(&path) {
            Some(fp) => fp,
            None => (String::new(), None),
        }
    };
    let mut existing_fps: Vec<(String, Option<u64>)> = vault::fetch_rows(
        conn,
        "SELECT id FROM media WHERE item_id=?",
        &[json!(item_id)],
    )
    .iter()
    .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
    .map(|id| fingerprint(&id))
    .collect();
    for slide_url in ingest::ig_embed_slides(url) {
        if total_media >= max_images() {
            break;
        }
        let data = match mediaio::download_bytes(&slide_url) {
            Ok((d, _)) => d,
            Err(_) => continue,
        };
        if data.len() < 200 {
            continue;
        }
        let row = match mediaio::save_media(conn, item_id, &data, &slide_url, "", "", "") {
            Ok(r) => r,
            Err(_) => continue,
        };
        total_media += 1;
        let row_id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (exact, phash) = fingerprint(&row_id);
        let mut dup = false;
        for (e_exact, e_phash) in &existing_fps {
            let same = (!exact.is_empty() && &exact == e_exact)
                || (phash.is_some() && e_phash.is_some() && {
                    let (a, b) = (phash.unwrap(), e_phash.unwrap());
                    (a ^ b).count_ones() <= 4
                });
            if same {
                dup = true;
                break;
            }
        }
        if dup {
            for name in [row.get("file").and_then(|v| v.as_str()).unwrap_or("").to_string()] {
                if !name.is_empty() {
                    let _ = std::fs::remove_file(mediaio::media_dir().join(&name));
                }
            }
            if let Some(thumb) = row.get("thumb").and_then(|v| v.as_str()) {
                if !thumb.is_empty() {
                    let _ = std::fs::remove_file(mediaio::media_dir().join(thumb));
                }
            }
            let _ = conn.execute("DELETE FROM media WHERE id=?", [&row_id]);
            total_media -= 1;
        } else {
            existing_fps.push((exact, phash));
        }
    }
    total_media
}

use std::sync::OnceLock;
