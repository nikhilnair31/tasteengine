//! Verbose item explainer via the OpenRouter vision model — port of
//! explain.py: single background worker, media gather (frames, thumbs,
//! transcripts, attachments), the exact prompt contract, tag post-processing,
//! follow-up extraction with vague-place filtering and link resolution.

use std::collections::HashSet;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::mediaio;
use crate::resolve;
use crate::vault;
use crate::write;
use crate::lock::LockExt;

static QUEUE: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static PROMPT: OnceLock<String> = OnceLock::new();

fn audio_generic_tags() -> Vec<&'static str> {
    vec![
        "audio", "audio attachment", "attachment", "note", "notes", "recording",
        "voice", "voice note", "transcript", "transcription", "test", "testing",
        "webm", "spoken word",
    ]
}

fn prompts_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("TV_PROMPTS_DIR") {
        if !dir.is_empty() {
            return std::path::PathBuf::from(dir);
        }
    }
    std::path::Path::new(&prompt_base()).join("app")
}

/// Insertion point for the optional, user-edited taste profile.
const PROFILE_MARKER: &str = "{{TASTE_PROFILE}}";

/// The explainer prompt with the user's own taste profile spliced in.
///
/// `taste_profile.txt` is optional and user-owned: the shipped copy holds only
/// comments. Without it the prompt tells the model to infer taste from the
/// saved item alone, so a fresh install never inherits someone else's profile.
fn prompt() -> String {
    PROMPT
        .get_or_init(|| {
            let dir = prompts_dir();
            let base = std::fs::read_to_string(dir.join("explainer_prompt.txt")).unwrap_or_default();
            let profile = std::fs::read_to_string(dir.join("taste_profile.txt"))
                .map(|raw| {
                    raw.lines()
                        .filter(|line| !line.trim_start().starts_with('#'))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_string()
                })
                .unwrap_or_default();
            let has_profile = !profile.is_empty();
            let block = if has_profile {
                profile.clone()
            } else {
                "No taste profile is configured. Infer taste only from the saved item, its metadata and its images. Never invent personal facts about the user.".to_string()
            };
            if base.contains(PROFILE_MARKER) {
                base.replace(PROFILE_MARKER, &block)
            } else if has_profile {
                format!("{base}\n\n{profile}")
            } else {
                base
            }
        })
        .clone()
}

fn prompt_base() -> String {
    std::env::var("TV_BASE").unwrap_or_else(|_| {
        format!("{}/Projects/taste-engine", std::env::var("HOME").unwrap_or_default())
    })
}



fn explainer_model() -> String {
    std::env::var("EXPLAINER_MODEL").unwrap_or_else(|_| "google/gemini-2.5-flash".into())
}

fn explain_max_images() -> usize {
    8 // config.EXPLAIN_MAX_IMAGES
}

/// explain.queue_explain
pub fn queue_explain(conn: &Connection, item_id: &str, force: bool) {
    let mut pending = PENDING.get_or_init(|| Mutex::new(HashSet::new())).lock_ok();
    if pending.contains(item_id) {
        return;
    }
    if !force {
        let status: Option<String> = conn
            .query_row("SELECT status FROM explanations WHERE item_id=?", [item_id], |r| r.get(0))
            .ok();
        if status.as_deref() == Some("done") {
            return;
        }
    }
    pending.insert(item_id.to_string());
    let _ = conn.execute(
        "INSERT OR REPLACE INTO explanations(item_id, status, data, updated_at) VALUES(?,?,?,datetime('now'))",
        rusqlite::params![item_id, "pending", "{}"],
    );
    QUEUE.get_or_init(|| Mutex::new(Vec::new())).lock_ok().push(item_id.to_string());
}

/// explain.backfill_explainers
pub fn backfill_explainers(conn: &Connection, limit: i64, force: bool) -> i64 {
    let where_clause = if force {
        "status IS NULL".to_string()
    } else {
        "status IS NULL OR status='failed' OR status='pending'".to_string()
    };
    let mut queued = 0i64;
    let rows = vault::fetch_rows(
        conn,
        &format!("SELECT item_id FROM explanations WHERE {where_clause} LIMIT {limit}"),
        &[],
    );
    for r in rows {
        if let Some(id) = r.get("item_id").and_then(|v| v.as_str()) {
            queue_explain(conn, id, force);
            queued += 1;
        }
    }
    let no_row = vault::fetch_rows(
        conn,
        &format!(
            "SELECT i.id FROM items i LEFT JOIN explanations e ON e.item_id=i.id \
             WHERE e.item_id IS NULL AND (EXISTS(SELECT 1 FROM media m WHERE m.item_id=i.id) \
             OR length(coalesce(i.text,''))+length(coalesce(i.title,''))>40) LIMIT {limit}"
        ),
        &[],
    );
    for r in no_row {
        if let Some(id) = r.get("id").and_then(|v| v.as_str()) {
            queue_explain(conn, id, false);
            queued += 1;
        }
    }
    queued
}

