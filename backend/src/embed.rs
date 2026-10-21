//! CLIP text embedding — CLIPTokenizer (BPE) port + ONNX runtime session via a
//! shared onnxruntime dylib loaded with `load-dynamic` (see `ORT_DYLIB_PATH`).
//! Mirrors sentence-transformers/clip-ViT-B-32: encode(text) -> 512-d float32,
//! already projected; callers normalize like embed.norm does.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use crate::lock::LockExt;

pub const EMBED_DIM: usize = 512;
const MAX_LEN: usize = 77;
const VISION_SIZE: usize = 224;

fn models_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TV_MODELS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from("./backend/models")
}

/// ONNX Runtime must be supplied by the environment. Set `ORT_DYLIB_PATH` to a
/// full library path, or `ORT_LIB_DIR` to the directory that holds it.
fn ort_dylib() -> String {
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        if !p.is_empty() {
            return p;
        }
    }
    if let Ok(dir) = std::env::var("ORT_LIB_DIR") {
        let dir = PathBuf::from(dir);
        for name in [
            "libonnxruntime.so",
            "libonnxruntime.so.1",
            "libonnxruntime.dylib",
            "onnxruntime.dll",
        ] {
            let candidate = dir.join(name);
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
        // versioned sonames (libonnxruntime.so.1.30.0) are the common case
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut best: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("libonnxruntime.so") || name == "onnxruntime.dll" {
                    if best.as_ref().map(|b| name > b.file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or(true) {
                        best = Some(entry.path());
                    }
                }
            }
            if let Some(p) = best {
                return p.to_string_lossy().to_string();
            }
        }
        return dir.join("libonnxruntime.so").to_string_lossy().to_string();
    }
    eprintln!(
        "taste-engine-rs: ONNX Runtime not configured. Set ORT_DYLIB_PATH or ORT_LIB_DIR."
    );
    "libonnxruntime.so".to_string()
}

// ---------- bytes_to_unicode (GPT-2 / CLIP byte encoder) ----------

fn bytes_to_unicode() -> HashMap<u8, char> {
    let mut bs: Vec<u32> = Vec::new();
    for b in (b'!'..=b'~').map(|x| x as u32) {
        bs.push(b);
    }
    for b in (0xA1..=0xAC).map(|x| x as u32) {
        bs.push(b);
    }
    for b in (0xAE..=0xFF).map(|x| x as u32) {
        bs.push(b);
    }
    let mut cs: Vec<u32> = bs.clone();
    let mut b = 0u32;
    while bs.len() < 256 {
        b += 1;
        if bs.contains(&b) {
            continue;
        }
        bs.push(b);
        cs.push(b + 256);
    }
    let mut out = HashMap::new();
    for (x, y) in bs.iter().zip(cs.iter()) {
        out.insert(*x as u8, char::from_u32(*y).unwrap());
    }
    out
}

// ---------- BPE ----------

struct Tokenizer {
    byte_encoder: HashMap<u8, char>,
    vocab: HashMap<String, i64>,
    ranks: HashMap<(String, String), usize>,
    cache: HashMap<String, Vec<String>>,
}

impl Tokenizer {
    fn load() -> Tokenizer {
        let dir = models_dir();
        let vocab_raw = std::fs::read_to_string(dir.join("vocab.json")).expect("vocab.json");
        let vocab: HashMap<String, i64> = serde_json::from_str(&vocab_raw).expect("vocab parse");
        let merges_raw = std::fs::read_to_string(dir.join("merges.txt")).expect("merges.txt");
        let mut ranks = HashMap::new();
        for (i, line) in merges_raw.lines().enumerate() {
            if i == 0 && line.starts_with("#") {
                continue; // "#version" header
            }
            let mut parts = line.split(' ');
            let a = parts.next().unwrap_or("");
            let b = parts.next().unwrap_or("");
            if a.is_empty() || b.is_empty() {
                continue;
            }
            ranks.insert((a.to_string(), b.to_string()), i);
        }
        Tokenizer {
            byte_encoder: bytes_to_unicode(),
            vocab,
            ranks,
            cache: HashMap::new(),
        }
    }

