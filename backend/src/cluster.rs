//! Vault clustering — one space for pictures and words, plus k-means over it.
//!
//! Two jobs live here:
//!
//! 1. **Fusion.** `item_embeds` used to hold the mean of an item's image
//!    vectors, so an item's caption never reached its own vector and text posts
//!    sat in a separate region of the space. `fused_vec` mixes the normalized
//!    image mean with the CLIP text embedding of title+body (both live in the
//!    same CLIP ViT-B/32 space, so a weighted sum is meaningful).
//! 2. **Clusters.** `fit` runs spherical k-means over the stored item vectors
//!    and writes `item_clusters` / `cluster_meta`. `random` hands back one
//!    coherent group with no model call and no job queue.
//!
//! No LLM anywhere: labels come from tags, dominant hue, and kinds.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

const DIM: usize = 512;
const IMG_W: f32 = 0.7;
const TEXT_W: f32 = 0.3;
/// A screenshot of a post carries the post's words, not a scene. Its image
/// vector describes a phone UI, so two unrelated screenshots look identical to
/// the image tower and would cluster together on nothing. Flip the weights for
/// those items and let the words decide.
const SHOT_IMG_W: f32 = 0.15;
const SHOT_TEXT_W: f32 = 0.85;
const MAX_TEXT: usize = 2000;
const MAX_ITERS: usize = 25;
/// Bump when the fusion RULES change, not when an item's content changes. Every
/// stored signature then stops matching once and the vault re-fuses in one pass.
const FUSE_V: &str = "v3";

/// Tags that every card gets from ingestion, so they say nothing about a group.
const GENERIC_TAGS: &[&str] = &[
    "twitter", "x", "image", "link", "screenshot", "video", "reddit", "instagram",
    "pinterest", "tumblr", "youtube", "note", "web", "post", "photo", "gif", "animation",
];

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "your", "you", "this", "that", "from", "how", "what", "why",
    "when", "into", "are", "was", "were", "his", "her", "its", "our", "their", "not", "but",
    "all", "can", "new", "one", "two", "out", "use", "using", "made", "make", "just", "like",
    "about", "more", "most", "some", "than", "then", "them", "they", "will", "would", "has",
    "have", "been", "who", "get", "got", "very", "best", "top", "day", "first", "now",
    "own", "way", "did", "does", "don", "off", "over", "under", "through", "between",
    "a", "an", "of", "in", "on", "to", "is", "it", "at", "by", "as", "or", "if", "my",
    "we", "be", "so", "no", "up", "do", "here", "there", "after", "before", "should",
    "could", "also", "only", "other", "self", "still", "while", "where", "which", "these",
];