/// explain._worker: EXPLAIN_WORKERS threads started at boot
pub fn start(db_path: String) {
    let workers = crate::write::env_num("EXPLAIN_WORKERS", 1).max(1);
    for i in 0..workers {
        let db = db_path.clone();
        std::thread::Builder::new()
            .name(format!("explain-worker-{i}"))
            .spawn(move || {
                let conn = match Connection::open(&db) {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let _ = conn.pragma_update(None, "foreign_keys", "ON");
                loop {
                    let Some(item_id) = QUEUE
                        .get_or_init(|| Mutex::new(Vec::new()))
                        .lock()
                        .unwrap()
                        .pop()
                    else {
                        std::thread::sleep(Duration::from_millis(300));
                        continue;
                    };
                    let _ = run_explain(&conn, &item_id);
                    PENDING
                        .get_or_init(|| Mutex::new(HashSet::new()))
                        .lock()
                        .unwrap()
                        .remove(&item_id);
                }
            })
            .expect("spawn explain worker");
    }
}

/// explain._downscale_to_b64: JPEG base64, longest edge 768, quality 82
pub fn downscale_to_b64(data: &[u8]) -> String {
    downscale_to_b64_opts(data, 768, 82)
}

/// Parameterized variant (inspire uses 640/80 for reference thumbs).
pub fn downscale_to_b64_opts(data: &[u8], max_edge: u32, quality: u8) -> String {
    let Ok(img) = image::load_from_memory(data) else { return String::new() };
    let img = image::DynamicImage::ImageRgb8(img.to_rgb8());
    let (w, h) = (img.width(), img.height());
    let rgb0 = img.to_rgb8();
    let img = if w.max(h) > max_edge {
        let ratio = max_edge as f64 / w.max(h) as f64;
        let nw = ((w as f64 * ratio) as u32).max(1);
        let nh = ((h as f64 * ratio) as u32).max(1);
        image::DynamicImage::ImageRgb8(image::imageops::resize(
            &rgb0, nw, nh, image::imageops::FilterType::Triangle,
        ))
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut out: Vec<u8> = Vec::new();
    {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        if enc.encode(&rgb, rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).is_err() {
            return String::new();
        }
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(&out)
}

/// explain._read_text_attachment: docx zip text or plain text files
fn read_text_attachment(path: &std::path::Path, mime: &str, name: &str) -> String {
    let suffix = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let name_has_ext = name.contains('.');
    let suffix = if name_has_ext { suffix } else { String::new() };
    let max_bytes = crate::write::env_num("TV_AI_FILE_MAX_BYTES", 32 * 1024 * 1024) as usize;
    let docx_mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    if suffix == "docx" || mime == docx_mime {
        if let Ok(file) = std::fs::File::open(path) {
            if let Ok(mut zip) = zip::ZipArchive::new(file) {
                if let Ok(mut xml) = zip.by_name("word/document.xml") {
                    let mut s = String::new();
                    let _ = xml.read_to_string(&mut s);
                    let re = regex::Regex::new(r"(?s)<w:t[^>]*>(.*?)</w:t>").unwrap();
                    let joined: String = re.captures_iter(&s).map(|c| c.get(1).unwrap().as_str()).collect();
                    return crate::vault::unescape_pub(&joined).chars().take(120000).collect();
                }
            }
        }
        return String::new();
    }
    let text_mime = mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/csv";
    let text_ext = matches!(suffix.as_str(), "txt" | "md" | "csv" | "json" | "xml" | "html" | "htm" | "rtf" | "log");
    if !text_mime && !text_ext {
        return String::new();
    }
    match std::fs::read(path) {
        Ok(data) => {
            let data = &data[..data.len().min(max_bytes)];
            String::from_utf8_lossy(data).chars().take(120000).collect()
        }
        Err(_) => String::new(),
    }
}

/// explain._file_part: base64 file content part for pdf/docx/xlsx/pptx
fn file_part(path: &std::path::Path, mime: &str, name: &str) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() as i64 > crate::write::env_num("TV_AI_FILE_MAX_BYTES", 32 * 1024 * 1024) {
        return None;
    }
    let suffix = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let supported = matches!(suffix.as_str(), "pdf" | "docx" | "xlsx" | "pptx")
        || matches!(mime, "application/pdf" | "application/vnd.openxmlformats-officedocument.wordprocessingml.document" | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" | "application/vnd.openxmlformats-officedocument.presentationml.presentation");
    if !supported {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    let file_mime = if mime.is_empty() {
        match suffix.as_str() {
            "pdf" => "application/pdf",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            _ => "application/octet-stream",
        }
    } else {
        mime
    };
    Some(json!({
        "type": "file",
        "file": {
            "filename": name,
            "file_data": format!("data:{file_mime};base64,{encoded}"),
        },
    }))
}

/// explain._media_for_explain
fn media_for_explain(conn: &Connection, item_id: &str) -> Value {
    let mut media = vault::fetch_rows(conn, "SELECT * FROM media WHERE item_id=? ORDER BY rowid", &[json!(item_id)]);
    // videos first so frames are not starved by a large carousel
    media.sort_by_key(|m| if m.get("kind").and_then(|v| v.as_str()) == Some("video") { 0 } else { 1 });
    let mut images: Vec<Value> = Vec::new();
    let mut files: Vec<Value> = Vec::new();
    let mut file_texts: Vec<Value> = Vec::new();
    let mut transcripts: Vec<String> = Vec::new();
    let image_count = media.iter().filter(|m| m.get("kind").and_then(|v| v.as_str()) == Some("image")).count();
    let mut note = "images";
    let mut frame_note = String::new();
    if image_count > 1 {
        note = "SEPARATE PICTURES of this post (a gallery/carousel — describe them as one post covering all pictures)";
    }
    for m in &media {
        let name = {
            let a = m.get("original_name").and_then(|v| v.as_str()).unwrap_or("");
            let b = m.get("file").and_then(|v| v.as_str()).unwrap_or("");
            if !a.is_empty() { a.to_string() } else if !b.is_empty() { b.to_string() } else { "attachment".to_string() }
        };
        let mime = {
            let a = m.get("mime").and_then(|v| v.as_str()).unwrap_or("");
            if a.is_empty() { "application/octet-stream" } else { a }
        };
        let kind = m.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        if images.len() >= explain_max_images() && (kind == "image" || kind == "video") {
            continue;
        }
        let transcript = m.get("transcript").and_then(|v| v.as_str()).unwrap_or("");
        if (kind == "audio" || kind == "video") && !transcript.is_empty() {
            transcripts.push(format!("{name}:\n{transcript}"));
            if kind == "audio" {
                continue;
            }
        }
        if kind == "video" {
            let file = m.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let path = mediaio::media_dir().join(file);
            let frames = mediaio::video_extract_frames(&path, explain_max_images());
            if !frames.is_empty() {
                let first_index = images.len();
                let remaining = explain_max_images().saturating_sub(images.len());
                for f in frames.iter().take(remaining) {
                    let b64 = downscale_to_b64(f);
                    if !b64.is_empty() {
                        images.push(json!(b64));
                    }
                }
                let got = images.len() - first_index;
                if got > 0 {
                    frame_note = format!(
                        "FRAMES FROM THIS POST'S VIDEO, in order (frame 1 is the start, frame {got} is near the end): walk through what happens from the first frame to the last in the explainer summary"
                    );
                }
            }
            continue;
        }
        if kind == "image" {
            let file = m.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
            let file = if file.is_empty() { m.get("file").and_then(|v| v.as_str()).unwrap_or("") } else { file };
            match std::fs::read(mediaio::media_dir().join(file)) {
                Ok(bytes) => {
                    let b64 = downscale_to_b64(&bytes);
                    if !b64.is_empty() {
                        images.push(json!(b64));
                    }
                }
                Err(_) => {}
            }
            continue;
        }
        if kind == "file" {
            let file = m.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let path = mediaio::media_dir().join(file);
            let text = read_text_attachment(&path, mime, &name);
            if !text.is_empty() {
                file_texts.push(json!(format!("{name}:\n{text}")));
            }
            let part = file_part(&path, mime, &name);
            if text.is_empty() {
                if let Some(part) = part {
                    files.push(part);
                } else {
                    file_texts.push(json!(format!("{name} ({mime}) could not be parsed by the worker.")));
                }
            }
        }
    }
    let ocr_texts: Vec<Value> = vault::fetch_rows(
        conn,
        "SELECT ocr_text FROM media WHERE item_id=? AND kind='video' AND ocr_text!='' ORDER BY rowid",
        &[json!(item_id)],
    )
    .iter()
    .filter_map(|r| r.get("ocr_text").cloned())
    .collect();
    json!({
        "images": images,
        "files": files,
        "file_texts": file_texts,
        "transcripts": transcripts,
        "note": note,
        "frame_note": frame_note,
        "ocr_texts": ocr_texts,
    })
}

/// explain._build_messages
fn build_messages(item: &Value, file_texts: &[Value], transcripts: &[String], ocr_texts: &[Value]) -> Vec<Value> {
    let get = |k: &str| item.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let mut ctx_lines: Vec<String> = Vec::new();
    ctx_lines.push(format!("Title: {}", get("title")));
    ctx_lines.push(format!("Kind: {}", get("kind")));
    ctx_lines.push(format!(
        "Source: {}",
        if get("platform").is_empty() { get("source") } else { get("platform") }
    ));
    ctx_lines.push(format!("URL: {}", get("url")));
    if !get("author").is_empty() {
        ctx_lines.push(format!("Author: {}", get("author")));
    }
    let text = get("text").trim();
    if !text.is_empty() {
        let reddit_cap = crate::write::env_num("TV_REDDIT_EXPLAIN_TEXT_CAP", 12000) as usize;
        let cap = if get("platform") == "reddit" { reddit_cap } else { 1600 };
        let text: String = text.chars().take(cap).collect();
        let text = if get("text").chars().count() > cap { format!("{text} [truncated]") } else { text };
        ctx_lines.push(format!("Post text:\n{text}"));
    }
    let note = get("note").trim();
    if !note.is_empty() && note != text {
        ctx_lines.push(format!("Saved note:\n{}", note.chars().take(1600).collect::<String>()));
    }
    if !transcripts.is_empty() {
        let joined = transcripts.join("\n\n");
        ctx_lines.push(format!("Audio transcripts:\n{}", joined.chars().take(120000).collect::<String>()));
    }
    if !file_texts.is_empty() {
        let joined: Vec<String> = file_texts.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
        ctx_lines.push(format!("Readable attached files:\n{}", joined.join("\n\n").chars().take(120000).collect::<String>()));
    }
    let extra: Map<String, Value> = get("extra")
        .is_empty()
        .then(|| Map::new())
        .unwrap_or_else(|| {
            serde_json::from_str::<Value>(get("extra"))
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default()
        });
    let outlinks: Vec<&Value> = extra
        .get("outlinks")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|o| o.is_object()).take(12).collect())
        .unwrap_or_default();
    if !outlinks.is_empty() {
        let mut lines = Vec::new();
        for o in outlinks {
            let label = o.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
            let url = o.get("url").and_then(|v| v.as_str()).unwrap_or("");
            lines.push(format!(
                "{} <{}>",
                if label.is_empty() { url } else { label },
                url
            ));
        }
        ctx_lines.push(format!("Outlinks in post:\n{}", lines.join("\n")));
    }
    let music = extra.get("ig_music").cloned().unwrap_or(json!({}));
    let song = music.get("song").and_then(|v| v.as_str()).unwrap_or("");
    let artist = music.get("artist").and_then(|v| v.as_str()).unwrap_or("");
    if !music.is_null() && (!song.is_empty() || !artist.is_empty()) {
        ctx_lines.push(format!(
            "Tagged audio on this reel: {} — {}",
            if song.is_empty() { "?" } else { song },
            if artist.is_empty() { "?" } else { artist }
        ));
    }
    if !ocr_texts.is_empty() {
        let joined: Vec<String> = ocr_texts.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
        ctx_lines.push(format!("Text found in video frames (OCR):\n{}", joined.join("\n\n").chars().take(6000).collect::<String>()));
    }
    vec![json!({"type": "text", "text": ctx_lines.join("\n")})]
}

/// explain._call_model
fn call_model(
    item: &Value,
    explain_images: &[Value],
    note: &str,
    explain_files: &[Value],
    file_texts: &[Value],
    transcripts: &[String],
    ocr_texts: &[Value],
    frame_note: &str,
) -> Result<Value, String> {
    let mut content = build_messages(item, file_texts, transcripts, ocr_texts);
    for b64 in explain_images {
        let b64_str = b64.as_str().unwrap_or_default();
        content.push(json!({
            "type": "image_url",
            "image_url": {"url": format!("data:image/jpeg;base64,{b64_str}")},
        }));
    }
    content.extend(explain_files.iter().cloned());
    content.push(json!({
        "type": "text",
        "text": format!(
            "Media note: the attached images are {} {note} from this post. There are {} attached PDF files. There are {} audio transcripts. Return the JSON explainer now.",
            explain_images.len(), explain_files.len(), transcripts.len()
        ),
    }));
    if !frame_note.is_empty() {
        content.push(json!({"type": "text", "text": format!("Media note: {frame_note}")}));
    }
    let payload = json!({
        "model": explainer_model(),
        "messages": [
            {"role": "system", "content": prompt()},
            {"role": "user", "content": content},
        ],
        "temperature": 0.4,
        "max_tokens": 1200,
    });
    let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build();
    let resp = match agent
        .post("https://openrouter.ai/api/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_string(&payload.to_string())
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let mut body = String::new();
            let _ = r.into_reader().read_to_string(&mut body);
            return Err(format!("openrouter {code}: {}", body.chars().take(400).collect::<String>()));
        }
        Err(e) => return Err(format!("openrouter: {e}")),
    };
    let mut body = String::new();
    resp.into_reader().read_to_string(&mut body).map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&body).map_err(|e| format!("openrouter json: {e}"))?;
    let raw = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "no content".to_string())?
        .trim()
        .to_string();
    let raw = if raw.starts_with("```") {
        raw.splitn(2, '\n').nth(1).unwrap_or("").rsplit_once("```").map(|(a, _)| a.trim()).unwrap_or("").to_string()
    } else {
        raw
    };
    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
        return Ok(v);
    }
    let a = raw.find('{');
    let b = raw.rfind('}');
    if let (Some(a), Some(b)) = (a, b) {
        if b > a {
            if let Ok(v) = serde_json::from_str::<Value>(&raw[a..=b]) {
                return Ok(v);
            }
        }
    }
    Err("model did not return JSON".into())
}

