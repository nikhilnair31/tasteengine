//! Media storage — port of media.py: download_bytes, sniffing, save_media
//! (file + 720px JPEG thumb + palette + blank detection + sha256/phash),
//! replace_preview, video probe/frame extraction via ffmpeg.

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::vault;

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

pub fn media_dir() -> PathBuf {
    // The container sets TV_DATA_DIR=/data; local runs fall back to the checkout.
    if let Ok(dir) = std::env::var("TV_DATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir).join("media");
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Projects/taste-engine/data/media")
}

pub fn media_keep_bytes() -> i64 {
    crate::write::env_num("TV_MEDIA_KEEP_BYTES", 64 * 1024 * 1024)
}

/// Shared HTTP agent with a custom timeout (resolvers use their own budgets).
pub fn http_agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(10)
        .user_agent(UA)
        .build()
}

/// Every download goes through the per-source gate: admit before the request,
/// report after it, so a platform 429 becomes a persisted cooldown.
pub fn download_bytes(url: &str) -> Result<(Vec<u8>, String), String> {
    crate::gate::fetch(url, 25)
}

pub fn sniff_image(data: &[u8]) -> bool {
    image::load_from_memory(data).is_ok()
}

pub fn sniff_video(path: &Path) -> bool {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type", "-of", "csv=p=0"])
        .arg(path)
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("video"),
        Err(_) => false,
    }
}

/// video.probe: (width, height, duration_seconds)
pub fn video_probe(path: &Path) -> (i64, i64, f64) {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
               "stream=width,height:format=duration", "-of", "json"])
        .arg(path)
        .output();
    let Ok(o) = out else { return (0, 0, 0.0) };
    let Ok(v) = serde_json::from_slice::<Value>(&o.stdout) else { return (0, 0, 0.0) };
    let w = v.get("streams").and_then(|s| s.as_array()).and_then(|a| a.first())
        .and_then(|s| s.get("width")).and_then(|x| x.as_i64()).unwrap_or(0);
    let h = v.get("streams").and_then(|s| s.as_array()).and_then(|a| a.first())
        .and_then(|s| s.get("height")).and_then(|x| x.as_i64()).unwrap_or(0);
    let dur = v.get("format").and_then(|f| f.get("duration")).and_then(|x| x.as_str())
        .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    (w, h, dur)
}

/// video.extract_frame: one JPEG at a time ratio (0..1)
pub fn video_extract_frame(path: &Path, at_ratio: f64) -> Option<Vec<u8>> {
    let (_w, _h, duration) = video_probe(path);
    let duration = if duration > 0.0 { duration } else { 1.0 };
    let t = (duration * 0.98).min(duration * at_ratio).max(0.0);
    let out_path = std::env::temp_dir().join(format!(
        "tvframe_{}_{}.jpg",
        std::process::id(),
        crate::write::token_hex8()
    ));
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-ss", &format!("{t}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-q:v", "3"])
        .arg(&out_path)
        .output()
        .ok()?;
    if !status.status.success() {
        let _ = std::fs::remove_file(&out_path);
        return None;
    }
    let data = std::fs::read(&out_path).ok();
    let _ = std::fs::remove_file(&out_path);
    data.filter(|d| !d.is_empty())
}

/// video.extract_frames: n frames spread over the video
pub fn video_extract_frames(path: &Path, n: usize) -> Vec<Vec<u8>> {
    let (_w, _h, duration) = video_probe(path);
    let duration = if duration > 0.0 { duration } else { 1.0 };
    let mut frames = Vec::new();
    for i in 0..n {
        let ratio = if n == 1 { 0.35 } else { (i as f64 + 0.5) / n as f64 };
        if let Some(f) = video_extract_frame(path, ratio.min(0.98)) {
            frames.push(f);
        }
    }
    let _ = duration;
    frames
}