pub fn ensure_tables(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS item_clusters (
           item_id TEXT PRIMARY KEY,
           cluster_id INTEGER NOT NULL,
           score REAL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_item_clusters ON item_clusters(cluster_id);
         CREATE TABLE IF NOT EXISTS cluster_meta (
           cluster_id INTEGER PRIMARY KEY,
           label TEXT DEFAULT '',
           size INTEGER DEFAULT 0,
           cohesion REAL DEFAULT 0,
           hue TEXT DEFAULT '',
           kinds TEXT DEFAULT '{}',
           fitted_at TEXT DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS cluster_runs (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           k INTEGER, n INTEGER, iters INTEGER, ms INTEGER,
           fitted_at TEXT DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS cluster_seen (
           cluster_id INTEGER PRIMARY KEY,
           views INTEGER DEFAULT 0,
           seen_at TEXT DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS item_fuse (
           item_id TEXT PRIMARY KEY,
           sig TEXT DEFAULT '',
           fused_at TEXT DEFAULT ''
         );",
    );
}

// ---------------------------------------------------------------- vectors

fn blob_to_vec(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() != DIM * 4 {
        return None;
    }
    let mut v = vec![0.0f32; DIM];
    for (i, chunk) in blob.chunks_exact(4).enumerate() {
        v[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Some(v)
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn normalize(v: &mut [f32]) -> bool {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return false;
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
    true
}

/// Mean of the item's stored image vectors, normalized. None when it has none.
fn media_mean(conn: &Connection, item_id: &str) -> Option<Vec<f32>> {
    let mut stmt = conn
        .prepare("SELECT embed FROM media WHERE item_id=? AND embed IS NOT NULL ORDER BY rowid")
        .ok()?;
    let rows = stmt
        .query_map([item_id], |r| r.get::<_, Vec<u8>>(0))
        .ok()?;
    let mut mean = vec![0.0f32; DIM];
    let mut used = 0usize;
    for blob in rows.flatten() {
        if let Some(mut v) = blob_to_vec(&blob) {
            if !normalize(&mut v) {
                continue;
            }
            for (m, x) in mean.iter_mut().zip(v.iter()) {
                *m += x;
            }
            used += 1;
        }
    }
    if used == 0 {
        return None;
    }
    for m in mean.iter_mut() {
        *m /= used as f32;
    }
    if normalize(&mut mean) {
        Some(mean)
    } else {
        None
    }
}

/// CLIP text of one prepared string. None when it is empty.
fn text_vec(joined: &str) -> Option<Vec<f32>> {
    let clipped: String = joined.chars().take(MAX_TEXT).collect();
    if clipped.trim().is_empty() {
        return None;
    }
    let mut v = crate::embed::embed_text(&[clipped]).ok()?.into_iter().next()?;
    if normalize(&mut v) {
        Some(v)
    } else {
        None
    }
}

/// Titles an importer invents: "screenshot · Aug 05 2025 · twitter", "Image
/// taken on May 24, 2025", "saved image". They describe the transport, not the
/// content, and they are near-identical across hundreds of rows, so they must
/// never reach the text tower.
fn generic_title(t: &str) -> bool {
    const PLATFORMS: &[&str] = &[
        "instagram", "twitter", "x", "reddit", "pinterest", "tumblr", "youtube", "tiktok",
        "threads", "bluesky", "cosmos", "steam", "spotify", "arena", "are.na", "web", "link",
        "note", "post", "image", "video",
    ];
    let l = t.trim().to_lowercase();
    if l.is_empty()
        || l == "saved image"
        || l.starts_with("image taken on")
        || l.starts_with("untitled")
        || (l.contains("screenshot") && l.len() < 80)
    {
        return true;
    }
    // "Instagram post", "twitter", "x post": the platform is not a subject.
    let words: Vec<&str> = l.split_whitespace().collect();
    words.len() <= 2 && words.first().is_some_and(|w| PLATFORMS.contains(w))
}

/// The words a fusion should read: the post's own text, its title when the
/// title says something, the OCR of the media (a screenshot's real subject) and
/// the explainer's digest of what the media shows.
fn item_text(title: &str, text: &str, digest: &str, ocr: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if !generic_title(title) {
        parts.push(title.trim());
    }
    if !text.trim().is_empty() {
        parts.push(text.trim());
    }
    if !ocr.trim().is_empty() {
        parts.push(ocr.trim());
    }
    if !digest.trim().is_empty() {
        parts.push(digest.trim());
    }
    let joined = parts.join("\n");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined.chars().take(MAX_TEXT).collect())
    }
}

/// Explainer summary, mood, style DNA, color words and tags as one paragraph.
/// This is the only content a screenshot-only save has, so it carries the weight
/// when the post text is empty.
fn explainer_digest(data: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return String::new();
    };
    let mut out: Vec<String> = Vec::new();
    for key in ["summary", "style_dna", "mood", "color_words"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                out.push(s.trim().to_string());
            }
        }
    }
    if let Some(tags) = v.get("tags").and_then(|x| x.as_array()) {
        let joined: Vec<&str> = tags.iter().filter_map(|t| t.as_str()).collect();
        if !joined.is_empty() {
            out.push(joined.join(", "));
        }
    }
    out.join(". ")
}

/// All OCR text the item's media carries, joined. Video frames and screenshots
/// both land in `media.ocr_text`.
fn item_ocr(conn: &Connection, item_id: &str) -> String {
    conn.query_row(
        "SELECT COALESCE(GROUP_CONCAT(ocr_text, ' '), '') FROM media \
         WHERE item_id=? AND COALESCE(ocr_text,'') != ''",
        [item_id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_default()
}

fn explainer_digest_for(conn: &Connection, item_id: &str) -> String {
    conn.query_row(
        "SELECT data FROM explanations WHERE item_id=? AND status='done'",
        [item_id],
        |r| r.get::<_, String>(0),
    )
    .map(|d| explainer_digest(&d))
    .unwrap_or_default()
}

/// True when the media is a phone screenshot rather than a scene: the file or
/// the title says so, or the first frame is far taller than it is wide.
fn is_screenshot(conn: &Connection, item_id: &str, title: &str) -> bool {
    if title.to_lowercase().contains("screenshot") {
        return true;
    }
    let named: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM media WHERE item_id=? \
             AND (lower(COALESCE(original_name,'')) LIKE '%screenshot%' \
                  OR lower(COALESCE(file,'')) LIKE '%screenshot%')",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if named > 0 {
        return true;
    }
    let ratio: f64 = conn
        .query_row(
            "SELECT COALESCE(height * 1.0 / NULLIF(width,0), 0) FROM media \
             WHERE item_id=? ORDER BY rowid LIMIT 1",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    (1.9..=4.0).contains(&ratio)
}

/// Weighted sum of the two towers, normalized. Screenshots lean on words.
fn blend(img: Option<Vec<f32>>, txt: Option<Vec<f32>>, shot: bool) -> Option<Vec<f32>> {
    let (iw, tw) = if shot { (SHOT_IMG_W, SHOT_TEXT_W) } else { (IMG_W, TEXT_W) };
    match (img, txt) {
        (Some(mut a), Some(b)) => {
            for (x, y) in a.iter_mut().zip(b.iter()) {
                *x = *x * iw + *y * tw;
            }
            if normalize(&mut a) {
                Some(a)
            } else {
                Some(b)
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// One vector per item: pictures and words, weighted, in the same 512-d space.
pub fn fused_vec(conn: &Connection, item_id: &str) -> Option<Vec<f32>> {
    let (title, text): (String, String) = conn
        .query_row("SELECT title, text FROM items WHERE id=?", [item_id], |r| {
            Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
        })
        .unwrap_or_default();
    let shot = is_screenshot(conn, item_id, &title);
    let digest = explainer_digest_for(conn, item_id);
    let ocr = item_ocr(conn, item_id);
    let img = media_mean(conn, item_id);
    let txt = item_text(&title, &text, &digest, &ocr).and_then(|t| text_vec(&t));
    blend(img, txt, shot)
}

fn fuse_sig(conn: &Connection, item_id: &str) -> String {
    let (tl, tx, ex): (i64, i64, i64) = conn
        .query_row(
            "SELECT COALESCE(LENGTH(i.title),0), COALESCE(LENGTH(i.text),0), \
             COALESCE(LENGTH(e.data),0) FROM items i \
             LEFT JOIN explanations e ON e.item_id=i.id AND e.status='done' \
             WHERE i.id=?",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or((0, 0, 0));
    let ol: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(ocr_text)),0) FROM media WHERE item_id=?",
            [item_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let (mc, mr): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(rowid),0) FROM media WHERE item_id=?",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    format!("{FUSE_V}:{tl}:{tx}:{ex}:{ol}:{mc}:{mr}")
}

/// Rewrite item_embeds with the fused vector. `limit` caps the items touched
/// per call (0 = all); the text tower runs in batches, because one ONNX call
/// per item costs about 100 ms.
/// Returns (fused, skipped, failed, remaining).
pub fn backfill_fused(conn: &Connection, limit: usize, force: bool) -> (usize, usize, usize, usize) {
    ensure_tables(conn);
    struct Cand {
        id: String,
        sig: String,
        title: String,
        text: String,
        digest: String,
        ocr: String,
        shot: bool,
    }
    // sig and prev sig both come from SQL, so the scan is one pass. The sig
    // carries the explainer length: a finished explainer is new evidence, and
    // the item must re-fuse or its screenshot stays wordless.
    let mut pending: Vec<Cand> = Vec::new();
    let mut skipped = 0usize;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT i.id, COALESCE(i.title,''), COALESCE(i.text,''), COALESCE(e.data,''), \
         COALESCE((SELECT GROUP_CONCAT(m.ocr_text,' ') FROM media m \
                   WHERE m.item_id=i.id AND COALESCE(m.ocr_text,'')!=''),''), \
         COALESCE(LENGTH(i.title),0) || ':' || COALESCE(LENGTH(i.text),0) || ':' || \
         COALESCE(LENGTH(e.data),0) || ':' || \
         COALESCE((SELECT SUM(LENGTH(m.ocr_text)) FROM media m WHERE m.item_id=i.id),0) || ':' || \
         (SELECT COUNT(*) FROM media m WHERE m.item_id=i.id) || ':' || \
         (SELECT COALESCE(MAX(rowid),0) FROM media m WHERE m.item_id=i.id) AS sig, \
         COALESCE(f.sig,'') AS prev \
         FROM items i \
         LEFT JOIN explanations e ON e.item_id=i.id AND e.status='done' \
         LEFT JOIN item_fuse f ON f.item_id=i.id \
         WHERE (i.deleted_at IS NULL OR i.deleted_at='') ORDER BY i.rowid",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        }) {
            for (id, title, text, data, ocr, sig, prev) in rows.flatten() {
                let sig = format!("{FUSE_V}:{sig}");
                if !force && sig == prev {
                    skipped += 1;
                    continue;
                }
                let shot = is_screenshot(conn, &id, &title);
                pending.push(Cand {
                    id,
                    sig,
                    title,
                    text,
                    digest: explainer_digest(&data),
                    ocr,
                    shot,
                });
            }
        }
    }
    let total_pending = pending.len();
    if limit > 0 && pending.len() > limit {
        pending.truncate(limit);
    }
    let remaining = total_pending - pending.len();

    // 1. image side: stored media vectors, normalized mean
    let mut imgs: Vec<Option<Vec<f32>>> = Vec::with_capacity(pending.len());
    for c in &pending {
        imgs.push(media_mean(conn, &c.id));
    }
    // 2. text side: one batched ONNX call per 32 items
    let mut txts: Vec<Option<Vec<f32>>> = vec![None; pending.len()];
    let mut slots: Vec<usize> = Vec::new();
    let mut batch: Vec<String> = Vec::new();
    let flush = |slots: &mut Vec<usize>, batch: &mut Vec<String>, txts: &mut Vec<Option<Vec<f32>>>| {
        if batch.is_empty() {
            return;
        }
        if let Ok(mut vecs) = crate::embed::embed_text(batch) {
            for (slot, v) in slots.iter().copied().zip(vecs.drain(..)) {
                let mut v = v;
                if normalize(&mut v) {
                    txts[slot] = Some(v);
                }
            }
        }
        slots.clear();
        batch.clear();
    };
    for (i, c) in pending.iter().enumerate() {
        let joined = item_text(&c.title, &c.text, &c.digest, &c.ocr);
        match joined {
            Some(t) => {
                slots.push(i);
                batch.push(t);
                if batch.len() >= 32 {
                    flush(&mut slots, &mut batch, &mut txts);
                }
            }
            None => {}
        }
    }
    flush(&mut slots, &mut batch, &mut txts);

    // 3. blend and store
    let (mut fused, mut failed) = (0usize, 0usize);
    for (i, c) in pending.iter().enumerate() {
        let img = imgs[i].take();
        let txt = txts[i].take();
        let vec = blend(img, txt, c.shot);
        match vec {
            Some(v) => {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO item_embeds(item_id, vec) VALUES(?,?)",
                    params![c.id, vec_to_blob(&v)],
                );
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO item_fuse(item_id, sig, fused_at) VALUES(?,?,datetime('now'))",
                    params![c.id, c.sig],
                );
                fused += 1;
            }
            None => {
                // Nothing to say about this item yet (no media, no words). Drop
                // any stale vector, or it would keep voting in the fit, and
                // record the sig so the next boot does not rescan it forever.
                let _ = conn.execute("DELETE FROM item_embeds WHERE item_id=?", params![c.id]);
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO item_fuse(item_id, sig, fused_at) VALUES(?,?,datetime('now'))",
                    params![c.id, c.sig],
                );
                failed += 1;
            }
        }
    }
    if fused > 0 {
        crate::search::invalidate_index();
    }
    (fused, skipped, failed, remaining)
}

/// OCR screenshot media that has none yet, one bounded batch per boot.
///
/// A screenshot's pixels say "phone UI"; its words live in the image. The fuse
/// signature carries the OCR length, so the next pass re-fuses these items with
/// real content. A media row that OCRs to nothing stores a single space, so it
/// is not retried on every boot.
pub fn ocr_screenshots(conn: &Connection, limit: usize) -> usize {
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT m.id, m.file FROM media m JOIN items i ON i.id=m.item_id \
         WHERE m.kind='image' AND COALESCE(m.file,'')!='' \
           AND COALESCE(m.ocr_text,'')='' \
           AND (lower(COALESCE(i.title,'')) LIKE '%screenshot%' \
                OR lower(COALESCE(m.original_name,'')) LIKE '%screenshot%' \
                OR lower(COALESCE(i.title,'')) LIKE '%screen%' \
                OR m.height * 1.0 / NULLIF(m.width,0) >= 1.9) \
           AND (i.deleted_at IS NULL OR i.deleted_at='') \
         ORDER BY m.rowid LIMIT ?1",
    ) {
        if let Ok(mapped) = stmt.query_map([limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            rows = mapped.flatten().collect();
        }
    }
    if rows.is_empty() {
        return 0;
    }
    let dir = crate::mediaio::media_dir();
    let mut done = 0usize;
    for (id, file) in rows {
        let text = crate::mediaio::image_ocr(&dir.join(&file), 80);
        let stored = if text.trim().is_empty() { " ".to_string() } else { text };
        if conn
            .execute("UPDATE media SET ocr_text=? WHERE id=?", params![stored, id])
            .is_ok()
        {
            done += 1;
        }
    }
    done
}

// ---------------------------------------------------------------- k-means

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

fn load_matrix(conn: &Connection) -> (Vec<String>, Vec<f32>) {
    let mut ids: Vec<String> = Vec::new();
    let mut mat: Vec<f32> = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT e.item_id, e.vec FROM item_embeds e JOIN items i ON i.id=e.item_id \
         AND (i.deleted_at IS NULL OR i.deleted_at='') ORDER BY i.rowid",
    ) else {
        return (ids, mat);
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))) {
        for (id, blob) in rows.flatten() {
            if let Some(mut v) = blob_to_vec(&blob) {
                if normalize(&mut v) {
                    ids.push(id);
                    mat.extend(v);
                }
            }
        }
    }
    (ids, mat)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut s = 0.0f32;
    let mut i = 0;
    while i + 8 <= a.len() {
        s += a[i] * b[i]
            + a[i + 1] * b[i + 1]
            + a[i + 2] * b[i + 2]
            + a[i + 3] * b[i + 3]
            + a[i + 4] * b[i + 4]
            + a[i + 5] * b[i + 5]
            + a[i + 6] * b[i + 6]
            + a[i + 7] * b[i + 7];
        i += 8;
    }
    while i < a.len() {
        s += a[i] * b[i];
        i += 1;
    }
    s
}