// ---------- vague-place filtering (explain.py regexes) ----------

fn vague_place_re() -> regex::Regex {
    regex::Regex::new(
        r#"(?i)\b(pacific northwest|pnw|northwest|northeast|north america|the mountains|the coast|the coastline|the beach|the shore|somewhere|scenic|wilderness|countryside|the woods|a lake|the lake|the park|a park|hiking near|trail near|waterfall den|best hiking|favorite hiking)\b"#,
    )
    .unwrap()
}

fn generic_place_only_re() -> regex::Regex {
    regex::Regex::new(
        r#"(?i)^\s*(hiking|trails?|waterfalls?|mountains?|mountains|park|parks|beach|coast|coastline|ocean|lake|lakes|nature|woods|wilderness|scenery|landscape|region)\s*$"#,
    )
    .unwrap()
}

const US_STATES: &[&str] = &[
    "alabama", "alaska", "arizona", "arkansas", "california", "colorado", "connecticut",
    "delaware", "florida", "georgia", "hawaii", "idaho", "illinois", "indiana", "iowa",
    "kansas", "kentucky", "louisiana", "maine", "maryland", "massachusetts", "michigan",
    "minnesota", "mississippi", "missouri", "montana", "nebraska", "nevada",
    "new hampshire", "new jersey", "new mexico", "new york", "north carolina",
    "north dakota", "ohio", "oklahoma", "oregon", "pennsylvania", "rhode island",
    "south carolina", "south dakota", "tennessee", "texas", "utah", "vermont",
    "virginia", "washington", "west virginia", "wisconsin", "wyoming",
];