    fn bpe(&mut self, token: &str) -> Vec<String> {
        if let Some(hit) = self.cache.get(token) {
            return hit.clone();
        }
        let chars: Vec<char> = token.chars().collect();
        // python: word = tuple(token[:-1]) + (token[-1] + "</w>",) — even 1-char
        // tokens get the end-of-word marker
        let mut word: Vec<String> = chars[..chars.len() - 1]
            .iter()
            .map(|c| c.to_string())
            .collect();
        word.push(format!("{}{}", chars[chars.len() - 1], "</w>"));
        loop {
            if word.len() < 2 {
                break;
            }
            // find the lowest-rank adjacent pair
            let mut best_rank = usize::MAX;
            let mut best_i = 0usize;
            for i in 0..word.len() - 1 {
                let rank = self
                    .ranks
                    .get(&(word[i].clone(), word[i + 1].clone()))
                    .copied()
                    .unwrap_or(usize::MAX);
                if rank < best_rank {
                    best_rank = rank;
                    best_i = i;
                }
            }
            if best_rank == usize::MAX {
                break;
            }
            let merged = format!("{}{}", word[best_i], word[best_i + 1]);
            let mut next: Vec<String> = Vec::with_capacity(word.len());
            let mut i = 0;
            while i < word.len() {
                if i == best_i && i + 1 < word.len() {
                    next.push(merged.clone());
                    i += 2;
                } else {
                    next.push(word[i].clone());
                    i += 1;
                }
            }
            word = next;
        }
        let out = word;
        self.cache.insert(token.to_string(), out.clone());
        out
    }

    /// HF CLIPTokenizer._tokenize: clean, lowercase, regex split, byte-encode, BPE.
    fn tokenize(&mut self, text: &str) -> Vec<String> {
        static PAT: OnceLock<regex::Regex> = OnceLock::new();
        let pat = PAT.get_or_init(|| {
            regex::Regex::new(
                r"<\|startoftext\|>|<\|endoftext\|>|'s|'t|'re|'ve|'m|'ll|'d|[\p{L}]+|[\p{N}]|[^\s\p{L}\p{N}]+",
            )
            .unwrap()
        });
        let cleaned = whitespace_clean(text).to_lowercase();
        let mut out: Vec<String> = Vec::new();
        for m in pat.find_iter(&cleaned) {
            let piece = m.as_str();
            let encoded: String = piece
                .chars()
                .map(|c| {
                    let mut buf = [0u8; 4];
                    let bytes = c.encode_utf8(&mut buf).as_bytes();
                    bytes
                        .iter()
                        .map(|b| self.byte_encoder.get(b).copied().unwrap_or('\u{fffd}'))
                        .collect::<String>()
                })
                .collect();
            out.extend(self.bpe(&encoded));
        }
        out
    }

    fn encode(&mut self, text: &str) -> Vec<i64> {
        // special-token keys are built at runtime — the bare literals kept
        // getting mangled in transit
        let sos = *self
            .vocab
            .get(&format!("<|{}|>", "startoftext"))
            .expect("sos token");
        let eos = *self
            .vocab
            .get(&format!("<|{}|>", "endoftext"))
            .expect("eos token");
        let pieces = self.tokenize(text);
        let mut ids: Vec<i64> = Vec::with_capacity(MAX_LEN);
        ids.push(sos);
        for p in &pieces {
            if let Some(id) = self.vocab.get(p) {
                ids.push(*id);
            } else {
                // unknown single chars -> byte fallback tokens (CLIP vocab covers all)
                for c in p.chars() {
                    if let Some(id) = self.vocab.get(&c.to_string()) {
                        ids.push(*id);
                    }
                }
            }
            if ids.len() >= MAX_LEN - 1 {
                break;
            }
        }
        ids.truncate(MAX_LEN - 1);
        ids.push(eos);
        // CLIP pads with the EOS token, not zeros
        while ids.len() < MAX_LEN {
            ids.push(eos);
        }
        ids
    }
}

fn whitespace_clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true;
    for ch in text.chars() {
        let is_space = ch.is_whitespace();
        if !is_space || !prev_space {
            out.push(if is_space { ' ' } else { ch });
        }
        prev_space = is_space;
    }
    out.trim().to_string()
}

// ---------- ONNX session ----------

struct Session {
    inner: ort::session::Session,
}

static TOK: OnceLock<Mutex<Tokenizer>> = OnceLock::new();
static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();

fn session() -> &'static Mutex<Option<Session>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

fn ensure_session() -> Result<(), String> {
    let mut slot = session().lock_ok();
    if slot.is_some() {
        return Ok(());
    }
    let path = models_dir().join("clip_text.onnx");
    if !path.exists() {
        return Err(format!("missing {}", path.display()));
    }
    // load-dynamic reads ORT_DYLIB_PATH; resolve it when the env is unset
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        // safe: single-threaded init point before any ort session exists
        unsafe { std::env::set_var("ORT_DYLIB_PATH", ort_dylib()) };
    }
    let sess = ort::session::Session::builder()
        .and_then(|mut b| b.commit_from_file(&path))
        .map_err(|e| format!("session: {e}"))?;
    *slot = Some(Session { inner: sess });
    Ok(())
}

/// verification helper: raw input ids for the debug endpoint
pub fn debug_tokens(text: &str) -> Vec<i64> {
    let tok = TOK.get_or_init(|| Mutex::new(Tokenizer::load()));
    let mut t = tok.lock_ok();
    t.encode(text)
}