/// Spherical k-means: assigned by cosine, centroids renormalized each round.
/// Returns (labels, scores, centroids, iterations).
fn kmeans(mat: &[f32], n: usize, k: usize, seed: u64)
    -> (Vec<i64>, Vec<f32>, Vec<f32>, usize, Vec<u64>) {
    let mut rng = Lcg(seed);
    let mut centroids = vec![0.0f32; k * DIM];
    let mut picked: Vec<usize> = Vec::new();
    let mut guard = 0;
    while picked.len() < k.min(n) && guard < k * 64 {
        guard += 1;
        let idx = rng.below(n);
        if picked.contains(&idx) {
            continue;
        }
        let slot = picked.len();
        picked.push(idx);
        centroids[slot * DIM..(slot + 1) * DIM]
            .copy_from_slice(&mat[idx * DIM..(idx + 1) * DIM]);
    }
    while picked.len() < k {
        // fewer points than clusters: duplicate the last centroid
        let src = picked[0] * DIM;
        let at = picked.len() * DIM;
        let chunk: Vec<f32> = centroids[src..src + DIM].to_vec();
        centroids[at..at + DIM].copy_from_slice(&chunk);
        picked.push(picked[0]);
    }

    let threads = std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let mut labels = vec![-1i64; n];
    let mut scores = vec![0.0f32; n];
    let mut iters = 0usize;
    let iter_ms = std::sync::Mutex::new(Vec::<u64>::new());

    for it in 0..MAX_ITERS {
        iters = it + 1;
        let it_started = std::time::Instant::now();
        let chunk = (n + threads - 1) / threads.max(1);
        let changed = std::thread::scope(|scope| {
            // chunks_mut hands out disjoint slices, so each worker owns its range
            let lab_chunks: Vec<&mut [i64]> = labels.chunks_mut(chunk).collect();
            let sc_chunks: Vec<&mut [f32]> = scores.chunks_mut(chunk).collect();
            let mut handles = Vec::new();
            for (t, (lab_slice, sc_slice)) in lab_chunks.into_iter().zip(sc_chunks).enumerate() {
                let lo = t * chunk;
                let mat_ref: &[f32] = mat;
                let cent_ref: &[f32] = &centroids;
                handles.push(scope.spawn(move || {
                    let mut local = 0usize;
                    for (slot, lab) in lab_slice.iter_mut().enumerate() {
                        let i = lo + slot;
                        let v = &mat_ref[i * DIM..(i + 1) * DIM];
                        let mut best = 0usize;
                        let mut best_score = f32::MIN;
                        for c in 0..k {
                            let s = dot(v, &cent_ref[c * DIM..(c + 1) * DIM]);
                            if s > best_score {
                                best_score = s;
                                best = c;
                            }
                        }
                        if *lab != best as i64 {
                            local += 1;
                        }
                        *lab = best as i64;
                        sc_slice[slot] = best_score;
                    }
                    local
                }));
            }
            handles.into_iter().map(|h| h.join().unwrap_or(0)).sum::<usize>()
        });

        // centroid update: per-thread partial sums, then one reduction
        let parts = std::thread::scope(|scope| {
            let chunk = (n + threads - 1) / threads.max(1);
            let mut handles = Vec::new();
            for t in 0..threads {
                let lo = t * chunk;
                let hi = ((t + 1) * chunk).min(n);
                if lo >= hi {
                    continue;
                }
                let mat_ref: &[f32] = mat;
                let lab_ref: &[i64] = &labels;
                handles.push(scope.spawn(move || {
                    let mut sums = vec![0.0f32; k * DIM];
                    let mut counts = vec![0u32; k];
                    for i in lo..hi {
                        let c = lab_ref[i] as usize;
                        if c >= k {
                            continue;
                        }
                        counts[c] += 1;
                        let v = &mat_ref[i * DIM..(i + 1) * DIM];
                        let s = &mut sums[c * DIM..(c + 1) * DIM];
                        for (a, b) in s.iter_mut().zip(v.iter()) {
                            *a += *b;
                        }
                    }
                    (sums, counts)
                }));
            }
            handles.into_iter().filter_map(|h| h.join().ok()).collect::<Vec<_>>()
        });

        let mut sums = vec![0.0f32; k * DIM];
        let mut counts = vec![0u32; k];
        for (s, c) in parts {
            for (dst, src) in sums.iter_mut().zip(s.iter()) {
                *dst += *src;
            }
            for (dst, src) in counts.iter_mut().zip(c.iter()) {
                *dst += *src;
            }
        }
        for c in 0..k {
            if counts[c] == 0 {
                // reseed an empty cluster on a random point so k stays honest
                let idx = rng.below(n);
                let src = idx * DIM;
                let chunk: Vec<f32> = mat[src..src + DIM].to_vec();
                centroids[c * DIM..(c + 1) * DIM].copy_from_slice(&chunk);
                continue;
            }
            let nf = counts[c] as f32;
            let mut cent = sums[c * DIM..(c + 1) * DIM].to_vec();
            for x in cent.iter_mut() {
                *x /= nf;
            }
            if !normalize(&mut cent) {
                let idx = rng.below(n);
                let chunk: Vec<f32> = mat[idx * DIM..(idx + 1) * DIM].to_vec();
                centroids[c * DIM..(c + 1) * DIM].copy_from_slice(&chunk);
                continue;
            }
            centroids[c * DIM..(c + 1) * DIM].copy_from_slice(&cent);
        }

        iter_ms.lock().map(|mut v| v.push(it_started.elapsed().as_millis() as u64)).ok();
        if changed * 400 < n {
            break;
        }
    }
    (labels, scores, centroids, iters, iter_ms.into_inner().unwrap_or_default())
}