fn state_place_re() -> regex::Regex {
    let states = US_STATES.join("|");
    regex::Regex::new(&format!(
        r#"(?i)^\s*({states})\s*(,\s*(usa|us|united states))?\s*$"#
    ))
    .unwrap()
}

fn caption_echo_re() -> regex::Regex {
    regex::Regex::new(r#"(?i)^\s*(photo (by|shared by)\b|may be (an|the)\b|tagging @)"#).unwrap()
}

fn broad_place_re() -> regex::Regex {
    regex::Regex::new(
        r#"(?i)\b(national park|national forest|national monument|national preserve|national recreation area|national seashore|national lakeshore|state park|provincial park|regional park|wilderness area|peninsula)\b"#,
    )
    .unwrap()
}

fn site_head_re() -> regex::Regex {
    regex::Regex::new(
        r#"(?i)\b(lake|landing|point|trail|trailhead|falls|waterfall|cabin|lookout|overlook|viewpoint|ridge|peak|summit|canyon|valley|cove|beach|harbor|lighthouse|bridge|garden|cafe|café|coffee|restaurant|shop|market|spring|springs|grove|basin|meadow|island|camp|campground|house|building|museum|theater|theatre|gallery|pier|boardwalk|cathedral|church|temple|pool|pools|dunes|cavern|cave)\b"#,
    )
    .unwrap()
}

pub fn is_vague_place(title: &str) -> bool {
    if title.is_empty() {
        return true;
    }
    let t = title.trim();
    if vague_place_re().is_match(t) || generic_place_only_re().is_match(t) {
        return true;
    }
    if state_place_re().is_match(t) {
        return true;
    }
    if caption_echo_re().is_match(t) {
        return true;
    }
    if let Some(m) = broad_place_re().find(t) {
        let head: String = t[..m.start()]
            .trim_matches(|c: char| " \t-\u{2013}\u{2014},|/(".contains(c))
            .to_string();
        if head.is_empty() || !site_head_re().is_match(&head) || vague_place_re().is_match(&head) {
            return true;
        }
    }
    false
}

fn norm_fu_title(title: &str) -> String {
    let s = regex::Regex::new(r"[^a-z0-9]+")
        .unwrap()
        .replace_all(&title.to_lowercase(), " ")
        .to_string();
    let s = regex::Regex::new(r"\bmt\b").unwrap().replace_all(&s, "mount").to_string();
    regex::Regex::new(r"\bst\b").unwrap().replace_all(&s, "saint").trim().to_string()
}

fn fu_titles_collide(a: &str, b: &str, max_tail: usize) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
    let mut prefix = short.to_string();
    prefix.push(' ');
    if !long.starts_with(&prefix) {
        return false;
    }
    long[prefix.len()..].split_whitespace().count() <= max_tail
}