fn name_ext(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    match base.rfind('.') {
        Some(i) if base.len() - i <= 6 => base[i..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn kind_for_upload(mime: &str, name: &str) -> String {
    let m = mime.to_string();
    let n = name.to_lowercase();
    if m.starts_with("image/") || ["jpg", "jpeg", "png", "webp", "gif", "avif", "heic", "svg"]
        .iter()
        .any(|e| n.ends_with(&format!(".{e}")))
    {
        return "image".into();
    }
    if m.starts_with("video/") || ["mp4", "webm", "mov", "m4v", "avi", "mkv"]
        .iter()
        .any(|e| n.ends_with(&format!(".{e}")))
    {
        return "video".into();
    }
    if m.starts_with("audio/") || ["mp3", "wav", "ogg", "opus", "m4a", "aac", "flac"]
        .iter()
        .any(|e| n.ends_with(&format!(".{e}")))
    {
        return "audio".into();
    }
    "file".into()
}

/// Grayscale stddev of a 64x64 downsample; ~0 means solid/blank.
pub fn blank_score(img: &image::DynamicImage) -> f64 {
    let g = img.to_luma8();
    let g = image::imageops::resize(&g, 64, 64, image::imageops::FilterType::Triangle);
    let px: Vec<f64> = g.pixels().map(|p| p.0[0] as f64).collect();
    let n = px.len() as f64;
    let mean = px.iter().sum::<f64>() / n;
    (px.iter().map(|p| (p - mean) * (p - mean)).sum::<f64>() / n).sqrt()
}

struct Bucket {
    rgb: (i64, i64, i64),
    lab: (f64, f64, f64),
    count: i64,
    /// Perceptual chroma from Oklch, not HSV saturation.
    chroma: f64,
    score: f64,
    pop: bool,
}

fn color_buckets(pixels: &[(i64, i64, i64)], step: i64, merge: f64) -> Vec<Bucket> {
    let mut hist: HashMap<(i64, i64, i64), i64> = HashMap::new();
    for p in pixels {
        *hist
            .entry((p.0 / step * step, p.1 / step * step, p.2 / step * step))
            .or_insert(0) += 1;
    }
    let mut buckets: Vec<Bucket> = hist
        .into_iter()
        .map(|(rgb, count)| {
            let lab = crate::color::srgb_to_oklab(rgb.0 as u8, rgb.1 as u8, rgb.2 as u8);
            Bucket { rgb, lab, count, chroma: 0.0, score: 0.0, pop: false }
        })
        .collect();
    buckets.sort_by(|a, b| b.count.cmp(&a.count));
    let mut out: Vec<Bucket> = Vec::new();
    for b in buckets {
        let mut merged = false;
        for o in out.iter_mut() {
            if crate::color::delta_e(b.lab, o.lab) <= merge {
                o.count += b.count;
                merged = true;
                break;
            }
        }
        if !merged {
            out.push(b);
        }
    }
    for o in out.iter_mut() {
        let (_l, c, _h) = crate::color::oklch(o.lab);
        o.chroma = c;
        o.score = (o.count as f64).powf(0.6) * crate::color::salience(o.lab);
    }
    out
}

fn far_from(lab: (f64, f64, f64), chosen: &[Bucket], dist: f64) -> bool {
    chosen.iter().all(|c| crate::color::delta_e(lab, c.lab) > dist)
}

/// media.palette_from_image: saturation-weighted greedy pass + vivid pass.
pub fn palette_from_image(img: &image::DynamicImage) -> Value {
    let img = img.to_rgb8();
    let (w, h) = (img.width() as i64, img.height() as i64);
    let n: usize = 6;
    // base palette on a workable downscale
    let base_img = if w * h > 240 * 240 {
        let nh = (240.0 * h as f64 / w as f64).max(1.0) as u32;
        image::imageops::resize(&img, 240, nh, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let base_px: Vec<(i64, i64, i64)> = base_img
        .pixels()
        .map(|p| (p[0] as i64, p[1] as i64, p[2] as i64))
        .collect();
    let mut buckets = color_buckets(&base_px, 8, crate::color::MERGE_DE);
    let total: i64 = buckets.iter().map(|b| b.count).sum::<i64>().max(1);
    buckets.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    let mut chosen: Vec<Bucket> = Vec::new();
    for b in buckets {
        if chosen.len() >= n {
            break;
        }
        if far_from(b.lab, &chosen, crate::color::DISTINCT_DE) {
            chosen.push(b);
        }
    }
    // vivid pass at (near-)full resolution
    let vw = w.min(900);
    let vimg = if w.max(h) <= 900 {
        img.clone()
    } else {
        let vh = (h as f64 * vw as f64 / w as f64).max(1.0) as u32;
        image::imageops::resize(&img, vw as u32, vh, image::imageops::FilterType::Triangle)
    };
    let vpx: Vec<(i64, i64, i64)> = vimg
        .pixels()
        .filter(|p| {
            let (_l, c, _h) = crate::color::oklch(crate::color::srgb_to_oklab(p[0], p[1], p[2]));
            c >= 0.05
        })
        .map(|p| (p[0] as i64, p[1] as i64, p[2] as i64))
        .collect();
    let vcount = (vpx.len() as i64).max(1);
    // Percentage denominator: every pixel of the vivid pass, NOT just the saturated ones.
    // Using the saturated count inflated pop swatches 3-5x against their true share of the image.
    let vpix_total = (vimg.width() as i64 * vimg.height() as i64).max(1);
    let mut vbuckets = color_buckets(&vpx, 8, crate::color::MERGE_DE);
    vbuckets.sort_by(|a, b| b.chroma.partial_cmp(&a.chroma).unwrap());
    let min_count = (vcount as f64 * 0.0001).floor() as i64;
    for vb in vbuckets {
        if vb.count < min_count {
            continue;
        }
        if chosen.len() >= n && chosen.iter().filter(|c| c.pop).count() >= 2 {
            break;
        }
        let (vl, vc, _vh) = crate::color::oklch(vb.lab);
        if vc >= 0.06 && (0.25..=0.95).contains(&vl)
            && far_from(vb.lab, &chosen, crate::color::DISTINCT_DE)
        {
            let mut vb = vb;
            vb.pop = true;
            if chosen.len() < n {
                chosen.push(vb);
            } else {
                let idx = (1..chosen.len())
                    .filter(|i| !chosen[*i].pop)
                    .min_by(|a, b| chosen[*a].score.partial_cmp(&chosen[*b].score).unwrap());
                if let Some(i) = idx {
                    if chosen[i].score < vb.score {
                        chosen[i] = vb;
                    }
                }
            }
        }
    }
    chosen.sort_by(|a, b| b.count.cmp(&a.count));
    let mut palette: Vec<Value> = Vec::new();
    for b in &chosen {
        // Both passes report a share of the frame they were sampled from, so the
        // percentages stay comparable across swatches.
        let denom = if b.pop { vpix_total } else { total };
        let pct = crate::vault::pyround_pub(100.0 * b.count as f64 / denom as f64, 1);
        palette.push(json!({
            "hex": format!("#{:02x}{:02x}{:02x}", b.rgb.0, b.rgb.1, b.rgb.2),
            "pct": pct,
        }));
    }
    let dominant = palette
        .first()
        .and_then(|p| p.get("hex"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let hue = if dominant.is_empty() {
        crate::vault::classify_hue(0.0, 0.0, 0.0)
    } else {
        let hex = dominant.trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
        crate::vault::classify_hue(byte(0) as f64, byte(2) as f64, byte(4) as f64)
    };
    json!({"palette": palette, "dominant": dominant, "hue": hue})
}

/// Recompute palette / dominant / hue for stored media, one page at a time.
///
/// Pages by media id, so a whole-vault pass is a loop of calls and never holds the
/// DB lock for minutes. Reads the thumb (falls back to the file) from
/// `<data_dir>/media`. Safe to re-run: it only overwrites the three color columns.
pub fn recompute_colors(conn: &Connection, data_dir: &Path, limit: i64, after_id: &str) -> Value {
    let limit = limit.clamp(1, 1000);
    let mut stmt = match conn.prepare(
        "SELECT id, thumb, file FROM media \
         WHERE kind IN ('image','video') AND id > ?1 ORDER BY id LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(e) => return json!({"error": e.to_string(), "scanned": 0, "updated": 0, "failed": 0}),
    };
    let rows: Vec<(String, String, String)> = match stmt
        .query_map(rusqlite::params![after_id, limit], |r| {
            // thumb/file are nullable on rows written before the column existed.
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })
        .and_then(|it| it.collect::<Result<Vec<_>, _>>())
    {
        Ok(v) => v,
        Err(e) => return json!({"error": e.to_string(), "scanned": 0, "updated": 0, "failed": 0}),
    };
    let dir = data_dir.join("media");
    let last = rows.last().map(|r| r.0.clone()).unwrap_or_default();
    let scanned = rows.len() as i64;
    let (mut updated, mut failed) = (0i64, 0i64);
    for (id, thumb, file) in rows {
        let name = if thumb.is_empty() { file } else { thumb };
        if name.is_empty() {
            failed += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(dir.join(&name)) else {
            failed += 1;
            continue;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            failed += 1;
            continue;
        };
        let colors = palette_from_image(&img);
        let palette = colors.get("palette").cloned().unwrap_or(json!([])).to_string();
        let dominant = colors.get("dominant").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let hue = colors.get("hue").and_then(|v| v.as_str()).unwrap_or("").to_string();
        match conn.execute(
            "UPDATE media SET palette=?, dominant=?, hue=? WHERE id=?",
            rusqlite::params![palette, dominant, hue, id],
        ) {
            Ok(_) => updated += 1,
            Err(_) => failed += 1,
        }
    }
    json!({
        "scanned": scanned,
        "updated": updated,
        "failed": failed,
        "next_after": last,
        "done": scanned < limit,
    })
}

fn img_ext_by_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" | "image/jpg" => Some(".jpg"),
        "image/png" => Some(".png"),
        "image/webp" => Some(".webp"),
        "image/gif" => Some(".gif"),
        "image/avif" => Some(".avif"),
        "image/heic" => Some(".heic"),
        _ => None,
    }
}

fn sniff_ext(data: &[u8], mime: &str, url: &str, original_name: &str) -> String {
    if let Some(e) = img_ext_by_mime(mime) {
        return e.to_string();
    }
    if mime.starts_with("video/") {
        return mime_video_ext(mime);
    }
    if let Some(e) = audio_ext_by_mime(mime) {
        return e.to_string();
    }
    if !url.is_empty() {
        let e = name_ext(url);
        if !e.is_empty() {
            return e;
        }
    }
    if data.len() > 12 && (&data[4..8] == b"ftyp" || &data[4..8] == b"moov" || &data[4..8] == b"mdat") {
        return ".mp4".into();
    }
    if data.starts_with(b"GIF8") {
        return ".gif".into();
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        return ".jpg".into();
    }
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return ".png".into();
    }
    if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return ".webp".into();
    }
    if !original_name.is_empty() {
        let e = name_ext(original_name);
        if !e.is_empty() {
            return e;
        }
    }
    ".bin".into()
}

fn mime_video_ext(mime: &str) -> String {
    match mime {
        "video/webm" => ".webm".into(),
        "video/quicktime" => ".mov".into(),
        _ => ".mp4".into(),
    }
}

fn audio_ext_by_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "audio/mpeg" | "audio/mp3" => Some(".mp3"),
        "audio/wav" | "audio/x-wav" => Some(".wav"),
        "audio/ogg" => Some(".ogg"),
        "audio/opus" => Some(".opus"),
        "audio/webm" => Some(".webm"),
        "audio/mp4" | "audio/x-m4a" => Some(".m4a"),
        "audio/aac" => Some(".aac"),
        "audio/flac" => Some(".flac"),
        _ => None,
    }
}

fn mime_from_ext(ext: &str) -> &'static str {
    match ext {
        ".jpg" | ".jpeg" => "image/jpeg",
        ".png" => "image/png",
        ".webp" => "image/webp",
        ".gif" => "image/gif",
        ".avif" => "image/avif",
        ".mp4" => "video/mp4",
        ".webm" => "video/webm",
        ".mov" => "video/quicktime",
        ".mp3" => "audio/mpeg",
        ".wav" => "audio/wav",
        ".m4a" => "audio/mp4",
        _ => "",
    }
}

/// media.save_media
pub fn save_media(
    conn: &Connection,
    item_id: &str,
    data: &[u8],
    url: &str,
    mime_in: &str,
    kind_hint: &str,
    original_name: &str,
) -> Result<Value, String> {
    let mime = mime_in.split(';').next().unwrap_or("").trim().to_lowercase();
    let original_name: String = original_name.trim().chars().take(255).collect();
    let ext = sniff_ext(data, &mime, url, &original_name);
    let mid = crate::write::token_hex8();
    let filename = format!("{mid}{ext}");
    let dir = media_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&filename);
    std::fs::write(&path, data).map_err(|e| e.to_string())?;

    let mut kind = if ["image", "video", "audio", "file"].contains(&kind_hint) && !kind_hint.is_empty() {
        kind_hint.to_string()
    } else {
        let mut k = kind_for_upload(&mime, if original_name.is_empty() { url } else { &original_name });
        if k == "file" {
            k = match ext.as_str() {
                ".jpg" | ".jpeg" | ".png" | ".webp" | ".gif" | ".avif" | ".heic" | ".svg" => "image",
                ".mp4" | ".webm" | ".mov" | ".m4v" | ".avi" | ".mkv" => "video",
                ".mp3" | ".wav" | ".ogg" | ".opus" | ".m4a" | ".aac" | ".flac" => "audio",
                _ => "file",
            }
            .to_string();
        }
        k
    };
    if kind == "file" {
        kind = match ext.as_str() {
            ".jpg" | ".jpeg" | ".png" | ".webp" | ".gif" | ".avif" | ".heic" | ".svg" => "image",
            ".mp4" | ".webm" | ".mov" | ".m4v" | ".avi" | ".mkv" => "video",
            ".mp3" | ".wav" | ".ogg" | ".opus" | ".m4a" | ".aac" | ".flac" => "audio",
            _ => "file",
        }
        .to_string();
    }
    let mime_out = if mime.is_empty() { mime_from_ext(&ext).to_string() } else { mime };
    let mut thumb: Option<String> = None;
    let mut placeholder = 0i64;
    let mut width = 0i64;
    let mut height = 0i64;
    let mut duration = 0.0f64;
    let mut palette = json!({"palette": [], "dominant": "", "hue": ""});

    if kind == "image" && ext != ".gif" {
        if let Ok(img) = image::load_from_memory(data) {
            width = img.width() as i64;
            height = img.height() as i64;
            let rgb = img.to_rgb8();
            let dyn_rgb = image::DynamicImage::ImageRgb8(rgb.clone());
            let (tw, th) = (rgb.width(), rgb.height());
            let thumb_img = if tw.max(th) > 720 {
                let ratio = 720.0 / tw.max(th) as f64;
                dyn_rgb.resize_exact(
                    ((tw as f64 * ratio) as u32).max(1),
                    ((th as f64 * ratio) as u32).max(1),
                    image::imageops::FilterType::Triangle,
                )
            } else {
                dyn_rgb
            };
            let thumb_name = format!("{mid}.jpg");
            let thumb_path = dir.join(&thumb_name);
            let mut jpg = Vec::new();
            if image::DynamicImage::ImageRgb8(thumb_img.to_rgb8())
                .write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg)
                .is_ok()
            {
                if std::fs::write(&thumb_path, &jpg).is_ok() {
                    thumb = Some(thumb_name);
                }
            }
            let dyn_orig = image::DynamicImage::ImageRgb8(rgb);
            if blank_score(&dyn_orig) < 5.0 {
                placeholder = 1;
            }
            palette = palette_from_image(&thumb_img);
        }
    } else if kind == "video" {
        let (w, h, dur) = video_probe(&path);
        width = w;
        height = h;
        duration = dur;
        if let Some(frame) = video_extract_frame(&path, 0.82) {
            let thumb_name = format!("{mid}.jpg");
            if std::fs::write(dir.join(&thumb_name), &frame).is_ok() {
                thumb = Some(thumb_name);
            }
            if let Ok(img) = image::load_from_memory(&frame) {
                if blank_score(&img) < 5.0 {
                    placeholder = 1;
                }
                palette = palette_from_image(&img.to_rgb8().into());
            }
        }
    } else if ext == ".gif" {
        if let Ok(img) = image::load_from_memory(data) {
            width = img.width() as i64;
            height = img.height() as i64;
            let first = img.to_rgb8();
            let dyn_first = image::DynamicImage::ImageRgb8(first.clone());
            let thumb_img = if first.width().max(first.height()) > 720 {
                let ratio = 720.0 / first.width().max(first.height()) as f64;
                dyn_first.resize_exact(
                    ((first.width() as f64 * ratio) as u32).max(1),
                    ((first.height() as f64 * ratio) as u32).max(1),
                    image::imageops::FilterType::Triangle,
                )
            } else {
                dyn_first
            };
            let thumb_name = format!("{mid}.jpg");
            let mut jpg = Vec::new();
            if image::DynamicImage::ImageRgb8(thumb_img.to_rgb8())
                .write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg)
                .is_ok()
            {
                if std::fs::write(dir.join(&thumb_name), &jpg).is_ok() {
                    thumb = Some(thumb_name);
                }
            }
            if blank_score(&image::DynamicImage::ImageRgb8(first)) < 5.0 {
                placeholder = 1;
            }
            palette = palette_from_image(&thumb_img);
        }
    }

    let hash_name = thumb.clone().unwrap_or_else(|| filename.clone());
    let hash_path = dir.join(&hash_name);
    let mut sha256 = String::new();
    let mut phash = 0i64;
    if let Some(fp) = vault::file_fingerprint(&hash_path) {
        sha256 = fp.0;
        phash = fp.1.unwrap_or(0) as i64;
    }

    let _ = conn.execute(
        "INSERT OR REPLACE INTO media \
         (id, item_id, kind, mime, original_name, transcript, file, thumb, width, height, duration, \
          palette, dominant, hue, placeholder, sha256, phash) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            mid, item_id, kind, mime_out, original_name, "", filename,
            thumb, width, height, duration,
            palette.get("palette").map(|p| p.to_string()).unwrap_or_else(|| "[]".into()),
            palette.get("dominant").and_then(|v| v.as_str()).unwrap_or(""),
            palette.get("hue").and_then(|v| v.as_str()).unwrap_or(""),
            placeholder, sha256, phash,
        ],
    );
    let row = vault::fetch_rows(conn, "SELECT * FROM media WHERE id=?", &[json!(mid)]);
    row.into_iter().next().map(Value::Object).ok_or_else(|| "insert failed".to_string())
}