// ---------------------------------------------------------------- labels

fn title_words(conn: &Connection) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT id, title FROM items WHERE (deleted_at IS NULL OR deleted_at='')") else {
        return out;
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))) {
        for (id, title) in rows.flatten() {
            let mut words = Vec::new();
            for raw in title.unwrap_or_default().split(|c: char| !c.is_alphanumeric() && c != '-') {
                let w = raw.trim().to_lowercase();
                if w.len() < 3 || STOP_WORDS.contains(&w.as_str()) {
                    continue;
                }
                if w.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                words.push(w);
                if words.len() >= 12 {
                    break;
                }
            }
            out.insert(id, words);
        }
    }
    out
}

fn tags_by_item(conn: &Connection) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT item_id, tag FROM tags") else {
        return out;
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
        for (id, tag) in rows.flatten() {
            let t = tag.trim().to_lowercase();
            // batch-import tags and author handles say nothing about a group
            if t.is_empty()
                || GENERIC_TAGS.contains(&t.as_str())
                || t.starts_with("import")
                || t.starts_with('_')
                || t.chars().all(|c| !c.is_alphabetic())
            {
                continue;
            }
            out.entry(id).or_default().push(t);
        }
    }
    out
}

fn hue_by_item(conn: &Connection) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT item_id, hue, COUNT(*) AS c FROM media WHERE hue!='' GROUP BY item_id, hue ORDER BY item_id, c DESC",
    ) else {
        return out;
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
        for (id, hue) in rows.flatten() {
            out.entry(id).or_insert(hue);
        }
    }
    out
}