const ALLOWED_FOLLOWUP_KINDS: &[&str] = &[
    "listen", "games", "app", "buy", "watch", "read", "recipe", "build", "visit", "todo",
];

fn app_store_hosts() -> Vec<&'static str> {
    crate::ingest::APP_STORE_HOSTS.to_vec()
}

fn promote_app_store_links(raw: &mut Vec<Value>, item_extra: &Value) {
    let outlinks = item_extra.get("outlinks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for link in outlinks {
        let url = link.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let host = crate::vault::host_of_pub(url);
        let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
        if !app_store_hosts().contains(&host.as_str()) {
            continue;
        }
        if raw.iter().any(|x| x.get("url").and_then(|v| v.as_str()) == Some(url)) {
            continue;
        }
        raw.push(json!({
            "kind": "app",
            "title": link.get("title").and_then(|v| v.as_str()).unwrap_or("app store app"),
            "url": url,
            "why": "app store link in the saved post",
            "confidence": 1.0,
        }));
    }
}

fn resolve_screenshot_buys(raw: &mut Vec<Value>, source_urls: &[String]) {
    let mut source_asins: HashSet<String> = HashSet::new();
    for u in source_urls {
        let a = resolve::amazon_asin(u);
        if !a.is_empty() {
            source_asins.insert(a);
        }
    }
    for candidate in raw.iter_mut() {
        if !candidate.is_object() {
            continue;
        }
        let kind = candidate.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        let title = candidate.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let why = candidate.get("why").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        if kind != "buy" || title.is_empty() {
            continue;
        }
        let candidate_url = candidate.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let candidate_asin = resolve::amazon_asin(&candidate_url);
        let source_match = source_urls.contains(&candidate_url)
            || (!candidate_asin.is_empty() && source_asins.contains(&candidate_asin));
        let screenshot_evidence = ["listing", "screenshot", "amazon", "product page", "plausible", "exact one is not visible", "image"]
            .iter()
            .any(|w| why.contains(w));
        let needs_lookup = screenshot_evidence
            && (candidate_url.is_empty() || (resolve::is_amazon_url(&candidate_url) && !source_match));
        if !needs_lookup {
            continue;
        }
        let hit = resolve::resolve_amazon_product(&title, 75);
        if hit.is_object() && !hit.as_object().unwrap().is_empty() {
            let url = hit.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            candidate.as_object_mut().unwrap().insert("url".into(), json!(url));
            candidate.as_object_mut().unwrap().insert("title".into(), json!(title));
            if hit.get("is_search").and_then(|v| v.as_bool()).unwrap_or(false) {
                let new_why = format!(
                    "{} · Amazon search",
                    candidate.get("why").and_then(|v| v.as_str()).unwrap_or("product listing shown in the saved image")
                );
                candidate.as_object_mut().unwrap().insert("why".into(), json!(new_why));
            }
        }
    }
}

fn resolve_masked_app_urls(raw: &mut Vec<Value>) {
    for candidate in raw.iter_mut() {
        if !candidate.is_object() {
            continue;
        }
        let ckind = candidate.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        let ctitle = candidate.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let cur_url = candidate.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let why = candidate.get("why").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if ckind != "app" && ckind != "buy" || ctitle.is_empty() {
            continue;
        }
        if !cur_url.is_empty() && !resolve::is_masked_url(&cur_url) {
            continue;
        }
        if !resolve::is_app_like(&ctitle, &why) {
            continue;
        }
        let store_url = resolve::applemac_store_url(&ctitle);
        if !store_url.is_empty() {
            let obj = candidate.as_object_mut().unwrap();
            obj.insert("url".into(), json!(store_url));
            obj.insert("kind".into(), json!("app"));
        }
    }
}

