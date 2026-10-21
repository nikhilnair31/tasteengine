//! Contact sheet: render vault search results as one dense grid image —
//! port of contact_sheet.py using ab_glyph + DejaVu fonts.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::mediaio;
use crate::vault;

const TILE: u32 = 132;
const COLS_WIDE: usize = 9;
const GAP: u32 = 4;
const PAD: u32 = 10;
const HEADER: u32 = 30;
const SCALE: u32 = 2;
const BG: [u8; 3] = [26, 23, 24];
const TEXT: [u8; 3] = [253, 246, 246];
const MUTED: [u8; 3] = [232, 184, 200];
const BADGE_BG: [u8; 3] = [26, 23, 24];
const MISSING_BG: [u8; 3] = [48, 42, 45];

const FONT_PATH: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
const FONT_BOLD_PATH: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";

fn cols_for(count: usize) -> usize {
    if count >= 15 {
        COLS_WIDE
    } else if count >= 8 {
        6
    } else if count >= 4 {
        4
    } else {
        count.max(2)
    }
}

fn safe_media_path(name: &str) -> Option<PathBuf> {
    let name = Path::new(name).file_name()?.to_string_lossy().to_string();
    if name.is_empty() {
        return None;
    }
    let path = mediaio::media_dir().join(&name);
    let root = mediaio::media_dir();
    let path = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    if !path.starts_with(&root) || !path.is_file() {
        return None;
    }
    Some(path)
}

fn load_cover(name: &str, tile: u32) -> Option<image::ImageBuffer<image::Rgb<u8>, Vec<u8>>> {
    let path = safe_media_path(name)?;
    let img = image::ImageReader::open(&path).ok()?.decode().ok()?;
    let rgb = img.to_rgb8();
    let (sw, sh) = (rgb.width(), rgb.height());
    if sw == 0 || sh == 0 {
        return None;
    }
    let side = sw.min(sh);
    let x0 = (sw - side) / 2;
    let y0 = ((sh - side) as f64 * 0.35) as u32;
    let crop = image::imageops::crop_imm(&rgb, x0, y0, side, side).to_image();
    Some(image::imageops::resize(&crop, tile, tile, image::imageops::FilterType::Triangle))
}

fn sheet_path(data_dir: &Path, query: &str, params: &Value) -> PathBuf {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(query.as_bytes());
    if let Some(o) = params.as_object() {
        for k in o.keys() {
            hasher.update(format!("|{}", o[k]));
        }
    }
    let digest = hex_encode_12(&hasher.finalize());
    let out_dir = data_dir.join("exports");
    let _ = std::fs::create_dir_all(&out_dir);
    out_dir.join(format!("sheet-{digest}.jpg"))
}

fn hex_encode_12(data: &[u8]) -> String {
    data.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn purge_old(data_dir: &Path, keep: usize) {
    let out_dir = data_dir.join("exports");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.file_name().map(|f| f.to_string_lossy().starts_with("sheet-")).unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t)
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    files.reverse();
    for stale in files.iter().skip(keep) {
        let _ = std::fs::remove_file(stale);
    }
}

fn shorten(text: &str, cap: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= cap {
        text
    } else {
        let mut s: String = text.chars().take(cap - 1).collect();
        s.push('\u{2026}');
        s
    }
}

fn font_arc(bold: bool) -> Option<ab_glyph::FontArc> {
    let font_path = if bold { FONT_BOLD_PATH } else { FONT_PATH };
    let bytes = std::fs::read(font_path).ok()?;
    ab_glyph::FontArc::try_from_vec(bytes).ok()
}

fn draw_text(img: &mut image::ImageBuffer<image::Rgb<u8>, Vec<u8>>, x: i64, y: i64, text: &str, size: f32, bold: bool, color: [u8; 3]) {
    use ab_glyph::{Font, ScaleFont};
    let Some(font) = font_arc(bold) else { return };
    let scaled = font.as_scaled(ab_glyph::PxScale::from(size));
    let mut pen_x = x as f32;
    let baseline = y as f32 + size;
    for ch in text.chars() {
        let glyph = scaled.glyph_id(ch).with_scale_and_position(size, ab_glyph::Point { x: pen_x, y: baseline });
        if let Some(outline) = scaled.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|px, py, alpha| {
                if alpha < 0.04 {
                    return;
                }
                let ix = bounds.min.x as i64 + px as i64;
                let iy = bounds.min.y as i64 + py as i64;
                if ix < 0 || iy < 0 {
                    return;
                }
                let (ix, iy) = (ix as u32, iy as u32);
                if ix >= img.width() || iy >= img.height() {
                    return;
                }
                let p = img.get_pixel_mut(ix, iy);
                let a = alpha as f32;
                for c in 0..3 {
                    p[c] = (p[c] as f32 * (1.0 - a) + color[c] as f32 * a) as u8;
                }
            });
            pen_x = bounds.max.x;
        } else {
            pen_x += scaled.h_advance(scaled.glyph_id(ch)) * 0.6;
        }
    }
}