fn kind_by_item(conn: &Connection) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT id, kind FROM items") else {
        return out;
    };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))) {
        for (id, kind) in rows.flatten() {
            out.insert(id, kind.unwrap_or_default());
        }
    }
    out
}

/// A short human label for each cluster, built from what the members share.
fn build_labels(
    conn: &Connection,
    ids: &[String],
    labels: &[i64],
    k: usize,
) -> Vec<(String, String, Value)> {
    let tags = tags_by_item(conn);
    let words = title_words(conn);
    let hues = hue_by_item(conn);
    let kinds = kind_by_item(conn);
    let mut tag_counts = vec![HashMap::<String, u32>::new(); k];
    let mut word_counts = vec![HashMap::<String, u32>::new(); k];
    let mut hue_counts = vec![HashMap::<String, u32>::new(); k];
    let mut kind_counts = vec![HashMap::<String, u32>::new(); k];
    for (i, id) in ids.iter().enumerate() {
        let c = labels[i];
        if c < 0 || c as usize >= k {
            continue;
        }
        let c = c as usize;
        for t in tags.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
            *tag_counts[c].entry(t.clone()).or_insert(0) += 1;
        }
        for w in words.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
            *word_counts[c].entry(w.clone()).or_insert(0) += 1;
        }
        if let Some(h) = hues.get(id) {
            *hue_counts[c].entry(h.clone()).or_insert(0) += 1;
        }
        let kk = kinds.get(id).cloned().unwrap_or_default();
        *kind_counts[c].entry(kk).or_insert(0) += 1;
    }
    let mut out = Vec::with_capacity(k);
    for c in 0..k {
        let size: u32 = kind_counts[c].values().sum();
        let tag = top_n(&tag_counts[c], 2, if size < 6 { 1 } else { 2 });
        let words = if tag.len() < 2 {
            top_n(&word_counts[c], if size < 6 { 2 } else { 3 }, 1)
        } else {
            Vec::new()
        };
        let hue = top_n(&hue_counts[c], 1, 1).into_iter().next().unwrap_or_default();
        let kinds_json = json!(kind_counts[c]
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect::<Map<String, Value>>());
        let mut parts: Vec<String> = tag;
        for w in words {
            if !parts.iter().any(|p| p == &w) {
                parts.push(w);
            }
        }
        if parts.is_empty() {
            let dom = top_n(&kind_counts[c], 1, 1)
                .into_iter()
                .next()
                .unwrap_or_else(|| "items".into());
            parts.push(format!("{dom} group"));
        }
        let label = if hue.is_empty() {
            parts.join(" · ")
        } else {
            format!("{} · {}", parts.join(" · "), hue)
        };
        out.push((label, hue, kinds_json));
    }
    out
}

fn top_n(counts: &HashMap<String, u32>, n: usize, min_count: u32) -> Vec<String> {
    let mut v: Vec<(String, u32)> = counts
        .iter()
        .filter(|(_, c)| **c >= min_count)
        .map(|(k, c)| (k.clone(), *c))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().take(n).map(|(k, _)| k).collect()
}

// ---------------------------------------------------------------- fit / read

pub fn default_k(n: usize) -> usize {
    if n < 40 {
        return 1.max(n / 4);
    }
    (n / 13).clamp(48, 512)
}

/// Run k-means and persist the result. Returns a summary dict.
pub fn fit(conn: &Connection, k_opt: usize) -> Value {
    ensure_tables(conn);
    let started = std::time::Instant::now();
    let (ids, mat) = load_matrix(conn);
    let n = ids.len();
    if n < 8 {
        return json!({"ok": false, "error": "not enough embedded items", "n": n});
    }
    let k = if k_opt == 0 { default_k(n) } else { k_opt.clamp(1, n) };
    let seed = 0x5DEECE66D_u64 ^ (n as u64) << 8;
    let t_load = started.elapsed().as_millis() as i64;
    let (labels, scores, _centroids, iters, iter_ms) = kmeans(&mat, n, k, seed);
    let t_kmeans = started.elapsed().as_millis() as i64 - t_load;
    let metas = build_labels(conn, &ids, &labels, k);
    let t_labels = started.elapsed().as_millis() as i64 - t_load - t_kmeans;
    let now = now_string(conn);

    let mut sizes = vec![0i64; k];
    let mut coh = vec![0f64; k];
    for i in 0..n {
        let c = labels[i] as usize;
        if c < k {
            sizes[c] += 1;
            coh[c] += scores[i] as f64;
        }
    }

    let Ok(tx) = conn.unchecked_transaction() else {
        return json!({"ok": false, "error": "transaction failed"});
    };
    let _ = tx.execute("DELETE FROM item_clusters", []);
    let _ = tx.execute("DELETE FROM cluster_meta", []);
    {
        let mut ins = tx.prepare("INSERT OR REPLACE INTO item_clusters(item_id, cluster_id, score) VALUES(?,?,?)").ok();
        if let Some(stmt) = ins.as_mut() {
            for i in 0..n {
                let _ = stmt.execute(params![ids[i], labels[i], scores[i] as f64]);
            }
        }
    }
    {
        let mut meta = tx
            .prepare("INSERT OR REPLACE INTO cluster_meta(cluster_id, label, size, cohesion, hue, kinds, fitted_at) VALUES(?,?,?,?,?,?,?)")
            .ok();
        if let Some(stmt) = meta.as_mut() {
            for c in 0..k {
                let (label, hue, kinds) = metas.get(c).cloned().unwrap_or((String::new(), String::new(), json!({})));
                let mean = if sizes[c] > 0 { coh[c] / sizes[c] as f64 } else { 0.0 };
                let _ = stmt.execute(params![
                    c as i64,
                    label,
                    sizes[c],
                    (mean * 1000.0).round() / 1000.0,
                    hue,
                    kinds.to_string(),
                    now
                ]);
            }
        }
    }
    let ms = started.elapsed().as_millis() as i64;
    let _ = tx.execute(
        "INSERT INTO cluster_runs(k, n, iters, ms, fitted_at) VALUES(?,?,?,?,?)",
        params![k as i64, n as i64, iters as i64, ms, now],
    );
    let _ = tx.commit();
    let live = sizes.iter().filter(|s| **s > 0).count();
    json!({"ok": true, "k": k, "n": n, "clusters": live, "iters": iters, "ms": ms,
           "load_ms": t_load, "kmeans_ms": t_kmeans, "labels_ms": t_labels,
           "write_ms": ms - t_load - t_kmeans - t_labels, "iter_ms": iter_ms,
           "fitted_at": now})
}