/// media.replace_preview: overwrite the file + thumb of an existing row.
pub fn replace_preview(conn: &Connection, row_id: &str, data: &[u8]) -> Result<Value, String> {
    let row = vault::fetch_rows(conn, "SELECT * FROM media WHERE id=?", &[json!(row_id)])
        .into_iter()
        .next()
        .ok_or_else(|| "missing".to_string())?;
    let dir = media_dir();
    for field in ["file", "thumb"] {
        let name = row.get(field).and_then(|v| v.as_str()).unwrap_or("");
        if !name.is_empty() {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
    let ext = sniff_ext(data, "", "", "");
    let filename = format!("{row_id}{ext}");
    std::fs::write(dir.join(&filename), data).map_err(|e| e.to_string())?;
    let mut thumb: Option<String> = None;
    let mut palette = json!({"palette": [], "dominant": "", "hue": ""});
    if let Ok(img) = image::load_from_memory(data) {
        let rgb = img.to_rgb8();
        let (rw, rh) = (rgb.width(), rgb.height());
        let thumb_img = if rw.max(rh) > 720 {
            let ratio = 720.0 / rw.max(rh) as f64;
            image::DynamicImage::ImageRgb8(image::imageops::resize(
                &rgb,
                ((rw as f64 * ratio) as u32).max(1),
                ((rh as f64 * ratio) as u32).max(1),
                image::imageops::FilterType::Triangle,
            ))
        } else {
            image::DynamicImage::ImageRgb8(rgb.clone())
        };
        let thumb_name = format!("{row_id}.jpg");
        let mut jpg = Vec::new();
        if image::DynamicImage::ImageRgb8(thumb_img.to_rgb8())
            .write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg)
            .is_ok()
        {
            if std::fs::write(dir.join(&thumb_name), &jpg).is_ok() {
                thumb = Some(thumb_name);
            }
        }
        palette = palette_from_image(&thumb_img);
    }
    let mut sha256 = String::new();
    let mut phash = 0i64;
    let hash_path = dir.join(thumb.clone().unwrap_or_else(|| filename.clone()));
    if let Some(fp) = vault::file_fingerprint(&hash_path) {
        sha256 = fp.0;
        phash = fp.1.unwrap_or(0) as i64;
    }
    let _ = conn.execute(
        "UPDATE media SET file=?, thumb=?, embed=NULL, placeholder=0, sha256=?, phash=?, \
         palette=?, dominant=?, hue=? WHERE id=?",
        rusqlite::params![
            filename,
            thumb,
            sha256,
            phash,
            palette.get("palette").map(|p| p.to_string()).unwrap_or_else(|| "[]".into()),
            palette.get("dominant").and_then(|v| v.as_str()).unwrap_or(""),
            palette.get("hue").and_then(|v| v.as_str()).unwrap_or(""),
            row_id,
        ],
    );
    let out = vault::fetch_rows(conn, "SELECT * FROM media WHERE id=?", &[json!(row_id)]);
    out.into_iter().next().map(Value::Object).ok_or_else(|| "missing".to_string())
}

/// media.delete_media_files
pub fn delete_media_files(row: &Value) {
    for field in ["file", "thumb"] {
        let name = row.get(field).and_then(|v| v.as_str()).unwrap_or("");
        if !name.is_empty() {
            let _ = std::fs::remove_file(media_dir().join(name));
        }
    }
}

/// OCR one still image with the tesseract CLI. Newline-joined deduped lines,
/// capped at `max_lines`. A screenshot of a post keeps its meaning in this text:
/// the image vector only knows it is a phone UI.
pub fn image_ocr(path: &Path, max_lines: usize) -> String {
    if !path.exists() {
        return String::new();
    }
    let out = match std::process::Command::new("tesseract")
        .arg(path)
        .arg("stdout")
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return String::new(),
    };
    let mut seen: Vec<String> = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.len() < 3 || line.chars().all(|c| !c.is_alphanumeric()) {
            continue;
        }
        if !seen.iter().any(|s: &String| s == line) {
            seen.push(line.to_string());
        }
        if seen.len() >= max_lines {
            break;
        }
    }
    seen.join("\n")
}

/// ocr.video_text port: OCR up to max_frames frames spread across a video.
/// Uses the tesseract CLI (rapidocr is python-only); newline-joined deduped
/// lines, capped at 60 — same shape python stores in media.ocr_text.
pub fn video_ocr(path: &Path, max_frames: usize) -> String {
    if !path.exists() {
        return String::new();
    }
    let frames = video_extract_frames(path, max_frames);
    if frames.is_empty() {
        return String::new();
    }
    let mut seen: Vec<String> = Vec::new();
    for raw in &frames {
        let mut child = match std::process::Command::new("tesseract")
            .arg("stdin")
            .arg("stdout")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return String::new(),
        };
        if child.stdin.as_mut().unwrap().write_all(raw).is_err() {
            continue;
        }
        let out = match child.wait_with_output() {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => continue,
        };
        for line in out.lines() {
            let line = line.trim();
            if !line.is_empty() && !seen.contains(&line.to_string()) {
                seen.push(line.to_string());
            }
        }
        if seen.len() >= 60 {
            break;
        }
    }
    seen.join("\n")
}