fn text_width(text: &str, size: f32, bold: bool) -> f64 {
    use ab_glyph::{Font, ScaleFont};
    let Some(font) = font_arc(bold) else { return text.chars().count() as f64 * size as f64 * 0.6 };
    let scaled = font.as_scaled(ab_glyph::PxScale::from(size));
    let mut pen_x = 0.0f32;
    for ch in text.chars() {
        let glyph = scaled.glyph_id(ch).with_scale_and_position(size, ab_glyph::Point { x: pen_x, y: size });
        if let Some(outline) = scaled.outline_glyph(glyph) {
            pen_x = outline.px_bounds().max.x;
        } else {
            pen_x += scaled.h_advance(scaled.glyph_id(ch)) * 0.6;
        }
    }
    pen_x as f64
}

/// contact_sheet.build
pub fn build(conn: &Connection, data_dir: &Path, query: &str, limit: i64, kind: &str, source: &str, hue: &str, tag: &str) -> Value {
    let limit = limit.clamp(4, 60);
    let res = vault::vault_search_pub(
        conn,
        query,
        "auto",
        limit,
        kind,
        source,
        hue,
        tag,
        0,
    );
    let entries = res.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if entries.is_empty() {
        return json!({"query": query, "count": 0, "tiles": [], "legend_markdown": "", "note": "no vault matches"});
    }
    let mut tiles: Vec<Map<String, Value>> = Vec::new();
    for entry in &entries {
        let cover = entry.get("cover").cloned().unwrap_or(json!({}));
        let thumb = cover.get("thumb").and_then(|v| v.as_str()).unwrap_or("");
        if thumb.is_empty() {
            continue;
        }
        let mut t = Map::new();
        t.insert("item_id".into(), entry.get("id").cloned().unwrap_or(json!("")));
        t.insert("title".into(), entry.get("title").cloned().unwrap_or(json!("")));
        t.insert("url".into(), entry.get("url").cloned().unwrap_or(json!("")));
        t.insert("kind".into(), cover.get("kind").cloned().unwrap_or_else(|| entry.get("kind").cloned().unwrap_or(json!("image"))));
        t.insert("hue".into(), cover.get("hue").cloned().unwrap_or(json!("")));
        t.insert("dominant".into(), cover.get("dominant").cloned().unwrap_or(json!("")));
        t.insert("similarity".into(), entry.get("similarity").cloned().unwrap_or(json!(0.0)));
        t.insert("source".into(), entry.get("source").cloned().unwrap_or(json!("")));
        t.insert("thumb".into(), json!(thumb));
        t.insert("width".into(), cover.get("width").cloned().unwrap_or(json!(0)));
        t.insert("height".into(), cover.get("height").cloned().unwrap_or(json!(0)));
        tiles.push(t);
        if tiles.len() >= limit as usize {
            break;
        }
    }
    if tiles.is_empty() {
        return json!({"query": query, "count": 0, "tiles": [], "legend_markdown": "", "note": "matches have no saved imagery"});
    }
    let cols = cols_for(tiles.len());
    let rows = tiles.len().div_ceil(cols);
    let tile = TILE * SCALE;
    let gap = GAP * SCALE;
    let pad = PAD * SCALE;
    let header = HEADER * SCALE;
    let width = cols as u32 * tile + (cols as u32 - 1) * gap + 2 * pad;
    let height = header + rows as u32 * tile + (rows as u32 - 1) * gap + 2 * pad;

    let mut sheet = image::ImageBuffer::from_pixel(width, height, image::Rgb(BG));

    let head = format!("{}  ·  {} of {} shown", shorten(query, 70), tiles.len(), entries.len());
    draw_text(&mut sheet, pad as i64, (pad + 2 * SCALE) as i64, &head, 13.0 * SCALE as f32, false, MUTED);

    for (idx, tile_info) in tiles.iter_mut().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let x = (pad + col as u32 * (tile + gap)) as i64;
        let y = (header + pad + row as u32 * (tile + gap)) as i64;
        let thumb_name = tile_info.get("thumb").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let cover_img = load_cover(&thumb_name, tile);
        let mut missing = false;
        match cover_img {
            Some(im) => {
                image::imageops::overlay(&mut sheet, &im, x as i64, y as i64);
            }
            None => {
                missing = true;
                for py in 0..tile {
                    for px in 0..tile {
                        sheet.put_pixel(x as u32 + px, y as u32 + py, image::Rgb(MISSING_BG));
                    }
                }
            }
        }
        if missing {
            tile_info.insert("thumb_missing".into(), json!(true));
        }
        // number badge, top-left
        let label = idx.to_string();
        let bw = text_width(&label, 11.0 * SCALE as f32, true) as u32 + 6 * SCALE;
        let bh = 15 * SCALE;
        for py in 0..bh {
            for px in 0..bw {
                let ix = x as u32 + px;
                let iy = y as u32 + py;
                if ix < width && iy < height {
                    sheet.put_pixel(ix, iy, image::Rgb(BADGE_BG));
                }
            }
        }
        draw_text(&mut sheet, x + 3 * SCALE as i64, y + 2 * SCALE as i64, &label, 11.0 * SCALE as f32, true, TEXT);
        // video marker, bottom-right
        if tile_info.get("kind").and_then(|v| v.as_str()) == Some("video") {
            let mark = "video";
            let mw = text_width(mark, 9.0 * SCALE as f32, true) as u32 + 6 * SCALE;
            let bx = x as u32 + tile - mw;
            let by = y as u32 + tile - 13 * SCALE;
            for py in 0..13 * SCALE {
                for px in 0..mw {
                    let ix = bx + px;
                    let iy = by + py;
                    if ix < width && iy < height {
                        sheet.put_pixel(ix, iy, image::Rgb(BADGE_BG));
                    }
                }
            }
            draw_text(
                &mut sheet,
                bx as i64 + 3 * SCALE as i64,
                by as i64 + SCALE as i64,
                mark,
                9.0 * SCALE as f32,
                true,
                MUTED,
            );
        }
    }

    let params = json!({"limit": limit, "kind": kind, "source": source, "hue": hue, "tag": tag});
    let out_path = sheet_path(data_dir, query, &params);
    let final_w = width / SCALE;
    let final_h = height / SCALE;
    let small = image::imageops::resize(&sheet, final_w, final_h, image::imageops::FilterType::Lanczos3);
    let _ = small.save_with_format(&out_path, image::ImageFormat::Jpeg);
    // quality 82: re-encode via JpegEncoder
    let mut jpg: Vec<u8> = Vec::new();
    {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 82);
        let _ = enc.encode_image(&small);
        let _ = std::fs::write(&out_path, &jpg);
    }
    purge_old(data_dir, 20);

    for (n, t) in tiles.iter_mut().enumerate() {
        t.insert("n".into(), json!(n));
    }
    let mut legend_rows = vec!["| n | title | kind | hue | sim |".to_string(), "|---|-------|------|-----|-----|".to_string()];
    for t in &tiles {
        let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("").replace('|', "/");
        let hue = t.get("hue").and_then(|v| v.as_str()).unwrap_or("");
        let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let sim = t.get("similarity").map(|v| v.to_string()).unwrap_or_default();
        legend_rows.push(format!("| {} | {} | {} | {} | {} |", t.get("n").map(|v| v.to_string()).unwrap_or_default(), shorten(&title, 48), kind, hue, sim));
    }
    let bytes = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    json!({
        "query": query,
        "count": tiles.len(),
        "shown_of_matches": entries.len(),
        "sheet": {
            "path": out_path.to_string_lossy(),
            "width": final_w,
            "height": final_h,
            "bytes": bytes,
            "cols": cols,
            "rows": rows,
            "tile_px": TILE,
        },
        "tiles": tiles,
        "legend_markdown": legend_rows.join("\n"),
    })
}