/// UTC timestamp from sqlite, so this module needs no chrono dependency.
fn now_string(conn: &Connection) -> String {
    conn.query_row("SELECT datetime('now')", [], |r| r.get::<_, String>(0))
        .unwrap_or_default()
}

fn covers_for(conn: &Connection, cluster_id: i64, want: usize) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT c.item_id FROM item_clusters c JOIN media m ON m.item_id=c.item_id \
         WHERE c.cluster_id=? AND m.placeholder=0 AND m.thumb!='' \
         GROUP BY c.item_id ORDER BY MAX(c.score) DESC LIMIT ?",
    ) else {
        return out;
    };
    if let Ok(rows) = stmt.query_map(params![cluster_id, want as i64], |r| r.get::<_, String>(0)) {
        for id in rows.flatten() {
            out.push(id);
        }
    }
    if out.is_empty() {
        // text-only group: hand back the top members so the tile is not blank
        if let Ok(mut stmt) =
            conn.prepare("SELECT item_id FROM item_clusters WHERE cluster_id=? ORDER BY score DESC LIMIT ?")
        {
            if let Ok(rows) =
                stmt.query_map(params![cluster_id, want as i64], |r| r.get::<_, String>(0))
            {
                for id in rows.flatten() {
                    out.push(id);
                }
            }
        }
    }
    out
}

fn meta_dict(conn: &Connection, cluster_id: i64) -> Option<Value> {
    conn.query_row(
        "SELECT cluster_id, label, size, cohesion, hue, kinds, fitted_at FROM cluster_meta WHERE cluster_id=?",
        [cluster_id],
        |r| {
            let kinds_raw: String = r.get(5).unwrap_or_else(|_| "{}".into());
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "label": r.get::<_, String>(1).unwrap_or_default(),
                "size": r.get::<_, i64>(2).unwrap_or(0),
                "cohesion": r.get::<_, f64>(3).unwrap_or(0.0),
                "hue": r.get::<_, String>(4).unwrap_or_default(),
                "kinds": serde_json::from_str::<Value>(&kinds_raw).unwrap_or(json!({})),
                "fitted_at": r.get::<_, String>(6).unwrap_or_default(),
            }))
        },
    )
    .ok()
}

fn last_run(conn: &Connection) -> Value {
    conn.query_row(
        "SELECT k, n, iters, ms, fitted_at FROM cluster_runs ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            Ok(json!({
                "k": r.get::<_, i64>(0).unwrap_or(0),
                "n": r.get::<_, i64>(1).unwrap_or(0),
                "iters": r.get::<_, i64>(2).unwrap_or(0),
                "ms": r.get::<_, i64>(3).unwrap_or(0),
                "fitted_at": r.get::<_, String>(4).unwrap_or_default(),
            }))
        },
    )
    .unwrap_or(json!(null))
}

/// Cluster list, biggest first. Each entry carries up to 4 cover item ids.
pub fn list(conn: &Connection, min_size: i64, limit: i64) -> Value {
    ensure_tables(conn);
    let mut clusters = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT cluster_id FROM cluster_meta WHERE size>=? ORDER BY size DESC, cluster_id ASC LIMIT ?",
    ) else {
        return json!({"clusters": [], "run": null});
    };
    if let Ok(rows) = stmt.query_map(params![min_size.max(1), limit.clamp(1, 400)], |r| r.get::<_, i64>(0)) {
        for id in rows.flatten() {
            if let Some(mut d) = meta_dict(conn, id) {
                let covers = covers_for(conn, id, 4);
                if let Some(obj) = d.as_object_mut() {
                    obj.insert("covers".into(), json!(covers));
                }
                clusters.push(d);
            }
        }
    }
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM cluster_meta WHERE size>=?", params![min_size.max(1)], |r| r.get(0))
        .unwrap_or(0);
    json!({"clusters": clusters, "total": total, "run": last_run(conn)})
}

/// The same item filters the posts grid uses: kind, source, hue, tag.
#[derive(Default, Clone, Debug)]
pub struct GroupFilter {
    pub kind: String,
    pub source: String,
    pub hue: String,
    pub tag: String,
}

impl GroupFilter {
    pub fn is_empty(&self) -> bool {
        self.kind.is_empty() && self.source.is_empty() && self.hue.is_empty() && self.tag.is_empty()
    }
    /// Read the filters from a query map (REST) or a JSON body (MCP).
    pub fn from_query(q: &std::collections::HashMap<String, String>) -> Self {
        let get = |k: &str| q.get(k).cloned().unwrap_or_default();
        GroupFilter { kind: get("kind"), source: get("source"), hue: get("hue"), tag: get("tag") }
    }
    pub fn from_json(v: &Value) -> Self {
        let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        GroupFilter { kind: get("kind"), source: get("source"), hue: get("hue"), tag: get("tag") }
    }
}

/// Ranked members of a cluster, best match first, near-duplicates folded.
fn ranked_members(conn: &Connection, cluster_id: i64, cap: i64) -> Vec<(String, f64)> {
    let mut ranked: Vec<(String, f64)> = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT c.item_id, c.score FROM item_clusters c JOIN items i ON i.id=c.item_id \
         AND (i.deleted_at IS NULL OR i.deleted_at='') WHERE c.cluster_id=? \
         ORDER BY c.score DESC LIMIT ?",
    ) else {
        return ranked;
    };
    if let Ok(rows) = stmt.query_map(params![cluster_id, cap], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    }) {
        let mut seen: HashSet<String> = HashSet::new();
        for (id, score) in rows.flatten() {
            let keys = crate::search::similar_keys(conn, &id);
            if !keys.is_empty() && seen.intersection(&keys).next().is_some() {
                continue;
            }
            seen.extend(keys);
            ranked.push((id, score));
        }
    }
    ranked
}

/// Filter, count, and hydrate. `matched` is the number that passed the filter.
fn hydrate(
    conn: &Connection,
    ranked: Vec<(String, f64)>,
    limit: i64,
    f: &GroupFilter,
) -> (Vec<Value>, i64) {
    let filtered = if f.is_empty() {
        ranked
    } else {
        crate::search::filter_ranked_pub(
            conn, ranked, &f.kind, &f.source, &f.hue, &f.tag, 500,
        )
    };
    let matched = filtered.len() as i64;
    let page: Vec<(String, f64)> = filtered.into_iter().take(limit.max(1) as usize).collect();
    (crate::search::entries_from_ranked(conn, page), matched)
}

/// Members of one cluster as hydrated cards, best match first, near-dupes folded.
pub fn entries(conn: &Connection, cluster_id: i64, limit: i64, f: &GroupFilter) -> Vec<Value> {
    let limit = limit.clamp(1, 200);
    let ranked = ranked_members(conn, cluster_id, 600);
    hydrate(conn, ranked, limit, f).0
}