fn followup_row(conn: &Connection, item_id: &str, f: &Value) -> Option<(String, String, String, String, f64)> {
    let kind = f.get("kind").and_then(|v| v.as_str()).unwrap_or("todo").trim().to_lowercase();
    if !ALLOWED_FOLLOWUP_KINDS.contains(&kind.as_str()) {
        return None;
    }
    let mut title: String = f.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(200).collect();
    let mut artist: String = f.get("artist").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(120).collect();
    let mut url: String = f.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(600).collect();
    let mut why: String = f.get("why").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(300).collect();
    if !url.is_empty() && resolve::is_masked_url(&url) {
        url = String::new();
    }
    let conf = f
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or_else(|| {
            f.get("confidence")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(0.8)
        });
    if kind == "listen" {
        if !artist.is_empty()
            && !title.to_lowercase().contains(&artist.to_lowercase())
            && !title.contains('\u{2014}')
        {
            title = format!("{title} \u{2014} {artist}");
        }
        if artist.is_empty() {
            let row = vault::fetch_rows(conn, "SELECT extra FROM items WHERE id=?", &[json!(item_id)])
                .into_iter()
                .next();
            if let Some(row) = row {
                let ex: Map<String, Value> = row
                    .get("extra")
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                let mus = ex.get("ig_music").cloned().unwrap_or(json!({}));
                let song = mus.get("song").and_then(|v| v.as_str()).unwrap_or("");
                let mus_artist = mus.get("artist").and_then(|v| v.as_str()).unwrap_or("");
                if !mus_artist.is_empty() && !song.is_empty() {
                    let tl = title.to_lowercase();
                    if song.to_lowercase().contains(&tl) || tl.contains(&song.to_lowercase()) {
                        artist = mus_artist.to_string();
                        if !title.to_lowercase().contains(&artist.to_lowercase()) {
                            title = format!("{title} \u{2014} {artist}");
                        }
                    }
                }
            }
        }
        if ["instagram.com", "tiktok.com", "x.com"].iter().any(|h| url.contains(h)) {
            url = String::new();
        }
        if url.is_empty() {
            let hit = resolve::youtube_for(&format!("{title} {artist}").trim());
            if hit.get("url").and_then(|v| v.as_str()).map(|u| !u.is_empty()).unwrap_or(false) {
                url = hit.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if artist.is_empty() {
                    if let Some(channel) = hit.get("channel").and_then(|v| v.as_str()) {
                        if !channel.is_empty() {
                            why = format!("top YouTube hit · {channel}");
                        }
                    }
                }
            }
        }
    } else if kind == "watch" && url.is_empty() {
        let hit = resolve::justwatch_for(&title);
        if let Some(stremio) = hit.get("stremio").and_then(|v| v.as_str()) {
            url = stremio.to_string();
            if why.is_empty() {
                if let Some(provider) = hit.get("provider").and_then(|v| v.as_str()) {
                    if !provider.is_empty() {
                        why = format!("streams: {provider} via Stremio");
                    } else {
                        why = "watch via Stremio".into();
                    }
                }
            }
        } else if let Some(u) = hit.get("url").and_then(|v| v.as_str()) {
            url = u.to_string();
            if why.is_empty() {
                if let Some(provider) = hit.get("provider").and_then(|v| v.as_str()) {
                    if !provider.is_empty() {
                        why = format!("streaming: {provider}");
                    }
                }
            }
        } else if let Some(u) = hit.get("jw_url").and_then(|v| v.as_str()) {
            url = u.to_string();
        }
    } else if kind == "games" && title.is_empty() {
        return None;
    } else if kind == "games" {
        let has_steam = regex::Regex::new(r"(?i)(?:https?://)?(?:store\.)?steampowered\.com/app/\d+")
            .map(|re| re.is_match(&url))
            .unwrap_or(false);
        if !has_steam {
            let irow = vault::fetch_rows(conn, "SELECT url FROM items WHERE id=?", &[json!(item_id)])
                .into_iter()
                .next();
            let item_url = irow.and_then(|r| r.get("url").and_then(|v| v.as_str()).map(|s| s.to_string())).unwrap_or_default();
            let src = if !url.is_empty() && ["x.com", "twitter.com"].contains(&crate::vault::host_of_pub(&url).as_str()) {
                item_url
            } else {
                String::new()
            };
            let mut hit = if src.is_empty() { json!({}) } else { resolve::steam_from_x_author(&src) };
            if hit.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                hit = resolve::steam_for(&title);
            }
            if let Some(u) = hit.get("url").and_then(|v| v.as_str()) {
                url = u.to_string();
                if why.is_empty() || u == f.get("url").and_then(|v| v.as_str()).unwrap_or("") {
                    if why.is_empty() {
                        why = "Steam store page".into();
                    }
                }
            } else if !url.is_empty() && ["x.com", "twitter.com"].contains(&crate::vault::host_of_pub(&url).as_str()) {
                url = String::new();
            }
        }
    }
    if title.is_empty() && url.is_empty() {
        return None;
    }
    if kind == "visit" {
        if is_vague_place(&title) {
            return None;
        }
        if url.is_empty() {
            let q = if artist.is_empty() {
                title.clone()
            } else {
                format!("{title}, {artist}")
            };
            url = resolve::maps_place_url(&q);
        }
    }
    let mut kind = kind;
    if !url.is_empty() && crate::ingest::is_shopping_url(&url) {
        kind = "buy".into();
    }
    let host = if url.is_empty() { String::new() } else { crate::vault::host_of_pub(&url) };
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if app_store_hosts().contains(&host.as_str()) {
        kind = "app".into();
    }
    Some((kind, title, url, why, conf))
}