/// embed.embed_text port: returns one 512-d f32 vector per input text.
pub fn embed_text(texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    ensure_session()?;
    let tok = TOK.get_or_init(|| Mutex::new(Tokenizer::load()));
    let ids: Vec<Vec<i64>> = {
        let mut t = tok.lock_ok();
        texts.iter().map(|s| t.encode(s)).collect()
    };
    let mut flat: Vec<i64> = Vec::with_capacity(texts.len() * MAX_LEN);
    for row in &ids {
        flat.extend(row.iter().copied());
    }
    let mut slot = session().lock_ok();
    let sess = slot.as_mut().unwrap();
    let input = ort::value::Tensor::from_array(
        ndarray::Array2::<i64>::from_shape_vec((texts.len(), MAX_LEN), flat).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let outputs = sess
        .inner
        .run(ort::inputs![input])
        .map_err(|e| format!("run: {e}"))?;
    let out = outputs["text_embeds"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("extract: {e}"))?;
    let (_shape, data) = out;
    let mut vecs: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for i in 0..texts.len() {
        let start = i * EMBED_DIM;
        vecs.push(data[start..start + EMBED_DIM].to_vec());
    }
    Ok(vecs)
}

// ---------- image embedding (CLIP vision tower) ----------

static VISION_SESSION: OnceLock<Mutex<Option<ort::session::Session>>> = OnceLock::new();

fn vision_session() -> &'static Mutex<Option<ort::session::Session>> {
    VISION_SESSION.get_or_init(|| Mutex::new(None))
}

fn ensure_vision_session() -> Result<(), String> {
    let mut slot = vision_session().lock_ok();
    if slot.is_some() {
        return Ok(());
    }
    let path = models_dir().join("clip_vision.onnx");
    if !path.exists() {
        return Err(format!("missing {}", path.display()));
    }
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        unsafe { std::env::set_var("ORT_DYLIB_PATH", ort_dylib()) };
    }
    let sess = ort::session::Session::builder()
        .and_then(|mut b| b.commit_from_file(&path))
        .map_err(|e| format!("vision session: {e}"))?;
    *slot = Some(sess);
    Ok(())
}

/// CLIPImageProcessor: resize shortest side to 224, center crop, RGB,
/// normalize with the CLIP mean/std. Returns CHW f32 buffer.
fn preprocess_image(img: &image::DynamicImage) -> Option<Vec<f32>> {
    const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_72];
    const STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_11];
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let scale = VISION_SIZE as f32 / w.min(h) as f32;
    let rw = ((w as f32 * scale).round() as u32).max(VISION_SIZE as u32);
    let rh = ((h as f32 * scale).round() as u32).max(VISION_SIZE as u32);
    let resized = img.resize_exact(rw, rh, image::imageops::FilterType::CatmullRom);
    let rgb = resized.to_rgb8();
    let left = (rgb.width() - VISION_SIZE as u32) / 2;
    let top = (rgb.height() - VISION_SIZE as u32) / 2;
    let crop = image::imageops::crop_imm(&rgb, left, top, VISION_SIZE as u32, VISION_SIZE as u32);
    let crop_img = crop.to_image();
    let mut out = vec![0.0f32; 3 * VISION_SIZE * VISION_SIZE];
    for y in 0..VISION_SIZE as u32 {
        for x in 0..VISION_SIZE as u32 {
            let p = crop_img.get_pixel(x, y);
            for c in 0..3 {
                let v = p[c] as f32 / 255.0;
                out[c * VISION_SIZE * VISION_SIZE + y as usize * VISION_SIZE + x as usize] =
                    (v - MEAN[c]) / STD[c];
            }
        }
    }
    Some(out)
}

/// embed.embed_image_bytes port: CLIP image embedding of raw image bytes.
pub fn embed_image_bytes(data: &[u8]) -> Option<Vec<f32>> {
    let img = match image::load_from_memory(data) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[embed-rs] image decode failed: {e}");
            return None;
        }
    };
    let pixels = match preprocess_image(&img) {
        Some(p) => p,
        None => {
            eprintln!("[embed-rs] preprocess failed");
            return None;
        }
    };
    if let Err(e) = ensure_vision_session() {
        eprintln!("[embed-rs] vision session: {e}");
        return None;
    }
    let mut slot = vision_session().lock_ok();
    let sess = slot.as_mut().unwrap();
    let arr = match ndarray::Array4::<f32>::from_shape_vec((1, 3, VISION_SIZE, VISION_SIZE), pixels) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[embed-rs] shape: {e}");
            return None;
        }
    };
    let input = match ort::value::Tensor::from_array(arr) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[embed-rs] tensor: {e}");
            return None;
        }
    };
    let outputs = match sess.run(ort::inputs![input]) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[embed-rs] run: {e}");
            return None;
        }
    };
    let out = match outputs["image_embeds"].try_extract_tensor::<f32>() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[embed-rs] extract: {e}");
            return None;
        }
    };
    let (_shape, data) = out;
    Some(data[..EMBED_DIM].to_vec())
}