fn pick_cluster(conn: &Connection, min_size: i64, exclude: &[i64]) -> Option<i64> {
    let mut sql = String::from(
        "SELECT m.cluster_id FROM cluster_meta m LEFT JOIN cluster_seen s ON s.cluster_id=m.cluster_id \
         WHERE m.size>=?",
    );
    if !exclude.is_empty() {
        sql.push_str(&format!(
            " AND m.cluster_id NOT IN ({})",
            exclude.iter().map(|_| "?").collect::<Vec<_>>().join(",")
        ));
    }
    sql.push_str(" ORDER BY COALESCE(s.seen_at,'') ASC, RANDOM() LIMIT 1");
    let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(min_size.max(1))];
    for id in exclude {
        params_dyn.push(Box::new(*id));
    }
    let params_ref: Vec<&dyn rusqlite::ToSql> = params_dyn.iter().map(|p| p.as_ref()).collect();
    conn.query_row(&sql, params_ref.as_slice(), |r| r.get(0)).ok()
}

/// Pick a cluster that CONTAINS items matching the filter, in one SQL statement.
/// Sampling whole clusters at random misses narrow combinations, and ranking
/// purely by match count returns the same cluster every time, so this keeps the
/// 24 densest candidates and rolls one of those.
fn pick_cluster_filtered(
    conn: &Connection,
    min_size: i64,
    exclude: &[i64],
    f: &GroupFilter,
) -> Option<i64> {
    let mut inner = String::from(
        "SELECT c.cluster_id AS cid, COUNT(*) AS n FROM item_clusters c \
         JOIN cluster_meta cm ON cm.cluster_id=c.cluster_id AND cm.size>=? \
         JOIN items i ON i.id=c.item_id AND (i.deleted_at IS NULL OR i.deleted_at='') \
         LEFT JOIN media m ON m.rowid=(SELECT MIN(rowid) FROM media WHERE item_id=i.id) \
         WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();
    if !f.kind.is_empty() {
        inner.push_str(" AND i.kind=?");
        binds.push(f.kind.clone());
    }
    if !f.source.is_empty() {
        inner.push_str(" AND i.source=?");
        binds.push(f.source.clone());
    }
    if !f.hue.is_empty() {
        inner.push_str(" AND m.hue=?");
        binds.push(f.hue.clone());
    }
    if !f.tag.is_empty() {
        inner.push_str(" AND EXISTS (SELECT 1 FROM tags t WHERE t.item_id=i.id AND t.tag=?)");
        binds.push(f.tag.to_lowercase());
    }
    inner.push_str(" GROUP BY c.cluster_id ORDER BY n DESC LIMIT 24");

    let not_in = if exclude.is_empty() {
        String::new()
    } else {
        format!(
            " AND cid NOT IN ({})",
            exclude.iter().map(|_| "?").collect::<Vec<_>>().join(",")
        )
    };
    let pick = |sql: &str, extra: &[i64]| -> Option<i64> {
        let mut dyn_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(min_size.max(1))];
        for b in &binds {
            dyn_params.push(Box::new(b.clone()));
        }
        for id in extra {
            dyn_params.push(Box::new(*id));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = dyn_params.iter().map(|p| p.as_ref()).collect();
        conn.query_row(sql, refs.as_slice(), |r| r.get(0)).ok()
    };
    let preferred = format!("SELECT cid FROM ({inner}){not_in} ORDER BY RANDOM() LIMIT 1");
    if let Some(id) = pick(&preferred, exclude) {
        return Some(id);
    }
    // every dense candidate was already shown: allow a repeat
    let any = format!("SELECT cid FROM ({inner}) ORDER BY RANDOM() LIMIT 1");
    pick(&any, &[])
}

/// One cluster the caller has not seen lately, with its cards.
/// With filters on, a cluster that would come back empty is skipped, so a
/// shuffle never hands back a blank grid.
pub fn random(
    conn: &Connection,
    min_size: i64,
    exclude: &[i64],
    limit: i64,
    f: &GroupFilter,
) -> Option<Value> {
    ensure_tables(conn);
    if !f.is_empty() {
        if let Some(cluster_id) = pick_cluster_filtered(conn, min_size, exclude, f) {
            let ranked = ranked_members(conn, cluster_id, 600);
            let (items, matched) = hydrate(conn, ranked, limit, f);
            let _ = conn.execute(
                "INSERT INTO cluster_seen(cluster_id, views, seen_at) VALUES(?,1,?) \
                 ON CONFLICT(cluster_id) DO UPDATE SET views=views+1, seen_at=excluded.seen_at",
                params![cluster_id, now_string(conn)],
            );
            let mut meta = meta_dict(conn, cluster_id)?;
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("matched".into(), json!(matched));
            }
            return Some(json!({"cluster": meta, "items": items}));
        }
        return None;
    }
    let mut tried: Vec<i64> = exclude.to_vec();
    for _ in 0..12 {
        let cluster_id = match pick_cluster(conn, min_size, &tried).or_else(|| {
            conn.query_row(
                "SELECT cluster_id FROM cluster_meta WHERE size>=? ORDER BY RANDOM() LIMIT 1",
                params![min_size.max(1)],
                |r| r.get::<_, i64>(0),
            )
            .ok()
        }) {
            Some(id) => id,
            None => return None,
        };
        let ranked = ranked_members(conn, cluster_id, 600);
        let (items, matched) = hydrate(conn, ranked, limit, f);
        if !f.is_empty() && items.is_empty() {
            // nothing here matches the filter: try another cluster
            if tried.contains(&cluster_id) {
                return None;
            }
            tried.push(cluster_id);
            continue;
        }
        let _ = conn.execute(
            "INSERT INTO cluster_seen(cluster_id, views, seen_at) VALUES(?,1,?) \
             ON CONFLICT(cluster_id) DO UPDATE SET views=views+1, seen_at=excluded.seen_at",
            params![cluster_id, now_string(conn)],
        );
        let mut meta = meta_dict(conn, cluster_id)?;
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("matched".into(), json!(matched));
        }
        return Some(json!({"cluster": meta, "items": items}));
    }
    None
}

/// Cluster detail by id, filtered.
pub fn get(conn: &Connection, cluster_id: i64, limit: i64, f: &GroupFilter) -> Option<Value> {
    ensure_tables(conn);
    let mut meta = meta_dict(conn, cluster_id)?;
    let ranked = ranked_members(conn, cluster_id, 600);
    let (items, matched) = hydrate(conn, ranked, limit, f);
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("matched".into(), json!(matched));
    }
    Some(json!({"cluster": meta, "items": items}))
}