fn store_followups(conn: &Connection, item_id: &str, raw: Value) {
    let mut raw: Vec<Value> = raw.as_array().cloned().unwrap_or_default();
    let (item_extra, source_urls) = followup_source(conn, item_id);
    let _ = promote_app_store_links(&mut raw, &item_extra);
    let _ = resolve_screenshot_buys(&mut raw, &source_urls);
    let _ = resolve_masked_app_urls(&mut raw);
    let mut rows: Vec<(String, String, String, String, f64)> = Vec::new();
    for f in raw {
        if !f.is_object() {
            continue;
        }
        if let Some(row) = followup_row(conn, item_id, &f) {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return;
    }
    // cross-item dedupe: first writer wins
    let mut have: Vec<(String, String)> = Vec::new();
    for r in vault::fetch_rows(
        conn,
        "SELECT kind, title FROM followups WHERE status='open' AND item_id<>?",
        &[json!(item_id)],
    ) {
        let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let n = norm_fu_title(&title);
        if !n.is_empty() {
            have.push((kind, n));
        }
    }
    let mut seen_same_batch: Vec<(String, String)> = Vec::new();
    let _ = conn.execute("DELETE FROM followups WHERE item_id=? AND status='open'", [item_id]);
    for (kind, title, url, why, conf) in rows {
        let key1 = norm_fu_title(&title);
        if key1.is_empty() {
            continue;
        }
        let tail = if kind == "visit" { 5 } else { 2 };
        if have.iter().any(|(k, n)| *k == kind && fu_titles_collide(n, &key1, tail)) {
            continue;
        }
        if seen_same_batch.iter().any(|(k, n)| *k == kind && fu_titles_collide(n, &key1, tail)) {
            continue;
        }
        seen_same_batch.push((kind.clone(), key1));
        let fid = format!("fu-{}", write::token_hex8());
        let _ = conn.execute(
            "INSERT INTO followups(id, item_id, kind, title, url, status, confidence, why, created_at) VALUES(?,?,?,?,?,'open',?,?,datetime('now'))",
            rusqlite::params![fid, item_id, kind, title, url, conf, why],
        );
    }
}

fn followup_source(conn: &Connection, item_id: &str) -> (Value, Vec<String>) {
    let row = vault::fetch_rows(
        conn,
        "SELECT title, text, url, extra FROM items WHERE id=?",
        &[json!(item_id)],
    )
    .into_iter()
    .next();
    let Some(row) = row else { return (json!({}), Vec::new()) };
    let item_extra: Value = row
        .get("extra")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(json!({}));
    let mut source_urls: Vec<String> = Vec::new();
    if let Some(u) = row.get("url").and_then(|v| v.as_str()) {
        if !u.is_empty() {
            source_urls.push(u.to_string());
        }
    }
    if let Some(links) = item_extra.get("outlinks").and_then(|v| v.as_array()) {
        for link in links {
            if let Some(u) = link.get("url").and_then(|v| v.as_str()) {
                if !u.is_empty() {
                    source_urls.push(u.to_string());
                }
            }
        }
    }
    let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let text = row.get("text").and_then(|v| v.as_str()).unwrap_or("");
    if let Ok(re) = regex::Regex::new(r#"https?://[^\s<>"']+"#) {
        for m in re.find_iter(&format!("{title} {text}")) {
            source_urls.push(m.as_str().to_string());
        }
    }
    (item_extra, source_urls)
}

/// explain._transcript_fallback_tags
fn transcript_fallback_tags(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut tags: Vec<String> = Vec::new();
    let phrase_tags: &[(&[&str], &str)] = &[
        (&["wonder", "uncertain", "unsure"], "uncertainty"),
        (&["question", "ask", "wonder"], "question"),
        (&["work", "function", "run"], "functionality"),
        (&["debug", "bug", "error"], "debugging"),
        (&["fix", "broken", "repair"], "troubleshooting"),
        (&["plan", "planning", "going to"], "planning"),
        (&["buy", "purchase", "shopping"], "shopping"),
        (&["build", "make", "create"], "building"),
        (&["call", "meeting", "talk"], "conversation"),
    ];
    for (needles, tag) in phrase_tags {
        if needles.iter().any(|n| lowered.contains(n)) && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    let words = regex::Regex::new(r"[a-z][a-z'-]{3,}").unwrap();
    let stop: HashSet<&str> = HashSet::from([
        "actually", "about", "after", "again", "being", "could", "does", "going",
        "gonna", "have", "just", "like", "look", "maybe", "really", "that",
        "this", "what", "when", "where", "which", "with", "would",
    ]);
    for w in words.find_iter(&lowered) {
        let word = w.as_str();
        if stop.contains(word) || audio_generic_tags().contains(&word) || tags.iter().any(|t| t == word) {
            continue;
        }
        tags.push(word.to_string());
        if tags.len() >= 6 {
            break;
        }
    }
    tags.truncate(6);
    tags
}

/// explain._run_explain
fn run_explain(conn: &Connection, item_id: &str) {
    let item = match vault::fetch_rows(conn, "SELECT * FROM items WHERE id=?", &[json!(item_id)])
        .into_iter()
        .next()
        .map(Value::Object)
    {
        Some(i) => i,
        None => return,
    };
    let gather = media_for_explain(conn, item_id);
    let result = call_model(
        &item,
        gather.get("images").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]),
        gather.get("note").and_then(|v| v.as_str()).unwrap_or("images"),
        gather.get("files").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]),
        gather.get("file_texts").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]),
        &gather
            .get("transcripts")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(|x| x.to_string())).collect::<Vec<_>>())
            .unwrap_or_default(),
        gather.get("ocr_texts").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]),
        gather.get("frame_note").and_then(|v| v.as_str()).unwrap_or(""),
    );
    let mut result = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[explain-rs] {item_id}: {e}");
            let _ = conn.execute(
                "INSERT OR REPLACE INTO explanations(item_id, status, data, updated_at) VALUES(?,?,?,datetime('now'))",
                rusqlite::params![item_id, "failed", json!({"error": e.chars().take(300).collect::<String>()}).to_string()],
            );
            refresh_explainer_fts(conn, item_id);
            return;
        }
    };
    let media_rows = vault::fetch_rows(conn, "SELECT kind, transcript FROM media WHERE item_id=? ORDER BY rowid", &[json!(item_id)]);
    let audio_transcript_only = !media_rows.is_empty()
        && media_rows.iter().all(|r| r.get("kind").and_then(|v| v.as_str()) == Some("audio"));
    let transcript_text = media_rows
        .iter()
        .filter_map(|r| r.get("transcript").and_then(|v| v.as_str()))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !result.get("tags").and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false) {
        result.as_object_mut().unwrap().insert("tags".into(), json!([]));
    }
    let mut tags: Vec<String> = result
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str())
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.to_lowercase().trim().chars().take(40).collect::<String>())
                .collect()
        })
        .unwrap_or_default();
    if audio_transcript_only {
        tags.retain(|tag| {
            let generic = audio_generic_tags();
            !generic.contains(&tag.as_str()) && !generic.contains(&tag.replace('-', " ").as_str())
        });
        if tags.is_empty() {
            tags = transcript_fallback_tags(&transcript_text);
        }
    }
    if tags.is_empty() {
        let words = result
            .get("color_words")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace("and", " ")
            .to_lowercase();
        let mut fallback: Vec<String> = words
            .replace(',', " ")
            .split_whitespace()
            .filter(|w| !w.is_empty())
            .map(|w| w.to_string())
            .collect();
        if !audio_transcript_only {
            fallback.push(item.get("platform").and_then(|v| v.as_str()).unwrap_or("saved").to_string());
            fallback.push(item.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string());
        }
        tags = fallback.into_iter().take(10).collect();
    }
    let existing = vault::fetch_rows(conn, "SELECT tag FROM tags WHERE item_id=?", &[json!(item_id)]);
    let mut have: HashSet<String> = existing
        .iter()
        .filter_map(|r| r.get("tag").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    if audio_transcript_only {
        let _ = conn.execute("DELETE FROM tags WHERE item_id=?", [item_id]);
        have.clear();
    }
    for tag in tags.iter().take(10) {
        let tag = tag.to_lowercase().trim().chars().take(40).collect::<String>();
        if tag.is_empty() || have.contains(&tag) {
            continue;
        }
        let _ = conn.execute("INSERT INTO tags(item_id, tag) VALUES(?,?)", rusqlite::params![item_id, tag]);
        have.insert(tag);
    }
    store_followups(conn, item_id, result.get("followups").cloned().unwrap_or(json!(null)));
    let _ = conn.execute(
        "INSERT OR REPLACE INTO explanations(item_id, status, data, updated_at) VALUES(?,?,?,datetime('now'))",
        rusqlite::params![item_id, "done", result.to_string()],
    );
    refresh_explainer_fts(conn, item_id);
}