/// Boot hook: create the tables, then fit in the background only when there is
/// no fit yet or the item count moved by more than 10%.
pub fn start_fitter(db_path: String) {
    let _ = std::thread::Builder::new()
        .name("cluster-fit".into())
        .spawn(move || {
            let Ok(conn) = Connection::open(&db_path) else { return };
            let _ = conn.pragma_update(None, "foreign_keys", "ON");
            ensure_tables(&conn);
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM item_embeds e JOIN items i ON i.id=e.item_id \
                     AND (i.deleted_at IS NULL OR i.deleted_at='')",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let last_n: i64 = conn
                .query_row("SELECT COALESCE(MAX(n),0) FROM cluster_runs", [], |r| r.get(0))
                .unwrap_or(0);
            // Screenshots first: their OCR is the only content some items have,
            // and fusion reads it (one bounded batch per boot).
            let ocr = ocr_screenshots(&conn, 800);
            if ocr > 0 {
                eprintln!("taste-engine-rs: OCR'd {ocr} screenshots");
            }
            // One-time migration: give every item its fused (image+text) vector
            // before clustering. Steady state this skips everything.
            let mut migrated = 0usize;
            loop {
                let (fused, _skipped, _failed, remaining) = backfill_fused(&conn, 400, false);
                migrated += fused;
                if fused == 0 || remaining == 0 {
                    break;
                }
                eprintln!("taste-engine-rs: fused {fused} item vectors, {remaining} to go");
            }
            // A migration rewrites vectors that the last fit read, so always
            // refit after one. Without this a restart that migrated leaves the
            // groups built from the old vectors.
            let stale = migrated > 0 || last_n == 0 || (n - last_n).abs() * 10 > last_n;
            if stale && n >= 16 {
                let out = fit(&conn, 0);
                eprintln!("taste-engine-rs: groups fit {out}");
            }
        });
}

pub fn fitted(conn: &Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM cluster_meta", [], |r| r.get::<_, i64>(0))
        .map(|c| c > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(seed: i64) -> Vec<f32> {
        // deterministic pseudo-random unit vector, no rand dependency
        let mut v = vec![0.0f32; DIM];
        let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        for x in v.iter_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *x = ((s >> 33) as f32) / (1u32 << 31) as f32 - 1.0;
        }
        assert!(normalize(&mut v));
        v
    }

    #[test]
    fn normalize_gives_unit_length() {
        let v = unit(7);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5, "length {n}");
    }

    #[test]
    fn normalize_rejects_zero() {
        let mut zero = vec![0.0f32; DIM];
        assert!(!normalize(&mut zero));
    }

    #[test]
    fn dot_of_a_vector_with_itself_is_one() {
        let v = unit(11);
        assert!((dot(&v, &v) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn default_k_stays_in_range() {
        assert_eq!(default_k(10), 2);
        assert_eq!(default_k(400), 48);
        assert_eq!(default_k(5301), 407);
        assert_eq!(default_k(20000), 512);
    }

    #[test]
    fn kmeans_separates_two_obvious_groups() {
        // 24 points, two clusters along dim 0, k = 2
        let mut mat = Vec::new();
        let mut ids = Vec::new();
        let n = 24usize;
        for i in 0..n {
            let mut v = vec![0.0f32; DIM];
            v[0] = if i < n / 2 { 1.0 } else { -1.0 };
            v[1] = 0.05 * (i as f32);
            normalize(&mut v);
            ids.push(format!("i{i}"));
            mat.extend(v);
        }
        let (labels, _scores, _centroids, iters, _ms) = kmeans(&mat, n, 2, 42);
        assert!(iters >= 1);
        let first_half: std::collections::HashSet<i64> = labels[..n / 2].iter().copied().collect();
        let second_half: std::collections::HashSet<i64> = labels[n / 2..].iter().copied().collect();
        assert_eq!(first_half.len(), 1, "first group split");
        assert_eq!(second_half.len(), 1, "second group split");
        assert_ne!(first_half, second_half, "both groups collapsed into one");
        let _ = ids;
    }

    #[test]
    fn item_text_skips_blank() {
        assert!(item_text("", "  ", "", "").is_none());
        assert!(item_text("title", "", "", "").is_some());
        // clipped to MAX_TEXT characters
        let long = item_text(&"x".repeat(5000), "", "", "").unwrap();
        assert_eq!(long.chars().count(), MAX_TEXT);
    }

    #[test]
    fn import_titles_never_reach_the_text_tower() {
        // The exact strings a bulk import writes. On their own they say nothing,
        // and 100 of them embed to the same vector.
        assert!(generic_title("screenshot · Aug 05 2025 · twitter"));
        assert!(generic_title("Screenshot_20260914-001152.png"));
        assert!(generic_title("Image taken on May 24, 2025"));
        assert!(generic_title("saved image"));
        // A platform is not a subject: 56 rows titled "Instagram post" once
        // formed their own 48-item group on that string alone.
        assert!(generic_title("Instagram post"));
        assert!(generic_title("x post"));
        assert!(generic_title("twitter"));
        assert!(!generic_title("Dumbbell chest press form check"));
        assert!(!generic_title("Instagram model with a red jacket"));
        // A generic title alone leaves nothing to embed.
        assert!(item_text("screenshot · Aug 05 2025 · twitter", "", "", "").is_none());
        // But the explainer still carries the content.
        let t = item_text("screenshot · Aug 05 2025 · twitter", "", "A neon poster of a skull.", "").unwrap();
        assert!(t.contains("skull"));
        assert!(!t.contains("Aug 05"), "the import title leaked into the text: {t}");
    }

    #[test]
    fn explainer_digest_reads_the_content_keys() {
        let data = r#"{"summary":"A neon poster.","mood":"electric","style_dna":"glitch","color_words":"cyan, ink","tags":["poster","neon"],"visuals":"grey text"}"#;
        let d = explainer_digest(data);
        assert!(d.contains("A neon poster."));
        assert!(d.contains("poster, neon"));
        assert!(!d.contains("grey text"), "visuals is scene description, not topic");
        assert!(explainer_digest("not json").is_empty());
    }

    #[test]
    fn screenshots_lean_on_words_and_scenes_on_pixels() {
        // Two orthogonal towers: the blend must flip which one dominates.
        let mut img = vec![0.0f32; DIM];
        img[0] = 1.0;
        let mut txt = vec![0.0f32; DIM];
        txt[1] = 1.0;

        let scene = blend(Some(img.clone()), Some(txt.clone()), false).unwrap();
        assert!(scene[0] > scene[1], "a photo should follow its pixels");

        let shot = blend(Some(img), Some(txt), true).unwrap();
        assert!(shot[1] > shot[0], "a screenshot should follow its words");
    }
}