/// db.refresh_explainer_fts — external-content FTS sync
fn refresh_explainer_fts(conn: &Connection, item_id: &str) {
    let _ = conn.execute("DELETE FROM expl_fts WHERE item_id=?", [item_id]);
    let row = vault::fetch_rows(
        conn,
        "SELECT data FROM explanations WHERE item_id=? AND status='done'",
        &[json!(item_id)],
    )
    .into_iter()
    .next();
    if let Some(row) = row {
        let data = row.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let Ok(parsed) = serde_json::from_str::<Value>(data) else { return };
        let text = flatten_explainer(&parsed);
        if !text.trim().is_empty() {
            let _ = conn.execute(
                "INSERT INTO expl_fts(item_id, body) VALUES(?,?)",
                rusqlite::params![item_id, text],
            );
        }
    }
}

/// db._flatten_explainer — every string and list-of-strings joined with \n
fn flatten_explainer(value: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    fn walk(v: &Value, parts: &mut Vec<String>) {
        match v {
            Value::String(s) => parts.push(s.clone()),
            Value::Object(o) => {
                for val in o.values() {
                    walk(val, parts);
                }
            }
            Value::Array(a) => {
                for val in a {
                    if val.is_string() {
                        walk(val, parts);
                    }
                }
            }
            _ => {}
        }
    }
    walk(value, &mut parts);
    parts.join("\n").chars().take(12000).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn b64_roundtrip() {
        // Self-contained: build the fixture instead of depending on a scratch
        // file from the port session (this test used to fail on a clean clone).
        let dir = std::env::temp_dir().join(format!("te-explain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = dir.join("explain_test.jpg");
        if !fixture.exists() {
            let img = image::RgbImage::from_fn(64, 48, |x, y| {
                image::Rgb([(x * 4) as u8, (y * 5) as u8, 128u8])
            });
            img.save_with_format(&fixture, image::ImageFormat::Jpeg)
                .expect("write fixture");
        }
        let data = std::fs::read(&fixture).expect("test jpeg");
        let b64 = super::downscale_to_b64(&data);
        assert!(!b64.is_empty(), "empty b64");
        eprintln!("b64 len={} head={}", b64.len(), &b64[..b64.len().min(24)]);
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .expect("b64 decode");
        eprintln!("jpeg bytes len={} head={:02x?}", bytes.len(), &bytes[..4.min(bytes.len())]);
        std::fs::write(dir.join("rs_b64_roundtrip.jpg"), &bytes).expect("write");
    }
}

